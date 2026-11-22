//! What a list shows of its data, and what is selected in it (ADR-0014).
//!
//! A [`View`] is a permutation: the data rows a sort and a filter keep, in
//! their order. It is built off the UI thread from a [`Snapshot`] (shared,
//! not copied) and delivered through a [`mailbox`](crate::mailbox), so
//! sorting a million rows never holds a frame. A [`Selection`] is kept in
//! data rows, as ranges, with an "all except" mode, so it survives a sort
//! or a filter unchanged and moves with the data when rows are spliced.

use std::cmp::Ordering;
use std::ops::Range;
use std::sync::Arc;

use crate::mailbox::Sender;
use crate::reactive::Snapshot;

/// How two rows of data compare, for a sort.
pub type Order<T> = dyn Fn(&T, &T) -> Ordering;

/// The data rows a list shows, in the order it shows them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    rows: Arc<Vec<u32>>,
    /// The data's version it was built from.
    version: u64,
}

impl View {
    /// Every row of `len`, in data order.
    #[must_use]
    pub fn identity(len: u32) -> Self {
        Self {
            rows: Arc::new((0..len).collect()),
            version: 0,
        }
    }

    /// The rows of `items` that `keep` keeps, ordered by `order` (a stable
    /// sort: equal rows keep their data order), or in data order.
    pub fn build<T: Clone>(items: &Snapshot<T>, keep: impl Fn(&T) -> bool, order: Option<&Order<T>>) -> Self {
        let mut kept: Vec<(u32, &T)> = items
            .iter()
            .zip(0_u32..)
            .filter(|(t, _)| keep(t))
            .map(|(t, i)| (i, t))
            .collect();
        if let Some(cmp) = order {
            kept.sort_by(|a, b| cmp(a.1, b.1));
        }
        Self {
            rows: Arc::new(kept.into_iter().map(|(i, _)| i).collect()),
            version: items.version(),
        }
    }

    /// Build a view on a thread of its own and send it to `to` (a mailbox
    /// whose handler stores it in a signal the list reads).
    pub fn spawn<T, K, O>(items: Snapshot<T>, keep: K, order: Option<O>, to: Sender<Self>)
    where
        T: Clone + Send + Sync + 'static,
        K: Fn(&T) -> bool + Send + 'static,
        O: Fn(&T, &T) -> Ordering + Send + 'static,
        Snapshot<T>: Send,
    {
        std::thread::spawn(move || {
            let view = Self::build(&items, keep, order.as_ref().map(|o| -> &Order<T> { o }));
            to.send(view);
        });
    }

    /// Rows shown.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The data row shown at `position`.
    #[must_use]
    pub fn row(&self, position: usize) -> Option<u32> {
        self.rows.get(position).copied()
    }

    /// The data's version the view was built from (a newer view replaces an older one).
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }
}

/// Selected data rows: ranges, or all rows but ranges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// The ranges are the rows *not* selected.
    except: bool,
    /// Sorted, disjoint, none empty, none touching.
    ranges: Vec<Range<u64>>,
}

impl Selection {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether data row `row` is selected.
    #[must_use]
    pub fn contains(&self, row: u64) -> bool {
        let inside = self
            .ranges
            .binary_search_by(|r| {
                if r.end <= row {
                    Ordering::Less
                } else if r.start > row {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            })
            .is_ok();
        inside != self.except
    }

    /// How many of `len` rows are selected.
    #[must_use]
    pub fn count(&self, len: u64) -> u64 {
        let listed: u64 = self
            .ranges
            .iter()
            .map(|r| r.end.min(len).saturating_sub(r.start.min(len)))
            .sum();
        if self.except {
            len.saturating_sub(listed)
        } else {
            listed
        }
    }

    /// Select every row (rows added later too, until they are spliced in unselected).
    pub fn select_all(&mut self) {
        self.except = true;
        self.ranges.clear();
    }

    pub fn clear(&mut self) {
        self.except = false;
        self.ranges.clear();
    }

    /// Select rows `range`.
    pub fn select(&mut self, range: Range<u64>) {
        if self.except {
            self.cut(&range);
        } else {
            self.add(range);
        }
    }

    /// Deselect rows `range`.
    pub fn deselect(&mut self, range: Range<u64>) {
        if self.except {
            self.add(range);
        } else {
            self.cut(&range);
        }
    }

    pub fn toggle(&mut self, row: u64) {
        let range = row..row.saturating_add(1);
        if self.contains(row) {
            self.deselect(range);
        } else {
            self.select(range);
        }
    }

    /// Data rows `at..at + removed` were replaced by `inserted` new ones,
    /// not selected; rows after them move.
    pub fn splice(&mut self, at: u64, removed: u64, inserted: u64) {
        let end = at.saturating_add(removed);
        self.cut(&(at..end));
        let shift = |v: u64| {
            if v >= end {
                v.saturating_sub(removed).saturating_add(inserted)
            } else {
                v
            }
        };
        let mut ranges: Vec<Range<u64>> = Vec::with_capacity(self.ranges.len().saturating_add(1));
        for r in std::mem::take(&mut self.ranges) {
            ranges.push(shift(r.start)..shift(r.end));
        }
        self.ranges = ranges;
        // Ranges on both sides of the cut may now touch: merge them.
        let joined = std::mem::take(&mut self.ranges);
        for r in joined {
            self.add(r);
        }
        if self.except && inserted > 0 {
            self.add(at..at.saturating_add(inserted));
        }
    }

    /// The listed ranges (the selected rows, or the rows left out of an "all").
    #[must_use]
    pub fn ranges(&self) -> (bool, &[Range<u64>]) {
        (self.except, &self.ranges)
    }

    fn add(&mut self, range: Range<u64>) {
        if range.is_empty() {
            return;
        }
        let mut start = range.start;
        let mut end = range.end;
        let mut out = Vec::with_capacity(self.ranges.len().saturating_add(1));
        let mut placed = false;
        for r in std::mem::take(&mut self.ranges) {
            if r.end < start {
                out.push(r);
            } else if r.start > end {
                if !placed {
                    out.push(start..end);
                    placed = true;
                }
                out.push(r);
            } else {
                start = start.min(r.start);
                end = end.max(r.end);
            }
        }
        if !placed {
            out.push(start..end);
        }
        self.ranges = out;
    }

    fn cut(&mut self, range: &Range<u64>) {
        if range.is_empty() {
            return;
        }
        let mut out = Vec::with_capacity(self.ranges.len().saturating_add(1));
        for r in std::mem::take(&mut self.ranges) {
            if r.end <= range.start || r.start >= range.end {
                out.push(r);
                continue;
            }
            if r.start < range.start {
                out.push(r.start..range.start);
            }
            if r.end > range.end {
                out.push(range.end..r.end);
            }
        }
        self.ranges = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::Collection;

    #[test]
    fn a_view_sorts_and_filters_by_data_row() {
        let c = Collection::new([5_u32, 3, 8, 1, 9, 2]);
        let odd = |v: &u32| v % 2 == 1;
        let by = |a: &u32, b: &u32| a.cmp(b);
        let v = View::build(&c.snapshot(), odd, Some(&by));
        let shown: Vec<u32> = (0..v.len()).filter_map(|p| v.row(p)).collect();
        // 1 (row 3), 3 (row 1), 5 (row 0), 9 (row 4).
        assert_eq!(shown, vec![3, 1, 0, 4]);
        c.dispose();
    }

    #[test]
    fn a_selection_survives_a_sort_and_moves_with_a_splice() {
        let c = Collection::new((0..100_u32).rev());
        let mut s = Selection::new();
        s.select(10..20);
        s.toggle(50);
        let by = |a: &u32, b: &u32| a.cmp(b);
        let sorted = View::build(&c.snapshot(), |_| true, Some(&by));
        // Data row 15 is shown at position 84 once sorted ascending; still selected.
        let p = (0..sorted.len()).find(|&p| sorted.row(p) == Some(15)).unwrap();
        assert_eq!(p, 84);
        assert!(s.contains(15) && s.contains(50) && !s.contains(20));
        assert_eq!(s.count(100), 11);
        // Five rows inserted at 0: the selection moves with its rows.
        s.splice(0, 0, 5);
        assert!(s.contains(15 + 5) && s.contains(55) && !s.contains(15 - 1));
        // Rows 18..25 removed: the selected ones among them go.
        s.splice(18, 7, 0);
        assert_eq!(s.count(1000), 4);
        c.dispose();
    }

    #[test]
    fn all_but_some_counts_from_the_length() {
        let mut s = Selection::new();
        s.select_all();
        s.deselect(5..10);
        assert_eq!(s.count(10_000_000), 9_999_995);
        assert!(s.contains(4) && !s.contains(7) && s.contains(10));
        s.splice(0, 0, 3);
        // The rows spliced in are not selected; the hole moved.
        assert!(!s.contains(0) && !s.contains(9) && s.contains(13));
        assert_eq!(s.count(10_000_003), 9_999_995);
    }
}
