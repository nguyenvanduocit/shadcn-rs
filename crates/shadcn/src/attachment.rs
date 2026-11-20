//! Attachment (`ui/attachment`): a file shown as a card, its media, name,
//! details and actions, in three sizes, two orientations and five states.
//!
//! Vue's `group-data-[…]/attachment` selectors become [`AttachmentProps`]:
//! every part takes the attachment's props so it styles itself as its root does.

use gui::prelude::*;
use gui::{Div, Icon, IconElement, Image, ImageSource, Text, div, icon, image, text};

use crate::button::{ButtonSize, ButtonVariant, button};
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
        .transition_colors()
        .relative()
        .inline_flex()
        .flex_wrap()
        .max_w_full()
        .min_w(0.0)
        .shrink_0()
        .rounded_xl()
        .border()
        .border_color(match p.state {
            AttachmentState::Error => t.destructive.opacity(0.3),
            _ => t.border,
        })
        .bg(t.card)
        .text_color(t.card_foreground);
    // (gap, padding with media, side and top padding with content only)
    let (gap, even, (x, y)) = match p.size {
        AttachmentSize::Default => (2.0, 2.0, (2.5, 2.0)),
        AttachmentSize::Sm => (2.5, 1.5, (2.0, 1.5)),
        AttachmentSize::Xs => (1.5, 1.0, (1.5, 1.0)),
    };
    let d = match p.size {
        AttachmentSize::Default => d.text_sm(),
        AttachmentSize::Sm => d.text_xs(),
        AttachmentSize::Xs => d.text_xs().rounded_lg(),
    }
    .gap(gap);
    // Vue's media rule sorts after its content rule, so media wins.
    let d = match (slots.media, slots.content) {
        (true, _) => d.p(even),
        (false, true) => d.px(x).py(y),
        (false, false) => d,
    };
    match p.orientation {
        AttachmentOrientation::Horizontal => d.min_w(40.0).items_center(),
        AttachmentOrientation::Vertical => d.w(if slots.content { 30.0 } else { 24.0 }).flex_col(),
    }
}

/// The square that holds the file's icon, spinner or preview image.
#[must_use]
pub fn attachment_media(p: AttachmentProps, variant: AttachmentMediaVariant) -> Div {
    let t = theme();
    let error = p.state == AttachmentState::Error;
    let d = div()
        .named("attachment-media")
        .relative()
        .flex()
        .aspect_square()
        .shrink_0()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .rounded_lg()
        .bg(if error { t.destructive.opacity(0.1) } else { t.muted })
        .text_color(if error { t.destructive } else { t.foreground });
    let d = match p.size {
        AttachmentSize::Default => d.w(10.0),
        AttachmentSize::Sm => d.w(8.0),
        AttachmentSize::Xs => d.w(7.0).rounded_md(),
    };
    let d = d.when(p.orientation == AttachmentOrientation::Vertical, Styled::w_full);
    let settled = matches!(p.state, AttachmentState::Idle | AttachmentState::Done);
    d.when(variant == AttachmentMediaVariant::Image && !settled, |d| d.opacity(0.6))
}

/// An icon at the size the media gives it (`size-4`, `size-3.5` when extra
/// small, `size-6` when vertical).
#[must_use]
pub fn attachment_icon(p: AttachmentProps, which: Icon) -> IconElement {
    let size = match (p.orientation, p.size) {
        (AttachmentOrientation::Vertical, _) => 6.0,
        (AttachmentOrientation::Horizontal, AttachmentSize::Xs) => 3.5,
        (AttachmentOrientation::Horizontal, _) => 4.0,
    };
    icon(which).size(size)
}

/// A preview filling the media square.
#[must_use]
pub fn attachment_image(source: ImageSource) -> Image {
    image(source).w_full().aspect_square()
}

/// The name and details column.
#[must_use]
pub fn attachment_content(p: AttachmentProps) -> Div {
    div()
        .named("attachment-content")
        .leading_tight()
        .max_w_full()
        .min_w(0.0)
        .flex_1()
        .when(p.orientation == AttachmentOrientation::Vertical, |d| d.px(1.0))
}

/// The file name, cut to one line.
#[must_use]
pub fn attachment_title(title: impl Into<String>) -> Text {
    text(title)
        .named("attachment-title")
        .font_medium()
        .max_w_full()
        .truncate()
}

/// Size, type or progress under the name; red when the upload failed.
#[must_use]
pub fn attachment_description(p: AttachmentProps, description: impl Into<String>) -> Text {
    let t = theme();
    text(description)
        .named("attachment-description")
        .mt(0.5)
        .text_xs()
        .max_w_full()
        .truncate()
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
        .relative()
        .flex()
        .shrink_0()
        .items_center();
    match p.orientation {
        AttachmentOrientation::Horizontal => d,
        AttachmentOrientation::Vertical => d.absolute().top(3.0).right(3.0).gap(1.0),
    }
}

/// A ghost icon button (remove, retry, cancel) showing `which` at `size-3`;
/// give it an `.on_click`.
#[must_use]
pub fn attachment_action(which: Icon) -> Div {
    button(ButtonVariant::Ghost, ButtonSize::IconXs)
        .named("attachment-action")
        .child(icon(which).size(3.0))
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
        .cursor_pointer()
        .hover(move |s| s.bg(t.muted.opacity(0.5)))
        .focus_visible(move |s| s.ring(1.0, t.ring.opacity(0.5)))
}

/// A row of attachments that scrolls sideways when it overflows, its
/// clipped ends faded and no scrollbar drawn.
#[must_use]
pub fn attachment_group() -> Div {
    div()
        .named("attachment-group")
        .flex()
        .gap(3.0)
        .py(1.0)
        .min_w(0.0)
        .overflow_x_scroll()
        .scroll_fade_x()
        .scrollbar_none()
}
