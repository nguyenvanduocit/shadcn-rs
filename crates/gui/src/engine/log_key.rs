//! The key of a `[Fork(log)]` piece (ADR-0034, S7): a collection that keeps
//! its own overlay or undo log ([`Logged`]) in the same value as its live
//! data. The fork machinery may own only what the fork made, so neither the
//! live data nor the log may pass through a value the caller can move, drop
//! or replace (Codex r15-2: `take()` moved the container out, a default
//! with no overlay took the check's writes into its base, and the rollback
//! found nothing to undo).
//!
//! So the key hands out no whole value and no cell: reads get `&L`, writes
//! get a [`LogMut`], which dereferences to `&L` only and writes through the
//! collection's own overlay API ([`ForkMap`], [`ForkSlab`]). There is no
//! `take`, `set`, `replace` or `&RefCell` on it:
//!
//! ```compile_fail,E0599
//! // Codex r15-2's reproducer, as it was written.
//! use std::cell::RefCell;
//! gui::engine_local! {
//!     [Fork(log)] static ROWS: RefCell<gui::engine::ForkMap<u64, u8>> = RefCell::default();
//! }
//! ROWS.with_borrow_mut(|rows| rows.replace(1, 7));
//! gui::testing::in_check(|| {
//!     drop(ROWS.take());
//!     ROWS.with_borrow_mut(|rows| rows.replace(2, 9));
//! });
//! ```
//!
//! ```compile_fail,E0599
//! use std::cell::RefCell;
//! gui::engine_local! {
//!     [Fork(log)] static ROWS: RefCell<gui::engine::ForkMap<u64, u8>> = RefCell::default();
//! }
//! ROWS.set(gui::engine::ForkMap::default());
//! ```
//!
//! ```compile_fail,E0599
//! use std::cell::RefCell;
//! gui::engine_local! {
//!     [Fork(log)] static ROWS: RefCell<gui::engine::ForkMap<u64, u8>> = RefCell::default();
//! }
//! ROWS.with(|c| drop(c.replace(gui::engine::ForkMap::default())));
//! ```
//!
//! Nor does a write borrow move the container out: a [`LogMut`] has no
//! `DerefMut`, so `std::mem::take`, `replace` and `swap` find no `&mut L`.
//!
//! ```compile_fail,E0596
//! use std::cell::RefCell;
//! gui::engine_local! {
//!     [Fork(log)] static ROWS: RefCell<gui::engine::ForkMap<u64, u8>> = RefCell::default();
//! }
//! ROWS.with_borrow_mut(|rows| drop(std::mem::take(&mut **rows)));
//! ```
//!
//! ```compile_fail,E0596
//! use std::cell::RefCell;
//! gui::engine_local! {
//!     [Fork(log)] static ROWS: RefCell<gui::engine::ForkMap<u64, u8>> = RefCell::default();
//! }
//! ROWS.with(|c| drop(std::mem::take(&mut *c.borrow_mut())));
//! ```
//!
//! ```compile_fail,E0596
//! use std::cell::RefCell;
//! gui::engine_local! {
//!     [Fork(log)] static A: RefCell<gui::engine::ForkSlab<u8>> = RefCell::default();
//!     [Fork(log)] static B: RefCell<gui::engine::ForkSlab<u8>> = RefCell::default();
//! }
//! A.with_borrow_mut(|a| B.with_borrow_mut(|b| std::mem::swap(&mut **a, &mut **b)));
//! ```
//!
//! Swapping two views swaps two borrows, not the collections. The writes
//! and reads compile (the positive control):
//!
//! ```
//! use std::cell::RefCell;
//! gui::engine_local! {
//!     [Fork(log)] static ROWS: RefCell<gui::engine::ForkMap<u64, u8>> = RefCell::default();
//!     [Fork(log)] static SLAB: RefCell<gui::engine::ForkSlab<u8>> = RefCell::default();
//! }
//! ROWS.with_borrow_mut(|rows| rows.replace(1, 7));
//! ROWS.with(|c| c.borrow_mut().insert(2, 8));
//! let id = SLAB.with_borrow_mut(|s| s.insert(3));
//! SLAB.with_borrow(|s| s.write(id, |v| *v += 1));
//! assert_eq!(ROWS.with_borrow(|r| r.len()), 2);
//! assert_eq!(SLAB.with(|s| s.borrow().read(id, |v| *v)), Some(4));
//! ```
//!
//! No code, `gui`'s included, reaches a logged collection mutably but
//! through these: `LogMut`'s field is private to this module, and the
//! only `Logged` types are [`ForkMap`] and [`ForkSlab`] (sealed; a state of
//! several maps and scalars is a piece each, as an interaction's handles
//! are). Were a container replaced all the same, the switch finds it: its
//! rollback finds no log it began, and the check faults (S2).

use std::cell::{Ref, RefCell, RefMut};
use std::ops::Deref;

use super::{EngineKey, ForkClone, ForkKey, ForkMap, ForkSlab, Logged, SlabId};

/// The key of a `[Fork(log)]` piece ([`engine_local!`](crate::engine_local)):
/// a collection read by `&L` and written through its own overlay API
/// ([`LogMut`]), never moved, replaced or handed out whole.
pub struct LogKey<C: 'static> {
    pub(super) key: EngineKey<C>,
}

impl<L: Logged> LogKey<RefCell<L>> {
    /// A `[Fork(log)]` piece: a collection with its own overlay or undo
    /// log ([`Logged`]).
    #[doc(hidden)]
    #[must_use]
    pub const fn logged(name: &'static str, init: fn() -> RefCell<L>) -> Self {
        #[cfg(debug_assertions)]
        let hooks = super::Hooks::Keyed {
            enter: super::log_enter::<L>,
            drain: super::log_drain::<L>,
            exit: super::log_exit::<L>,
        };
        #[cfg(debug_assertions)]
        return Self {
            key: EngineKey::new(init, Some(super::Policy { name, hooks })),
        };
        #[cfg(not(debug_assertions))]
        {
            let _ = name;
            Self {
                key: EngineKey::new(init, None),
            }
        }
    }

    /// `f` given the piece's cell, which lends the collection out by
    /// [`LogCell::borrow`] and [`LogCell::borrow_mut`] only.
    pub fn with<R>(&'static self, f: impl FnOnce(LogCell<'_, L>) -> R) -> R {
        self.key.with(|cell| f(LogCell { cell }))
    }

    /// [`Self::with`], unless the thread is ending.
    ///
    /// # Errors
    /// The thread's state is gone.
    pub fn try_with<R>(&'static self, f: impl FnOnce(LogCell<'_, L>) -> R) -> Result<R, std::thread::AccessError> {
        self.key.try_with(|cell| f(LogCell { cell }))
    }

    /// `f` given the collection to read.
    pub fn with_borrow<R>(&'static self, f: impl FnOnce(&L) -> R) -> R {
        self.key.with(|cell| f(&cell.borrow()))
    }

    /// `f` given the collection to write through its overlay API.
    pub fn with_borrow_mut<R>(&'static self, f: impl FnOnce(&mut LogMut<'_, L>) -> R) -> R {
        self.key.with(|cell| {
            f(&mut LogMut {
                inner: cell.borrow_mut(),
            })
        })
    }
}

/// A `[Fork(log)]` piece's cell, as its key lends it: the collection
/// borrowed to read or to write through its overlay, never the cell itself
/// (no `replace`, `take` or `swap`).
pub struct LogCell<'a, L> {
    cell: &'a RefCell<L>,
}

impl<L> Clone for LogCell<'_, L> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<L> Copy for LogCell<'_, L> {}

impl<'a, L> LogCell<'a, L> {
    /// The collection, to read.
    ///
    /// # Panics
    /// It is borrowed to write.
    #[must_use]
    pub fn borrow(self) -> Ref<'a, L> {
        self.cell.borrow()
    }

    /// The collection, to write through its overlay API.
    ///
    /// # Panics
    /// It is borrowed.
    #[must_use]
    pub fn borrow_mut(self) -> LogMut<'a, L> {
        LogMut {
            inner: self.cell.borrow_mut(),
        }
    }

    /// [`Self::borrow`], unless it is borrowed to write.
    ///
    /// # Errors
    /// It is borrowed to write.
    pub fn try_borrow(self) -> Result<Ref<'a, L>, std::cell::BorrowError> {
        self.cell.try_borrow()
    }

    /// [`Self::borrow_mut`], unless it is borrowed.
    ///
    /// # Errors
    /// It is borrowed.
    pub fn try_borrow_mut(self) -> Result<LogMut<'a, L>, std::cell::BorrowMutError> {
        self.cell.try_borrow_mut().map(|inner| LogMut { inner })
    }
}

/// A `[Fork(log)]` collection borrowed to write: it reads as `&L`, and
/// writes through the collection's own overlay API alone. It has no
/// `DerefMut`, so the collection, which holds both the live data and the
/// check's log, is never moved out, replaced or swapped (ADR-0034, S7).
pub struct LogMut<'a, L> {
    inner: RefMut<'a, L>,
}

impl<L> Deref for LogMut<'_, L> {
    type Target = L;
    fn deref(&self) -> &L {
        &self.inner
    }
}

impl<K: ForkKey, V: super::ForkValue> LogMut<'_, ForkMap<K, V>> {
    /// [`ForkMap::replace`].
    pub fn replace(&mut self, k: K, v: V) {
        self.inner.replace(k, v);
    }

    /// [`ForkMap::forget`].
    pub fn forget(&mut self, k: &K) {
        self.inner.forget(k);
    }
}

impl<K: ForkKey, V: ForkClone> LogMut<'_, ForkMap<K, V>> {
    /// [`ForkMap::insert`].
    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        self.inner.insert(k, v)
    }

    /// [`ForkMap::remove`].
    pub fn remove(&mut self, k: &K) -> Option<V> {
        self.inner.remove(k)
    }

    /// [`ForkMap::entry_mut`].
    pub fn entry_mut(&mut self, k: K) -> &mut V
    where
        V: Default,
    {
        self.inner.entry_mut(k)
    }
}

impl<T: super::ForkValue> LogMut<'_, ForkSlab<T>> {
    /// [`ForkSlab::insert`].
    pub fn insert(&mut self, value: T) -> SlabId {
        self.inner.insert(value)
    }

    /// [`ForkSlab::forget`].
    pub fn forget(&mut self, id: SlabId) {
        self.inner.forget(id);
    }
}

impl<T: ForkClone> LogMut<'_, ForkSlab<T>> {
    /// [`ForkSlab::remove`].
    pub fn remove(&mut self, id: SlabId) -> Option<T> {
        self.inner.remove(id)
    }
}

/// The backstop behind the type: a logged collection put in another's
/// place during a check, by the engine-private path no key exposes, faults
/// the check at its switch, named; one left in place does not.
#[cfg(all(test, debug_assertions))]
mod tests {
    use super::super::{Ended, ForkMap, ForkSlab, fork};
    use std::cell::RefCell;

    crate::engine_local! {
        [Fork(log)] static MAP: RefCell<ForkMap<u64, u8>> = RefCell::default();
        [Fork(log)] static SLAB: RefCell<ForkSlab<u8>> = RefCell::default();
    }

    /// The piece the check faulted on, if it faulted.
    fn faulted<R>(ended: &Ended<R>) -> Option<String> {
        match ended {
            Ended::Faulted(f) => Some(f.piece.clone()),
            _ => None,
        }
    }

    #[test]
    fn a_map_replaced_in_its_check_faults() {
        MAP.with_borrow_mut(|m| m.replace(1, 7));
        let ended = fork(|| MAP.key.with(|c| drop(std::mem::take(&mut *c.borrow_mut()))));
        assert!(
            faulted(&ended).is_some_and(|p| p.contains("a ForkMap replaced in its check")),
            "{:?}",
            faulted(&ended)
        );
    }

    #[test]
    fn a_slab_replaced_in_its_check_faults() {
        SLAB.with(|_| ());
        let ended = fork(|| SLAB.key.with(|c| drop(std::mem::take(&mut *c.borrow_mut()))));
        assert!(
            faulted(&ended).is_some_and(|p| p.contains("a ForkSlab replaced in its check")),
            "{:?}",
            faulted(&ended)
        );
    }

    #[test]
    fn writes_through_the_overlay_do_not_fault() {
        MAP.with_borrow_mut(|m| m.replace(1, 7));
        let id = SLAB.with_borrow_mut(|s| s.insert(1));
        let ended = fork(|| {
            MAP.with_borrow_mut(|m| {
                m.forget(&1);
                m.replace(2, 9);
            });
            SLAB.with_borrow_mut(|s| s.forget(id));
            SLAB.with_borrow_mut(|s| s.insert(2))
        });
        assert!(matches!(ended, Ended::Kept(_)), "{:?}", faulted(&ended));
        assert_eq!(
            MAP.with_borrow(|m| (m.get(&1).copied(), m.get(&2).copied())),
            (Some(7), None)
        );
        assert_eq!(SLAB.with_borrow(|s| s.read(id, |v| *v)), Some(1));
    }
}
