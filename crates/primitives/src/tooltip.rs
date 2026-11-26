//! A short label for a trigger (reka's `Tooltip` and `TooltipProvider`):
//! shown while the pointer rests on the trigger or while the trigger holds
//! focus, read by assistive technology as the trigger's description.
//!
//! **Provider.** Tooltips under one [`TooltipProvider`] share its options
//! and its skip-delay window: while one of them is open, and for the skip
//! delay (300 ms) after it closes, the pointer reaching another trigger
//! opens that one at once (`instant-open`); otherwise after the delay
//! (700 ms, `delayed-open`). A tooltip's own options win over its
//! provider's. reka requires a provider (`TooltipRoot` throws without
//! one); here a tooltip outside any takes the window's
//! ([`TooltipProvider::window`]): its delays are app-wide anyway, and only
//! one tooltip is ever open in a window, whatever its provider (reka's
//! `TOOLTIP_OPEN` event on the document). Recorded in ADR-0025.
//!
//! **Opening.** The first pointer move on the trigger opens it (not the
//! pointer's arrival), unless the pointer is crossing to another tooltip's
//! content; once opened in a hover it does not open again until the
//! pointer leaves and comes back. Focus reaching the trigger opens it at
//! once (only keyboard focus when [`TooltipOptions::keyboard_focus_only`]),
//! but not the focus its own press gives it.
//!
//! **Closing.** Leaving the trigger closes it at once when its content is
//! not hoverable; otherwise a grace area ([`crate::hover::Grace`]) lets the
//! pointer cross to the content, and leaving that area closes it at once
//! (no close delay). Its trigger losing focus, pressed or clicked (Enter
//! and Space click it) closes it unless [`TooltipOptions::closing_trigger`]
//! is off. Escape, a press outside, a scroll of what holds the trigger,
//! another tooltip opening, and the content covering another tooltip's
//! trigger under the pointer close it too. Focus going outside does not.
//! It never moves focus.
//!
//! **What is read.** The content is a `tooltip`, and the trigger is
//! described by it only while it is open (a typed relation the engine sends
//! only toward a node that is there). reka copies the content's text into a
//! visually hidden `role=tooltip` node; the content node itself is read
//! here, its arrow included only if the look makes it readable, and a
//! caller's own label replaces its text (`aria-label`).

#[expect(
    clippy::disallowed_types,
    reason = "per-thread singletons: the window's provider, the open tooltip, the triggers' boxes each tooltip writes at layout and takes out as it unmounts"
)]
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gui::{Bounds, Dismiss, DismissPolicy, DismissReason, Div, FocusHandle, Layer, Placement, Role, Signal, UseModel};

use crate::dialog::Veto;
use crate::hover::{Boxes, Delay, Grace};
use crate::presence::Presence;
use crate::relation::{Linked, Points};

/// reka's `delayDuration`.
pub const DELAY: Duration = Duration::from_millis(700);
/// reka's `skipDelayDuration`.
pub const SKIP_DELAY: Duration = Duration::from_millis(300);

/// How tooltips behave (reka's `TooltipProvider` props; each also a
/// `TooltipRoot` prop).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooltipOptions {
    /// How long the pointer rests before it opens.
    pub delay: Duration,
    /// How long after one closes another still opens at once.
    pub skip_delay: Duration,
    /// The pointer may move onto the content (reka's
    /// `disableHoverableContent` false).
    pub hoverable: bool,
    /// Pressing or clicking the trigger closes it (reka's
    /// `disableClosingTrigger` false).
    pub closing_trigger: bool,
    /// Only keyboard focus opens it (reka's `ignoreNonKeyboardFocus`).
    pub keyboard_focus_only: bool,
    /// The trigger does nothing.
    pub disabled: bool,
}

impl Default for TooltipOptions {
    fn default() -> Self {
        Self {
            delay: DELAY,
            skip_delay: SKIP_DELAY,
            hoverable: true,
            closing_trigger: true,
            keyboard_focus_only: false,
            disabled: false,
        }
    }
}

/// A tooltip's own choices over its provider's: each `None` takes the
/// provider's.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TooltipOverrides {
    pub delay: Option<Duration>,
    pub hoverable: Option<bool>,
    pub closing_trigger: Option<bool>,
    pub keyboard_focus_only: Option<bool>,
    pub disabled: Option<bool>,
    /// Kept mounted while closed (reka's `forceMount`).
    pub keep_mounted: bool,
    /// Asked before Escape, a press outside or a scroll closes it.
    pub veto: Option<Veto>,
}

/// How a tooltip is open: what its look reads (reka's `data-state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TooltipState {
    Closed,
    /// Opened after the delay.
    DelayedOpen,
    /// Opened at once (keyboard focus, or within the skip delay).
    InstantOpen,
}

/// Tooltips sharing options and a skip-delay window: made once, in a
/// provider's setup, or the window's ([`Self::window`]).
#[derive(Clone, Copy, PartialEq)]
pub struct TooltipProvider {
    options: Signal<Rc<dyn Fn() -> TooltipOptions>>,
    /// The next tooltip waits for the delay (true at first).
    open_delayed: Signal<bool>,
    skip: Delay,
    /// The open tooltip's grace area is being crossed.
    in_transit: Signal<Option<Grace>>,
}

impl std::fmt::Debug for TooltipProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TooltipProvider").finish_non_exhaustive()
    }
}

/// The tooltips whose closers were dropped (tests): a probe outside the
/// engine, which a check neither forks nor puts back.
#[cfg(test)]
static CLOSERS_DROPPED: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

/// A closer's drop, recorded by its tooltip's number (tests): captured by
/// the close callback, so dropped with it.
#[cfg(test)]
struct CloserDropped(u64);

#[cfg(test)]
impl Drop for CloserDropped {
    fn drop(&mut self) {
        if let Ok(mut d) = CLOSERS_DROPPED.lock() {
            d.push(self.0);
        }
    }
}

gui::engine_local! {
    /// The window's provider, for tooltips outside any: made the first
    /// time it is asked for, by a fixed init, so it is made live once a
    /// fork that asked first has rolled back (ADR-0028). It holds only the
    /// provider's identity: its options are read from [`WINDOW_OPTIONS`]
    /// each time a tooltip acts.
    [Shared("the window's provider: a handle to signals of the live arena, made by a fixed init")]
    static WINDOW: TooltipProvider = TooltipProvider::new(|| WINDOW_OPTIONS.with(Cell::get).unwrap_or_default());
    /// The options the first live caller of [`TooltipProvider::window`]
    /// gave: written state, so forked (a Shared piece is immutable once
    /// made); a fork's call sets them only in the fork.
    [Fork] static WINDOW_OPTIONS: Cell<Option<TooltipOptions>> = const { Cell::new(None) };
    /// The tooltip open in each window ([`gui::focus::window_id`]), by
    /// number alone: how it closes is its live owner's ([`CLOSERS`]), so a
    /// check that opens or closes one copies, holds and drops no callback
    /// (ADR-0034, S7). Keyed by window, so opening one closes only the one
    /// open in its own window: a closer runs in the window it was made in
    /// (ADR-0015, r17).
    #[expect(clippy::disallowed_types, reason = "per-thread singletons: the window's provider, the open tooltip, the triggers' boxes each tooltip writes at layout and takes out as it unmounts")]
    [Fork(log)] static OPEN: RefCell<gui::engine::ForkMap<u64, u64>> = RefCell::default();
    /// How each mounted tooltip closes, by number: put in by its setup and
    /// taken out as it unmounts, by replacement only (a check copies none,
    /// and what a check's setup puts in is the check's).
    #[expect(clippy::disallowed_types, reason = "per-thread singletons: the window's provider, the open tooltip, the triggers' boxes each tooltip writes at layout and takes out as it unmounts")]
    [Fork(log)] static CLOSERS: RefCell<gui::engine::ForkMap<u64, gui::engine::AppHandle<dyn Fn()>>> = RefCell::default();
    /// Every mounted trigger's box, as layout last placed it, by tooltip.
    #[expect(clippy::disallowed_types, reason = "per-thread singletons: the window's provider, the open tooltip, the triggers' boxes each tooltip writes at layout and takes out as it unmounts")]
    [Fork(log)] static TRIGGERS: RefCell<gui::engine::ForkMap<u64, Bounds>> = RefCell::default();
}

/// The tooltips made, numbered.
static NEXT: gui::engine::IdKind = gui::engine::IdKind::new("tooltip");

/// The tooltip open in `window`, if any.
fn open_in(window: u64) -> Option<u64> {
    OPEN.with_borrow(|o| o.get(&window).copied())
}

/// Tooltip `id` is no longer the one open in its `window`; whether it was.
fn shut(window: u64, id: u64) -> bool {
    let was = open_in(window) == Some(id);
    if was {
        OPEN.with_borrow_mut(|o| o.remove(&window));
    }
    was
}

/// Tooltip `id` is the one open in its `window`: the one open there before
/// it, if another, closed by its owner's closer (found by number, called
/// with nothing borrowed). Only a tooltip of the same window is closed, so
/// the closer runs in the turn of the window that made it.
fn open_alone(window: u64, id: u64) {
    let Some(other) = OPEN.with_borrow_mut(|o| o.insert(window, id)) else {
        return;
    };
    if other == id {
        return;
    }
    let close = CLOSERS.with_borrow(|c| c.get(&other).map(gui::engine::AppHandle::rc));
    if let Some(close) = close {
        close();
    }
}

impl TooltipProvider {
    /// A provider with `options` (read when a tooltip acts); called in its
    /// setup.
    #[must_use]
    #[track_caller]
    pub fn new(options: impl Fn() -> TooltipOptions + 'static) -> Self {
        Self {
            options: Signal::new(Rc::new(options)),
            open_delayed: Signal::labeled("tooltip open delayed", true),
            skip: Delay::new(),
            in_transit: Signal::new(None),
        }
    }

    /// The window's provider: made at the first call, with `options` (the
    /// first live call's: a call in a fork is undone with it), and kept
    /// for the thread's life.
    #[must_use]
    pub fn window(options: TooltipOptions) -> Self {
        WINDOW_OPTIONS.with(|o| {
            if o.get().is_none() {
                o.set(Some(options));
            }
        });
        WINDOW.with(|w| *w)
    }

    /// The provider `gui::provide` gave the caller's subtree, else the
    /// window's with `options`.
    #[must_use]
    pub fn nearest_or_window(options: TooltipOptions) -> Self {
        gui::use_context::<Self>().unwrap_or_else(|| Self::window(options))
    }

    fn options(self) -> TooltipOptions {
        let f = self.options.with_untracked(Rc::clone);
        gui::untracked(|| f())
    }

    fn opened(self) {
        self.skip.cancel();
        self.open_delayed.set(false);
    }

    /// A tooltip of its closed, or was unmounted open: the skip-delay
    /// window starts, after which the next waits its delay again.
    fn closed(self) {
        if !self.options.is_alive() || !self.open_delayed.is_alive() {
            return;
        }
        let skip = self.options().skip_delay;
        let delayed = self.open_delayed;
        self.skip.after(skip, move || {
            if delayed.is_alive() {
                delayed.set(true);
            }
        });
    }

    fn crossing(self) -> bool {
        self.in_transit
            .get_untracked()
            .is_some_and(|g| gui::untracked(|| g.in_transit()))
    }
}

/// A tooltip's state: made once in its root's setup (`Copy`).
pub struct Tooltip<P: 'static> {
    open: UseModel<P, bool>,
    provider: TooltipProvider,
    overrides: Signal<Rc<dyn Fn() -> TooltipOverrides>>,
    state: Signal<TooltipState>,
    opening: Delay,
    grace: Grace,
    linked: Linked,
    placed: Signal<Option<Placement>>,
    trigger: FocusHandle,
    /// The pointer moved on the trigger since it came: opened once.
    moved: Signal<bool>,
    /// The trigger was just pressed: the focus that press gives opens
    /// nothing.
    pressed: Signal<bool>,
    id: u64,
    /// The window its setup ran in: where it is the one open.
    window: u64,
}

impl<P: 'static> Clone for Tooltip<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Tooltip<P> {}

impl<P: 'static> PartialEq for Tooltip<P> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<P: 'static> std::fmt::Debug for Tooltip<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tooltip").field("id", &self.id).finish_non_exhaustive()
    }
}

/// The options in force: the tooltip's over its provider's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Resolved {
    delay: Duration,
    hoverable: bool,
    closing_trigger: bool,
    keyboard_focus_only: bool,
    disabled: bool,
}

impl<P: 'static> Tooltip<P> {
    /// A tooltip open as `open` says, under `provider`, with its own
    /// `overrides`; called in its root's setup, which owns its timers.
    #[must_use]
    #[track_caller]
    pub fn new(
        open: UseModel<P, bool>,
        provider: TooltipProvider,
        overrides: impl Fn() -> TooltipOverrides + 'static,
    ) -> Self {
        let id = gui::engine::unique_id(&NEXT);
        let window = gui::focus::window_id().get();
        let state = Signal::labeled(
            "tooltip state",
            if gui::untracked(|| open.get()) {
                TooltipState::InstantOpen
            } else {
                TooltipState::Closed
            },
        );
        let close = move || {
            if state.is_alive() {
                open.set(false);
            }
        };
        let tip = Self {
            open,
            provider,
            overrides: Signal::new(Rc::new(overrides)),
            state,
            opening: Delay::new(),
            grace: Grace::new(move |()| close()),
            linked: Linked::new(Points::DescribedBy),
            placed: Signal::labeled("tooltip placed", None),
            trigger: FocusHandle::new(),
            moved: Signal::new(false),
            pressed: Signal::new(false),
            id,
            window,
        };
        // How it closes, owned here until it unmounts: the open tooltip's
        // slot holds its number alone.
        #[cfg(test)]
        let dropped = CloserDropped(id);
        let closer: Rc<dyn Fn()> = Rc::new(move || {
            #[cfg(test)]
            let _dropped = &dropped;
            close();
        });
        CLOSERS.with_borrow_mut(|c| c.replace(id, gui::engine::AppHandle::new(closer)));
        // However it opens or closes (the caller's write included): the
        // provider's window, one open in the window, the state the look reads.
        gui::watch(
            move || open.get(),
            move |is_open, _| {
                if *is_open {
                    if tip.state.get_untracked() == TooltipState::Closed {
                        tip.state.set(TooltipState::InstantOpen);
                    }
                    tip.provider.opened();
                    tip.provider.in_transit.set(Some(tip.grace));
                    open_alone(tip.window, id);
                } else {
                    tip.opening.cancel();
                    tip.grace.clear();
                    tip.state.set(TooltipState::Closed);
                    tip.provider.closed();
                    shut(tip.window, id);
                }
            },
        );
        // Focus reaching the trigger opens it; leaving it, closes it.
        gui::watch(
            move || tip.trigger.is_focused(),
            move |focused, _| {
                let r = tip.resolved();
                if r.disabled {
                    return;
                }
                if !*focused {
                    tip.close();
                    return;
                }
                if tip.pressed.get_untracked() {
                    tip.pressed.set(false);
                    return;
                }
                if r.keyboard_focus_only && !gui::untracked(gui::focus::focus_visible) {
                    return;
                }
                tip.open_now();
            },
        );
        gui::on_unmounted(move || {
            TRIGGERS.with_borrow_mut(|t| t.forget(&id));
            // Its closer let go, dropped with nothing borrowed.
            let closer = CLOSERS.with_borrow(|c| c.get(&id).cloned());
            CLOSERS.with_borrow_mut(|c| c.forget(&id));
            drop(closer);
            // The provider (the window's outlives every tooltip) keeps no
            // grace area of one gone: the next tooltip's moves read it.
            let transit = tip.provider.in_transit;
            if transit.is_alive() && transit.get_untracked() == Some(tip.grace) {
                transit.set(None);
            }
            // Unmounted open, it closes as far as its provider knows: the
            // same skip-delay window as a close.
            if shut(tip.window, id) {
                tip.provider.closed();
            }
        });
        tip
    }

    fn resolved(self) -> Resolved {
        let p = self.provider.options();
        let f = self.overrides.with_untracked(Rc::clone);
        let o = gui::untracked(|| f());
        Resolved {
            delay: o.delay.unwrap_or(p.delay),
            hoverable: o.hoverable.unwrap_or(p.hoverable),
            closing_trigger: o.closing_trigger.unwrap_or(p.closing_trigger),
            keyboard_focus_only: o.keyboard_focus_only.unwrap_or(p.keyboard_focus_only),
            disabled: o.disabled.unwrap_or(p.disabled),
        }
    }

    /// Whether it is open (tracked).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.open.get()
    }

    /// How it is open (tracked): what its trigger's and content's look read.
    #[must_use]
    pub fn state(self) -> TooltipState {
        self.state.get()
    }

    pub fn set_open(self, open: bool) {
        self.opening.cancel();
        self.open.set(open);
    }

    /// Where the content is (tracked).
    #[must_use]
    pub fn presence(self) -> Presence {
        let f = self.overrides.with_untracked(Rc::clone);
        Presence::of(self.is_open(), f().keep_mounted)
    }

    /// Where the content was placed, as layout last reported it (tracked).
    #[must_use]
    pub fn placement(self) -> Option<Placement> {
        self.placed.get()
    }

    fn open_now(self) {
        self.opening.cancel();
        if !self.open.get_untracked() {
            self.state.set(TooltipState::InstantOpen);
        }
        self.open.set(true);
    }

    fn close(self) {
        self.opening.cancel();
        self.open.set(false);
    }

    /// The pointer came onto the trigger (its first move there).
    fn enter(self) {
        if !self.provider.open_delayed.get_untracked() {
            self.open_now();
            return;
        }
        let (delay, open, state) = (self.resolved().delay, self.open, self.state);
        self.opening.after(delay, move || {
            if !open.get_untracked() {
                state.set(TooltipState::DelayedOpen);
            }
            open.set(true);
        });
    }

    /// The focus of its trigger (the trigger part tracks it).
    #[must_use]
    pub fn trigger_focus(self) -> FocusHandle {
        self.trigger
    }

    /// `d` as the trigger: the pointer's first move on it opens the
    /// tooltip; leaving it, pressing it or clicking it closes it (see the
    /// module's docs); it is described by the content while open.
    #[must_use]
    pub fn trigger(self, d: Div) -> Div {
        let id = self.id;
        let open = self.is_open();
        self.linked
            .trigger(d.track_focus(self.trigger), open)
            .on_measured(move |b| {
                TRIGGERS.with_borrow_mut(|t| {
                    if t.get(&id) != Some(b) {
                        t.replace(id, *b);
                    }
                });
            })
            .on_pointer_move(move |_| {
                if self.moved.get_untracked() || self.resolved().disabled || self.provider.crossing() {
                    return;
                }
                self.moved.set(true);
                self.grace.clear();
                self.enter();
            })
            .on_hover(move |inside| {
                self.grace.trigger(inside);
                if inside {
                    return;
                }
                self.moved.set(false);
                self.pressed.set(false);
                let r = self.resolved();
                if r.disabled {
                    return;
                }
                if !r.hoverable {
                    self.close();
                    return;
                }
                self.opening.cancel();
                if !self.open.get_untracked() {
                    return;
                }
                match self.placed.get_untracked().as_ref().map(Boxes::of) {
                    Some(boxes) => self.grace.leave(true, boxes),
                    // Open but not placed yet: no content to cross to.
                    None => self.close(),
                }
            })
            .on_mouse_down(move |_| {
                let r = self.resolved();
                if r.disabled {
                    return;
                }
                if r.closing_trigger && self.open.get_untracked() {
                    self.close();
                }
                self.pressed.set(true);
            })
            // Beside the trigger's own click (a button's action): handlers
            // add up.
            .on_click(move |_| {
                // The press is over: the next focus is not its (reka clears
                // the flag 1 ms after pointerup, once focus has moved).
                self.pressed.set(false);
                let r = self.resolved();
                if !r.disabled && r.closing_trigger {
                    self.close();
                }
            })
    }

    /// `d` as the content: a `tooltip`, its trigger's description while
    /// open, the layer the module's docs list. Render it as
    /// [`Self::presence`] says, inside the deferred layer
    /// [`crate::popper::Popper`] places ([`Self::floating`] does both).
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        let id = self.id;
        let policy = DismissPolicy::DEFAULT.focus_outside(false).scroll(true);
        self.linked
            .content(d, false)
            .role(Role::Tooltip)
            .on_hover(move |inside| {
                if !self.resolved().hoverable {
                    return;
                }
                if inside {
                    self.grace.clear();
                } else if let Some(boxes) = self.placed.get_untracked().as_ref().map(Boxes::of) {
                    self.grace.leave(false, boxes);
                }
            })
            .on_pointer_move(move |e| {
                // Over another tooltip's trigger: let that one be reached.
                let covers = TRIGGERS.with_borrow(|t| {
                    t.iter().any(|(&i, b)| {
                        i != id && e.x >= b.x && e.x < b.x + b.width && e.y >= b.y && e.y < b.y + b.height
                    })
                });
                if covers {
                    self.close();
                }
            })
            .layer(Layer::new().dismiss(policy).on_dismiss(move |r| self.dismiss(r)))
    }

    /// `d` as the tooltip's content ([`Self::content`]) on its own layer,
    /// placed beside the trigger as `popper` asks, its placement reported
    /// to [`Self::placement`] (which its grace area reads too).
    #[must_use]
    pub fn floating(self, d: Div, popper: &crate::popper::Popper) -> gui::Deferred {
        gui::deferred(popper.positioning(), self.content(d)).placement(self.placed)
    }

    fn dismiss(self, reason: &DismissReason) -> Dismiss {
        if !self.overrides.is_alive() {
            return Dismiss::Veto;
        }
        let f = self.overrides.with_untracked(Rc::clone);
        if *reason != DismissReason::Programmatic
            && let Some(veto) = gui::untracked(|| f()).veto
            && veto.call(*reason) == Dismiss::Veto
        {
            return Dismiss::Veto;
        }
        self.close();
        Dismiss::Close
    }
}

/// The fork tests: forks are built in debug builds only (ADR-0028, F2).
#[cfg(all(test, debug_assertions))]
mod tests {
    use super::{TooltipOptions, TooltipProvider};

    /// F1 review 9 (ADR-0028): the window's provider asked for first inside
    /// a cold frame is the fork's stand-in, gone with it; the first live
    /// caller makes the live one, with its own options, and it works.
    #[test]
    fn the_windows_provider_made_first_in_a_cold_frame_is_a_stand_in() {
        std::thread::spawn(|| {
            let options = TooltipOptions {
                delay: std::time::Duration::from_millis(5),
                ..TooltipOptions::default()
            };
            let cold = gui::testing::in_check(move || TooltipProvider::window(options));
            assert!(!cold.options.is_alive(), "the fork's stand-in, gone with it");
            let live = TooltipProvider::window(TooltipOptions::default());
            assert!(cold != live, "the live one is made live");
            assert!(
                live.open_delayed.is_alive() && live.options.is_alive(),
                "its state is live"
            );
            live.opened();
            assert!(!live.open_delayed.get_untracked(), "and it works");
            assert_eq!(
                live.options(),
                TooltipOptions::default(),
                "the first live caller's options, not the fork's"
            );
        })
        .join()
        .expect("the thread ends");
    }

    /// Codex 7 (F1 review 9): the window's options are what the first
    /// LIVE caller gives; a fork's call (rolled back) sets nothing.
    #[test]
    fn first_live_caller_sets_window_options() {
        std::thread::spawn(|| {
            let speculative = TooltipOptions {
                delay: std::time::Duration::from_millis(5),
                ..TooltipOptions::default()
            };
            let _ = gui::testing::in_check(move || TooltipProvider::window(speculative));
            let expected = TooltipOptions::default();
            assert_eq!(TooltipProvider::window(expected).options(), expected);
        })
        .join()
        .expect("the thread ends");
    }

    gui::engine_local! {
        /// A Shared piece whose init makes a provider (with fixed options:
        /// asking for the window's would read its forked options, which a
        /// Shared init may not).
        [Shared("a test of a Shared init that makes a provider")]
        static ASKS: TooltipProvider = TooltipProvider::new(TooltipOptions::default);
    }

    /// Opus I1 (F1 review 7), F1 review 9: a provider first made by a
    /// Shared piece's init in a check is the fork's stand-in, its delay's
    /// scope and timer state with it, all gone with the fork; live code
    /// makes the piece whole at its first touch, and it works.
    #[test]
    fn a_provider_first_made_inside_a_shared_init_in_a_check_is_a_stand_in() {
        std::thread::spawn(|| {
            let cold = gui::testing::in_check(|| ASKS.with(|p| *p));
            assert!(
                !cold.open_delayed.is_alive() && !cold.in_transit.is_alive(),
                "the fork's stand-in, gone with it"
            );
            let live = ASKS.with(|p| *p);
            assert!(cold != live, "made live at the first live touch");
            assert!(
                live.open_delayed.is_alive() && live.options.is_alive() && live.in_transit.is_alive(),
                "its state is live"
            );
            live.closed();
            live.opened();
            assert!(!live.open_delayed.get_untracked(), "and it works, its delay too");
        })
        .join()
        .expect("the thread ends");
    }

    /// ADR-0034 S7: the open tooltip is an identity; how it closes is its
    /// live owner's. A check that opens one over the live open one (what
    /// its open watch does: a check runs no effect, so the steps are run
    /// here), closes it and opens it again, makes, holds and drops no close
    /// callback; after it the live one is open, its closer as it was, and
    /// it still closes.
    #[test]
    fn a_check_opening_a_tooltip_holds_no_close_callback() {
        use std::rc::Rc;

        use super::{CLOSERS, CLOSERS_DROPPED, Tooltip, TooltipOverrides, open_alone, open_in, shut};

        #[derive(Clone, PartialEq)]
        struct Props {
            open: gui::Bind<bool>,
        }
        std::thread::spawn(|| {
            let make = || {
                let props = gui::Signal::new(Props {
                    open: gui::Bind::default_value(false),
                });
                Tooltip::new(
                    gui::use_model(props, |p: &Props| &p.open),
                    TooltipProvider::new(TooltipOptions::default),
                    TooltipOverrides::default,
                )
            };
            let owner = gui::Scope::new();
            let (a, b) = owner.run(|| (make(), make()));
            a.set_open(true);
            let w = a.window;
            assert_eq!(open_in(w), Some(a.id));
            let strong = |id: u64| CLOSERS.with_borrow(|c| c.get(&id).map(|h| Rc::strong_count(&h.rc()) - 1));
            assert_eq!((strong(a.id), strong(b.id)), (Some(1), Some(1)));
            let b_id = b.id;
            gui::testing::in_check(move || {
                open_alone(w, b_id);
                assert_eq!(open_in(w), Some(b_id), "b is open in the check");
                assert!(shut(w, b_id));
                assert_eq!(open_in(w), None);
                open_alone(w, b_id);
            });
            let dropped = || {
                let d = CLOSERS_DROPPED.lock().expect("the probe");
                d.iter().filter(|id| **id == a.id || **id == b.id).count()
            };
            assert_eq!(dropped(), 0, "no close callback dropped by the check");
            assert_eq!(open_in(w), Some(a.id), "the live one is open");
            assert!(a.is_open() && !b.is_open());
            let both = (strong(a.id), strong(b.id));
            assert_eq!(both, (Some(1), Some(1)), "the closers as they were");
            b.set_open(true);
            assert!(!a.is_open(), "the live closer still closes it");
            owner.dispose();
            assert_eq!(dropped(), 2, "dropped by their owners, as they unmount");
        })
        .join()
        .expect("the thread ends");
    }

    /// Review r17, decision 6: a tooltip's closer runs in its own window.
    /// Opening a tooltip in window B leaves window A's open one alone (one
    /// open per window, not per thread: B's turn never calls A's closer);
    /// opening a second in B closes B's first, in B's turn.
    #[test]
    fn a_tooltip_opened_in_one_window_closes_none_of_another() {
        use std::cell::Cell;
        use std::rc::Rc;

        use super::{Tooltip, TooltipOverrides, open_in};

        /// Where a turn hands out the tooltip it made.
        type Made = Rc<Cell<Option<Tooltip<Props>>>>;

        #[derive(Clone, PartialEq)]
        struct Props {
            open: gui::Bind<bool>,
        }
        std::thread::spawn(|| {
            let make = || {
                let props = gui::Signal::new(Props {
                    open: gui::Bind::default_value(false),
                });
                Tooltip::new(
                    gui::use_model(props, |p: &Props| &p.open),
                    TooltipProvider::new(TooltipOptions::default),
                    TooltipOverrides::default,
                )
            };
            let window = || gui::app::Pipeline::new(|| gui::IntoElement::into_element(gui::div()), false);
            let (mut a, mut b) = (window(), window());
            let owner = gui::Scope::new();
            let made: Made = Rc::default();
            let in_a = Rc::clone(&made);
            a.act("open in a", move || {
                let t = owner.run(make);
                t.set_open(true);
                in_a.set(Some(t));
            });
            let ta = made.take().expect("made in a");
            let (b1, b2): (Made, Made) = (Rc::default(), Rc::default());
            let (k1, k2) = (Rc::clone(&b1), Rc::clone(&b2));
            b.act("open in b", move || {
                let t = owner.run(make);
                t.set_open(true);
                k1.set(Some(t));
            });
            let tb = b1.take().expect("made in b");
            assert_ne!(ta.window, tb.window, "two windows");
            assert!(ta.is_open(), "b's open tooltip closed none of a's");
            assert_eq!(open_in(ta.window), Some(ta.id));
            assert_eq!(open_in(tb.window), Some(tb.id));
            b.act("open again in b", move || {
                let t = owner.run(make);
                t.set_open(true);
                k2.set(Some(t));
            });
            let tb2 = b2.take().expect("made in b");
            assert!(!tb.is_open() && tb2.is_open(), "b's first closed in b");
            assert!(ta.is_open(), "a's still open");
            assert_eq!(open_in(tb.window), Some(tb2.id));
            owner.dispose();
            assert_eq!((open_in(ta.window), open_in(tb.window)), (None, None));
        })
        .join()
        .expect("the thread ends");
    }
}
