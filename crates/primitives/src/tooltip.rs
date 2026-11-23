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

use std::cell::Cell;
#[expect(
    clippy::disallowed_types,
    reason = "per-thread singletons: the window's provider, the open tooltip, the triggers' boxes each tooltip writes at layout and takes out as it unmounts"
)]
use std::cell::RefCell;
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

/// The tooltip open in a window: its number, and how to close it.
struct OpenTooltip {
    id: u64,
    close: Box<dyn Fn()>,
}

gui::engine_local! {
    /// The window's provider, for tooltips outside any: made the first
    /// time it is asked for, outside any cold frame ([`gui::engine::live`]).
    #[expect(clippy::disallowed_types, reason = "per-thread singletons: the window's provider, the open tooltip, the triggers' boxes each tooltip writes at layout and takes out as it unmounts")]
    [Shared("the window's provider: a handle to signals of the live arena, made outside any cold frame")]
    static WINDOW: RefCell<Option<TooltipProvider>> = RefCell::new(None);
    /// The tooltip open in the window, and how to close it.
    #[expect(clippy::disallowed_types, reason = "per-thread singletons: the window's provider, the open tooltip, the triggers' boxes each tooltip writes at layout and takes out as it unmounts")]
    [Fork] static OPEN: RefCell<Option<Rc<OpenTooltip>>> = RefCell::new(None);
    /// The next tooltip's number.
    [Fork] static NEXT: Cell<u64> = Cell::new(1);
    /// Every mounted trigger's box, as layout last placed it, by tooltip.
    #[expect(clippy::disallowed_types, reason = "per-thread singletons: the window's provider, the open tooltip, the triggers' boxes each tooltip writes at layout and takes out as it unmounts")]
    [Fork(log)] static TRIGGERS: RefCell<gui::engine::ForkMap<u64, Bounds>> = RefCell::default();
}

/// Tooltip `id` is no longer the one open in the window; whether it was.
fn shut(id: u64) -> bool {
    OPEN.with(|o| {
        let mut o = o.borrow_mut();
        let was = o.as_ref().is_some_and(|t| t.id == id);
        if was {
            *o = None;
        }
        was
    })
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

    /// The window's provider: made at the first call, with `options`, and
    /// kept for the thread's life.
    #[must_use]
    pub fn window(options: TooltipOptions) -> Self {
        gui::engine::live(|| {
            WINDOW.with(|w| {
                *w.borrow_mut()
                    .get_or_insert_with(|| gui::detached(|| Self::new(move || options)))
            })
        })
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
        let id = NEXT.with(|n| {
            let id = n.get();
            n.set(id.wrapping_add(1));
            id
        });
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
        };
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
                    let me = Rc::new(OpenTooltip {
                        id: tip.id,
                        close: Box::new(close),
                    });
                    let others = OPEN.with(|o| o.borrow_mut().replace(me));
                    if let Some(other) = others
                        && other.id != id
                    {
                        (other.close)();
                    }
                } else {
                    tip.opening.cancel();
                    tip.grace.clear();
                    tip.state.set(TooltipState::Closed);
                    tip.provider.closed();
                    shut(id);
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
            TRIGGERS.with(|t| t.borrow_mut().remove(&id));
            // The provider (the window's outlives every tooltip) keeps no
            // grace area of one gone: the next tooltip's moves read it.
            let transit = tip.provider.in_transit;
            if transit.is_alive() && transit.get_untracked() == Some(tip.grace) {
                transit.set(None);
            }
            // Unmounted open, it closes as far as its provider knows: the
            // same skip-delay window as a close.
            if shut(id) {
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
                TRIGGERS.with(|t| {
                    let mut t = t.borrow_mut();
                    if t.get(&id) != Some(b) {
                        t.insert(id, *b);
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
                let covers = TRIGGERS.with(|t| {
                    t.borrow().iter().any(|(&i, b)| {
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

#[cfg(test)]
mod tests {
    use super::{TooltipOptions, TooltipProvider};

    /// The window's provider asked for first inside a cold frame is made
    /// live: the next live caller gets the same one, and it works.
    #[test]
    fn the_windows_provider_made_first_in_a_cold_frame_is_a_live_one() {
        std::thread::spawn(|| {
            let cold = gui::engine::in_cold_frame(|| TooltipProvider::window(TooltipOptions::default()));
            let live = TooltipProvider::window(TooltipOptions::default());
            assert!(cold == live, "one provider for the window");
            assert!(
                live.open_delayed.is_alive() && live.options.is_alive(),
                "its state is live"
            );
            live.opened();
            assert!(!live.open_delayed.get_untracked(), "and it works");
            assert_eq!(live.options().delay, TooltipOptions::default().delay);
        })
        .join()
        .expect("the thread ends");
    }
}
