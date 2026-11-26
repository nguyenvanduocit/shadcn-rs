//! Cells of one world (ADR-0034, S7, review r18, decision 2): interior
//! mutability for state that is not a registry piece's but belongs to the
//! world that made it (a layout tree's lists, a `Members` list, a
//! registry's read count, a component's setup state its handlers write).
//! Its world is stated by the type, not by a reason in an allowlist: made
//! live, it is written live only; made by a check, by that check only. A
//! write from another world is a bug of the writer, and it panics there,
//! naming the cell, before the write lands (a check that wrote a live cell
//! through `&` would change the live world with no log to roll back).
//! Reading is free in every world: a check reads the live value as it
//! reads any live state.
//!
//! A copy (`Clone`) is the copier's world's: a check's copy of a live value
//! holding one is the check's own.
//!
//! A release build runs no check ([`super::forking`] is `false`), so every
//! cell is live there and the test is `0 == 0`.

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::fmt;

/// The running world: 0 live, else the running check's serial.
#[cfg(debug_assertions)]
fn now() -> u64 {
    super::world()
}

/// A release build runs live alone.
#[cfg(not(debug_assertions))]
const fn now() -> u64 {
    0
}

/// The refusal of a write from another world.
#[expect(
    clippy::panic,
    reason = "a check writing state of the world that made it is a bug in its caller, as `cold_refuses` refuses a live signal's write"
)]
fn refused(what: &str) -> ! {
    panic!(
        "gui: a check wrote `{what}`, state of the world that made it (ADR-0034, S7): write it in that world, or keep it in a registry piece"
    )
}

/// A [`Cell`] of the world that made it: written in that world alone (the
/// [module](self)'s rule). For state a component's setup makes and its
/// handlers write, which a check must never write live.
///
/// ```
/// let count = gui::engine::WorldCell::new("clicks", 0_u32);
/// count.set(count.get() + 1);
/// let refused = gui::testing::in_check(|| {
///     std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| count.set(9))).is_err()
/// });
/// assert!(refused);
/// assert_eq!(count.get(), 1);
/// ```
pub struct WorldCell<T> {
    world: u64,
    what: &'static str,
    cell: Cell<T>,
}

impl<T> WorldCell<T> {
    /// `value`, of the running world; `what` names it in a refusal.
    #[must_use]
    pub fn new(what: &'static str, value: T) -> Self {
        Self {
            world: now(),
            what,
            cell: Cell::new(value),
        }
    }

    /// Whether the running world made it, so may write it.
    #[must_use]
    pub fn is_here(&self) -> bool {
        self.world == now()
    }

    /// A write: refused (a panic) from a world that did not make it.
    fn writes(&self) {
        if !self.is_here() {
            refused(self.what);
        }
    }

    /// Store `value`.
    ///
    /// # Panics
    /// From a world that did not make it.
    pub fn set(&self, value: T) {
        self.writes();
        self.cell.set(value);
    }

    /// Store `value`, giving back the old one.
    ///
    /// # Panics
    /// From a world that did not make it.
    pub fn replace(&self, value: T) -> T {
        self.writes();
        self.cell.replace(value)
    }

    /// Take the value, leaving the default.
    ///
    /// # Panics
    /// From a world that did not make it.
    pub fn take(&self) -> T
    where
        T: Default,
    {
        self.writes();
        self.cell.take()
    }

    /// Its value (a read: free in every world).
    #[must_use]
    pub fn get(&self) -> T
    where
        T: Copy,
    {
        self.cell.get()
    }
}

impl<T: Copy> Clone for WorldCell<T> {
    /// The copier's world's.
    fn clone(&self) -> Self {
        Self::new(self.what, self.get())
    }
}

impl<T: Default> Default for WorldCell<T> {
    /// Of the running world, named by its type.
    fn default() -> Self {
        Self::new(std::any::type_name::<T>(), T::default())
    }
}

impl<T: Copy + fmt::Debug> fmt::Debug for WorldCell<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

/// A [`RefCell`] of the world that made it: written in that world alone
/// (the [module](self)'s rule).
pub struct WorldRefCell<T> {
    world: u64,
    what: &'static str,
    cell: RefCell<T>,
}

impl<T> WorldRefCell<T> {
    /// `value`, of the running world; `what` names it in a refusal.
    #[must_use]
    pub fn new(what: &'static str, value: T) -> Self {
        Self {
            world: now(),
            what,
            cell: RefCell::new(value),
        }
    }

    /// Whether the running world made it, so may write it.
    #[must_use]
    pub fn is_here(&self) -> bool {
        self.world == now()
    }

    fn writes(&self) {
        if !self.is_here() {
            refused(self.what);
        }
    }

    /// A read: free in every world.
    ///
    /// # Panics
    /// While it is written.
    pub fn borrow(&self) -> Ref<'_, T> {
        self.cell.borrow()
    }

    /// [`Self::borrow`], `Err` while it is written.
    ///
    /// # Errors
    /// While it is written.
    pub fn try_borrow(&self) -> Result<Ref<'_, T>, std::cell::BorrowError> {
        self.cell.try_borrow()
    }

    /// A write.
    ///
    /// # Panics
    /// From a world that did not make it, or while it is borrowed.
    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        self.writes();
        self.cell.borrow_mut()
    }

    /// Its value, written through its one owner.
    ///
    /// # Panics
    /// From a world that did not make it.
    pub fn get_mut(&mut self) -> &mut T {
        self.writes();
        self.cell.get_mut()
    }

    /// [`Self::borrow_mut`], `Err` while it is borrowed.
    ///
    /// # Errors
    /// While it is borrowed.
    ///
    /// # Panics
    /// From a world that did not make it.
    pub fn try_borrow_mut(&self) -> Result<RefMut<'_, T>, std::cell::BorrowMutError> {
        self.writes();
        self.cell.try_borrow_mut()
    }

    /// Store `value`, giving back the old one.
    ///
    /// # Panics
    /// From a world that did not make it, or while it is borrowed.
    pub fn replace(&self, value: T) -> T {
        self.writes();
        self.cell.replace(value)
    }

    /// Take the value, leaving the default.
    ///
    /// # Panics
    /// From a world that did not make it, or while it is borrowed.
    pub fn take(&self) -> T
    where
        T: Default,
    {
        self.writes();
        self.cell.take()
    }
}

impl<T: Clone> Clone for WorldRefCell<T> {
    /// The copier's world's.
    fn clone(&self) -> Self {
        Self::new(self.what, self.borrow().clone())
    }
}

impl<T: Default> Default for WorldRefCell<T> {
    /// Of the running world, named by its type.
    fn default() -> Self {
        Self::new(std::any::type_name::<T>(), T::default())
    }
}

impl<T: fmt::Debug> fmt::Debug for WorldRefCell<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cell.fmt(f)
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::{WorldCell, WorldRefCell};

    /// Made live: written live, read anywhere, and a check's write refused
    /// before it lands. Made by a check: the check's to write.
    #[test]
    fn a_cell_is_written_in_its_own_world_alone() {
        let live = WorldCell::new("live count", 1_u32);
        let list = WorldRefCell::new("live list", vec![1]);
        live.set(2);
        list.borrow_mut().push(2);
        let wrote = crate::engine::in_cold_frame(|| {
            assert_eq!((live.get(), list.borrow().len()), (2, 2), "a check reads live");
            let own = WorldCell::new("the check's", 0_u32);
            own.set(1);
            let cell = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| live.set(9)));
            let vec = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| list.borrow_mut().push(9)));
            (own.get(), cell.is_err(), vec.is_err(), live.is_here())
        });
        assert_eq!(
            wrote,
            (1, true, true, false),
            "the check wrote its own and was refused the live"
        );
        assert_eq!(
            (live.get(), list.borrow().clone()),
            (2, vec![1, 2]),
            "nothing live changed"
        );
    }

    /// A check's copy of a live cell is the check's.
    #[test]
    fn a_copy_is_the_copiers() {
        let live = WorldCell::new("live", 1_u32);
        let copied = crate::engine::in_cold_frame(|| {
            let mine = live.clone();
            mine.set(5);
            mine.get()
        });
        assert_eq!((copied, live.get()), (5, 1));
    }
}
