//! A window's scroll events taken from the OS directly, as it tells them,
//! for the toolkit's wheel events to carry.
//!
//! winit 0.30 folds macOS's two phases of a scroll event (`NSEvent`'s
//! `phase`, the fingers', and `momentumPhase`, the inertia AppKit sends
//! after a lift) into one `TouchPhase`, so the beginning of momentum and
//! fingers going down again look alike, and drops the event's time. It
//! hands on no identity of the `NSEvent` either (no timestamp, no event
//! number; AppKit gives scroll events no `eventNumber`), so no event seen
//! elsewhere can be matched to winit's for sure, and a match by window and
//! delta is a guess (a zero-delta end of one gesture and a zero-delta
//! begin of the next look alike).
//!
//! **Taken, not matched.** A [`ScrollTap`] installs an AppKit local event
//! monitor (`addLocalMonitorForEventsMatchingMask:handler:`) for scroll
//! events. AppKit calls it with each scroll event the application
//! dispatches, before dispatching it; for an event to the tapped window
//! the monitor hands the event, read whole (both phases, the OS's time,
//! where it is), to the tap's handler and returns nil, which tells AppKit
//! to dispatch nothing: the window's view never gets `scrollWheel:`. That
//! is the only place winit 0.30 makes a `MouseWheel` (window event and
//! device event both: `platform_impl/macos/view.rs`, `scroll_wheel`), and
//! its `sendEvent:` override (`app.rs`, `send_event`) makes no device
//! event of a scroll, so each scroll event reaches the toolkit once, by
//! the tap, and none by winit. Events to other windows go on as they came.
//!
//! **What the tap keeps of winit's view.** winit's `scrollWheel:` also
//! reads the pointer's place off the event (`mouse_motion`, skipped for a
//! place outside the content view with no button down) and the modifiers
//! (`update_modifiers`); the tapped event carries both, and whether its
//! place counts ([`Scroll::pointer`], [`Scroll::modifiers`]), for the
//! toolkit to take as winit's view would. AppKit calls no monitor for an event a nested tracking loop
//! takes (a menu, a window drag): such an event goes nowhere, as it would
//! with no tap.
//!
//! Elsewhere there is nothing to tap: Wayland's phases come through winit
//! whole (a stop is its end; it has no momentum), and X11 and Windows tell
//! none; [`tap_scrolls`] is `None` there, and winit's event is the one.

use std::time::Instant;

/// One of a scroll event's phases, as AppKit tells it (`NSEventPhase`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollPhase {
    /// None: a notched wheel's, or not this phase's part of the gesture.
    #[default]
    None,
    /// Fingers touched, not yet moving (`MayBegin`).
    MayBegin,
    Began,
    /// Fingers down, not moving (`Stationary`).
    Stationary,
    Changed,
    Ended,
    /// Lifted without scrolling (`Cancelled`).
    Cancelled,
}

/// A scroll event as the OS told it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scroll {
    /// The fingers' phase.
    pub phase: ScrollPhase,
    /// The momentum's phase (the inertia after a lift).
    pub momentum: ScrollPhase,
    /// When it happened, by the OS's timestamp, on the monotonic clock.
    pub at: Instant,
    /// Its delta as the event gave it: points when precise, else lines.
    pub delta: (f64, f64),
    /// Its delta is precise (a trackpad's points), not a notched wheel's
    /// lines.
    pub precise: bool,
    /// Where it is, in the window's content, points from its top left
    /// (winit's logical pixels).
    pub point: (f64, f64),
    /// [`Self::point`] lies in the window's content view (winit's
    /// `mouse_motion` reads no place off an event outside it unless a
    /// button is down).
    pub inside: bool,
    /// A mouse button is down (`NSEvent.pressedMouseButtons`).
    pub buttons: bool,
    /// The modifiers held as the event says (winit's `update_modifiers`
    /// reads them off each scroll event it is given).
    pub modifiers: ScrollModifiers,
}

/// The modifier keys held, as a scroll event tells them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollModifiers {
    pub shift: bool,
    pub control: bool,
    /// Option.
    pub alt: bool,
    /// ⌘.
    pub command: bool,
}

impl Scroll {
    /// The pointer's place to take from it, as winit's `scrollWheel:` does
    /// (its `mouse_motion`): none when it is outside the content view and
    /// no button is down.
    #[must_use]
    pub fn pointer(&self) -> Option<(f64, f64)> {
        (self.inside || self.buttons).then_some(self.point)
    }
}

/// The scroll events of one window, taken from the OS as they come (see
/// the module's notes): while it lives they go to its handler, and not to
/// the window. Dropped, the window gets them again.
pub struct ScrollTap {
    #[cfg(target_os = "macos")]
    _monitor: crate::macos::Tap,
}

/// Take `window`'s scroll events (winit's `WindowId`, as `u64`) from the
/// OS, each to `on` as it comes, on the main thread, the window getting
/// none. Called on the main thread; `None` off it, and where the OS tells
/// nothing more than winit (everywhere but macOS).
#[must_use]
pub fn tap_scrolls(window: u64, on: impl Fn(Scroll) + 'static) -> Option<ScrollTap> {
    #[cfg(target_os = "macos")]
    {
        crate::macos::tap(window, Box::new(on)).map(|t| ScrollTap { _monitor: t })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, on);
        None
    }
}
