//! Bubble (`ui/bubble`): a chat bubble in seven variants, aligned to either
//! side, with reactions pinned to its edge.
//!
//! Vue styles `bubble-content` from its bubble's `data-variant` and
//! `data-align`; here [`bubble_content`] takes the same [`BubbleProps`].

use gui::prelude::*;
use gui::{Color, Div, div, px, relative};

use crate::focus::focus_ring;
use crate::theme::{Theme, theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BubbleVariant {
    #[default]
    Default,
    Secondary,
    Muted,
    /// A soft tint of the primary colour.
    Tinted,
    Outline,
    /// No frame, full width: assistant text.
    Ghost,
    Destructive,
}

/// Which side of the thread a bubble sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BubbleAlign {
    #[default]
    Start,
    End,
}

/// What a bubble's content reads from it. In an end-aligned
/// [`message`](crate::message) give the bubble [`BubbleAlign::End`]: Vue reads
/// that from the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BubbleProps {
    pub variant: BubbleVariant,
    pub align: BubbleAlign,
}

/// The bubble: a column holding one [`bubble_content`] and, optionally,
/// [`bubble_reactions`]. At most 80% of the thread's width (a ghost bubble
/// takes it all).
#[must_use]
pub fn bubble(p: BubbleProps) -> Div {
    let mut d = div().named("bubble").relative().flex_col().gap(1.0).min_w(0.0);
    d.style_mut().max_width = relative(if p.variant == BubbleVariant::Ghost { 1.0 } else { 0.8 });
    match p.align {
        BubbleAlign::Start => d.self_start(),
        BubbleAlign::End => d.self_end(),
    }
}

/// The bubble's body; add its text with `.child`.
#[must_use]
pub fn bubble_content(p: BubbleProps) -> Div {
    let t = theme();
    let d = div()
        .named("bubble-content")
        .transition_colors()
        .rounded_xl()
        .border()
        .border_color(Color::TRANSPARENT)
        .px(3.0)
        .py(2.0)
        .text_sm()
        .leading_relaxed()
        .max_w_full()
        .min_w(0.0)
        .overflow_hidden();
    let d = match p.align {
        BubbleAlign::Start => d.self_start(),
        BubbleAlign::End => d.self_end(),
    };
    variant(d, p.variant, &t)
}

/// A [`bubble_content`] that is a button (a suggestion, "show more"): it
/// takes focus and darkens on hover; give it `.on_click`.
#[must_use]
pub fn bubble_content_button(p: BubbleProps) -> Div {
    let t = theme();
    let hover = hover_bg(p.variant, &t);
    let hover_text = matches!(p.variant, BubbleVariant::Outline | BubbleVariant::Ghost).then_some(t.foreground);
    bubble_content(p)
        .focusable()
        .cursor_pointer()
        .text_left()
        .hover(move |s| {
            let s = s.bg(hover);
            match hover_text {
                Some(c) => s.text_color(c),
                None => s,
            }
        })
        .focus_visible(move |s| focus_ring(s, &t))
}

/// Vue's `oklch(from var(--primary) l calc(c*k) h)`. `gui::Color` does not
/// expose a colour's oklch parts, so the hue and chroma of the primary
/// cannot be read; the neutral theme's primary has none, which this is.
fn primary_tint(lightness: f32) -> Color {
    Color::oklch(lightness, 0.0, 0.0)
}

fn variant(d: Div, v: BubbleVariant, t: &Theme) -> Div {
    match v {
        BubbleVariant::Default => d.bg(t.primary).text_color(t.primary_foreground),
        BubbleVariant::Secondary => d.bg(t.secondary).text_color(t.secondary_foreground),
        BubbleVariant::Muted => d.bg(t.muted),
        BubbleVariant::Tinted => d
            .bg(primary_tint(if t.dark { 0.3 } else { 0.93 }))
            .text_color(t.foreground),
        BubbleVariant::Outline => d.bg(t.background).border_color(t.border),
        BubbleVariant::Ghost => d.rounded_none().bg(Color::TRANSPARENT).p(0.0),
        BubbleVariant::Destructive => d
            .bg(t.destructive.opacity(if t.dark { 0.2 } else { 0.1 }))
            .text_color(t.destructive),
    }
}

/// The hover background of a pressable bubble's content.
fn hover_bg(v: BubbleVariant, t: &Theme) -> Color {
    match v {
        BubbleVariant::Default => t.primary.opacity(0.8),
        BubbleVariant::Secondary => t.foreground.opacity(0.05).over(t.secondary),
        BubbleVariant::Muted => t.foreground.opacity(0.05).over(t.muted),
        BubbleVariant::Tinted => primary_tint(if t.dark { 0.35 } else { 0.88 }),
        BubbleVariant::Outline => {
            if t.dark {
                t.input.opacity(0.3)
            } else {
                t.muted
            }
        }
        BubbleVariant::Ghost => {
            if t.dark {
                t.muted.opacity(0.5)
            } else {
                t.muted
            }
        }
        BubbleVariant::Destructive => t.destructive.opacity(if t.dark { 0.3 } else { 0.2 }),
    }
}

/// Bubbles from one sender, stacked.
#[must_use]
pub fn bubble_group() -> Div {
    div().named("bubble-group").flex_col().gap(2.0).min_w(0.0)
}

/// Which edge of the bubble the reactions sit on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BubbleReactionsSide {
    Top,
    #[default]
    Bottom,
}

/// The height of a reactions pill holding text: `text-sm` (20px) and `py-0.5`.
const REACTIONS_HEIGHT: f32 = 24.0;

/// A pill of reactions overlapping the bubble's top or bottom edge by three
/// quarters of its height (Vue's `translate-y-3/4`), inset from one side.
/// Add emoji or a count as text, or a small button.
#[must_use]
pub fn bubble_reactions(side: BubbleReactionsSide, align: BubbleAlign) -> Div {
    reactions(side, align, REACTIONS_HEIGHT)
}

/// The height of a small button (`size="sm"` or `"icon-sm"`): 32px.
const REACTIONS_BUTTON_HEIGHT: f32 = 32.0;

/// [`bubble_reactions`] holding a small button (`sm` or `icon-sm`), which
/// fills it edge to edge (Vue's `has-[button]:p-0`).
#[must_use]
pub fn bubble_reactions_button(side: BubbleReactionsSide, align: BubbleAlign) -> Div {
    reactions(side, align, REACTIONS_BUTTON_HEIGHT).p(0.0)
}

fn reactions(side: BubbleReactionsSide, align: BubbleAlign, height: f32) -> Div {
    let t = theme();
    let overhang = px(-0.75 * height);
    let d = div()
        .named("bubble-reactions")
        .absolute()
        .flex()
        .items_center()
        .justify_center()
        .shrink_0()
        .gap(1.0)
        .px(1.5)
        .py(0.5)
        .rounded_full()
        .ring(3.0, t.card)
        .bg(t.muted)
        .text_sm();
    let d = match side {
        BubbleReactionsSide::Top => d.top_len(overhang),
        BubbleReactionsSide::Bottom => d.bottom_len(overhang),
    };
    match align {
        BubbleAlign::Start => d.left(3.0),
        BubbleAlign::End => d.right(3.0),
    }
}
