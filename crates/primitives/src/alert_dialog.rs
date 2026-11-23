//! A dialog that asks for a decision (reka's `AlertDialog`): a
//! [`Dialog`](crate::dialog::Dialog) that is always modal, read as an
//! `alertdialog`, closed only by Escape, its Cancel and its Action.
//!
//! - No press and no focus outside it closes it (`AlertDialogContent`
//!   prevents `pointerDownOutside` and `interactOutside`). Its options have
//!   no `modal` and no outside close to set, so neither can be asked for.
//! - Opening focuses its Cancel (the one mounted last; one that unmounts
//!   gives way only if it held the role, as reka's `onCancelElementChange`),
//!   else its first Tab stop. reka focuses the Cancel even when its caller
//!   prevented `openAutoFocus` (its handler reads no `defaultPrevented`);
//!   here a caller keeping focus keeps it (research §4 item 13, recorded in
//!   ADR-0025).
//! - Closing gives focus back as a modal dialog does.

use gui::element::Node;
use gui::{Div, FocusHandle, UseModel};

use crate::dialog::{Dialog, DialogOptions, Kind, Missing, Veto};
use crate::presence::Presence;

/// What an alert dialog's caller chooses.
#[derive(Clone, Debug, PartialEq)]
pub struct AlertDialogOptions {
    /// Kept mounted while closed (reka's `forceMount`).
    pub keep_mounted: bool,
    /// Focus moves to the Cancel (or the first Tab stop) as it opens.
    pub open_auto_focus: bool,
    /// Focus goes back as it closes.
    pub close_auto_focus: bool,
    /// Asked before Escape closes it.
    pub veto: Option<Veto>,
    /// Described by a description part (as [`DialogOptions::described`]).
    pub described: bool,
}

impl Default for AlertDialogOptions {
    fn default() -> Self {
        Self {
            keep_mounted: false,
            open_auto_focus: true,
            close_auto_focus: true,
            veto: None,
            described: true,
        }
    }
}

impl From<AlertDialogOptions> for DialogOptions {
    fn from(o: AlertDialogOptions) -> Self {
        Self {
            modal: true,
            keep_mounted: o.keep_mounted,
            open_auto_focus: o.open_auto_focus,
            close_auto_focus: o.close_auto_focus,
            veto: o.veto,
            described: o.described,
        }
    }
}

/// An alert dialog's state: made once in its root's setup (`Copy`).
pub struct AlertDialog<P: 'static>(Dialog<P>);

impl<P: 'static> Clone for AlertDialog<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for AlertDialog<P> {}

impl<P: 'static> PartialEq for AlertDialog<P> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<P: 'static> std::fmt::Debug for AlertDialog<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("AlertDialog").field(&self.0).finish()
    }
}

impl<P: 'static> AlertDialog<P> {
    /// An alert dialog open as `open` says, as `options` choose; called in
    /// its root's setup.
    #[must_use]
    #[track_caller]
    pub fn new(open: UseModel<P, bool>, options: impl Fn() -> AlertDialogOptions + 'static) -> Self {
        Self(Dialog::of(
            Kind::Alert,
            open,
            std::rc::Rc::new(move || options().into()),
        ))
    }

    /// The dialog it is, for what alert dialogs share with dialogs.
    #[must_use]
    pub fn dialog(self) -> Dialog<P> {
        self.0
    }

    /// Whether it is open (tracked).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.0.is_open()
    }

    pub fn set_open(self, open: bool) {
        self.0.set_open(open);
    }

    /// Where the content is (tracked).
    #[must_use]
    pub fn presence(self) -> Presence {
        self.0.presence()
    }

    /// `d` as the trigger (as [`Dialog::trigger`]).
    #[must_use]
    pub fn trigger(self, d: Div) -> Div {
        self.0.trigger(d)
    }

    /// `d` as the title.
    #[must_use]
    pub fn title<K>(self, d: Node<K>) -> Node<K> {
        self.0.title(d)
    }

    /// `d` as the description.
    #[must_use]
    pub fn description<K>(self, d: Node<K>) -> Node<K> {
        self.0.description(d)
    }

    /// Called in the title part's setup (as [`Dialog::register_title`]).
    pub fn register_title(self) {
        self.0.register_title();
    }

    /// Called in the description part's setup.
    pub fn register_description(self) {
        self.0.register_description();
    }

    /// What the content lacks now (tracked; as [`Dialog::missing`]).
    #[must_use]
    pub fn missing(self) -> Missing {
        self.0.missing()
    }

    /// `d` as the content: an `alertdialog`, the modal layer and trapped
    /// focus scope the module's docs list.
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        self.0.content(d)
    }

    /// Called in a Cancel's setup: it is the button the content focuses as
    /// it opens, while it is mounted (the one mounted last).
    pub fn register_cancel(self, focus: FocusHandle) {
        self.0.register_cancel(focus);
    }

    /// `d` as the Cancel, focused through `focus` (registered in its
    /// setup): pressing it closes the dialog.
    #[must_use]
    pub fn cancel(self, d: Div, focus: FocusHandle) -> Div {
        self.0.close(d.track_focus(focus))
    }

    /// `d` as the Action: pressing it runs `act` (the decision itself,
    /// which is the caller's), then closes the dialog.
    #[must_use]
    pub fn action(self, d: Div, act: impl Into<gui::Callback<()>>) -> Div {
        let act = act.into();
        d.on_click(move |_| {
            act.call(());
            self.set_open(false);
        })
    }
}
