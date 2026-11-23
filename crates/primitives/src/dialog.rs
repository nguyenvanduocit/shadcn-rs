//! A window over the page (reka's `Dialog`): its root's state, its trigger,
//! content, title, description and close parts.
//!
//! The rules of a layer are the engine's (ADR-0024); this primitive picks
//! them for a dialog, as reka's `DialogContentModal` and
//! `DialogContentNonModal` do:
//!
//! - **Modal** (the default) is the engine's modal layer: focus trapped and
//!   cycling, the page below inert to the pointer and the wheel, and hidden
//!   from assistive technology. Escape and a press outside it (on its
//!   overlay) close it, but not a secondary press or a Ctrl+press (a context
//!   menu's); focus going outside never does.
//! - **Not modal**: a focus scope that is not trapped but still cycles (Tab
//!   stays in the content, `DialogContentImpl`'s `loop`), and Escape, a press
//!   outside and focus outside close it. A press on its trigger is no press
//!   outside it (the engine's "inside" holds a layer's owner for one that is
//!   not modal), so the trigger toggles it once. Nor is a press on (or focus
//!   coming to) what held focus as it opened, a toggle elsewhere that opened
//!   it by code ([`gui::Layer::opener`], reka's `getFocusedBeforeOpen`): its
//!   click closes it once instead of the press closing it and the click
//!   opening it again.
//! - Each close a user asks for (Escape, a press or focus outside) goes to
//!   the caller's veto first ([`DialogOptions::veto`], reka's
//!   `escapeKeyDown`, `pointerDownOutside`, `focusOutside` and
//!   `interactOutside`, typed by [`DismissReason`]).
//! - **Opening** focuses the content's first Tab stop, else the content
//!   (the engine's `AutoFocus::First`, a text field's text selected); a
//!   caller may keep focus where it is ([`DialogOptions::open_auto_focus`]).
//! - **Closing** gives focus back to what held it as it opened, else to the
//!   trigger, unless something else took focus meanwhile (the engine's
//!   restore rule); a caller may keep it ([`DialogOptions::close_auto_focus`]).
//! - **Kept mounted** ([`DialogOptions::keep_mounted`], reka's `forceMount`):
//!   closed, the content is hidden and inert ([`Presence::Kept`]), so it is
//!   in no layer stack and no focus scope, swallows no Escape, and focuses
//!   again each time it opens.
//!
//! Assistive technology reads the content as a `dialog` named by its title
//! and described by its description (typed relations: a description left
//! out is simply not pointed at). The trigger says whether it is expanded
//! and controls the content only while the content is there (reka's
//! `useTriggerId` with its `MutationObserver`).
//!
//! reka does not set `aria-modal`; neither does this, for the engine hides
//! what is below a modal layer from assistive technology instead (as reka's
//! `hideOthers`). The trigger says it opens a dialog (`aria-haspopup`).
//!
//! **Development warnings** (reka's `useWarning`, `Dialog/utils.ts`): the
//! title and description parts are registered from their setups
//! ([`Dialog::register_title`], [`Dialog::register_description`]); once the
//! content is laid out open, a debug build reports, once per dialog, a
//! content with no title (it would be read unnamed), and one with no
//! description unless its caller said it has none
//! ([`DialogOptions::described`], reka's `aria-describedby="undefined"`).
//! [`Dialog::missing`] tells the same at any time, in every build.

use std::panic::Location;
use std::rc::Rc;

use gui::element::Node;
use gui::focus::FocusId;
use gui::{
    AutoFocus, Callback, Dismiss, DismissPolicy, DismissReason, Div, FocusHandle, FocusScope, Layer, NodeRef,
    PointerButton, Role, Signal, UseModel,
};

use crate::presence::Presence;
use crate::relation::{Linked, Points};

/// A caller's answer to a request to close (reka's `preventDefault` on
/// `escapeKeyDown`, `pointerDownOutside`, `focusOutside`,
/// `interactOutside`): [`Dismiss::Veto`] keeps it open.
pub type Veto = Callback<DismissReason, Dismiss>;

/// What a dialog's caller chooses (reka's `DialogRoot` and `DialogContent`
/// props), read when the content renders and when a request arrives.
#[derive(Clone, Debug, PartialEq)]
pub struct DialogOptions {
    /// Trapped focus, an inert and hidden page (reka: `true`).
    pub modal: bool,
    /// Kept mounted while closed (reka's `forceMount`, `unmountOnHide`
    /// false).
    pub keep_mounted: bool,
    /// Focus moves into the content as it opens (reka's `openAutoFocus`
    /// not prevented).
    pub open_auto_focus: bool,
    /// Focus goes back as it closes (reka's `closeAutoFocus` not
    /// prevented).
    pub close_auto_focus: bool,
    /// Asked before any close a user asks for.
    pub veto: Option<Veto>,
    /// The content is described by a description part (reka: on). Off, it
    /// points at none and no missing description is reported (reka's
    /// `aria-describedby="undefined"`).
    pub described: bool,
}

impl Default for DialogOptions {
    fn default() -> Self {
        Self {
            modal: true,
            keep_mounted: false,
            open_auto_focus: true,
            close_auto_focus: true,
            veto: None,
            described: true,
        }
    }
}

/// What a modal's content lacks to be read as it should (reka's
/// development warnings): no title names it, or no description describes
/// it though its caller did not say it has none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Missing {
    pub title: bool,
    pub description: bool,
}

impl Missing {
    /// Nothing is missing.
    #[must_use]
    pub const fn none(self) -> bool {
        !self.title && !self.description
    }
}

/// What a modal's parts are called in a development warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PartNames {
    pub(crate) content: &'static str,
    pub(crate) title: &'static str,
    pub(crate) description: &'static str,
}

/// The title and description parts a modal's content is named and
/// described by, counted as they mount and unmount (reka's `titleId` and
/// `descriptionId`, looked up with `getElementById` once the content is
/// mounted). Shared by [`Dialog`], [`crate::alert_dialog::AlertDialog`] and
/// [`crate::drawer::Drawer`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Names {
    pub(crate) title: NodeRef,
    pub(crate) description: NodeRef,
    titles: Signal<u32>,
    descriptions: Signal<u32>,
    /// A warning was given: one per modal, not one per opening.
    warned: Signal<bool>,
    /// Where its root was made, which the warning names.
    made_at: &'static Location<'static>,
}

impl Names {
    #[track_caller]
    pub(crate) fn new() -> Self {
        Self {
            title: NodeRef::new(),
            description: NodeRef::new(),
            titles: Signal::labeled("modal titles", 0),
            descriptions: Signal::labeled("modal descriptions", 0),
            warned: Signal::labeled("modal warned", false),
            made_at: Location::caller(),
        }
    }

    /// Count one part in `count` while the calling setup's instance is
    /// mounted.
    fn register(count: Signal<u32>) {
        // Counted from its mount to its unmount: an unmount undoes only a
        // mount that counted.
        gui::while_mounted(move || {
            count.update(|n| *n = n.saturating_add(1));
            move || {
                if count.is_alive() {
                    count.update(|n| *n = n.saturating_sub(1));
                }
            }
        });
    }

    pub(crate) fn register_title(self) {
        Self::register(self.titles);
    }

    pub(crate) fn register_description(self) {
        Self::register(self.descriptions);
    }

    /// What is missing now (tracked); a description only when `described`.
    pub(crate) fn missing(self, described: bool) -> Missing {
        Missing {
            title: self.titles.get() == 0,
            description: described && self.descriptions.get() == 0,
        }
    }

    /// In the root's setup: each time `shown` turns true (and now, if it
    /// is), once the content is laid out, report what is missing, once per
    /// modal, in a debug build (reka warns in development only).
    pub(crate) fn warn_when_shown(
        self,
        names: PartNames,
        shown: impl Fn() -> bool + 'static,
        described: impl Fn() -> bool + 'static,
    ) {
        if !cfg!(debug_assertions) {
            return;
        }
        let described = Rc::new(described);
        let check = move || {
            let described = Rc::clone(&described);
            gui::after_layout(move || {
                if !self.warned.is_alive() || self.warned.get_untracked() {
                    return;
                }
                let missing = gui::untracked(|| self.missing(described()));
                if missing.none() {
                    return;
                }
                self.warned.set(true);
                let at = self.made_at;
                if missing.title {
                    eprintln!(
                        "primitives: a `{}` (its root made at {at}) has no `{}`: a screen reader reads it unnamed. \
                         A title kept off screen (`visually_hidden`) still names it.",
                        names.content, names.title
                    );
                }
                if missing.description {
                    eprintln!(
                        "primitives: a `{}` (its root made at {at}) has no `{}`; one with none says so \
                         (`described: false`, reka's `aria-describedby=\"undefined\"`).",
                        names.content, names.description
                    );
                }
            });
        };
        if gui::untracked(&shown) {
            check();
        }
        gui::watch(shown, move |&now, _| {
            if now {
                check();
            }
        });
    }
}

/// Whether a press outside a modal layer is a context menu's: a secondary
/// press, or a primary one with Control (reka's `DialogContentModal`
/// `pointerDownOutside`: `ctrlLeftClick || isRightClick`).
#[must_use]
pub fn is_context_press(reason: &DismissReason) -> bool {
    matches!(reason, DismissReason::PointerOutside(p)
        if p.button == PointerButton::Secondary
            || (p.button == PointerButton::Primary && p.modifiers.control))
}

/// Which window a [`Dialog`] is: what the content is to assistive
/// technology, and the closes it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    /// reka's `Dialog`.
    Dialog,
    /// reka's `AlertDialog`: always modal, closed by Escape and its buttons
    /// only, focusing its Cancel as it opens.
    Alert,
}

/// A dialog's state: made once in its root's setup (`Copy`), provided to its
/// parts.
pub struct Dialog<P: 'static> {
    open: UseModel<P, bool>,
    options: Signal<Rc<dyn Fn() -> DialogOptions>>,
    linked: Linked,
    names: Names,
    trigger: FocusHandle,
    kind: Kind,
    /// An alert dialog's Cancel, which it focuses as it opens.
    cancel: gui::Holder<FocusHandle>,
    /// What held focus as it last opened (reka's `focusedBeforeOpen`).
    opener: Signal<Option<FocusId>>,
}

impl<P: 'static> Clone for Dialog<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Dialog<P> {}

impl<P: 'static> PartialEq for Dialog<P> {
    fn eq(&self, other: &Self) -> bool {
        self.options == other.options
    }
}

impl<P: 'static> std::fmt::Debug for Dialog<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dialog")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl<P: 'static> Dialog<P> {
    /// A dialog open as `open` says, as `options` choose (read tracked where
    /// the content renders, untracked when a request arrives); called in its
    /// root's setup.
    #[must_use]
    #[track_caller]
    pub fn new(open: UseModel<P, bool>, options: impl Fn() -> DialogOptions + 'static) -> Self {
        Self::of(Kind::Dialog, open, Rc::new(options))
    }

    #[track_caller]
    pub(crate) fn of(kind: Kind, open: UseModel<P, bool>, options: Rc<dyn Fn() -> DialogOptions>) -> Self {
        let d = Self {
            open,
            options: Signal::new(options),
            linked: Linked::new(Points::Controls),
            names: Names::new(),
            trigger: FocusHandle::new(),
            kind,
            cancel: gui::Holder::new(),
            opener: Signal::labeled("dialog opener", None),
        };
        // What holds focus as it opens, read before its content takes focus
        // (reka's `recordFocusBeforeOpen`): a toggle that opened it by code.
        let record = move || d.opener.set(gui::focus::focused());
        if gui::untracked(|| d.is_open()) {
            record();
        }
        gui::watch(
            move || d.is_open(),
            move |&now, _| {
                if now {
                    record();
                }
            },
        );
        // reka's alert dialog warns with the dialog's part names (research
        // §7); here each kind names its own.
        let names = match kind {
            Kind::Dialog => PartNames {
                content: "DialogContent",
                title: "DialogTitle",
                description: "DialogDescription",
            },
            Kind::Alert => PartNames {
                content: "AlertDialogContent",
                title: "AlertDialogTitle",
                description: "AlertDialogDescription",
            },
        };
        d.names.warn_when_shown(
            names,
            move || d.presence() == Presence::Open,
            move || d.options().described,
        );
        d
    }

    /// The caller's options now (tracked).
    #[must_use]
    pub fn options(self) -> DialogOptions {
        let f = self.options.with_untracked(Rc::clone);
        f()
    }

    /// Whether it is open (tracked): what the trigger, overlay and content
    /// look read, a closing one included (its exit plays while this is
    /// already false).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.open.get()
    }

    /// Open or close it, as the caller's code or a part does.
    pub fn set_open(self, open: bool) {
        self.open.set(open);
    }

    /// Where the content is (tracked): open, kept mounted while closed, or
    /// gone.
    #[must_use]
    pub fn presence(self) -> Presence {
        Presence::of(self.is_open(), self.options().keep_mounted)
    }

    /// Whether it is modal (tracked): an alert dialog always is.
    #[must_use]
    pub fn modal(self) -> bool {
        self.kind == Kind::Alert || self.options().modal
    }

    /// The focus of its trigger.
    #[must_use]
    pub fn trigger_focus(self) -> FocusHandle {
        self.trigger
    }

    /// `d` as the trigger: pressing it (a click, Enter, Space) opens the
    /// dialog, or closes it while open (only a dialog that is not modal can
    /// be pressed then); it takes focus, which the dialog gives back to it;
    /// it opens a dialog (`aria-haspopup`), says whether the dialog is
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

    /// `d` as a close button: pressing it closes the dialog.
    #[must_use]
    pub fn close(self, d: Div) -> Div {
        d.on_click(move |_| self.set_open(false))
    }

    /// `d` as the title: a heading (reka's `h2`), the content's name. Its
    /// part calls [`Self::register_title`] in its setup.
    #[must_use]
    pub fn title<K>(self, d: Node<K>) -> Node<K> {
        d.node_ref(self.names.title).role(Role::Heading).aria_level(2)
    }

    /// `d` as the description: what the content is described by. Its part
    /// calls [`Self::register_description`] in its setup.
    #[must_use]
    pub fn description<K>(self, d: Node<K>) -> Node<K> {
        d.node_ref(self.names.description)
    }

    /// Called in the title part's setup: the content has a title while
    /// that instance is mounted.
    pub fn register_title(self) {
        self.names.register_title();
    }

    /// Called in the description part's setup.
    pub fn register_description(self) {
        self.names.register_description();
    }

    /// What the content lacks now (tracked): no title, or no description
    /// though [`DialogOptions::described`] is on. A debug build reports it
    /// once the content is laid out open.
    #[must_use]
    pub fn missing(self) -> Missing {
        let described = self.options().described;
        self.names.missing(described)
    }

    /// Whether the overlay shows (tracked): only behind a modal dialog
    /// (reka's `DialogOverlay`).
    #[must_use]
    pub fn has_overlay(self) -> bool {
        self.modal()
    }

    /// `d` as the content: a `dialog` (an `alertdialog` for an alert
    /// dialog), named by its title, described by its description, the
    /// dismissable layer and focus scope the module's docs list. Render it
    /// as [`Self::presence`] says, inside a deferred layer.
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        let role = match self.kind {
            Kind::Dialog => Role::Dialog,
            Kind::Alert => Role::AlertDialog,
        };
        let d = self.linked.content(d, false).role(role).labelled_by(self.names.title);
        let d = if self.options().described {
            d.described_by(self.names.description)
        } else {
            d
        };
        d.layer(self.layer())
    }

    /// The content's layer, as the options now say (tracked).
    fn layer(self) -> Layer {
        let o = self.options();
        let modal = self.modal();
        let policy = match (self.kind, modal) {
            // `AlertDialogContent` prevents every press and interaction
            // outside: Escape is its only dismissal.
            (Kind::Alert, _) => DismissPolicy::NONE.escape(true),
            // `DialogContentModal` prevents focus outside.
            (Kind::Dialog, true) => DismissPolicy::NONE.escape(true).pointer_outside(true),
            (Kind::Dialog, false) => DismissPolicy::DEFAULT,
        };
        let auto_focus = match (o.open_auto_focus, self.kind, self.cancel.get()) {
            // A caller keeping focus wins, an alert dialog's Cancel too:
            // reka's `AlertDialogContent` focuses its Cancel whether or not
            // `openAutoFocus` was prevented (research §7, a reka bug).
            (false, ..) => AutoFocus::Keep,
            (true, Kind::Alert, Some(cancel)) => AutoFocus::Node(cancel),
            (true, ..) => AutoFocus::First,
        };
        let scope = FocusScope::new()
            .trap(modal)
            .auto_focus(auto_focus)
            .restore(o.close_auto_focus);
        Layer::new()
            .modal(modal)
            .opener(self.opener.get())
            .dismiss(policy)
            .focus_scope(scope)
            .on_dismiss(move |reason| self.dismiss(reason))
    }

    /// Answer a request to close.
    fn dismiss(self, reason: &DismissReason) -> Dismiss {
        if !self.options.is_alive() {
            return Dismiss::Veto;
        }
        let o = gui::untracked(|| self.options());
        if (self.kind == Kind::Alert || o.modal) && is_context_press(reason) {
            return Dismiss::Veto;
        }
        if *reason != DismissReason::Programmatic
            && let Some(veto) = &o.veto
            && veto.call(*reason) == Dismiss::Veto
        {
            return Dismiss::Veto;
        }
        self.set_open(false);
        Dismiss::Close
    }

    /// Called in an alert dialog's Cancel's setup: `focus` is what the
    /// content focuses as it opens. The Cancel mounted last holds it; one
    /// unmounting gives it up only if it holds it, so another Cancel still
    /// there keeps it (reka's `onCancelElementChange`).
    pub(crate) fn register_cancel(self, focus: FocusHandle) {
        // Held from its mount to its unmount, by its registration: a Cancel
        // that replaced it (the same `focus` or not) keeps it.
        self.cancel.hold(focus);
    }
}

#[cfg(test)]
mod tests {
    use gui::{Modifiers, PointerKind, PointerOutside};

    use super::*;

    fn press(button: PointerButton, control: bool) -> DismissReason {
        DismissReason::PointerOutside(PointerOutside {
            x: 0.0,
            y: 0.0,
            kind: PointerKind::Mouse,
            button,
            modifiers: Modifiers {
                control,
                ..Modifiers::default()
            },
        })
    }

    #[test]
    fn a_context_menus_press_is_a_secondary_or_a_control_press() {
        assert!(is_context_press(&press(PointerButton::Secondary, false)));
        assert!(is_context_press(&press(PointerButton::Primary, true)));
        assert!(!is_context_press(&press(PointerButton::Primary, false)));
        assert!(!is_context_press(&press(PointerButton::Middle, false)));
        assert!(!is_context_press(&DismissReason::Escape));
    }
}
