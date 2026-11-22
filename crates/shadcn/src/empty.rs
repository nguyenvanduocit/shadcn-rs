//! Empty (`ui/empty`): what a place shows when it has nothing yet: a
//! picture, a title, a description and what to do about it.

use gui::prelude::*;
use gui::{Div, Icon, Span, Text, div, icon, span, text};

use crate::theme::theme;

/// The centred column. `md:p-12` applies: a native window is a desktop
/// viewport, past Tailwind's `md` breakpoint. Add `.border(1.0)` for the outline
/// look (`border border-dashed`).
#[must_use]
pub fn empty() -> Div {
    let t = theme();
    div()
        .named("empty")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .min_width(0.0)
        .flex(Flex::grow(1.0))
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .gap(24.0)
        .radius(10.0)
        .border_color(t.border)
        .padding(48.0)
        .text_wrap(WrapStyle::Balance)
        .text_align(TextAlign::Center)
}

#[must_use]
pub fn empty_header() -> Div {
    div()
        .named("empty-header")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .max_width(384.0)
        .align_items(Align::Center)
        .gap(8.0)
        .text_align(TextAlign::Center)
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
        .margin_bottom(8.0)
        .display(Display::Flex)
        .flex_shrink(0.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center);
    match variant {
        EmptyMediaVariant::Default => d,
        EmptyMediaVariant::Icon => d.background(t.muted).text_color(t.foreground).size(40.0).radius(10.0),
    }
}

/// `icon_name` on the media tile, at `size-6`
/// (`[&_svg:not([class*='size-'])]:size-6`).
#[must_use]
pub fn empty_media_icon(icon_name: Icon) -> Div {
    empty_media(EmptyMediaVariant::Icon).child(icon(icon_name).size(24.0))
}

#[must_use]
pub fn empty_title(title: impl Into<String>) -> Text {
    text(title)
        .named("empty-title")
        .text_size(18.0)
        .line_height(28.0)
        .font_weight(500)
        .tracking(-0.025)
}

#[must_use]
pub fn empty_description(description: impl Into<String>) -> Text {
    let t = theme();
    text(description)
        .named("empty-description")
        .text_color(t.muted_foreground)
        .text_size(14.0)
        .line_height(20.0)
        .line_height(LineHeight::Relative(1.625))
}

/// A link inside an [`empty_description`], appended with `.span`
/// (`[&>a]:underline [&>a]:underline-offset-4`); give it its `.on_click`.
#[must_use]
pub fn empty_link(label: impl Into<String>) -> Span {
    span(label).underline(true).underline_offset(4.0)
}

/// What to do about it: the actions under the header.
#[must_use]
pub fn empty_content() -> Div {
    div()
        .named("empty-content")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .min_width(0.0)
        .max_width(384.0)
        .align_items(Align::Center)
        .gap(16.0)
        .text_wrap(WrapStyle::Balance)
        .text_size(14.0)
        .line_height(20.0)
}
