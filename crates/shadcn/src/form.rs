//! Form (`ui/form`): the parts shadcn lays a validated field out with: an
//! item holding a label, the control, a description and the message.
//!
//! The state behind them is `primitives::form` (ADR-0025): a [`FormField`]
//! holds one control's state, its rule and the errors last found; [`Checks`]
//! says which events check it; [`form_submit`] checks every field as a
//! submit does, [`form_reset`] puts every value back and [`form_data`] is
//! what the form sends. They are re-exported here, as every shadcn module
//! re-exports the primitive types its parts take, so an application depends
//! on `shadcn` alone.

use gui::element::Node;
use gui::prelude::*;
use gui::{Div, div, text};

pub use primitives::form::{
    Checks, FormData, FormEntry, FormField, FormValue, OnChange, Submit, Unset, form_data, form_reset, form_submit,
};

use crate::focus::StateLook;
use crate::focus::destructive_ring;
use crate::label::label;
use crate::theme::theme;

/// One field of a form: its parts stacked.
#[must_use]
pub fn form_item() -> Div {
    div().named("form-item").display(Display::Grid).gap(8.0)
}

/// The field's name, destructive while it is `invalid`.
#[must_use]
pub fn form_label(name: impl Into<String>, invalid: bool) -> Div {
    let t = theme();
    label(name)
        .named("form-label")
        .when(invalid, |d| d.text_color(t.destructive))
}

/// A control a form marks `invalid` (`FormControl`'s `aria-invalid`).
pub trait FormControl: Sized {
    /// Marked as `aria-invalid` marks it: a destructive border, and a
    /// destructive ring while focused.
    #[must_use]
    fn invalid(self, invalid: bool) -> Self;
}

impl<K> FormControl for Node<K> {
    fn invalid(self, invalid: bool) -> Self {
        let t = theme();
        self.when(invalid, |c| {
            c.border_color(t.destructive).with_interaction(|c, i| {
                c.ring_while(
                    move || i.focus_visible(),
                    &destructive_ring(gui::Style::default(), &t).border_color(t.destructive),
                )
            })
        })
    }
}

impl FormControl for crate::input::Input {
    fn invalid(self, invalid: bool) -> Self {
        self.when(invalid, |c| c.map(move |el| el.invalid(true)))
    }
}

impl FormControl for crate::checkbox::Checkbox {
    fn invalid(self, invalid: bool) -> Self {
        if invalid {
            self.class(|el| el.invalid(true))
        } else {
            self
        }
    }
}

/// `control` (an input, a textarea, a select trigger, a checkbox) marked
/// `invalid` as `aria-invalid` marks it.
#[must_use]
pub fn form_control<C: FormControl>(control: C, invalid: bool) -> C {
    control.invalid(invalid)
}

/// Help text under the control.
#[must_use]
pub fn form_description(description: impl Into<String>) -> Div {
    let t = theme();
    div()
        .named("form-description")
        .text_color(t.muted_foreground)
        .text_size(14.0)
        .line_height(20.0)
        .child(text(description))
}

/// The field's error, when it has one.
#[must_use]
pub fn form_message(error: Option<&str>) -> Option<Div> {
    let t = theme();
    error.map(|e| {
        div()
            .named("form-message")
            .text_color(t.destructive)
            .text_size(14.0)
            .line_height(20.0)
            .child(text(e))
    })
}
