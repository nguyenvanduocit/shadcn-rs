//! Tap and long-press recognition (ADR-0024), one state machine per
//! contact, owned by that contact's pointer, for touch and pen contacts:
//! the input layer knows when a press is a tap, so nothing waits for a
//! browser's `click`. A mouse does not go
//! through it: its press is a click wherever it is released on the same
//! node, as Chrome's `click` is (no slop), it dismisses a layer as it goes
//! down, and it has a secondary button where a contact has a long press.
//!
//! - A press is a **tap** when it is released having moved no further than
//!   [`SLOP`] from where it went down, and it was not a long press.
//! - A touch or pen press held [`LONG_PRESS`] without moving further than
//!   [`SLOP`] is a **long press** (reka's context menu's `pressOpenDelay`);
//!   it is then no tap. A mouse has its secondary button instead.
//! - A press that moves further than [`SLOP`] is a **pan** (a scroll or a
//!   drag) from then on: neither a tap nor a long press.
//! - A contact that goes down while another is down makes neither a tap
//!   nor a long press, nor does that other (Chrome's `CancelTaps` on a
//!   second pointer); each may still pan.
//! - Moves of no more than [`NOISE`] are not moves (reka's body scroll
//!   lock's threshold, so a still finger's jitter keeps a tap).

use std::time::Duration;

use crate::layer::PointerKind;

/// How long a touch or pen press is held to be a long press
/// (`ContextMenu/ContextMenuRoot.vue:41`).
pub const LONG_PRESS: Duration = Duration::from_millis(700);

/// How far a press may move and stay a tap or a long press, in logical
/// pixels: the distance after which reka's NumberField stops repeating a
/// held touch or pen press (`NumberField/utils.ts:83-93`).
pub const SLOP: f32 = 10.0;

/// Moves this small are not moves (`shared/useBodyScrollLock.ts:114`).
pub const NOISE: f32 = 0.01;

/// What a press is so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Down, within the slop, not yet long.
    Pressed,
    /// Held long enough: a long press, already told.
    Long,
    /// Moved past the slop.
    Pan,
}

/// One contact's press being recognized: each pointer down owns its own
/// (Chrome's per-pointer gesture detection).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    pub kind: PointerKind,
    /// Where it went down, window pixels.
    pub start: (f32, f32),
    /// Where it is now.
    pub at: (f32, f32),
    /// When it went down, on the frame clock.
    pub since: Duration,
    pub phase: Phase,
    /// How long it is held to be long ([`LONG_PRESS`] unless the node
    /// below asks otherwise).
    pub long: Duration,
    /// It may still be a tap or a long press: not once another contact
    /// went down while it was down (Chrome's `CancelTaps` on a second
    /// pointer), which leaves it a pan at most.
    pub single: bool,
}

/// What a step of a press turned out to be.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    /// Released as a tap, at its start.
    Tap { x: f32, y: f32 },
    /// Held long enough, at its start.
    LongPress { x: f32, y: f32 },
    /// It began to pan: moves from here on scroll or drag.
    PanStart,
    /// Released after a pan or a long press: nothing more.
    Done,
}

impl Press {
    /// A contact of `kind` went down at `(x, y)` at `now`.
    #[must_use]
    pub fn down(kind: PointerKind, x: f32, y: f32, now: Duration) -> Self {
        Self {
            kind,
            start: (x, y),
            at: (x, y),
            since: now,
            phase: Phase::Pressed,
            long: LONG_PRESS,
            single: true,
        }
    }

    /// It is long after `after` (a node's own delay).
    pub fn long_after(&mut self, after: Duration) {
        self.long = after;
    }

    /// Another contact went down while it is down: it is no tap and no
    /// long press any more; it may still pan.
    pub fn not_single(&mut self) {
        self.single = false;
    }

    /// It moved to `(x, y)`; `PanStart` when this move takes it past the slop.
    pub fn moved(&mut self, x: f32, y: f32) -> Option<Gesture> {
        let (dx, dy) = (x - self.at.0, y - self.at.1);
        if dx.abs() <= NOISE && dy.abs() <= NOISE {
            return None;
        }
        self.at = (x, y);
        let far = (x - self.start.0).hypot(y - self.start.1) > SLOP;
        if far && self.phase != Phase::Pan {
            self.phase = Phase::Pan;
            return Some(Gesture::PanStart);
        }
        None
    }

    /// It went up at `(x, y)`: a tap, or done.
    pub fn up(&mut self, x: f32, y: f32) -> Gesture {
        self.moved(x, y);
        match self.phase {
            Phase::Pressed if self.single => Gesture::Tap {
                x: self.start.0,
                y: self.start.1,
            },
            Phase::Pressed | Phase::Long | Phase::Pan => Gesture::Done,
        }
    }

    /// Its long press, when it is held long enough by `now` and was not
    /// told yet: told once.
    pub fn due(&mut self, now: Duration) -> Option<Gesture> {
        let long = self.waiting() && now.saturating_sub(self.since) >= self.long;
        long.then(|| {
            self.phase = Phase::Long;
            Gesture::LongPress {
                x: self.start.0,
                y: self.start.1,
            }
        })
    }

    /// Whether it may still become a long press: the frame clock must wake
    /// for it. A mouse has its secondary button instead.
    #[must_use]
    pub fn waiting(&self) -> bool {
        self.single && self.phase == Phase::Pressed && matches!(self.kind, PointerKind::Touch | PointerKind::Pen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: Duration = Duration::ZERO;

    #[test]
    fn a_press_released_within_the_slop_is_a_tap_at_its_start() {
        let mut p = Press::down(PointerKind::Touch, 100.0, 100.0, T0);
        assert_eq!(p.moved(106.0, 108.0), None, "10 px away is still a tap");
        assert_eq!(p.up(106.0, 108.0), Gesture::Tap { x: 100.0, y: 100.0 });
    }

    #[test]
    fn a_press_moved_past_the_slop_pans_and_is_no_tap() {
        let mut p = Press::down(PointerKind::Touch, 100.0, 100.0, T0);
        assert_eq!(p.moved(100.0, 110.5), Some(Gesture::PanStart));
        assert_eq!(p.moved(100.0, 100.0), None, "a pan stays a pan back at the start");
        assert_eq!(p.due(LONG_PRESS), None);
        assert_eq!(p.up(100.0, 100.0), Gesture::Done);
    }

    #[test]
    fn a_touch_held_700_ms_is_a_long_press_once_and_then_no_tap() {
        let mut p = Press::down(PointerKind::Touch, 10.0, 20.0, T0);
        assert_eq!(p.due(Duration::from_millis(699)), None);
        assert!(p.waiting());
        assert_eq!(p.due(LONG_PRESS), Some(Gesture::LongPress { x: 10.0, y: 20.0 }));
        assert_eq!(p.due(Duration::from_secs(2)), None, "told once");
        assert_eq!(p.up(10.0, 20.0), Gesture::Done);
    }

    #[test]
    fn a_mouse_press_is_never_long_and_jitter_is_no_move() {
        let mut mouse = Press::down(PointerKind::Mouse, 0.0, 0.0, T0);
        assert!(!mouse.waiting());
        assert_eq!(mouse.due(Duration::from_secs(5)), None);
        let mut pen = Press::down(PointerKind::Pen, 0.0, 0.0, T0);
        assert_eq!(pen.moved(0.005, 0.0), None);
        assert_eq!(pen.at, (0.0, 0.0), "under the noise it did not move");
        assert_eq!(mouse.up(3.0, 4.0), Gesture::Tap { x: 0.0, y: 0.0 });
    }

    #[test]
    fn a_contact_down_beside_another_is_no_tap_nor_long_press_but_still_pans() {
        let mut p = Press::down(PointerKind::Touch, 0.0, 0.0, T0);
        p.not_single();
        assert!(!p.waiting());
        assert_eq!(p.due(LONG_PRESS), None);
        assert_eq!(p.up(0.0, 0.0), Gesture::Done, "no tap");
        let mut q = Press::down(PointerKind::Touch, 0.0, 0.0, T0);
        q.not_single();
        assert_eq!(q.moved(0.0, 20.0), Some(Gesture::PanStart));
    }
}
