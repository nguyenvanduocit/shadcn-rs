//! Pagination (`ui/pagination`): page links between previous and next,
//! styled as ghost buttons with the current page outlined.
//!
//! The behaviour is `primitives::pagination` (reka's `Pagination`):
//! [`pagination`] keeps the page, from 1; the list below it is worked out
//! from the page, the page count and the siblings; the step buttons are
//! disabled at their ends. This file draws the parts.

use gui::prelude::*;
use gui::{Bind, ComponentElement, Div, Icon, Model, component, div, icon, provide, text, use_model};
use primitives::Availability;
use primitives::context::use_root;
use primitives::pagination::{Entry, Pagination, PaginationOptions, Step};

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
    disabled: bool,
    clamp: bool,
}

impl Default for PaginationProps {
    fn default() -> Self {
        Self {
            page: Bind::default_value(1),
            total: 0,
            items_per_page: 10,
            // shadcn's demos list one page each side; reka's default is 2.
            sibling_count: 1,
            show_edges: false,
            disabled: false,
            clamp: false,
        }
    }
}

impl PaginationProps {
    fn options(&self) -> PaginationOptions {
        PaginationOptions {
            total: self.total,
            items_per_page: self.items_per_page,
            siblings: self.sibling_count,
            show_edges: self.show_edges,
            availability: Availability::from_disabled(self.disabled),
            clamp: self.clamp,
        }
    }
}

/// What the parts below [`pagination`] read: its primitive.
#[derive(Clone, Copy)]
struct PaginationContext(Pagination<PartProps<PaginationProps>>);

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
            let pagination = Pagination::new(
                use_model(props, |p: &PartProps<PaginationProps>| &p.props.page),
                move || props.with(|p| p.props.options()),
            );
            provide(PaginationContext(pagination));
            move || pagination.root(props.with(|p| p.slot.0.clone())).into_element()
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
    pub fn on_change(self, f: impl Into<gui::Callback<usize>>) -> Self {
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

    /// Every button disabled (reka's `disabled`).
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }

    /// A page the caller sets out of range is corrected, and the caller
    /// told (reka's `clamp`; off unless set).
    #[must_use]
    pub fn clamp(self, clamp: bool) -> Self {
        self.with_props(|p| p.clamp = clamp)
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
            let ctx = use_root::<PaginationContext>("pagination_list", "pagination");
            move || {
                let row = props.with(|p| p.slot.0.clone());
                let Some(PaginationContext(pagination)) = ctx else {
                    return row.into_element();
                };
                row.children(pagination.entries().into_iter().map(|e| match e {
                    Entry::Page(n) => pagination_item(n).into_element(),
                    Entry::Ellipsis => pagination_ellipsis().into_element(),
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
        let ctx = use_root::<PaginationContext>("pagination_item", "pagination");
        move || {
            let page = props.with(|p| p.page);
            let active = ctx.is_some_and(|PaginationContext(c)| c.is_current(page));
            let variant = if active {
                ButtonVariant::Outline
            } else {
                ButtonVariant::Ghost
            };
            let link = button(variant, ButtonSize::Icon)
                .named("pagination-item")
                .child(page.to_string())
                .into_div();
            match ctx {
                Some(PaginationContext(pagination)) => pagination.page_button(link, page).into_element(),
                None => link.into_element(),
            }
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

fn step(to: Step, root: Button) -> Part<PaginationStepProps> {
    Part::new(PaginationStepProps { to }, root.into_div(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = use_root::<PaginationContext>("pagination_step", "pagination");
            move || {
                let (to, root) = props.with(|p| (p.props.to, p.slot.0.clone()));
                match ctx {
                    Some(PaginationContext(pagination)) => pagination.step_button(root, to).into_element(),
                    None => root.into_element(),
                }
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
