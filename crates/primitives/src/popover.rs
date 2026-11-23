//! A floating panel beside its trigger (reka's `Popover`): its root's
//! state, its trigger, content and close parts.
//!
//! - **Not modal** (the default): a focus scope that is not trapped but
//!   cycles (Tab stays in the content, `PopoverContentImpl`'s `loop`), and
//!   Escape, a press outside and focus outside close it. A press on its
//!   trigger is not outside it, so the trigger toggles it once.
//! - **Modal**: the engine's modal layer (focus trapped, the page inert to
//!   the pointer and the wheel and hidden from assistive technology); focus
//!   outside never closes it. Unlike a dialog, a secondary press outside
//!   does close it (`PopoverContentModal` vetoes no right click), and then
//!   gives no focus back (the engine's rule for a press that is not a
//!   primary one outside a modal layer).
//! - **Opening** focuses the content's first Tab stop, else the content
//!   (with nothing to tab to, the content holding focus is not focus
//!   outside it); a caller may keep focus where it is.
//! - **Closing** gives focus to the **trigger**, not to what held it as it
//!   opened (`PopoverContentModal` and `PopoverContentNonModal` focus
//!   `triggerElement`), unless something else took focus meanwhile, or the
//!   user pressed outside a popover that is not modal.
//! - Each close a user asks for goes to the caller's veto first.
//!
//! Assistive technology reads the content as a `dialog` named by its
//! trigger (`aria-labelledby` the trigger, as reka); the trigger says
//! whether it is expanded and controls the content while it is there.
//!
//! Its place beside the trigger is [`crate::popper::Popper`]'s, which the
//! caller gives to the deferred layer (reka: bottom, centred, no offset):
//! [`Popover::floating`] places it against the node that declares it, or
//! against the [`Popover::anchor`] part when there is one (reka's
//! `PopoverAnchor`: a box to place against other than the trigger, which
//! stays the toggle).

use std::rc::Rc;

use gui::{
    AutoFocus, Dismiss, DismissPolicy, DismissReason, Div, FocusHandle, FocusScope, Layer, Role, Signal, UseModel,
};

use crate::dialog::Veto;
use crate::presence::Presence;
use crate::relation::{Linked, Points};

/// What a popover's caller chooses (reka's `PopoverRoot` and
/// `PopoverContent` props).
#[derive(Clone, Debug, PartialEq)]
pub struct PopoverOptions {
    /// Trapped focus, an inert and hidden page (reka: `false`).
    pub modal: bool,
    /// Kept mounted while closed (reka's `forceMount`).
    pub keep_mounted: bool,
    /// Focus moves into the content as it opens (reka's `openAutoFocus`
    /// not prevented).
    pub open_auto_focus: bool,
    /// Focus goes to the trigger as it closes (reka's `closeAutoFocus`
    /// not prevented).
    pub close_auto_focus: bool,
    /// Asked before any close a user asks for.
    pub veto: Option<Veto>,
}

impl Default for PopoverOptions {
    fn default() -> Self {
        Self {
            modal: false,
            keep_mounted: false,
            open_auto_focus: true,
            close_auto_focus: true,
            veto: None,
        }
    }
}

/// A popover's state: made once in its root's setup (`Copy`).
pub struct Popover<P: 'static> {
    open: UseModel<P, bool>,
    options: Signal<Rc<dyn Fn() -> PopoverOptions>>,
    linked: Linked,
    trigger: FocusHandle,
    /// An anchor part is rendered: the content is placed against it.
    /// Anchor parts mounted (a count: one replacing another registers
    /// before the one it replaces leaves).
    anchored: Signal<u32>,
}

/// The key the anchor part's node is found by, inside the node that
/// declares the content.
pub const ANCHOR_KEY: gui::ElementKey = 0x0070_6f70_616e_6368;

impl<P: 'static> Clone for Popover<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Popover<P> {}

impl<P: 'static> PartialEq for Popover<P> {
    fn eq(&self, other: &Self) -> bool {
        self.options == other.options
    }
}

impl<P: 'static> std::fmt::Debug for Popover<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Popover").finish_non_exhaustive()
    }
}

impl<P: 'static> Popover<P> {
    /// A popover open as `open` says, as `options` choose; called in its
    /// root's setup.
    #[must_use]
    #[track_caller]
    pub fn new(open: UseModel<P, bool>, options: impl Fn() -> PopoverOptions + 'static) -> Self {
        Self {
            open,
            options: Signal::new(Rc::new(options)),
            linked: Linked::new(Points::Controls),
            trigger: FocusHandle::new(),
            anchored: Signal::labeled("popover anchored", 0),
        }
    }

    /// Make an anchor part known, from its setup: while it is mounted the
    /// content is placed against it (reka's `PopoverAnchor`).
    pub fn register_anchor(self) {
        let anchored = self.anchored;
        // Counted from its mount to its unmount (a setup, as a render,
        // writes only what it made): an unmount undoes only a mount that
        // counted.
        gui::while_mounted(move || {
            anchored.update(|n| *n = n.saturating_add(1));
            move || {
                if anchored.is_alive() {
                    anchored.update(|n| *n = n.saturating_sub(1));
                }
            }
        });
    }

    /// `d` as the anchor part's node (its key is [`ANCHOR_KEY`]): render it
    /// inside the node that declares the content, after
    /// [`Self::register_anchor`].
    #[must_use]
    pub fn anchor(self, d: Div) -> Div {
        d.key(ANCHOR_KEY)
    }

    /// `d` as the content ([`Self::content`]) on its own layer, placed as
    /// `popper` asks against the anchor part, else against the node that
    /// declares it.
    #[must_use]
    pub fn floating(self, d: Div, popper: &crate::popper::Popper) -> gui::Deferred {
        let layer = gui::deferred(popper.positioning(), self.content(d));
        if self.anchored.get() > 0 {
            layer.anchor_part(ANCHOR_KEY)
        } else {
            layer
        }
    }

    /// The caller's options now (tracked).
    #[must_use]
    pub fn options(self) -> PopoverOptions {
        let f = self.options.with_untracked(Rc::clone);
        f()
    }

    /// Whether it is open (tracked).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.open.get()
    }

    pub fn set_open(self, open: bool) {
        self.open.set(open);
    }

    /// Where the content is (tracked).
    #[must_use]
    pub fn presence(self) -> Presence {
        Presence::of(self.is_open(), self.options().keep_mounted)
    }

    /// The focus of its trigger (the trigger part tracks it).
    #[must_use]
    pub fn trigger_focus(self) -> FocusHandle {
        self.trigger
    }

    /// `d` as the trigger: pressing it toggles the popover; it takes focus,
    /// which the popover gives back to it; it says whether the popover is
    /// expanded and controls it while open.
    #[must_use]
    pub fn trigger(self, d: Div) -> Div {
        let open = self.is_open();
        self.linked
            .trigger(d.track_focus(self.trigger), open)
            .aria_has_popup(gui::HasPopup::Dialog)
            .aria_expanded(open)
            .on_click(move |_| self.set_open(!self.open.get_untracked()))
    }

    /// `d` as a close button: pressing it closes the popover.
    #[must_use]
    pub fn close(self, d: Div) -> Div {
        d.on_click(move |_| self.set_open(false))
    }

    /// `d` as the content: a `dialog` named by the trigger, the layer and
    /// focus scope the module's docs list. Render it as [`Self::presence`]
    /// says, inside the deferred layer [`crate::popper::Popper`] places.
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        self.linked.content(d, true).role(Role::Dialog).layer(self.layer())
    }

    fn layer(self) -> Layer {
        let o = self.options();
        let policy = if o.modal {
            DismissPolicy::NONE.escape(true).pointer_outside(true)
        } else {
            DismissPolicy::DEFAULT
        };
        let auto_focus = if o.open_auto_focus {
            AutoFocus::First
        } else {
            AutoFocus::Keep
        };
        let scope = FocusScope::new()
            .trap(o.modal)
            .auto_focus(auto_focus)
            .restore(o.close_auto_focus)
            .return_to(self.trigger);
        Layer::new()
            .modal(o.modal)
            .dismiss(policy)
            .focus_scope(scope)
            .on_dismiss(move |reason| self.dismiss(reason))
    }

    fn dismiss(self, reason: &DismissReason) -> Dismiss {
        if !self.options.is_alive() {
            return Dismiss::Veto;
        }
        let o = gui::untracked(|| self.options());
        if *reason != DismissReason::Programmatic
            && let Some(veto) = &o.veto
            && veto.call(*reason) == Dismiss::Veto
        {
            return Dismiss::Veto;
        }
        self.set_open(false);
        Dismiss::Close
    }
}
