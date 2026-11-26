//! Alert dialog (`ui/alert-dialog`): a modal that asks for a decision. Unlike
//! a dialog it has no close button and a press outside does not dismiss it:
//! the user answers through its action or cancel button (or Escape).
//!
//! The behaviour is `primitives::alert_dialog`'s: always modal, read as an
//! alert dialog, focusing its cancel button as it opens. It opens through
//! [`dialog_trigger`](crate::dialog_trigger), and its title and
//! description are [`alert_dialog_title`] and [`alert_dialog_description`]
//! (or the dialog's); it keeps its open state as a dialog does (`open`,
//! `default_open`, `on_open_change`).

use gui::prelude::*;
use gui::{Bind, Div, Element, ElementKey, FocusHandle, Model, component, provide, text, use_model};
use primitives::alert_dialog::{AlertDialog, AlertDialogOptions};
use primitives::presence::Presence;

use crate::button::{Button, ButtonSize, ButtonVariant, button};
use crate::dialog::{
    Modal, ModalContentProps, ModalLook, ModalText, Veto, centred_surface, dialog_description, dialog_footer,
    dialog_header, modal_layer, modal_title,
};
use crate::part::{Part, PartProps, part_root};

/// What [`alert_dialog`] is given.
#[derive(Clone, PartialEq, Default)]
pub struct AlertDialogProps {
    open: Bind<bool>,
    veto: Option<Veto>,
}

/// The alert dialog of an [`alert_dialog`] root.
type AlertState = AlertDialog<PartProps<AlertDialogProps>>;

/// The alert dialog's root: give it a trigger and an [`alert_dialog_content`].
#[must_use]
pub fn alert_dialog() -> Part<AlertDialogProps> {
    Part::new(AlertDialogProps::default(), gui::div().named("alert-dialog"), |p| {
        component(p, |cx| {
            let props = cx.props();
            let d = AlertDialog::new(
                use_model(props, |p: &PartProps<AlertDialogProps>| &p.props.open),
                move || {
                    props.with(|p| AlertDialogOptions {
                        veto: p.props.veto.clone(),
                        ..AlertDialogOptions::default()
                    })
                },
            );
            provide(d);
            provide(Modal::Alert(d));
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

impl Part<AlertDialogProps> {
    /// Open or not, kept by the caller (`v-model:open`).
    #[must_use]
    pub fn open(self, open: impl Into<Model<bool>>) -> Self {
        self.with_props(|p| p.open = p.open.clone().with_model(open))
    }

    /// Open from the start, when no model is given (`default-open`).
    #[must_use]
    pub fn default_open(self, open: bool) -> Self {
        self.with_props(|p| p.open = p.open.clone().with_default(open))
    }

    /// Call `f` each time it opens or closes (`@update:open`).
    #[must_use]
    pub fn on_open_change(self, f: impl Into<gui::Callback<bool>>) -> Self {
        self.with_props(|p| p.open = p.open.clone().on_change(f))
    }

    /// Answer Escape, its one dismissal (`escapeKeyDown` with
    /// `preventDefault`).
    #[must_use]
    pub fn on_dismiss(self, f: impl Into<gui::Callback<gui::DismissReason, gui::Dismiss>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.veto = Some(f))
    }
}

/// The alert dialog, centred on a dimmed page (`AlertDialogContent`); style
/// and fill it as a `Div`.
#[must_use]
#[track_caller]
pub fn alert_dialog_content() -> Part<ModalContentProps> {
    let look = ModalLook {
        centred: true,
        overlay_ms: None,
        close_button: false,
    };
    Part::new(
        ModalContentProps { look },
        centred_surface().named("alert-dialog-content"),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let dialog = part_root::<_, AlertState>(props, "alert_dialog_content", "alert_dialog");
                move || {
                    let Some(d) = dialog else {
                        return Element::Empty;
                    };
                    let presence = d.presence();
                    if presence == Presence::Gone {
                        return Element::Empty;
                    }
                    props.with(|p| modal_layer(presence, true, p.props.look, d.content(p.slot.0.clone())))
                }
            })
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

/// The alert dialog's title, which names it to a screen reader.
pub fn alert_dialog_title(title: impl Into<String>) -> ModalText {
    modal_title(
        text(title)
            .named("alert-dialog-title")
            .text_size(18.0)
            .line_height(28.0)
            .font_weight(600),
    )
}

/// The alert dialog's description, which describes it to a screen reader.
pub fn alert_dialog_description(description: impl Into<String>) -> ModalText {
    dialog_description(description).named("alert-dialog-description")
}

/// The confirming button: a default button that runs `on_action` and closes
/// the dialog; give it its label with `.child`.
#[track_caller]
pub fn alert_dialog_action(on_action: impl Into<gui::Callback<()>>) -> AlertDialogButton {
    let on_action: gui::Callback<()> = on_action.into();
    let at = std::panic::Location::caller();
    AlertDialogButton {
        button: button(ButtonVariant::Default, ButtonSize::Default).named("alert-dialog-action"),
        key: None,
        part: Box::new(move |root, key| {
            keyed(
                Part::new(on_action, root, |p| {
                    component(p, |cx| {
                        let props = cx.props();
                        let dialog = part_root::<_, AlertState>(props, "alert_dialog_action", "alert_dialog");
                        move || {
                            props
                                .with(|p| {
                                    let action = p.props.clone();
                                    match dialog {
                                        Some(d) => d.action(p.slot.0.clone(), action),
                                        None => p.slot.0.clone(),
                                    }
                                })
                                .into_element()
                        }
                    })
                })
                .made_at(at),
                key,
            )
        }),
    }
}

/// The declining button: an outline button that closes the dialog, and
/// takes focus as it opens; give it its label with `.child`.
#[track_caller]
pub fn alert_dialog_cancel() -> AlertDialogButton {
    let at = std::panic::Location::caller();
    AlertDialogButton {
        button: button(ButtonVariant::Outline, ButtonSize::Default).named("alert-dialog-cancel"),
        key: None,
        part: Box::new(|root, key| {
            keyed(
                Part::new((), root, |p| {
                    component(p, |cx| {
                        let props = cx.props();
                        let dialog = part_root::<_, AlertState>(props, "alert_dialog_cancel", "alert_dialog");
                        let focus = FocusHandle::new();
                        if let Some(d) = dialog {
                            d.register_cancel(focus);
                        }
                        move || {
                            let root = props.with(|p| p.slot.0.clone());
                            match dialog {
                                Some(d) => d.cancel(root, focus),
                                None => root,
                            }
                            .into_element()
                        }
                    })
                })
                .made_at(at),
                key,
            )
        }),
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
        n.content_box(gui::inspect::OUTSIDE).x - n.bounds(gui::inspect::OUTSIDE).x
    }

    #[test]
    fn an_icon_given_after_the_part_narrows_its_padding() {
        let action = |with_icon: bool| {
            inset("alert-dialog-action", move || {
                alert_dialog_action(|()| {})
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
