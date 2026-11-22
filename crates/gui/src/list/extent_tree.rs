//! Row heights of a virtual list (ADR-0014): a B+ tree ([`SumTree`]) of runs
//! of rows that share a height, heights in 1/64 px (Blink's `LayoutUnit`),
//! sums in `u64`. Offset to row, row to offset, a height set and a splice
//! are O(log n); a list of ten million rows none of which was measured is
//! one run.
//!
//! Unmeasured rows take an estimate ([`ExtentTree::set_estimate`]), kept
//! apart from the rows measured, so a better estimate moves only them.

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

/// A run of rows of one height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    rows: u64,
    height: u32,
    measured: bool,
}

/// Rows and their extent, in units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Sum {
    rows: u64,
    extent: u64,
}

impl Summary for Sum {
    fn add(&mut self, other: &Self) {
        self.rows = self.rows.saturating_add(other.rows);
        self.extent = self.extent.saturating_add(other.extent);
    }
}

impl Item for Run {
    type Summary = Sum;
    fn summary(&self) -> Sum {
        Sum {
            rows: self.rows,
            extent: self.rows.saturating_mul(u64::from(self.height)),
        }
    }
}

/// The heights of a list's rows.
#[derive(Clone, Debug)]
pub struct ExtentTree {
    runs: SumTree<Run>,
    estimate: u32,
}

impl Default for ExtentTree {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl ExtentTree {
    /// `rows` rows, none measured, each `estimate` units tall.
    #[must_use]
    pub fn new(rows: u64, estimate: u32) -> Self {
        let mut runs = SumTree::new();
        if rows > 0 {
            runs.push(Run {
                rows,
                height: estimate,
                measured: false,
            });
        }
        Self { runs, estimate }
    }

    /// How many rows.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.runs.totals().summary.rows
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The rows' total height, in units.
    #[must_use]
    pub fn extent(&self) -> u64 {
        self.runs.totals().summary.extent
    }

    /// Runs held (what the tree costs: rows measured apart, not rows).
    #[must_use]
    pub fn runs(&self) -> usize {
        self.runs.len()
    }

    /// The estimate unmeasured rows take, in units.
    #[must_use]
    pub fn estimate(&self) -> u32 {
        self.estimate
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
        match self.find(row) {
            Some((_, before, run)) => before
                .extent
                .saturating_add(row.saturating_sub(before.rows).saturating_mul(u64::from(run.height))),
            None => self.extent(),
        }
    }

    /// `row`'s height in units.
    #[must_use]
    pub fn height(&self, row: u64) -> Option<u32> {
        self.find(row).map(|(_, _, r)| r.height)
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
        let (i, before) = self.runs.seek(|t| t.summary.extent <= offset);
        let Some(run) = self.runs.get(i) else {
            return (len.saturating_sub(1), 0);
        };
        let into = offset.saturating_sub(before.summary.extent);
        let h = u64::from(run.height.max(1));
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
        let mut new = Vec::new();
        if inserted > 0 {
            new.push(Run {
                rows: inserted,
                height: self.estimate,
                measured: false,
            });
        }
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

    /// Every row unmeasured again at `estimate` (the width they were
    /// measured at changed).
    pub fn reset(&mut self, estimate: u32) {
        *self = Self::new(self.len(), estimate);
    }

    /// The height unmeasured rows take; whether any moved.
    pub fn set_estimate(&mut self, estimate: u32) -> bool {
        if estimate == self.estimate {
            return false;
        }
        self.estimate = estimate;
        let runs: Vec<Run> = self
            .runs
            .iter()
            .map(|r| if r.measured { *r } else { Run { height: estimate, ..*r } })
            .collect();
        let changed = runs.iter().any(|r| !r.measured);
        self.runs = SumTree::new();
        let mut merged: Vec<Run> = Vec::with_capacity(runs.len());
        for r in runs {
            match merged.last_mut() {
                Some(l) if l.height == r.height && l.measured == r.measured => l.rows = l.rows.saturating_add(r.rows),
                _ => merged.push(r),
            }
        }
        self.runs = merged.into_iter().collect();
        changed
    }

    /// Whether the tree keeps its invariants: no empty run, no two adjacent
    /// runs alike, the sum tree's own.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let runs: Vec<Run> = self.runs.iter().copied().collect();
        runs.iter().all(|r| r.rows > 0)
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

    /// The tree against a plain vector of heights.
    struct Oracle {
        heights: Vec<(u32, bool)>,
        estimate: u32,
    }

    impl Oracle {
        fn offset_of(&self, row: usize) -> u64 {
            self.heights.iter().take(row).map(|h| u64::from(h.0)).sum()
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
        let total = o.offset_of(o.heights.len());
        assert_eq!(t.extent(), total);
        for _ in 0..20 {
            let row = r.below(o.heights.len() as u64 + 1);
            assert_eq!(t.offset_of(row), o.offset_of(usize::try_from(row).unwrap()));
            if let Some(&(h, m)) = o.heights.get(usize::try_from(row).unwrap()) {
                assert_eq!(t.height(row), Some(h));
                assert_eq!(t.is_measured(row), m);
            }
            let off = r.below(total + 10);
            let (row, into) = t.row_at(off);
            if off < total {
                let top = o.offset_of(usize::try_from(row).unwrap());
                let h = u64::from(o.heights[usize::try_from(row).unwrap()].0);
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
                heights: vec![(est, false); usize::try_from(n).unwrap()],
                estimate: est,
            };
            for _ in 0..200 {
                match r.below(4) {
                    0 | 1 if !o.heights.is_empty() => {
                        let row = r.below(o.heights.len() as u64);
                        let h = u32::try_from(r.below(4)).unwrap() * 64 + 32;
                        t.set(row, h);
                        o.heights[usize::try_from(row).unwrap()] = (h, true);
                    }
                    2 => {
                        let len = o.heights.len() as u64;
                        let at = r.below(len + 1);
                        let removed = r.below(len - at + 1).min(5);
                        let inserted = r.below(6);
                        t.splice(at, removed, inserted);
                        let at = usize::try_from(at).unwrap();
                        o.heights.splice(
                            at..at + usize::try_from(removed).unwrap(),
                            std::iter::repeat_n((o.estimate, false), usize::try_from(inserted).unwrap()),
                        );
                    }
                    _ => {
                        let est = 1 + u32::try_from(r.below(5)).unwrap() * 64;
                        t.set_estimate(est);
                        o.estimate = est;
                        for h in &mut o.heights {
                            if !h.1 {
                                h.0 = est;
                            }
                        }
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
}
