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
    let d = div()
        .named("message")
        .position(Position::Relative)
        .width(pct(100.0))
        .min_width(0.0)
        .gap(8.0)
        .text_size(14.0)
        .line_height(20.0);
    match p.align {
        BubbleAlign::Start => d.display(Display::Flex),
        BubbleAlign::End => d.display(Display::Flex).flex_direction(FlexDirection::RowReverse),
    }
}

/// The round slot an avatar sits in, at the foot of the message.
#[must_use]
pub fn message_avatar(p: MessageProps) -> Div {
    let t = theme();
    div()
        .named("message-avatar")
        .display(Display::Flex)
        .min_width(32.0)
        .flex_shrink(0.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .align_self(Align::End)
        .overflow(Overflow::Hidden)
        .radius(9999.0)
        .background(t.muted)
        // Vue lifts it with `-translate-y-8`; a bottom margin moves a
        // bottom-aligned box the same 32px.
        .when(p.footer, |d| d.margin_bottom(32.0))
}

/// The column of header, bubbles and footer.
#[must_use]
pub fn message_content(p: MessageProps) -> Div {
    div()
        .named("message-content")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .min_width(0.0)
        .gap(10.0)
        .when(p.align == BubbleAlign::End, |e| e.align_items(Align::End))
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
        .when(p.align == BubbleAlign::End, |e| e.justify_content(Justify::End))
}

fn caption(p: MessageProps) -> Div {
    div()
        .display(Display::Flex)
        .align_items(Align::Center)
        .max_width(pct(100.0))
        .min_width(0.0)
        .padding_x(if p.bubble == BubbleVariant::Ghost { 0.0 } else { 12.0 })
        .text_size(12.0)
        .line_height(16.0)
        .font_weight(500)
        .text_color(theme().muted_foreground)
}

/// Consecutive messages, stacked.
#[must_use]
pub fn message_group() -> Div {
    div()
        .named("message-group")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(8.0)
        .min_width(0.0)
}
