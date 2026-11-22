//! Breadcrumb (`ui/breadcrumb`): the path to the current page, as links
//! between chevrons.

use gui::prelude::*;
use gui::{Div, Icon, div, icon, text};

use crate::focus::StateLook;
use crate::theme::theme;

/// The `<nav>`.
#[must_use]
pub fn breadcrumb() -> Div {
    div().named("breadcrumb")
}

/// The `<ol>`: muted, wrapping; `sm:gap-2.5` taken as the desktop width it is.
#[must_use]
pub fn breadcrumb_list() -> Div {
    let t = theme();
    div()
        .named("breadcrumb-list")
        .text_color(t.muted_foreground)
        .display(Display::Flex)
        .flex_wrap(true)
        .align_items(Align::Center)
        .gap(10.0)
        .text_size(14.0)
        .line_height(20.0)
}

/// An `<li>`: holds a link, the page, or a menu.
#[must_use]
pub fn breadcrumb_item() -> Div {
    div()
        .named("breadcrumb-item")
        .display(Display::Flex)
        .inline(true)
        .align_items(Align::Center)
        .gap(6.0)
}

/// A link to an ancestor page; add where it goes with `.on_click`.
#[must_use]
pub fn breadcrumb_link(label: impl Into<String>) -> Div {
    let t = theme();
    div()
        .named("breadcrumb-link")
        .transition(Properties::COLORS)
        .cursor(Cursor::Pointer)
        .focusable()
        .with_interaction(|d, i| d.text_color_while(move || i.hovered(), t.foreground))
        .child(text(label))
}

/// The current page: not a link.
#[must_use]
pub fn breadcrumb_page(label: impl Into<String>) -> Div {
    let t = theme();
    div()
        .named("breadcrumb-page")
        .text_color(t.foreground)
        .font_weight(400)
        .child(text(label))
}

/// The chevron between two items.
#[must_use]
pub fn breadcrumb_separator() -> Div {
    breadcrumb_separator_of(Icon::ChevronRight)
}

/// A separator drawn as `glyph` (the slot `<BreadcrumbSeparator><SlashIcon /></…>` fills).
#[must_use]
pub fn breadcrumb_separator_of(glyph: Icon) -> Div {
    div()
        .named("breadcrumb-separator")
        .display(Display::Flex)
        .child(icon(glyph).size(14.0))
}

/// Collapsed items, shown as `…`.
#[must_use]
pub fn breadcrumb_ellipsis() -> Div {
    div()
        .named("breadcrumb-ellipsis")
        .display(Display::Flex)
        .size(36.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .child(icon(Icon::Ellipsis))
}
