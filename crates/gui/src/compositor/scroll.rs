//! Scroll on the render thread (ADR-0015): offsets as synced properties
//! (cc's `SyncedProperty`: the compositor's offset is the UI's committed
//! one plus what it scrolled and the UI has not yet taken in), wheel
//! gestures resampled at each frame's present time (Chromium's
//! `ScrollPredictor`, linear), and flings where the platform has none.

use foldhash::HashMap;

use super::{ScrollDelta, ScrollNode};

/// A scroll container a wheel scrolls, by index in the scene's scrollers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WheelTarget(pub u32);

/// How long past its last event a gesture is still predicted
/// (Chromium's `kScrollPredictorMaxResampleTime`), seconds.
const PREDICT_FOR: f32 = 0.035;
/// The furthest a prediction reaches past the last event (Chromium's
/// `linear_resampling` `kResampleMaxPrediction`), seconds.
const MAX_PREDICTION: f32 = 0.008;
/// Events closer than this are not predicted from (`kResampleMinDelta`), seconds.
const MIN_DELTA: f32 = 0.002;
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

/// One scroll container's synced offset.
#[derive(Clone, Debug, Default)]
struct Synced {
    generation: u64,
    /// Scrolled since the last main frame was sent.
    pending: (f32, f32),
    /// Sent with main frames the UI has not answered: (sequence, delta).
    sent: Vec<(u64, (f32, f32))>,
}

/// Where a wheel gesture's events, summed, put the content: the samples a
/// prediction is made from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScrollPredictor {
    /// (seconds on the frame clock, the gesture's total so far).
    samples: Vec<(f32, (f32, f32))>,
}

impl ScrollPredictor {
    /// The gesture moved by `delta` at `at`.
    pub fn push(&mut self, at: f32, delta: (f32, f32)) {
        let total = self.samples.last().map_or((0.0, 0.0), |s| s.1);
        let total = (total.0 + delta.0, total.1 + delta.1);
        // Two events in the same instant are one.
        if let Some(last) = self.samples.last_mut()
            && (at - last.0).abs() < 1e-6
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
    pub fn at(&self, at: f32) -> (f32, f32) {
        let Some(&(t1, p1)) = self.samples.last() else {
            return (0.0, 0.0);
        };
        if at >= t1 + PREDICT_FOR {
            return p1;
        }
        // Between two events: where the content was then.
        if let Some(w) = self
            .samples
            .windows(2)
            .find(|w| matches!(w, [a, b] if a.0 <= at && at <= b.0))
            && let [(ta, pa), (tb, pb)] = w
        {
            let k = if tb > ta { (at - ta) / (tb - ta) } else { 1.0 };
            return (pa.0 + (pb.0 - pa.0) * k, pa.1 + (pb.1 - pa.1) * k);
        }
        let Some(&(t0, p0)) = self.samples.len().checked_sub(2).and_then(|i| self.samples.get(i)) else {
            return p1;
        };
        let gap = t1 - t0;
        if gap < MIN_DELTA || at <= t1 {
            return p1;
        }
        let ahead = (at - t1).min(MAX_PREDICTION).min(gap / 2.0);
        let v = ((p1.0 - p0.0) / gap, (p1.1 - p0.1) / gap);
        (p1.0 + v.0 * ahead, p1.1 + v.1 * ahead)
    }

    /// The gesture's speed at its end, pixels a second, from its last events.
    #[must_use]
    pub fn velocity(&self) -> (f32, f32) {
        let n = self.samples.len();
        let (Some(&(t0, p0)), Some(&(t1, p1))) = (self.samples.get(n.saturating_sub(3)), self.samples.last()) else {
            return (0.0, 0.0);
        };
        let dt = t1 - t0;
        if dt <= 0.0 {
            return (0.0, 0.0);
        }
        ((p1.0 - p0.0) / dt, (p1.1 - p0.1) / dt)
    }

    /// When the last event came.
    #[must_use]
    pub fn last(&self) -> Option<f32> {
        self.samples.last().map(|s| s.0)
    }
}

/// A fling: the content slides on after the fingers lift, slowing
/// exponentially (a time constant of 325 ms, the decay of Android's and
/// iOS's scroll views at their normal rate).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fling {
    start: f32,
    velocity: (f32, f32),
}

/// A fling's time constant, seconds.
const FLING_TAU: f32 = 0.325;
/// Slower than this (pixels a second) a fling ends, or never starts.
const FLING_MIN: f32 = 50.0;

impl Fling {
    /// A fling from `velocity` (pixels a second) at `start`; `None` when too slow.
    #[must_use]
    pub fn new(start: f32, velocity: (f32, f32)) -> Option<Self> {
        (velocity.0.hypot(velocity.1) >= FLING_MIN).then_some(Self { start, velocity })
    }

    /// How far it has carried the content by `at`.
    #[must_use]
    pub fn at(&self, at: f32) -> (f32, f32) {
        let t = (at - self.start).max(0.0);
        let k = FLING_TAU * (1.0 - (-t / FLING_TAU).exp());
        (self.velocity.0 * k, self.velocity.1 * k)
    }

    /// How fast it carries the content at `at`, pixels a second.
    #[must_use]
    pub fn velocity_at(&self, at: f32) -> (f32, f32) {
        let k = (-(at - self.start).max(0.0) / FLING_TAU).exp();
        (self.velocity.0 * k, self.velocity.1 * k)
    }

    /// Whether it still moves at `at`.
    #[must_use]
    pub fn moving(&self, at: f32) -> bool {
        let t = (at - self.start).max(0.0);
        self.velocity.0.hypot(self.velocity.1) * (-t / FLING_TAU).exp() >= FLING_MIN
    }
}

/// A precise wheel gesture (a trackpad's) the compositor resamples.
#[derive(Clone, Debug)]
struct Gesture {
    /// The scroll container's instance ([`ScrollNode::instance`]).
    target: u64,
    predictor: ScrollPredictor,
    /// Its events of the last [`FLING_WINDOW`]: (when, delta).
    recent: Vec<(f32, (f32, f32))>,
    /// How much of it the scroll container was moved by.
    applied: (f32, f32),
    fling: Option<(Fling, (f32, f32))>,
}

/// When the compositor moved a container (seconds on the frame clock), and by how much.
type Moved = (f32, (f32, f32));

impl Gesture {
    /// Its speed when the fingers lifted at `at`, pixels a second: what it
    /// moved from its first event within [`FLING_WINDOW`] of `at`, over the
    /// time from that event to `at`.
    fn lift_velocity(&self, at: f32) -> (f32, f32) {
        let mut recent = self.recent.iter().filter(|(t, _)| at - *t <= FLING_WINDOW);
        let Some(&(first, d)) = recent.next() else {
            return (0.0, 0.0);
        };
        let span = at - first;
        if span <= 0.0 {
            return (0.0, 0.0);
        }
        let sum = recent.fold(d, |a, (_, d)| (a.0 + d.0, a.1 + d.1));
        (sum.0 / span, sum.1 / span)
    }
}

/// A scroll container's identity on this side: the instance paint gave it
/// ([`ScrollNode::instance`]), new for another component instance at its
/// path and for one that left the tables and came back. Every piece of
/// per-container state (what is owed, its moves, its scrollbar, its
/// gesture) is the instance's, so none outlives it.
pub(super) type ScrollerId = u64;

/// `node`'s identity.
pub(super) fn id_of(node: &ScrollNode) -> ScrollerId {
    node.instance
}

/// Every scroll container's synced offset, and the gesture under way, each
/// by the container's identity ([`ScrollerId`]).
pub(super) struct Scrolls {
    by_key: HashMap<ScrollerId, Synced>,
    gesture: Option<Gesture>,
    /// What the compositor moved each container by lately, when: its speed.
    moves: HashMap<ScrollerId, Vec<Moved>>,
    /// Whether a trackpad gesture is flung on after the fingers lift: the
    /// platform sends no momentum of its own.
    flings: bool,
    /// When the user last scrolled each container: its overlay scrollbar
    /// shows from then, and fades. A program's scroll shows none.
    touched: HashMap<ScrollerId, f32>,
    /// The instances the last commit showed ([`Self::committed`]'s, kept
    /// so a commit allocates nothing).
    shown: foldhash::HashSet<ScrollerId>,
}

impl Default for Scrolls {
    fn default() -> Self {
        Self {
            by_key: HashMap::default(),
            gesture: None,
            moves: HashMap::default(),
            flings: PLATFORM_FLINGS_NOT,
            touched: HashMap::default(),
            shown: foldhash::HashSet::default(),
        }
    }
}

impl Scrolls {
    /// Fling trackpad gestures after they end (`true`), or leave it to the
    /// platform's momentum events; by default as the platform built for
    /// does. A replayed input log says what its platform did.
    pub(super) fn set_flings(&mut self, flings: bool) {
        self.flings = flings;
    }

    /// The offset shown for `node` (at `index` among the scrollers).
    pub(super) fn offset(&self, node: &ScrollNode, _index: u32) -> (f32, f32) {
        let mut o = node.offset;
        if let Some(s) = self.by_key.get(&id_of(node)) {
            for (_, d) in &s.sent {
                o = (o.0 + d.0, o.1 + d.1);
            }
            o = (o.0 + s.pending.0, o.1 + s.pending.1);
        }
        (o.0.clamp(0.0, node.max.0), o.1.clamp(0.0, node.max.1))
    }

    /// Scroll `target` by `delta` (the wheel's: positive moves the content
    /// up and toward the inline start); whether it moved.
    /// `at` is when, seconds on the frame clock.
    pub(super) fn scroll(&mut self, scrollers: &[ScrollNode], target: WheelTarget, delta: (f32, f32), at: f32) -> bool {
        let Some(node) = usize::try_from(target.0).ok().and_then(|i| scrollers.get(i)) else {
            return false;
        };
        self.scroll_node(node, delta, at) != (0.0, 0.0)
    }

    /// Scroll `node` by the wheel's `delta` at `at`; how far it moved, in
    /// offsets. Every move the compositor makes comes here, so here it
    /// keeps its one rule: a container below a modal layer is not moved.
    fn scroll_node(&mut self, node: &ScrollNode, delta: (f32, f32), at: f32) -> (f32, f32) {
        if !movable(node) {
            return (0.0, 0.0);
        }
        let along = if node.rtl { -delta.0 } else { delta.0 };
        let was = self.offset(node, 0);
        let next = (
            (was.0 + along).clamp(0.0, node.max.0),
            (was.1 + delta.1).clamp(0.0, node.max.1),
        );
        let moved = (next.0 - was.0, next.1 - was.1);
        if moved != (0.0, 0.0) {
            let s = self.by_key.entry(id_of(node)).or_insert_with(|| Synced {
                generation: node.generation,
                ..Synced::default()
            });
            s.pending = (s.pending.0 + moved.0, s.pending.1 + moved.1);
            let m = self.moves.entry(id_of(node)).or_default();
            m.retain(|(t, _)| at - *t < SPEED_WINDOW);
            m.push((at, moved));
            self.touched.insert(id_of(node), at);
        }
        moved
    }

    /// The opacity of `node`'s overlay scrollbar at `at`: 1 while the user
    /// scrolls it and for [`THUMB_SHOWN`] after, then down to 0 over
    /// [`THUMB_FADE`]; 0 at rest.
    pub(super) fn thumb_opacity(&self, node: &ScrollNode, at: f32) -> f32 {
        self.touched.get(&id_of(node)).map_or(0.0, |&t| {
            let idle = at - t - THUMB_SHOWN;
            if idle <= 0.0 {
                1.0
            } else {
                (1.0 - idle / THUMB_FADE).max(0.0)
            }
        })
    }

    /// Whether an overlay scrollbar is still shown or fading at `at`.
    fn thumbs_shown(&self, at: f32) -> bool {
        self.touched.values().any(|&t| at < t + THUMB_SHOWN + THUMB_FADE)
    }

    /// How fast the compositor scrolls `key` at `at`, offset pixels a
    /// second: a fling's own speed, else what it moved over the last
    /// [`SPEED_WINDOW`], from its first event there to its last.
    fn speed(&self, node: &ScrollNode, at: f32) -> (f32, f32) {
        let key = id_of(node);
        if let Some(g) = &self.gesture
            && g.target == node.instance
            && let Some((f, _)) = &g.fling
            && f.moving(at)
        {
            return f.velocity_at(at);
        }
        let Some(m) = self.moves.get(&key) else {
            return (0.0, 0.0);
        };
        let recent: Vec<&Moved> = m.iter().filter(|(t, _)| at - *t < SPEED_WINDOW).collect();
        // What moved after the first event, over the time since it.
        let (Some(first), Some(last)) = (recent.first(), recent.last()) else {
            return (0.0, 0.0);
        };
        let span = last.0 - first.0;
        if span <= 0.0 {
            return (0.0, 0.0);
        }
        let sum = recent
            .iter()
            .skip(1)
            .fold((0.0, 0.0), |a, (_, d)| (a.0 + d.0, a.1 + d.1));
        (sum.0 / span, sum.1 / span)
    }

    /// A precise wheel event of a gesture on `target` at `at`: resampled at
    /// each frame ([`Self::tick`]). `ended`: the fingers lifted, and a fling
    /// carries it on where the platform has none of its own. Whether it
    /// went to a container: none is below a modal layer.
    pub(super) fn gesture(
        &mut self,
        scrollers: &[ScrollNode],
        target: Option<WheelTarget>,
        delta: (f32, f32),
        at: f32,
        ended: bool,
    ) -> bool {
        let node = target
            .and_then(|t| usize::try_from(t.0).ok())
            .and_then(|i| scrollers.get(i))
            .filter(|n| movable(n));
        // An event no container takes moves nothing: its end flings
        // nothing either, whatever gesture ran before it.
        let Some(node) = node else { return false };
        let fresh = self.gesture.as_ref().is_none_or(|g| {
            g.target != node.instance || g.fling.is_some() || g.predictor.last().is_none_or(|l| at - l > 0.25)
        });
        let g = match &mut self.gesture {
            Some(g) if !fresh => g,
            slot => slot.insert(Gesture {
                target: node.instance,
                predictor: ScrollPredictor::default(),
                recent: Vec::new(),
                applied: (0.0, 0.0),
                fling: None,
            }),
        };
        if delta != (0.0, 0.0) {
            g.predictor.push(at, delta);
            g.recent.retain(|(t, _)| at - *t <= FLING_WINDOW);
            g.recent.push((at, delta));
        }
        // Flung from this gesture's own samples: its container took the end.
        if ended && self.flings && g.fling.is_none() {
            g.fling = Fling::new(at, g.lift_velocity(at)).map(|f| (f, g.predictor.at(at + PREDICT_FOR)));
        }
        true
    }

    /// Move the gesture's scroll container to where its events, resampled
    /// at `at`, or its fling put it.
    pub(super) fn tick(&mut self, scrollers: &[ScrollNode], at: f32) {
        self.touched.retain(|_, t| at < *t + THUMB_SHOWN + THUMB_FADE);
        let Some(mut g) = self.gesture.take() else { return };
        // Gone, or below a modal layer opened since: its fling stops.
        let Some(node) = scrollers.iter().find(|s| s.instance == g.target).filter(|n| movable(n)) else {
            return;
        };
        let want = match &g.fling {
            Some((f, base)) => {
                let d = f.at(at);
                (base.0 + d.0, base.1 + d.1)
            }
            None => g.predictor.at(at),
        };
        let step = (want.0 - g.applied.0, want.1 - g.applied.1);
        if step != (0.0, 0.0) {
            // What an edge stops is not owed later.
            self.scroll_node(node, step, at);
        }
        g.applied = want;
        let kept = match &g.fling {
            Some((f, _)) => f.moving(at),
            // Past its prediction it moves no more, but the fingers may
            // still lift and fling it, as fast as its last events went.
            None => g
                .predictor
                .last()
                .is_some_and(|l| at < l + PREDICT_FOR || (self.flings && at < l + FLING_WINDOW)),
        };
        if kept {
            self.gesture = Some(g);
        }
    }

    /// Whether it scrolled anything no main frame has taken yet.
    pub(super) fn pending(&self) -> bool {
        self.by_key.values().any(|s| s.pending != (0.0, 0.0))
    }

    /// Whether a gesture is still resampled or a fling moves at `at`.
    pub(super) fn animating(&self, at: f32) -> bool {
        self.thumbs_shown(at)
            || self.gesture.as_ref().is_some_and(|g| match &g.fling {
                Some((f, _)) => f.moving(at),
                None => g.predictor.last().is_some_and(|l| at < l + PREDICT_FOR),
            })
    }

    /// Hand what was scrolled since to the main frame `sequence`, granted
    /// at `at`, with how far each container moving may be scrolled within
    /// `horizon` seconds (until the commit after the one it answers shows).
    pub(super) fn send(&mut self, scrollers: &[ScrollNode], sequence: u64, at: f32, horizon: f32) -> Vec<ScrollDelta> {
        self.moves.retain(|_, m| {
            m.retain(|(t, _)| at - *t < SPEED_WINDOW);
            !m.is_empty()
        });
        let mut out = Vec::new();
        for node in scrollers {
            let speed = self.speed(node, at);
            let reach = (speed.0 * horizon, speed.1 * horizon);
            let synced = self.by_key.get(&id_of(node));
            let pending = synced.map_or((0.0, 0.0), |s| s.pending);
            if pending == (0.0, 0.0) && reach == (0.0, 0.0) {
                continue;
            }
            out.push(ScrollDelta {
                key: node.key,
                owner: node.owner,
                delta: pending,
                generation: synced.map_or(node.generation, |s| s.generation),
                reach,
            });
            if let Some(s) = self.by_key.get_mut(&id_of(node))
                && pending != (0.0, 0.0)
            {
                s.sent.push((sequence, pending));
                s.pending = (0.0, 0.0);
            }
        }
        out
    }

    /// A commit answering main frame `answers` came with `scrollers`: what it
    /// took in is dropped from what is owed, and a container the program
    /// scrolled since forgets what the compositor did before.
    pub(super) fn committed(&mut self, scrollers: &[ScrollNode], answers: u64) {
        // A gesture is its container's, the one shown: gone from the
        // commit (even if a later one brings back its key), or covered by
        // a modal layer, it stops at once, its fling too.
        if self
            .gesture
            .as_ref()
            .is_some_and(|g| !scrollers.iter().any(|s| s.instance == g.target && movable(s)))
        {
            self.gesture = None;
        }
        // Nothing held for any container (scrollers at rest): nothing to do.
        if self.by_key.is_empty() && self.moves.is_empty() && self.touched.is_empty() {
            return;
        }
        // An instance gone from the commit is gone for good (one coming
        // back is another): what is owed it, its moves and its scrollbar go
        // with it. The set of those shown is one kept, refilled.
        self.shown.clear();
        self.shown.extend(scrollers.iter().map(id_of));
        let shown = &self.shown;
        self.moves.retain(|id, _| shown.contains(id));
        self.touched.retain(|id, _| shown.contains(id));
        self.by_key.retain(|id, _| shown.contains(id));
        for node in scrollers {
            if let Some(s) = self.by_key.get_mut(&id_of(node)) {
                s.sent.retain(|(seq, _)| *seq > answers);
                if s.generation != node.generation {
                    *s = Synced {
                        generation: node.generation,
                        ..Synced::default()
                    };
                }
            }
        }
        self.by_key.retain(|_, s| s.pending != (0.0, 0.0) || !s.sent.is_empty());
    }
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
