//! Row heights of a virtual list (ADR-0014): a B+ tree ([`SumTree`]) of runs
//! of rows that share a height, heights in 1/64 px (Blink's `LayoutUnit`),
//! sums in `u64`. Offset to row, row to offset, a height set and a splice
//! are O(log n); a list of ten million rows none of which was measured is
//! one run.
//!
//! Unmeasured rows take an estimate ([`ExtentTree::estimate`]): the mean of
//! the rows measured, read from the tree's totals, so it is never stale (a
//! measured row laid out again at another height moves it) and moving it
//! costs nothing. Unmeasured runs hold no height of their own; the tree
//! sums the measured rows' extent and counts the unmeasured, and a seek
//! prices those at the estimate as it goes.

use crate::sum_tree::{Item, SumTree, Summary};

/// Units a pixel holds.
pub const UNITS: f32 = 64.0;

/// A height in pixels as units (rounded to the nearest 1/64 px).
#[must_use]
pub fn units(px: f32) -> u32 {
    // Saturating: a height past 2^26 px is the largest one.
    crate::convert::f32_to_u32((px.max(0.0) * UNITS).round())
}

/// A height in units as pixels.
#[must_use]
pub fn pixels(units: u64) -> f64 {
    // Exact below 2^53 units (a 1.4e14 px list).
    crate::convert::u64_to_f64(units) / f64::from(UNITS)
}

/// A run of rows of one height; an unmeasured run's `height` is 0 (they
/// take the estimate, whatever it is when read).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    rows: u64,
    height: u32,
    measured: bool,
}

impl Run {
    const fn unmeasured(rows: u64) -> Self {
        Self {
            rows,
            height: 0,
            measured: false,
        }
    }

    /// Each row's height, unmeasured ones at `estimate`.
    const fn height_at(&self, estimate: u32) -> u32 {
        if self.measured { self.height } else { estimate }
    }
}

/// Rows, the measured among them, and the measured rows' extent, in units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Sum {
    rows: u64,
    measured: u64,
    extent: u64,
}

impl Sum {
    /// The extent of these rows, the unmeasured at `estimate`.
    fn extent_at(&self, estimate: u32) -> u64 {
        self.rows
            .saturating_sub(self.measured)
            .saturating_mul(u64::from(estimate))
            .saturating_add(self.extent)
    }
}

impl Summary for Sum {
    fn add(&mut self, other: &Self) {
        self.rows = self.rows.saturating_add(other.rows);
        self.measured = self.measured.saturating_add(other.measured);
        self.extent = self.extent.saturating_add(other.extent);
    }
}

impl Item for Run {
    type Summary = Sum;
    fn summary(&self) -> Sum {
        if self.measured {
            Sum {
                rows: self.rows,
                measured: self.rows,
                extent: self.rows.saturating_mul(u64::from(self.height)),
            }
        } else {
            Sum {
                rows: self.rows,
                ..Sum::default()
            }
        }
    }
}

/// The heights of a list's rows.
#[derive(Clone, Debug)]
pub struct ExtentTree {
    runs: SumTree<Run>,
    /// The estimate while no row is measured.
    initial: u32,
}

impl Default for ExtentTree {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl ExtentTree {
    /// `rows` rows, none measured, each `estimate` units tall until one is.
    #[must_use]
    pub fn new(rows: u64, estimate: u32) -> Self {
        let mut runs = SumTree::new();
        if rows > 0 {
            runs.push(Run::unmeasured(rows));
        }
        Self {
            runs,
            initial: estimate,
        }
    }

    fn totals(&self) -> &Sum {
        &self.runs.totals().summary
    }

    /// How many rows.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.totals().rows
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The rows' total height, in units.
    #[must_use]
    pub fn extent(&self) -> u64 {
        self.totals().extent_at(self.estimate())
    }

    /// Runs held (what the tree costs: rows measured apart, not rows).
    #[must_use]
    pub fn runs(&self) -> usize {
        self.runs.len()
    }

    /// The height unmeasured rows take, in units: the mean of the rows
    /// measured (rounded to the nearest unit), the one it was made with
    /// while none is. O(1).
    #[must_use]
    pub fn estimate(&self) -> u32 {
        let t = self.totals();
        if t.measured == 0 {
            return self.initial;
        }
        let mean = t
            .extent
            .saturating_add(t.measured.div_euclid(2))
            .checked_div(t.measured)
            .unwrap_or(0);
        u32::try_from(mean).unwrap_or(u32::MAX)
    }

    /// The run holding `row`, and the rows and extent before it.
    fn find(&self, row: u64) -> Option<(usize, Sum, Run)> {
        if row >= self.len() {
            return None;
        }
        let (i, before) = self.runs.seek(|t| t.summary.rows <= row);
        Some((i, before.summary, *self.runs.get(i)?))
    }

    /// The top of `row`, in units from the list's top (the extent for `row == len`).
    #[must_use]
    pub fn offset_of(&self, row: u64) -> u64 {
        let estimate = self.estimate();
        match self.find(row) {
            Some((_, before, run)) => before.extent_at(estimate).saturating_add(
                row.saturating_sub(before.rows)
                    .saturating_mul(u64::from(run.height_at(estimate))),
            ),
            None => self.extent(),
        }
    }

    /// `row`'s height in units.
    #[must_use]
    pub fn height(&self, row: u64) -> Option<u32> {
        self.find(row).map(|(_, _, r)| r.height_at(self.estimate()))
    }

    /// Whether `row` was measured.
    #[must_use]
    pub fn is_measured(&self, row: u64) -> bool {
        self.find(row).is_some_and(|(_, _, r)| r.measured)
    }

    /// The row at `offset` units from the top and how far into it; the last
    /// row's end for an offset past the end, `(0, 0)` for an empty list.
    #[must_use]
    pub fn row_at(&self, offset: u64) -> (u64, u64) {
        let len = self.len();
        if len == 0 {
            return (0, 0);
        }
        if offset >= self.extent() {
            let last = len.saturating_sub(1);
            return (last, u64::from(self.height(last).unwrap_or(0)));
        }
        let estimate = self.estimate();
        let (i, before) = self.runs.seek(|t| t.summary.extent_at(estimate) <= offset);
        let Some(run) = self.runs.get(i) else {
            return (len.saturating_sub(1), 0);
        };
        let into = offset.saturating_sub(before.summary.extent_at(estimate));
        let h = u64::from(run.height_at(estimate).max(1));
        let k = into.checked_div(h).unwrap_or(0).min(run.rows.saturating_sub(1));
        (
            before.summary.rows.saturating_add(k),
            into.saturating_sub(k.saturating_mul(h)),
        )
    }

    /// Set `row`'s measured height; whether it changed.
    pub fn set(&mut self, row: u64, height: u32) -> bool {
        let Some((i, before, run)) = self.find(row) else {
            return false;
        };
        if run.height == height && run.measured {
            return false;
        }
        let at = row.saturating_sub(before.rows);
        let mut parts = Vec::with_capacity(3);
        if at > 0 {
            parts.push(Run { rows: at, ..run });
        }
        parts.push(Run {
            rows: 1,
            height,
            measured: true,
        });
        let after = run.rows.saturating_sub(at).saturating_sub(1);
        if after > 0 {
            parts.push(Run { rows: after, ..run });
        }
        self.runs.splice(i, 1, parts);
        let at = if at > 0 { i.saturating_add(1) } else { i };
        self.merge_around(at);
        true
    }

    /// Merge the run at `i` with neighbours of the same height and kind.
    fn merge_around(&mut self, i: usize) {
        let mut i = i;
        if let Some(prev) = i.checked_sub(1)
            && let (Some(a), Some(b)) = (self.runs.get(prev).copied(), self.runs.get(i).copied())
            && a.height == b.height
            && a.measured == b.measured
        {
            self.runs.splice(
                prev,
                2,
                [Run {
                    rows: a.rows.saturating_add(b.rows),
                    ..a
                }],
            );
            i = prev;
        }
        if let (Some(a), Some(b)) = (self.runs.get(i).copied(), self.runs.get(i.saturating_add(1)).copied())
            && a.height == b.height
            && a.measured == b.measured
        {
            self.runs.splice(
                i,
                2,
                [Run {
                    rows: a.rows.saturating_add(b.rows),
                    ..a
                }],
            );
        }
    }

    /// Replace rows `at..at + removed` with `inserted` unmeasured ones (the
    /// data changed there).
    pub fn splice(&mut self, at: u64, removed: u64, inserted: u64) {
        let len = self.len();
        let at = at.min(len);
        let removed = removed.min(len.saturating_sub(at));
        // Cut the runs at `at` and at `at + removed`, drop what lies between.
        let first = self.cut(at);
        let last = self.cut(at.saturating_add(removed));
        let new = (inserted > 0).then(|| Run::unmeasured(inserted));
        self.runs.splice(first, last.saturating_sub(first), new);
        self.merge_around(first);
    }

    /// Split the run holding `row` so a run starts there; the index of that run.
    fn cut(&mut self, row: u64) -> usize {
        match self.find(row) {
            None => self.runs.len(),
            Some((i, before, run)) => {
                let at = row.saturating_sub(before.rows);
                if at == 0 {
                    return i;
                }
                self.runs.splice(
                    i,
                    1,
                    [
                        Run { rows: at, ..run },
                        Run {
                            rows: run.rows.saturating_sub(at),
                            ..run
                        },
                    ],
                );
                i.saturating_add(1)
            }
        }
    }

    /// Every row unmeasured again, at the estimate they have now (the width
    /// they were measured at changed).
    pub fn reset(&mut self) {
        *self = Self::new(self.len(), self.estimate());
    }

    /// The rows were reordered or filtered: there are `rows` now, and the
    /// row at each old position `p` is at `to(p)` (`None`, or a position
    /// past `rows`: left out). Measured heights follow their rows; the rows
    /// no old one went to are unmeasured. O(m log m + runs) for `m` rows
    /// measured, whatever the rows.
    pub fn reorder(&mut self, rows: u64, to: impl Fn(u64) -> Option<u64>) {
        let estimate = self.estimate();
        let mut moved: Vec<(u64, u32)> = Vec::new();
        let mut at = 0_u64;
        for run in self.runs.iter() {
            let end = at.saturating_add(run.rows);
            if run.measured {
                moved.extend((at..end).filter_map(|p| to(p).filter(|&q| q < rows).map(|q| (q, run.height))));
            }
            at = end;
        }
        moved.sort_unstable_by_key(|&(q, _)| q);
        moved.dedup_by_key(|&mut (q, _)| q);
        let mut built: Vec<Run> = Vec::new();
        let mut push = |run: Run| {
            if run.rows == 0 {
                return;
            }
            if let Some(last) = built.last_mut()
                && last.height == run.height
                && last.measured == run.measured
            {
                last.rows = last.rows.saturating_add(run.rows);
                return;
            }
            built.push(run);
        };
        let mut next = 0_u64;
        for (q, height) in moved {
            push(Run::unmeasured(q.saturating_sub(next)));
            push(Run {
                rows: 1,
                height,
                measured: true,
            });
            next = q.saturating_add(1);
        }
        push(Run::unmeasured(rows.saturating_sub(next)));
        self.runs = built.into_iter().collect();
        // None measured is left: the rows keep the estimate they had.
        self.initial = estimate;
    }

    /// Whether the tree keeps its invariants: no empty run, no two adjacent
    /// runs alike, no height on an unmeasured run, the sum tree's own.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let runs: Vec<Run> = self.runs.iter().copied().collect();
        runs.iter().all(|r| r.rows > 0 && (r.measured || r.height == 0))
            && runs.windows(2).all(|w| match w {
                [a, b] => !(a.height == b.height && a.measured == b.measured),
                _ => true,
            })
            && self.runs.is_valid()
    }
}

#[cfg(test)]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    reason = "tests: seeds, counts and offsets are small"
)]
mod tests {
    use super::*;

    /// The tree against a plain vector of heights (`None`: unmeasured).
    struct Oracle {
        heights: Vec<Option<u32>>,
        initial: u32,
    }

    impl Oracle {
        fn estimate(&self) -> u32 {
            let measured: Vec<u64> = self.heights.iter().flatten().map(|&h| u64::from(h)).collect();
            if measured.is_empty() {
                return self.initial;
            }
            let n = measured.len() as u64;
            u32::try_from((measured.iter().sum::<u64>() + n / 2) / n).unwrap()
        }

        fn height(&self, row: usize) -> u32 {
            self.heights[row].unwrap_or_else(|| self.estimate())
        }

        fn offset_of(&self, row: usize) -> u64 {
            (0..row).map(|r| u64::from(self.height(r))).sum()
        }
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    fn agree(t: &ExtentTree, o: &Oracle, r: &mut Rng) {
        assert!(t.is_valid());
        assert_eq!(t.len(), o.heights.len() as u64);
        assert_eq!(t.estimate(), o.estimate());
        let total = o.offset_of(o.heights.len());
        assert_eq!(t.extent(), total);
        for _ in 0..20 {
            let row = r.below(o.heights.len() as u64 + 1);
            assert_eq!(t.offset_of(row), o.offset_of(usize::try_from(row).unwrap()));
            if let Some(&h) = o.heights.get(usize::try_from(row).unwrap()) {
                assert_eq!(t.height(row), Some(h.unwrap_or_else(|| o.estimate())));
                assert_eq!(t.is_measured(row), h.is_some());
            }
            let off = r.below(total + 10);
            let (row, into) = t.row_at(off);
            if off < total {
                let top = o.offset_of(usize::try_from(row).unwrap());
                let h = u64::from(o.height(usize::try_from(row).unwrap()));
                assert!(
                    top <= off && (off < top + h || h == 0),
                    "offset {off}: row {row} at {top}+{h}"
                );
                assert_eq!(into, off - top);
            }
        }
    }

    #[test]
    fn edits_agree_with_a_vector_of_heights() {
        for seed in 0..40 {
            let mut r = Rng(seed);
            let n = r.below(300);
            let est = 1 + u32::try_from(r.below(5)).unwrap() * 64;
            let mut t = ExtentTree::new(n, est);
            let mut o = Oracle {
                heights: vec![None; usize::try_from(n).unwrap()],
                initial: est,
            };
            for _ in 0..200 {
                match r.below(6) {
                    0..=2 if !o.heights.is_empty() => {
                        let row = r.below(o.heights.len() as u64);
                        let h = u32::try_from(r.below(4)).unwrap() * 64 + 32;
                        t.set(row, h);
                        o.heights[usize::try_from(row).unwrap()] = Some(h);
                    }
                    3 => {
                        let len = o.heights.len() as u64;
                        let at = r.below(len + 1);
                        let removed = r.below(len - at + 1).min(5);
                        let inserted = r.below(6);
                        t.splice(at, removed, inserted);
                        let at = usize::try_from(at).unwrap();
                        o.heights.splice(
                            at..at + usize::try_from(removed).unwrap(),
                            std::iter::repeat_n(None, usize::try_from(inserted).unwrap()),
                        );
                    }
                    4 => {
                        // A sort and a filter: a seeded shuffle of what is
                        // kept, some rows dropped, some new ones added.
                        let len = o.heights.len();
                        let mut order: Vec<usize> = (0..len).filter(|_| r.below(8) != 0).collect();
                        for i in (1..order.len()).rev() {
                            order.swap(i, usize::try_from(r.below(i as u64 + 1)).unwrap());
                        }
                        let added = usize::try_from(r.below(4)).unwrap();
                        let mut to = vec![None; len];
                        for (q, &p) in order.iter().enumerate() {
                            to[p] = Some(q as u64);
                        }
                        let rows = order.len() + added;
                        t.reorder(rows as u64, |p| to[usize::try_from(p).unwrap()]);
                        o.initial = o.estimate();
                        let mut heights: Vec<Option<u32>> = order.iter().map(|&p| o.heights[p]).collect();
                        heights.resize(rows, None);
                        o.heights = heights;
                    }
                    _ => {
                        t.reset();
                        o.initial = o.estimate();
                        o.heights.iter_mut().for_each(|h| *h = None);
                    }
                }
                agree(&t, &o, &mut r);
            }
        }
    }

    #[test]
    fn ten_million_unmeasured_rows_are_one_run_and_sum_past_f32() {
        let mut t = ExtentTree::new(10_000_000, units(32.0));
        assert_eq!(t.runs(), 1);
        assert_eq!(pixels(t.extent()), 320_000_000.0);
        // Rows past 2^24 px are found exactly.
        let row = 9_999_999;
        let top = t.offset_of(row);
        assert_eq!(t.row_at(top), (row, 0));
        assert_eq!(t.row_at(top + 5), (row, 5));
        // A measured window in the middle costs its rows, not the list.
        for k in 5_000_000..5_000_040 {
            t.set(k, units(20.0 + f32::from(u16::try_from(k % 3).unwrap())));
        }
        assert!(t.runs() <= 42);
        assert_eq!(t.len(), 10_000_000);
    }

    #[test]
    fn rows_measured_alike_merge_into_one_run() {
        let mut t = ExtentTree::new(100, units(10.0));
        for k in 0..100 {
            t.set(k, units(12.0));
        }
        assert_eq!(t.runs(), 1);
        assert_eq!(t.extent(), 100 * u64::from(units(12.0)));
    }

    /// Frontiers 7.3 item 1: a row measured on the scroll path, moving the
    /// estimate, looks at O(log n) runs, allocating or not (the allocation
    /// count is `tests/extent_alloc.rs`).
    #[test]
    fn a_row_measured_that_moves_the_estimate_visits_no_pass_over_the_runs() {
        let mut t = ExtentTree::new(1_000_000, units(30.0));
        for k in 0..10_000_u64 {
            t.set(k * 2, units(if k % 2 == 0 { 20.0 } else { 21.0 }));
        }
        assert!(t.runs() >= 20_000);
        let estimate = t.estimate();
        let before = crate::sum_tree::visits();
        t.set(500_001, units(400.0));
        let top = t.offset_of(700_000);
        let _ = (t.row_at(top), t.height(999_999), t.extent());
        let seen = crate::sum_tree::visits() - before;
        assert_ne!(t.estimate(), estimate, "the row moved the estimate");
        assert!(
            seen < 2_000,
            "{seen} entries looked at for one row of {} runs",
            t.runs()
        );
    }

    /// Frontiers 7.3 (Codex): the mean counted a row once, at its first
    /// height; a row measured again at another moves it.
    #[test]
    fn the_estimate_is_the_mean_of_the_heights_measured_now() {
        let mut t = ExtentTree::new(100, units(10.0));
        t.set(0, units(20.0));
        t.set(1, units(40.0));
        assert_eq!(t.estimate(), units(30.0));
        t.set(1, units(60.0));
        assert_eq!(t.estimate(), units(40.0));
        assert_eq!(t.height(99), Some(units(40.0)));
        assert_eq!(t.extent(), u64::from(units(20.0 + 60.0 + 98.0 * 40.0)));
    }

    /// Frontiers 7.3 item 5: a sort keeps the heights with their rows, not
    /// their positions.
    #[test]
    fn heights_follow_their_rows_through_a_reorder() {
        let mut t = ExtentTree::new(1_000_000, units(10.0));
        t.set(0, units(100.0));
        t.set(1, units(20.0));
        // Reversed, the last row dropped.
        let n = 1_000_000_u64;
        t.reorder(n - 1, |p| (p + 1 < n).then(|| n - 2 - p));
        assert_eq!(t.len(), n - 1);
        assert_eq!(t.height(n - 2), Some(units(100.0)));
        assert_eq!(t.height(n - 3), Some(units(20.0)));
        assert!(!t.is_measured(0));
        assert_eq!(t.runs(), 3);
    }
}
