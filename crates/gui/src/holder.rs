//! What a mounted part gives a shared owner, and the identity that takes it
//! back (Compose's `DisposableEffect`, as a value).
//!
//! A part's setup must not write what it did not make (it is a render), so
//! a part registers with its owner as it mounts: [`while_mounted`] runs a step
//! once the setup has returned, and its undo as the part unmounts. One
//! owner slot shared by every part that may give it a value is a [`Holder`]:
//! the part mounted last holds it, and a part leaving takes back only what
//! is still its own, by the [`Registration`] the slot issued it, never by
//! the value it gave (two mounts may give the same `Rc`, the same list
//! state, the same focus handle). A keyed part replaced in one update
//! mounts its new instance before the old one leaves: the old one's leaving
//! finds the slot is no longer its own and leaves it be.

use std::cell::Cell;
use std::collections::HashMap;

use crate::engine::WorldCell;
use std::fmt;
use std::rc::Rc;

use crate::reactive::{Effect, Signal, on_cleanup, untracked};

/// The identity of one registration: issued by the engine, unique on its
/// thread, never by the caller's data.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Registration(u64);

impl Registration {
    /// A registration no other is equal to.
    #[must_use]
    #[expect(clippy::new_without_default, reason = "each one is new: no default one")]
    pub fn new() -> Self {
        static ISSUED: crate::engine::IdKind = crate::engine::IdKind::new("registration");
        Self(crate::engine::unique_id(&ISSUED))
    }
}

/// Identities run out: a bug (2^64 issued), reported rather than reused.
#[expect(clippy::panic, reason = "an identity issued twice would be a silent bug")]
fn exhausted(what: &str) -> ! {
    panic!("gui: the {what} issued ran out of identities")
}

/// Run `register` once the calling setup has returned (as an effect: a
/// setup, as a render, writes only what it made), untracked, and the undo
/// it returns as the calling scope ends. A setup that never mounts (a cold
/// re-render's) runs neither; a mount that ran it always runs its undo.
#[track_caller]
pub fn while_mounted<U: FnOnce() + 'static>(register: impl FnOnce() -> U + 'static) {
    let register = Cell::new(Some(register));
    // Untracked, it reads nothing: it runs once, and its cleanup runs as it
    // is disposed with the scope.
    let _mounted = Effect::new(move || {
        let Some(register) = register.take() else {
            return;
        };
        // The owner learns of the registration before anything is
        // published: its teardown is installed first, so an observer the
        // registering notifies that ends the owner (disposes the scope)
        // finds it, and the undo runs as the register returns.
        let undo: Rc<Cell<Option<U>>> = Rc::new(Cell::new(None));
        let gone = Rc::new(Cell::new(false));
        let (u, g) = (Rc::clone(&undo), Rc::clone(&gone));
        on_cleanup(move || {
            g.set(true);
            if let Some(undo) = u.take() {
                undo();
            }
        });
        // One batch from the side effect to the record of its undo: what
        // the registering notifies runs as the batch ends, once the undo is
        // the owner's, so an observer that panics leaves nothing behind.
        crate::reactive::batch(|| {
            let made = untracked(register);
            if gone.get() {
                made();
            } else {
                undo.set(Some(made));
            }
        });
    });
}

/// A value one part at a time gives its owner: the part that set it last
/// holds it (tracked reads), until it clears it with its [`Registration`].
pub struct Holder<T: 'static> {
    held: Signal<Option<(Registration, T)>>,
}

impl<T: 'static> Clone for Holder<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Holder<T> {}

impl<T: 'static> PartialEq for Holder<T> {
    fn eq(&self, other: &Self) -> bool {
        self.held == other.held
    }
}

impl<T: 'static> fmt::Debug for Holder<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Holder").finish_non_exhaustive()
    }
}

impl<T: 'static> Holder<T> {
    /// An empty slot, owned by the running scope (made where its owner is).
    #[must_use]
    #[track_caller]
    #[expect(
        clippy::new_without_default,
        reason = "made where its owner is, as a signal: no default one"
    )]
    pub fn new() -> Self {
        Self {
            held: Signal::new(None),
        }
    }

    /// Hold `value` from now on: what the slot held before is let go. The
    /// registration returned takes it back ([`Self::clear`]).
    pub fn set(self, value: T) -> Registration {
        let r = Registration::new();
        self.held.update(|h| *h = Some((r, value)));
        r
    }

    /// Let go of what `r` set, if the slot still holds it; `false` (and
    /// nothing written) when a later [`Self::set`] took the slot, or when
    /// the slot is gone.
    pub fn clear(self, r: Registration) -> bool {
        if !self.held.is_alive() || !self.held.with_untracked(|h| h.as_ref().is_some_and(|(by, _)| *by == r)) {
            return false;
        }
        self.held.update(|h| *h = None);
        true
    }

    /// Hold `value` while the calling setup's instance is mounted: set as
    /// it mounts ([`while_mounted`]), cleared as it unmounts unless another part
    /// set the slot since.
    #[track_caller]
    pub fn hold(self, value: T) {
        while_mounted(move || {
            let r = self.set(value);
            move || {
                self.clear(r);
            }
        });
    }

    /// Whether the slot holds a value (tracked).
    #[must_use]
    pub fn is_held(self) -> bool {
        self.held.with(Option::is_some)
    }

    /// Read what it holds (tracked).
    pub fn with<R>(self, f: impl FnOnce(Option<&T>) -> R) -> R {
        self.held.with(|h| f(h.as_ref().map(|(_, v)| v)))
    }

    /// Read what it holds (untracked); `None` once the slot is gone.
    pub fn with_untracked<R>(self, f: impl FnOnce(Option<&T>) -> R) -> R {
        if !self.held.is_alive() {
            return f(None);
        }
        self.held.with_untracked(|h| f(h.as_ref().map(|(_, v)| v)))
    }

    /// A copy of what it holds (tracked).
    #[must_use]
    pub fn get(self) -> Option<T>
    where
        T: Clone,
    {
        self.with(|v| v.cloned())
    }

    /// A copy of what it holds (untracked).
    #[must_use]
    pub fn get_untracked(self) -> Option<T>
    where
        T: Clone,
    {
        self.with_untracked(|v| v.cloned())
    }

    /// Take the slot out of the graph (a detached one, from its owner's
    /// `Drop`).
    pub(crate) fn dispose_quietly(self) {
        self.held.dispose_quietly();
    }

    /// Whether the slot itself is still alive (its owner's scope has not
    /// ended).
    #[must_use]
    pub fn is_alive(self) -> bool {
        self.held.is_alive()
    }
}

/// What the mounted parts of one kind give their owner, in the order they
/// registered: a part is in it while it is mounted, and only then. The one
/// way in is [`Self::register`] (as the part mounts, [`while_mounted`]):
/// a setup never pushes into a registry another part owns, so a setup that
/// never mounts (a cold re-render's) leaves it as it was, and a part leaving
/// takes out its own entry alone, by its [`Registration`].
pub struct Registry<T: 'static> {
    entries: Signal<Entries<T>>,
}

/// No slot: the end of the order.
const NIL: usize = usize::MAX;

/// One slot of the slab: its entry while it holds one, its generation (a
/// key from before the slot was reused finds another), and its neighbours
/// in the order the entries registered.
struct Slot<T> {
    value: Option<T>,
    generation: u64,
    prev: usize,
    next: usize,
    /// Taken out while a read was under way: every read skips it, and it
    /// stays linked (a walk standing on it steps on to the next) until the
    /// last read ends, which unlinks it.
    gone: WorldCell<bool>,
}

/// A registration's place: its slot, and the generation it holds there.
#[derive(Clone, Copy)]
struct Key {
    index: usize,
    generation: u64,
}

/// The entries: a slab whose free slots are reused, linked in the order
/// they registered (an intrusive list): adding one, taking one out by its
/// key and stepping to the next are each a few slots' work.
///
/// A signal's value, written by `update` (which a check is refused on a
/// live signal) and, while read, through `&`: those cells are the world's
/// that made the registry ([`WorldCell`]), so a check reading a live
/// registry counts no read, queues no removal and adds no visit to it
/// (review r18, Codex nit, Opus plausible 1).
struct Entries<T> {
    slots: Vec<Slot<T>>,
    free: Vec<usize>,
    head: usize,
    tail: usize,
    /// The slots linked, those taken out while read included.
    linked: usize,
    /// Of those, the ones taken out while read ([`Slot::gone`]).
    gone: WorldCell<usize>,
    /// Reads under way ([`Registry::with`], [`Registry::walk`]): a removal meanwhile
    /// waits for them.
    readers: WorldCell<u32>,
    /// The removals that waited, applied as the last read ends.
    queued: WorldCell<Vec<Key>>,
    /// Slots stepped through, read or written: the measure of the work.
    visits: WorldCell<u64>,
}

impl<T> Entries<T> {
    /// One slot stepped through: counted in the registry's own world (a
    /// check reading a live one leaves its count).
    fn visit(&self) {
        if self.visits.is_here() {
            self.visits.set(self.visits.get().saturating_add(1));
        }
    }

    /// Whether the running world made it: may count a read and queue a
    /// removal through `&`.
    fn is_here(&self) -> bool {
        self.readers.is_here()
    }

    fn slot(&self, i: usize) -> Option<&Slot<T>> {
        let s = self.slots.get(i)?;
        self.visit();
        Some(s)
    }

    fn slot_mut(&mut self, i: usize) -> Option<&mut Slot<T>> {
        let s = self.slots.get_mut(i)?;
        self.visits.set(self.visits.get().saturating_add(1));
        Some(s)
    }

    fn insert(&mut self, value: T) -> Key {
        let tail = self.tail;
        let (index, generation) = match self.free.pop() {
            Some(index) => match self.slot_mut(index) {
                Some(s) => {
                    s.value = Some(value);
                    s.prev = tail;
                    s.next = NIL;
                    (index, s.generation)
                }
                None => (index, 0),
            },
            None => {
                self.slots.push(Slot {
                    value: Some(value),
                    generation: 0,
                    prev: tail,
                    next: NIL,
                    gone: WorldCell::new("a registry slot's removal mark (`holder::Slot::gone`)", false),
                });
                self.visit();
                (self.slots.len().saturating_sub(1), 0)
            }
        };
        match self.slot_mut(tail) {
            Some(t) => t.next = index,
            None => self.head = index,
        }
        self.tail = index;
        self.linked = self.linked.saturating_add(1);
        Key { index, generation }
    }

    /// Whether `key` holds an entry still.
    fn holds(&self, key: Key) -> bool {
        self.slot(key.index)
            .is_some_and(|s| s.generation == key.generation && s.value.is_some())
    }

    /// Take `key`'s entry out while a read is under way: skipped from now
    /// on, unlinked as the last read ends.
    fn take_out_later(&self, key: Key) {
        let Some(s) = self.slot(key.index) else { return };
        if s.generation != key.generation || s.value.is_none() || s.gone.replace(true) {
            return;
        }
        self.gone.set(self.gone.get().saturating_add(1));
        let mut queued = self.queued.take();
        queued.push(key);
        self.queued.set(queued);
    }

    fn remove(&mut self, key: Key) {
        let Some(s) = self.slot_mut(key.index) else {
            return;
        };
        if s.generation != key.generation || s.value.is_none() {
            return;
        }
        s.value = None;
        s.generation = s
            .generation
            .checked_add(1)
            .unwrap_or_else(|| exhausted("registry slots"));
        let was_gone = s.gone.replace(false);
        let (prev, next) = (s.prev, s.next);
        match self.slot_mut(prev) {
            Some(p) => p.next = next,
            None => self.head = next,
        }
        match self.slot_mut(next) {
            Some(n) => n.prev = prev,
            None => self.tail = prev,
        }
        self.free.push(key.index);
        self.linked = self.linked.saturating_sub(1);
        if was_gone {
            self.gone.set(self.gone.get().saturating_sub(1));
        }
    }

    fn len(&self) -> usize {
        self.linked.saturating_sub(self.gone.get())
    }

    fn iter(&self) -> Iter<'_, T> {
        Iter {
            entries: Some(self),
            at: self.head,
        }
    }
}

/// The entries of a [`Registry`], in the order they registered.
pub struct Iter<'a, T> {
    entries: Option<&'a Entries<T>>,
    at: usize,
}

impl<T> Clone for Iter<'_, T> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries,
            at: self.at,
        }
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        let e = self.entries?;
        loop {
            let s = e.slot(self.at)?;
            self.at = s.next;
            if !s.gone.get()
                && let Some(v) = s.value.as_ref()
            {
                return Some(v);
            }
        }
    }
}

impl<T: 'static> Clone for Registry<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Registry<T> {}

impl<T: 'static> PartialEq for Registry<T> {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl<T: 'static> fmt::Debug for Registry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry").finish_non_exhaustive()
    }
}

/// A read of a registry under way: removals wait until the last one ends.
/// A check's read of a live registry is not counted: no removal of the
/// check's reaches it (its `update` is refused), so none waits for it.
struct Reading<T: 'static> {
    registry: Registry<T>,
    counted: bool,
}

impl<T: 'static> Reading<T> {
    fn begin(r: Registry<T>) -> Option<Self> {
        if !r.entries.is_alive() {
            return None;
        }
        let counted = r.entries.with_untracked(|e| {
            let here = e.is_here();
            if here {
                e.readers.set(e.readers.get().saturating_add(1));
            }
            here
        });
        Some(Self { registry: r, counted })
    }
}

impl<T: 'static> Drop for Reading<T> {
    fn drop(&mut self) {
        let entries = self.registry.entries;
        if !self.counted || !entries.is_alive() {
            return;
        }
        let due = entries.with_untracked(|e| {
            let left = e.readers.get().saturating_sub(1);
            e.readers.set(left);
            if left == 0 { e.queued.take() } else { Vec::new() }
        });
        if due.is_empty() {
            return;
        }
        let apply = move || entries.update(|e| due.into_iter().for_each(|k| e.remove(k)));
        if std::thread::panicking() {
            // What its writing runs may panic: never raised over the panic
            // going on.
            drop(std::panic::catch_unwind(std::panic::AssertUnwindSafe(apply)));
        } else {
            apply();
        }
    }
}

impl<T: 'static> Registry<T> {
    /// An empty registry, owned by the running scope (made where its owner
    /// is).
    #[must_use]
    #[track_caller]
    #[expect(
        clippy::new_without_default,
        reason = "made where its owner is, as a signal: no default one"
    )]
    pub fn new() -> Self {
        Self {
            entries: Signal::new(Entries {
                slots: Vec::new(),
                free: Vec::new(),
                head: NIL,
                tail: NIL,
                linked: 0,
                gone: WorldCell::new("a registry's removal count (`holder::Entries::gone`)", 0),
                readers: WorldCell::new("a registry's read count (`holder::Entries::readers`)", 0),
                queued: WorldCell::new("a registry's waiting removals (`holder::Entries::queued`)", Vec::new()),
                visits: WorldCell::new("a registry's work count (`holder::Entries::visits`)", 0),
            }),
        }
    }

    /// Give it `value` while the calling setup's instance is mounted: added
    /// as it mounts, taken out as it unmounts (by its slot, at once, or as
    /// the reads under way end).
    #[track_caller]
    pub fn register(self, value: T) {
        while_mounted(move || {
            let mut key = None;
            if self.entries.is_alive() {
                self.entries.update(|e| key = Some(e.insert(value)));
            }
            move || {
                if let Some(key) = key {
                    self.take_out(key);
                }
            }
        });
    }

    /// Take `key`'s entry out: at once, or, while a read is under way,
    /// skipped from now on and unlinked as the last read ends. Never
    /// panics, never leaves the entry behind.
    fn take_out(self, key: Key) {
        if !self.entries.is_alive() {
            return;
        }
        let waits = self.entries.with_untracked(|e| {
            // Another world's registry: its write below is refused, as
            // every write to another world's signal is.
            if !e.is_here() {
                return false;
            }
            let reading = e.readers.get() > 0;
            if reading {
                e.take_out_later(key);
            }
            reading || !e.holds(key)
        });
        if !waits {
            self.entries.update(|e| e.remove(key));
        }
    }

    /// Read what the mounted parts gave, in the order they registered
    /// (tracked); nothing once the registry is gone. A part leaving
    /// meanwhile takes its entry out as the read ends.
    pub fn with<R>(self, f: impl FnOnce(Iter<'_, T>) -> R) -> R {
        let Some(_reading) = Reading::begin(self) else {
            return f(Iter { entries: None, at: NIL });
        };
        self.entries.with(|e| f(e.iter()))
    }

    /// [`Self::with`], untracked.
    pub fn with_untracked<R>(self, f: impl FnOnce(Iter<'_, T>) -> R) -> R {
        let Some(_reading) = Reading::begin(self) else {
            return f(Iter { entries: None, at: NIL });
        };
        self.entries.with_untracked(|e| f(e.iter()))
    }

    /// Walk the entries in order, `f` given a copy of one at a time
    /// (untracked) until it says to stop (`false`). No borrow is held while
    /// `f` runs: it may take entries out, the current one included (the walk
    /// still reaches each one after it), read the registry, or end its
    /// owner (the walk then ends). The walk is the call: it cannot be kept
    /// past it, and what was taken out during it is unlinked as the last
    /// read ends. Each step is a few slots' work.
    pub fn walk(self, mut f: impl FnMut(T) -> bool)
    where
        T: Clone,
    {
        let Some(_reading) = Reading::begin(self) else {
            return;
        };
        let mut at: Option<Key> = None;
        loop {
            if !self.entries.is_alive() {
                return;
            }
            let found = self.entries.with_untracked(|e| {
                let mut i = match at {
                    None => e.head,
                    Some(k) => e
                        .slot(k.index)
                        .filter(|s| s.generation == k.generation)
                        .map_or(NIL, |s| s.next),
                };
                loop {
                    let s = e.slot(i)?;
                    if !s.gone.get()
                        && let Some(v) = s.value.as_ref()
                    {
                        return Some((
                            Key {
                                index: i,
                                generation: s.generation,
                            },
                            v.clone(),
                        ));
                    }
                    i = s.next;
                }
            });
            let Some((key, value)) = found else { return };
            at = Some(key);
            if !f(value) {
                return;
            }
        }
    }

    /// The slots it keeps linked, entries taken out during a read and not
    /// yet unlinked included: none once every part left and no read is
    /// under way.
    #[must_use]
    pub fn retained(self) -> usize {
        if !self.entries.is_alive() {
            return 0;
        }
        self.entries.with_untracked(|e| e.linked)
    }

    /// How many entries it holds (untracked).
    #[must_use]
    pub fn len_untracked(self) -> usize {
        if !self.entries.is_alive() {
            return 0;
        }
        self.entries.with_untracked(Entries::len)
    }

    /// A copy of every entry (tracked).
    #[must_use]
    pub fn to_vec_tracked(self) -> Vec<T>
    where
        T: Clone,
    {
        self.with(|it| it.cloned().collect())
    }

    /// A copy of every entry (untracked).
    #[must_use]
    pub fn to_vec_untracked(self) -> Vec<T>
    where
        T: Clone,
    {
        self.with_untracked(|it| it.cloned().collect())
    }

    /// The slots its reads and writes have stepped through so far: the
    /// measure of its work, for tests of what it costs.
    #[must_use]
    pub fn visits(self) -> u64 {
        if !self.entries.is_alive() {
            return 0;
        }
        self.entries.with_untracked(|e| e.visits.get())
    }
}

/// The identities an owner issues its parts by key (a row's focus by the
/// row's key, a tab's nodes by its value): each made by the engine's
/// counters ([`crate::FocusHandle::new`], [`crate::NodeRef::new`]) the
/// first time its key is asked for, the same while the key is kept, and a
/// new one for a key asked for again after it was let go, so a handle
/// saved from a key that left never reaches what came in its place. Keys
/// are told apart by their equality, never by a digest of them. The
/// owner's own state, made in its setup and gone with it.
///
/// A key is kept while any of its users holds it ([`Self::hold`]: a tab's
/// trigger and its panel share their value's nodes, kept while either is
/// on screen), or, for an owner that knows when its keys leave, until it
/// lets go of them ([`Self::retain`]).
pub struct Keyed<K: 'static, V: 'static> {
    /// Its memo in [`MEMOS`].
    id: crate::engine::SlabId,
    ty: std::marker::PhantomData<fn() -> (K, V)>,
}

/// What a [`Keyed`] remembers: not reactive state, since nothing reads it
/// tracked (an answer for a key never changes while it is kept).
struct Memo<K, V> {
    map: Layered<K, Entry<V>>,
    /// Map operations so far: the measure of the work, for tests.
    work: u64,
    /// The next entry's generation: a hold is of one entry, not of a key.
    next: u64,
}

/// A map whose copy costs O(1) and whose writes after a copy cost what
/// they change: a fork's first write to a memo saves a copy of it
/// ([`MEMOS`], ADR-0028 rule 4), and a memo of a million keys must not be
/// copied for one insert (Codex 7, F1 review 6). A shared base, and the
/// writes since in a layer of their own (`None`: taken out). The layer is
/// folded into the base once it holds [`LAYER`] writes and nothing shares
/// either, which is always so live (a write costs amortised O(1)). The
/// persistent structures' path copying (Driscoll et al., 1989), at the one
/// level a hash map needs.
///
/// A check's copy ([`Self::overlaid`]) shares the live base and layer
/// read-only and writes an overlay of its own (ADR-0034, S7): its first
/// write copies no live key, every key operation runs in the check's body
/// (the [`Keyed`] method its code called), and its drain hands back the
/// overlay one entry per box ([`Self::parts`], S9).
struct Layered<K, V> {
    base: Rc<HashMap<K, V>>,
    layer: Rc<HashMap<K, Option<V>>>,
    /// A check's writes over the live base and layer (`None` taken out);
    /// `None` live, and in a memo a check made, which owns its maps.
    over: Option<HashMap<K, Option<V>>>,
    len: usize,
}

/// How many writes a [`Layered`] keeps beside its base before folding them in.
const LAYER: usize = 64;

impl<K: Eq + std::hash::Hash + Clone, V: Clone> Layered<K, V> {
    fn new() -> Self {
        Self {
            base: Rc::default(),
            layer: Rc::default(),
            over: None,
            len: 0,
        }
    }

    /// A check's copy of a live map: its base and layer shared (O(1), no
    /// key touched), its writes in an overlay of its own. Only a live map
    /// is copied, which has no overlay.
    fn overlaid(&self) -> Self {
        Self {
            base: Rc::clone(&self.base),
            layer: Rc::clone(&self.layer),
            over: Some(HashMap::new()),
            len: self.len,
        }
    }

    /// `k` as the base and layer have it.
    fn lower(&self, k: &K) -> Option<&V> {
        match self.layer.get(k) {
            Some(v) => v.as_ref(),
            None => self.base.get(k),
        }
    }

    fn get(&self, k: &K) -> Option<&V> {
        if let Some(o) = &self.over
            && let Some(v) = o.get(k)
        {
            return v.as_ref();
        }
        self.lower(k)
    }

    fn len(&self) -> usize {
        self.len
    }

    fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        let over = self.over.as_ref();
        let layer = &self.layer;
        let shown = move |k: &K| over.is_none_or(|o| !o.contains_key(k));
        self.base
            .iter()
            .filter(move |(k, _)| !layer.contains_key(*k) && shown(k))
            .chain(
                layer
                    .iter()
                    .filter(move |(k, _)| shown(k))
                    .filter_map(|(k, v)| v.as_ref().map(|v| (k, v))),
            )
            .chain(
                over.into_iter()
                    .flat_map(HashMap::iter)
                    .filter_map(|(k, v)| v.as_ref().map(|v| (k, v))),
            )
    }

    /// Where a write goes: a check's overlay; live, the layer (folded
    /// first once full).
    fn top(&mut self) -> &mut HashMap<K, Option<V>> {
        if self.over.is_none() {
            self.fold();
        }
        match &mut self.over {
            Some(o) => o,
            None => Rc::make_mut(&mut self.layer),
        }
    }

    /// The layer folded into the base, once it is full and nothing shares
    /// either (live: always so).
    fn fold(&mut self) {
        if self.layer.len() < LAYER {
            return;
        }
        if let (Some(base), Some(layer)) = (Rc::get_mut(&mut self.base), Rc::get_mut(&mut self.layer)) {
            for (k, v) in layer.drain() {
                match v {
                    Some(v) => base.insert(k, v),
                    None => base.remove(&k),
                };
            }
        }
    }

    fn write(&mut self, k: K, v: Option<V>) {
        // A removal leaves a tombstone over what is under the top map.
        let under = match self.over {
            Some(_) => self.lower(&k).is_some(),
            None => self.base.contains_key(&k),
        };
        let top = self.top();
        if v.is_none() && !under {
            top.remove(&k);
        } else {
            top.insert(k, v);
        }
    }

    fn get_mut(&mut self, k: &K) -> Option<&mut V> {
        let v = self.get(k)?.clone();
        let top = self.top();
        if !top.contains_key(k) {
            top.insert(k.clone(), Some(v));
        }
        top.get_mut(k).and_then(Option::as_mut)
    }

    fn insert(&mut self, k: K, v: V) -> Option<V> {
        let old = self.get(&k).cloned();
        if old.is_none() {
            self.len = self.len.saturating_add(1);
        }
        self.write(k, Some(v));
        old
    }

    fn remove_entry(&mut self, k: &K) -> Option<(K, V)> {
        let old = self.get(k)?.clone();
        self.write(k.clone(), None);
        self.len = self.len.saturating_sub(1);
        Some((k.clone(), old))
    }
}

impl<K: 'static, V: 'static> Layered<K, V> {
    /// Taken apart for a check's drain (ADR-0034, S9): a map it owns, one
    /// entry per box; one it shares with the live memo let go of here.
    /// Shared, the handle's drop only counts down (another holds it, so no
    /// key drops and no code runs); handed back instead, a runaway check
    /// would set it aside, and it would pin the live maps shared for good,
    /// so they would never fold again.
    fn parts(self, out: &mut Vec<Box<dyn std::any::Any>>) {
        use crate::engine::map_apart;
        match Rc::try_unwrap(self.base) {
            Ok(base) => map_apart(base, out),
            Err(shared) => drop(shared),
        }
        match Rc::try_unwrap(self.layer) {
            Ok(layer) => map_apart(layer, out),
            Err(shared) => drop(shared),
        }
        if let Some(over) = self.over {
            map_apart(over, out);
        }
    }
}

/// An identity kept for a key, and who holds it.
#[derive(Clone, Copy)]
struct Entry<V> {
    value: V,
    holders: u32,
    /// Which issue of the key's identity this is.
    generation: u64,
}

impl<K: Eq + std::hash::Hash + Clone, V: Copy> Memo<K, V> {
    /// `value` kept for `key` unless an identity is already kept for it
    /// (an `issue` that asked for its own key): the one kept.
    fn keep(&mut self, key: &K, value: V, holders: u32) -> Entry<V> {
        if let Some(e) = self.map.get_mut(key) {
            e.holders = e.holders.saturating_add(holders);
            return *e;
        }
        let key = key.clone();
        let generation = self.next;
        self.next = self.next.saturating_add(1);
        let e = Entry {
            value,
            holders,
            generation,
        };
        self.map.insert(key, e);
        e
    }
}

/// A memo as the store holds it, its type erased.
trait ErasedMemo {
    fn as_any(&self) -> &dyn std::any::Any;
    /// A check's copy, made on its first write to a live memo ([`MEMOS`]):
    /// the live maps shared read-only and an empty overlay for the check's
    /// writes ([`Layered::overlaid`]). O(1); no key is touched.
    fn fork_clone(&self) -> Rc<dyn ErasedMemo>;
    /// Taken apart for a check's drain: one box per key the check wrote.
    fn take_apart(self: Rc<Self>, out: &mut Vec<Box<dyn std::any::Any>>);
}

impl<K: Eq + std::hash::Hash + Clone + 'static, V: Copy + 'static> ErasedMemo for std::cell::RefCell<Memo<K, V>> {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn fork_clone(&self) -> Rc<dyn ErasedMemo> {
        let m = self.borrow();
        Rc::new(std::cell::RefCell::new(Memo {
            map: m.map.overlaid(),
            work: m.work,
            next: m.next,
        }))
    }

    fn take_apart(self: Rc<Self>, out: &mut Vec<Box<dyn std::any::Any>>) {
        match Rc::try_unwrap(self) {
            Ok(memo) => memo.into_inner().map.parts(out),
            // Shared: a count-down, as in [`Layered::parts`].
            Err(shared) => drop(shared),
        }
    }
}

/// A memo in the store. Its [`ForkClone`](crate::engine::ForkClone) is a
/// check's overlay on the memo, not a copy of it, and the check's drain
/// takes it apart ([`crate::engine::Parts`]).
struct MemoBox(Rc<dyn ErasedMemo>);

// A logged value (ADR-0034 S7, audited): through `&MemoBox` the memo is
// only read; a write goes through the slab's own copy (`ForkClone`).
crate::engine::fork_value_audited!(MemoBox);
impl crate::engine::sealed::Sealed for MemoBox {}
impl crate::engine::ForkClone for MemoBox {
    fn fork_clone(&self) -> Self {
        // An overlay on the memo, not the pointer (`Rc`'s own `ForkClone`).
        Self(ErasedMemo::fork_clone(&*self.0))
    }
}

impl crate::engine::parts_seal::Sealed for MemoBox {}
impl crate::engine::Parts for MemoBox {
    fn parts(self, out: &mut Vec<Box<dyn std::any::Any>>) {
        let Self(memo) = self;
        ErasedMemo::take_apart(memo, out);
    }
}

crate::engine::engine_local! {
    /// The memos of every [`Keyed`] on the thread, each held by its scope's
    /// life: freed as the scope that made it ends. A slab of handles, by
    /// the one slot rule of every arena ([`crate::engine::Generations`],
    /// ADR-0028 rule 3): a memo a fork writes is kept as it was and put
    /// back, a [`Keyed`] a fork made is the fork's, and no `Keyed` a fork handed out
    /// names a memo made after it. What leaves is handed back to be
    /// dropped outside the store's borrow. A check's drain takes its memos
    /// apart, one box per key (ADR-0034, S9).
    [Fork(log)] static MEMOS: std::cell::RefCell<crate::engine::ForkSlab<MemoBox>> =
        std::cell::RefCell::default();
}

impl<K: 'static, V: 'static> Clone for Keyed<K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: 'static, V: 'static> Copy for Keyed<K, V> {}

impl<K: 'static, V: 'static> fmt::Debug for Keyed<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keyed").finish_non_exhaustive()
    }
}

impl<K: Eq + std::hash::Hash + Clone + 'static, V: Copy + 'static> Keyed<K, V> {
    /// None issued yet, owned by the running scope (made where its owner
    /// is), and freed with it. Made with no scope running it is never
    /// freed: a debug build reports it.
    #[must_use]
    #[track_caller]
    #[expect(clippy::new_without_default, reason = "made where its owner is: no default one")]
    pub fn new() -> Self {
        let memo: Rc<dyn ErasedMemo> = Rc::new(std::cell::RefCell::new(Memo::<K, V> {
            map: Layered::new(),
            work: 0,
            next: 0,
        }));
        let id = crate::engine::allocate(&MEMOS, MemoBox(memo));
        // A fork frees what it made as it ends, and a cleanup it registers
        // may run live, on an owner that outlives it. Making a handle: the
        // engine's own bookkeeping, which a Shared init may reach, reads
        // whether a fork runs unchecked and the owner through
        // [`crate::engine::allocating`]: what it decides is not given back.
        if !crate::engine::is_cold() {
            if crate::engine::allocating(crate::reactive::has_owner) {
                crate::reactive::on_release(move || {
                    let freed = MEMOS.with(|m| m.borrow_mut().remove(id));
                    drop(freed);
                });
            } else {
                #[cfg(debug_assertions)]
                eprintln!(
                    "gui: `Keyed::new` at {} has no scope running: its memo is never freed",
                    std::panic::Location::caller()
                );
            }
        }
        Self {
            id,
            ty: std::marker::PhantomData,
        }
    }

    /// The memo, as the running frame sees it, read by `f` (nothing of the
    /// caller's runs in it).
    fn read<R>(self, f: impl FnOnce(&Memo<K, V>) -> R) -> Option<R> {
        let rc = MEMOS.with(|m| m.borrow().read(self.id, |b| Rc::clone(&b.0)))?;
        let cell = rc.as_any().downcast_ref::<std::cell::RefCell<Memo<K, V>>>()?;
        let memo = cell.borrow();
        Some(f(&memo))
    }

    /// The memo written by `f`; in a check, its overlay on a live memo (made
    /// on its first write, [`Layered::overlaid`]), the live one untouched. What `f`
    /// takes out of the memo is handed back, to be dropped by the caller
    /// with nothing borrowed.
    fn write<R>(self, f: impl FnOnce(&mut Memo<K, V>) -> R) -> Option<R> {
        let rc = MEMOS.with(|m| m.borrow().write(self.id, |b| Rc::clone(&b.0)))?;
        let cell = rc.as_any().downcast_ref::<std::cell::RefCell<Memo<K, V>>>()?;
        let mut memo = cell.borrow_mut();
        Some(f(&mut memo))
    }

    /// Count `n` operations of the live frame: the measure of its work (a
    /// fork reads, and does not copy a memo to count).
    fn count(self, n: u64) {
        if !crate::component::is_cold_render() {
            self.write(|m| m.work = m.work.saturating_add(n));
        }
    }

    /// The identity of `key`: the one issued while it is kept, else a new
    /// one `issue` makes from the engine's counters. Asking again gives
    /// the same, so a render (a cold one included) that asks gives what
    /// the live frame gave. What a cold frame keeps is its own (a fork's),
    /// and thrown away with it.
    pub fn get(self, key: &K, issue: impl FnOnce() -> V) -> V {
        // One batch: what `issue` triggers runs once the identity is kept,
        // so an effect asking for the same key finds it and issues none.
        crate::reactive::batch(|| self.get_in_batch(key, issue))
    }

    fn get_in_batch(self, key: &K, issue: impl FnOnce() -> V) -> V {
        self.count(1);
        match self.read(|m| m.map.get(key).map(|e| e.value)) {
            Some(Some(v)) => v,
            Some(None) => {
                let v = issue();
                self.write(|m| m.keep(key, v, 0).value).unwrap_or(v)
            }
            None => issue(),
        }
    }

    /// The identity issued to `key` while it is kept, if any: asks for
    /// none, keeps none.
    pub fn peek(self, key: &K) -> Option<V> {
        self.count(1);
        self.read(|m| m.map.get(key).map(|e| e.value)).flatten()
    }

    /// [`Self::get`], and `key` held by the calling scope until it ends (in
    /// a render, while that render's output may be on screen): let go as
    /// the last of its holders leaves. The hold and its release are the
    /// scope's in the step that publishes the key, before any other code
    /// runs, so a scope a panic ends still lets go. A hold is of the
    /// identity it was given, not of the key: after [`Self::retain`] let
    /// the key go, the old holder's release touches the identity issued
    /// since no more. A cold frame, or no scope, holds nothing (a cleanup
    /// a cold frame registers may run live).
    #[track_caller]
    pub fn hold(self, key: &K, issue: impl FnOnce() -> V) -> V {
        if crate::component::is_cold_render() || !crate::reactive::has_owner() {
            return self.get(key, issue);
        }
        // Issue, count the hold and install its release in one batch: what
        // is flushed as it ends (an observer that panics) finds all three.
        crate::reactive::batch(|| {
            let v = self.get_in_batch(key, issue);
            self.count(1);
            if let Some(generation) = self.write(|m| m.keep(key, v, 1).generation) {
                let key = key.clone();
                crate::reactive::on_release(move || self.release_hold(&key, generation));
            }
            v
        })
    }

    /// One holder fewer of the identity of `key` issued as `generation`:
    /// kept while it has one; none if that identity is gone.
    fn release_hold(self, key: &K, generation: u64) {
        self.count(1);
        let gone = self
            .write(|m| match m.map.get_mut(key) {
                Some(e) if e.generation == generation => {
                    e.holders = e.holders.saturating_sub(1);
                    if e.holders == 0 { m.map.remove_entry(key) } else { None }
                }
                _ => None,
            })
            .flatten();
        drop(gone);
    }

    /// Let go of each key `keep` refuses: asked for again, it is issued a
    /// new identity. `keep` must not ask this map.
    pub fn retain(self, mut keep: impl FnMut(&K, &V) -> bool) {
        // `keep` runs with nothing borrowed (it may trigger what reads this
        // map); what it refused is taken out after, and dropped after that.
        crate::reactive::batch(|| {
            let Some(held) = self.read(|m| {
                m.map
                    .iter()
                    .map(|(k, e)| (k.clone(), e.value, e.generation))
                    .collect::<Vec<_>>()
            }) else {
                return;
            };
            self.count(u64::try_from(held.len()).unwrap_or(u64::MAX));
            let refused: Vec<(K, u64)> = held
                .into_iter()
                .filter(|(k, v, _)| !keep(k, v))
                .map(|(k, _, g)| (k, g))
                .collect();
            // Only the identities it was shown: one issued since stays.
            let gone = self.write(|m| {
                refused
                    .iter()
                    .filter_map(|(k, g)| {
                        let same = m.map.get(k).is_some_and(|e| e.generation == *g);
                        if same { m.map.remove_entry(k) } else { None }
                    })
                    .collect::<Vec<_>>()
            });
            drop(gone);
            drop(refused);
        });
    }

    /// How many keys it holds an identity for.
    #[must_use]
    pub fn len(self) -> usize {
        self.read(|m| m.map.len()).unwrap_or(0)
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// The map operations done so far: the measure of its work.
    #[must_use]
    pub fn work(self) -> u64 {
        self.read(|m| m.work).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::reactive::Scope;

    /// A mount that never ran its register (a cold re-render's, whose
    /// effects are dropped) never runs its undo either: a count it keeps
    /// stays as the live mounts left it.
    #[cfg(debug_assertions)]
    #[test]
    fn a_cold_setups_mount_neither_registers_nor_undoes() {
        let count = Rc::new(Cell::new(1_i32));
        let c = Rc::clone(&count);
        let cold = crate::engine::ColdFrame::begin();
        let scope = Scope::new();
        scope.run(|| {
            crate::reactive::rendering("cold", || {
                while_mounted(move || {
                    c.set(c.get() + 1);
                    move || c.set(c.get() - 1)
                });
            });
        });
        scope.dispose();
        drop(cold);
        assert_eq!(count.get(), 1, "neither ran");
    }

    /// A cold frame writes a copy of a live memo and the copy goes with the
    /// frame: what it asks for it keeps for itself (two asks of one key in
    /// the frame share an identity, as live ones do), `retain` lets go of
    /// nothing live, work is counted live only, and a `Keyed` made there
    /// has its memo in the frame.
    #[cfg(debug_assertions)]
    #[test]
    fn a_cold_frame_writes_a_copy_of_a_keyed_memo_and_the_live_one_is_untouched() {
        let owner = Scope::new();
        let keyed: Keyed<&'static str, u32> = owner.run(Keyed::new);
        assert_eq!(keyed.get(&"a", || 1), 1);
        let work = keyed.work();
        let issued = Cell::new(0_u32);
        let cold = crate::engine::ColdFrame::begin();
        assert_eq!(keyed.peek(&"a"), Some(1), "it reads the live memo");
        let ask = || {
            keyed.get(&"b", || {
                issued.set(issued.get() + 1);
                20 + issued.get()
            })
        };
        assert_eq!((ask(), ask()), (21, 21), "one identity for the key in the frame");
        assert_eq!(issued.get(), 1);
        keyed.retain(|k, _| *k == "b");
        assert_eq!(keyed.peek(&"a"), None, "the frame's copy let it go");
        let made: Keyed<&'static str, u32> = Scope::new().run(Keyed::new);
        assert_eq!(made.get(&"x", || 7), 7);
        assert_eq!(made.len(), 1, "a Keyed made in a cold frame keeps its own");
        drop(cold);
        assert_eq!(keyed.work(), work, "no work counted from the cold frame");
        assert_eq!(keyed.peek(&"a"), Some(1), "the live memo kept what the frame let go");
        assert_eq!(keyed.peek(&"b"), None, "and has none of what the frame asked for");
        assert_eq!(made.len(), 0, "what the frame made is gone with it");
        owner.dispose();
    }

    /// Opus (F1 review 5): a `Keyed` a check made is the fork's: after it,
    /// a live `Keyed` made in its slot is never what its handle reads, and
    /// in the next check, which makes its own, the handle is still dead.
    #[cfg(debug_assertions)]
    #[test]
    fn a_keyed_a_check_made_names_nothing_after_it() {
        let check = |key: &'static str| {
            let _cold = crate::engine::ColdFrame::begin();
            let k: Keyed<&'static str, u32> = Scope::new().run(Keyed::new);
            assert_eq!(k.get(&key, || 1), 1);
            k
        };
        let k1 = check("k1");
        assert_eq!(k1.len(), 0, "dead after its check");
        {
            // The next check makes its own, in the slot k1 had.
            let _cold = crate::engine::ColdFrame::begin();
            let k2: Keyed<&'static str, u32> = Scope::new().run(Keyed::new);
            assert_eq!(k2.get(&"k2", || 3), 3);
            assert_eq!(k1.peek(&"k2"), None, "k1 reads none of the next fork's");
            assert_eq!(k1.len(), 0, "dead in the next check");
        }
        let owner = Scope::new();
        let live: Keyed<&'static str, u32> = owner.run(Keyed::new);
        assert_eq!(live.get(&"live", || 2), 2);
        assert_eq!(k1.peek(&"live"), None, "the fork's handle reads no live memo");
        assert_eq!(k1.len(), 0);
        owner.dispose();
    }

    /// Codex 7 (F1 review 6): a fork's write to a memo of a million keys
    /// costs what it changes, not a copy of the memo; what it wrote is its
    /// own, and the live memo is as it was after it.
    #[cfg(debug_assertions)]
    #[test]
    fn a_forks_insert_into_a_memo_of_a_million_keys_copies_none_of_them() {
        let owner = Scope::new();
        let keyed: Keyed<u64, u64> = owner.run(Keyed::new);
        let n = 1_000_000_u64;
        for k in 0..n {
            keyed.get(&k, || k);
        }
        {
            let _cold = crate::engine::ColdFrame::begin();
            let (got, bytes) = crate::heap::allocated_bytes(|| keyed.get(&n, || 7));
            assert_eq!(got, 7);
            assert!(bytes < 64 * 1024, "{bytes} bytes for one insert");
            assert_eq!(keyed.peek(&n), Some(7));
            assert_eq!(keyed.peek(&3), Some(3), "the fork reads the live keys");
            assert_eq!(keyed.len(), 1_000_001);
        }
        assert_eq!(keyed.peek(&n), None, "the live memo has none of the fork's");
        assert_eq!(keyed.len(), 1_000_000);
        assert_eq!(keyed.peek(&999_999), Some(999_999));
        owner.dispose();
    }

    /// A key whose clones are counted, in a probe the test holds (outside
    /// the engine: a check neither forks nor puts it back); told apart by
    /// its number alone. The probe is behind `Arc` and atomic: it is no
    /// shared handle of the library's, so it is not held as one.
    struct Counted(u32, Arc<AtomicU32>);

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    impl PartialEq for Counted {
        fn eq(&self, o: &Self) -> bool {
            self.0 == o.0
        }
    }

    impl Eq for Counted {}

    impl std::hash::Hash for Counted {
        fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
            self.0.hash(h);
        }
    }

    impl Clone for Counted {
        fn clone(&self) -> Self {
            self.1.fetch_add(1, Ordering::Relaxed);
            Self(self.0, Arc::clone(&self.1))
        }
    }

    /// ADR-0034 S7: a check's first write to a memo whose layer holds live
    /// keys copies none of them (the check writes an overlay of its own,
    /// over the live maps shared read-only): one clone, of the key it
    /// wrote, in its body. The live memo is as it was after.
    #[cfg(debug_assertions)]
    #[test]
    fn a_checks_write_to_a_memo_clones_no_live_key() {
        let clones = Arc::new(AtomicU32::new(0));
        let key = |k: u32| Counted(k, Arc::clone(&clones));
        let owner = Scope::new();
        let keyed: Keyed<Counted, u32> = owner.run(Keyed::new);
        // Fewer than a layer's worth: all of them in the live layer.
        for k in 0..40 {
            keyed.get(&key(k), || k);
        }
        clones.store(0, Ordering::Relaxed);
        crate::engine::in_cold_frame(|| {
            assert_eq!(keyed.get(&key(1000), || 7), 7);
            assert_eq!(keyed.peek(&key(3)), Some(3), "it reads the live keys");
            assert_eq!(keyed.len(), 41);
        });
        assert_eq!(clones.load(Ordering::Relaxed), 1, "the key written, and no live key");
        assert_eq!(keyed.peek(&key(1000)), None, "none of the check's in the live memo");
        assert_eq!((keyed.len(), keyed.peek(&key(39))), (40, Some(39)));
        owner.dispose();
    }

    /// A key whose clones panic as they drop while armed (a probe the test
    /// holds, outside the engine); told apart by its number alone.
    struct Bomb(u32, bool, Arc<AtomicBool>);

    impl PartialEq for Bomb {
        fn eq(&self, o: &Self) -> bool {
            self.0 == o.0
        }
    }

    impl Eq for Bomb {}

    impl std::hash::Hash for Bomb {
        fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
            self.0.hash(h);
        }
    }

    impl Clone for Bomb {
        fn clone(&self) -> Self {
            Self(self.0, true, Arc::clone(&self.2))
        }
    }

    impl Drop for Bomb {
        fn drop(&mut self) {
            if self.1 && self.2.load(Ordering::Relaxed) {
                panic!("key {}'s drop", self.0);
            }
        }
    }

    /// The message a check that faulted panics with.
    fn said(p: &Box<dyn std::any::Any + Send>) -> String {
        p.downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default()
    }

    /// ADR-0034 S7, S9 and S2 (review r16): two keys a check wrote to a
    /// live memo, each panicking as it drops, are handed back one box per
    /// key and both dropped: the first is the check's fault, the second
    /// follows it, and the live memo is whole.
    #[cfg(debug_assertions)]
    #[test]
    fn two_panicking_keys_a_check_wrote_are_two_faults_in_one_report() {
        let armed = Arc::new(AtomicBool::new(false));
        let key = |k: u32| Bomb(k, false, Arc::clone(&armed));
        let owner = Scope::new();
        let keyed: Keyed<Bomb, u32> = owner.run(Keyed::new);
        assert_eq!(keyed.get(&key(0), || 5), 5);
        armed.store(true, Ordering::Relaxed);
        let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::engine::in_cold_frame(|| {
                keyed.get(&key(1), || 1);
                keyed.get(&key(2), || 2);
            });
        }))
        .expect_err("the check faults");
        armed.store(false, Ordering::Relaxed);
        let said = said(&p);
        assert!(said.contains("a check faulted (Disposal of MEMOS"), "{said}");
        assert_eq!(
            said.matches("Disposal of MEMOS").count(),
            2,
            "both keys dropped: {said}"
        );
        assert!(said.contains("key 1's drop") && said.contains("key 2's drop"), "{said}");
        assert_eq!(keyed.len(), 1, "the live memo is whole");
        assert_eq!(keyed.peek(&key(0)), Some(5));
        assert_eq!(keyed.peek(&key(1)), None);
        owner.dispose();
    }

    /// ADR-0034 S9: a memo a check made holds its keys in two maps (its
    /// base, and the layer of writes since it folded). Dropped whole, a
    /// key panicking in the base's drop and another in the layer's, in the
    /// same unwind, abort the process; taken apart one key per box, every
    /// one is dropped (S2, review r16): the first is the check's fault and
    /// each other follows it.
    #[cfg(debug_assertions)]
    #[test]
    fn panicking_keys_of_a_memo_a_check_made_are_each_dropped_in_one_report() {
        let armed = Arc::new(AtomicBool::new(true));
        let n = u32::try_from(LAYER).expect("a small number") + 1;
        let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::engine::in_cold_frame(|| {
                let made: Keyed<Bomb, u32> = Scope::new().run(Keyed::new);
                for k in 0..n {
                    made.get(&Bomb(k, false, Arc::clone(&armed)), || k);
                }
            });
        }))
        .expect_err("the check faults");
        armed.store(false, Ordering::Relaxed);
        let said = said(&p);
        assert!(said.contains("a check faulted (Disposal of MEMOS"), "{said}");
        let each = usize::try_from(n).expect("a small number");
        assert_eq!(
            said.matches("Disposal of MEMOS").count(),
            each,
            "every key dropped: {said}"
        );
    }

    /// How many writes a live memo keeps beside its base, and whether
    /// anything but it holds its base or layer.
    fn layering(keyed: Keyed<u32, u32>) -> (usize, bool) {
        keyed
            .read(|m| {
                let shared = Rc::strong_count(&m.map.base) > 1 || Rc::strong_count(&m.map.layer) > 1;
                (m.map.layer.len(), shared)
            })
            .expect("a live memo")
    }

    /// A value whose drop panics: a check's, in a piece it enters after
    /// [`MEMOS`], so it drops first and what the memos hand back drops
    /// after its fault.
    #[cfg(debug_assertions)]
    struct Boom;

    #[cfg(debug_assertions)]
    impl crate::engine::FreshValue for Boom {}

    #[cfg(debug_assertions)]
    impl Drop for Boom {
        fn drop(&mut self) {
            panic!("a boom's drop");
        }
    }

    crate::engine::engine_local! {
        /// Booms a check queues.
        #[cfg(debug_assertions)]
        [Fresh] static BOOMS: std::cell::RefCell<Vec<Boom>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// ADR-0034 S2, S7: a check that wrote over a live memo shares its base
    /// and layer; another value the check made faults as it drops. A shared
    /// handle a runaway set aside would pin the live maps (shared, they
    /// never fold again: every live write after grows the layer). The drain
    /// lets such a handle go as it takes the memo apart (a count-down, which
    /// runs no code), and the disposal goes on past the fault (review r16),
    /// so the live memo folds as before.
    #[cfg(debug_assertions)]
    #[test]
    fn a_live_memo_a_faulted_check_wrote_folds_again() {
        let owner = Scope::new();
        let plain: Keyed<u32, u32> = owner.run(Keyed::new);
        let n = u32::try_from(LAYER).expect("a small number");
        for k in 0..n {
            plain.get(&k, || k);
        }
        let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::engine::in_cold_frame(|| {
                plain.get(&10_000, || 1);
                BOOMS.with_borrow_mut(|b| b.push(Boom));
            });
        }))
        .expect_err("the check faults");
        assert!(said(&p).contains("a check faulted (Disposal of BOOMS"), "{}", said(&p));
        assert!(!layering(plain).1, "nothing of the check's holds the live maps");
        for k in n..n * 3 {
            plain.get(&k, || k);
        }
        let (layer, shared) = layering(plain);
        assert!(
            layer <= LAYER && !shared,
            "the live memo folds: {layer} writes beside its base"
        );
        assert_eq!((plain.len(), plain.peek(&10_000)), (LAYER * 3, None));
        owner.dispose();
    }

    crate::engine::engine_local! {
        /// A Shared memo a check asks for first.
        #[cfg(debug_assertions)]
        [Shared("a test of a Shared memo a fork touches first")]
        static SHARED_MEMO: Keyed<&'static str, u32> = Keyed::new();
    }

    /// RC1 (F1 review 5), F1 review 9: a Shared `Keyed` a check asks for
    /// first is the check's stand-in, gone with it (ADR-0028), never made
    /// live by it; live code makes the live one at its first touch, which
    /// a later check then shares, and which holds what live code asks.
    #[cfg(debug_assertions)]
    #[test]
    fn a_shared_keyed_a_check_asks_for_first_is_a_stand_in_and_made_live_later() {
        std::thread::spawn(|| {
            let stand_in = {
                let _cold = crate::engine::ColdFrame::begin();
                SHARED_MEMO.with(|k| *k)
            };
            let held = |k: Keyed<&'static str, u32>| MEMOS.with(|m| m.borrow().contains(k.id));
            assert!(!held(stand_in), "the fork's stand-in is gone with it");
            let live = SHARED_MEMO.with(|k| *k);
            let kept = crate::engine::in_cold_frame(|| SHARED_MEMO.with(|k| *k));
            assert!(kept.id == live.id && held(kept), "a later check shares the live one");
            assert_eq!(kept.get(&"a", || 5), 5);
            assert_eq!(kept.len(), 1, "it holds what live code asked");
        })
        .join()
        .expect("a run");
    }

    /// A key whose `Drop` asks another `Keyed` does not panic as a scope
    /// frees a memo, lets a hold go or retains: no memo or store is
    /// borrowed while it drops.
    #[test]
    fn a_keys_drop_that_asks_another_keyed_does_not_panic_in_cleanup() {
        #[derive(Clone)]
        struct Probe(u32, Keyed<u32, u32>);
        impl PartialEq for Probe {
            fn eq(&self, o: &Self) -> bool {
                self.0 == o.0
            }
        }
        impl Eq for Probe {}
        impl std::hash::Hash for Probe {
            fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
                self.0.hash(h);
            }
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                std::hint::black_box(self.1.len());
            }
        }
        let owner = Scope::new();
        let other: Keyed<u32, u32> = owner.run(Keyed::new);
        let probes: Keyed<Probe, u32> = owner.run(Keyed::new);
        let part = Scope::new();
        part.run(|| probes.hold(&Probe(1, other), || 1));
        probes.get(&Probe(2, other), || 2);
        probes.retain(|k, _| k.0 != 2);
        part.dispose();
        assert_eq!(probes.len(), 0);
        let scoped = Scope::new();
        scoped.run(|| {
            let inner: Keyed<Probe, u32> = Keyed::new();
            inner.get(&Probe(3, other), || 3);
        });
        scoped.dispose();
        owner.dispose();
    }

    /// A hold is of the identity it was given: after `retain` let the key
    /// go and another scope held the identity issued since, the first
    /// scope ending takes nothing from it.
    #[test]
    fn a_hold_released_after_retain_leaves_the_identity_issued_since() {
        let owner = Scope::new();
        let keyed: Keyed<&'static str, u32> = owner.run(Keyed::new);
        let (a, b) = (Scope::new(), Scope::new());
        assert_eq!(a.run(|| keyed.hold(&"k", || 1)), 1);
        keyed.retain(|_, _| false);
        assert_eq!(keyed.peek(&"k"), None);
        assert_eq!(b.run(|| keyed.hold(&"k", || 2)), 2, "a new identity");
        a.dispose();
        assert_eq!(keyed.peek(&"k"), Some(2), "a's release did not touch b's identity");
        b.dispose();
        assert_eq!(keyed.peek(&"k"), None, "b's release lets it go");
        owner.dispose();
    }

    /// `hold` counts the hold and installs its release before what the
    /// batch flushes runs: an observer that panics leaves a hold its scope
    /// still releases.
    #[test]
    fn a_hold_whose_flush_panics_still_has_its_release() {
        let owner = Scope::new();
        let keyed: Keyed<&'static str, u32> = owner.run(Keyed::new);
        let poke = Signal::new(0_u32);
        owner.run(|| {
            Effect::new(move || {
                assert!(poke.get() == 0, "an observer that panics");
            });
        });
        let part = Scope::new();
        let held = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            part.run(|| {
                keyed.hold(&"k", || {
                    poke.set(1);
                    5
                })
            })
        }));
        assert!(held.is_err(), "the observer's panic came out of the flush");
        part.dispose();
        assert_eq!(keyed.len(), 0, "the scope's end released the hold");
        owner.dispose();
    }

    /// `get` runs `issue` and keeps its result in one batch: an effect that
    /// `issue` wakes and that asks for the same key finds the identity kept
    /// and issues none of its own.
    #[test]
    fn what_issue_wakes_finds_the_identity_kept() {
        let owner = Scope::new();
        let keyed: Keyed<&'static str, u32> = owner.run(Keyed::new);
        let poke = Signal::new(0_u32);
        let issued = Rc::new(Cell::new(0_u32));
        let seen = Rc::new(Cell::new(0_u32));
        let (i, s) = (Rc::clone(&issued), Rc::clone(&seen));
        owner.run(|| {
            Effect::new(move || {
                if poke.get() > 0 {
                    let i = Rc::clone(&i);
                    s.set(keyed.get(&"k", move || {
                        i.set(i.get() + 1);
                        99
                    }));
                }
            });
        });
        let i = Rc::clone(&issued);
        let v = keyed.get(&"k", move || {
            i.set(i.get() + 1);
            poke.set(1);
            5
        });
        assert_eq!(v, 5);
        assert_eq!(seen.get(), 5, "the effect found the identity the caller issued");
        assert_eq!(issued.get(), 1, "one was issued");
        owner.dispose();
    }

    /// A `retain` predicate that wakes what reads the map does not run
    /// with the map borrowed.
    #[test]
    fn a_retain_that_wakes_a_reader_of_the_map_does_not_panic() {
        let owner = Scope::new();
        let keyed: Keyed<u32, u32> = owner.run(Keyed::new);
        for k in 0..3 {
            keyed.get(&k, || k * 10);
        }
        let poke = Signal::new(0_u32);
        let lens = Rc::new(Cell::new(usize::MAX));
        let l = Rc::clone(&lens);
        owner.run(|| {
            Effect::new(move || {
                poke.get();
                l.set(keyed.len());
            });
        });
        keyed.retain(|k, _| {
            poke.set(poke.get_untracked() + 1);
            *k != 1
        });
        assert_eq!(keyed.len(), 2);
        assert_eq!(lens.get(), 2, "the reader ran once the map was changed");
        owner.dispose();
    }

    /// A keyed issuer is asked in a render and keeps what it issues there:
    /// its memo is not reactive state, so a render that asks writes no
    /// signal (which a render must not) and asking again gives the same.
    #[test]
    fn a_keyed_issuer_asked_in_a_render_keeps_what_it_issued() {
        let owner = Scope::new();
        let keyed: Keyed<&'static str, u32> = owner.run(Keyed::new);
        let issued = Rc::new(Cell::new(0_u32));
        let ask = |key: &'static str| {
            let issued = Rc::clone(&issued);
            let mut got = 0;
            crate::reactive::rendering("ask", || {
                got = keyed.get(&key, || {
                    issued.set(issued.get() + 1);
                    issued.get() * 10
                });
            });
            got
        };
        assert_eq!(ask("a"), 10);
        assert_eq!(ask("a"), 10, "the same while kept");
        assert_eq!(ask("b"), 20);
        assert_eq!(keyed.len(), 2);
        assert_eq!(keyed.peek(&"a"), Some(10));
        keyed.retain(|k, _| *k != "a");
        assert_eq!(keyed.peek(&"a"), None, "let go");
        assert_eq!(ask("a"), 30, "asked for again after it left: a new one");
        assert!(keyed.work() > 0, "its operations are counted");
        owner.dispose();
    }

    /// A registry is given an entry as a part mounts and loses only that
    /// one as it leaves, whatever the value; a setup that never mounts (a
    /// cold re-render's, debug builds) gives it nothing.
    #[test]
    fn a_registry_holds_the_mounted_parts_entries_alone() {
        let owner = Scope::new();
        let registry: Registry<&'static str> = owner.run(Registry::new);
        #[cfg(debug_assertions)]
        {
            let cold = crate::engine::ColdFrame::begin();
            let scope = Scope::new();
            scope.run(|| crate::reactive::rendering("cold", || registry.register("cold")));
            scope.dispose();
            drop(cold);
            assert_eq!(registry.len_untracked(), 0, "a setup that never mounts gives nothing");
        }
        let (a, m, b) = (Scope::new(), Scope::new(), Scope::new());
        a.run(|| registry.register("same"));
        m.run(|| registry.register("marker"));
        b.run(|| registry.register("same"));
        assert_eq!(registry.to_vec_untracked(), ["same", "marker", "same"]);
        a.dispose();
        assert_eq!(
            registry.to_vec_untracked(),
            ["marker", "same"],
            "the one leaving takes out its own, the first"
        );
        let c = Scope::new();
        c.run(|| registry.register("last"));
        assert_eq!(
            registry.to_vec_untracked(),
            ["marker", "same", "last"],
            "in the order they came"
        );
        b.dispose();
        assert_eq!(registry.to_vec_untracked(), ["marker", "last"]);
        m.dispose();
        c.dispose();
        assert_eq!(registry.len_untracked(), 0);
        owner.dispose();
    }

    /// The owner's teardown is installed before the registration is
    /// published: an observer the publishing tells synchronously that ends
    /// the owner leaves no entry behind.
    #[test]
    fn an_owner_ended_while_its_registration_is_published_leaves_no_entry() {
        let owner = Scope::new();
        let registry: Registry<&'static str> = owner.run(Registry::new);
        let part = Scope::new();
        // An observer the publishing tells synchronously: it ends the part.
        type Tell = Box<dyn FnOnce()>;
        let observer: Rc<Cell<Option<Tell>>> = Rc::new(Cell::new(Some(Box::new(move || part.dispose()))));
        part.run(|| {
            while_mounted(move || {
                let mut key = None;
                registry.entries.update(|e| key = Some(e.insert("published")));
                if let Some(tell) = observer.take() {
                    tell();
                }
                move || {
                    if let Some(key) = key {
                        registry.entries.update(|e| e.remove(key));
                    }
                }
            });
        });
        assert_eq!(
            registry.len_untracked(),
            0,
            "the observer ended the part: no entry survives"
        );
        owner.dispose();
    }

    /// Taking out 500 entries, in any order, is O(500) work.
    #[test]
    fn a_registry_takes_out_entries_in_constant_work() {
        let owner = Scope::new();
        let registry: Registry<u32> = owner.run(Registry::new);
        let parts: Vec<Scope> = (0..500_u32)
            .map(|v| {
                let s = Scope::new();
                s.run(|| registry.register(v));
                s
            })
            .collect();
        let added = registry.visits();
        // Every 7th, round and round: neither the first nor the last each time.
        let mut order: Vec<usize> = (0..500).map(|i| (i * 7) % 500).collect();
        order.dedup();
        for i in order {
            if let Some(s) = parts.get(i) {
                s.dispose();
            }
        }
        assert_eq!(registry.len_untracked(), 0);
        let removed = registry.visits() - added;
        assert!(
            removed <= 5 * 500,
            "taking out 500 entries stepped through {removed} slots"
        );
        owner.dispose();
    }

    /// A part leaving while its registry is read (an observer the read
    /// runs ends it): nothing panics, and its entry is gone once the read
    /// ends.
    #[test]
    fn a_part_leaving_during_a_read_leaves_no_entry() {
        let owner = Scope::new();
        let registry: Registry<u32> = owner.run(Registry::new);
        let part = Scope::new();
        part.run(|| registry.register(1));
        let keep = Scope::new();
        keep.run(|| registry.register(2));
        let seen = registry.with(|it| {
            part.dispose();
            it.copied().collect::<Vec<_>>()
        });
        assert_eq!(seen, [2], "the read no longer sees it");
        assert_eq!(registry.to_vec_untracked(), [2], "its entry is gone");
        assert_eq!(registry.len_untracked(), 1);
        keep.dispose();
        assert_eq!(registry.len_untracked(), 0);
        owner.dispose();
    }

    /// A walk whose step ends the part it stands on still reaches the next
    /// one, and every part, in order.
    #[test]
    fn a_walk_that_ends_the_part_it_stands_on_reaches_the_next() {
        let owner = Scope::new();
        let registry: Registry<usize> = owner.run(Registry::new);
        let parts: Vec<Scope> = (0..5)
            .map(|v| {
                let s = Scope::new();
                s.run(|| registry.register(v));
                s
            })
            .collect();
        let mut seen = Vec::new();
        registry.walk(|v| {
            seen.push(v);
            if let Some(p) = parts.get(v) {
                p.dispose();
            }
            true
        });
        assert_eq!(seen, [0, 1, 2, 3, 4]);
        assert_eq!(registry.len_untracked(), 0, "every entry taken out as the walk ended");
        owner.dispose();
    }

    /// Frame after frame a part mounts and leaves, each frame a walk that
    /// stops at it as it leaves: what each walk left is unlinked as it
    /// ends, so the registry retains nothing.
    #[test]
    fn parts_that_come_and_go_across_walks_leave_nothing_linked() {
        let owner = Scope::new();
        let registry: Registry<u32> = owner.run(Registry::new);
        let stays = Scope::new();
        stays.run(|| registry.register(0));
        for n in 1..=100 {
            let part = Scope::new();
            part.run(|| registry.register(n));
            registry.walk(|v| {
                if v == n {
                    part.dispose();
                }
                v != n
            });
            assert_eq!(registry.retained(), 1, "frame {n}: the part that stays alone");
        }
        stays.dispose();
        assert_eq!(registry.retained(), 0);
        owner.dispose();
    }

    /// A walk whose step ends the registry's owner ends there, cleanly.
    #[test]
    fn a_walk_whose_step_ends_the_owner_ends_cleanly() {
        let owner = Scope::new();
        let registry: Registry<u32> = owner.run(Registry::new);
        let parts: Vec<Scope> = (0..3)
            .map(|v| {
                let s = Scope::new();
                s.run(|| registry.register(v));
                s
            })
            .collect();
        let mut seen = Vec::new();
        registry.walk(|v| {
            seen.push(v);
            owner.dispose();
            true
        });
        assert_eq!(seen, [0], "the walk ended with its owner");
        assert_eq!(registry.retained(), 0);
        parts.into_iter().for_each(Scope::dispose);
    }

    /// A key held by two users is kept while either is, and let go as the
    /// last leaves: asked again, it is a new identity.
    #[test]
    fn a_key_held_by_two_users_lives_while_either_does() {
        let owner = Scope::new();
        let keyed: Keyed<&'static str, u64> = owner.run(Keyed::new);
        let next = Rc::new(Cell::new(0_u64));
        let issue = || {
            let n = Rc::clone(&next);
            move || {
                n.set(n.get() + 1);
                n.get()
            }
        };
        let (a, b) = (Scope::new(), Scope::new());
        let first = a.run(|| keyed.hold(&"k", issue()));
        assert_eq!(b.run(|| keyed.hold(&"k", issue())), first, "the second user shares it");
        a.dispose();
        assert_eq!(keyed.peek(&"k"), Some(first), "kept while the second holds it");
        b.dispose();
        assert!(keyed.is_empty(), "let go as the last leaves");
        assert_ne!(keyed.get(&"k", issue()), first, "a new identity after");
        owner.dispose();
    }

    /// Two mounts give the same value; the one set first leaving lets go of
    /// nothing, the one holding it leaving lets go.
    #[test]
    fn a_holder_is_taken_back_by_registration_not_by_value() {
        let holder: Holder<Rc<str>> = Holder::new();
        let same: Rc<str> = Rc::from("same");
        let first = holder.set(Rc::clone(&same));
        let second = holder.set(Rc::clone(&same));
        assert!(!holder.clear(first), "the first no longer holds it");
        assert_eq!(holder.get_untracked().as_deref(), Some("same"));
        assert!(holder.clear(second));
        assert_eq!(holder.get_untracked(), None);
    }
}
