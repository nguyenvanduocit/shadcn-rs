//! Message (`ui/message`): one turn in a chat: an avatar beside a column of
//! header, bubbles and footer, on either side of the thread.
//!
//! Vue's `group-data-[align]/message` and `group-has-[…]/message` selectors
//! become [`MessageProps`], which every part takes.

use gui::prelude::*;
use gui::{Div, div};

use crate::bubble::{BubbleAlign, BubbleVariant};
use crate::theme::theme;

/// What a message's parts read from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MessageProps {
    /// The sender's side: [`BubbleAlign::End`] for the user's own turns.
    pub align: BubbleAlign,
    /// Whether the message has a [`message_footer`]: the avatar then lines
    /// up with the last bubble rather than the footer.
    pub footer: bool,
    /// The variant of its bubbles: header and footer drop their inset beside
    /// a frameless [`BubbleVariant::Ghost`].
    pub bubble: BubbleVariant,
}

#[must_use]
pub fn message(p: MessageProps) -> Div {
    let d = div().named("message").relative().w_full().min_w(0.0).gap(2.0).text_sm();
    match p.align {
        BubbleAlign::Start => d.flex(),
        BubbleAlign::End => d.flex_row_reverse(),
    }
}

/// The round slot an avatar sits in, at the foot of the message.
#[must_use]
pub fn message_avatar(p: MessageProps) -> Div {
    let t = theme();
    div()
        .named("message-avatar")
        .flex()
        .min_w(8.0)
        .shrink_0()
        .items_center()
        .justify_center()
        .self_end()
        .overflow_hidden()
        .rounded_full()
        .bg(t.muted)
        // Vue lifts it with `-translate-y-8`; a bottom margin moves a
        // bottom-aligned box the same 32px.
        .when(p.footer, |d| d.mb(8.0))
}

/// The column of header, bubbles and footer.
#[must_use]
pub fn message_content(p: MessageProps) -> Div {
    div()
        .named("message-content")
        .flex_col()
        .w_full()
        .min_w(0.0)
        .gap(2.5)
        .when(p.align == BubbleAlign::End, Styled::items_end)
}

/// The sender's name above the bubbles.
#[must_use]
pub fn message_header(p: MessageProps) -> Div {
    caption(p).named("message-header")
}

/// Delivery or read state under the bubbles.
#[must_use]
pub fn message_footer(p: MessageProps) -> Div {
    caption(p)
        .named("message-footer")
        .when(p.align == BubbleAlign::End, Styled::justify_end)
}

fn caption(p: MessageProps) -> Div {
    div()
        .flex()
        .items_center()
        .max_w_full()
        .min_w(0.0)
        .px(if p.bubble == BubbleVariant::Ghost { 0.0 } else { 3.0 })
        .text_xs()
        .font_medium()
        .text_color(theme().muted_foreground)
}

/// Consecutive messages, stacked.
#[must_use]
pub fn message_group() -> Div {
    div().named("message-group").flex_col().gap(2.0).min_w(0.0)
}
