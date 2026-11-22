//! Badge (`ui/badge`): a small label in four variants.

use gui::prelude::*;
use gui::{Color, Div, Properties, div};

use crate::theme::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BadgeVariant {
    #[default]
    Default,
    Secondary,
    Destructive,
    Outline,
}

/// A badge box; add its text and a `size-3` icon with `.child`.
#[must_use]
pub fn badge(variant: BadgeVariant) -> Div {
    let t = theme();
    let d = div()
        .named("badge")
        .transition(Properties::COLOR | Properties::SHADOW)
        .display(Display::Flex)
        .inline(true)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .radius(9999.0)
        .border(1.0)
        .padding_x(8.0)
        .padding_y(2.0)
        .text_size(12.0)
        .line_height(16.0)
        .font_weight(500)
        .nowrap(true)
        .flex_shrink(0.0)
        .icon_size(12.0)
        .gap(4.0)
        .overflow(Overflow::Hidden);
    match variant {
        BadgeVariant::Default => d
            .border_color(Color::TRANSPARENT)
            .background(t.primary)
            .text_color(t.primary_foreground),
        BadgeVariant::Secondary => d
            .border_color(Color::TRANSPARENT)
            .background(t.secondary)
            .text_color(t.secondary_foreground),
        BadgeVariant::Destructive => d
            .border_color(Color::TRANSPARENT)
            .background(if t.dark {
                t.destructive.opacity(0.6)
            } else {
                t.destructive
            })
            .text_color(Color::WHITE),
        BadgeVariant::Outline => d.border_color(t.border).text_color(t.foreground),
    }
}
