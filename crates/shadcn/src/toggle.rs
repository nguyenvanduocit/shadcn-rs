//! Toggle (`ui/toggle`): a two-state button, pressed or not.
//!
//! As reka's `Toggle` it keeps its own value (`default_pressed`), is bound
//! to the caller's (`model`, `v-model`), or shows one the caller keeps
//! (`pressed`, `:model-value`); `on_change` hears each click.

use gui::Properties;
use gui::prelude::*;
use gui::{Bind, Div, Element, Model, Slot, component, div, use_model};

use crate::focus::focus_ring;
use crate::theme::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToggleVariant {
    #[default]
    Default,
    Outline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToggleSize {
    #[default]
    Default,
    Sm,
    Lg,
}

/// A toggle (`<Toggle>`): released and kept by itself until given a value.
/// A click, or Space while focused, flips it. Add its icon or label with
/// `.child`.
#[must_use]
pub fn toggle(variant: ToggleVariant, size: ToggleSize) -> Toggle {
    Toggle {
        variant,
        size,
        pressed: Bind::default(),
        disabled: false,
        children: Slot(Vec::new()),
    }
}

/// A [`toggle`] being built.
#[derive(Clone, PartialEq)]
pub struct Toggle {
    variant: ToggleVariant,
    size: ToggleSize,
    pressed: Bind<bool>,
    disabled: bool,
    children: Slot<Vec<Element>>,
}

impl Toggle {
    /// Bound both ways to `model` (`v-model`).
    #[must_use]
    pub fn model(mut self, model: impl Into<Model<bool>>) -> Self {
        self.pressed = self.pressed.with_model(model);
        self
    }

    /// Showing `pressed`, which the caller keeps (`:model-value`): a click
    /// only calls [`Toggle::on_change`].
    #[must_use]
    pub fn pressed(mut self, pressed: bool) -> Self {
        self.pressed = self.pressed.with_value(pressed);
        self
    }

    /// Kept by the toggle, starting at `pressed` (`default-value`).
    #[must_use]
    pub fn default_pressed(mut self, pressed: bool) -> Self {
        self.pressed = self.pressed.with_default(pressed);
        self
    }

    /// Call `f` with the value each click turns it to (`@update:model-value`).
    #[must_use]
    pub fn on_change(mut self, f: impl Fn(bool) + 'static) -> Self {
        self.pressed = self.pressed.on_change(f);
        self
    }

    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[must_use]
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.0.push(child.into_element());
        self
    }

    fn render(&self, on: bool, set: impl Fn(bool) + 'static) -> Element {
        toggle_variants(self.variant, self.size, on)
            .disabled(self.disabled)
            .on_click(move |_| set(!on))
            .children(self.children.0.iter().cloned())
            .into_element()
    }
}

impl IntoElement for Toggle {
    fn into_element(self) -> Element {
        // A value the caller keeps: nothing to hold, so no instance.
        if let Some(&on) = self.pressed.as_value() {
            let bind = self.pressed.clone();
            return self.render(on, move |v| bind.emit(v));
        }
        component(self, |cx| {
            let props = cx.props();
            let pressed = use_model(props, |p: &Toggle| &p.pressed);
            move || {
                let on = pressed.get();
                props.with(|p| p.render(on, move |v| pressed.set(v)))
            }
        })
        .named("toggle")
        .into_element()
    }
}

/// `toggleVariants`: the look of a toggle that is `on` or not, without its
/// behaviour; a toggle group's items share it.
#[must_use]
pub fn toggle_variants(variant: ToggleVariant, size: ToggleSize, on: bool) -> Div {
    let t = theme();
    let d = div()
        .named("toggle")
        .role(gui::Role::Button)
        .aria_checked(on)
        .transition_props(Properties::COLOR | Properties::SHADOW)
        .inline_flex()
        .items_center()
        .justify_center()
        .gap(2.0)
        .rounded_md()
        .text_sm()
        .font_medium()
        .whitespace_nowrap()
        .border_color(t.border)
        .focusable()
        .focus_visible(move |s| focus_ring(s, &t))
        .disabled_style(|s| s.opacity(0.5));
    let d = match variant {
        ToggleVariant::Default => d,
        ToggleVariant::Outline => d.border().border_color(t.input).shadow_xs(),
    };
    // `data-[state=on]:` outranks `hover:`, so a pressed toggle keeps its look under the pointer.
    let d = if on {
        d.bg(t.accent).text_color(t.accent_foreground)
    } else {
        match variant {
            ToggleVariant::Default => d.hover(move |s| s.bg(t.muted).text_color(t.muted_foreground)),
            ToggleVariant::Outline => d.hover(move |s| s.bg(t.accent).text_color(t.accent_foreground)),
        }
    };
    match size {
        ToggleSize::Default => d.h(9.0).px(2.0).min_w(9.0),
        ToggleSize::Sm => d.h(8.0).px(1.5).min_w(8.0),
        ToggleSize::Lg => d.h(10.0).px(2.5).min_w(10.0),
    }
}
