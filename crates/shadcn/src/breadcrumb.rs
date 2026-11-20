//! Breadcrumb (`ui/breadcrumb`): the path to the current page, as links
//! between chevrons.

use gui::prelude::*;
use gui::{Div, Icon, div, icon, text};

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
        .flex()
        .flex_wrap()
        .items_center()
        .gap(2.5)
        .text_sm()
}

/// An `<li>`: holds a link, the page, or a menu.
#[must_use]
pub fn breadcrumb_item() -> Div {
    div().named("breadcrumb-item").inline_flex().items_center().gap(1.5)
}

/// A link to an ancestor page; add where it goes with `.on_click`.
#[must_use]
pub fn breadcrumb_link(label: impl Into<String>) -> Div {
    let t = theme();
    div()
        .named("breadcrumb-link")
        .transition_colors()
        .cursor_pointer()
        .focusable()
        .hover(move |s| s.text_color(t.foreground))
        .child(text(label))
}

/// The current page: not a link.
#[must_use]
pub fn breadcrumb_page(label: impl Into<String>) -> Div {
    let t = theme();
    div()
        .named("breadcrumb-page")
        .text_color(t.foreground)
        .font_normal()
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
    div().named("breadcrumb-separator").flex().child(icon(glyph).size(3.5))
}

/// Collapsed items, shown as `…`.
#[must_use]
pub fn breadcrumb_ellipsis() -> Div {
    div()
        .named("breadcrumb-ellipsis")
        .flex()
        .size(9.0)
        .items_center()
        .justify_center()
        .child(icon(Icon::Ellipsis))
}
