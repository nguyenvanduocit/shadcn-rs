//! What a click, Enter or Space does to a selection (reka's
//! `useSelectionBehavior`, and Listbox's own copy of it), and Shift ranges.
//!
//! A single selection is an `Option<T>`, a multiple one a list of `T` in the
//! order chosen, or, for a list of any length, `gui::list::Selection`'s
//! ranges of data rows: two types, never one value that is sometimes `T`
//! and sometimes a list (research §3.2). Values compare by `T`'s
//! `PartialEq`, the one equality rule (ADR-0025).
//!
//! - Toggle, single: choosing the chosen item clears it, another chooses it.
//! - Replace, single: the item is chosen; choosing it again keeps it.
//! - Toggle, multiple: a chosen item leaves, another joins at the end; the
//!   others keep their order.
//! - Replace, multiple: only the item stays, and it becomes the anchor.
//! - Range (Shift): from the anchor to the item, both ends included, in
//!   display order, disabled and unavailable items left out (Listbox's
//!   rule; Tree keeps disabled ones inside a range, which one notion of
//!   "cannot be used" rules out). Ranges extend only under Replace, as reka
//!   has it: the anchor is set by a Replace choice, and a range keeps it.

use std::ops::Range;

use gui::list::Selection;

use crate::collection::Items;

/// How choosing an item changes the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SelectionBehavior {
    /// Choosing an item adds it or takes it away (reka's default).
    #[default]
    Toggle,
    /// Choosing an item makes it the selection.
    Replace,
}

/// The single selection after choosing `item` from `current`.
#[must_use]
pub fn choose_one<T: PartialEq + Clone>(current: Option<&T>, item: &T, behavior: SelectionBehavior) -> Option<T> {
    match behavior {
        SelectionBehavior::Toggle if current == Some(item) => None,
        SelectionBehavior::Toggle | SelectionBehavior::Replace => Some(item.clone()),
    }
}

/// The multiple selection after choosing `item` from `current`.
#[must_use]
pub fn choose_many<T: PartialEq + Clone>(current: &[T], item: &T, behavior: SelectionBehavior) -> Vec<T> {
    match behavior {
        SelectionBehavior::Replace => vec![item.clone()],
        SelectionBehavior::Toggle if current.contains(item) => current.iter().filter(|v| *v != item).cloned().collect(),
        SelectionBehavior::Toggle => current.iter().chain(std::iter::once(item)).cloned().collect(),
    }
}

/// The items from `anchor` to `to`, both ends included, that can be
/// chosen, as runs of indices in display order.
#[must_use]
pub fn range<I: Items + ?Sized>(items: &I, anchor: usize, to: usize) -> Vec<Range<usize>> {
    let (lo, hi) = if anchor <= to { (anchor, to) } else { (to, anchor) };
    let hi = hi.min(items.len().saturating_sub(1));
    let mut runs: Vec<Range<usize>> = Vec::new();
    let mut i = lo;
    while i <= hi && !items.is_empty() {
        if items.availability(i).actionable() {
            let next = i.saturating_add(1);
            match runs.last_mut() {
                Some(r) if r.end == i => r.end = next,
                _ => runs.push(i..next),
            }
        }
        let Some(next) = i.checked_add(1) else { break };
        i = next;
    }
    runs
}

/// A selection of data rows by index, with its anchor: what a list of any
/// length (a virtual listbox) keeps, on `gui::list::Selection`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rows {
    pub selection: Selection,
    /// Where a Shift range starts.
    pub anchor: Option<usize>,
}

impl Rows {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Choose row `row` of `items` as `behavior` says; Replace makes it the
    /// anchor. A row that cannot be chosen changes nothing.
    pub fn choose<I: Items + ?Sized>(&mut self, items: &I, row: usize, behavior: SelectionBehavior) {
        if !items.availability(row).actionable() {
            return;
        }
        let r = wide(row);
        match behavior {
            SelectionBehavior::Toggle => self.selection.toggle(r),
            SelectionBehavior::Replace => {
                self.selection.clear();
                self.selection.select(r..r.saturating_add(1));
                self.anchor = Some(row);
            }
        }
    }

    /// Extend from the anchor to row `to` (Shift with a click or an arrow):
    /// the range is the selection. Without an anchor `to` becomes it.
    pub fn extend<I: Items + ?Sized>(&mut self, items: &I, to: usize) {
        let anchor = *self.anchor.get_or_insert(to);
        self.selection.clear();
        for run in range(items, anchor, to) {
            self.selection.select(wide(run.start)..wide(run.end));
        }
    }

    /// Whether row `row` is selected.
    #[must_use]
    pub fn contains(&self, row: usize) -> bool {
        self.selection.contains(wide(row))
    }
}

/// A row index as `gui::list::Selection` keeps it.
fn wide(row: usize) -> u64 {
    u64::try_from(row).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{Rows, SelectionBehavior, choose_many, choose_one, range};
    use crate::availability::Availability;
    use crate::collection::from_fn;

    #[test]
    fn toggle_and_replace_on_one_and_on_many() {
        use SelectionBehavior::{Replace, Toggle};
        let a = "a".to_owned();
        let b = "b".to_owned();
        assert_eq!(choose_one(None, &a, Toggle), Some(a.clone()));
        assert_eq!(choose_one(Some(&a), &a, Toggle), None, "toggle clears the chosen");
        assert_eq!(choose_one(Some(&a), &b, Toggle), Some(b.clone()));
        assert_eq!(choose_one(Some(&a), &a, Replace), Some(a.clone()), "replace keeps it");
        let many = vec![a.clone(), b.clone()];
        assert_eq!(choose_many(&many, &a, Toggle), vec![b.clone()]);
        assert_eq!(
            choose_many(std::slice::from_ref(&b), &a, Toggle),
            vec![b.clone(), a.clone()],
            "joins at the end"
        );
        assert_eq!(choose_many(&many, &b, Replace), vec![b.clone()]);
    }

    #[test]
    fn ranges_run_both_ways_and_leave_out_what_cannot_be_chosen() {
        let items = from_fn(
            8,
            |i| match i {
                3 => Availability::Disabled,
                5 => Availability::Unavailable,
                _ => Availability::Enabled,
            },
            |_| Cow::Borrowed(""),
        );
        assert_eq!(range(&items, 1, 6), vec![1..3, 4..5, 6..7]);
        assert_eq!(range(&items, 6, 1), vec![1..3, 4..5, 6..7], "from either end");
        assert_eq!(range(&items, 2, 2), vec![2..3]);
        assert_eq!(range(&items, 6, 99), vec![6..8], "clipped to the list");
    }

    #[test]
    fn rows_keep_an_anchor_over_millions() {
        let n = 5_000_000;
        let items = from_fn(n, |i| Availability::from_disabled(i == 10), |_| Cow::Borrowed(""));
        let mut rows = Rows::new();
        rows.choose(&items, 4, SelectionBehavior::Replace);
        rows.extend(&items, 12);
        assert!(rows.contains(4) && rows.contains(9) && rows.contains(12));
        assert!(!rows.contains(10), "a disabled row is no part of a range");
        assert_eq!(rows.selection.count(u64::try_from(n).unwrap_or(0)), 8);
        rows.extend(&items, 2);
        assert!(
            rows.contains(2) && rows.contains(4) && !rows.contains(5),
            "the anchor stays"
        );
        rows.choose(&items, 10, SelectionBehavior::Toggle);
        assert!(!rows.contains(10), "a disabled row is not chosen");
    }
}
