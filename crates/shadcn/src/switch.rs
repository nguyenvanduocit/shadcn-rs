//! Switch (`ui/switch`): an on/off control with a sliding thumb.
//!
//! As reka's `SwitchRoot` it keeps its own value (`default_checked`), is
//! bound to the caller's (`model`, `v-model`), or shows one the caller keeps
//! (`checked`, `:model-value`); `on_change` hears each flip.

use std::rc::Rc;

use gui::prelude::*;
use gui::style::Calc;
use gui::{Bind, Color, Div, Element, Model, Slot, component, div, use_model};

use crate::checkbox::Class;
use crate::focus::StateLook;
use crate::theme::theme;

/// A switch (`<Switch>`): off and kept by itself until given a value. A
/// click, or Space while focused, flips it.
#[must_use]
pub fn switch() -> Switch {
    Switch {
        checked: Bind::default(),
        disabled: false,
        class: None,
    }
}

/// A [`switch`] being built.
#[derive(Clone, PartialEq)]
pub struct Switch {
    checked: Bind<bool>,
    disabled: bool,
    class: Option<Class>,
}

impl Switch {
    /// Bound both ways to `model` (`v-model`).
    #[must_use]
    pub fn model(mut self, model: impl Into<Model<bool>>) -> Self {
        self.checked = self.checked.with_model(model);
        self
    }

    /// Showing `checked`, which the caller keeps (`:model-value`): a click
    /// only calls [`Switch::on_change`].
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = self.checked.with_value(checked);
        self
    }

    /// Kept by the switch, starting at `checked` (`default-value`).
    #[must_use]
    pub fn default_checked(mut self, checked: bool) -> Self {
        self.checked = self.checked.with_default(checked);
        self
    }

    /// Call `f` with the value each flip turns it to (`@update:model-value`).
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

    /// Classes after the switch's own, its state's included.
    #[must_use]
    pub fn class(mut self, f: impl Fn(Div) -> Div + 'static) -> Self {
        self.class = Some(Slot(Rc::new(f)));
        self
    }

    fn render(&self, on: bool, set: impl Fn(bool) + 'static) -> Element {
        let d = look(on).disabled(self.disabled).on_click(move |_| set(!on));
        match &self.class {
            Some(f) => f(d),
            None => d,
        }
        .into_element()
    }
}

impl IntoElement for Switch {
    fn into_element(self) -> Element {
        // A value the caller keeps: nothing to hold, so no instance.
        if let Some(&on) = self.checked.as_value() {
            let bind = self.checked.clone();
            return self.render(on, move |v| bind.emit(v));
        }
        component(self, |cx| {
            let props = cx.props();
            let checked = use_model(props, |p: &Switch| &p.checked);
            move || {
                let on = checked.get();
                props.with(|p| p.render(on, move |v| checked.set(v)))
            }
        })
        .named("switch")
        .into_element()
    }
}

/// The track and thumb, on or off.
fn look(on: bool) -> Div {
    let t = theme();
    let track = match (on, t.dark) {
        (true, _) => t.primary,
        (false, false) => t.input,
        (false, true) => t.input.opacity(0.8),
    };
    let thumb = match (on, t.dark) {
        (_, false) => t.background,
        (false, true) => t.foreground,
        (true, true) => t.primary_foreground,
    };
    let i = Interaction::new();
    div()
        .named("switch")
        .interaction(i)
        .role(gui::Role::Switch)
        .aria_checked(on)
        .display(Display::Flex)
        .inline(true)
        .height(18.4)
        .width(32.0)
        .flex_shrink(0.0)
        .align_items(Align::Center)
        .radius(9999.0)
        .border(1.0)
        .border_color(Color::TRANSPARENT)
        .shadow(Shadow {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: 0.0,
            color: Color::BLACK.opacity(0.05),
        })
        .background(track)
        .transition(Properties::ALL)
        .focusable()
        .focus_ring_on(i, &t)
        .dim_disabled(i, 0.5)
        .cursor_disabled(i)
        .child(
            div()
                .named("switch-thumb")
                .pass_through()
                .display(Display::Block)
                .size(16.0)
                .radius(9999.0)
                .background(thumb)
                .transition(Properties::TRANSFORM)
                // `translate-x-[calc(100%-2px)]`: the thumb's width less 2px.
                .when(on, |d| {
                    d.translate_x(Calc {
                        fraction: 1.0,
                        px: -2.0,
                        ch: 0.0,
                    })
                }),
        )
}

impl crate::form::FormControl for Switch {
    fn invalid(self, invalid: bool) -> Self {
        if invalid {
            self.class(|el| crate::form::FormControl::invalid(el, true))
        } else {
            self
        }
    }
}
