//! Alert dialog (`ui/alert-dialog`): a modal that asks for a decision. Unlike
//! a dialog it has no close button and a press outside does not dismiss it:
//! the user answers through its action or cancel button (or Escape).
//!
//! As Reka builds `AlertDialogRoot` on `DialogRoot`, [`alert_dialog`] is a
//! [`dialog`](crate::dialog) root: it opens through
//! [`dialog_trigger`](crate::dialog_trigger) and keeps its open state the
//! same way (`open`, `default_open`, `on_open_change`).

use gui::prelude::*;
use gui::{Div, Element, ElementKey, Text, component, text};

use crate::button::{Button, ButtonSize, ButtonVariant, button};
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
    text(title)
        .named("alert-dialog-title")
        .text_size(18.0)
        .line_height(28.0)
        .font_weight(600)
}

#[must_use]
pub fn alert_dialog_description(description: impl Into<String>) -> Text {
    dialog_description(description).named("alert-dialog-description")
}

/// The confirming button: a default button that runs `on_action` and closes
/// the dialog; give it its label with `.child`.
pub fn alert_dialog_action(on_action: impl Fn() + 'static) -> AlertDialogButton {
    let on_action = gui::Callback::new(move |()| on_action());
    AlertDialogButton {
        button: button(ButtonVariant::Default, ButtonSize::Default).named("alert-dialog-action"),
        key: None,
        part: Box::new(move |root, key| {
            keyed(
                Part::new(on_action, root, |p| {
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
                }),
                key,
            )
        }),
    }
}

/// The declining button: an outline button that closes the dialog; give it
/// its label with `.child`.
pub fn alert_dialog_cancel() -> AlertDialogButton {
    AlertDialogButton {
        button: button(ButtonVariant::Outline, ButtonSize::Default).named("alert-dialog-cancel"),
        key: None,
        part: Box::new(|root, key| keyed(dialog_close(root), key)),
    }
}

/// An [`alert_dialog_action`] or [`alert_dialog_cancel`] being built: a
/// [`Button`] that becomes its part when it goes into a tree, so the padding
/// is decided after every child is given (an icon child narrows it, shadcn's
/// `has-[>svg]:px-3`).
#[must_use]
pub struct AlertDialogButton {
    button: Button,
    key: Option<ElementKey>,
    /// Makes the part from the finished box and the key.
    part: Box<dyn FnOnce(Div, Option<ElementKey>) -> Element>,
}

impl AlertDialogButton {
    /// Add a child, as [`Button::child`].
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.button = self.button.child(child);
        self
    }

    /// Add children, as [`Button::children`].
    pub fn children<E: IntoElement>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.button = self.button.children(children);
        self
    }

    /// The name the box shows in the debug tools and the verdict.
    pub fn named(mut self, name: &'static str) -> Self {
        self.button = self.button.named(name);
        self
    }

    /// Its identity among its siblings.
    pub fn key(mut self, key: impl Into<ElementKey>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Fluent for AlertDialogButton {}

impl Styled for AlertDialogButton {
    fn style_and_bindings(&mut self) -> (&mut gui::Style, Option<&mut gui::value::Bindings>) {
        self.button.style_and_bindings()
    }

    fn node_interaction(&self) -> Option<gui::Interaction> {
        self.button.node_interaction()
    }
}

impl IntoElement for AlertDialogButton {
    fn into_element(self) -> Element {
        (self.part)(self.button.into_div(), self.key)
    }
}

fn keyed<P: 'static>(part: Part<P>, key: Option<ElementKey>) -> Element {
    match key {
        Some(key) => part.key(key).into_element(),
        None => part.into_element(),
    }
}

#[cfg(test)]
mod tests {
    use gui::{Icon, Scenario, icon};

    use super::*;

    /// The inset of the named node's content box from its left edge.
    fn inset(name: &'static str, el: impl Fn() -> Element + 'static) -> f32 {
        let mut s = Scenario::measuring(el, 400, 200, 1.0).expect("a scenario");
        s.settle().expect("settled");
        let n = s
            .last()
            .nodes
            .iter()
            .find(|n| n.name == Some(name))
            .expect("the button");
        n.content_box.x - n.bounds.x
    }

    #[test]
    fn an_icon_given_after_the_part_narrows_its_padding() {
        let action = |with_icon: bool| {
            inset("alert-dialog-action", move || {
                alert_dialog_action(|| {})
                    .when(with_icon, |b| b.child(icon(Icon::Trash)))
                    .child("Delete")
                    .into_element()
            })
        };
        assert_eq!(action(false), 16.0);
        assert_eq!(action(true), 12.0);
        // The outline's 1px border, then the padding.
        let cancel = inset("alert-dialog-cancel", || {
            alert_dialog_cancel()
                .child(icon(Icon::X))
                .child("Cancel")
                .into_element()
        });
        assert_eq!(cancel, 13.0);
    }
}
