//! Item (`ui/item`): a row of media, content and actions, alone or in a group.

use gui::prelude::*;
use gui::{Color, Div, div, text};

use crate::focus::focus_ring;
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
        .transition_colors()
        .duration_ms(100.0)
        .flex()
        .items_center()
        .border()
        .border_color(Color::TRANSPARENT)
        .text_sm()
        .rounded_md()
        .flex_wrap()
        .focus_visible(move |s| focus_ring(s, &t));
    let d = match variant {
        ItemVariant::Default => d,
        ItemVariant::Outline => d.border_color(t.border),
        ItemVariant::Muted => d.bg(t.muted.opacity(0.5)),
    };
    match size {
        ItemSize::Default => d.p(4.0).gap(4.0),
        ItemSize::Sm => d.py(3.0).px(4.0).gap(2.5),
    }
}

/// An item that is a link (`as-child` on an `<a>`): focusable, tinted on
/// hover; add where it goes with `.on_click`.
#[must_use]
pub fn item_link(variant: ItemVariant, size: ItemSize) -> Div {
    let t = theme();
    item(variant, size)
        .cursor_pointer()
        .focusable()
        .hover(move |s| s.bg(t.accent.opacity(0.5)))
}

/// The icon, avatar or image at the start of an item.
#[must_use]
pub fn item_media(variant: ItemMediaVariant) -> Div {
    let t = theme();
    let d = div()
        .named("item-media")
        .flex()
        .shrink_0()
        .items_center()
        .justify_center()
        .gap(2.0);
    match variant {
        ItemMediaVariant::Default => d,
        ItemMediaVariant::Icon => d.size(8.0).border().border_color(t.border).rounded_sm().bg(t.muted),
        ItemMediaVariant::Image => d.size(10.0).rounded_sm().overflow_hidden(),
    }
}

/// The media of an item whose content has an [`item_description`]
/// (`group-has-[[data-slot=item-description]]/item:self-start
/// translate-y-0.5`): at the top, beside the title, not centred on the
/// taller title-and-description column. The item cannot see what its
/// children hold, so its caller picks this over [`item_media`].
#[must_use]
pub fn item_media_described(variant: ItemMediaVariant) -> Div {
    item_media(variant).self_start().relative().top(0.5)
}

/// Title and description, taking the free width.
#[must_use]
pub fn item_content() -> Div {
    div().named("item-content").flex().flex_1().flex_col().gap(1.0)
}

/// The title: words, or a [`gui::Text`] with styled spans; add a badge
/// beside it with `.child`.
#[must_use]
pub fn item_title(title: impl IntoElement) -> Div {
    // `w-fit` in the content column: as wide as its text, not stretched.
    div()
        .named("item-title")
        .flex()
        .self_start()
        .items_center()
        .gap(2.0)
        .text_sm()
        .leading_snug()
        .font_medium()
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
        .text_sm()
        .leading_normal()
        .font_normal()
}

/// Buttons at the end of an item.
#[must_use]
pub fn item_actions() -> Div {
    div().named("item-actions").flex().items_center().gap(2.0)
}

/// A full-width row above the rest of the item.
#[must_use]
pub fn item_header() -> Div {
    div()
        .named("item-header")
        .flex()
        .basis_len(gui::relative(1.0))
        .items_center()
        .justify_between()
        .gap(2.0)
}

/// A full-width row below the rest of the item.
#[must_use]
pub fn item_footer() -> Div {
    div()
        .named("item-footer")
        .flex()
        .basis_len(gui::relative(1.0))
        .items_center()
        .justify_between()
        .gap(2.0)
}

/// A list of items.
#[must_use]
pub fn item_group() -> Div {
    div().named("item-group").flex_col()
}

/// The rule between items in a group.
#[must_use]
pub fn item_separator() -> Div {
    separator(Orientation::Horizontal).named("item-separator").my(0.0)
}
