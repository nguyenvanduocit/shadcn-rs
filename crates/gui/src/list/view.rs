//! What a list shows of its data, and what is selected in it (ADR-0014).
//!
//! A [`View`] is a permutation: the data rows a sort and a filter keep, in
//! their order, each known by the data's own id. It is built off the UI
//! thread from a [`Snapshot`] (shared, not copied) by [`Views`]: one worker
//! per list, which drops a build the moment a newer one is asked for and
//! delivers only the latest, the list's heights reordered with it, so
//! sorting a million rows never holds a frame. A [`Selection`] is kept in
//! data rows, as ranges, with an "all except" mode, so it survives a sort
//! or a filter unchanged and moves with the data when rows are spliced.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering as Atomic};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use super::virtual_list::{Heights, ListState};
use crate::mailbox::{Sender, mailbox};
use crate::reactive::Snapshot;

/// How two rows of data compare, for a sort.
pub type Order<T> = dyn Fn(&T, &T) -> Ordering;

/// Rows a build goes through between two looks at whether it is still wanted.
const CHUNK: usize = 1 << 14;

/// The data rows a list shows, in the order it shows them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// The data row (index in the snapshot) at each position.
    rows: Arc<Vec<u32>>,
    /// The data's own id of the row at each position.
    ids: Arc<Vec<u64>>,
    /// Each id and its position, by id: what a list's heights follow
    /// across a sort, and across rows spliced into the data between two
    /// views ([`View::moved_to`]).
    by_id: Arc<Vec<(u64, u32)>>,
    /// The data's version it was built from.
    version: u64,
}

impl View {
    /// Every row of `len`, in data order, each row's id its index (data
    /// that is never spliced).
    #[must_use]
    pub fn identity(len: u32) -> Self {
        Self {
            rows: Arc::new((0..len).collect()),
            ids: Arc::new((0..u64::from(len)).collect()),
            by_id: Arc::new((0..len).map(|i| (u64::from(i), i)).collect()),
            version: 0,
        }
    }

    /// The rows of `items` that `keep` keeps, ordered by `order` (a stable
    /// sort: equal rows keep their data order), or in data order; `id`
    /// gives each row's id, the same for a row in every snapshot and
    /// unique among them (the data's key).
    pub fn build<T: Clone>(
        items: &Snapshot<T>,
        id: impl Fn(&T) -> u64,
        keep: impl Fn(&T) -> bool,
        order: Option<&Order<T>>,
    ) -> Self {
        Self::build_until(items, &id, &keep, order, &|| false).unwrap_or_default()
    }

    /// [`Self::build`], given up (`None`) at the first chunk boundary where
    /// `stale` says it is no longer wanted.
    fn build_until<T: Clone>(
        items: &Snapshot<T>,
        id: &dyn Fn(&T) -> u64,
        keep: &dyn Fn(&T) -> bool,
        order: Option<&Order<T>>,
        stale: &dyn Fn() -> bool,
    ) -> Option<Self> {
        let mut kept: Vec<(u32, &T)> = Vec::new();
        for ((t, i), n) in items.iter().zip(0_u32..).zip(0_usize..) {
            if n.is_multiple_of(CHUNK) && stale() {
                return None;
            }
            if keep(t) {
                kept.push((i, t));
            }
        }
        if let Some(cmp) = order {
            kept = sort_until(kept, cmp, stale)?;
        }
        let mut rows = Vec::with_capacity(kept.len());
        let mut ids = Vec::with_capacity(kept.len());
        for (n, &(i, t)) in kept.iter().enumerate() {
            if n.is_multiple_of(CHUNK) && stale() {
                return None;
            }
            rows.push(i);
            ids.push(id(t));
        }
        let mut by_id: Vec<(u64, u32)> = ids.iter().copied().zip(0_u32..).collect();
        if stale() {
            return None;
        }
        by_id.sort_unstable();
        Some(Self {
            rows: Arc::new(rows),
            ids: Arc::new(ids),
            by_id: Arc::new(by_id),
            version: items.version(),
        })
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

    /// The id of the row shown at `position`.
    #[must_use]
    pub fn id(&self, position: usize) -> Option<u64> {
        self.ids.get(position).copied()
    }

    /// Where the row of id `id` is shown; `None` when the view leaves it
    /// out. O(log n).
    #[must_use]
    pub fn position_of(&self, id: u64) -> Option<usize> {
        let at = self.by_id.binary_search_by_key(&id, |&(i, _)| i).ok()?;
        let &(_, p) = self.by_id.get(at)?;
        usize::try_from(p).ok()
    }

    /// Where the row this view shows at `position` is shown in `to` (the
    /// same id, whatever was spliced into the data between them); `None`
    /// when `to` leaves it out: what a list given `to` in place of this
    /// view passes [`ListState::reorder`].
    #[must_use]
    pub fn moved_to(&self, to: &Self, position: u64) -> Option<u64> {
        let id = self.id(usize::try_from(position).ok()?)?;
        to.position_of(id).and_then(|p| u64::try_from(p).ok())
    }

    /// The data's version the view was built from.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }
}

/// `kept` sorted stably by `cmp`: chunks sorted, then merged pass by pass,
/// given up (`None`) at the first chunk boundary where `stale` says so.
fn sort_until<'a, T>(
    mut kept: Vec<(u32, &'a T)>,
    cmp: &Order<T>,
    stale: &dyn Fn() -> bool,
) -> Option<Vec<(u32, &'a T)>> {
    for chunk in kept.chunks_mut(CHUNK) {
        if stale() {
            return None;
        }
        chunk.sort_by(|a, b| cmp(a.1, b.1));
    }
    let mut out = Vec::with_capacity(kept.len());
    let mut width = CHUNK;
    while width < kept.len() {
        out.clear();
        for pair in kept.chunks(width.saturating_mul(2)) {
            let (left, right) = pair.split_at(width.min(pair.len()));
            merge_until(left, right, cmp, &mut out, stale)?;
        }
        std::mem::swap(&mut kept, &mut out);
        width = width.saturating_mul(2);
    }
    Some(kept)
}

/// `left` and `right`, each sorted, merged onto `out` (equal rows: the
/// left's first, so the sort stays stable); `None` once `stale`.
fn merge_until<'a, T>(
    left: &[(u32, &'a T)],
    right: &[(u32, &'a T)],
    cmp: &Order<T>,
    out: &mut Vec<(u32, &'a T)>,
    stale: &dyn Fn() -> bool,
) -> Option<()> {
    let (mut l, mut r) = (left.iter().peekable(), right.iter().peekable());
    loop {
        if out.len().is_multiple_of(CHUNK) && stale() {
            return None;
        }
        let right_first = match (l.peek(), r.peek()) {
            (Some(a), Some(b)) => cmp(b.1, a.1) == Ordering::Less,
            (Some(_), None) => false,
            (None, Some(_)) => true,
            (None, None) => return Some(()),
        };
        let next = if right_first { r.next() } else { l.next() };
        out.extend(next.copied());
    }
}

/// A build asked for: run with a test of whether it is still wanted.
type Job = Box<dyn FnOnce(&dyn Fn() -> bool) -> Option<Built> + Send>;

/// What a build sends back: the view, and the list's heights reordered for it.
struct Built {
    request: u64,
    view: View,
    heights: Heights,
}

/// What the UI thread and the worker share.
struct Shared {
    /// The latest request's number: a build for another stops.
    latest: AtomicU64,
    /// Builds that ran to the end.
    finished: AtomicU64,
    slot: Mutex<Slot>,
    wake: Condvar,
}

/// The one build waiting (a newer request replaces it, so at most one runs
/// and one waits), heights to drop off the UI thread, and whether the
/// views went.
#[derive(Default)]
struct Slot {
    job: Option<(u64, Job)>,
    trash: Vec<Heights>,
    closed: bool,
}

impl Shared {
    fn slot(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Inner {
    /// The latest request's number.
    asked: Cell<u64>,
    shared: Arc<Shared>,
    /// The view shown: what the next one is reordered from.
    shown: RefCell<View>,
    list: ListState,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let mut slot = self.shared.slot();
        slot.closed = true;
        slot.job = None;
        drop(slot);
        self.shared.wake.notify_one();
    }
}

/// A list's views, built on a worker of its own and handed to a handler on
/// the UI thread, the list's heights reordered with them:
/// - **The latest wins.** Each request supersedes the ones before it: a
///   build asked for earlier stops at its next chunk, one still waiting is
///   replaced, and only the latest is delivered, so two sorts of one
///   snapshot can never land in the other order (a snapshot's version
///   cannot tell them apart; the order they were asked in does).
/// - **Heights follow rows, off the UI thread.** The worker reorders the
///   list's heights ([`ListState::lend_heights`]) with the view, by the
///   rows' ids; the UI thread puts them in place in O(1) and adds the rows
///   measured meanwhile ([`ListState::reorder_with`]).
///
/// Clones share it; the worker ends when the last goes.
#[derive(Clone)]
pub struct Views(Rc<Inner>);

impl Views {
    /// Views for `list`, which shows `shown` now, each handed to `on_view`
    /// on the UI thread (a handler that stores it in a signal the list
    /// reads) once the list's heights follow it. Made in a scope, its
    /// handler closes with it ([`mailbox`]).
    pub fn new(list: &ListState, shown: View, mut on_view: impl FnMut(View) + 'static) -> Self {
        let shared = Arc::new(Shared {
            latest: AtomicU64::new(0),
            finished: AtomicU64::new(0),
            slot: Mutex::new(Slot {
                trash: Vec::with_capacity(2),
                ..Slot::default()
            }),
            wake: Condvar::new(),
        });
        let inner = Rc::new(Inner {
            asked: Cell::new(0),
            shared: Arc::clone(&shared),
            shown: RefCell::new(shown),
            list: list.clone(),
        });
        let weak = Rc::downgrade(&inner);
        let to = mailbox(move |built: Built| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            if built.request != inner.asked.get() {
                return;
            }
            let old = inner.shown.replace(built.view.clone());
            let new = &built.view;
            let rows = u64::try_from(new.len()).unwrap_or(u64::MAX);
            if let Some(replaced) = inner.list.reorder_with(built.heights, rows, |p| old.moved_to(new, p)) {
                // The heights replaced go to the worker to drop.
                inner.shared.slot().trash.push(replaced);
                inner.shared.wake.notify_one();
            }
            drop(old);
            on_view(built.view);
        });
        std::thread::spawn(move || work(&shared, &to));
        Self(inner)
    }

    /// Build the view of `items` that `id`, `keep` and `order` give
    /// ([`View::build`]) on the worker, superseding every request before it.
    pub fn spawn<T, I, K, O>(&self, items: Snapshot<T>, id: I, keep: K, order: Option<O>)
    where
        T: Clone + Send + Sync + 'static,
        I: Fn(&T) -> u64 + Send + 'static,
        K: Fn(&T) -> bool + Send + 'static,
        O: Fn(&T, &T) -> Ordering + Send + 'static,
        Snapshot<T>: Send,
    {
        let inner = &self.0;
        let request = inner.asked.get().wrapping_add(1);
        inner.asked.set(request);
        inner.shared.latest.store(request, Atomic::Release);
        let old = inner.shown.borrow().clone();
        let mut heights = inner.list.lend_heights();
        let job: Job = Box::new(move |stale| {
            let order = order.as_ref().map(|o| -> &Order<T> { o });
            let view = View::build_until(&items, &id, &keep, order, stale)?;
            let rows = u64::try_from(view.len()).unwrap_or(u64::MAX);
            heights.reorder(rows, |p| old.moved_to(&view, p));
            (!stale()).then_some(Built { request, view, heights })
        });
        let superseded = inner.shared.slot().job.replace((request, job));
        inner.shared.wake.notify_one();
        // Shared snapshots only: dropping them here is a few counts.
        drop(superseded);
    }

    /// Builds that ran to the end (the rest stopped, superseded).
    #[cfg(test)]
    fn finished(&self) -> u64 {
        self.0.shared.finished.load(Atomic::Acquire)
    }
}

/// The worker: the build waiting, if still the latest, then the next.
fn work(shared: &Shared, to: &Sender<Built>) {
    let mut spare: Vec<Heights> = Vec::with_capacity(2);
    loop {
        let (request, job) = {
            let mut slot = shared.slot();
            loop {
                // Heights the UI thread let go of, dropped here, outside the lock.
                std::mem::swap(&mut slot.trash, &mut spare);
                if !spare.is_empty() {
                    drop(slot);
                    spare.clear();
                    slot = shared.slot();
                    continue;
                }
                if slot.closed {
                    return;
                }
                if let Some(job) = slot.job.take() {
                    break job;
                }
                slot = shared.wake.wait(slot).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let stale = || shared.latest.load(Atomic::Acquire) != request;
        if let Some(built) = job(&stale) {
            shared.finished.fetch_add(1, Atomic::AcqRel);
            if !stale() {
                to.send(built);
            }
        }
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
        let v = View::build(&c.snapshot(), |v| u64::from(*v), odd, Some(&by));
        let shown: Vec<u32> = (0..v.len()).filter_map(|p| v.row(p)).collect();
        // 1 (row 3), 3 (row 1), 5 (row 0), 9 (row 4).
        assert_eq!(shown, vec![3, 1, 0, 4]);
        c.dispose();
    }

    /// Frontiers 4.1: two sorts of one snapshot, the first asked finishing
    /// last. Only the latest request's view reaches the handler.
    /// Wait for a delivery from another thread, and run its handler.
    fn arrive() {
        for _ in 0..10_000 {
            if crate::mailbox::deliver() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("no view arrived");
    }

    /// Views delivering to a vector of what each shows (data rows), and
    /// the gate the first build waits at.
    fn views_into(len: u32) -> (Views, Rc<RefCell<Vec<Vec<u32>>>>) {
        let shown = Rc::new(RefCell::new(Vec::new()));
        let into = Rc::clone(&shown);
        let views = Views::new(&ListState::new(24.0), View::identity(len), move |v: View| {
            into.borrow_mut()
                .push((0..v.len()).filter_map(|p| v.row(p)).collect::<Vec<_>>());
        });
        (views, shown)
    }

    /// A `keep` that holds its build at `gate` until it opens.
    fn held(gate: &Arc<std::sync::atomic::AtomicBool>) -> impl Fn(&u32) -> bool + Send + 'static {
        let gate = Arc::clone(gate);
        move |_: &u32| {
            while !gate.load(Atomic::SeqCst) {
                std::thread::yield_now();
            }
            true
        }
    }

    /// Frontiers 4.1: two sorts of one snapshot, the first asked still
    /// running when the second is. Only the latest request's view reaches
    /// the handler.
    #[test]
    fn of_two_sorts_of_one_snapshot_the_later_asked_wins_whichever_ends_first() {
        let c = Collection::new([2_u32, 3, 1]);
        let (views, shown) = views_into(3);
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let id = |v: &u32| u64::from(*v);
        views.spawn(c.snapshot(), id, held(&gate), Some(|a: &u32, b: &u32| a.cmp(b)));
        views.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| b.cmp(a)));
        gate.store(true, Atomic::SeqCst);
        arrive();
        // 3 (row 1), 2 (row 0), 1 (row 2): the descending view, alone.
        assert_eq!(*shown.borrow(), [vec![1, 0, 2]]);
        c.dispose();
    }

    /// Codex (M15 review 1): requests asked faster than built. One worker,
    /// one build running and one waiting: the running one stops at its
    /// next chunk once superseded, the waiting ones are replaced, and only
    /// the latest runs to the end.
    #[test]
    fn superseded_sorts_stop_and_only_the_latest_is_built() {
        let c = Collection::new((0..100_000_u32).rev());
        let (views, shown) = views_into(100_000);
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let id = |v: &u32| u64::from(*v);
        views.spawn(c.snapshot(), id, held(&gate), Some(|a: &u32, b: &u32| a.cmp(b)));
        for _ in 0..20 {
            views.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        }
        views.spawn(c.snapshot(), id, |v: &u32| *v < 3, Some(|a: &u32, b: &u32| b.cmp(a)));
        gate.store(true, Atomic::SeqCst);
        arrive();
        assert_eq!(*shown.borrow(), [vec![99_997, 99_998, 99_999]], "the latest, alone");
        assert_eq!(views.finished(), 1, "every other build stopped or never ran");
        c.dispose();
    }

    #[test]
    fn a_view_knows_where_each_row_went() {
        let c = Collection::new([5_u32, 3, 8, 1]);
        let all = View::identity(4);
        let odd = View::build(
            &c.snapshot(),
            |v| u64::from(*v),
            |v| v % 2 == 1,
            Some(&|a: &u32, b: &u32| a.cmp(b)),
        );
        // Odd ascending: 1 (row 3), 3 (row 1), 5 (row 0); 8 (row 2) left out.
        assert_eq!(odd.position_of(1), Some(0));
        assert_eq!(odd.position_of(8), None);
        // The identity's ids are its rows: 5 is not an id there, row 0 is.
        let ids = View::build(&c.snapshot(), |v| u64::from(*v), |_| true, None);
        let moved: Vec<Option<u64>> = (0..4).map(|p| ids.moved_to(&odd, p)).collect();
        assert_eq!(moved, [Some(2), Some(1), None, Some(0)]);
        assert_eq!(all.id(3), Some(3));
        c.dispose();
    }

    /// Codex and Sonnet (M15 review 1): rows spliced into the data between
    /// two views. Heights follow the rows by id, not by data index.
    #[test]
    fn heights_follow_rows_by_id_across_rows_spliced_into_the_data() {
        use crate::list::ExtentTree;

        // (id, amount)
        let c = Collection::new((0..10_u64).map(|i| (i, i * 10)));
        let id = |t: &(u64, u64)| t.0;
        let up = |a: &(u64, u64), b: &(u64, u64)| a.1.cmp(&b.1);
        let down = |a: &(u64, u64), b: &(u64, u64)| b.1.cmp(&a.1);
        let before = View::build(&c.snapshot(), id, |_| true, Some(&up));
        // Each row's height tells its id.
        let tall = |id: u64| 100 + u32::try_from(id).unwrap();
        let mut heights = ExtentTree::new(10, 10);
        for p in 0..10 {
            heights.set(p, tall(before.id(usize::try_from(p).unwrap()).unwrap()));
        }
        c.splice(0, 0, (100..103).map(|i| (i, 1_000 + i)));
        let after = View::build(&c.snapshot(), id, |_| true, Some(&down));
        heights.reorder(13, |p| before.moved_to(&after, p));
        for q in 0..13 {
            let id = after.id(usize::try_from(q).unwrap()).unwrap();
            if id < 10 {
                assert_eq!(heights.height(q), Some(tall(id)), "row {id} at {q}");
            } else {
                assert!(!heights.is_measured(q), "row {id}, new, at {q}");
            }
        }
        c.dispose();
    }

    #[test]
    fn a_selection_survives_a_sort_and_moves_with_a_splice() {
        let c = Collection::new((0..100_u32).rev());
        let mut s = Selection::new();
        s.select(10..20);
        s.toggle(50);
        let by = |a: &u32, b: &u32| a.cmp(b);
        let sorted = View::build(&c.snapshot(), |v| u64::from(*v), |_| true, Some(&by));
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
