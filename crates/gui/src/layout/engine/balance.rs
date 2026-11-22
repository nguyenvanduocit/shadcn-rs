//! `flex-wrap: balance`: Taffy 0.15.1's `balance` module (css-flexbox-2
//! §balancing), ported as it is. Items are divided into the number of lines
//! greedy breaking gives (at least `flex-line-count`), minimising the sum of
//! the squared line sizes, ties broken by giving the earliest lines the most
//! items; a dynamic program over (lines, first item of the suffix) with the
//! divide-and-conquer optimisation, in `f64` (finite `f32` sizes convert
//! exactly, so equal divisions compare equal).

/// Score of a division that breaks the line size limit.
const INFEASIBLE: f64 = f64::INFINITY;

/// An item size in `f64`, negative ones floored at zero, an infinite one
/// clamped to the largest finite `f32`.
fn to_size(value: f32) -> f64 {
    f64::from(value).clamp(0.0, f64::from(f32::MAX))
}

fn to_u32(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

fn to_usize(v: u32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// Line sizing shared by the scoring and readback phases.
struct LineSizes {
    /// Per item, the prefix sum of the item sizes through it (each item
    /// also adds one trailing gap), and whether its (floored) size is zero.
    sums: Vec<(f64, bool)>,
    gap_between_items: f64,
    /// The container's inner main size, which a line of more than one item
    /// may not exceed.
    limit: f64,
}

impl LineSizes {
    fn item_count(&self) -> usize {
        self.sums.len()
    }

    fn is_zero_item(&self, index: usize) -> bool {
        self.sums.get(index).is_some_and(|s| s.1)
    }

    fn sum(&self, index: usize) -> f64 {
        self.sums.get(index).map_or(0.0, |s| s.0)
    }

    /// The size of the line holding items `start..=end`.
    fn line_size(&self, start: usize, end: usize) -> f64 {
        let start_sum = match start.checked_sub(1) {
            Some(before) => self.sum(before),
            None => 0.0,
        };
        self.sum(end) - start_sum - self.gap_between_items
    }

    /// The squared size of the line `start..=end`, or [`INFEASIBLE`] for a
    /// line of more than one item over the limit.
    fn line_cost(&self, start: usize, end: usize) -> f64 {
        let size = self.line_size(start, end);
        if end > start && size > self.limit {
            return INFEASIBLE;
        }
        size * size
    }

    /// For every suffix, the number of lines greedy breaking gives it (the
    /// fewest it can be validly divided into); the empty suffix's is zero.
    fn suffix_greedy_line_counts(&self) -> Vec<u32> {
        let item_count = self.item_count();
        let mut counts = vec![0_u32; item_count.saturating_add(1)];
        let mut line_end = item_count;
        for start in (0..item_count).rev() {
            while line_end > start.saturating_add(1) && self.line_size(start, line_end.saturating_sub(1)) > self.limit {
                line_end = line_end.saturating_sub(1);
            }
            let next = counts.get(line_end).copied().unwrap_or(0);
            if let Some(c) = counts.get_mut(start) {
                *c = next.saturating_add(1);
            }
        }
        counts
    }

    /// For every `start`, the largest `end` such that `start..=end` fits
    /// (or `start` itself if even that one item overflows).
    fn fit_ends(&self) -> Vec<u32> {
        let item_count = self.item_count();
        let mut fit_ends = Vec::with_capacity(item_count);
        let mut fit_end = 0;
        for start in 0..item_count {
            if fit_end < start {
                fit_end = start;
            }
            while fit_end.saturating_add(1) < item_count
                && self.line_size(start, fit_end.saturating_add(1)) <= self.limit
            {
                fit_end = fit_end.saturating_add(1);
            }
            fit_ends.push(to_u32(fit_end));
        }
        fit_ends
    }
}

/// One row of the dynamic program: divisions of suffixes into exactly
/// `lines` lines.
struct Row<'a> {
    sizes: &'a LineSizes,
    fit_ends: &'a [u32],
    /// `prev[start]`: the least score of a division of `start..` into
    /// `lines - 1` lines.
    prev: &'a [f64],
    /// The line ends that precede a non-zero item, ascending, capped to
    /// `max_end`.
    nonzero_ends: &'a [u32],
    /// The largest end of the first line (the other lines need an item each).
    max_end: usize,
}

impl Row<'_> {
    fn prev(&self, i: usize) -> f64 {
        self.prev.get(i).copied().unwrap_or(INFEASIBLE)
    }

    fn nonzero_end(&self, col: usize) -> usize {
        self.nonzero_ends.get(col).copied().map_or(usize::MAX, to_usize)
    }
}

/// `cur[start]` and `opts[start]` for `start` in `start_lo..=start_hi`:
/// the least `line_cost(start, end) + prev[end + 1]` over valid ends, and
/// the largest end giving it.
fn fill_row(
    row: &Row<'_>,
    cur: &mut [f64],
    opts: &mut [u32],
    start_lo: usize,
    start_hi: usize,
    col_lo: usize,
    col_hi: usize,
) {
    if start_lo > start_hi {
        return;
    }
    let start = start_lo.saturating_add(start_hi.saturating_sub(start_lo) / 2);

    let first_col = col_lo.saturating_add(
        row.nonzero_ends
            .get(col_lo..col_hi)
            .map_or(0, |ends| ends.partition_point(|&end| to_usize(end) < start)),
    );
    let mut min_cost = INFEASIBLE;
    let mut min_col = None;
    for col in first_col..col_hi {
        let end = row.nonzero_end(col);
        let line_cost = row.sizes.line_cost(start, end);
        if line_cost == INFEASIBLE {
            break;
        }
        let cost = line_cost + row.prev(end.saturating_add(1));
        if cost <= min_cost {
            min_cost = cost;
            min_col = Some(col);
        }
    }

    let mut min_end = min_col.map(|col| row.nonzero_end(col));
    let zero_end = row.fit_ends.get(start).copied().map_or(0, to_usize).min(row.max_end);
    if row.sizes.is_zero_item(zero_end.saturating_add(1)) {
        let cost = row.sizes.line_cost(start, zero_end) + row.prev(zero_end.saturating_add(1));
        if cost < min_cost || (cost == min_cost && min_end.is_none_or(|end| end < zero_end)) {
            min_cost = cost;
            min_end = Some(zero_end);
        }
    }

    if let Some(c) = cur.get_mut(start) {
        *c = min_cost;
    }
    if let Some(o) = opts.get_mut(start) {
        *o = to_u32(min_end.unwrap_or(start));
    }

    if start > start_lo {
        let col_hi = min_col.map_or(col_hi, |col| col.saturating_add(1));
        fill_row(row, cur, opts, start_lo, start.saturating_sub(1), col_lo, col_hi);
    }
    if start < start_hi {
        let col_lo = min_col.unwrap_or(first_col);
        fill_row(row, cur, opts, start.saturating_add(1), start_hi, col_lo, col_hi);
    }
}

/// The number of items on each line that balances `item_sizes` across at
/// least `min_line_count` lines (or one per item if there are fewer),
/// each line at most `line_limit` unless it holds one item. Every count is
/// non-zero and they sum to the item count; an empty input gives none.
pub(super) fn balanced_line_item_counts(
    item_sizes: impl ExactSizeIterator<Item = f32>,
    line_limit: f32,
    gap_between_items: f32,
    min_line_count: usize,
) -> Vec<usize> {
    let item_count = item_sizes.len();
    if item_count == 0 {
        return Vec::new();
    }
    let gap_between_items = to_size(gap_between_items);
    let limit = f64::from(line_limit);

    let mut sums = Vec::with_capacity(item_count);
    let mut sum = 0.0;
    for size in item_sizes {
        let size = to_size(size);
        sum += size + gap_between_items;
        sums.push((sum, size == 0.0));
    }
    let sizes = LineSizes {
        sums,
        gap_between_items,
        limit,
    };

    let suffix_greedy = sizes.suffix_greedy_line_counts();
    let line_count = suffix_greedy
        .first()
        .copied()
        .map_or(1, to_usize)
        .max(min_line_count.clamp(1, item_count));

    let mut item_counts = Vec::with_capacity(line_count);
    if line_count == item_count {
        item_counts.resize(item_count, 1);
        return item_counts;
    }

    let fit_ends = sizes.fit_ends();
    let mut nonzero_ends = Vec::with_capacity(item_count.saturating_sub(1));
    for end in 0..item_count.saturating_sub(1) {
        if !sizes.is_zero_item(end.saturating_add(1)) {
            nonzero_ends.push(to_u32(end));
        }
    }

    let last = item_count.saturating_sub(1);
    let mut prev: Vec<f64> = (0..item_count).map(|start| sizes.line_cost(start, last)).collect();
    let mut cur = vec![INFEASIBLE; item_count];
    let mut opts = vec![0_u32; line_count.saturating_sub(1).saturating_mul(item_count)];
    for lines in 2..=line_count {
        let max_end = item_count.saturating_sub(lines);
        let mut first_start = 0;
        while first_start < max_end && suffix_greedy.get(first_start).copied().map_or(0, to_usize) > lines {
            if let Some(c) = cur.get_mut(first_start) {
                *c = INFEASIBLE;
            }
            first_start = first_start.saturating_add(1);
        }
        let col_hi = nonzero_ends.partition_point(|&end| to_usize(end) <= max_end);
        let row = Row {
            sizes: &sizes,
            fit_ends: &fit_ends,
            prev: &prev,
            nonzero_ends: &nonzero_ends,
            max_end,
        };
        let from = lines.saturating_sub(2).saturating_mul(item_count);
        let to = lines.saturating_sub(1).saturating_mul(item_count);
        if let Some(opts_row) = opts.get_mut(from..to) {
            fill_row(&row, &mut cur, opts_row, first_start, max_end, 0, col_hi);
        }
        std::mem::swap(&mut prev, &mut cur);
    }

    let mut start = 0_usize;
    for lines in (2..=line_count).rev() {
        let at = lines.saturating_sub(2).saturating_mul(item_count).saturating_add(start);
        let end = opts.get(at).copied().map_or(start, to_usize);
        item_counts.push(end.saturating_sub(start).saturating_add(1));
        start = end.saturating_add(1);
    }
    item_counts.push(item_count.saturating_sub(start));
    item_counts
}

#[cfg(test)]
mod tests {
    use super::balanced_line_item_counts;

    #[test]
    fn balances_equal_items_evenly() {
        // Five 10 px items in 30 px: greedy gives 3 + 2; balanced the same.
        let counts = balanced_line_item_counts([10.0_f32; 5].into_iter(), 30.0, 0.0, 1);
        assert_eq!(counts, vec![3, 2]);
        // Four 10 px items in 30 px: greedy 3 + 1, balanced 2 + 2.
        let counts = balanced_line_item_counts([10.0_f32; 4].into_iter(), 30.0, 0.0, 1);
        assert_eq!(counts, vec![2, 2]);
    }

    #[test]
    fn a_minimum_line_count_is_kept() {
        let counts = balanced_line_item_counts([10.0_f32; 6].into_iter(), f32::INFINITY, 0.0, 3);
        assert_eq!(counts, vec![2, 2, 2]);
    }
}
