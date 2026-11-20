//! Pagination (`ui/pagination`): page links between previous and next,
//! styled as ghost buttons with the current page outlined.

use gui::prelude::*;
use gui::{Div, Icon, Signal, div, icon, text};

use crate::button::{ButtonSize, ButtonVariant, button};

/// Which page is shown out of how many; `page` is 1-based, as reka's.
#[derive(Clone)]
pub struct PaginationState {
    pub page: Signal<usize>,
    pub total: usize,
    pub items_per_page: usize,
    /// Pages shown each side of the current one.
    pub sibling_count: usize,
    /// Always show the first and last page, with ellipses between.
    pub show_edges: bool,
}

/// One entry of the page list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaginationEntry {
    Page(usize),
    Ellipsis,
}

impl PaginationState {
    /// `total` items, `items_per_page` to a page, opening on `default_page`.
    #[must_use]
    pub fn new(total: usize, items_per_page: usize, default_page: usize) -> Self {
        Self {
            page: Signal::labeled("pagination page", default_page.max(1)),
            total,
            items_per_page,
            sibling_count: 1,
            show_edges: false,
        }
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.total.div_ceil(self.items_per_page.max(1)).max(1)
    }

    /// The pages and ellipses to list around the current page.
    #[must_use]
    pub fn entries(&self) -> Vec<PaginationEntry> {
        pagination_range(self.page.get(), self.page_count(), self.sibling_count, self.show_edges)
    }

    /// Go to `page`, kept within the pages there are.
    pub fn go(&self, page: usize) {
        self.page.set(page.clamp(1, self.page_count()));
    }
}

/// reka-ui's `getRange`: which pages to list for `page` of `count`.
#[must_use]
pub fn pagination_range(page: usize, count: usize, siblings: usize, show_edges: bool) -> Vec<PaginationEntry> {
    use PaginationEntry::{Ellipsis, Page};
    let pages = |from: usize, to: usize| (from..=to).map(Page);
    let left = page.saturating_sub(siblings).max(1);
    let right = page.saturating_add(siblings).min(count);
    if show_edges {
        let total_numbers = siblings.saturating_mul(2).saturating_add(5).min(count);
        let item_count = total_numbers.saturating_sub(2);
        let show_left = left > 3 && count.abs_diff(item_count) > 2 && left.abs_diff(1) > 2;
        let show_right = right.saturating_add(2) < count && count.abs_diff(item_count) > 2 && count.abs_diff(right) > 2;
        return match (show_left, show_right) {
            (false, true) => pages(1, item_count).chain([Ellipsis, Page(count)]).collect(),
            (true, false) => [Page(1), Ellipsis]
                .into_iter()
                .chain(pages(count.saturating_sub(item_count).saturating_add(1), count))
                .collect(),
            (true, true) => [Page(1), Ellipsis]
                .into_iter()
                .chain(pages(left, right))
                .chain([Ellipsis, Page(count)])
                .collect(),
            (false, false) => pages(1, count).collect(),
        };
    }
    let item_count = siblings.saturating_mul(2).saturating_add(1);
    if count < item_count {
        pages(1, count).collect()
    } else if page <= siblings.saturating_add(1) {
        pages(1, item_count).collect()
    } else if count.saturating_sub(page) <= siblings {
        pages(count.saturating_sub(item_count).saturating_add(1), count).collect()
    } else {
        pages(left, right).collect()
    }
}

/// The `<nav>`: centred in its row.
#[must_use]
pub fn pagination() -> Div {
    div().named("pagination").mx_auto().flex().w_full().justify_center()
}

/// The list of links.
#[must_use]
pub fn pagination_content() -> Div {
    div().named("pagination-content").flex_row().items_center().gap(1.0)
}

/// A page link: outlined when it is the current page.
#[must_use]
pub fn pagination_item(state: &PaginationState, page: usize) -> Div {
    let active = state.page.get() == page;
    let go = state.clone();
    button(
        if active {
            ButtonVariant::Outline
        } else {
            ButtonVariant::Ghost
        },
        ButtonSize::Icon,
    )
    .named("pagination-item")
    .on_click(move |_| go.go(page))
    .child(page.to_string())
}

/// The gap in the list where pages are skipped.
#[must_use]
pub fn pagination_ellipsis() -> Div {
    div()
        .named("pagination-ellipsis")
        .flex()
        .size(9.0)
        .items_center()
        .justify_center()
        .child(icon(Icon::Ellipsis))
}

fn step(state: &PaginationState, to: usize, enabled: bool) -> Div {
    let go = state.clone();
    button(ButtonVariant::Ghost, ButtonSize::Default)
        .gap(1.0)
        .px(2.5)
        .disabled(!enabled)
        .on_click(move |_| go.go(to))
}

/// Back one page; disabled on the first.
#[must_use]
pub fn pagination_previous(state: &PaginationState) -> Div {
    let page = state.page.get();
    step(state, page.saturating_sub(1), page > 1)
        .named("pagination-previous")
        .child(icon(Icon::ChevronLeft))
        .child(text("Previous"))
}

/// On one page; disabled on the last.
#[must_use]
pub fn pagination_next(state: &PaginationState) -> Div {
    let page = state.page.get();
    step(state, page.saturating_add(1), page < state.page_count())
        .named("pagination-next")
        .child(text("Next"))
        .child(icon(Icon::ChevronRight))
}

/// To the first page; disabled on it.
#[must_use]
pub fn pagination_first(state: &PaginationState) -> Div {
    let page = state.page.get();
    step(state, 1, page > 1)
        .named("pagination-first")
        .child(icon(Icon::ChevronLeft))
        .child(text("First"))
}

/// To the last page; disabled on it.
#[must_use]
pub fn pagination_last(state: &PaginationState) -> Div {
    let page = state.page.get();
    let count = state.page_count();
    step(state, count, page < count)
        .named("pagination-last")
        .child(text("Last"))
        .child(icon(Icon::ChevronRight))
}
