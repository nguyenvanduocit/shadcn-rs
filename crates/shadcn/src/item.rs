//! Item (`ui/item`): a row of media, content and actions, alone or in a group.

use gui::prelude::*;
use gui::{Color, Div, div, text};

use crate::focus::StateLook;
use crate::separator::{Orientation, separator};
use crate::theme::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ItemVariant {
    #[default]
    Default,
    Outline,
    Muted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ItemSize {
    #[default]
    Default,
    Sm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ItemMediaVariant {
    #[default]
    Default,
    /// A bordered muted tile holding an icon.
    Icon,
    /// A rounded square cropping an image.
    Image,
}

/// An item: put [`item_media`], [`item_content`] and [`item_actions`] in it.
#[must_use]
pub fn item(variant: ItemVariant, size: ItemSize) -> Div {
    let t = theme();
    let d = div()
        .named("item")
        .transition(Properties::COLORS)
        .duration_ms(100.0)
        .display(Display::Flex)
        .align_items(Align::Center)
        .border(1.0)
        .border_color(Color::TRANSPARENT)
        .text_size(14.0)
        .line_height(20.0)
        .radius(8.0)
        .flex_wrap(true);
    let d = match variant {
        ItemVariant::Default => d,
        ItemVariant::Outline => d.border_color(t.border),
        ItemVariant::Muted => d.background(t.muted.opacity(0.5)),
    };
    let d = d.with_interaction(|d, i| d.focus_ring_on(i, &t));
    match size {
        ItemSize::Default => d.padding(16.0).gap(16.0),
        ItemSize::Sm => d.padding_y(12.0).padding_x(16.0).gap(10.0),
    }
}

/// An item that is a link (`as-child` on an `<a>`): focusable, tinted on
/// hover; add where it goes with `.on_click`.
#[must_use]
pub fn item_link(variant: ItemVariant, size: ItemSize) -> Div {
    let t = theme();
    item(variant, size)
        .cursor(Cursor::Pointer)
        .focusable()
        .with_interaction(|d, i| d.background_while(move || i.hovered(), t.accent.opacity(0.5)))
}

/// The icon, avatar or image at the start of an item.
#[must_use]
pub fn item_media(variant: ItemMediaVariant) -> Div {
    let t = theme();
    let d = div()
        .named("item-media")
        .display(Display::Flex)
        .flex_shrink(0.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .gap(8.0);
    match variant {
        ItemMediaVariant::Default => d,
        ItemMediaVariant::Icon => d
            .size(32.0)
            .border(1.0)
            .border_color(t.border)
            .radius(6.0)
            .background(t.muted),
        ItemMediaVariant::Image => d.size(40.0).radius(6.0).overflow(Overflow::Hidden),
    }
}

/// The media of an item whose content has an [`item_description`]
/// (`group-has-[[data-slot=item-description]]/item:self-start
/// translate-y-0.5`): at the top, beside the title, not centred on the
/// taller title-and-description column. The item cannot see what its
/// children hold, so its caller picks this over [`item_media`].
#[must_use]
pub fn item_media_described(variant: ItemMediaVariant) -> Div {
    item_media(variant)
        .align_self(Align::Start)
        .position(Position::Relative)
        .top(2.0)
}

/// Title and description, taking the free width.
#[must_use]
pub fn item_content() -> Div {
    div()
        .named("item-content")
        .display(Display::Flex)
        .flex(Flex::grow(1.0))
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(4.0)
}

/// The title: words, or a [`gui::Text`] with styled spans; add a badge
/// beside it with `.child`.
#[must_use]
pub fn item_title(title: impl IntoElement) -> Div {
    // `w-fit` in the content column: as wide as its text, not stretched.
    div()
        .named("item-title")
        .display(Display::Flex)
        .align_self(Align::Start)
        .align_items(Align::Center)
        .gap(8.0)
        .text_size(14.0)
        .line_height(20.0)
        .line_height(LineHeight::Relative(1.375))
        .font_weight(500)
        .child(title)
}

/// Up to two lines of muted text under the title.
#[must_use]
pub fn item_description(description: impl Into<String>) -> impl IntoElement {
    let t = theme();
    text(description)
        .named("item-description")
        .text_color(t.muted_foreground)
        .line_clamp(2)
        .text_size(14.0)
        .line_height(20.0)
        .line_height(LineHeight::Relative(1.5))
        .font_weight(400)
}

/// Buttons at the end of an item.
#[must_use]
pub fn item_actions() -> Div {
    div()
        .named("item-actions")
        .display(Display::Flex)
        .align_items(Align::Center)
        .gap(8.0)
}

/// A full-width row above the rest of the item.
#[must_use]
pub fn item_header() -> Div {
    div()
        .named("item-header")
        .display(Display::Flex)
        .flex_basis(gui::Length::Fraction(1.0))
        .align_items(Align::Center)
        .justify_content(Justify::Between)
        .gap(8.0)
}

/// A full-width row below the rest of the item.
#[must_use]
pub fn item_footer() -> Div {
    div()
        .named("item-footer")
        .display(Display::Flex)
        .flex_basis(gui::Length::Fraction(1.0))
        .align_items(Align::Center)
        .justify_content(Justify::Between)
        .gap(8.0)
}

/// A list of items.
#[must_use]
pub fn item_group() -> Div {
    div()
        .named("item-group")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
}

/// The rule between items in a group.
#[must_use]
pub fn item_separator() -> Div {
    separator(Orientation::Horizontal).named("item-separator").margin_y(0.0)
}
