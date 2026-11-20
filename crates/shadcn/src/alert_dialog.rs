//! Alert dialog (`ui/alert-dialog`): a modal that asks for a decision. Unlike
//! a dialog it has no close button and a press outside does not dismiss it:
//! the user answers through its action or cancel button (or Escape).
//!
//! It opens and closes through a [`DialogState`] and [`dialog_trigger`](crate::dialog_trigger),
//! as Reka builds `AlertDialogRoot` on `DialogRoot`.

use gui::prelude::*;
use gui::{Div, Element, Text, text};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::dialog::{
    DialogState, centred_modal, centred_surface, dialog_close, dialog_description, dialog_footer, dialog_header,
};

/// The alert dialog, centred on a dimmed page; `build` adds the classes and
/// children a caller gives `AlertDialogContent`.
#[must_use]
pub fn alert_dialog_content(state: &DialogState, build: impl FnOnce(Div) -> Div) -> Element {
    if !state.is_open() {
        return Element::Empty;
    }
    centred_modal(
        state,
        build(
            centred_surface()
                .named("alert-dialog-content")
                .role(gui::Role::AlertDialog),
        ),
        false,
    )
}

#[must_use]
pub fn alert_dialog_header() -> Div {
    dialog_header().named("alert-dialog-header")
}

#[must_use]
pub fn alert_dialog_footer() -> Div {
    dialog_footer().named("alert-dialog-footer")
}

#[must_use]
pub fn alert_dialog_title(title: impl Into<String>) -> Text {
    text(title).named("alert-dialog-title").text_lg().font_semibold()
}

#[must_use]
pub fn alert_dialog_description(description: impl Into<String>) -> Text {
    dialog_description(description).named("alert-dialog-description")
}

/// The confirming button: a default button that runs `on_action` and closes
/// the dialog; give it its label with `.child`.
#[must_use]
pub fn alert_dialog_action(state: &DialogState, on_action: impl Fn() + 'static) -> Div {
    let state = state.clone();
    button(ButtonVariant::Default, ButtonSize::Default)
        .named("alert-dialog-action")
        .on_click(move |_| {
            on_action();
            state.close();
        })
}

/// The declining button: an outline button that closes the dialog; give it
/// its label with `.child`.
#[must_use]
pub fn alert_dialog_cancel(state: &DialogState) -> Div {
    dialog_close(
        state,
        button(ButtonVariant::Outline, ButtonSize::Default).named("alert-dialog-cancel"),
    )
}
