//! A card that shows while the pointer rests on its trigger (reka's
//! `HoverCard`), for sighted users: no role, no relation, no focus moved.
//!
//! - **Opening.** The pointer coming onto the trigger, or focus reaching it
//!   (keyboard focus too: reka's 700 ms holds for Tab), asks it to open
//!   after the open delay (700 ms). Several asking keep the first deadline
//!   (hover then focus: `HoverCardRoot`'s `openRequesters`); the pointer
//!   leaving before then drops it.
//! - **Closing.** Leaving the trigger toward the card holds a grace area
//!   ([`crate::hover::Grace`]); leaving that area, leaving the card, or the
//!   trigger losing focus closes it after the close delay (300 ms), which
//!   coming onto the card or the trigger again cancels. A press held in the
//!   card keeps it open, wherever the pointer goes meanwhile (the grace
//!   area's exit included); let go off the card and the trigger, the close
//!   delay starts then. Escape, a press outside, and a scroll of what
//!   holds the trigger close it at once; focus going outside does not.
//! - **Touch.** A touch does nothing by default (reka's `enableTouch`): the
//!   hover a tap makes and the focus it gives the trigger open nothing. With
//!   [`HoverCardOptions::touch`], a tap on the trigger opens or closes it
//!   at once, with no delay.
//! - **Tab order.** Nothing in the card is a Tab stop (reka sets
//!   `tabindex="-1"` on each): the card's layer takes its nodes out of the
//!   order; they still take focus from the pointer and from code.
//!
//! reka also keeps the card open while text in it is selected
//! (`document.getSelection()`), and keeps a drag's selection inside the
//! card. `gui` selects text only in fields, whose selection
//! `getSelection()` does not see either, so there is nothing to keep
//! (recorded in ADR-0025).

use std::rc::Rc;
use std::time::Duration;

use gui::{Dismiss, DismissPolicy, DismissReason, Div, FocusHandle, Interaction, Layer, Placement, Signal, UseModel};

use crate::dialog::Veto;
use crate::hover::{Boxes, Delay, Grace};
use crate::presence::Presence;

/// reka's `openDelay`.
pub const OPEN_DELAY: Duration = Duration::from_millis(700);
/// reka's `closeDelay`.
pub const CLOSE_DELAY: Duration = Duration::from_millis(300);

/// What a hover card's caller chooses (reka's `HoverCardRoot` and
/// `HoverCardContent` props).
#[derive(Clone, Debug, PartialEq)]
pub struct HoverCardOptions {
    pub open_delay: Duration,
    pub close_delay: Duration,
    /// Kept mounted while closed (reka's `forceMount`).
    pub keep_mounted: bool,
    /// A tap on the trigger toggles it at once (reka's `enableTouch`).
    pub touch: bool,
    /// Asked before Escape, a press outside or a scroll closes it.
    pub veto: Option<Veto>,
}

impl Default for HoverCardOptions {
    fn default() -> Self {
        Self {
            open_delay: OPEN_DELAY,
            close_delay: CLOSE_DELAY,
            keep_mounted: false,
            touch: false,
            veto: None,
        }
    }
}

/// A hover card's state: made once in its root's setup (`Copy`).
pub struct HoverCard<P: 'static> {
    open: UseModel<P, bool>,
    options: Signal<Rc<dyn Fn() -> HoverCardOptions>>,
    opening: Delay,
    closing: Delay,
    grace: Grace,
    /// Where the trigger and the card were last placed.
    placed: Signal<Option<Placement>>,
    trigger: FocusHandle,
    /// The card's node: pressed, it keeps the card open.
    card: Interaction,
    /// The pointer is on the trigger.
    trigger_hovered: Signal<bool>,
    /// Where a finger went down on the trigger.
    touched: Signal<Option<(f32, f32)>>,
}

impl<P: 'static> Clone for HoverCard<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for HoverCard<P> {}

impl<P: 'static> PartialEq for HoverCard<P> {
    fn eq(&self, other: &Self) -> bool {
        self.placed == other.placed
    }
}

impl<P: 'static> std::fmt::Debug for HoverCard<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HoverCard").finish_non_exhaustive()
    }
}

impl<P: 'static> HoverCard<P> {
    /// A hover card open as `open` says, as `options` choose; called in its
    /// root's setup, which owns its timers.
    #[must_use]
    #[track_caller]
    pub fn new(open: UseModel<P, bool>, options: impl Fn() -> HoverCardOptions + 'static) -> Self {
        let closing = Delay::new();
        let pressed = Interaction::new();
        let options: Rc<dyn Fn() -> HoverCardOptions> = Rc::new(options);
        // The pointer leaving the grace area: reka's `handleClose`, which a
        // press held in the card refuses, as `request_close` does.
        let close_after = {
            let options = Rc::clone(&options);
            move |()| {
                if gui::untracked(|| pressed.pressed()) {
                    return;
                }
                let delay = gui::untracked(|| options().close_delay);
                closing.after(delay, move || open.set(false));
            }
        };
        let card = Self {
            open,
            options: Signal::new(options),
            opening: Delay::new(),
            closing,
            grace: Grace::new(close_after),
            placed: Signal::labeled("hover card placed", None),
            trigger: FocusHandle::new(),
            card: pressed,
            trigger_hovered: Signal::labeled("hover card trigger hovered", false),
            touched: Signal::new(None),
        };
        // A press held in the card and let go with the pointer on neither
        // the card nor the trigger: the close it held off starts now. reka
        // asks nothing as the press ends, so its card stays open until the
        // pointer next comes and leaves; the press only meant to keep it
        // while held (ADR-0025).
        gui::watch(
            // Read again as it opens, out of setup, where the card's node
            // binds its interaction.
            move || card.open.get() && card.card.pressed(),
            move |&now, &was| {
                if was
                    && !now
                    && card.open.get_untracked()
                    && !gui::untracked(|| card.card.hovered())
                    && !card.trigger_hovered.get_untracked()
                {
                    card.grace.clear();
                    card.request_close();
                }
            },
        );
        // Focus on the trigger asks it to open; losing it, to close.
        gui::watch(
            move || card.trigger.is_focused(),
            move |focused, _| {
                if !*focused {
                    card.request_close();
                } else if card.touched.get_untracked().is_none() {
                    // A touch's focus is the touch's: it opens nothing.
                    card.request_open();
                }
            },
        );
        card
    }

    fn options(self) -> HoverCardOptions {
        let f = self.options.with_untracked(Rc::clone);
        f()
    }

    /// Whether it is open (tracked).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.open.get()
    }

    pub fn set_open(self, open: bool) {
        self.opening.cancel();
        self.closing.cancel();
        self.open.set(open);
    }

    /// Where the card is (tracked).
    #[must_use]
    pub fn presence(self) -> Presence {
        Presence::of(self.is_open(), self.options().keep_mounted)
    }

    /// Where the card was placed, as layout last reported it (tracked):
    /// what its look reads for its side, arrow and origin.
    #[must_use]
    pub fn placement(self) -> Option<Placement> {
        self.placed.get()
    }

    /// Open after the open delay, keeping a deadline already set.
    fn request_open(self) {
        self.closing.cancel();
        if self.opening.pending() || self.open.get_untracked() {
            return;
        }
        let delay = gui::untracked(|| self.options().open_delay);
        let open = self.open;
        self.opening.after(delay, move || open.set(true));
    }

    /// Close after the close delay, unless a press is held in the card.
    fn request_close(self) {
        self.opening.cancel();
        if gui::untracked(|| self.card.pressed()) {
            return;
        }
        let delay = gui::untracked(|| self.options().close_delay);
        let open = self.open;
        self.closing.after(delay, move || open.set(false));
    }

    fn boxes(self) -> Option<Boxes> {
        self.placed.get_untracked().as_ref().map(Boxes::of)
    }

    /// The focus of its trigger (the trigger part tracks it).
    #[must_use]
    pub fn trigger_focus(self) -> FocusHandle {
        self.trigger
    }

    /// `d` as the trigger (its look decides what it is: reka's default is a
    /// link): the pointer on it, or focus in it, opens the card after the
    /// delay; leaving it closes the card as the module's docs say.
    #[must_use]
    pub fn trigger(self, d: Div) -> Div {
        d.track_focus(self.trigger)
            .on_hover(move |inside| {
                self.trigger_hovered.set(inside);
                self.grace.trigger(inside);
                if inside {
                    // A touch's hover is no hover (reka's `pointerType`).
                    if self.touched.get_untracked().is_none() {
                        self.grace.clear();
                        self.request_open();
                    }
                } else if self.open.get_untracked()
                    && let Some(boxes) = self.boxes()
                {
                    // Toward the card: the grace area decides.
                    self.opening.cancel();
                    self.grace.leave(true, boxes);
                } else {
                    self.request_close();
                }
            })
            .on_touch(move |t| {
                use gui::events::TouchPhase;
                if t.kind != gui::PointerKind::Touch {
                    return;
                }
                match t.phase {
                    TouchPhase::Started => self.touched.set(Some((t.x, t.y))),
                    TouchPhase::Moved => {}
                    TouchPhase::Ended => {
                        // A tap: it went up where it went down.
                        if gui::untracked(|| self.options().touch)
                            && let Some((x, y)) = self.touched.get_untracked()
                            && (t.x - x).hypot(t.y - y) <= gui::gesture::SLOP
                        {
                            self.set_open(!self.open.get_untracked());
                        }
                        // Held a moment longer: the focus the tap gave is
                        // the touch's, which opens nothing by itself.
                        let touched = self.touched;
                        gui::clock::after(Duration::from_millis(1), move || {
                            if touched.is_alive() {
                                touched.set(None);
                            }
                        });
                    }
                    TouchPhase::Cancelled => self.touched.set(None),
                }
            })
    }

    /// `d` as the card: the pointer on it keeps it open, leaving it closes
    /// it after the delay (through the grace area toward the trigger); a
    /// layer that Escape, a press outside and a scroll of what holds the
    /// trigger close, focus going outside not. Its node's [`Interaction`]
    /// is the card's (a look reads hover and press from
    /// [`Self::card_interaction`]). Render it as [`Self::presence`] says,
    /// inside the deferred layer [`Self::floating`] makes.
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        let policy = DismissPolicy::DEFAULT.focus_outside(false).scroll(true);
        d.interaction(self.card)
            .on_hover(move |inside| {
                if inside {
                    self.grace.clear();
                    self.closing.cancel();
                } else if let Some(boxes) = self.boxes() {
                    self.grace.leave(false, boxes);
                } else {
                    self.request_close();
                }
            })
            .layer(
                Layer::new()
                    .dismiss(policy)
                    .tab_stops(false)
                    .on_dismiss(move |r| self.dismiss(r)),
            )
    }

    /// `d` as the card's content ([`Self::content`]) on its own layer,
    /// placed beside the trigger as `popper` asks, its placement reported
    /// to [`Self::placement`] (which its grace area reads too).
    #[must_use]
    pub fn floating(self, d: Div, popper: &crate::popper::Popper) -> gui::Deferred {
        gui::deferred(popper.positioning(), self.content(d)).placement(self.placed)
    }

    /// The card node's interaction.
    #[must_use]
    pub fn card_interaction(self) -> Interaction {
        self.card
    }

    fn dismiss(self, reason: &DismissReason) -> Dismiss {
        if !self.options.is_alive() {
            return Dismiss::Veto;
        }
        if *reason != DismissReason::Programmatic
            && let Some(veto) = gui::untracked(|| self.options()).veto
            && veto.call(*reason) == Dismiss::Veto
        {
            return Dismiss::Veto;
        }
        self.grace.clear();
        if self.open.get_untracked() {
            self.set_open(false);
        } else {
            self.opening.cancel();
        }
        Dismiss::Close
    }
}
