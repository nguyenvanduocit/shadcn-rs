//! What a list shows of its data, and what is selected in it (ADR-0014).
//!
//! A [`View`] is a permutation: the data rows a sort and a filter keep, in
//! their order. It is built off the UI thread from a [`Snapshot`] (shared,
//! not copied) by [`Views`], which delivers only the view the latest
//! request asked for, so sorting a million rows never holds a frame. A
//! [`Selection`] is kept in data rows, as ranges, with an "all except"
//! mode, so it survives a sort or a filter unchanged and moves with the
//! data when rows are spliced.

use std::cell::Cell;
use std::cmp::Ordering;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use crate::mailbox::{Sender, mailbox};
use crate::reactive::Snapshot;

/// How two rows of data compare, for a sort.
pub type Order<T> = dyn Fn(&T, &T) -> Ordering;

/// Where no position is: a data row the view leaves out.
const LEFT_OUT: u32 = u32::MAX;

/// The data rows a list shows, in the order it shows them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    rows: Arc<Vec<u32>>,
    /// Each data row's position in `rows` ([`LEFT_OUT`]: filtered out),
    /// built with it off the UI thread: what a list's heights follow
    /// across a sort ([`crate::list::ListState::reorder`]).
    positions: Arc<Vec<u32>>,
    /// The data's version it was built from.
    version: u64,
}

impl View {
    /// Every row of `len`, in data order.
    #[must_use]
    pub fn identity(len: u32) -> Self {
        let rows = Arc::new((0..len).collect());
        Self {
            positions: Arc::clone(&rows),
            rows,
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
        let rows: Vec<u32> = kept.into_iter().map(|(i, _)| i).collect();
        let mut positions = vec![LEFT_OUT; items.len()];
        for (row, at) in rows.iter().zip(0_u32..) {
            if let Some(p) = usize::try_from(*row).ok().and_then(|r| positions.get_mut(r)) {
                *p = at;
            }
        }
        Self {
            rows: Arc::new(rows),
            positions: Arc::new(positions),
            version: items.version(),
        }
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

    /// Where data row `row` is shown; `None` when the view leaves it out.
    #[must_use]
    pub fn position(&self, row: u32) -> Option<usize> {
        let at = *self.positions.get(usize::try_from(row).ok()?)?;
        (at != LEFT_OUT).then(|| usize::try_from(at).ok()).flatten()
    }

    /// Where the row this view shows at `position` is shown in `to` (the
    /// same data row); `None` when `to` leaves it out: what a list given
    /// `to` in place of this view passes [`crate::list::ListState::reorder`].
    #[must_use]
    pub fn moved_to(&self, to: &Self, position: u64) -> Option<u64> {
        let row = self.row(usize::try_from(position).ok()?)?;
        to.position(row).and_then(|p| u64::try_from(p).ok())
    }

    /// The data's version the view was built from.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }
}

/// Builds views off the UI thread and hands its handler only the one the
/// latest request asked for: each request supersedes the ones before it,
/// so two sorts of one snapshot that finish in the other order (the
/// threads race) never leave the older on screen. A snapshot's version
/// cannot tell them apart; the order they were asked in does. Clones share
/// it.
#[derive(Clone)]
pub struct Views {
    /// The latest request's number.
    asked: Rc<Cell<u64>>,
    to: Sender<(u64, View)>,
}

impl Views {
    /// Views handed to `on_view` on the UI thread (a handler that stores it
    /// in a signal the list reads), each the latest one asked for. Made in
    /// a scope, it closes with it ([`mailbox`]).
    pub fn new(mut on_view: impl FnMut(View) + 'static) -> Self {
        let asked = Rc::new(Cell::new(0));
        let latest = Rc::clone(&asked);
        let to = mailbox(move |(request, view): (u64, View)| {
            if request == latest.get() {
                on_view(view);
            }
        });
        Self { asked, to }
    }

    /// Build the view of `items` that `keep` and `order` give ([`View::build`])
    /// on a thread of its own, superseding every request before it.
    pub fn spawn<T, K, O>(&self, items: Snapshot<T>, keep: K, order: Option<O>)
    where
        T: Clone + Send + Sync + 'static,
        K: Fn(&T) -> bool + Send + 'static,
        O: Fn(&T, &T) -> Ordering + Send + 'static,
        Snapshot<T>: Send,
    {
        let request = self.asked.get().wrapping_add(1);
        self.asked.set(request);
        let to = self.to.clone();
        std::thread::spawn(move || {
            let view = View::build(&items, keep, order.as_ref().map(|o| -> &Order<T> { o }));
            to.send((request, view));
        });
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

    /// Frontiers 4.1: two sorts of one snapshot, the first asked finishing
    /// last. Only the latest request's view reaches the handler.
    #[test]
    fn of_two_sorts_of_one_snapshot_the_later_asked_wins_whichever_ends_first() {
        use std::cell::RefCell;
        use std::sync::atomic::{AtomicBool, Ordering as Atomic};

        let c = Collection::new([2_u32, 3, 1]);
        let shown = Rc::new(RefCell::new(Vec::new()));
        let into = Rc::clone(&shown);
        let views = Views::new(move |v: View| {
            into.borrow_mut()
                .push((0..v.len()).filter_map(|p| v.row(p)).collect::<Vec<_>>())
        });
        let gate = Arc::new(AtomicBool::new(false));
        let held = Arc::clone(&gate);
        // Ascending, asked first, held until the descending one arrived.
        views.spawn(
            c.snapshot(),
            move |_: &u32| {
                while !held.load(Atomic::SeqCst) {
                    std::thread::yield_now();
                }
                true
            },
            Some(|a: &u32, b: &u32| a.cmp(b)),
        );
        views.spawn(c.snapshot(), |_: &u32| true, Some(|a: &u32, b: &u32| b.cmp(a)));
        let arrive = || {
            for _ in 0..10_000 {
                if crate::mailbox::deliver() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            panic!("no view arrived");
        };
        arrive();
        gate.store(true, Atomic::SeqCst);
        arrive();
        // 3 (row 1), 2 (row 0), 1 (row 2): the descending view, alone.
        assert_eq!(*shown.borrow(), [vec![1, 0, 2]]);
        c.dispose();
    }

    #[test]
    fn a_view_knows_where_each_data_row_went() {
        let c = Collection::new([5_u32, 3, 8, 1]);
        let all = View::identity(4);
        let odd = View::build(&c.snapshot(), |v| v % 2 == 1, Some(&|a: &u32, b: &u32| a.cmp(b)));
        // Odd ascending: 1 (row 3), 3 (row 1), 5 (row 0); 8 (row 2) left out.
        assert_eq!(odd.position(3), Some(0));
        assert_eq!(odd.position(2), None);
        let moved: Vec<Option<u64>> = (0..4).map(|p| all.moved_to(&odd, p)).collect();
        assert_eq!(moved, [Some(2), Some(1), None, Some(0)]);
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
