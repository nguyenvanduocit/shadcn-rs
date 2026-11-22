//! Pagination (`ui/pagination`): page links between previous and next,
//! styled as ghost buttons with the current page outlined.
//!
//! [`pagination`] keeps the page; the links below it read and move it.

use gui::prelude::*;
use gui::{
    Bind, ComponentElement, Div, Icon, Model, Signal, UseModel, component, div, icon, provide, text, use_context,
    use_model,
};

use crate::button::{Button, ButtonSize, ButtonVariant, button};
use crate::part::{Part, PartProps};

/// What [`pagination`] is given (reka's `PaginationRoot`); `page` is
/// 1-based, as reka's.
#[derive(Clone, Debug, PartialEq)]
pub struct PaginationProps {
    page: Bind<usize>,
    total: usize,
    items_per_page: usize,
    sibling_count: usize,
    show_edges: bool,
}

impl Default for PaginationProps {
    fn default() -> Self {
        Self {
            page: Bind::default_value(1),
            total: 0,
            items_per_page: 10,
            sibling_count: 1,
            show_edges: false,
        }
    }
}

/// One entry of the page list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaginationEntry {
    Page(usize),
    Ellipsis,
}

/// What the parts below [`pagination`] read.
#[derive(Clone)]
struct PaginationContext {
    props: Signal<PartProps<PaginationProps>>,
    page: UseModel<PartProps<PaginationProps>, usize>,
}

impl PaginationContext {
    fn find() -> Option<Self> {
        use_context::<Self>()
    }

    fn page_count(&self) -> usize {
        self.props
            .with(|p| p.props.total.div_ceil(p.props.items_per_page.max(1)).max(1))
    }

    /// The pages and ellipses to list around the current page.
    fn entries(&self) -> Vec<PaginationEntry> {
        let (siblings, edges) = self.props.with(|p| (p.props.sibling_count, p.props.show_edges));
        pagination_range(self.page.get(), self.page_count(), siblings, edges)
    }

    /// Go to `page`, kept within the pages there are.
    fn go(&self, page: usize) {
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

/// The `<nav>`: centred in its row. It keeps the page: its own, from
/// `default_page`, the caller's through `.model(…)` (`v-model:page`), or one
/// the caller shows (`.page(…)` with `.on_change(…)`).
#[must_use]
pub fn pagination() -> Part<PaginationProps> {
    let root = div()
        .named("pagination")
        .margin_x(Length::Auto)
        .display(Display::Flex)
        .width(pct(100.0))
        .justify_content(Justify::Center);
    Part::new(PaginationProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(PaginationContext {
                props,
                page: use_model(props, |p| &p.props.page),
            });
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

impl Part<PaginationProps> {
    /// How many items there are, `items_per_page` to a page.
    #[must_use]
    pub fn total(self, total: usize, items_per_page: usize) -> Self {
        self.with_props(|p| {
            p.total = total;
            p.items_per_page = items_per_page;
        })
    }

    /// The page shown first, when no model is given (reka's `default-page`).
    #[must_use]
    pub fn default_page(self, page: usize) -> Self {
        self.with_props(|p| p.page = std::mem::take(&mut p.page).with_default(page.max(1)))
    }

    /// The page, kept by the caller (`v-model:page`).
    #[must_use]
    pub fn model(self, page: impl Into<Model<usize>>) -> Self {
        self.with_props(|p| p.page = std::mem::take(&mut p.page).with_model(page))
    }

    /// The page as the caller shows it (`:page`); a move only calls
    /// `on_change`.
    #[must_use]
    pub fn page(self, page: usize) -> Self {
        self.with_props(|p| p.page = std::mem::take(&mut p.page).with_value(page.max(1)))
    }

    /// Called with each page moved to (`@update:page`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(usize) + 'static) -> Self {
        self.with_props(|p| p.page = std::mem::take(&mut p.page).on_change(f))
    }

    /// Pages listed each side of the current one (1 unless set).
    #[must_use]
    pub fn sibling_count(self, count: usize) -> Self {
        self.with_props(|p| p.sibling_count = count)
    }

    /// Always list the first and last page, with ellipses between.
    #[must_use]
    pub fn show_edges(self, show: bool) -> Self {
        self.with_props(|p| p.show_edges = show)
    }
}

/// The list of links.
#[must_use]
pub fn pagination_content() -> Div {
    div()
        .named("pagination-content")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Row)
        .align_items(Align::Center)
        .gap(4.0)
}

/// The pages and ellipses around the current page, in a
/// [`pagination_content`] row (reka's `PaginationList` and its `items`).
#[must_use]
pub fn pagination_list() -> Part<()> {
    Part::new((), pagination_content(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = PaginationContext::find();
            move || {
                let row = props.with(|p| p.slot.0.clone());
                let Some(ctx) = &ctx else {
                    return row.into_element();
                };
                row.children(ctx.entries().into_iter().map(|e| match e {
                    PaginationEntry::Page(n) => pagination_item(n).into_element(),
                    PaginationEntry::Ellipsis => pagination_ellipsis().into_element(),
                }))
                .into_element()
            }
        })
    })
}

/// What a page link is given: its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaginationItemProps {
    page: usize,
}

/// A page link labelled with its number: outlined when it is the current
/// page.
#[must_use]
pub fn pagination_item(page: usize) -> ComponentElement {
    component(PaginationItemProps { page }, |cx| {
        let props = cx.props();
        let ctx = PaginationContext::find();
        move || {
            let page = props.with(|p| p.page);
            let active = ctx.as_ref().is_some_and(|c| c.page.get() == page);
            let go = ctx.clone();
            let variant = if active {
                ButtonVariant::Outline
            } else {
                ButtonVariant::Ghost
            };
            button(variant, ButtonSize::Icon)
                .named("pagination-item")
                .on_click(move |_| {
                    if let Some(go) = &go {
                        go.go(page);
                    }
                })
                .child(page.to_string())
                .into_element()
        }
    })
}

/// The gap in the list where pages are skipped.
#[must_use]
pub fn pagination_ellipsis() -> Div {
    div()
        .named("pagination-ellipsis")
        .display(Display::Flex)
        .size(36.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .child(icon(Icon::Ellipsis))
}

/// Which way a step button goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaginationStepProps {
    to: Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    First,
    Previous,
    Next,
    Last,
}

fn step(to: Step, root: Button) -> Part<PaginationStepProps> {
    Part::new(PaginationStepProps { to }, root.into_div(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = PaginationContext::find();
            move || {
                let (to, root) = props.with(|p| (p.props.to, p.slot.0.clone()));
                let Some(ctx) = ctx.clone() else {
                    return root.into_element();
                };
                let (page, count) = (ctx.page.get(), ctx.page_count());
                let (target, enabled) = match to {
                    Step::First => (1, page > 1),
                    Step::Previous => (page.saturating_sub(1), page > 1),
                    Step::Next => (page.saturating_add(1), page < count),
                    Step::Last => (count, page < count),
                };
                root.disabled(!enabled).on_click(move |_| ctx.go(target)).into_element()
            }
        })
    })
}

fn step_button() -> Button {
    button(ButtonVariant::Ghost, ButtonSize::Default)
        .gap(4.0)
        .padding_x(10.0)
}

/// Back one page; disabled on the first.
#[must_use]
pub fn pagination_previous() -> Part<PaginationStepProps> {
    let root = step_button()
        .named("pagination-previous")
        .child(icon(Icon::ChevronLeft))
        .child(text("Previous"));
    step(Step::Previous, root)
}

/// On one page; disabled on the last.
#[must_use]
pub fn pagination_next() -> Part<PaginationStepProps> {
    let root = step_button()
        .named("pagination-next")
        .child(text("Next"))
        .child(icon(Icon::ChevronRight));
    step(Step::Next, root)
}

/// To the first page; disabled on it.
#[must_use]
pub fn pagination_first() -> Part<PaginationStepProps> {
    let root = step_button()
        .named("pagination-first")
        .child(icon(Icon::ChevronLeft))
        .child(text("First"));
    step(Step::First, root)
}

/// To the last page; disabled on it.
#[must_use]
pub fn pagination_last() -> Part<PaginationStepProps> {
    let root = step_button()
        .named("pagination-last")
        .child(text("Last"))
        .child(icon(Icon::ChevronRight));
    step(Step::Last, root)
}
