//! Attachment (`ui/attachment`): a file shown as a card, its media, name,
//! details and actions, in three sizes, two orientations and five states.
//!
//! Vue's `group-data-[…]/attachment` selectors become [`AttachmentProps`]:
//! every part takes the attachment's props so it styles itself as its root does.

use gui::prelude::*;
use gui::{Div, Icon, IconElement, Image, ImageSource, Text, div, icon, image, text};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::focus::StateLook;
use crate::theme::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AttachmentSize {
    #[default]
    Default,
    Sm,
    Xs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AttachmentOrientation {
    #[default]
    Horizontal,
    Vertical,
}

/// Where the file is in its upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AttachmentState {
    /// Picked, not sent yet: a dashed border.
    Idle,
    Uploading,
    Processing,
    Error,
    #[default]
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AttachmentMediaVariant {
    #[default]
    Icon,
    Image,
}

/// What an attachment's parts read from it (Vue's `data-size`, `data-orientation`, `data-state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AttachmentProps {
    pub size: AttachmentSize,
    pub orientation: AttachmentOrientation,
    pub state: AttachmentState,
}

/// Which parts an attachment holds: Vue's `has-data-[slot=…]` selectors,
/// which set the card's padding and a vertical card's width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachmentSlots {
    /// It holds an [`attachment_media`]: even padding all round.
    pub media: bool,
    /// It holds an [`attachment_content`]: wider side padding, a wider vertical card.
    pub content: bool,
}

impl Default for AttachmentSlots {
    /// Media and content, as most attachments are.
    fn default() -> Self {
        Self {
            media: true,
            content: true,
        }
    }
}

/// The attachment card; its children are [`attachment_media`],
/// [`attachment_content`] and [`attachment_actions`], as `slots` says.
#[must_use]
pub fn attachment(p: AttachmentProps, slots: AttachmentSlots) -> Div {
    let t = theme();
    let d = div()
        .named("attachment")
        .transition(Properties::COLORS)
        .position(Position::Relative)
        .display(Display::Flex)
        .inline(true)
        .flex_wrap(true)
        .max_width(pct(100.0))
        .min_width(0.0)
        .flex_shrink(0.0)
        .radius(14.0)
        .border(1.0)
        .border_color(match p.state {
            AttachmentState::Error => t.destructive.opacity(0.3),
            _ => t.border,
        })
        .background(t.card)
        .text_color(t.card_foreground);
    // (gap, padding with media, side and top padding with content only)
    let (gap, even, (x, y)) = match p.size {
        AttachmentSize::Default => (8.0, 8.0, (10.0, 8.0)),
        AttachmentSize::Sm => (10.0, 6.0, (8.0, 6.0)),
        AttachmentSize::Xs => (6.0, 4.0, (6.0, 4.0)),
    };
    let d = match p.size {
        AttachmentSize::Default => d.text_size(14.0).line_height(20.0),
        AttachmentSize::Sm => d.text_size(12.0).line_height(16.0),
        AttachmentSize::Xs => d.text_size(12.0).line_height(16.0).radius(10.0),
    }
    .gap(gap);
    // Vue's media rule sorts after its content rule, so media wins.
    let d = match (slots.media, slots.content) {
        (true, _) => d.padding(even),
        (false, true) => d.padding_x(x).padding_y(y),
        (false, false) => d,
    };
    match p.orientation {
        AttachmentOrientation::Horizontal => d.min_width(160.0).align_items(Align::Center),
        AttachmentOrientation::Vertical => d
            .width(if slots.content { 120.0 } else { 96.0 })
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column),
    }
}

/// The square that holds the file's icon, spinner or preview image.
#[must_use]
pub fn attachment_media(p: AttachmentProps, variant: AttachmentMediaVariant) -> Div {
    let t = theme();
    let error = p.state == AttachmentState::Error;
    let d = div()
        .named("attachment-media")
        .position(Position::Relative)
        .display(Display::Flex)
        .aspect_ratio(1.0)
        .flex_shrink(0.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .overflow(Overflow::Hidden)
        .radius(10.0)
        .background(if error { t.destructive.opacity(0.1) } else { t.muted })
        .text_color(if error { t.destructive } else { t.foreground });
    let d = match p.size {
        AttachmentSize::Default => d.width(40.0),
        AttachmentSize::Sm => d.width(32.0),
        AttachmentSize::Xs => d.width(28.0).radius(8.0),
    };
    let d = d.when(p.orientation == AttachmentOrientation::Vertical, |e| {
        e.width(pct(100.0))
    });
    let settled = matches!(p.state, AttachmentState::Idle | AttachmentState::Done);
    d.when(variant == AttachmentMediaVariant::Image && !settled, |d| d.opacity(0.6))
}

/// An icon at the size the media gives it (`size-4`, `size-3.5` when extra
/// small, `size-6` when vertical).
#[must_use]
pub fn attachment_icon(p: AttachmentProps, which: Icon) -> IconElement {
    let size = match (p.orientation, p.size) {
        (AttachmentOrientation::Vertical, _) => 24.0,
        (AttachmentOrientation::Horizontal, AttachmentSize::Xs) => 14.0,
        (AttachmentOrientation::Horizontal, _) => 16.0,
    };
    icon(which).size(size)
}

/// A preview filling the media square.
#[must_use]
pub fn attachment_image(source: ImageSource) -> Image {
    image(source).width(pct(100.0)).aspect_ratio(1.0)
}

/// The name and details column.
#[must_use]
pub fn attachment_content(p: AttachmentProps) -> Div {
    div()
        .named("attachment-content")
        .line_height(LineHeight::Relative(1.25))
        .max_width(pct(100.0))
        .min_width(0.0)
        .flex(Flex::grow(1.0))
        .when(p.orientation == AttachmentOrientation::Vertical, |d| d.padding_x(4.0))
}

/// The file name, cut to one line.
#[must_use]
pub fn attachment_title(title: impl Into<String>) -> Text {
    text(title)
        .named("attachment-title")
        .font_weight(500)
        .max_width(pct(100.0))
        .truncate(true)
        .nowrap(true)
        .overflow_x(Overflow::Hidden)
        .min_width(0.0)
}

/// Size, type or progress under the name; red when the upload failed.
#[must_use]
pub fn attachment_description(p: AttachmentProps, description: impl Into<String>) -> Text {
    let t = theme();
    text(description)
        .named("attachment-description")
        .margin_top(2.0)
        .text_size(12.0)
        .line_height(16.0)
        .max_width(pct(100.0))
        .truncate(true)
        .nowrap(true)
        .overflow_x(Overflow::Hidden)
        .min_width(0.0)
        .text_color(if p.state == AttachmentState::Error {
            t.destructive.opacity(0.8)
        } else {
            t.muted_foreground
        })
}

/// The row of [`attachment_action`] buttons; on a vertical attachment it
/// floats over the media's top-right corner.
#[must_use]
pub fn attachment_actions(p: AttachmentProps) -> Div {
    let d = div()
        .named("attachment-actions")
        .position(Position::Relative)
        .display(Display::Flex)
        .flex_shrink(0.0)
        .align_items(Align::Center);
    match p.orientation {
        AttachmentOrientation::Horizontal => d,
        AttachmentOrientation::Vertical => d.position(Position::Absolute).top(12.0).right(12.0).gap(4.0),
    }
}

/// A ghost icon button (remove, retry, cancel) showing `which` at `size-3`;
/// give it an `.on_click`.
#[must_use]
pub fn attachment_action(which: Icon) -> Div {
    button(ButtonVariant::Ghost, ButtonSize::IconXs)
        .named("attachment-action")
        .child(icon(which).size(12.0))
        .into_div()
}

/// Makes the whole attachment pressable (open, preview). Vue lays a
/// transparent button over the card because HTML cannot nest buttons; here
/// the card takes the press itself, and the [`attachment_action`] buttons
/// inside keep theirs, as the innermost handler wins.
#[must_use]
pub fn attachment_trigger(attachment: Div) -> Div {
    let t = theme();
    attachment
        .focusable()
        .cursor(Cursor::Pointer)
        .with_interaction(|mut d, i| {
            let ring = d.style_mut().ring;
            let shown = Ring::new(1.0, t.ring.opacity(0.5));
            d.background_while(move || i.hovered(), t.muted.opacity(0.5))
                .ring(move || if i.focus_visible() { Some(shown) } else { ring })
        })
}

/// A row of attachments that scrolls sideways when it overflows, its
/// clipped ends faded and no scrollbar drawn.
#[must_use]
pub fn attachment_group() -> Div {
    div()
        .named("attachment-group")
        .display(Display::Flex)
        .gap(12.0)
        .padding_y(4.0)
        .min_width(0.0)
        .overflow_x(Overflow::Scroll)
        .overflow_y(Overflow::Hidden)
        .mask(ScrollFade {
            axis: Axis::Horizontal,
            start: true,
            end: true,
            size: None,
        })
        .scrollbar_hidden(true)
}
