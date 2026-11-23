//! Page links between first, previous, next and last (reka's `Pagination`:
//! `PaginationRoot`, `PaginationList`, `PaginationListItem`,
//! `PaginationEllipsis`, `PaginationFirst`, `PaginationPrev`,
//! `PaginationNext`, `PaginationLast`).
//!
//! **Model.** The current page, from 1, the part's `v-model:page`; 1 unless
//! the caller says otherwise (`Pagination/PaginationRoot.vue:56-62`). There
//! are N = max(1, ⌈total / per page⌉) pages, a page of 0 items counting as
//! one (`:87`). What the user does is kept within 1..=N (`:91-96`): a page
//! button goes to its page, First to 1, Previous one back, Next one on,
//! Last to N. A page the caller writes out of that range stays as written,
//! no button then being the current one, unless the caller asks for it to
//! be clamped: then it is corrected and the caller told once, from the
//! first value on, as reka's `immediate` watch does: a pagination made on
//! page 15 of 10 shows page 10 in its first frame
//! (`:91-106`; `Pagination/Pagination.test.ts:310-381`). Fewer pages later
//! leave the page as it is unless clamped (`:265-307`).
//!
//! **The list** is worked out from the page p, N and the siblings s on
//! each side ([`range`], reka's `getRange`, `Pagination/utils.ts:20-93`):
//!
//! - without the first and last pages shown: a window of 2s+1 pages around
//!   p, moved inwards at the ends, all of them when N < 2s+1; never an
//!   ellipsis;
//! - with them shown and N ≤ 2s+5: every page, no ellipsis;
//! - with them shown and N > 2s+5: always 2s+5 entries, so the buttons do
//!   not move as the page changes: `1..k … N`, `1 … N−k+1..N` or
//!   `1 … p−s..p+s … N`, k = 2s+3, an ellipsis standing for 2 pages or
//!   more.
//!
//! **Keys.** None of its own: every enabled button is a Tab stop, Enter and
//! Space press it.
//!
//! **Availability.** The root's applies to every button. First and
//! Previous are disabled on page 1 (below it too), Next and Last on page N
//! (above it too); a page button only when the root is
//! (`Pagination/PaginationFirst.vue:18` and its three siblings,
//! `PaginationListItem.vue:22`). These are disabled, not only unavailable,
//! as reka's native `disabled`: the look and Chrome show them so, and Next
//! pressed onto the last page then lets focus go, as in Chrome (ADR-0025,
//! group 8).
//!
//! **To assistive technology.** The root is a navigation landmark; each
//! page button is named "Page N" and the current one is `aria-current`
//! page; the four steps are named "First Page", "Previous Page", "Next
//! Page", "Last Page" (`Pagination/PaginationListItem.vue:29-30`,
//! `PaginationFirst.vue:24-29`). Those are reka's English strings, the
//! defaults of [`PaginationNames`]; an app in another language gives its own,
//! a page's name a closure that may read the app's locale or message
//! catalogue (research §2.2 item 15, §4 item 9).

use std::rc::Rc;

use gui::prelude::*;

use gui::{AriaCurrent, Div, Signal, UseModel};

use crate::availability::Availability;

/// Pages listed each side of the current one unless the caller says
/// otherwise (`Pagination/PaginationRoot.vue:59`).
pub const SIBLINGS: usize = 2;

/// One entry of the page list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Entry {
    /// The button of a page, from 1.
    Page(usize),
    /// Pages left out.
    Ellipsis,
}

/// The four step buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    First,
    Previous,
    Next,
    Last,
}

/// What the caller decides of a pagination, read when it is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaginationOptions {
    /// How many items there are.
    pub total: usize,
    /// How many go on a page (reka requires it; 0 counts as 1).
    pub items_per_page: usize,
    /// Pages listed each side of the current one.
    pub siblings: usize,
    /// Always list the first and the last page, with ellipses between.
    pub show_edges: bool,
    pub availability: Availability,
    /// A page the caller writes out of range is corrected and told.
    pub clamp: bool,
}

impl Default for PaginationOptions {
    fn default() -> Self {
        Self {
            total: 0,
            items_per_page: 10,
            siblings: SIBLINGS,
            show_edges: false,
            availability: Availability::Enabled,
            clamp: false,
        }
    }
}

impl PaginationOptions {
    /// How many pages there are: never fewer than one.
    #[must_use]
    pub const fn page_count(self) -> usize {
        let per = if self.items_per_page == 0 {
            1
        } else {
            self.items_per_page
        };
        let n = self.total.div_ceil(per);
        if n == 0 { 1 } else { n }
    }
}

/// What a screen reader is told the buttons are: reka's English strings by
/// default.
#[derive(Clone)]
pub struct PaginationNames {
    /// The name of a page's button, from its number ("Page 3"): a closure,
    /// so it may read the app's locale or catalogue (tracked where a
    /// button is built).
    pub page: Rc<dyn Fn(usize) -> String>,
    pub first: String,
    pub previous: String,
    pub next: String,
    pub last: String,
}

fn page_name(page: usize) -> String {
    format!("Page {page}")
}

impl std::fmt::Debug for PaginationNames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaginationNames")
            .field("first", &self.first)
            .field("previous", &self.previous)
            .field("next", &self.next)
            .field("last", &self.last)
            .finish_non_exhaustive()
    }
}

impl Default for PaginationNames {
    fn default() -> Self {
        Self {
            page: Rc::new(page_name),
            first: "First Page".to_owned(),
            previous: "Previous Page".to_owned(),
            next: "Next Page".to_owned(),
            last: "Last Page".to_owned(),
        }
    }
}

/// reka's `getRange`: the entries listed for `page` of `count` pages with
/// `siblings` each side, the first and last always shown when `show_edges`.
#[must_use]
pub fn range(page: usize, count: usize, siblings: usize, show_edges: bool) -> Vec<Entry> {
    use Entry::{Ellipsis, Page};
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

struct State {
    options: Rc<dyn Fn() -> PaginationOptions>,
    names: PaginationNames,
}

/// A pagination's state: made once in its root's setup, `Copy`.
pub struct Pagination<P: 'static> {
    page: UseModel<P, usize>,
    state: Signal<State>,
}

impl<P: 'static> Clone for Pagination<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Pagination<P> {}

impl<P: 'static> PartialEq for Pagination<P> {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}

impl<P: 'static> std::fmt::Debug for Pagination<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pagination").finish_non_exhaustive()
    }
}

impl<P: 'static> Pagination<P> {
    /// A pagination on the page `page` holds (the part's `v-model:page`),
    /// with the caller's `options` read when used. Called in setup: a
    /// clamping pagination corrects an out-of-range page from here on, its
    /// first value included (reka's `immediate` watch).
    #[must_use]
    #[track_caller]
    pub fn new(page: UseModel<P, usize>, options: impl Fn() -> PaginationOptions + 'static) -> Self {
        let options: Rc<dyn Fn() -> PaginationOptions> = Rc::new(options);
        let me = Self {
            page,
            state: Signal::new(State {
                options: Rc::clone(&options),
                names: PaginationNames::default(),
            }),
        };
        // Runs now, on the first value too: a page made out of range is
        // corrected before the first frame shows it.
        gui::Effect::new(move || {
            let o = options();
            let at = page.get();
            if o.clamp {
                let fixed = at.clamp(1, o.page_count());
                if fixed != at {
                    gui::untracked(|| page.set(fixed));
                }
            }
        });
        me
    }

    /// Tell a screen reader the buttons by `names` from now on.
    pub fn set_names(self, names: PaginationNames) {
        self.state.update(|s| s.names = names);
    }

    /// The caller's options now (tracked through what its closure reads).
    #[must_use]
    pub fn options(self) -> PaginationOptions {
        let f = self.state.with_untracked(|s| Rc::clone(&s.options));
        f()
    }

    /// The current page (tracked).
    #[must_use]
    pub fn page(self) -> usize {
        self.page.get()
    }

    /// How many pages there are (tracked).
    #[must_use]
    pub fn page_count(self) -> usize {
        self.options().page_count()
    }

    /// The entries to list now (tracked).
    #[must_use]
    pub fn entries(self) -> Vec<Entry> {
        let o = self.options();
        range(self.page(), o.page_count(), o.siblings, o.show_edges)
    }

    /// The user went to `page`: kept within the pages there are.
    pub fn go(self, page: usize) {
        let count = gui::untracked(|| self.page_count());
        self.page.set(page.clamp(1, count));
    }

    /// Where `step` leads from the current page (tracked).
    #[must_use]
    pub fn target(self, step: Step) -> usize {
        let (page, count) = (self.page(), self.page_count());
        match step {
            Step::First => 1,
            Step::Previous => page.saturating_sub(1),
            Step::Next => page.saturating_add(1),
            Step::Last => count,
        }
    }

    /// Whether `step`'s button can be used now (tracked): what its look
    /// dims by.
    #[must_use]
    pub fn step_availability(self, step: Step) -> Availability {
        let o = self.options();
        if o.availability != Availability::Enabled {
            return o.availability;
        }
        let (page, count) = (self.page(), o.page_count());
        let at_end = match step {
            Step::First | Step::Previous => page <= 1,
            Step::Next | Step::Last => page >= count,
        };
        if at_end {
            Availability::Disabled
        } else {
            Availability::Enabled
        }
    }

    /// Whether `page` is the current one (tracked): its look shows it.
    #[must_use]
    pub fn is_current(self, page: usize) -> bool {
        self.page() == page
    }

    /// `d` as the root: a navigation landmark.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        d.role(gui::Role::Navigation)
    }

    /// `d`, the look's button, as the button of `page`: named for it,
    /// current when it is, pressed it goes there. Sets `on_click`.
    #[must_use]
    pub fn page_button(self, d: Div, page: usize) -> Div {
        let name = self.state.with_untracked(|s| Rc::clone(&s.names.page))(page);
        let d = d.role(gui::Role::Button).focusable().aria_label(name);
        let d = self.options().availability.apply(d);
        d.when(self.is_current(page), |d| d.aria_current(AriaCurrent::Page))
            .on_click(move |_| self.go(page))
    }

    /// `d`, the look's button, as `step`'s: named for it, disabled at its
    /// end, pressed it moves the page. Sets `on_click`.
    #[must_use]
    pub fn step_button(self, d: Div, step: Step) -> Div {
        let name = self.state.with_untracked(|s| match step {
            Step::First => s.names.first.clone(),
            Step::Previous => s.names.previous.clone(),
            Step::Next => s.names.next.clone(),
            Step::Last => s.names.last.clone(),
        });
        let target = self.target(step);
        self.step_availability(step)
            .apply(d.role(gui::Role::Button).focusable().aria_label(name))
            .on_click(move |_| self.go(target))
    }
}

#[cfg(test)]
mod tests {
    use super::Entry::{Ellipsis, Page};
    use super::{Entry, PaginationOptions, range};

    fn pages(entries: &[Entry]) -> String {
        entries
            .iter()
            .map(|e| match e {
                Page(n) => n.to_string(),
                Ellipsis => "…".to_owned(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn the_list_of_reka_s_tests() {
        // `Pagination/Pagination.test.ts:173-194`.
        assert_eq!(pages(&range(1, 10, 1, true)), "1 2 3 4 5 … 10");
        assert_eq!(pages(&range(5, 10, 1, true)), "1 … 4 5 6 … 10");
        assert_eq!(pages(&range(7, 10, 1, true)), "1 … 6 7 8 9 10");
        assert_eq!(pages(&range(5, 10, 2, true)), "1 2 3 4 5 6 7 … 10");
    }

    #[test]
    fn edges_shown_keep_2s_plus_5_entries_and_hidden_never_an_ellipsis() {
        for s in 0..4 {
            for count in (2 * s + 6)..30 {
                for p in 1..=count {
                    assert_eq!(range(p, count, s, true).len(), 2 * s + 5, "s={s} N={count} p={p}");
                    let hidden = range(p, count, s, false);
                    assert!(!hidden.contains(&Ellipsis));
                    assert_eq!(hidden.len(), (2 * s + 1).min(count));
                    assert!(hidden.contains(&Page(p)), "the window holds the page");
                }
            }
        }
        assert_eq!(pages(&range(2, 7, 1, true)), "1 2 3 4 5 6 7", "N ≤ 2s+5: every page");
    }

    #[test]
    fn page_count_is_never_zero() {
        let o = |total, items_per_page| PaginationOptions {
            total,
            items_per_page,
            ..PaginationOptions::default()
        };
        assert_eq!(o(0, 10).page_count(), 1);
        assert_eq!(o(95, 10).page_count(), 10);
        assert_eq!(o(5, 0).page_count(), 5, "0 per page counts as 1");
    }
}
