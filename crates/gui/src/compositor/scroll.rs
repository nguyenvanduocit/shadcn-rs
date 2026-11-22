//! Scroll on the render thread (ADR-0015): offsets as synced properties
//! (cc's `SyncedProperty`: the compositor's offset is the UI's committed
//! one plus what it scrolled and the UI has not yet taken in), wheel
//! gestures resampled at each frame's present time (Chromium's
//! `ScrollPredictor`, linear), and flings where the platform has none.

use foldhash::HashMap;

use super::{NodeKey, ScrollDelta, ScrollNode};

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
    target: NodeKey,
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

/// Every scroll container's synced offset, and the gesture under way.
pub(super) struct Scrolls {
    by_key: HashMap<NodeKey, Synced>,
    gesture: Option<Gesture>,
    /// What the compositor moved each container by lately, when: its speed.
    moves: HashMap<NodeKey, Vec<Moved>>,
    /// Whether a trackpad gesture is flung on after the fingers lift: the
    /// platform sends no momentum of its own.
    flings: bool,
}

impl Default for Scrolls {
    fn default() -> Self {
        Self {
            by_key: HashMap::default(),
            gesture: None,
            moves: HashMap::default(),
            flings: PLATFORM_FLINGS_NOT,
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
        if let Some(s) = self.by_key.get(&node.key) {
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

    /// Scroll `node` by the wheel's `delta` at `at`; how far it moved, in offsets.
    fn scroll_node(&mut self, node: &ScrollNode, delta: (f32, f32), at: f32) -> (f32, f32) {
        let along = if node.rtl { -delta.0 } else { delta.0 };
        let was = self.offset(node, 0);
        let next = (
            (was.0 + along).clamp(0.0, node.max.0),
            (was.1 + delta.1).clamp(0.0, node.max.1),
        );
        let moved = (next.0 - was.0, next.1 - was.1);
        if moved != (0.0, 0.0) {
            let s = self.by_key.entry(node.key).or_insert_with(|| Synced {
                generation: node.generation,
                ..Synced::default()
            });
            s.pending = (s.pending.0 + moved.0, s.pending.1 + moved.1);
            let m = self.moves.entry(node.key).or_default();
            m.retain(|(t, _)| at - *t < SPEED_WINDOW);
            m.push((at, moved));
        }
        moved
    }

    /// How fast the compositor scrolls `key` at `at`, offset pixels a
    /// second: a fling's own speed, else what it moved over the last
    /// [`SPEED_WINDOW`], from its first event there to its last.
    fn speed(&self, key: NodeKey, at: f32) -> (f32, f32) {
        if let Some(g) = &self.gesture
            && g.target == key
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
    /// carries it on where the platform has none of its own.
    pub(super) fn gesture(
        &mut self,
        scrollers: &[ScrollNode],
        target: Option<WheelTarget>,
        delta: (f32, f32),
        at: f32,
        ended: bool,
    ) {
        let node = target
            .and_then(|t| usize::try_from(t.0).ok())
            .and_then(|i| scrollers.get(i));
        if let Some(node) = node {
            let fresh = self.gesture.as_ref().is_none_or(|g| {
                g.target != node.key || g.fling.is_some() || g.predictor.last().is_none_or(|l| at - l > 0.25)
            });
            if fresh {
                self.gesture = Some(Gesture {
                    target: node.key,
                    predictor: ScrollPredictor::default(),
                    recent: Vec::new(),
                    applied: (0.0, 0.0),
                    fling: None,
                });
            }
        }
        let Some(g) = &mut self.gesture else { return };
        if delta != (0.0, 0.0) && node.is_some() {
            g.predictor.push(at, delta);
            g.recent.retain(|(t, _)| at - *t <= FLING_WINDOW);
            g.recent.push((at, delta));
        }
        if ended && self.flings && g.fling.is_none() {
            g.fling = Fling::new(at, g.lift_velocity(at)).map(|f| (f, g.predictor.at(at + PREDICT_FOR)));
        }
    }

    /// Move the gesture's scroll container to where its events, resampled
    /// at `at`, or its fling put it.
    pub(super) fn tick(&mut self, scrollers: &[ScrollNode], at: f32) {
        let Some(mut g) = self.gesture.take() else { return };
        let Some(node) = scrollers.iter().find(|s| s.key == g.target) else {
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
        self.gesture.as_ref().is_some_and(|g| match &g.fling {
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
            let speed = self.speed(node.key, at);
            let reach = (speed.0 * horizon, speed.1 * horizon);
            let pending = self.by_key.get(&node.key).map_or((0.0, 0.0), |s| s.pending);
            if pending == (0.0, 0.0) && reach == (0.0, 0.0) {
                continue;
            }
            out.push(ScrollDelta {
                key: node.key,
                delta: pending,
                reach,
            });
            if let Some(s) = self.by_key.get_mut(&node.key)
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
        let mut keep: HashMap<NodeKey, Synced> = HashMap::default();
        for node in scrollers {
            if let Some(mut s) = self.by_key.remove(&node.key) {
                s.sent.retain(|(seq, _)| *seq > answers);
                if s.generation != node.generation {
                    s = Synced {
                        generation: node.generation,
                        ..Synced::default()
                    };
                }
                if s.pending != (0.0, 0.0) || !s.sent.is_empty() {
                    keep.insert(node.key, s);
                }
            }
        }
        self.by_key = keep;
    }
}

/// Whether the platform does not fling a trackpad gesture itself (macOS
/// sends its own momentum events; Windows sends no gesture end).
const PLATFORM_FLINGS_NOT: bool = !cfg!(any(target_os = "macos", target_os = "ios", windows));
