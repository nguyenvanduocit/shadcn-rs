//! The platform's wheel gesture under way: one object for one gesture,
//! from the first event the platform says begins it to the event that
//! ends it, its momentum included. It holds what the gesture is latched
//! to and when it began (its identity); the routes a wheel takes (the
//! compositor's, scrolling a container itself, and the UI's, a wheel
//! handler declining and a container scrolled there) each keep one, and
//! the same transitions run both. A jump the program makes refuses no
//! gesture: what the reader's fingers move is never dropped, and the rest
//! of a gesture scrolls on from the place jumped to (ADR-0032).
//!
//! A gesture is a state machine of the phases the platform tells
//! ([`WheelPhase`]): fingers down, moving, lifted, the platform's momentum,
//! over. Phases bound a phased gesture, however long it rests; an
//! unphased one (a notched wheel, a platform that tells no phases) is
//! bounded by time and place, as Chrome's wheel latching transaction is.

use super::NodeKey;

/// How far a notched wheel's pointer moves before its turns are a new
/// gesture, logical pixels.
pub const LATCH_SLOP: f32 = 10.0;

/// How long after an unphased wheel's turn the next is of the same
/// gesture, nanoseconds (Chrome's mouse wheel latching transaction,
/// 500 ms); a phased gesture is bounded by its phases instead.
pub const WHEEL_LATCH: u64 = 500_000_000;

/// A wheel event's place in its platform gesture, as the platform tells
/// it (on macOS `NSEvent`'s `phase` and `momentumPhase`, read where the OS
/// hands the event over: winit 0.30 folds both into one `TouchPhase`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WheelPhase {
    /// No phase told: a notched wheel, or a device whose platform tells
    /// none (X11, Windows). Its gesture is bounded by time and place.
    #[default]
    None,
    /// Fingers touched, not moving yet (macOS `MayBegin`, a touch-to-stop):
    /// it ends the gesture before and stops its fling, takes no target, and
    /// is the first contact of the gesture `Began` goes on with, whose start
    /// it is ([`WheelGesture::touch`]). `Cancelled` follows if the fingers
    /// lift without moving.
    Touched,
    /// Fingers went down moving (macOS `Began`, Wayland's first event of a
    /// sequence, a pan's first move): a gesture of its own, always, begun
    /// at the touch before it if one came.
    Began,
    /// The fingers moved.
    Changed,
    /// The fingers lifted (macOS `Ended`, Wayland's axis stop): the
    /// platform's momentum may follow.
    Ended,
    /// The fingers lifted without scrolling (macOS `Cancelled`): no
    /// momentum follows; the gesture is over.
    Cancelled,
    /// The platform's momentum after a lift began (macOS `momentumPhase`
    /// `Began`).
    MomentumBegan,
    /// The platform's momentum carries the gesture on.
    Momentum,
    /// The platform's momentum ended: the gesture is over.
    MomentumEnded,
}

impl WheelPhase {
    /// Whether the platform tells this gesture's phases.
    #[must_use]
    pub fn phased(self) -> bool {
        self != Self::None
    }

    /// Whether it is of the platform's momentum.
    #[must_use]
    pub fn momentum(self) -> bool {
        matches!(self, Self::MomentumBegan | Self::Momentum | Self::MomentumEnded)
    }

    /// Whether the fingers lifted with it (momentum may follow).
    #[must_use]
    pub fn lifted(self) -> bool {
        self == Self::Ended
    }

    /// Whether a wheel event so phased (`precise`: a trackpad's) is of
    /// another gesture than a fling under way, which it then stops, as the
    /// compositor's dispatch stops it ([`super::Compositor::wheel_input`]):
    /// a notched wheel, fingers down, or a trackpad's with no phases (which
    /// never flings). The rest (moves, the lift, the platform's momentum)
    /// go on the flung gesture, and leave its fling be (ADR-0033, "Accepted
    /// motion": a fling handed on stops only for one that stops it here).
    #[must_use]
    pub fn stops_a_fling(self, precise: bool) -> bool {
        !precise || matches!(self, Self::None | Self::Touched | Self::Began)
    }
}

/// A wheel event's place in its gesture.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelStep {
    /// Where, logical window pixels.
    pub point: (f32, f32),
    /// When, nanoseconds on the frame clock ([`crate::clock::nanos`]), as
    /// its source (the platform) told it: compared only with the times of
    /// the same source (a gesture's identity, an unphased gesture's
    /// [`WHEEL_LATCH`]), never to order it against other input.
    pub time: u64,
    /// A trackpad's (or a pan's) pixels, resampled; else a notched wheel's
    /// lines, applied.
    pub precise: bool,
    /// Its phase in its platform gesture.
    pub phase: WheelPhase,
    /// The gesture the compositor found it in (when that began, its
    /// identity: [`Went::began`]), set as it hands the step to the UI. The
    /// compositor owns a platform gesture's life, from where it begins to
    /// where it ends; the UI's latch (a wheel handler that declines, a
    /// container scrolled there) follows it rather than deciding again.
    /// `None` for what the UI alone sees (a pan, the devtools' wheel).
    pub gesture: Option<u64>,
}

impl WheelStep {
    /// When, seconds on the frame clock (what motion is sampled in).
    #[must_use]
    pub fn at(&self) -> f32 {
        crate::clock::seconds_of(self.time)
    }

    /// Whether it is of a gesture its phases bound: told by the platform,
    /// and not a notched wheel's (whose phases are none anywhere).
    fn phased(&self) -> bool {
        self.precise && self.phase.phased()
    }
}

/// Where a gesture is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// Unphased: on while it turns within [`WHEEL_LATCH`] of its last
    /// turn, the pointer not moved past [`LATCH_SLOP`] nor onto another
    /// container.
    Unphased,
    /// Fingers down, moving or resting.
    Fingers,
    /// The fingers lifted: the platform's momentum, if any, comes next
    /// (or the compositor's fling runs, where the platform has none).
    Lifted,
    /// The platform's momentum carries it.
    Momentum,
}

/// The gesture under way, latched to a `T`: a gesture stays on what took
/// its first event, even once that cannot go on (no chaining within a
/// gesture). `U` tells containers apart for an unphased gesture's place:
/// the UI's node key, or the compositor's scroll container instance.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WheelGesture<T, U = NodeKey> {
    latch: Option<Latch<T, U>>,
    /// Fingers that went down over nothing that takes them yet (a touch
    /// with no delta, `MayBegin` or a zero `Began`): the gesture is open,
    /// its target pending, its start kept. Its first move is routed then,
    /// as the same gesture.
    opened: Option<u64>,
}

impl<T, U> Default for WheelGesture<T, U> {
    fn default() -> Self {
        Self {
            latch: None,
            opened: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Latch<T, U> {
    to: T,
    stage: Stage,
    precise: bool,
    /// When its first event came, nanoseconds: its identity.
    began: u64,
    /// When its last event came, nanoseconds.
    last: u64,
    /// Where its first event came (what [`LATCH_SLOP`] is measured from).
    point: (f32, f32),
    /// Where its last event came: a turn at the same place is over the
    /// same container, whatever a commit since put there.
    at: (f32, f32),
    under: Option<U>,
}

/// The gesture a wheel event went on in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Went {
    /// When its gesture began, nanoseconds: the gesture's identity.
    pub began: u64,
    /// The event is the first of its gesture.
    pub first: bool,
}

impl<T, U: Copy + PartialEq> WheelGesture<T, U> {
    /// What `w` goes on to, if it is of the gesture under way; `under`
    /// finds the innermost container under it (`Some(None)` when there is
    /// none, `None` when it is not known), which an unphased gesture's
    /// turns must stay over once the pointer moved: asked only then, so
    /// the rest of a gesture is not hit-tested again, and content that
    /// changed under a pointer at rest (a commit since) keeps the latch.
    pub(crate) fn held(&self, w: &WheelStep, under: impl FnOnce() -> Option<Option<U>>) -> Option<&T> {
        let l = self.latch.as_ref().filter(|l| l.precise == w.precise)?;
        // The compositor decided which gesture it is: the same one, or
        // another begun.
        let on = if let Some(gesture) = w.gesture {
            l.began == gesture
        } else if w.phased() {
            match (l.stage, w.phase) {
                // Fingers down are always a gesture of their own.
                (_, WheelPhase::Began) => false,
                (Stage::Fingers, WheelPhase::Changed | WheelPhase::Ended | WheelPhase::Cancelled) => true,
                // The platform's momentum is the lifted gesture's, over
                // whatever the pointer is over by then.
                (Stage::Lifted | Stage::Momentum, p) => p.momentum(),
                _ => false,
            }
        } else {
            let (dx, dy) = (w.point.0 - l.point.0, w.point.1 - l.point.1);
            l.stage == Stage::Unphased
                && w.time.saturating_sub(l.last) <= WHEEL_LATCH
                && dx * dx + dy * dy <= LATCH_SLOP * LATCH_SLOP
                && (w.point == l.at || under().is_none_or(|u| u == l.under))
        };
        on.then_some(&l.to)
    }

    /// `w` went to `to`: on in the gesture under way (`held`, as
    /// [`Self::held`] gave it), else the first of a new one over `under`.
    /// What it went to, in which gesture; `None` when it went to nothing
    /// (no gesture is under way then).
    pub(crate) fn went(&mut self, w: &WheelStep, under: Option<U>, to: Option<T>, held: bool) -> Option<Went> {
        let kept = self.latch.take().filter(|_| held);
        // A gesture opened with no target goes on to its first move (the
        // fingers' own phases), and no further.
        let opened = self
            .opened
            .take()
            .filter(|_| w.precise && matches!(w.phase, WheelPhase::Began | WheelPhase::Changed | WheelPhase::Ended));
        let Some(to) = to else {
            if w.phased() && w.phase == WheelPhase::Began {
                // The first contact's start, never a later begin's.
                self.opened = Some(w.gesture.or(opened).unwrap_or(w.time));
            } else if opened.is_some() && w.phase == WheelPhase::Changed {
                self.opened = opened;
            }
            return None;
        };
        let stage = |was: Option<Stage>| -> Option<Stage> {
            if !w.phased() {
                return Some(Stage::Unphased);
            }
            Some(match (was, w.phase) {
                (_, WheelPhase::Cancelled | WheelPhase::MomentumEnded) => return None,
                (_, WheelPhase::Ended) => Stage::Lifted,
                (_, p) if p.momentum() => Stage::Momentum,
                _ => Stage::Fingers,
            })
        };
        let (latch, first) = match kept {
            Some(l) => (
                Latch {
                    to,
                    stage: stage(Some(l.stage)).unwrap_or(l.stage),
                    last: w.time,
                    at: w.point,
                    ..l
                },
                false,
            ),
            None => {
                let stage = stage(None).unwrap_or(Stage::Fingers);
                let began = w.gesture.or(opened).unwrap_or(w.time);
                (
                    Latch {
                        to,
                        stage,
                        precise: w.precise,
                        began,
                        last: w.time,
                        point: w.point,
                        at: w.point,
                        under,
                    },
                    true,
                )
            }
        };
        let went = Went {
            began: latch.began,
            first,
        };
        // Its last event: the gesture is over once it is taken.
        if stage(Some(latch.stage)).is_some() {
            self.latch = Some(latch);
        }
        Some(went)
    }

    /// The same gesture, latched to what `to` makes of its target and
    /// telling containers apart by what `under` makes of theirs: none
    /// latched when `to` makes nothing. How a gesture the compositor latched
    /// is handed to the UI's latch (ADR-0033).
    pub(crate) fn map<T2, U2>(
        &self,
        to: impl FnOnce(&T) -> Option<T2>,
        under: impl Fn(U) -> Option<U2>,
    ) -> Option<WheelGesture<T2, U2>> {
        let latch = match self.latch.as_ref() {
            Some(l) => Some(Latch {
                to: to(&l.to)?,
                stage: l.stage,
                precise: l.precise,
                began: l.began,
                last: l.last,
                point: l.point,
                at: l.at,
                under: l.under.and_then(&under),
            }),
            // Fingers down, their target pending: the gesture is open all
            // the same, and goes on as one.
            None => {
                self.opened?;
                None
            }
        };
        Some(WheelGesture {
            latch,
            opened: self.opened,
        })
    }

    /// The gesture it holds, by its identity (when it began, or when its
    /// fingers opened it), and what it is latched to: `None` when it holds
    /// none. Which side holds a gesture is checked by it (ADR-0033: one at
    /// a time), and its motion handed with it by.
    pub(crate) fn holds(&self) -> Option<(u64, Option<&T>)> {
        match self.latch.as_ref() {
            Some(l) => Some((l.began, Some(&l.to))),
            None => self.opened.map(|at| (at, None)),
        }
    }

    /// No gesture is under way: the next event begins one (what it
    /// latched to can no longer take it).
    pub(crate) fn end(&mut self) {
        self.latch = None;
        self.opened = None;
    }

    /// Fingers touched at `at` ([`WheelPhase::Touched`]): the gesture
    /// before is over, and the next begins here, whatever it moves.
    pub(crate) fn touch(&mut self, at: u64) {
        self.latch = None;
        self.opened = Some(at);
    }

    /// Fingers went down moving ([`WheelPhase::Began`]): the gesture
    /// before is over; a touch just before it is this one's start.
    pub(crate) fn begin(&mut self) {
        self.latch = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(time: u64, phase: WheelPhase) -> WheelStep {
        WheelStep {
            point: (0.0, 0.0),
            time,
            precise: true,
            phase,
            gesture: None,
        }
    }

    /// `w` over `over`: what it went to, and in which gesture.
    fn feed(g: &mut WheelGesture<u8, u8>, w: &WheelStep, over: u8, _: u64) -> (u8, Went) {
        let held = g.held(w, || Some(Some(over))).copied();
        let to = held.unwrap_or(over);
        let went = g.went(w, Some(over), Some(to), held.is_some());
        let none = Went { began: 0, first: false };
        (to, went.unwrap_or(none))
    }

    /// The platform's momentum is the lifted gesture's, over whatever the
    /// pointer is over by then; fingers down again are always new.
    #[test]
    fn momentum_carries_the_lifted_gesture_and_fingers_down_begin_another() {
        let mut g = WheelGesture::<u8, u8>::default();
        let (_, a) = feed(&mut g, &step(1, WheelPhase::Began), 1, 0);
        assert!(a.first);
        feed(&mut g, &step(2, WheelPhase::Ended), 1, 0);
        let (to, m) = feed(&mut g, &step(3, WheelPhase::MomentumBegan), 2, 0);
        assert_eq!((to, m.first), (1, false), "momentum stays latched");
        let (to, n) = feed(&mut g, &step(4, WheelPhase::Began), 2, 0);
        assert_eq!(
            (to, n.first, n.began),
            (2, true, 4),
            "fingers down are a gesture of their own"
        );
    }

    /// A rest is not an end: fingers resting long, then moving, are the
    /// same gesture.
    #[test]
    fn a_phased_gesture_never_ends_by_resting() {
        let mut g = WheelGesture::<u8, u8>::default();
        feed(&mut g, &step(1, WheelPhase::Began), 1, 0);
        let (_, w) = feed(&mut g, &step(10 * WHEEL_LATCH, WheelPhase::Changed), 1, 0);
        assert!(!w.first, "still the same gesture after a long rest");
    }

    /// A latch handed the compositor's gesture follows it: a step of the
    /// gesture the compositor says goes on is held, by whatever its own
    /// rules would say (here past the unphased latch's time, and over
    /// another container); one of another gesture is not, however soon.
    /// Its identity is the compositor's, so a jump is ordered against the
    /// same start on both routes.
    #[test]
    fn a_latch_follows_the_compositors_gesture() {
        let mut g = WheelGesture::<u8, u8>::default();
        let of = |time, gesture| WheelStep {
            time,
            gesture: Some(gesture),
            ..WheelStep::default()
        };
        let (_, w) = feed(&mut g, &of(7, 3), 1, 0);
        assert_eq!(w.began, 3, "the compositor's start");
        let (to, w) = feed(&mut g, &of(7 + 2 * WHEEL_LATCH, 3), 2, 0);
        assert!(!w.first && to == 1, "the compositor's gesture goes on");
        let (to, w) = feed(&mut g, &of(8 + 2 * WHEEL_LATCH, 9), 1, 0);
        assert!(w.first && to == 1 && w.began == 9, "another of its own");
    }
}
