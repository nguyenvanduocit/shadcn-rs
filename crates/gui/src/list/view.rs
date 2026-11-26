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

use std::cell::RefCell;
use std::cmp::Ordering;
use std::ops::Range;
use std::panic::AssertUnwindSafe;
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
            kept = sort_until(kept, &|a: &(u32, &T), b: &(u32, &T)| cmp(a.1, b.1), stale)?;
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
        let by_id: Vec<(u64, u32)> = ids.iter().copied().zip(0_u32..).collect();
        let by_id = sort_until(by_id, &|a: &(u64, u32), b: &(u64, u32)| a.cmp(b), stale)?;
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
fn sort_until<E: Copy>(mut kept: Vec<E>, cmp: &dyn Fn(&E, &E) -> Ordering, stale: &dyn Fn() -> bool) -> Option<Vec<E>> {
    for chunk in kept.chunks_mut(CHUNK) {
        if stale() {
            return None;
        }
        chunk.sort_by(cmp);
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
fn merge_until<E: Copy>(
    left: &[E],
    right: &[E],
    cmp: &dyn Fn(&E, &E) -> Ordering,
    out: &mut Vec<E>,
    stale: &dyn Fn() -> bool,
) -> Option<()> {
    let (mut l, mut r) = (left.iter().peekable(), right.iter().peekable());
    loop {
        if out.len().is_multiple_of(CHUNK) && stale() {
            return None;
        }
        let right_first = match (l.peek(), r.peek()) {
            (Some(a), Some(b)) => cmp(b, a) == Ordering::Less,
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

/// How to ask for a request's build again: given the view shown, the
/// heights lent, whether they are laid out as that view says, and the
/// request's number.
type Recipe = Rc<dyn Fn(View, Heights, bool, u64) -> Job>;

/// What a build sends back: the view, and the list's heights reordered for it.
struct Built {
    request: u64,
    view: View,
    heights: Heights,
}

/// What the worker sends the UI thread.
enum Outcome {
    Built(Built),
    /// A closure the request gave (its `id`, `keep` or `order`) panicked:
    /// the request is dropped, the worker serves the next.
    Failed {
        request: u64,
        message: String,
    },
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
    /// Where the worker sends what it built; taken as the worker starts, at
    /// the first request asked outside a check (a check starts no thread).
    to: Option<Sender<Outcome>>,
}

impl Shared {
    fn slot(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The views as their handles share them: what is fixed as they are made.
/// What changes is the registry's ([`VIEWS`], review r17, decision 1), so
/// what a check does to views it reaches is the check's, by construction;
/// what the worker shares is the outside world's ([`Shared`]).
struct Inner {
    /// Their state's id in [`VIEWS`].
    key: crate::engine::SlabId,
    shared: Arc<Shared>,
    list: ListState,
    /// Who these views are to the list ([`ListState::claim_heights`]).
    holder: u64,
    /// Made in a fork (a check, ADR-0028): what it made is the fork's, so
    /// kept past its rollback these views ask for nothing (no worker, no
    /// loan, no claim on the list).
    inert: bool,
    /// Asks the latest request again: what the list wakes, once the
    /// heights another holder held are let go, when it refused these views
    /// ([`ListState::wait_for_heights`], which holds it weakly).
    wake: Rc<dyn Fn()>,
    /// Their end as the last `Rc` goes ([`Gone`]).
    _gone: crate::engine::Ends<Gone>,
}

/// What views are now, by their id ([`Inner::key`]).
#[derive(Clone)]
struct ViewsState {
    /// The latest request's number.
    asked: u64,
    /// How to ask for the latest request again (its heights came back
    /// refused: rows moved while it was built).
    recipe: Option<Recipe>,
    /// The view shown: what the next one is reordered from.
    shown: View,
    /// The list's shape when `shown` was last true of its rows: a list
    /// changed since (spliced by its owner) is not laid out as `shown`
    /// says, so its heights cannot follow ids and are dropped.
    shown_shape: u64,
    /// Why the latest request failed, if it did.
    failed: Option<String>,
    /// The loan the latest request took ([`ListState::lend_heights`]): the
    /// only one these views end, whoever lent after it.
    loan: Option<u64>,
    /// The live handles ([`Views`]): the last one's drop releases them.
    handles: usize,
    /// Whether they were released ([`release`]): once, and nothing asked
    /// after.
    released: bool,
}

// A logged value (ADR-0034 S7, audited): `&ViewsState` changes nothing;
// its recipe is called, never replaced, through it.
crate::engine::fork_value_audited!(ViewsState);
impl crate::engine::parts_seal::Sealed for ViewsState {}
impl crate::engine::Parts for ViewsState {
    /// The recipe (an app closure, what it captured) and the view each on
    /// their own box (S9).
    /// Every field named (the drain lint).
    fn parts(self, out: &mut Vec<Box<dyn std::any::Any>>) {
        let Self {
            asked,
            recipe,
            shown,
            shown_shape,
            failed,
            loan,
            handles,
            released,
        } = self;
        recipe.parts(out);
        crate::engine::whole(shown, out);
        failed.parts(out);
        // Data: nothing to drop.
        let _data = (asked, shown_shape, loan, handles, released);
    }
}
impl crate::engine::sealed::Sealed for ViewsState {}
impl crate::engine::ForkClone for ViewsState {
    /// O(1): the recipe is shared by identity (`Rc`), the view's rows by
    /// `Arc`; the message is copied (one string, a failure's).
    fn fork_clone(&self) -> Self {
        self.clone()
    }
}

crate::engine::engine_local! {
    /// Every list views' state, by its id: a check's writes are its own.
    [Fork(log)] static VIEWS: RefCell<crate::engine::ForkSlab<ViewsState>> = RefCell::default();
}

impl Inner {
    /// The heights replaced or refused, to the worker to drop.
    fn dispose(&self, heights: Heights) {
        self.shared.slot().trash.push(heights);
        self.shared.wake.notify_one();
    }

    /// Ask for the build `recipe` makes, superseding every request before it.
    /// Refused while other views hold the list's heights: nothing lent, the
    /// request kept (the latest), and asked again when they are let go.
    /// Released views ask for nothing (a check's leaked clone woken late).
    fn ask(&self, recipe: &Recipe) {
        // The worker and what it shares are the outside world's: a check
        // (whatever woke it: a spawn, a loan let go) asks none of it.
        if crate::engine::forking() || self.read(|s| s.released).unwrap_or(true) {
            return;
        }
        if !self.list.claim_heights(self.holder) {
            self.list.wait_for_heights(self.holder, Rc::downgrade(&self.wake));
            return;
        }
        let to = self.shared.slot().to.take();
        if let Some(to) = to {
            let shared = Arc::clone(&self.shared);
            std::thread::spawn(move || work(&shared, &to));
        }
        let Some((request, old, shape)) = self.write(|s| {
            s.asked = s.asked.wrapping_add(1);
            (s.asked, s.shown.clone(), s.shown_shape)
        }) else {
            return;
        };
        self.shared.latest.store(request, Atomic::Release);
        let mappable = self.list.shape() == shape;
        let heights = self.list.lend_heights_for(self.holder);
        let loan = heights.loan();
        self.write(|s| s.loan = Some(loan));
        let job = recipe(old, heights, mappable, request);
        let superseded = self.shared.slot().job.replace((request, job));
        self.shared.wake.notify_one();
        // Its view is not built yet and its heights are the list's, shared:
        // dropping it here is a few counts.
        drop(superseded);
    }

    /// Ask the latest request again, if one is kept.
    fn ask_again(&self) {
        let recipe = self.read(|s| s.recipe.clone()).flatten();
        if let Some(recipe) = recipe {
            self.ask(&recipe);
        }
    }

    /// End the loan these views took, if it is still the list's.
    fn end_loan(&self) {
        if let Some(loan) = self.write(|s| s.loan.take()).flatten() {
            self.list.end_loan(loan);
        }
    }

    /// Their state, as the running world sees it (`None`: gone).
    fn read<R>(&self, f: impl FnOnce(&ViewsState) -> R) -> Option<R> {
        VIEWS.with_borrow(|v| v.read(self.key, f))
    }

    /// Change their state, in the running world.
    fn write<R>(&self, f: impl FnOnce(&mut ViewsState) -> R) -> Option<R> {
        VIEWS.with_borrow(|v| v.write(self.key, f))
    }

    /// What releases them, by value: the queue may run it once the views
    /// are gone.
    fn releaser(&self) -> Releaser {
        Releaser {
            key: self.key,
            shared: Arc::clone(&self.shared),
            list: self.list,
            holder: self.holder,
        }
    }

    /// An explicit end (their scope's dispose, their window's close): in a
    /// check, the check's copy of them ends, and nothing of the live world
    /// (their worker, the list's loan and claim are the outside world's,
    /// which no rollback takes back; the live end does it), review r17,
    /// decision 3.
    fn end(&self) {
        if crate::engine::forking() && !self.inert {
            self.write(|s| s.released = true);
            return;
        }
        self.releaser().release();
    }
}

/// What releases views (ADR-0034, S4): the worker told to close, the loan
/// ended, the claim on the list's heights and the wait for them let go.
/// Once: their state says whether it ran.
struct Releaser {
    key: crate::engine::SlabId,
    shared: Arc<Shared>,
    list: ListState,
    holder: u64,
}

impl Releaser {
    /// Release them, once, in the running world.
    fn release(&self) {
        let loan = VIEWS
            .try_with(|v| {
                v.with(|v| {
                    v.write(self.key, |s| {
                        if s.released {
                            None
                        } else {
                            s.released = true;
                            Some(s.loan.take())
                        }
                    })
                })
            })
            .ok()
            .flatten()
            .flatten();
        let Some(loan) = loan else { return };
        // The worker is told first (the running build stops at its next
        // chunk, the worker ends), whatever ending the loan meets: the
        // list may be gone, and that is quiet.
        self.shared.latest.fetch_add(1, Atomic::AcqRel);
        let mut slot = self.shared.slot();
        slot.closed = true;
        slot.job = None;
        slot.to = None;
        drop(slot);
        self.shared.wake.notify_one();
        if let Some(loan) = loan {
            self.list.end_loan(loan);
        }
        // Quiet if it never held them or the list is gone.
        self.list.release_heights(self.holder);
    }

    /// The last live handle dropped: one fewer, and released at none.
    fn handle_dropped(&self) {
        let left = VIEWS
            .try_with(|v| {
                v.with(|v| {
                    v.write(self.key, |s| {
                        s.handles = s.handles.saturating_sub(1);
                        s.handles
                    })
                })
            })
            .ok()
            .flatten();
        if left == Some(0) {
            self.release();
        }
    }
}

impl crate::focus::WindowOwned for Inner {
    /// Its scope or its window ended ([`crate::engine::own`]).
    fn end_with_window(&self) {
        self.end();
    }
}

/// Views' end as their last `Rc` goes, in the world that made them
/// ([`crate::engine::Ends`], ADR-0035). Views a check made have no live
/// handle: released as they go, and their state with them. Live ones were
/// released by their last live handle or their owner; this is a no-op for
/// them but for their state. Live views a check lets go are let go once it
/// has switched back: their worker is the outside world.
struct Gone(Releaser);

impl crate::engine::Ending for Gone {
    fn end(self) {
        self.0.release();
        let _gone = VIEWS.try_with(|v| v.with_mut(|v| v.remove(self.0.key)));
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
/// Clones share it; the worker ends when the last live handle goes. A
/// handle cloned while a check runs is the check's: it holds no obligation
/// (ADR-0034, S4), so the worker's end never waits on a check's garbage.
pub struct Views(Rc<Inner>, Option<crate::engine::Ends<Handle>>);

/// A live [`Views`] handle, one live code made or cloned, counted in
/// `Inner::handles`: as it goes, one fewer, in the world that made it
/// ([`crate::engine::Ends`], ADR-0035; a live handle a check lets go is gone
/// from the live world, so its count goes once the check has switched
/// back), and the views released at none. A check's handle has none.
///
/// It holds the views themselves: a live handle a check lets go moves to
/// the live world whole, so the views (their wake, which the list holds
/// weakly) live on until its end there, as they do when live code lets
/// go of it (V2: a waiting view, woken by a sibling's release, asked the
/// list again live and not when dropped in a check).
struct Handle(Rc<Inner>);

impl crate::engine::Ending for Handle {
    fn end(self) {
        self.0.releaser().handle_dropped();
    }
}

impl Views {
    /// A live handle's count, for `inner`'s views.
    fn live(inner: &Rc<Inner>) -> Option<crate::engine::Ends<Handle>> {
        Some(crate::engine::Ends::new(Handle(Rc::clone(inner))))
    }
}

impl Clone for Views {
    fn clone(&self) -> Self {
        // Which world the clone is of is fixed as it is made: one a check
        // makes is counted nowhere, so it pins nothing live.
        let handle = if self.1.is_some() && !crate::engine::is_cold() {
            self.0.write(|s| s.handles = s.handles.saturating_add(1));
            Self::live(&self.0)
        } else {
            None
        };
        Self(Rc::clone(&self.0), handle)
    }
}

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
        let inert = crate::engine::forking();
        let key = crate::engine::allocate(
            &VIEWS,
            ViewsState {
                asked: 0,
                recipe: None,
                shown,
                shown_shape: list.shape(),
                failed: None,
                loan: None,
                handles: usize::from(!inert),
                released: false,
            },
        );
        let holder = crate::engine::unique_id(&super::virtual_list::HOLDERS);
        let inner = Rc::new_cyclic(|me: &std::rc::Weak<Inner>| Inner {
            wake: {
                let me = me.clone();
                Rc::new(move || {
                    if let Some(inner) = me.upgrade() {
                        inner.ask_again();
                    }
                })
            },
            key,
            shared: Arc::clone(&shared),
            list: *list,
            holder,
            inert,
            _gone: crate::engine::Ends::new(Gone(Releaser {
                key,
                shared: Arc::clone(&shared),
                list: *list,
                holder,
            })),
        });
        // The obligations end with the live owner, exactly once, whatever
        // still holds a handle (ADR-0034, S4): a handle an opaque value
        // holds, which a runaway check set aside, can pin the count but
        // not the release. Its owner is the scope running, else the window
        // whose code runs (ADR-0015, a window owns what is made in it);
        // with neither, a debug build refuses it (review r17, decision 5).
        let as_owned: Rc<dyn crate::focus::WindowOwned> = Rc::<Inner>::clone(&inner);
        crate::engine::own("gui::list::Views::new", &as_owned);
        drop(as_owned);
        let handle = if inert { None } else { Self::live(&inner) };
        let weak = Rc::downgrade(&inner);
        let to = mailbox(move |outcome: Outcome| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            if !inner.list.is_alive() {
                // The list is gone for good, not reshaped: nothing to
                // build again for, so the request ends here, built or
                // failed, its recipe (the snapshot and the closures it
                // holds) with it, and no later one is asked.
                let recipe = inner.write(|s| {
                    s.loan = None;
                    s.recipe.take()
                });
                drop(recipe);
                if let Outcome::Built(built) = outcome {
                    inner.dispose(built.heights);
                }
                return;
            }
            let asked = inner.read(|s| s.asked);
            let built = match outcome {
                Outcome::Built(built) => built,
                Outcome::Failed { request, message } => {
                    if Some(request) == asked {
                        inner.end_loan();
                        inner.write(|s| s.failed = Some(message));
                    }
                    return;
                }
            };
            if Some(built.request) != asked {
                inner.dispose(built.heights);
                return;
            }
            let new = built.view;
            let rows = u64::try_from(new.len()).unwrap_or(u64::MAX);
            let old = inner.read(|s| s.shown.clone()).unwrap_or_default();
            match inner.list.reorder_with(built.heights, rows, |p| old.moved_to(&new, p)) {
                Ok(replaced) => {
                    inner.dispose(replaced);
                    let shape = inner.list.shape();
                    let failed = inner.write(|s| {
                        s.shown = new.clone();
                        s.shown_shape = shape;
                        s.failed.take()
                    });
                    drop(failed);
                    drop(old);
                    on_view(new);
                }
                Err(refused) => {
                    // Rows moved while it was built: built again on a
                    // fresh loan, never moved by places that no longer hold.
                    inner.dispose(refused);
                    inner.ask_again();
                }
            }
        });
        // The worker starts at the first request ([`Inner::ask`]).
        shared.slot().to = Some(to);
        drop(shared);
        Self(inner, handle)
    }

    /// Build the view of `items` that `id`, `keep` and `order` give
    /// ([`View::build`]) on the worker, superseding every request before it.
    pub fn spawn<T, I, K, O>(&self, items: Snapshot<T>, id: I, keep: K, order: Option<O>)
    where
        T: Clone + Send + Sync + 'static,
        I: Fn(&T) -> u64 + Send + Sync + 'static,
        K: Fn(&T) -> bool + Send + Sync + 'static,
        O: Fn(&T, &T) -> Ordering + Send + Sync + 'static,
        Snapshot<T>: Send,
    {
        // Views a check made, kept past it, ask for nothing.
        if self.0.inert {
            return;
        }
        let (id, keep, order) = (Arc::new(id), Arc::new(keep), order.map(Arc::new));
        let recipe: Recipe = Rc::new(move |old: View, mut heights: Heights, mappable: bool, request: u64| {
            let (items, id, keep, order) = (items.clone(), Arc::clone(&id), Arc::clone(&keep), order.clone());
            let job: Job = Box::new(move |stale| {
                let order = order.as_deref().map(|o| -> &Order<T> { o });
                let view = View::build_until(&items, &*id, &*keep, order, stale)?;
                let rows = u64::try_from(view.len()).unwrap_or(u64::MAX);
                if mappable {
                    if !heights.reorder_until(rows, |p| old.moved_to(&view, p), stale) {
                        return None;
                    }
                } else {
                    // The list is not laid out as the view shown says: its
                    // heights cannot follow ids, so none is kept.
                    heights.forget(rows);
                }
                (!stale()).then_some(Built { request, view, heights })
            });
            job
        });
        // The request is the running world's (a check's, its copy of the
        // views); the worker is the outside world, which a check never
        // asks: what a check asked for goes with it, as a send does.
        let old = self.0.write(|s| s.recipe.replace(Rc::clone(&recipe)));
        drop(old);
        if crate::engine::forking() {
            return;
        }
        self.0.ask(&recipe);
    }

    /// Why the latest request failed (a closure it gave panicked), until a
    /// view is delivered. A request refused because other views hold the
    /// list's heights is no failure: it is kept, the latest only, and asked
    /// again when they let them go (their views dropped), so a list remounted
    /// with new views before the old ones went still ends sorted.
    #[must_use]
    pub fn failure(&self) -> Option<String> {
        self.0.read(|s| s.failed.clone()).flatten()
    }

    /// Builds that ran to the end (the rest stopped, superseded).
    #[cfg(test)]
    fn finished(&self) -> u64 {
        self.0.shared.finished.load(Atomic::Acquire)
    }
}

/// The worker: the build waiting, if still the latest, then the next.
fn work(shared: &Shared, to: &Sender<Outcome>) {
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
        // A closure the request gave may panic: that request goes, the
        // worker stays (its state is the slot, behind a lock that shrugs
        // off poisoning, and the build's own values, dropped with it).
        match std::panic::catch_unwind(AssertUnwindSafe(|| job(&stale))) {
            Ok(Some(built)) => {
                shared.finished.fetch_add(1, Atomic::AcqRel);
                if !stale() {
                    to.send(Outcome::Built(built)).ok();
                }
            }
            Ok(None) => {}
            Err(panic) => {
                let message = panic
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_owned())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "a closure of the request panicked".to_owned());
                to.send(Outcome::Failed { request, message }).ok();
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
    use std::cell::Cell;

    impl Inner {
        /// The latest request's number, as the running world sees it.
        fn asked(&self) -> u64 {
            self.read(|s| s.asked).unwrap_or_default()
        }

        /// Whether a request is kept to ask again.
        fn has_recipe(&self) -> bool {
            self.read(|s| s.recipe.is_some()).unwrap_or(false)
        }

        /// Whether the worker is still to start.
        fn unstarted(&self) -> bool {
            self.shared.slot().to.is_some()
        }
    }

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

    /// Views for `list` delivering to a vector of what each shows (data rows).
    fn views_into(list: &ListState, len: u32) -> (Views, Rc<RefCell<Vec<Vec<u32>>>>) {
        let shown = Rc::new(RefCell::new(Vec::new()));
        let into = Rc::clone(&shown);
        let views = crate::engine::owned(|| {
            Views::new(list, View::identity(len), move |v: View| {
                into.borrow_mut()
                    .push((0..v.len()).filter_map(|p| v.row(p)).collect::<Vec<_>>());
            })
        });
        (views, shown)
    }

    /// Where a build is held: it says it got there, and waits to be let go.
    #[derive(Default)]
    struct Gate {
        entered: std::sync::atomic::AtomicBool,
        open: std::sync::atomic::AtomicBool,
    }

    impl Gate {
        /// Until a build is held here.
        fn wait_entered(&self) {
            while !self.entered.load(Atomic::SeqCst) {
                std::thread::yield_now();
            }
        }

        fn open(&self) {
            self.open.store(true, Atomic::SeqCst);
        }
    }

    /// A `keep` that holds its build at `gate` until it opens.
    fn held(gate: &Arc<Gate>) -> impl Fn(&u32) -> bool + Send + Sync + 'static {
        let gate = Arc::clone(gate);
        move |_: &u32| {
            gate.entered.store(true, Atomic::SeqCst);
            while !gate.open.load(Atomic::SeqCst) {
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
        let (views, shown) = views_into(&ListState::new(24.0), 3);
        let gate = Arc::new(Gate::default());
        let id = |v: &u32| u64::from(*v);
        views.spawn(c.snapshot(), id, held(&gate), Some(|a: &u32, b: &u32| a.cmp(b)));
        gate.wait_entered();
        views.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| b.cmp(a)));
        gate.open();
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
        let (views, shown) = views_into(&ListState::new(24.0), 100_000);
        let gate = Arc::new(Gate::default());
        let id = |v: &u32| u64::from(*v);
        views.spawn(c.snapshot(), id, held(&gate), Some(|a: &u32, b: &u32| a.cmp(b)));
        // The first build is running, held in its `keep`, when the rest come.
        gate.wait_entered();
        for _ in 0..20 {
            views.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        }
        views.spawn(c.snapshot(), id, |v: &u32| *v < 3, Some(|a: &u32, b: &u32| b.cmp(a)));
        gate.open();
        arrive();
        assert_eq!(*shown.borrow(), [vec![99_997, 99_998, 99_999]], "the latest, alone");
        assert_eq!(views.finished(), 1, "every other build stopped or never ran");
        c.dispose();
    }

    /// Codex and Sonnet (M15 review 2): every stage of a build stops once
    /// it is no longer wanted, the id index's sort too: a sort given up
    /// after its first chunk compares within that chunk only.
    #[test]
    fn a_sort_no_longer_wanted_stops_after_a_chunk() {
        let ids: Vec<(u64, u32)> = (0..100_000_u32).rev().map(|i| (u64::from(i), i)).collect();
        let asked = Cell::new(0_u64);
        let compared = Cell::new(0_u64);
        let stale = || {
            asked.set(asked.get() + 1);
            asked.get() > 1
        };
        let cmp = |a: &(u64, u32), b: &(u64, u32)| {
            compared.set(compared.get() + 1);
            a.cmp(b)
        };
        assert!(sort_until(ids, &cmp, &stale).is_none());
        assert!(compared.get() < 400_000, "{} comparisons", compared.get());
    }

    /// Sonnet and Codex (M15 review 2): the views dropped while a build
    /// runs. The build stops at its next chunk (its ids are never asked
    /// for), and the list's loan ends, so nothing is noted for it.
    #[test]
    fn dropping_the_views_stops_the_running_build_and_ends_the_loan() {
        let c = Collection::new((0..100_000_u32).rev());
        let list = ListState::new(24.0);
        let (views, _) = views_into(&list, 100_000);
        let gate = Arc::new(Gate::default());
        let asked_ids = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&asked_ids);
        let id = move |v: &u32| {
            counted.fetch_add(1, Atomic::SeqCst);
            u64::from(*v)
        };
        views.spawn(c.snapshot(), id, held(&gate), Some(|a: &u32, b: &u32| a.cmp(b)));
        gate.wait_entered();
        assert!(list.is_lent());
        let shared = Arc::clone(&views.0.shared);
        drop(views);
        assert!(!list.is_lent(), "the loan ended");
        gate.open();
        // The worker holds the other count until it ends.
        for _ in 0..10_000 {
            if Arc::strong_count(&shared) == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(Arc::strong_count(&shared), 1, "the worker ended");
        assert_eq!(shared.finished.load(Atomic::SeqCst), 0, "the build stopped");
        assert_eq!(asked_ids.load(Atomic::SeqCst), 0, "before its ids");
        c.dispose();
    }

    /// Codex (F1 review 1): views made, and asked for a sort, in a fork (a
    /// check, ADR-0028) change nothing live: no worker starts, no request
    /// is counted, the list's heights are not lent; the first request
    /// outside a fork starts the worker.
    #[cfg(debug_assertions)]
    #[test]
    fn views_asked_in_a_fork_change_nothing_live() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let (live, shown) = views_into(&list, 3);
        let id = |v: &u32| u64::from(*v);
        let made = {
            let _cold = crate::engine::ColdFrame::begin();
            let (made, _) = views_into(&list, 3);
            made.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
            live.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
            made
        };
        for views in [&made, &live] {
            // Their own two: the views', and their end's ([`Gone`]).
            assert_eq!(Arc::strong_count(&views.0.shared), 2, "no worker started");
            assert!(views.0.unstarted());
            assert_eq!(views.0.asked(), 0, "no request counted");
        }
        assert!(!list.is_lent(), "nothing lent");
        drop(made);
        live.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        assert!(!live.0.unstarted(), "the worker started");
        arrive();
        assert_eq!(*shown.borrow(), [vec![2, 0, 1]]);
        c.dispose();
    }

    /// Codex (F1 review 1): views dropped after their list's owner ended
    /// still tell their worker to stop, before ending the loan (quiet: the
    /// list is gone), and the worker exits.
    #[test]
    fn views_dropped_after_their_list_ended_stop_their_worker() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let (views, _) = views_into(&list, 3);
        let gate = Arc::new(Gate::default());
        let id = |v: &u32| u64::from(*v);
        views.spawn(c.snapshot(), id, held(&gate), Some(|a: &u32, b: &u32| a.cmp(b)));
        gate.wait_entered();
        let shared = Arc::downgrade(&views.0.shared);
        list.dispose();
        drop(views);
        gate.open();
        for _ in 0..10_000 {
            if shared.upgrade().is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(shared.upgrade().is_none(), "the worker exited");
        c.dispose();
    }

    /// Codex (F1 review 2): views made in a fork took no loan, so dropping
    /// them after a live request started leaves its loan as it was, and
    /// the live sort lands.
    #[cfg(debug_assertions)]
    #[test]
    fn dropping_views_made_in_a_fork_leaves_the_live_loan() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let (live, shown) = views_into(&list, 3);
        let made = {
            let _cold = crate::engine::ColdFrame::begin();
            views_into(&list, 3).0
        };
        let gate = Arc::new(Gate::default());
        let id = |v: &u32| u64::from(*v);
        live.spawn(c.snapshot(), id, held(&gate), Some(|a: &u32, b: &u32| a.cmp(b)));
        gate.wait_entered();
        drop(made);
        assert!(list.is_lent(), "the live loan stands");
        gate.open();
        arrive();
        assert_eq!(*shown.borrow(), [vec![2, 0, 1]]);
        c.dispose();
    }

    /// Codex (F1 review 2): the list disposed while a sort is built, the
    /// views kept. Its delivery ends the request for good, quietly: no
    /// panic, nothing asked again, nothing shown.
    #[test]
    fn a_sort_delivered_to_a_disposed_list_ends_its_request() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let (views, shown) = views_into(&list, 3);
        let gate = Arc::new(Gate::default());
        let id = |v: &u32| u64::from(*v);
        views.spawn(c.snapshot(), id, held(&gate), Some(|a: &u32, b: &u32| a.cmp(b)));
        gate.wait_entered();
        list.dispose();
        gate.open();
        arrive();
        assert!(shown.borrow().is_empty());
        assert_eq!(views.0.asked(), 1, "nothing asked again");
        assert!(!views.0.has_recipe(), "the request ended");
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!crate::mailbox::deliver(), "no build sent again");
        assert_eq!(views.finished(), 1);
        c.dispose();
    }

    /// Codex 7 (F1 review 3): a sort whose `order` panics, delivered after
    /// its list was disposed, ends its request too: the recipe (the
    /// snapshot and the closures) goes.
    #[test]
    fn a_failure_delivered_to_a_disposed_list_drops_its_recipe() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let (views, _) = views_into(&list, 3);
        let gate = Arc::new(Gate::default());
        let id = |v: &u32| u64::from(*v);
        views.spawn(
            c.snapshot(),
            id,
            held(&gate),
            Some(|_: &u32, _: &u32| -> Ordering { panic!("no order today") }),
        );
        gate.wait_entered();
        list.dispose();
        gate.open();
        arrive();
        assert!(!views.0.has_recipe(), "the request ended");
        assert_eq!(views.0.asked(), 1, "nothing asked again");
        c.dispose();
    }

    /// Opus (F1 review 3): views made in a fork and kept past its rollback
    /// ask for nothing: no worker, no loan, no claim on the list.
    #[cfg(debug_assertions)]
    #[test]
    fn views_a_fork_made_and_kept_ask_for_nothing() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let made = {
            let _cold = crate::engine::ColdFrame::begin();
            views_into(&list, 3).0
        };
        let id = |v: &u32| u64::from(*v);
        made.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        // Their own two: the views', and their end's ([`Gone`]).
        assert_eq!(Arc::strong_count(&made.0.shared), 2, "no worker started");
        assert_eq!(made.0.asked(), 0, "no request counted");
        assert!(!list.is_lent(), "nothing lent");
        let (live, shown) = views_into(&list, 3);
        live.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        arrive();
        assert_eq!(*shown.borrow(), [vec![2, 0, 1]], "the list is the live views'");
        c.dispose();
    }

    /// Opus (F1 review 3): two views on one list. The second is refused
    /// while the first holds the heights, so the two never void each
    /// other's loans in turn; its request is kept (Opus, F1 review 4: no
    /// failure, the latest asked) and asked again once the first goes.
    #[test]
    fn two_views_on_one_list_take_turns() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let (first, first_shown) = views_into(&list, 3);
        let (second, second_shown) = views_into(&list, 3);
        let id = |v: &u32| u64::from(*v);
        let gate = Arc::new(Gate::default());
        first.spawn(c.snapshot(), id, held(&gate), Some(|a: &u32, b: &u32| a.cmp(b)));
        gate.wait_entered();
        second.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        second.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| b.cmp(a)));
        assert_eq!(second.failure(), None, "refused, waiting: no failure");
        assert_eq!(second.0.asked(), 0, "nothing lent");
        gate.open();
        arrive();
        assert_eq!(*first_shown.borrow(), [vec![2, 0, 1]]);
        assert_eq!(first.0.asked(), 1, "never voided, never asked again");
        drop(first);
        arrive();
        assert_eq!(
            *second_shown.borrow(),
            [vec![1, 0, 2]],
            "the latest request, asked again"
        );
        assert_eq!(second.0.asked(), 1, "once");
        c.dispose();
    }

    /// Codex 4 (F1 review 4): a loan belongs to its holder. Views drop
    /// after a newer loan another holder took: the views end only their
    /// own, and the newer loan, and the rows measured for it, stand.
    #[test]
    fn a_loan_another_holder_took_survives_a_views_drop() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        list.heights_mut(|ext| {
            let units = crate::list::extent_tree::units;
            *ext = crate::list::ExtentTree::new(3, units(24.0));
            ext.set(0, units(24.0));
        });
        let (views, _) = views_into(&list, 3);
        let gate = Arc::new(Gate::default());
        let id = |v: &u32| u64::from(*v);
        views.spawn(c.snapshot(), id, held(&gate), Some(|a: &u32, b: &u32| a.cmp(b)));
        gate.wait_entered();
        let mut newer = list.lend_heights();
        list.adjust_row_height(0, 16.0);
        drop(views);
        assert!(list.is_lent(), "the newer loan stands");
        newer.reorder(3, |p| Some(2 - p));
        assert!(
            list.reorder_with(newer, 3, |p| Some(2 - p)).is_ok(),
            "and is taken back"
        );
        assert_eq!(list.row_height(2), 40.0, "with the row measured for it");
        gate.open();
        c.dispose();
    }

    /// F1 review 4: a list remounted with new views, which ask before the
    /// old views' scope is disposed. The new ones wait, then sort once the
    /// old ones let the heights go: the list ends sorted, with no failure.
    #[test]
    fn views_remounted_before_the_old_ones_go_end_sorted() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let id = |v: &u32| u64::from(*v);
        let (old, _) = views_into(&list, 3);
        old.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| b.cmp(a)));
        arrive();
        let (new, shown) = views_into(&list, 3);
        new.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        assert_eq!(new.failure(), None);
        drop(old);
        arrive();
        assert_eq!(*shown.borrow(), [vec![2, 0, 1]], "sorted");
        assert_eq!(new.failure(), None, "no error");
        c.dispose();
    }

    /// Codex 4 (F1 review 5): the last clone of live views dropped in a
    /// fork. Their worker is not closed in it (no outside-world effect a
    /// rollback cannot take back); once the fork has rolled back they are
    /// released whole, live: the worker closes, the claim goes, and the
    /// views waiting on it are woken and sort.
    #[cfg(debug_assertions)]
    #[test]
    fn live_views_a_fork_drops_are_released_after_it_and_wake_the_waiting() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let id = |v: &u32| u64::from(*v);
        let (old, _) = views_into(&list, 3);
        old.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| b.cmp(a)));
        arrive();
        let (new, shown) = views_into(&list, 3);
        new.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        assert_eq!(new.0.asked(), 0, "refused: the old views hold the heights");
        let shared = Arc::clone(&old.0.shared);
        {
            let _cold = crate::engine::ColdFrame::begin();
            drop(old);
            assert!(!shared.slot().closed, "the live worker is not closed in the fork");
        }
        assert!(!shared.slot().closed, "the release is live work, not the fork's");
        crate::engine::run_releases();
        assert!(shared.slot().closed, "closed by the live loop after the fork");
        arrive();
        assert_eq!(*shown.borrow(), [vec![2, 0, 1]], "the waiting views sort");
        assert_eq!(new.failure(), None);
        c.dispose();
    }

    /// ADR-0034, S4: a check that clones the last live handle and then
    /// faults (its clone forgotten, as a runaway's set-aside does)
    /// leaves no obligation pending: the live handle's drop closes the
    /// worker, lets the claim go and wakes the views waiting on it.
    #[cfg(debug_assertions)]
    #[test]
    #[expect(clippy::mem_forget, reason = "a runaway's set-aside clone, as the check makes it")]
    fn a_checks_forgotten_clone_of_live_views_holds_no_obligation() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let id = |v: &u32| u64::from(*v);
        let (old, _) = views_into(&list, 3);
        old.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| b.cmp(a)));
        arrive();
        let (new, shown) = views_into(&list, 3);
        new.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        assert_eq!(new.0.asked(), 0, "refused: the old views hold the heights");
        {
            let _cold = crate::engine::ColdFrame::begin();
            std::mem::forget(old.clone());
        }
        let shared = Arc::clone(&old.0.shared);
        drop(old);
        assert!(shared.slot().closed, "the live owner's end closed the worker");
        assert_eq!(crate::engine::releases_pending(), 0, "nothing left pending");
        arrive();
        assert_eq!(*shown.borrow(), [vec![2, 0, 1]], "the waiting views sort");
        c.dispose();
    }

    /// A value whose drop panics: what makes a check's teardown fault.
    #[cfg(debug_assertions)]
    struct Boom;

    #[cfg(debug_assertions)]
    impl Drop for Boom {
        fn drop(&mut self) {
            panic!("a boom's drop");
        }
    }

    crate::engine::engine_local! {
        /// What a check parks, opaque: a boom first, then what it shares.
        #[cfg(debug_assertions)]
        [Fresh] static PARKED: RefCell<Vec<Box<dyn std::any::Any>>> = RefCell::new(Vec::new());
    }

    /// Opus r15 I1 (ADR-0034, S4): live views held behind an `Rc` the app
    /// shares (a rebuild closure cloned into a handler on every render, as
    /// stress/scenes.rs does). A check clones the `Rc`, not the views, so
    /// its clone shares the live handle; it parks it in an opaque value
    /// after a boom, and its teardown faults at the boom. Since review r16
    /// the disposal goes on past the fault, so the check's clone is
    /// dropped (S2). A runaway would still set such a clone aside, so the
    /// views' obligations end with their live owner (the scope that made
    /// them), not with the last count of the handle: pinned as a runaway
    /// pins them, as that scope ends, the worker closes, the claim on the
    /// heights goes, and the views waiting on it wake and sort.
    #[cfg(debug_assertions)]
    #[test]
    #[expect(clippy::mem_forget, reason = "a runaway's set-aside clone, as the check makes it")]
    fn live_views_a_faulted_check_pinned_end_with_their_owner() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let id = |v: &u32| u64::from(*v);
        let owner = crate::reactive::Scope::new();
        let old = Rc::new(owner.run(|| views_into(&list, 3).0));
        old.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| b.cmp(a)));
        arrive();
        let (new, shown) = views_into(&list, 3);
        new.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        assert_eq!(new.0.asked(), 0, "refused: the old views hold the heights");
        let shared = Arc::clone(&old.0.shared);
        let parked = Rc::clone(&old);
        let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::engine::in_cold_frame(|| {
                PARKED.with_borrow_mut(|v| {
                    v.push(Box::new(Boom));
                    v.push(Box::new(Rc::clone(&parked)));
                });
            });
        }));
        assert!(p.is_err(), "the check faults at the boom");
        assert_eq!(
            Rc::strong_count(&old),
            2,
            "the check's clone was dropped past the fault"
        );
        std::mem::forget(Rc::clone(&old));
        drop((parked, old));
        crate::engine::run_releases();
        assert!(!shared.slot().closed, "pinned by the set-aside clone");
        owner.dispose();
        crate::engine::run_releases();
        assert!(shared.slot().closed, "the owner's end closed the worker");
        arrive();
        assert_eq!(*shown.borrow(), [vec![2, 0, 1]], "the waiting views sort");
        assert_eq!(new.failure(), None);
        c.dispose();
    }

    /// ADR-0015 (a window owns what is made in it), F1's handoff: views
    /// made with no scope, in a window, that a runaway check pinned (a
    /// clone parked in an opaque value it set aside) end as their window
    /// closes: the worker closes and the views waiting on the heights wake.
    #[cfg(debug_assertions)]
    #[test]
    #[expect(clippy::mem_forget, reason = "a runaway's set-aside clone, as the check makes it")]
    fn scope_less_views_a_faulted_check_pinned_end_with_their_window() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let id = |v: &u32| u64::from(*v);
        let mut window = crate::app::Pipeline::new(|| crate::element::Element::Empty, false);
        let mut made = None;
        window.act("make", || made = Some(Rc::new(views_into(&list, 3).0)));
        let old = made.expect("made");
        old.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| b.cmp(a)));
        arrive();
        // The waiting views are another window's: its close leaves them.
        let mut other = crate::app::Pipeline::new(|| crate::element::Element::Empty, false);
        let mut waiting = None;
        other.act("make", || waiting = Some(views_into(&list, 3)));
        let (new, shown) = waiting.expect("made");
        new.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        assert_eq!(new.0.asked(), 0, "refused: the old views hold the heights");
        let shared = Arc::clone(&old.0.shared);
        // Since review r16 a fault no longer sets values aside, only a
        // runaway does: pinned as a runaway's set-aside clone pins them.
        std::mem::forget(Rc::clone(&old));
        drop(old);
        assert!(!shared.slot().closed, "pinned by the set-aside clone");
        assert_eq!(window.close(), Ok(()));
        crate::engine::run_releases();
        assert!(shared.slot().closed, "the window's close closed the worker");
        arrive();
        assert_eq!(*shown.borrow(), [vec![2, 0, 1]], "the waiting views sort");
        assert_eq!(new.failure(), None);
        c.dispose();
    }

    /// ADR-0034, V5, the vertical slice: a real check of a virtual-list
    /// page (the pipeline's own, its layout, paint and comparison), a row
    /// of which panics once the fork has written, so the check ends part
    /// way and the switch puts the live world back with no destructor run
    /// (the tripwire would fault it). The page holds views with a worker
    /// and views waiting on its claim. From then on the engine that checked
    /// does as one that never did: the next input and what it shows, the
    /// next live ids, the worker's next view, its close as its owner drops
    /// it, and the waiting views' wake.
    #[cfg(debug_assertions)]
    #[test]
    fn a_virtual_list_check_interrupted_after_its_first_write_leaves_the_engine_as_one_never_checked() {
        fn run(check: bool) -> Vec<String> {
            use crate::app::Scenario;
            use crate::element::{IntoElement, div, text};
            use crate::list::virtual_list;
            use crate::styled::Styled;
            const ROWS: u32 = 1_000;
            static KIND: crate::engine::IdKind = crate::engine::IdKind::new("v5 slice");
            let first_id = crate::engine::unique_id(&KIND);
            let c = Collection::new((0..ROWS).map(|v| v.wrapping_mul(7) % ROWS));
            let list = ListState::new(24.0);
            let id = |v: &u32| u64::from(*v);
            let view = crate::Signal::new(View::identity(ROWS));
            let old = crate::engine::owned(|| Views::new(&list, View::identity(ROWS), move |v: View| view.set(v)));
            old.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| b.cmp(a)));
            arrive();
            let (new, waiting) = views_into(&list, ROWS);
            new.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
            assert_eq!(new.0.asked(), 0, "waiting: the old views hold the heights");
            // Armed with the fork's work so far: a row panics once the
            // check has written past it.
            let trip: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
            let (st, armed) = (list, Rc::clone(&trip));
            let ui = move || {
                let armed = Rc::clone(&armed);
                let row = move |i: u64| {
                    if let Some(at) = armed.get()
                        && crate::engine::forking()
                        && crate::engine::fork_work_done() > at
                    {
                        armed.set(None);
                        panic!("a row of the check fails");
                    }
                    let shown = view.with(|v| v.row(usize::try_from(i).unwrap_or(usize::MAX)));
                    div().height(24.0).child(text(format!("row {shown:?}"))).into_element()
                };
                div()
                    .width(300.0)
                    .child(virtual_list(&st, u64::from(ROWS), |i| i, row).height(400.0))
                    .into_element()
            };
            let mut s = Scenario::measuring(ui, 300, 400, 1.0).unwrap();
            s.check_cold(false);
            s.settle().unwrap();
            let mut seen = Vec::new();
            if check {
                trip.set(Some(crate::engine::fork_work_done()));
                let p = std::panic::catch_unwind(AssertUnwindSafe(|| s.check_now())).unwrap_err();
                assert_eq!(
                    p.downcast_ref::<String>().map(String::as_str),
                    Some("a row of the check fails"),
                    "the row's panic, as a string, and no fault"
                );
                assert_eq!(trip.get(), None, "interrupted after the fork wrote");
                // Not poisoned: the next check runs whole, and agrees.
                s.check_now();
            }
            seen.push(format!("pending {}", crate::engine::releases_pending()));
            seen.push(format!(
                "id +{}",
                crate::engine::unique_id(&KIND).wrapping_sub(first_id)
            ));
            seen.push(format!("node {:?}", crate::Signal::new(0_u8).node_id()));
            s.hover(150.0, 200.0);
            seen.push(format!("wheel {}", s.wheel(480.0)));
            s.settle().unwrap();
            seen.push(format!("top {:?} {:?}", list.view_top(), list.window()));
            seen.push(crate::inspect::dump(&s.pipeline().frame().nodes));
            // The worker's next view.
            old.spawn(
                c.snapshot(),
                id,
                |v: &u32| v.is_multiple_of(2),
                Some(|a: &u32, b: &u32| b.cmp(a)),
            );
            arrive();
            seen.push(format!("view {:?}", view.with(|v| (v.len(), v.row(0), v.row(1)))));
            // Its owner drops it: the worker closes and the waiting wake.
            let shared = Arc::clone(&old.0.shared);
            drop(old);
            seen.push(format!("closed {}", shared.slot().closed));
            arrive();
            seen.push(format!(
                "woken {:?} {:?}",
                waiting
                    .borrow()
                    .iter()
                    .map(|v| (v.len(), v.first().copied()))
                    .collect::<Vec<_>>(),
                new.failure()
            ));
            s.settle().unwrap();
            seen.push(crate::inspect::dump(&s.pipeline().frame().nodes));
            seen.push(format!("pending {}", crate::engine::releases_pending()));
            drop((new, s));
            c.dispose();
            seen
        }
        let never = std::thread::spawn(|| run(false)).join().unwrap();
        let checked = std::thread::spawn(|| run(true)).join().unwrap();
        assert_eq!(checked, never);
    }

    /// Codex 5 (F1 review 6): an action the fork left to run live that
    /// panics loses none after it: the views' release still runs, and the
    /// waiting views sort; the panic is raised once all have run.
    #[cfg(debug_assertions)]
    #[test]
    fn a_panicking_live_action_still_lets_the_views_release_run() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let id = |v: &u32| u64::from(*v);
        let (old, _) = views_into(&list, 3);
        old.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| b.cmp(a)));
        arrive();
        let (new, shown) = views_into(&list, 3);
        new.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        {
            let _cold = crate::engine::ColdFrame::begin();
            crate::engine::when_live(|| panic!("an action fails"));
            drop(old);
        }
        // Live work, run by the live loop after the fork (F1 review 13).
        let ended = std::panic::catch_unwind(crate::engine::run_releases);
        assert!(ended.is_err(), "the panic is raised");
        arrive();
        assert_eq!(
            *shown.borrow(),
            [vec![2, 0, 1]],
            "the release ran: the waiting views sort"
        );
        c.dispose();
    }

    /// Sonnet (M15 review 2): a request whose `order` panics is dropped and
    /// said to have failed; the worker serves the next.
    #[test]
    fn a_panicking_order_fails_its_request_and_the_worker_serves_the_next() {
        let c = Collection::new([2_u32, 3, 1]);
        let (views, shown) = views_into(&ListState::new(24.0), 3);
        let id = |v: &u32| u64::from(*v);
        views.spawn(
            c.snapshot(),
            id,
            |_: &u32| true,
            Some(|_: &u32, _: &u32| -> Ordering { panic!("no order today") }),
        );
        arrive();
        assert_eq!(views.failure().as_deref(), Some("no order today"));
        assert!(shown.borrow().is_empty());
        views.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
        arrive();
        assert_eq!(*shown.borrow(), [vec![2, 0, 1]]);
        assert_eq!(views.failure(), None);
        c.dispose();
    }

    /// Codex (M15 review 2): the list spliced by its owner while a sort is
    /// built. The result is refused and the sort asked again on a fresh
    /// loan; the list is no longer laid out as the view shown says, so no
    /// height is moved by it.
    #[test]
    fn a_sort_refused_after_a_splice_is_built_again() {
        let c = Collection::new([2_u32, 3, 1]);
        let list = ListState::new(24.0);
        let (views, shown) = views_into(&list, 3);
        let gate = Arc::new(Gate::default());
        let id = |v: &u32| u64::from(*v);
        views.spawn(c.snapshot(), id, held(&gate), Some(|a: &u32, b: &u32| a.cmp(b)));
        gate.wait_entered();
        list.splice(0, 0, 3);
        gate.open();
        arrive();
        assert!(shown.borrow().is_empty(), "refused");
        assert!(list.is_lent(), "asked again");
        arrive();
        assert_eq!(*shown.borrow(), [vec![2, 0, 1]]);
        assert_eq!(views.finished(), 2);
        assert!(!list.is_measured(0) && !list.is_measured(2));
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
