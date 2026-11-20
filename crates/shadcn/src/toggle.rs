//! Toggle (`ui/toggle`): a two-state button, pressed or not.

use gui::Properties;
use gui::prelude::*;
use gui::{Div, Signal, div};

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

/// A toggle bound to `pressed`; a click, or Space while focused, flips it.
/// Add its icon or label with `.child`; disable it with `.disabled(true)`.
#[must_use]
pub fn toggle(variant: ToggleVariant, size: ToggleSize, pressed: &Signal<bool>) -> Div {
    let flip = pressed.clone();
    toggle_variants(variant, size, pressed.get()).on_click(move |_| flip.update(|p| !p))
}

/// `toggleVariants`: the look of a toggle that is `on` or not, without its
/// behaviour; a toggle group's items share it.
#[must_use]
pub fn toggle_variants(variant: ToggleVariant, size: ToggleSize, on: bool) -> Div {
    let t = theme();
    let d = div()
        .named("toggle")
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
