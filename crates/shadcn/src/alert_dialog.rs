//! Alert dialog (`ui/alert-dialog`): a modal that asks for a decision. Unlike
//! a dialog it has no close button and a press outside does not dismiss it:
//! the user answers through its action or cancel button (or Escape).
//!
//! As Reka builds `AlertDialogRoot` on `DialogRoot`, [`alert_dialog`] is a
//! [`dialog`](crate::dialog) root: it opens through
//! [`dialog_trigger`](crate::dialog_trigger) and keeps its open state the
//! same way (`open`, `default_open`, `on_open_change`).

use gui::prelude::*;
use gui::{Div, Text, component, text};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::dialog::{
    DialogContext, DialogProps, ModalContentProps, ModalLook, centred_surface, dialog_close, dialog_description,
    dialog_footer, dialog_header, modal_content, modal_root,
};
use crate::part::Part;

/// The alert dialog's root: give it a trigger and an [`alert_dialog_content`].
#[must_use]
pub fn alert_dialog() -> Part<DialogProps> {
    modal_root(gui::div().named("alert-dialog"))
}

/// The alert dialog, centred on a dimmed page (`AlertDialogContent`); style
/// and fill it as a `Div`.
#[must_use]
pub fn alert_dialog_content() -> Part<ModalContentProps> {
    modal_content(
        centred_surface()
            .named("alert-dialog-content")
            .role(gui::Role::AlertDialog),
        ModalLook {
            centred: true,
            overlay_ms: None,
            dismiss_outside: false,
            close_button: false,
        },
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
pub fn alert_dialog_action(on_action: impl Fn() + 'static) -> Part<gui::Callback<()>> {
    let root = button(ButtonVariant::Default, ButtonSize::Default).named("alert-dialog-action");
    Part::new(gui::Callback::new(move |()| on_action()), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let dialog = DialogContext::find();
            move || {
                props
                    .with(|p| {
                        let action = p.props.clone();
                        p.slot.0.clone().on_click(move |_| {
                            action.call(());
                            if let Some(d) = dialog {
                                d.set_open(false);
                            }
                        })
                    })
                    .into_element()
            }
        })
    })
}

/// The declining button: an outline button that closes the dialog; give it
/// its label with `.child`.
#[must_use]
pub fn alert_dialog_cancel() -> Part<()> {
    dialog_close(button(ButtonVariant::Outline, ButtonSize::Default).named("alert-dialog-cancel"))
}
