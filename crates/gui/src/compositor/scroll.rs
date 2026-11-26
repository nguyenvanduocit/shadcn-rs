//! Scroll on the render thread (ADR-0015, ADR-0032): each container's
//! offset shown is its committed base plus what the compositor moved it
//! by that the base does not hold yet (`owed`), as Chromium's synced scroll
//! offset; wheel gestures resampled at each frame's present time
//! (Chromium's `ScrollPredictor`, linear); flings where the platform has
//! none.
//!
//! **Order (ADR-0032, ADR-0033).** What the compositor moves goes to the
//! UI through the outbox ([`super::outbox`]), pulled as the render thread
//! hands the UI each input and each main frame, which the UI takes in
//! additively before it handles them; what the UI writes (a jump) comes
//! back as a commit's base, and what it had not taken then is kept on top
//! of that base, clamped once
//! (`cc/trees/property_tree.cc`, `PushScrollUpdatesFromMainThread`). No
//! clock orders anything here: OS times are read only within one source
//! (a gesture's resampling, its fling's speed).
//!
//! **Cost.** Every pass here is over the containers something is held for
//! (moved lately, owed, scrolled by the gesture): a dirty set, never the
//! scene's scroll table.
//!
//! Every instant here is exact nanoseconds on the frame clock
//! ([`crate::clock::Moment`]); only the span between two of them is taken
//! as seconds ([`span`]), so the physics is the same at any uptime.
//!
//! **Exact motion ([`super::motion`]).** Every position and every piece of
//! motion here is a whole number of units: a gesture's motion cut where
//! frames fall, summed here at each frame and by the UI at each pull, sums
//! to the same in any grouping; a resampled or flung position is rounded
//! to a unit once, as a position, and each cut is the difference of two
//! such positions, so what is applied is exactly what the container moved.

use foldhash::HashMap;

use super::motion::{self, Exact, clamp2 as clamped};
use super::outbox::{Moved, Outbox, ScrollerId};
use super::{Compositing, ScrollDelta, ScrollNode};
use crate::clock::{Moment, rate, span};

/// A scroll container a wheel scrolls, by index in the scene's scrollers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WheelTarget(pub u32);

/// How long past its last event a gesture is still predicted
/// (Chromium's `kScrollPredictorMaxResampleTime`), seconds.
const PREDICT_FOR: f32 = 0.035;
/// The furthest a prediction reaches past the last event (Chromium's
/// `linear_resampling` `kResampleMaxPrediction`), seconds.
const MAX_PREDICTION: f32 = 0.008;
/// How far back a scroll container's speed is measured, seconds: what it
/// moved over this, divided by it.
const SPEED_WINDOW: f32 = 0.1;
/// How far back from the fingers lifting a fling's speed is measured,
/// seconds: what the gesture moved from its first event in this window to
/// the lift, over that time (GTK's `SCROLL_CAPTURE_THRESHOLD_MS`,
/// `gtkeventcontrollerscroll.c`). The lift's own time counts, so fingers
/// that stopped before lifting fling slower, and not at all past it.
const FLING_WINDOW: f32 = 0.15;

/// How long an overlay scrollbar stays shown after the user last scrolled
/// its container, then how long it takes to fade out, seconds (cc's
/// overlay scrollbar fade delay and duration, `ScrollbarAnimationController`).
pub(super) const THUMB_SHOWN: f32 = 0.5;
pub(super) const THUMB_FADE: f32 = 0.2;

#[cfg(test)]
crate::engine::engine_local! {
    /// Container identities visited by the passes here (a cost test).
    [Fork] static VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// One container identity visited, or looked up.
pub(super) fn visit() {
    #[cfg(test)]
    VISITS.with(|v| v.set(v.get().saturating_add(1)));
}

/// Container identities this thread's passes visited so far.
#[cfg(test)]
pub(super) fn visits() -> u64 {
    VISITS.with(std::cell::Cell::get)
}

/// What one container shows above its committed base (I3): the
/// compositor's displacement the base does not hold, within the range.
/// Whether a commit jumped the container is told from the outbox's own
/// entries ([`super::outbox::Moved::generation`]), which a compositor
/// rebuilt after a panic is handed, not from this.
type Owed = Exact;

/// Where the gesture's events, summed, put the content: the samples a
/// prediction is made from. Exactly ([`super::motion`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScrollPredictor {
    /// (nanoseconds on the frame clock, the gesture's total so far).
    samples: Vec<(u64, Exact)>,
}

/// `a + (b - a) * k` on each axis, rounded to a unit: a resampled
/// position, computed in `f64` (exact for any total under 2^33 px).
fn lerp(a: Exact, b: Exact, k: f64) -> Exact {
    let one = |a: i64, b: i64| {
        let (fa, fb) = (crate::convert::i64_to_f64(a), crate::convert::i64_to_f64(b));
        crate::convert::f64_to_i64((fa + (fb - fa) * k).round())
    };
    (one(a.0, b.0), one(a.1, b.1))
}

/// `p + v * seconds` (`v` units a second) on each axis, rounded to a unit.
fn ahead_of(p: Exact, v: (f64, f64), seconds: f64) -> Exact {
    let one = |p: i64, v: f64| crate::convert::f64_to_i64((crate::convert::i64_to_f64(p) + v * seconds).round());
    (one(p.0, v.0), one(p.1, v.1))
}

/// The rate of `amount` units over `from..to`, pixels a second.
fn rate_px(amount: Exact, from: u64, to: u64) -> Option<(f32, f32)> {
    rate(motion::px2(amount), from, to)
}

impl ScrollPredictor {
    /// The gesture moved by `delta` at `at`.
    pub fn push(&mut self, at: impl Moment, delta: Exact) {
        let at = at.nanos();
        let total = self.samples.last().map_or((0, 0), |s| s.1);
        let total = motion::add2(total, delta);
        // Two events in the same instant are one.
        if let Some(last) = self.samples.last_mut()
            && span(last.0, at).abs() < 1e-6
        {
            last.1 = total;
            return;
        }
        self.samples.push((at, total));
        if self.samples.len() > 8 {
            self.samples.remove(0);
        }
    }

    /// The total the gesture has at `at`: interpolated between the events
    /// around it, or extrapolated from the last two, at most
    /// `min(8 ms, half their gap)` past the last; 35 ms after the last
    /// event, the last total as it is.
    #[must_use]
    pub fn at(&self, at: impl Moment) -> Exact {
        let at = at.nanos();
        let Some(&(t1, p1)) = self.samples.last() else {
            return (0, 0);
        };
        if span(t1, at) >= PREDICT_FOR {
            return p1;
        }
        // Between two events: where the content was then.
        if let Some(w) = self
            .samples
            .windows(2)
            .find(|w| matches!(w, [a, b] if a.0 <= at && at <= b.0))
            && let [(ta, pa), (tb, pb)] = w
        {
            let k = if tb > ta {
                f64::from(span(*ta, at)) / f64::from(span(*ta, *tb))
            } else {
                1.0
            };
            return lerp(*pa, *pb, k);
        }
        let Some(&(t0, p0)) = self.samples.len().checked_sub(2).and_then(|i| self.samples.get(i)) else {
            return p1;
        };
        // As [`rate`] has it: too close in time, no speed.
        let over = span(t0, t1);
        if at <= t1 || over < crate::clock::MIN_RATE_SPAN {
            return p1;
        }
        let over = f64::from(over);
        let d = motion::sub2(p1, p0);
        let v = (
            crate::convert::i64_to_f64(d.0) / over,
            crate::convert::i64_to_f64(d.1) / over,
        );
        let ahead = span(t1, at).min(MAX_PREDICTION).min(span(t0, t1) / 2.0);
        ahead_of(p1, v, f64::from(ahead))
    }

    /// The gesture's speed at its end, pixels a second, from its last events.
    #[must_use]
    pub fn velocity(&self) -> (f32, f32) {
        let n = self.samples.len();
        let (Some(&(t0, p0)), Some(&(t1, p1))) = (self.samples.get(n.saturating_sub(3)), self.samples.last()) else {
            return (0.0, 0.0);
        };
        rate_px(motion::sub2(p1, p0), t0, t1).unwrap_or_default()
    }

    /// When the last event came, nanoseconds on the frame clock.
    #[must_use]
    pub fn last(&self) -> Option<u64> {
        self.samples.last().map(|s| s.0)
    }

    /// What its events moved in all, unresampled; `None` before any.
    #[must_use]
    pub fn total(&self) -> Option<Exact> {
        self.samples.last().map(|s| s.1)
    }

    /// Its events' motion less `by`, which an edge stopped: every total
    /// moved by it, so what it predicts and interpolates is unchanged in
    /// shape and goes on from where the content is.
    pub fn absorb(&mut self, by: Exact) {
        for s in &mut self.samples {
            s.1 = motion::sub2(s.1, by);
        }
    }
}

/// What of `step` (a gesture's, signed as its events are) an edge of
/// `node` stopped, which moved it by `moved` (signed as its offset is) to
/// `shows`: on each axis it scrolls along and that now shows an edge, what
/// was asked less what moved; elsewhere nothing.
fn stopped(node: &ScrollNode, step: Exact, moved: Exact, shows: Exact) -> Exact {
    let one = |asked: i64, moved: i64, at: i64, end: i64| {
        if end > 0 && (at <= 0 || at >= end) {
            asked.saturating_sub(moved)
        } else {
            0
        }
    };
    let along = if node.rtl { step.0.saturating_neg() } else { step.0 };
    let x = one(along, moved.0, shows.0, node.end.0);
    (
        if node.rtl { x.saturating_neg() } else { x },
        one(step.1, moved.1, shows.1, node.end.1),
    )
}

/// Of `absorbed`, the part an edge stopped of a resampled step that went
/// `ahead` past its events' motion (a prediction's overshoot) is the
/// prediction's; the rest is the events' own (one axis).
fn events_absorbed(absorbed: i64, ahead: i64) -> i64 {
    if absorbed == 0 || ahead == 0 || absorbed.signum() != ahead.signum() {
        return absorbed;
    }
    let predicted = absorbed
        .saturating_abs()
        .min(ahead.saturating_abs())
        .saturating_mul(absorbed.signum());
    absorbed.saturating_sub(predicted)
}

/// A fling: the content slides on after the fingers lift, slowing
/// exponentially (a time constant of 325 ms, the decay of Android's and
/// iOS's scroll views at their normal rate).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fling {
    /// When it began, nanoseconds on the frame clock.
    start: u64,
    velocity: (f32, f32),
}

/// A fling's time constant, seconds.
const FLING_TAU: f32 = 0.325;
/// Slower than this (pixels a second) a fling ends, or never starts.
const FLING_MIN: f32 = 50.0;

impl Fling {
    /// A fling from `velocity` (pixels a second) at `start`; `None` when too slow.
    #[must_use]
    pub fn new(start: impl Moment, velocity: (f32, f32)) -> Option<Self> {
        (velocity.0.hypot(velocity.1) >= FLING_MIN).then_some(Self {
            start: start.nanos(),
            velocity,
        })
    }

    /// How long it moves, seconds: until its speed falls to [`FLING_MIN`].
    fn lasts(&self) -> f64 {
        let speed = f64::from(self.velocity.0.hypot(self.velocity.1));
        f64::from(FLING_TAU) * (speed / f64::from(FLING_MIN)).ln().max(0.0)
    }

    /// How far it has carried the content by `at`, exactly
    /// ([`super::motion`]): a function of time alone, computed in `f64` and
    /// rounded to a unit, so the fling handed to another compositor
    /// ([`super::Accepted`]) carries the content the same way on. It ends
    /// where it stops moving ([`Self::lasts`]), whenever the frame that
    /// finds it stopped comes: where it ends is no frame's to decide.
    #[must_use]
    pub fn at(&self, at: impl Moment) -> Exact {
        let t = f64::from(span(self.start, at.nanos()).max(0.0)).min(self.lasts());
        let tau = f64::from(FLING_TAU);
        let k = tau * (1.0 - (-t / tau).exp()) * crate::convert::i64_to_f64(motion::PER_PX);
        let one = |v: f32| crate::convert::f64_to_i64((f64::from(v) * k).round());
        (one(self.velocity.0), one(self.velocity.1))
    }

    /// How fast it carries the content at `at`, pixels a second.
    #[must_use]
    pub fn velocity_at(&self, at: impl Moment) -> (f32, f32) {
        let k = (-span(self.start, at.nanos()).max(0.0) / FLING_TAU).exp();
        (self.velocity.0 * k, self.velocity.1 * k)
    }

    /// Whether it still moves at `at`.
    #[must_use]
    pub fn moving(&self, at: impl Moment) -> bool {
        f64::from(span(self.start, at.nanos()).max(0.0)) < self.lasts()
    }
}

/// What the compositor resamples and flings of the platform's precise
/// gesture under way (a trackpad's): one per platform gesture, as its
/// latch ([`super::WheelGesture`]) owns it.
#[derive(Clone, Debug)]
struct Gesture {
    /// Its container as last committed (or sampled): what its rest is
    /// applied to when it stops, without a look at the table.
    node: ScrollNode,
    /// The platform gesture's identity ([`super::latch::Went::began`]).
    began: u64,
    predictor: ScrollPredictor,
    /// Its events of the last [`FLING_WINDOW`]: (when, delta).
    recent: Vec<(u64, Exact)>,
    /// How much of its events' motion (resampled) moved the container:
    /// exactly what it moved it by, an edge's stop taken out.
    applied: Exact,
    /// Its fling, and how far the fling moved the container so far.
    fling: Option<(Fling, Exact)>,
}

impl Gesture {
    /// Its speed when the fingers lifted at `at`, pixels a second: what it
    /// moved from its first event within [`FLING_WINDOW`] of `at`, over the
    /// time from that event to `at`.
    fn lift_velocity(&self, at: u64) -> (f32, f32) {
        let mut recent = self
            .recent
            .iter()
            .filter(|(t, _)| *t <= at && span(*t, at) <= FLING_WINDOW);
        let Some(&(first, d)) = recent.next() else {
            return (0.0, 0.0);
        };
        let sum = recent.fold(d, |a, (_, d)| motion::add2(a, *d));
        rate_px(sum, first, at).unwrap_or_default()
    }

    /// What its events moved and the container was not yet moved by: its
    /// resampling behind them, or ahead of them (a prediction's overshoot,
    /// negative). Settled, the container holds exactly its events' motion.
    fn rest(&mut self) -> Exact {
        let Some(total) = self.predictor.total() else {
            return (0, 0);
        };
        let rest = motion::sub2(total, self.applied);
        self.applied = total;
        rest
    }
}

/// A fling under way as data ([`super::Accepted`]): its container as last
/// committed, its gesture's identity, the fling (when it began, how fast:
/// a function of time alone), and how far it moved the container so far.
/// A compositor handed it carries the content on from there by the time
/// of its frames, as the one before would have.
#[derive(Clone, Debug)]
pub(super) struct Flinging {
    pub(super) node: ScrollNode,
    pub(super) began: u64,
    pub(super) fling: Fling,
    pub(super) flung: Exact,
}

/// A precise gesture's motion so far, as whichever side routes it keeps
/// it: its events of the last [`FLING_WINDOW`] and when its fingers
/// lifted. It goes with the gesture's latch between the compositor and
/// the UI (ADR-0033, "One owner of a gesture's latch"), so the fling at
/// its lift is the same whichever side took its events or its lift.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GestureMotion {
    /// The gesture's identity ([`super::latch::Went::began`]).
    pub(crate) began: u64,
    pub(crate) recent: Vec<(u64, Exact)>,
    /// When its fingers lifted, if they did.
    pub(crate) lifted: Option<u64>,
    /// The container it moved: where its fling goes on once lifted,
    /// whether or not its latch outlived the lift (the platform's momentum
    /// ends the latch, not the fling).
    pub(crate) to: Option<super::HitTarget>,
}

impl GestureMotion {
    /// An event of gesture `began` at `at`, moving `delta` on `to`,
    /// `lifted` or not.
    pub(crate) fn record(&mut self, began: u64, at: u64, delta: Exact, lifted: bool, to: super::HitTarget) {
        if self.began != began {
            *self = Self {
                began,
                ..Self::default()
            };
        }
        if self.lifted.is_none() {
            self.to = Some(to);
        }
        // Past the lift (the platform's momentum) is no part of the speed
        // the fingers lifted at.
        if delta != (0, 0) && self.lifted.is_none() {
            self.recent.retain(|(t, _)| span(*t, at) <= FLING_WINDOW);
            self.recent.push((at, delta));
        }
        if lifted && self.lifted.is_none() {
            self.lifted = Some(at);
        }
    }
}

/// `node`'s identity: the instance paint gave it ([`ScrollNode::instance`]),
/// new for another component instance at its path and for one that left
/// the tables and came back. Every piece of per-container state (what is
/// owed, its moves, its scrollbar, its gesture) is the instance's, so none
/// outlives it.
pub(super) fn id_of(node: &ScrollNode) -> ScrollerId {
    node.instance
}

/// A move the compositor made: when, and by how much.
type Step = (u64, Exact);

/// Every scroll container's displacement above its base, the gesture under
/// way, and the outbox they go to the UI from, each by the container's
/// identity ([`ScrollerId`]).
pub(super) struct Scrolls {
    outbox: Outbox,
    /// What each container shows above its committed base; none, nothing.
    owed: HashMap<ScrollerId, Owed>,
    gesture: Option<Gesture>,
    /// What the compositor moved each container by lately, when: its speed.
    moves: HashMap<ScrollerId, Vec<Step>>,
    /// Whether a trackpad gesture is flung on after the fingers lift: the
    /// platform sends no momentum of its own.
    flings: bool,
    /// When the user last scrolled each container, on the frame clock: its
    /// overlay scrollbar shows from then, and fades. A program's scroll
    /// shows none.
    touched: HashMap<ScrollerId, u64>,
    /// Kept between commits, empty: a commit's bookkeeping allocates
    /// nothing once warm (each container it visits, once).
    aggs: HashMap<ScrollerId, Agg>,
    /// A main frame's reach, kept between frames: given back with the
    /// commit answering it ([`super::Commit::reach`]), so granting frames
    /// while a container moves allocates nothing once warm.
    reach_spare: Vec<ScrollDelta>,
    /// A main frame's pull, kept between frames alike ([`super::Commit::pulled`]).
    pull_spare: Vec<ScrollDelta>,
    /// Made afresh with a fling handed on and no commit taken in yet: its
    /// tables hold no container, so a tick neither moves nor ends the
    /// fling; the first commit has its container, and the next tick
    /// carries it on by time.
    uncommitted: bool,
}

/// A container as a commit has it: its new base, range's end and
/// generation, exactly.
type Committed = (Exact, Exact, u64);

/// What a commit gathers of one container as it goes over the entries
/// held for it ([`Scrolls::committed`]).
#[derive(Clone, Copy)]
struct Agg {
    /// As the commit has it; `None`: gone from the commit.
    node: Option<Committed>,
    /// Where what was pulled since the commit was built, and then what
    /// was not, put it, each clamped as it lands in turn; `None`: nothing
    /// held for it moved it.
    pos: Option<Exact>,
    /// A jump the commit brings stopped what the compositor moved there.
    jumped: bool,
}

impl Default for Scrolls {
    fn default() -> Self {
        Self::with(Outbox::default(), None, PLATFORM_FLINGS_NOT)
    }
}

impl Scrolls {
    /// Scrolls whose motion goes to the UI through `outbox`, carrying on
    /// `fling` (handed on by a compositor before it, [`super::Accepted`]),
    /// flinging gestures as `flings` says ([`Self::set_flings`]).
    pub(super) fn with(outbox: Outbox, fling: Option<Flinging>, flings: bool) -> Self {
        let uncommitted = fling.is_some();
        let gesture = fling.map(|f| Gesture {
            node: f.node,
            began: f.began,
            predictor: ScrollPredictor::default(),
            recent: Vec::new(),
            applied: (0, 0),
            fling: Some((f.fling, f.flung)),
        });
        Self {
            outbox,
            owed: HashMap::default(),
            gesture,
            moves: HashMap::default(),
            flings,
            touched: HashMap::default(),
            aggs: HashMap::default(),
            reach_spare: Vec::new(),
            pull_spare: Vec::new(),
            uncommitted,
        }
    }

    /// The outbox what it moves goes to the UI from.
    #[cfg(test)]
    pub(super) fn outbox(&mut self) -> &mut Outbox {
        &mut self.outbox
    }

    /// All it holds of input accepted ([`super::Accepted`]), for a
    /// compositor made afresh after a panic: its outbox, its fling as data
    /// (its gesture settled first, [`Self::settle_gesture`]), and whether
    /// it flings. Every field is named: each is carried, or is shown state
    /// made again by the next motion.
    pub(super) fn into_accepted(self) -> super::Accepted {
        let Self {
            outbox,
            // Owed above a base the next compositor has from the UI's full
            // commit, which holds what was pulled; what was not is in the
            // outbox, kept on top of that base as it comes.
            owed: _,
            gesture,
            // A speed for a list's reach, and a scrollbar's fading: shown
            // state, made again by the next motion.
            moves: _,
            flings,
            touched: _,
            aggs: _,
            reach_spare: _,
            pull_spare: _,
            uncommitted: _,
        } = self;
        // Only a fling goes on: a gesture still resampled had its rest
        // applied, and its latch went to the UI.
        let fling = gesture.and_then(|g| {
            let (fling, flung) = g.fling?;
            Some(Flinging {
                node: g.node,
                began: g.began,
                fling,
                flung,
            })
        });
        super::Accepted::new(outbox, fling, flings)
    }

    /// Hand the UI, into `into`, what it moved since the last pull
    /// ([`Outbox::pull`]): the pull's number.
    pub(super) fn pull(&mut self, into: &mut Vec<ScrollDelta>) -> u64 {
        self.outbox.pull(into)
    }

    /// A main frame's pull, into the buffer kept ([`Self::recycle_pull`]).
    pub(super) fn pull_for_frame(&mut self) -> super::Pulled {
        let mut deltas = std::mem::take(&mut self.pull_spare);
        let take = self.outbox.pull(&mut deltas);
        super::Pulled { take, deltas }
    }

    /// A main frame's pull given back: kept for the next, the larger.
    pub(super) fn recycle_pull(&mut self, mut pulled: Vec<ScrollDelta>) {
        if pulled.capacity() > self.pull_spare.capacity() {
            pulled.clear();
            self.pull_spare = pulled;
        }
    }

    /// Fling trackpad gestures after they end (`true`), or leave it to the
    /// platform's momentum events; by default as the platform built for
    /// does. A replayed input log says what its platform did.
    pub(super) fn set_flings(&mut self, flings: bool) {
        self.flings = flings;
    }

    /// The offset shown for `node` (at `index` among the scrollers): its
    /// base and what it owes, within its range.
    pub(super) fn offset(&self, node: &ScrollNode, index: u32) -> (f32, f32) {
        motion::px2(self.pos(node, index))
    }

    /// [`Self::offset`], exactly ([`super::motion`]).
    pub(super) fn pos(&self, node: &ScrollNode, _index: u32) -> Exact {
        let owed = self.owed.get(&id_of(node)).copied().unwrap_or_default();
        clamped(motion::add2(node.pos, owed), node.end)
    }

    /// Scroll `target` by `delta` (the wheel's: positive moves the content
    /// up and toward the inline start; exactly, [`super::motion`]) at
    /// `time` on the frame clock: a notched turn, applied at once. Whether
    /// that is news for a frame: it moved, or it was the first step held
    /// against its edge the UI has not taken (its overlay scrollbar shows).
    pub(super) fn scroll(&mut self, scrollers: &[ScrollNode], target: WheelTarget, delta: Exact, time: u64) -> bool {
        let Some(node) = usize::try_from(target.0).ok().and_then(|i| scrollers.get(i)) else {
            return false;
        };
        let Some(fresh) = self.held(node, delta, time) else {
            return false;
        };
        self.scroll_node(node, delta, time, false)
            .is_some_and(|moved| moved != (0, 0) || fresh)
    }

    /// Whether `node` takes a wheel event moving `delta` at `at`, decided
    /// as the event comes (a notched turn, a trackpad's event alike): a
    /// container below a modal layer takes none; one asked along an axis
    /// it scrolls takes it, moved or held against its edge, as one wheel
    /// step the UI is told of once ([`ScrollDelta::steps`]), its overlay
    /// scrollbar shown either way (Chrome's overlay scrollbars show on
    /// every scroll their container takes). `Some(fresh)` when taken:
    /// fresh, no step of it waits for the UI yet.
    fn held(&mut self, node: &ScrollNode, delta: Exact, at: u64) -> Option<bool> {
        let along = if node.rtl { delta.0.saturating_neg() } else { delta.0 };
        if !movable(node) || !wheel_takes(node.end, (along, delta.1)) {
            return None;
        }
        visit();
        self.touched.insert(id_of(node), at);
        let m = self.outbox.pending.entry(id_of(node), node.key, node.owner);
        m.generation = node.generation;
        let fresh = m.steps == 0;
        m.steps = m.steps.saturating_add(1);
        Some(fresh)
    }

    /// Scroll `node` by `delta` at `at` (a wheel it took, a gesture's
    /// resampling, a fling: `synthetic`), within its range from where it
    /// shows (I3): what an edge stops is not owed, so a push outward at an
    /// edge moves nothing now and nothing later. How far it moved, or
    /// `None` when it cannot move (below a modal layer). Every move the
    /// compositor makes comes here, and goes to the outbox as it is made.
    fn scroll_node(&mut self, node: &ScrollNode, delta: Exact, at: u64, synthetic: bool) -> Option<Exact> {
        if !movable(node) {
            return None;
        }
        let along = if node.rtl { delta.0.saturating_neg() } else { delta.0 };
        let asked = (
            if node.end.0 > 0 { along } else { 0 },
            if node.end.1 > 0 { delta.1 } else { 0 },
        );
        if asked == (0, 0) {
            return Some((0, 0));
        }
        visit();
        let id = id_of(node);
        // Exactly: what it moves is what goes to the UI, whatever the
        // depth (ADR-0033, "One exact motion account").
        let was = self.pos(node, 0);
        let next = clamped(motion::add2(was, asked), node.end);
        let moved = motion::sub2(next, was);
        if moved == (0, 0) {
            return Some(moved);
        }
        self.owed.insert(id, motion::sub2(next, node.pos));
        let m = self.outbox.pending.entry(id, node.key, node.owner);
        m.generation = node.generation;
        let into = if synthetic { &mut m.synthetic } else { &mut m.real };
        *into = motion::add2(*into, moved);
        // Its overlay scrollbar shows while it moves (a fling too).
        self.touched.insert(id, at);
        let m = self.moves.entry(id).or_default();
        m.retain(|(t, _)| span(*t, at) < SPEED_WINDOW);
        m.push((at, moved));
        Some(moved)
    }

    /// The opacity of `node`'s overlay scrollbar at `at`: 1 while the user
    /// scrolls it and for [`THUMB_SHOWN`] after, then down to 0 over
    /// [`THUMB_FADE`]; 0 at rest.
    pub(super) fn thumb_opacity(&self, node: &ScrollNode, at: impl Moment) -> f32 {
        let at = at.nanos();
        self.touched.get(&id_of(node)).map_or(0.0, |&t| {
            let idle = span(t, at) - THUMB_SHOWN;
            if idle <= 0.0 {
                1.0
            } else {
                (1.0 - idle / THUMB_FADE).max(0.0)
            }
        })
    }

    /// Whether an overlay scrollbar is still shown or fading at `at`.
    fn thumbs_shown(&self, at: u64) -> bool {
        self.touched.values().any(|&t| span(t, at) < THUMB_SHOWN + THUMB_FADE)
    }

    /// How fast the compositor scrolls `id` at `at`, offset pixels a
    /// second: a fling's own speed, else what it moved over the last
    /// [`SPEED_WINDOW`], from its first event there to its last.
    fn speed(&self, id: ScrollerId, at: u64) -> (f32, f32) {
        if let Some(g) = &self.gesture
            && id_of(&g.node) == id
            && let Some((f, _)) = &g.fling
            && f.moving(at)
        {
            return f.velocity_at(at);
        }
        let Some(m) = self.moves.get(&id) else {
            return (0.0, 0.0);
        };
        let mut recent = m.iter().filter(|(t, _)| span(*t, at) < SPEED_WINDOW);
        let Some(&(first, _)) = recent.next() else {
            return (0.0, 0.0);
        };
        let (last, sum) = recent.fold((first, (0, 0)), |(_, a), &(t, d)| (t, motion::add2(a, d)));
        rate_px(sum, first, last).unwrap_or_default()
    }

    /// A precise wheel event of the platform gesture `began` names
    /// ([`super::WheelGesture`]), on `target` at `at`: resampled at each
    /// frame ([`Self::tick`]). `lifted`: the fingers lifted, and a fling
    /// carries it on where the platform has none of its own. Whether it is
    /// news for a frame: its container took it and it may move it, or it
    /// is the first step the UI has not taken, or its lift began a fling
    /// (none is below a modal layer).
    pub(super) fn gesture(
        &mut self,
        scrollers: &[ScrollNode],
        target: Option<WheelTarget>,
        delta: Exact,
        (at, began): (impl Moment, u64),
        lifted: bool,
    ) -> bool {
        let at = at.nanos();
        let node = target
            .and_then(|t| usize::try_from(t.0).ok())
            .and_then(|i| scrollers.get(i))
            .filter(|n| movable(n));
        // An event no container takes moves nothing: its end flings
        // nothing either, whatever gesture ran before it.
        let Some(node) = node else { return false };
        // Taken as it comes, a step for the UI ([`Self::held`]): news when
        // it is the first it has not taken, or when it may move its
        // container. One along an axis it does not scroll (a sideways step
        // of a gesture latched to a list) is no part of the gesture's
        // motion and asks for nothing, but its lift still ends the gesture:
        // the fling is decided from what the gesture moved.
        let taken = if delta == (0, 0) {
            Some(false)
        } else {
            self.held(node, delta, at).map(|fresh| {
                let along = if node.rtl { delta.0.saturating_neg() } else { delta.0 };
                fresh || wheel_moves(self.pos(node, 0), node.end, (along, delta.1))
            })
        };
        let same = self
            .gesture
            .as_ref()
            .is_some_and(|g| g.node.instance == node.instance && g.began == began);
        if taken.is_none() && !(same && lifted) {
            return false;
        }
        // Another gesture replaces the one under way: that one's motion
        // is the container's whole first (I6).
        if !same {
            self.stop_gesture(at);
        }
        let mut g = self.gesture.take().unwrap_or_else(|| Gesture {
            node: node.clone(),
            began,
            predictor: ScrollPredictor::default(),
            recent: Vec::new(),
            applied: (0, 0),
            fling: None,
        });
        g.node = node.clone();
        if taken.is_some() && delta != (0, 0) {
            g.predictor.push(at, delta);
            g.recent.retain(|(t, _)| span(*t, at) <= FLING_WINDOW);
            g.recent.push((at, delta));
        }
        // Flung from this gesture's own samples: its container took it.
        // The fling goes on from where its events put the content, all of
        // their motion applied first.
        let mut flung = false;
        if lifted && self.flings && g.fling.is_none() {
            g.fling = Fling::new(at, g.lift_velocity(at)).map(|f| (f, (0, 0)));
            flung = g.fling.is_some();
            if flung {
                let rest = g.rest();
                self.scroll_node(&g.node, rest, at, false);
            }
        }
        self.gesture = Some(g);
        taken.unwrap_or(false) || flung
    }

    /// The gesture under way ends, or is replaced (fingers down again, a
    /// new gesture, a jump): what its events moved and the container was
    /// not yet moved by moves it now, a prediction's overshoot taken back
    /// (I6), so no real motion is lost however few frames ran; its
    /// resampling and fling stop.
    /// A gesture is resampled or flung: its motion waits for a tick.
    pub(super) fn gesture_moving(&self) -> bool {
        // One settled with its fingers down holds nothing until its next
        // event ([`Self::settle_gesture`]).
        self.gesture
            .as_ref()
            .is_some_and(|g| g.fling.is_some() || g.predictor.total().is_some())
    }

    pub(super) fn stop_gesture(&mut self, at: u64) {
        if let Some(mut g) = self.gesture.take() {
            let rest = g.rest();
            if rest != (0, 0) {
                self.scroll_node(&g.node, rest, at, false);
            }
            // Its fling stops at `at`, brought up to then if no frame did
            // (none drawn: the device lost, the window hidden), never
            // taken back from where a frame ahead of `at` showed it.
            if let Some((f, flung)) = &g.fling {
                let d = f.at(at);
                let ahead = |d: i64, by: i64| {
                    let more = d.saturating_sub(by);
                    if d >= 0 { more.max(0) } else { more.min(0) }
                };
                let step = (ahead(d.0, flung.0), ahead(d.1, flung.1));
                if step != (0, 0) {
                    self.scroll_node(&g.node, step, at, true);
                }
            }
        }
    }

    /// Apply at `at` all the gesture under way holds of its events (its
    /// resampling's rest, I6), and its fling up to `at`, so the outbox has
    /// every motion accepted so far, with no frame drawn (input diverted,
    /// the window hidden, no device): the fling is not ended, only brought
    /// up to `at`; it goes on by time from there, here or in a compositor
    /// handed it ([`Self::into_accepted`]), as it would have with frames
    /// (ADR-0033, "Accepted motion"). Resampling with nothing left to apply
    /// ends.
    pub(super) fn settle_gesture(&mut self, at: u64) {
        let Some(mut g) = self.gesture.take() else { return };
        let rest = g.rest();
        if rest != (0, 0) {
            self.scroll_node(&g.node, rest, at, false);
        }
        let Some((f, flung)) = &mut g.fling else {
            // Fingers still down: all its events moved is applied, and
            // its resampling starts afresh from the next event, but the
            // gesture goes on, its recent events kept, so its lift still
            // flings as fast as they went, whether frames were drawn or
            // not (the fault matrix: a lift with the device lost flung
            // nothing).
            let lifts = self.flings && g.recent.last().is_some_and(|(t, _)| span(*t, at) < FLING_WINDOW);
            if lifts {
                g.predictor = ScrollPredictor::default();
                g.applied = (0, 0);
                self.gesture = Some(g);
            }
            return;
        };
        let d = f.at(at);
        let step = motion::sub2(d, *flung);
        *flung = d;
        if step != (0, 0) {
            self.scroll_node(&g.node, step, at, true);
        }
        if f.moving(at) {
            self.gesture = Some(g);
        }
    }

    /// A fling handed on stops: a wheel reached the UI while it was handed
    /// on ([`super::Compositor::take_gesture`]), as a new gesture stops it
    /// here.
    /// The motion of gesture `began` it holds, fingers still down, handed
    /// to the UI with its latch: none once its fling began (that goes on
    /// here, or in [`super::Accepted`]).
    pub(super) fn motion_of(&self, began: u64) -> Option<GestureMotion> {
        let g = self
            .gesture
            .as_ref()
            .filter(|g| g.began == began && g.fling.is_none())?;
        Some(GestureMotion {
            began,
            recent: g.recent.clone(),
            lifted: None,
            to: Some(super::HitTarget {
                key: g.node.key,
                owner: g.node.owner,
            }),
        })
    }

    /// Gesture `motion` goes on here on `node`, handed back by the UI
    /// that routed its events meanwhile: its recent events kept, so its
    /// lift here flings as fast as they went, and lifted there, it flings
    /// from the lift, carried on by time from then, as it would have been
    /// with no hand-over (ADR-0033, "Accepted motion").
    pub(super) fn adopt(&mut self, node: &ScrollNode, motion: GestureMotion) {
        if !movable(node) || (motion.recent.is_empty() && motion.lifted.is_none()) {
            return;
        }
        // Its fling goes on here already (handed on in [`super::Accepted`]):
        // what the UI took meanwhile (the platform's momentum) is no new
        // lift.
        if self
            .gesture
            .as_ref()
            .is_some_and(|g| g.began == motion.began && g.fling.is_some())
        {
            return;
        }
        let mut g = Gesture {
            node: node.clone(),
            began: motion.began,
            predictor: ScrollPredictor::default(),
            recent: motion.recent,
            applied: (0, 0),
            fling: None,
        };
        if let Some(lift) = motion.lifted {
            if !self.flings {
                return;
            }
            let Some(f) = Fling::new(lift, g.lift_velocity(lift)) else {
                return;
            };
            g.fling = Some((f, (0, 0)));
        }
        self.gesture = Some(g);
    }

    pub(super) fn stop_fling(&mut self) {
        if self.gesture.as_ref().is_some_and(|g| g.fling.is_some()) {
            self.gesture = None;
        }
    }

    /// Move the gesture's scroll container to where its events, resampled
    /// at `at`, or its fling put it. Its container is looked up by
    /// identity in the tables' index ([`Compositing::scroller`]), never
    /// found by a walk of the table, and the scrollbars that fade are
    /// those touched: a tick costs the same on a page of any size.
    pub(super) fn tick(&mut self, tables: &Compositing, at: impl Moment) {
        let at = at.nanos();
        self.touched.retain(|_, t| {
            visit();
            span(*t, at) < THUMB_SHOWN + THUMB_FADE
        });
        let Some(mut g) = self.gesture.take() else { return };
        visit();
        // A fling handed on before any commit came: its container is in
        // none of the tables yet; it waits, and goes on by time.
        if self.uncommitted {
            self.gesture = Some(g);
            return;
        }
        // Gone, or below a modal layer opened since: it stops where it is,
        // its unapplied rest dropped (intended, ADR-0032 I6: a container
        // below a modal takes no scroll, as a wheel over it takes none).
        let Some((_, node)) = tables.scroller(id_of(&g.node)).filter(|(_, n)| movable(n)) else {
            return;
        };
        g.node = node.clone();
        match &mut g.fling {
            Some((f, flung)) => {
                let d = f.at(at);
                let step = motion::sub2(d, *flung);
                *flung = d;
                if step != (0, 0) {
                    self.scroll_node(node, step, at, true);
                }
            }
            None => {
                let want = g.predictor.at(at);
                let step = motion::sub2(want, g.applied);
                g.applied = want;
                if step != (0, 0)
                    && let Some(moved) = self.scroll_node(node, step, at, false)
                    && let absorbed = stopped(node, step, moved, self.pos(node, 0))
                    && absorbed != (0, 0)
                {
                    // What the edge stopped (Codex r8 5): the gesture keeps
                    // what the container accepted, and its events' motion
                    // the edge stopped is gone from its total, so settling
                    // never pulls the content back off the edge, and a turn
                    // back moves it from the edge at once.
                    let total = g.predictor.total().unwrap_or_default();
                    let ahead = motion::sub2(want, total);
                    g.applied = motion::sub2(want, absorbed);
                    g.predictor.absorb((
                        events_absorbed(absorbed.0, ahead.0),
                        events_absorbed(absorbed.1, ahead.1),
                    ));
                }
            }
        }
        let kept = match &g.fling {
            Some((f, _)) => f.moving(at),
            // Past its prediction it moves no more, but the fingers may
            // still lift and fling it, as fast as its last events went.
            // Its recent events, not its resampling's: one settled (no
            // frame drawn) resamples afresh, yet its lift still flings.
            None => {
                g.predictor.last().is_some_and(|l| span(l, at) < PREDICT_FOR)
                    || (self.flings && g.recent.last().is_some_and(|(l, _)| span(*l, at) < FLING_WINDOW))
            }
        };
        self.gesture = Some(g);
        if !kept {
            self.stop_gesture(at);
        }
    }

    /// Whether it moved anything, or took a wheel, the UI has not taken
    /// yet: the UI is owed a main frame.
    pub(super) fn pending(&self) -> bool {
        self.outbox.untaken()
    }

    /// Whether a gesture is still resampled or a fling moves at `at`.
    pub(super) fn animating(&self, at: impl Moment) -> bool {
        let at = at.nanos();
        self.thumbs_shown(at)
            || self.gesture.as_ref().is_some_and(|g| match &g.fling {
                Some((f, _)) => f.moving(at),
                None => g.predictor.last().is_some_and(|l| span(l, at) < PREDICT_FOR),
            })
    }

    /// How far each container moving at `at` may be scrolled within
    /// `horizon` seconds (until the commit after the main frame granted
    /// now shows), for that main frame: the content a virtual list builds
    /// beyond its viewport (ADR-0015). Over the containers that moved
    /// lately alone (I7).
    pub(super) fn reach(&mut self, tables: &Compositing, at: impl Moment, horizon: f32) -> Vec<ScrollDelta> {
        let at = at.nanos();
        self.moves.retain(|_, m| {
            visit();
            m.retain(|(t, _)| span(*t, at) < SPEED_WINDOW);
            !m.is_empty()
        });
        let mut out = std::mem::take(&mut self.reach_spare);
        out.clear();
        for &id in self.moves.keys() {
            visit();
            let Some((_, node)) = tables.scroller(id) else { continue };
            let speed = self.speed(id, at);
            let reach = (speed.0 * horizon, speed.1 * horizon);
            if reach != (0.0, 0.0) {
                out.push(ScrollDelta {
                    key: node.key,
                    owner: node.owner,
                    instance: id,
                    reach,
                    ..ScrollDelta::default()
                });
            }
        }
        out
    }

    /// A commit came with `tables`, its bases holding every pull the UI
    /// took in through the one numbered `taken_through` (I4): those are
    /// dropped, and what was pulled since and what was not pulled is kept
    /// on top of each new base, clamped once to the range it has now, in
    /// the order it was moved (`PushScrollUpdatesFromMainThread`). A commit
    /// that did not come (dropped after a render thread panic) changes
    /// nothing here: the next one carries its pulls, which stay kept until
    /// then, as Chromium's `ApplySentScrollDeltasFromAbortedCommit` keeps a
    /// sent delta whose commit was aborted.
    ///
    /// A container the UI jumped since (another [`ScrollNode::generation`],
    /// Chromium's clobbering commit, `commit_state.h`) stops what the
    /// compositor moves by itself there: its gesture's fling, the gesture's
    /// resampling (its events' motion applied whole first, I6), and what of
    /// its fling the UI has not taken. Real input is never dropped.
    ///
    /// A container gone from the commit is gone for good (one coming back
    /// is another instance): what was held for it goes. Each container
    /// something is held for is looked up by identity in the tables' index
    /// ([`Compositing::scroller`]): a commit costs what is held, not the
    /// size of the page.
    pub(super) fn committed(&mut self, tables: &Compositing, taken_through: u64, at: u64) {
        self.uncommitted = false;
        // The gesture's container: gone or covered by a modal layer, it
        // stops where it is; jumped, its events' motion is applied on the
        // new base and it stops.
        let mut settle = None;
        if let Some(g) = &self.gesture {
            visit();
            match tables.scroller(id_of(&g.node)).map(|(_, n)| n).filter(|n| movable(n)) {
                None => self.gesture = None,
                Some(n) if n.generation != g.node.generation => {
                    settle = self.gesture.take().map(|mut g| (g.rest(), n.clone()));
                }
                Some(n) => {
                    let n = n.clone();
                    if let Some(g) = &mut self.gesture {
                        g.node = n;
                    }
                }
            }
        }
        let mut aggs = std::mem::take(&mut self.aggs);
        aggs.clear();
        self.owed.clear();
        let owed = &mut self.owed;
        let moves = &mut self.moves;
        // Each container's aggregate, made at its first entry: one lookup
        // of the tables apiece (Codex r9 I2: one pass over the entries, in
        // the order moved; nothing scans the runs for a container).
        let agg = |aggs: &mut HashMap<ScrollerId, Agg>, id: ScrollerId| -> Agg {
            visit();
            *aggs.entry(id).or_insert_with(|| {
                visit();
                Agg {
                    node: tables.scroller(id).map(|(_, n)| (n.pos, n.end, n.generation)),
                    pos: None,
                    jumped: false,
                }
            })
        };
        let o = &mut self.outbox;
        while o.sent.front().is_some_and(|(n, _)| *n <= taken_through) {
            if let Some((_, sums)) = o.sent.pop_front() {
                o.recycle(sums);
            }
        }
        // A jump the commit brings is told from each entry's own
        // generation, so it is told the same by a compositor rebuilt after
        // a panic, which was handed the outbox (Opus r8 nit): the
        // compositor's own motion made before it goes, in what was pulled
        // since the commit was built (the UI left that out) and in what was
        // not pulled. A container gone from the commit is gone for good:
        // its entries go (cc drops a gone element's synced offset,
        // `property_tree.cc` `PushScrollUpdatesFromMainThread`).
        let jump = |a: &mut Agg, m: &mut Moved, generation: u64| {
            if m.generation != generation {
                a.jumped = true;
                m.synthetic = (0, 0);
                m.generation = generation;
            }
        };
        // What was pulled since the commit was built, on each base, a pull
        // at a time in the order handed on, each clamped where it lands:
        // as the UI, which takes each in before the input it rides with
        // and clamps it to the range it laid out (the commit's), will hold
        // it (Codex r10 2: a sum clamped once differs once a range shrank
        // under two pulls that turn back).
        for (_, s) in &mut o.sent {
            s.0.retain(|&id, m| {
                let mut a = agg(&mut aggs, id);
                let Some((offset, max, generation)) = a.node else {
                    return false;
                };
                jump(&mut a, m, generation);
                let t = m.total();
                let at = a.pos.unwrap_or_else(|| clamped(offset, max));
                a.pos = Some(clamped(motion::add2(at, t), max));
                aggs.insert(id, a);
                t != (0, 0) || m.steps > 0
            });
        }
        // Then what was not pulled, clamped where it lands on the base and
        // what was pulled (`PushScrollUpdatesFromMainThread`).
        o.pending.0.retain(|&id, m| {
            let mut a = agg(&mut aggs, id);
            let Some((offset, max, generation)) = a.node else {
                return false;
            };
            jump(&mut a, m, generation);
            let pos = a.pos.unwrap_or_else(|| clamped(offset, max));
            let t = m.total();
            let next = clamped(motion::add2(pos, t), max);
            keep(m, motion::sub2(next, pos));
            a.pos = Some(next);
            aggs.insert(id, a);
            m.total() != (0, 0) || m.steps > 0
        });
        for (&id, a) in &aggs {
            visit();
            let Some((offset, max, _)) = a.node else { continue };
            if a.jumped {
                moves.remove(&id);
            }
            let pos = a.pos.unwrap_or_else(|| clamped(offset, max));
            let delta = motion::sub2(pos, offset);
            if delta != (0, 0) {
                owed.insert(id, delta);
            }
        }
        // A pull emptied here stays until a commit holds it, its sums
        // recycled then.
        self.aggs = aggs;
        // An instance gone from the commit is gone for good: its moves and
        // its scrollbar go with it.
        self.moves.retain(|id, _| {
            visit();
            tables.scroller(*id).is_some()
        });
        self.touched.retain(|id, _| {
            visit();
            tables.scroller(*id).is_some()
        });
        if let Some((rest, node)) = settle
            && rest != (0, 0)
        {
            self.scroll_node(&node, rest, at, false);
        }
    }

    /// A main frame's reach given back: kept for the next, the larger of
    /// it and the one kept.
    pub(super) fn recycle_reach(&mut self, mut reach: Vec<ScrollDelta>) {
        if reach.capacity() > self.reach_spare.capacity() {
            reach.clear();
            self.reach_spare = reach;
        }
    }

    /// The displacement each container owes now, for tests.
    #[cfg(test)]
    pub(super) fn owed(&self, id: ScrollerId) -> (f32, f32) {
        motion::px2(self.owed.get(&id).copied().unwrap_or_default())
    }

    /// The containers whose offset or scrollbar it holds apart from what
    /// was committed: those it owes a displacement, and those whose overlay
    /// scrollbar shows. Any other shows as committed: a frame composes
    /// only these (ADR-0032 I7).
    pub(super) fn live(&self) -> impl Iterator<Item = ScrollerId> + '_ {
        self.owed
            .keys()
            .chain(self.touched.keys().filter(|id| !self.owed.contains_key(id)))
            .copied()
    }
}

/// `m` cut to move `kept` in all (a range that shrank clamps it), each
/// kind cut by the same share on each axis: what survives of the
/// compositor's fling stays synthetic, so a jump still stops it, and what
/// survives of the reader's input stays real (Codex r8 4). Exactly: the
/// reader's share is rounded to a unit and the fling's is the rest, so the
/// two sum to `kept`.
fn keep(m: &mut Moved, kept: Exact) {
    let t = m.total();
    let one = |kept: i64, t: i64, real: &mut i64, synthetic: &mut i64| {
        if t == 0 || kept == t {
            return;
        }
        let k = crate::convert::i64_to_f64(kept) / crate::convert::i64_to_f64(t);
        *real = crate::convert::f64_to_i64((crate::convert::i64_to_f64(*real) * k).round());
        *synthetic = kept.saturating_sub(*real);
    };
    one(kept.0, t.0, &mut m.real.0, &mut m.synthetic.0);
    one(kept.1, t.1, &mut m.real.1, &mut m.synthetic.1);
}

/// Whether a wheel moving `delta` (signed as an offset moves: `along` the
/// inline axis, then down) can move a container at `offset` within `max`:
/// what a wheel that starts a gesture goes to first, walking up from what
/// it is over (chaining past containers at their end). One rule for the
/// UI's route and the compositor's.
pub(crate) fn wheel_moves(pos: Exact, end: Exact, delta: Exact) -> bool {
    (delta.1 > 0 && pos.1 < end.1)
        || (delta.1 < 0 && pos.1 > 0)
        || (delta.0 > 0 && pos.0 < end.0)
        || (delta.0 < 0 && pos.0 > 0)
}

/// Whether a container within `end` takes a wheel moving `delta` (as in
/// [`wheel_moves`]) at all: it scrolls along an axis the wheel moves on,
/// moved or held against its edge, its overlay scrollbar shown either way.
/// A wheel only along an axis it does not scroll is never its. One rule
/// for the UI's route and the compositor's. Exactly, as positions are
/// ([`super::motion`]).
pub(crate) fn wheel_takes(end: Exact, delta: Exact) -> bool {
    (delta.0 != 0 && end.0 > 0) || (delta.1 != 0 && end.1 > 0)
}

/// Whether the compositor may move `node`: never while a modal layer is
/// over it, whatever path the move comes by (a wheel, a gesture's
/// resampling, a fling).
fn movable(node: &ScrollNode) -> bool {
    !node.below_modal
}

/// Whether the platform does not fling a trackpad gesture itself (macOS
/// sends its own momentum events; Windows sends no gesture end).
const PLATFORM_FLINGS_NOT: bool = !cfg!(any(target_os = "macos", target_os = "ios", windows));
