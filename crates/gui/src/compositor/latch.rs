//! The wheel gesture under way, one for both routes a wheel takes: the
//! compositor's, scrolling a container itself, and the UI's, a wheel
//! handler declining and a container scrolled there. Each keeps one with
//! its own target, and the same rules end it.

use super::NodeKey;

/// How far a notched wheel's pointer moves before its turns are a new
/// gesture, logical pixels.
pub const LATCH_SLOP: f32 = 10.0;

/// How long after a notched wheel's turn the next is of the same gesture,
/// seconds (Chrome's mouse wheel latching transaction, 500 ms); a
/// trackpad's gesture is bounded by its phases instead.
pub const WHEEL_LATCH: f32 = 0.5;

/// How soon after a trackpad's fingers lift a gesture that begins is the
/// platform's momentum carrying it on, not fingers down again, seconds.
/// winit gives both phases as one `Started`; Chrome waits as long for
/// momentum after a lift (`kMaximumTimeBetweenPhaseEndedAndMomentumPhaseBegan`).
pub const MOMENTUM_GAP: f32 = 0.1;

/// A wheel event's place in its gesture.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelStep {
    /// Where, logical window pixels.
    pub point: (f32, f32),
    /// When, seconds on the frame clock.
    pub at: f32,
    /// A trackpad's (or a pan's): its phases bound its gesture; else a
    /// notched wheel's, bounded by time and place.
    pub precise: bool,
    /// A trackpad's fingers went down (or its momentum began).
    pub began: bool,
    /// A trackpad's fingers lifted (or its momentum ended).
    pub ended: bool,
}

/// The gesture under way, latched to a `T`: a gesture stays on what took
/// its first event, even once that cannot go on. A trackpad's lasts from
/// its fingers going down to their lifting and the platform's momentum
/// after, however long it rests; a notched wheel's while it turns within
/// [`WHEEL_LATCH`] of its last turn, the pointer not moved past
/// [`LATCH_SLOP`] nor onto another container.
#[derive(Clone, Copy, Debug)]
/// `U` is what tells containers apart: the UI's node key, or the
/// compositor's scroll container instance.
pub(crate) struct WheelGesture<T, U = NodeKey> {
    latch: Option<Latch<T, U>>,
}

impl<T, U> Default for WheelGesture<T, U> {
    fn default() -> Self {
        Self { latch: None }
    }
}

#[derive(Clone, Copy, Debug)]
struct Latch<T, U> {
    to: T,
    last: f32,
    precise: bool,
    point: (f32, f32),
    under: Option<U>,
    /// When a trackpad's fingers lifted, while its momentum may begin.
    lifted: Option<f32>,
    /// The platform's momentum carries the gesture on: its end is the
    /// gesture's, and the next fingers down begin another.
    momentum: bool,
}

impl<T, U: Copy + PartialEq> WheelGesture<T, U> {
    /// Whether `w` begins within [`MOMENTUM_GAP`] of the fingers lifting.
    fn after_lift(&self, w: &WheelStep) -> bool {
        w.precise
            && w.began
            && self
                .latch
                .as_ref()
                .and_then(|l| l.lifted)
                .is_some_and(|t| (0.0..=MOMENTUM_GAP).contains(&(w.at - t)))
    }

    /// Whether `w` begins the platform's momentum after the fingers lifted:
    /// soon after the lift ([`Self::after_lift`]), over the same innermost
    /// container (`under`, as [`Self::held`] takes it). A quick second
    /// flick over another, or over none (or where nothing was hit), is
    /// fingers down again, a gesture of its own.
    fn momentum(&self, w: &WheelStep, under: Option<Option<U>>) -> bool {
        self.after_lift(w)
            && self
                .latch
                .as_ref()
                .is_some_and(|l| matches!(under, Some(Some(u)) if l.under == Some(u)))
    }

    /// What `w` goes on to, if it is of the gesture under way; `under`:
    /// the innermost container under it, or `None` when nothing is.
    pub(crate) fn held(&self, w: &WheelStep, under: Option<Option<U>>) -> Option<&T> {
        let momentum = self.momentum(w, under);
        self.latch
            .as_ref()
            .filter(|l| l.precise == w.precise)
            .filter(|l| {
                if w.precise {
                    // The platform's phases bound a trackpad's gesture.
                    !w.began || momentum
                } else {
                    let (dx, dy) = (w.point.0 - l.point.0, w.point.1 - l.point.1);
                    w.at - l.last <= WHEEL_LATCH
                        && dx * dx + dy * dy <= LATCH_SLOP * LATCH_SLOP
                        && under.is_none_or(|u| u == l.under)
                }
            })
            .map(|l| &l.to)
    }

    /// `w` went to `to`: on in the gesture under way (`held`, as
    /// [`Self::held`] gave it), else the first of a new one over `under`.
    pub(crate) fn went(&mut self, w: &WheelStep, under: Option<U>, to: Option<T>, held: bool) {
        // Only what went on in the gesture under way can be its momentum
        // ([`Self::held`] checked its place).
        let momentum = held && self.after_lift(w);
        let kept = self.latch.take().filter(|_| held);
        self.latch = to.map(|to| {
            let (point, under, momentum) = match kept {
                Some(l) => (l.point, l.under, l.momentum || momentum),
                None => (w.point, under, false),
            };
            Latch {
                to,
                last: w.at,
                precise: w.precise,
                point,
                under,
                // Momentum's own end is the gesture's: none follows it.
                lifted: (w.precise && w.ended && !momentum).then_some(w.at),
                momentum,
            }
        });
    }

    /// No gesture is under way: the next event begins one (what it
    /// latched to can no longer take it).
    pub(crate) fn end(&mut self) {
        self.latch = None;
    }
}
