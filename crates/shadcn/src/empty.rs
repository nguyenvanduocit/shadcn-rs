//! Empty (`ui/empty`): what a place shows when it has nothing yet: a
//! picture, a title, a description and what to do about it.

use gui::{Div, Icon, Span, Styled, Text, div, icon, span, text};

use crate::theme::theme;

/// The centred column. `md:p-12` applies: a native window is a desktop
/// viewport, past Tailwind's `md` breakpoint. Add `.border()` for the outline
/// look (`border border-dashed`).
#[must_use]
pub fn empty() -> Div {
    let t = theme();
    div()
        .named("empty")
        .flex_col()
        .min_w(0.0)
        .flex_1()
        .items_center()
        .justify_center()
        .gap(6.0)
        .rounded_lg()
        .border_color(t.border)
        .p(12.0)
        .text_balance()
        .text_center()
}

#[must_use]
pub fn empty_header() -> Div {
    div()
        .named("empty-header")
        .flex_col()
        .max_w_px(384.0)
        .items_center()
        .gap(2.0)
        .text_center()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EmptyMediaVariant {
    /// Bare (`bg-transparent`): an avatar, an image.
    #[default]
    Default,
    /// A muted tile holding an icon or a spinner.
    Icon,
}

/// The picture above the title. For a lucide icon on the tile use
/// [`empty_media_icon`], which sizes it as shadcn does.
#[must_use]
pub fn empty_media(variant: EmptyMediaVariant) -> Div {
    let t = theme();
    let d = div()
        .named("empty-media")
        .mb(2.0)
        .flex()
        .shrink_0()
        .items_center()
        .justify_center();
    match variant {
        EmptyMediaVariant::Default => d,
        EmptyMediaVariant::Icon => d.bg(t.muted).text_color(t.foreground).size(10.0).rounded_lg(),
    }
}

/// `icon_name` on the media tile, at `size-6`
/// (`[&_svg:not([class*='size-'])]:size-6`).
#[must_use]
pub fn empty_media_icon(icon_name: Icon) -> Div {
    empty_media(EmptyMediaVariant::Icon).child(icon(icon_name).size(6.0))
}

#[must_use]
pub fn empty_title(title: impl Into<String>) -> Text {
    text(title)
        .named("empty-title")
        .text_lg()
        .font_medium()
        .tracking_tight()
}

#[must_use]
pub fn empty_description(description: impl Into<String>) -> Text {
    let t = theme();
    text(description)
        .named("empty-description")
        .text_color(t.muted_foreground)
        .text_sm()
        .leading_relaxed()
}

/// A link inside an [`empty_description`], appended with `.span`
/// (`[&>a]:underline [&>a]:underline-offset-4`); give it its `.on_click`.
#[must_use]
pub fn empty_link(label: impl Into<String>) -> Span {
    span(label).underline().underline_offset(4.0)
}

/// What to do about it: the actions under the header.
#[must_use]
pub fn empty_content() -> Div {
    div()
        .named("empty-content")
        .flex_col()
        .w_full()
        .min_w(0.0)
        .max_w_px(384.0)
        .items_center()
        .gap(4.0)
        .text_balance()
        .text_sm()
}
