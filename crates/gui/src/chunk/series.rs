//! A series of points kept for the GPU and the CPU alike (ADR-0018): x as
//! an `i64` (nanosecond timestamps included), y as an `f32` and optionally a
//! low part of its own, in pages of at most [`MAX_PAGE`] points, each with a
//! min/max pyramid of 64 children a node.
//!
//! The pyramid lets a pixel column's M4 (Jugel et al.: its first, last,
//! least and greatest point) be read in `O(64 · log₆₄ n)`, so what a frame
//! costs follows the plot's width, not the point count. The GPU keeps the
//! same pages and pyramid ([`super::store`]) and computes the same M4
//! (`m4.wgsl`); [`m4`] here is its reference, equal to it bit for bit: a
//! column's points are chosen by index and by an order key of y's bits
//! ([`key`]), never by arithmetic, and ties go to the earliest point.
//!
//! A y that is not finite (NaN, an infinity) is a gap: it is no point of
//! any column, and a line breaks across it. Appending writes the tail of the
//! last page and the pyramid nodes above it; a series that keeps only its
//! last points drops whole pages from its start (the ring buffer of
//! ADR-0018, at page granularity), so nothing is moved or renumbered.

use std::sync::{Arc, RwLock, RwLockReadGuard};

/// Children a pyramid node has.
pub const FANOUT: usize = 64;
/// The most points a page holds: its GPU buffer (x high and low words, y,
/// y's low part and the pyramid) stays under 128 MiB.
pub const MAX_PAGE: usize = 1 << 22;
/// The fewest points a page is made for.
pub const MIN_PAGE: usize = 1 << 10;
/// No point.
pub const NONE: u32 = u32::MAX;

/// y's bits as a key whose unsigned order is y's order (`-0.0` before
/// `0.0`), so a min or max compares integers, never floats, and the GPU
/// (which may flush subnormals in float compares) picks the same point.
#[must_use]
pub fn key(y: f32) -> u32 {
    let b = y.to_bits();
    if b & 0x8000_0000 == 0 { b | 0x8000_0000 } else { !b }
}

/// The y whose [`key`] `k` is.
#[must_use]
pub fn unkey(k: u32) -> f32 {
    f32::from_bits(if k & 0x8000_0000 == 0 { !k } else { k & 0x7fff_ffff })
}

/// A y is a point, not a gap.
#[must_use]
pub fn valid(y: f32) -> bool {
    y.is_finite()
}

/// What a range of points holds: its least and greatest valid y (as keys)
/// and where they are (the earliest of equals), and its first and last
/// valid point. Indices are the page's own in a pyramid node, the
/// snapshot's in a [`Summary`] of a range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    pub min: u32,
    pub max: u32,
    pub imin: u32,
    pub imax: u32,
    pub first: u32,
    pub last: u32,
}

impl Node {
    /// Nothing valid.
    pub const EMPTY: Self = Self {
        min: u32::MAX,
        max: 0,
        imin: NONE,
        imax: NONE,
        first: NONE,
        last: NONE,
    };

    /// Point `i` with value `y`.
    #[must_use]
    pub fn leaf(i: u32, y: f32) -> Self {
        if !valid(y) {
            return Self::EMPTY;
        }
        let k = key(y);
        Self {
            min: k,
            max: k,
            imin: i,
            imax: i,
            first: i,
            last: i,
        }
    }

    /// `self` followed by `next`: ties keep `self`'s, the earlier, point.
    #[must_use]
    pub fn then(self, next: Self) -> Self {
        let (min, imin) = if next.min < self.min {
            (next.min, next.imin)
        } else {
            (self.min, self.imin)
        };
        let (max, imax) = if next.max > self.max {
            (next.max, next.imax)
        } else {
            (self.max, self.imax)
        };
        Self {
            min,
            max,
            imin,
            imax,
            first: if self.first == NONE { next.first } else { self.first },
            last: if next.last == NONE { self.last } else { next.last },
        }
    }

    /// Its indices moved by `by` (a page's into a snapshot's).
    #[must_use]
    pub fn shifted(self, by: u32) -> Self {
        let s = |i: u32| if i == NONE { NONE } else { i.saturating_add(by) };
        Self {
            imin: s(self.imin),
            imax: s(self.imax),
            first: s(self.first),
            last: s(self.last),
            ..self
        }
    }

    #[must_use]
    pub fn words(&self) -> [u32; 6] {
        [self.min, self.max, self.imin, self.imax, self.first, self.last]
    }
}

/// One page of points and its pyramid.
#[derive(Debug, Default)]
pub struct PageData {
    pub x: Vec<i64>,
    pub y: Vec<f32>,
    /// y's low part (`y_true - y`), for a series made precise.
    pub lo: Option<Vec<f32>>,
    /// Level 1 up: node `j` of level `k` covers points `j·64ᵏ .. (j+1)·64ᵏ`.
    pub levels: Vec<Vec<Node>>,
}

impl PageData {
    fn len(&self) -> usize {
        self.y.len()
    }

    /// Node `j` of level `k` (0: the points).
    fn node(&self, level: usize, j: usize) -> Node {
        if level == 0 {
            return self
                .y
                .get(j)
                .map_or(Node::EMPTY, |y| Node::leaf(u32::try_from(j).unwrap_or(NONE), *y));
        }
        self.levels
            .get(level.saturating_sub(1))
            .and_then(|l| l.get(j))
            .copied()
            .unwrap_or(Node::EMPTY)
    }

    /// Bring the pyramid up to the points from `from` on.
    fn build(&mut self, from: usize) {
        let mut below = self.len();
        let mut start = from;
        let mut level = 0_usize;
        while below > 1 {
            let count = below.div_ceil(FANOUT);
            let first = start / FANOUT;
            let mut nodes = Vec::with_capacity(count.saturating_sub(first));
            for j in first..count {
                let lo = j.saturating_mul(FANOUT);
                let hi = lo.saturating_add(FANOUT).min(below);
                let mut n = Node::EMPTY;
                for i in lo..hi {
                    n = n.then(self.node(level, i));
                }
                nodes.push(n);
            }
            if self.levels.len() <= level {
                self.levels.push(Vec::new());
            }
            if let Some(l) = self.levels.get_mut(level) {
                l.truncate(first);
                l.extend(nodes);
            }
            below = count;
            start = first;
            level = level.saturating_add(1);
        }
        self.levels.truncate(level);
    }

    /// Points `lo..hi` (the page's) summed up, in order.
    #[must_use]
    pub fn fold(&self, lo: usize, hi: usize) -> Node {
        let (mut l, mut r) = (lo, hi.min(self.len()));
        let (mut left, mut right) = (Node::EMPTY, Node::EMPTY);
        let mut level = 0_usize;
        while l < r {
            while l < r && !l.is_multiple_of(FANOUT) {
                left = left.then(self.node(level, l));
                l = l.saturating_add(1);
            }
            while l < r && !r.is_multiple_of(FANOUT) {
                r = r.saturating_sub(1);
                right = self.node(level, r).then(right);
            }
            if l >= r {
                break;
            }
            l /= FANOUT;
            r /= FANOUT;
            level = level.saturating_add(1);
        }
        left.then(right)
    }

    /// The first point at or after `x`, from 0 to the page's length.
    #[must_use]
    pub fn lower_bound(&self, x: i64) -> usize {
        self.x.partition_point(|v| *v < x)
    }
}

/// A page and where it starts in the series (points ever appended before it).
///
/// Its data is the engine's alone (ADR-0035, D5): app code reads points by
/// copy, through [`SeriesSnapshot::get`], up to the snapshot's length. No
/// reference into a live page reaches app code, so none is held across a
/// check's switch, and the series' lock is never held across user code.
///
/// ```compile_fail,E0616
/// // Codex r19-2's reproducer: a check wrote a live point through a snapshot.
/// let owner = gui::Scope::new();
/// let plot = owner.run(gui::chunk::plot::series);
/// plot.push(5, 2.0);
/// let sampled = plot.snapshot_untracked();
/// let (page, _, _) = sampled.pages().next().unwrap();
/// page.data.write().unwrap().y[0] = 9.0;
/// ```
///
/// ```compile_fail,E0599
/// // Codex F1b r1-2's reproducer: a check opened with a live page lent.
/// let owner = gui::Scope::new();
/// let plot = owner.run(gui::chunk::plot::series);
/// plot.push(1, 1.0);
/// let sampled = plot.snapshot_untracked();
/// let (page, _, _) = sampled.pages().next().unwrap();
/// let _ = page.with(|data| gui::testing::check(|| data.y[0]));
/// ```
pub struct Page {
    /// Its identity, for the GPU's copy.
    pub seq: u64,
    pub start: u64,
    data: RwLock<PageData>,
}

/// By its identity alone: formatting takes no lock, so the caller's writer
/// never runs while the page is held (ADR-0035 D4).
impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Page")
            .field("seq", &self.seq)
            .field("start", &self.start)
            .finish_non_exhaustive()
    }
}

impl Page {
    /// Its data to read, held by the engine alone (a poisoned lock still
    /// holds whole points: every write leaves the page consistent before
    /// it can panic).
    pub(crate) fn read(&self) -> RwLockReadGuard<'_, PageData> {
        self.data.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// A series as one frame saw it: its pages and how many points of each.
/// Equal while the series did not change (its id and version).
#[derive(Clone, Debug)]
pub struct SeriesSnapshot(Arc<Snap>);

#[derive(Debug)]
struct Snap {
    id: u64,
    version: u64,
    /// Bumped when every point is replaced.
    generation: u64,
    pages: Vec<(Arc<Page>, usize)>,
    /// The first point's place in the series (pages dropped before it).
    start: u64,
    len: u64,
    precise: bool,
}

impl PartialEq for SeriesSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id && self.0.version == other.0.version
    }
}

impl SeriesSnapshot {
    /// Whether it holds the same points as `other` (each x, and y to the
    /// bit), whichever series and pages hold them: the cold check's
    /// compare ([`super::Source`]'s). Where its first point lies among all
    /// the series was ever given ([`Self::start`]) is the series' history,
    /// not what it draws (nothing drawn reads it): a series built again
    /// from the same points, from 0, holds the same content.
    pub(crate) fn same_points(&self, other: &Self) -> bool {
        self.same_points_counted(other, &mut 0)
    }

    /// [`Self::same_points`], counting the page locks it takes in `locks`:
    /// both sides walked page by page, contiguous runs compared as slices,
    /// one lock a page and no search.
    fn same_points_counted(&self, other: &Self, locks: &mut usize) -> bool {
        if (self.0.len, self.0.precise) != (other.0.len, other.0.precise) {
            return false;
        }
        let (mut a, mut b) = (Run::new(&self.0.pages), Run::new(&other.0.pages));
        loop {
            let (left_a, left_b) = (a.left(locks), b.left(locks));
            if left_a == 0 || left_b == 0 {
                return left_a == left_b;
            }
            let k = left_a.min(left_b);
            let (Some(ra), Some(rb)) = (a.take(k), b.take(k)) else {
                return false;
            };
            let same_bits = |p: &[f32], q: &[f32]| p.iter().zip(q).all(|(p, q)| p.to_bits() == q.to_bits());
            let same_lo = match (ra.2, rb.2) {
                (Some(la), Some(lb)) => same_bits(la, lb),
                (la, lb) => la.is_none() && lb.is_none(),
            };
            if ra.0 != rb.0 || !same_bits(ra.1, rb.1) || !same_lo {
                return false;
            }
        }
    }
}

/// A snapshot's points walked page by page: the page it is on, locked
/// once, and how far along it.
struct Run<'a> {
    pages: std::slice::Iter<'a, (Arc<Page>, usize)>,
    held: Option<RwLockReadGuard<'a, PageData>>,
    /// The page's points the snapshot holds, and how many are walked.
    n: usize,
    at: usize,
}

/// A run of points: x's, y's and y's low parts.
type Slices<'a> = (&'a [i64], &'a [f32], Option<&'a [f32]>);

impl<'a> Run<'a> {
    fn new(pages: &'a [(Arc<Page>, usize)]) -> Self {
        Self {
            pages: pages.iter(),
            held: None,
            n: 0,
            at: 0,
        }
    }

    /// How many points are left on the page held, the next one locked
    /// (and counted in `locks`) when it is spent; 0 past the last.
    fn left(&mut self, locks: &mut usize) -> usize {
        while self.at >= self.n {
            let Some((page, n)) = self.pages.next() else {
                self.held = None;
                return 0;
            };
            *locks = locks.saturating_add(1);
            let data = page.read();
            self.n = (*n).min(data.len());
            self.held = Some(data);
            self.at = 0;
        }
        self.n.saturating_sub(self.at)
    }

    /// The next `k` points of the page held.
    fn take(&mut self, k: usize) -> Option<Slices<'_>> {
        let range = self.at..self.at.checked_add(k)?;
        self.at = range.end;
        let data = self.held.as_ref()?;
        let lo = match &data.lo {
            Some(lo) => Some(lo.get(range.clone())?),
            None => None,
        };
        Some((data.x.get(range.clone())?, data.y.get(range)?, lo))
    }
}

/// What a range of a snapshot holds (indices the snapshot's).
pub type Summary = Node;

/// The four points M4 keeps of a pixel column, in index order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Column {
    pub count: u32,
    /// Snapshot indices.
    pub idx: [u32; 4],
    /// Their y's bits.
    pub y: [u32; 4],
}

/// The columns M4 reduces a view to: each column `c` holds the points with
/// x in `bounds[c] .. bounds[c + 1]`; `before` and `after` are the points
/// just outside them, which a line runs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Columns {
    pub bounds: Vec<i64>,
    pub before: Option<u32>,
    pub after: Option<u32>,
}

impl SeriesSnapshot {
    pub(crate) fn new(state: &SeriesState) -> Self {
        Self(Arc::new(Snap {
            id: state.id,
            version: state.version,
            generation: state.generation,
            pages: state.pages.iter().map(|p| (Arc::clone(p), p.read().len())).collect(),
            start: state.start,
            len: state.len(),
            precise: state.precise,
        }))
    }

    #[must_use]
    pub fn id(&self) -> u64 {
        self.0.id
    }

    #[must_use]
    pub fn version(&self) -> u64 {
        self.0.version
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.0.generation
    }

    /// Points it holds (at most `u32::MAX`: indices are 32-bit on the GPU).
    #[must_use]
    pub fn len(&self) -> u32 {
        u32::try_from(self.0.len).unwrap_or(u32::MAX)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.len == 0
    }

    /// Its first point's place among every point the series was given.
    #[must_use]
    pub fn start(&self) -> u64 {
        self.0.start
    }

    #[must_use]
    pub fn precise(&self) -> bool {
        self.0.precise
    }

    /// Its pages with how many of their points it holds, and where each
    /// starts in it.
    pub fn pages(&self) -> impl Iterator<Item = (&Arc<Page>, usize, u32)> + '_ {
        self.0.pages.iter().map(move |(p, n)| {
            let at = u32::try_from(p.start.saturating_sub(self.0.start)).unwrap_or(NONE);
            (p, *n, at)
        })
    }

    /// The page holding point `i`, and `i` in it.
    fn locate(&self, i: u32) -> Option<(&Arc<Page>, usize, usize)> {
        let pages = &self.0.pages;
        let k = pages
            .partition_point(|(p, _)| p.start.saturating_sub(self.0.start) <= u64::from(i))
            .checked_sub(1)?;
        let (p, n) = pages.get(k)?;
        let local = usize::try_from(u64::from(i).checked_sub(p.start.saturating_sub(self.0.start))?).ok()?;
        (local < *n).then_some((p, local, *n))
    }

    /// Point `i`: its x and y (y's low part added, for a precise series).
    #[must_use]
    pub fn get(&self, i: u32) -> Option<(i64, f64)> {
        let (p, local, _) = self.locate(i)?;
        let d = p.read();
        let x = *d.x.get(local)?;
        let y = f64::from(*d.y.get(local)?) + d.lo.as_ref().and_then(|l| l.get(local)).map_or(0.0, |v| f64::from(*v));
        Some((x, y))
    }

    /// The first point at or after `x`.
    #[must_use]
    pub fn lower_bound(&self, x: i64) -> u32 {
        for (p, n, at) in self.pages() {
            let d = p.read();
            let last = d.x.get(n.saturating_sub(1)).copied();
            if n > 0 && last.is_some_and(|l| l >= x) {
                let local = d.x.get(..n).map_or(0, |xs| xs.partition_point(|v| *v < x));
                return at.saturating_add(u32::try_from(local).unwrap_or(NONE));
            }
        }
        self.len()
    }

    /// Points `lo..hi` summed up.
    #[must_use]
    pub fn fold(&self, lo: u32, hi: u32) -> Summary {
        let mut acc = Node::EMPTY;
        for (p, n, at) in self.pages() {
            let end = at.saturating_add(u32::try_from(n).unwrap_or(NONE));
            if end <= lo || at >= hi {
                continue;
            }
            let a = usize::try_from(lo.saturating_sub(at)).unwrap_or(0);
            let b = usize::try_from(hi.min(end).saturating_sub(at)).unwrap_or(0);
            acc = acc.then(p.read().fold(a, b.min(n)).shifted(at));
        }
        acc
    }

    /// The least and greatest valid y with x in `x0..x1`, in O(log n): what
    /// a y axis that follows the view reads ([`super::plot::YAuto`]).
    #[must_use]
    pub fn range_y(&self, x0: i64, x1: i64) -> Option<(f32, f32)> {
        let s = self.fold(self.lower_bound(x0), self.lower_bound(x1));
        (s.first != NONE).then(|| (unkey(s.min), unkey(s.max)))
    }

    /// The valid point nearest `x` (by x; the earlier of two as near), by
    /// binary search.
    #[must_use]
    pub fn nearest(&self, x: i64) -> Option<u32> {
        let i = self.lower_bound(x);
        let after = self.fold(i, self.len()).first;
        let before = self.fold(0, i).last;
        let dist = |j: u32| self.get(j).map(|(px, _)| px.abs_diff(x));
        match (before, after) {
            (NONE, NONE) => None,
            (b, NONE) => Some(b),
            (NONE, a) => Some(a),
            (b, a) => match (dist(b), dist(a)) {
                (Some(db), Some(da)) if da < db => Some(a),
                _ => Some(b),
            },
        }
    }

    /// The valid point of `lo..hi` nearest `at` (data x and y), distance
    /// measured after scaling by `k` (pixels per unit of x and of y), within
    /// `radius` pixels; ties keep the earlier. Ranges whose y never comes
    /// within reach are skipped through the pyramid, so a dense view looks
    /// at the points near the pointer, not at every point in its columns.
    #[must_use]
    pub fn nearest_within(&self, lo: u32, hi: u32, at: (i64, f64), k: (f64, f64), radius: f64) -> Option<u32> {
        let reach_y = if k.1 > 0.0 { radius / k.1 } else { f64::INFINITY };
        let (ylo, yhi) = (at.1 - reach_y, at.1 + reach_y);
        let mut best: Option<(f64, u32)> = None;
        for (p, n, start) in self.pages() {
            let end = start.saturating_add(u32::try_from(n).unwrap_or(NONE));
            if end <= lo || start >= hi {
                continue;
            }
            let a = usize::try_from(lo.saturating_sub(start)).unwrap_or(0);
            let b = usize::try_from(hi.min(end).saturating_sub(start)).unwrap_or(0).min(n);
            let d = p.read();
            let near = |node: &Node| {
                node.first != NONE && f64::from(unkey(node.max)) >= ylo && f64::from(unkey(node.min)) <= yhi
            };
            // The range as whole nodes, as `fold` cuts it, then down
            // through those that reach.
            let mut stack: Vec<(usize, usize)> = Vec::new();
            let (mut l, mut r, mut level) = (a, b, 0_usize);
            while l < r {
                while l < r && !l.is_multiple_of(FANOUT) {
                    stack.push((level, l));
                    l = l.saturating_add(1);
                }
                while l < r && !r.is_multiple_of(FANOUT) {
                    r = r.saturating_sub(1);
                    stack.push((level, r));
                }
                l /= FANOUT;
                r /= FANOUT;
                level = level.saturating_add(1);
            }
            while let Some((level, j)) = stack.pop() {
                let node = d.node(level, j);
                if !near(&node) {
                    continue;
                }
                if level == 0 {
                    let (Some(x), Some(y)) = (d.x.get(j), d.y.get(j)) else {
                        continue;
                    };
                    let dx = i128_to_f64(i128::from(*x).saturating_sub(i128::from(at.0))) * k.0;
                    let dy = (f64::from(*y) - at.1) * k.1;
                    let dist = dx.hypot(dy);
                    let i = start.saturating_add(u32::try_from(j).unwrap_or(NONE));
                    if dist <= radius && best.is_none_or(|(bd, bi)| dist < bd || (dist <= bd && i < bi)) {
                        best = Some((dist, i));
                    }
                    continue;
                }
                let first = j.saturating_mul(FANOUT);
                for c in first..first.saturating_add(FANOUT) {
                    stack.push((level.saturating_sub(1), c));
                }
            }
        }
        best.map(|(_, i)| i)
    }

    /// The columns of a view of `width` device pixels showing x from `x0`
    /// to `x1`, the view's left edge `frac` of a pixel right of the first
    /// column's: column `c` is device pixel `c`.
    #[must_use]
    pub fn columns(&self, x0: i64, x1: i64, width: f32, frac: f32) -> Columns {
        let bounds = column_bounds(x0, x1, width, frac);
        let first = bounds.first().copied().unwrap_or(x0);
        let last = bounds.last().copied().unwrap_or(x1);
        let lo = self.lower_bound(first);
        let hi = self.lower_bound(last);
        Columns {
            bounds,
            before: lo.checked_sub(1),
            after: (hi < self.len()).then_some(hi),
        }
    }
}

/// Column `c`'s first x for each `c` up to the column count, which is the
/// device pixels the view's `width` touches from `frac` on.
#[must_use]
pub fn column_bounds(x0: i64, x1: i64, width: f32, frac: f32) -> Vec<i64> {
    let cols = column_count(width, frac);
    let span = i128_to_f64(i128::from(x1).saturating_sub(i128::from(x0)).max(1));
    let w = f64::from(width.max(1e-3));
    let f = f64::from(frac);
    (0..=cols)
        .map(|c| {
            // Device pixel c starts (c - frac) / width of the way through.
            let t = (f64::from(c) - f) / w;
            let at = (t * span).ceil();
            // Within ±2^62 an i64 holds it, and the sum saturates as the
            // i128 one would (no 128-bit float conversion, a library call).
            if at.abs() < 4.611_686_018_427_388e18 {
                x0.saturating_add(crate::convert::f64_to_i64(at))
            } else {
                i64::try_from(i128::from(x0).saturating_add(f64_to_i128(at))).unwrap_or(if at < 0.0 {
                    i64::MIN
                } else {
                    i64::MAX
                })
            }
        })
        .collect()
}

/// The device pixel columns a view of `width` touches from `frac` on.
#[must_use]
pub fn column_count(width: f32, frac: f32) -> u32 {
    crate::convert::f32_to_u32((frac + width).ceil()).max(1)
}

#[expect(
    clippy::as_conversions,
    reason = "an i128 span to a float scale, rounding as intended"
)]
fn i128_to_f64(v: i128) -> f64 {
    v as f64
}

#[expect(
    clippy::as_conversions,
    reason = "a float offset to whole x units, saturating as intended"
)]
fn f64_to_i128(v: f64) -> i128 {
    v as i128
}

/// M4 of `columns` (the CPU reference of the `m4_*.wgsl` passes): the
/// point before the view, the `bounds.len() - 1` columns, the point after.
#[must_use]
pub fn m4(s: &SeriesSnapshot, columns: &Columns) -> Vec<Column> {
    let single = |i: Option<u32>| i.map_or(Column::default(), |i| column(s, s.fold(i, i.saturating_add(1))));
    let mut out = vec![single(columns.before)];
    out.extend(columns.bounds.windows(2).map(|w| match *w {
        [a, b] => {
            let (lo, hi) = (s.lower_bound(a), s.lower_bound(b));
            column(s, s.fold(lo, hi))
        }
        _ => Column::default(),
    }));
    out.push(single(columns.after));
    out
}

/// A range's summary as M4's points: first, least and greatest in index
/// order, last, each once.
#[must_use]
pub fn column(s: &SeriesSnapshot, sum: Summary) -> Column {
    let mut c = Column::default();
    if sum.first == NONE {
        return c;
    }
    let (a, b) = if sum.imin <= sum.imax {
        (sum.imin, sum.imax)
    } else {
        (sum.imax, sum.imin)
    };
    let mut n = 0_usize;
    for i in [sum.first, a, b, sum.last] {
        let dup = n > 0 && c.idx.get(n.saturating_sub(1)) == Some(&i);
        if !dup && let (Some(slot), Some(ys)) = (c.idx.get_mut(n), c.y.get_mut(n)) {
            *slot = i;
            *ys = s.get(i).map_or(0, |(_, y)| crate::convert::f64_to_f32(y).to_bits());
            n = n.saturating_add(1);
        }
    }
    c.count = u32::try_from(n).unwrap_or(0);
    c
}

/// A series as its owner holds it (the UI thread): its pages, what it keeps.
#[derive(Debug)]
pub struct SeriesState {
    id: u64,
    version: u64,
    generation: u64,
    pages: Vec<Arc<Page>>,
    /// The first kept point's place among all ever given.
    start: u64,
    /// Keep at most about this many of the last points (`None`: all).
    retain: Option<u64>,
    /// Points a page holds.
    page: usize,
    precise: bool,
    next_seq: u64,
    last_x: i64,
}

/// The series and the pages made, numbered ([`crate::engine::unique_id`]).
static NEXT_SERIES: crate::engine::IdKind = crate::engine::IdKind::new("series");
static NEXT_PAGE: crate::engine::IdKind = crate::engine::IdKind::new("series page");

impl SeriesState {
    /// An empty series keeping its last `retain` points (all with `None`);
    /// `precise` keeps y's low part too.
    #[must_use]
    pub fn new(retain: Option<u64>, precise: bool) -> Self {
        let page = retain.map_or(MAX_PAGE, |r| {
            usize::try_from(r / 4)
                .unwrap_or(MAX_PAGE)
                .next_power_of_two()
                .clamp(MIN_PAGE, MAX_PAGE)
        });
        Self {
            id: crate::engine::unique_id(&NEXT_SERIES),
            version: 0,
            generation: 0,
            pages: Vec::new(),
            start: 0,
            retain,
            page,
            precise,
            next_seq: 0,
            last_x: i64::MIN,
        }
    }

    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    #[must_use]
    pub fn len(&self) -> u64 {
        self.pages
            .iter()
            .map(|p| u64::try_from(p.read().len()).unwrap_or(0))
            .sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Append points: x never goes back (one before the last is moved up
    /// to it), a y that is not finite is a gap.
    pub fn append(&mut self, xs: &[i64], ys: &[f64]) {
        let mut i = 0_usize;
        let n = xs.len().min(ys.len());
        while i < n {
            let room = self.room();
            let take = room.min(n.saturating_sub(i));
            let Some(page) = self.pages.last() else { break };
            let mut d = page.data.write().unwrap_or_else(std::sync::PoisonError::into_inner);
            let from = d.len();
            for (x, y) in xs.iter().zip(ys).skip(i).take(take) {
                let x = (*x).max(self.last_x);
                self.last_x = x;
                let hi = crate::convert::f64_to_f32(*y);
                d.x.push(x);
                d.y.push(hi);
                if let Some(lo) = d.lo.as_mut() {
                    lo.push(if valid(hi) {
                        crate::convert::f64_to_f32(*y - f64::from(hi))
                    } else {
                        0.0
                    });
                }
            }
            d.build(from);
            drop(d);
            i = i.saturating_add(take);
        }
        self.drop_old();
        self.version = self.version.saturating_add(1);
    }

    /// Points the last page has room for, a page added when it is full.
    fn room(&mut self) -> usize {
        let full = self.pages.last().is_none_or(|p| p.read().len() >= self.page);
        if full {
            let start = self.start.saturating_add(self.len());
            let data = PageData {
                lo: self.precise.then(Vec::new),
                ..PageData::default()
            };
            self.next_seq = self.next_seq.saturating_add(1);
            self.pages.push(Arc::new(Page {
                seq: crate::engine::unique_id(&NEXT_PAGE),
                start,
                data: RwLock::new(data),
            }));
        }
        self.pages
            .last()
            .map_or(0, |p| self.page.saturating_sub(p.read().len()))
    }

    /// Whole pages before the last `retain` points let go.
    fn drop_old(&mut self) {
        let Some(retain) = self.retain else { return };
        while self.pages.len() > 1 {
            let first = self
                .pages
                .first()
                .map_or(0, |p| u64::try_from(p.read().len()).unwrap_or(0));
            if self.len().saturating_sub(first) < retain {
                break;
            }
            self.pages.remove(0);
            self.start = self.start.saturating_add(first);
        }
    }

    /// Replace every point.
    pub fn set(&mut self, xs: &[i64], ys: &[f64]) {
        self.start = self.start.saturating_add(self.len());
        self.pages.clear();
        self.last_x = i64::MIN;
        self.generation = self.generation.saturating_add(1);
        self.append(xs, ys);
    }

    #[must_use]
    pub fn snapshot(&self) -> SeriesSnapshot {
        SeriesSnapshot::new(self)
    }
}

#[cfg(test)]
#[expect(clippy::as_conversions, reason = "test values are small and known")]
mod tests {
    use super::*;

    /// The cold check's compare of two series of a million points, paged
    /// differently: it locks each page once, whatever the points.
    #[test]
    fn comparing_series_by_content_locks_each_page_once() {
        let n = 1_000_000_usize;
        let xs: Vec<i64> = (0..n).map(|i| i64::try_from(i).unwrap()).collect();
        let ys: Vec<f64> = (0..n).map(|i| f64::from(u32::try_from(i % 977).unwrap())).collect();
        let made = |retain: u64| {
            let mut s = SeriesState::new(Some(retain), false);
            s.append(&xs, &ys);
            s.snapshot()
        };
        let (a, b) = (made(1_000_000), made(2_000_000));
        let pages = a.pages().count() + b.pages().count();
        assert!(a.pages().count() != b.pages().count(), "paged differently");
        let mut locks = 0;
        assert!(a.same_points_counted(&b, &mut locks), "the same points");
        assert_eq!(locks, pages, "one lock a page");
        let mut other = ys.clone();
        other[n - 1] = 0.5;
        let mut s = SeriesState::new(Some(1_000_000), false);
        s.append(&xs, &other);
        assert!(!a.same_points(&s.snapshot()), "a point apart");
    }

    /// A kept window that dropped its first pages holds the same points as
    /// a series built afresh from those points (what a cold build makes):
    /// where they lie in the series' history is not content.
    #[test]
    fn a_series_built_again_from_its_points_holds_the_same_content() {
        let xs: Vec<i64> = (0..10_000).collect();
        let ys: Vec<f64> = (0..10_000_u32).map(|i| f64::from(i % 31)).collect();
        let mut live = SeriesState::new(Some(1_000), false);
        live.append(&xs, &ys);
        let live = live.snapshot();
        assert!(live.start() > 0, "pages dropped before it");
        let first = usize::try_from(live.start()).unwrap();
        let mut cold = SeriesState::new(None, false);
        cold.append(&xs[first..], &ys[first..]);
        let cold = cold.snapshot();
        assert_eq!(cold.start(), 0, "built from its first point");
        assert!(live.same_points(&cold), "the same points, the same content");
    }

    fn series(ys: &[f64]) -> SeriesState {
        let xs: Vec<i64> = (0..ys.len()).map(|i| i64::try_from(i).unwrap()).collect();
        let mut s = SeriesState::new(None, false);
        s.append(&xs, ys);
        s
    }

    /// Every column by scanning its points: what the pyramid must equal.
    fn scan(s: &SeriesSnapshot, lo: u32, hi: u32) -> Summary {
        (lo..hi).fold(Node::EMPTY, |acc, i| {
            let y = s.get(i).map_or(f32::NAN, |(_, y)| y as f32);
            acc.then(Node::leaf(i, y))
        })
    }

    #[test]
    fn keys_order_as_the_floats_do() {
        let vs = [
            f32::MIN,
            -1.0,
            -f32::MIN_POSITIVE / 2.0,
            -0.0,
            0.0,
            1e-45,
            1.0,
            f32::MAX,
        ];
        for w in vs.windows(2) {
            assert!(key(w[0]) < key(w[1]), "{} < {}", w[0], w[1]);
        }
        for v in vs {
            assert_eq!(unkey(key(v)).to_bits(), v.to_bits());
        }
    }

    #[test]
    fn the_pyramid_folds_as_a_scan_does() {
        let mut rng = crate::testing::Rng::new(7);
        let ys: Vec<f64> = (0..70_000)
            .map(|i| {
                if i % 997 == 3 {
                    f64::NAN
                } else {
                    f64::from(u32::try_from(rng.below(1000)).unwrap()) - 500.0
                }
            })
            .collect();
        let s = series(&ys).snapshot();
        for _ in 0..300 {
            let a = u32::try_from(rng.below(70_000)).unwrap();
            let b = (a + u32::try_from(rng.below(u64::from(70_000 - a + 1))).unwrap()).min(70_000);
            assert_eq!(s.fold(a, b), scan(&s, a, b), "{a}..{b}");
        }
    }

    #[test]
    fn appending_in_pieces_builds_the_same_pyramid() {
        let ys: Vec<f64> = (0..20_000).map(|i| f64::from((i * 7919) % 1013)).collect();
        let whole = series(&ys).snapshot();
        let mut s = SeriesState::new(None, false);
        let mut at = 0;
        for n in [1, 63, 64, 65, 4095, 4097, 11_615] {
            let xs: Vec<i64> = (at..at + n).map(|i| i64::try_from(i).unwrap()).collect();
            s.append(&xs, &ys[at..at + n]);
            at += n;
        }
        let parts = s.snapshot();
        for (a, b) in [(0, 20_000), (17, 19_999), (4096, 8192), (63, 65)] {
            assert_eq!(whole.fold(a, b), parts.fold(a, b));
        }
    }

    #[test]
    fn a_kept_window_drops_whole_pages_and_keeps_its_points() {
        let mut s = SeriesState::new(Some(5000), false);
        for k in 0..20 {
            let xs: Vec<i64> = (k * 1000..(k + 1) * 1000).collect();
            let ys: Vec<f64> = xs.iter().map(|x| *x as f64).collect();
            s.append(&xs, &ys);
        }
        let snap = s.snapshot();
        assert!(snap.len() >= 5000 && snap.len() < 5000 + 2048, "{}", snap.len());
        let (x, y) = snap.get(snap.len() - 1).unwrap();
        assert_eq!((x, y), (19_999, 19_999.0));
        assert_eq!(snap.get(0).unwrap().0, i64::try_from(snap.start()).unwrap());
    }

    #[test]
    fn columns_hold_every_point_once() {
        let s = series(&(0..1000).map(f64::from).collect::<Vec<_>>()).snapshot();
        let c = s.columns(100, 900, 333.0, 0.25);
        let cols = m4(&s, &c);
        assert_eq!(c.before, Some(99));
        // The last column runs to its pixel's right edge, past x1.
        assert_eq!(c.after, Some(902));
        // Each column's first point follows the last column's last.
        let mut next = 100;
        for col in cols.iter().skip(1).take(cols.len() - 2).filter(|c| c.count > 0) {
            assert_eq!(col.idx[0], next);
            next = col.idx[col.count as usize - 1] + 1;
        }
        assert_eq!(next, 902);
    }

    #[test]
    fn the_nearest_point_within_reach_is_found_through_the_pyramid() {
        let mut rng = crate::testing::Rng::new(11);
        let ys: Vec<f64> = (0..50_000).map(|_| f64::from(rng.unit()) * 1000.0).collect();
        let s = series(&ys).snapshot();
        for _ in 0..50 {
            let at = (
                i64::try_from(rng.below(50_000)).unwrap(),
                f64::from(rng.unit()) * 1000.0,
            );
            let k = (0.5, 0.3);
            let (lo, hi) = (s.lower_bound(at.0 - 40), s.lower_bound(at.0 + 41));
            let scan = (lo..hi)
                .filter_map(|i| {
                    let (x, y) = s.get(i)?;
                    let d = (((x - at.0) as f64) * k.0).hypot((y as f32 as f64 - at.1) * k.1);
                    (d <= 20.0).then_some((d, i))
                })
                .fold(None::<(f64, u32)>, |b, (d, i)| match b {
                    Some((bd, _)) if bd <= d => b,
                    _ => Some((d, i)),
                })
                .map(|(_, i)| i);
            assert_eq!(s.nearest_within(lo, hi, at, k, 20.0), scan);
        }
    }

    #[test]
    fn a_column_keeps_first_least_greatest_and_last() {
        let s = series(&[3.0, 9.0, -2.0, 9.0, f64::NAN, 1.0]).snapshot();
        let c = column(&s, s.fold(0, 6));
        assert_eq!(c.count, 4);
        assert_eq!(&c.idx, &[0, 1, 2, 5]);
        assert_eq!(c.y.map(f32::from_bits), [3.0, 9.0, -2.0, 1.0]);
        assert_eq!(s.range_y(0, 6), Some((-2.0, 9.0)));
        assert_eq!(s.nearest(4), Some(3));
    }
}
