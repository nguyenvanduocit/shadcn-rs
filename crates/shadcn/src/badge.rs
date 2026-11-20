//! Badge (`ui/badge`): a small label in four variants.

use gui::{Color, Div, Properties, Styled, div};

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
        .transition_props(Properties::COLOR | Properties::SHADOW)
        .inline_flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border()
        .px(2.0)
        .py(0.5)
        .text_xs()
        .font_medium()
        .whitespace_nowrap()
        .shrink_0()
        .icon_size(3.0)
        .gap(1.0)
        .overflow_hidden();
    match variant {
        BadgeVariant::Default => d
            .border_color(Color::TRANSPARENT)
            .bg(t.primary)
            .text_color(t.primary_foreground),
        BadgeVariant::Secondary => d
            .border_color(Color::TRANSPARENT)
            .bg(t.secondary)
            .text_color(t.secondary_foreground),
        BadgeVariant::Destructive => d
            .border_color(Color::TRANSPARENT)
            .bg(if t.dark {
                t.destructive.opacity(0.6)
            } else {
                t.destructive
            })
            .text_color(Color::WHITE),
        BadgeVariant::Outline => d.border_color(t.border).text_color(t.foreground),
    }
}
