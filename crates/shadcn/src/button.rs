//! Button (`ui/button`): a pressable control in six variants and eight sizes.

use gui::prelude::*;
use gui::{Div, div};

use crate::focus::{destructive_ring, focus_ring};
use crate::theme::{Theme, theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    #[default]
    Default,
    Destructive,
    Outline,
    Secondary,
    Ghost,
    Link,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ButtonSize {
    #[default]
    Default,
    Xs,
    Sm,
    Lg,
    Icon,
    IconXs,
    IconSm,
    IconLg,
}

/// A button box; add its label and icons with `.child`, its action with `.on_click`.
#[must_use]
pub fn button(variant: ButtonVariant, size: ButtonSize) -> Div {
    let t = theme();
    let base = div()
        .named("button")
        .transition_all()
        .inline_flex()
        .items_center()
        .justify_center()
        .gap(2.0)
        .whitespace_nowrap()
        .rounded_md()
        .text_sm()
        .font_medium()
        .shrink_0()
        .border_color(t.border)
        .focusable()
        .disabled_style(|s| s.opacity(0.5));
    sized(styled(base, variant, &t), size)
}

fn styled(d: Div, variant: ButtonVariant, t: &Theme) -> Div {
    let t = *t;
    match variant {
        ButtonVariant::Default => d
            .bg(t.primary)
            .text_color(t.primary_foreground)
            .hover(move |s| s.bg(t.primary.opacity(0.9)))
            .focus_visible(move |s| focus_ring(s, &t)),
        ButtonVariant::Destructive => d
            .bg(if t.dark {
                t.destructive.opacity(0.6)
            } else {
                t.destructive
            })
            .text_color(gui::Color::WHITE)
            .hover(move |s| s.bg(t.destructive.opacity(0.9)))
            .focus_visible(move |s| destructive_ring(s, &t)),
        ButtonVariant::Outline => d
            .border()
            .when(t.dark, |d| d.bg(t.input.opacity(0.3)).border_color(t.input))
            .when(!t.dark, |d| d.bg(t.background))
            .shadow_xs()
            .hover(move |s| {
                let bg = if t.dark { t.input.opacity(0.5) } else { t.accent };
                s.bg(bg).text_color(t.accent_foreground)
            })
            .focus_visible(move |s| focus_ring(s, &t)),
        ButtonVariant::Secondary => d
            .bg(t.secondary)
            .text_color(t.secondary_foreground)
            .hover(move |s| s.bg(t.secondary.opacity(0.8)))
            .focus_visible(move |s| focus_ring(s, &t)),
        // No base colour: a ghost button takes its parent's, as shadcn's does.
        ButtonVariant::Ghost => d
            .hover(move |s| {
                let bg = if t.dark { t.accent.opacity(0.5) } else { t.accent };
                s.bg(bg).text_color(t.accent_foreground)
            })
            .focus_visible(move |s| focus_ring(s, &t)),
        ButtonVariant::Link => d
            .text_color(t.primary)
            .hover(|s| s.underline())
            .focus_visible(move |s| focus_ring(s, &t)),
    }
}

fn sized(d: Div, size: ButtonSize) -> Div {
    match size {
        ButtonSize::Default => d.h(9.0).px(4.0).py(2.0).has_icon(|s| s.px(3.0)),
        ButtonSize::Xs => d
            .h(6.0)
            .gap(1.0)
            .px(2.0)
            .text_xs()
            .icon_size(3.0)
            .has_icon(|s| s.px(1.5)),
        ButtonSize::Sm => d.h(8.0).gap(1.5).px(3.0).has_icon(|s| s.px(2.5)),
        ButtonSize::Lg => d.h(10.0).px(6.0).has_icon(|s| s.px(4.0)),
        ButtonSize::Icon => d.size(9.0),
        ButtonSize::IconXs => d.size(6.0).icon_size(3.0),
        ButtonSize::IconSm => d.size(8.0),
        ButtonSize::IconLg => d.size(10.0),
    }
}
