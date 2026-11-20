//! Kbd (`ui/kbd`): a keyboard key, and a group of keys.

use gui::{Div, Styled, div};

use crate::theme::theme;

/// One key; put its label (text, or an icon at `.size(3.0)` as shadcn's
/// `[&_svg]:size-3`) in with `.child`.
#[must_use]
pub fn kbd() -> Div {
    let t = theme();
    div()
        .named("kbd")
        .bg(t.muted)
        .text_color(t.muted_foreground)
        .pass_through()
        .inline_flex()
        .h(5.0)
        .min_w(5.0)
        .items_center()
        .justify_center()
        .gap(1.0)
        .rounded_sm()
        .px(1.0)
        .font_sans()
        .text_xs()
        .font_medium()
}

/// Keys pressed together.
#[must_use]
pub fn kbd_group() -> Div {
    div().named("kbd-group").inline_flex().items_center().gap(1.0)
}

/// A key inside a tooltip, inverted with it
/// (`[[data-slot=tooltip-content]_&]:bg-background/20 text-background`,
/// `/10` in dark).
#[must_use]
pub fn kbd_on_tooltip() -> Div {
    let t = theme();
    kbd()
        .bg(t.background.opacity(if t.dark { 0.1 } else { 0.2 }))
        .text_color(t.background)
}
