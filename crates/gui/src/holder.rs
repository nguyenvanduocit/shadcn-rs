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
        crate::engine::engine_local! {
            [Fork] static ISSUED: Cell<u64> = const { Cell::new(0) };
        }
        Self(ISSUED.with(|n| {
            let next = n.get().checked_add(1).unwrap_or_else(|| exhausted("registrations"));
            n.set(next);
            next
        }))
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
    gone: Cell<bool>,
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
struct Entries<T> {
    slots: Vec<Slot<T>>,
    free: Vec<usize>,
    head: usize,
    tail: usize,
    /// The slots linked, those taken out while read included.
    linked: usize,
    /// Of those, the ones taken out while read ([`Slot::gone`]).
    gone: Cell<usize>,
    /// Reads under way ([`Registry::with`], [`Registry::walk`]): a removal meanwhile
    /// waits for them.
    readers: Cell<u32>,
    /// The removals that waited, applied as the last read ends.
    queued: Cell<Vec<Key>>,
    /// Slots stepped through, read or written: the measure of the work.
    visits: Cell<u64>,
}

impl<T> Entries<T> {
    fn visit(&self) {
        self.visits.set(self.visits.get().saturating_add(1));
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
                    gone: Cell::new(false),
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
struct Reading<T: 'static>(Registry<T>);

impl<T: 'static> Reading<T> {
    fn begin(r: Registry<T>) -> Option<Self> {
        if !r.entries.is_alive() {
            return None;
        }
        r.entries
            .with_untracked(|e| e.readers.set(e.readers.get().saturating_add(1)));
        Some(Self(r))
    }
}

impl<T: 'static> Drop for Reading<T> {
    fn drop(&mut self) {
        let entries = self.0.entries;
        if !entries.is_alive() {
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
                gone: Cell::new(0),
                readers: Cell::new(0),
                queued: Cell::new(Vec::new()),
                visits: Cell::new(0),
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
    map: Signal<std::cell::RefCell<std::collections::HashMap<K, (V, u32)>>>,
    /// Map operations so far: the measure of the work, for tests.
    work: Signal<Cell<u64>>,
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
    /// is).
    #[must_use]
    #[track_caller]
    #[expect(
        clippy::new_without_default,
        reason = "made where its owner is, as a signal: no default one"
    )]
    pub fn new() -> Self {
        Self {
            map: Signal::new(std::cell::RefCell::new(std::collections::HashMap::new())),
            work: Signal::new(Cell::new(0)),
        }
    }

    fn count(self) {
        if self.work.is_alive() {
            self.work.with_untracked(|w| w.set(w.get().saturating_add(1)));
        }
    }

    /// The identity of `key`: the one issued while it is kept, else a new
    /// one `issue` makes from the engine's counters. Asking again gives
    /// the same, so a render (a cold one included) that asks gives what
    /// the live frame gave. A cold frame keeps none it issues: what it
    /// made is thrown away with it.
    pub fn get(self, key: &K, issue: impl FnOnce() -> V) -> V {
        if !self.map.is_alive() {
            return issue();
        }
        self.count();
        self.map.with_untracked(|m| {
            let kept = m.borrow().get(key).map(|(v, _)| *v);
            kept.unwrap_or_else(|| {
                let v = issue();
                if !crate::component::is_cold_render() {
                    m.borrow_mut().insert(key.clone(), (v, 0));
                }
                v
            })
        })
    }

    /// The identity issued to `key` while it is kept, if any: asks for
    /// none, keeps none.
    pub fn peek(self, key: &K) -> Option<V> {
        if !self.map.is_alive() {
            return None;
        }
        self.count();
        self.map.with_untracked(|m| m.borrow().get(key).map(|(v, _)| *v))
    }

    /// [`Self::get`], and `key` held by the calling scope until it ends (in
    /// a render, while that render's output may be on screen): let go as
    /// the last of its holders leaves. The hold and its release are the
    /// scope's in the step that publishes the key, before any other code
    /// runs, so a scope a panic ends still lets go. A cold frame, or no
    /// scope, holds nothing.
    #[track_caller]
    pub fn hold(self, key: &K, issue: impl FnOnce() -> V) -> V {
        if crate::component::is_cold_render() || !crate::reactive::has_owner() {
            return self.get(key, issue);
        }
        let v = self.get(key, issue);
        self.add(key, v, 1);
        let key = key.clone();
        on_cleanup(move || self.add(&key, v, -1));
        v
    }

    /// One holder more (`1`) or fewer (`-1`) of `key`, issued `v`: kept
    /// while it has one.
    fn add(self, key: &K, v: V, by: i8) {
        if !self.map.is_alive() {
            return;
        }
        self.count();
        self.map.with_untracked(|m| {
            let mut m = m.borrow_mut();
            if by > 0 {
                let e = m.entry(key.clone()).or_insert((v, 0));
                e.1 = e.1.saturating_add(1);
            } else if let Some(e) = m.get_mut(key) {
                e.1 = e.1.saturating_sub(1);
                if e.1 == 0 {
                    m.remove(key);
                }
            }
        });
    }

    /// Let go of each key `keep` refuses: asked for again, it is issued a
    /// new identity. `keep` must not ask this map.
    pub fn retain(self, mut keep: impl FnMut(&K, &V) -> bool) {
        if self.map.is_alive() {
            self.map.with_untracked(|m| {
                let mut m = m.borrow_mut();
                let n = u64::try_from(m.len()).unwrap_or(u64::MAX);
                if self.work.is_alive() {
                    self.work.with_untracked(|w| w.set(w.get().saturating_add(n)));
                }
                m.retain(|k, (v, _)| keep(k, v));
            });
        }
    }

    /// How many keys it holds an identity for.
    #[must_use]
    pub fn len(self) -> usize {
        if !self.map.is_alive() {
            return 0;
        }
        self.map.with_untracked(|m| m.borrow().len())
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// The map operations done so far: the measure of its work.
    #[must_use]
    pub fn work(self) -> u64 {
        if !self.work.is_alive() {
            return 0;
        }
        self.work.with_untracked(Cell::get)
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

    /// A registry is given an entry as a part mounts and loses only that
    /// one as it leaves, whatever the value; a setup that never mounts (a
    /// cold re-render's) gives it nothing.
    #[test]
    fn a_registry_holds_the_mounted_parts_entries_alone() {
        let owner = Scope::new();
        let registry: Registry<&'static str> = owner.run(Registry::new);
        let cold = crate::engine::ColdFrame::begin();
        let scope = Scope::new();
        scope.run(|| crate::reactive::rendering("cold", || registry.register("cold")));
        scope.dispose();
        drop(cold);
        assert_eq!(registry.len_untracked(), 0, "a setup that never mounts gives nothing");
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
