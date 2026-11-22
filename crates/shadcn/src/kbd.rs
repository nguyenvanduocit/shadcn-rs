//! Kbd (`ui/kbd`): a keyboard key, and a group of keys.

use gui::prelude::*;
use gui::{Div, div};

use crate::theme::theme;

/// One key; put its label (text, or an icon at `.size(12.0)` as shadcn's
/// `[&_svg]:size-3`) in with `.child`.
#[must_use]
pub fn kbd() -> Div {
    let t = theme();
    div()
        .named("kbd")
        .background(t.muted)
        .text_color(t.muted_foreground)
        .pass_through()
        .display(Display::Flex)
        .inline(true)
        .height(20.0)
        .min_width(20.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .gap(4.0)
        .radius(6.0)
        .padding_x(4.0)
        .font_family(FontFamily::INTER)
        .text_size(12.0)
        .line_height(16.0)
        .font_weight(500)
}

/// Keys pressed together.
#[must_use]
pub fn kbd_group() -> Div {
    div()
        .named("kbd-group")
        .display(Display::Flex)
        .inline(true)
        .align_items(Align::Center)
        .gap(4.0)
}

/// A key inside a tooltip, inverted with it
/// (`[[data-slot=tooltip-content]_&]:bg-background/20 text-background`,
/// `/10` in dark).
#[must_use]
pub fn kbd_on_tooltip() -> Div {
    let t = theme();
    kbd()
        .background(t.background.opacity(if t.dark { 0.1 } else { 0.2 }))
        .text_color(t.background)
}
