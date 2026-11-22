//! Checkbox (`ui/checkbox`): a box that is checked, not, or indeterminate.
//!
//! As reka's `CheckboxRoot` it keeps its own value (`default_checked`), is
//! bound to the caller's (`model`, `v-model`), or shows one the caller
//! keeps (`checked`, `:model-value`); `on_change` hears each click.

use std::rc::Rc;

use gui::prelude::*;
use gui::{Bind, Div, Element, Icon, Model, Slot, component, div, icon, use_model};

use crate::focus::focus_ring;
use crate::theme::theme;

/// What a checkbox shows: reka's `boolean | 'indeterminate'`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedState {
    Unchecked,
    Checked,
    /// Some of what it stands for is checked (a "select all" over a part).
    Indeterminate,
}

impl From<bool> for CheckedState {
    fn from(checked: bool) -> Self {
        if checked { Self::Checked } else { Self::Unchecked }
    }
}

/// Classes a caller adds after a control's own (Vue's `class` prop), run in
/// the control's render: what it reads there renders the control again.
pub type Class = Slot<Rc<dyn Fn(Div) -> Div>>;

/// A checkbox (`<Checkbox>`): unchecked and kept by itself until given a
/// value. A click, or Space while focused, checks it unless it is checked
/// (reka checks an indeterminate one).
#[must_use]
pub fn checkbox() -> Checkbox {
    Checkbox {
        checked: Bind::default(),
        indeterminate: false,
        disabled: false,
        class: None,
    }
}

/// A [`checkbox`] being built.
#[derive(Clone, PartialEq)]
pub struct Checkbox {
    checked: Bind<bool>,
    indeterminate: bool,
    disabled: bool,
    class: Option<Class>,
}

impl Checkbox {
    /// Bound both ways to `model` (`v-model`).
    #[must_use]
    pub fn model(mut self, model: impl Into<Model<bool>>) -> Self {
        self.checked = self.checked.with_model(model);
        self
    }

    /// Showing `checked`, which the caller keeps (`:model-value`): a click
    /// only calls [`Checkbox::on_change`].
    #[must_use]
    pub fn checked(mut self, checked: impl Into<CheckedState>) -> Self {
        let state = checked.into();
        self.indeterminate = state == CheckedState::Indeterminate;
        self.checked = self.checked.with_value(state == CheckedState::Checked);
        self
    }

    /// Kept by the checkbox, starting at `checked` (`default-value`).
    #[must_use]
    pub fn default_checked(mut self, checked: bool) -> Self {
        self.checked = self.checked.with_default(checked);
        self
    }

    /// Showing the indeterminate mark while unchecked.
    #[must_use]
    pub fn indeterminate(mut self, indeterminate: bool) -> Self {
        self.indeterminate = indeterminate;
        self
    }

    /// Call `f` with the value each click turns it to (`@update:model-value`).
    #[must_use]
    pub fn on_change(mut self, f: impl Fn(bool) + 'static) -> Self {
        self.checked = self.checked.on_change(f);
        self
    }

    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Classes after the checkbox's own, its state's included.
    #[must_use]
    pub fn class(mut self, f: impl Fn(Div) -> Div + 'static) -> Self {
        self.class = Some(Slot(Rc::new(f)));
        self
    }
}

impl Checkbox {
    fn render(&self, on: bool, set: impl Fn(bool) + 'static) -> Element {
        let state = match (on, self.indeterminate) {
            (true, _) => CheckedState::Checked,
            (false, true) => CheckedState::Indeterminate,
            (false, false) => CheckedState::Unchecked,
        };
        let d = look(state)
            .disabled(self.disabled)
            .on_click(move |_| set(state != CheckedState::Checked));
        match &self.class {
            Some(f) => f(d),
            None => d,
        }
        .into_element()
    }
}

impl IntoElement for Checkbox {
    fn into_element(self) -> Element {
        // A value the caller keeps: nothing to hold, so no instance.
        if let Some(&on) = self.checked.as_value() {
            let bind = self.checked.clone();
            return self.render(on, move |v| bind.emit(v));
        }
        component(self, |cx| {
            let props = cx.props();
            let checked = use_model(props, |p: &Checkbox| &p.checked);
            move || {
                let on = checked.get();
                props.with(|p| p.render(on, move |v| checked.set(v)))
            }
        })
        .named("checkbox")
        .into_element()
    }
}

/// The box for `state`.
fn look(state: CheckedState) -> Div {
    let t = theme();
    let base = div()
        .named("checkbox")
        .role(gui::Role::CheckBox)
        .aria_checked(match state {
            CheckedState::Checked => gui::Toggled::True,
            CheckedState::Indeterminate => gui::Toggled::Mixed,
            CheckedState::Unchecked => gui::Toggled::False,
        })
        .transition_shadow()
        .flex()
        .items_center()
        .justify_center()
        .size(4.0)
        .shrink_0()
        .rounded(4.0)
        .border()
        .border_color(t.input)
        .shadow_xs()
        .focusable()
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed());
    let indicator = || icon(Icon::Check).named("checkbox-indicator").size(3.5);
    match state {
        // `data-[state=checked]:border-primary` outranks `focus-visible:border-ring`.
        CheckedState::Checked => base
            .bg(t.primary)
            .text_color(t.primary_foreground)
            .border_color(t.primary)
            .focus_visible(move |s| s.ring(3.0, t.ring.opacity(0.5)))
            .child(indicator()),
        // The indicator shows, but the `checked` fill and colour do not.
        CheckedState::Indeterminate => base.focus_visible(move |s| focus_ring(s, &t)).child(indicator()),
        CheckedState::Unchecked => base.focus_visible(move |s| focus_ring(s, &t)),
    }
}
