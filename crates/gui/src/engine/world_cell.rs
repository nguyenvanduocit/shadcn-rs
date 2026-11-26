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
//! A world is named by [`super::world`], the one source in every build
//! (ADR-0035, D5): process-wide, so a cell sent to another thread matches
//! no check there.
//!
//! A [`WorldRefCell`] lends its value only to a closure (`with`,
//! `with_mut`), never as a guard or a reference, not even to its one
//! owner (ADR-0035, D4): no borrow outlives the call, so none can be
//! forgotten or held across a check's switch. A check's body is `'static`
//! (`gui::testing::in_check`), so no lent borrow can be carried into one,
//! and a check opened inside a lend is refused (`Skipped(InUserCode)`,
//! ADR-0035 D4's run-time rule). Its `Debug` names it and runs no payload
//! code.
//!
//! ```compile_fail,E0599
//! // Codex r19-3's reproducer: a guard taken live and written in a check.
//! let actions = gui::window::decoration::Actions::default();
//! let mut queue = actions.borrow_mut();
//! gui::testing::in_check(|| queue.push(gui::window::decoration::WindowAction::Close));
//! ```
//!
//! ```compile_fail,E0599
//! let actions = gui::window::decoration::Actions::default();
//! let sampled = gui::testing::in_check(|| actions.borrow());
//! ```
//!
//! ```compile_fail,E0599
//! let cell = gui::engine::WorldRefCell::new("a list", vec![1]);
//! std::mem::forget(cell.try_borrow_mut());
//! ```
//!
//! ```compile_fail,E0624
//! // Codex F1b r1-1's reproducer: its owner's `&mut` held into a check.
//! use gui::window::decoration::{Actions, WindowAction};
//! let mut actions = Actions::default();
//! let queue = std::rc::Rc::get_mut(&mut actions).unwrap().get_mut();
//! let _ = gui::testing::check(|| queue.push(WindowAction::Close));
//! ```

use std::cell::{Cell, RefCell};
use std::fmt;

/// The running world ([`super::world`]).
fn now() -> u64 {
    super::world()
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
/// let count = std::rc::Rc::new(gui::engine::WorldCell::new("clicks", 0_u32));
/// count.set(count.get() + 1);
/// let held = std::rc::Rc::clone(&count);
/// let refused = gui::testing::in_check(move || {
///     std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| held.set(9))).is_err()
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

    /// `f` given the value to read: free in every world.
    ///
    /// # Panics
    /// While it is written.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        super::lend(|| f(&self.cell.borrow()))
    }

    /// [`Self::with`], `None` while it is written.
    pub fn try_with<R>(&self, f: impl FnOnce(&T) -> R) -> Option<R> {
        super::lend(|| self.cell.try_borrow().ok().map(|v| f(&v)))
    }

    /// `f` given the value to write.
    ///
    /// # Panics
    /// From a world that did not make it, or while it is borrowed.
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        self.writes();
        super::lend(|| f(&mut self.cell.borrow_mut()))
    }

    /// [`Self::with_mut`], `None` while it is borrowed.
    ///
    /// # Panics
    /// From a world that did not make it.
    pub fn try_with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        self.writes();
        super::lend(|| self.cell.try_borrow_mut().ok().map(|mut v| f(&mut v)))
    }

    /// Store `value`, giving back the old one.
    ///
    /// # Panics
    /// From a world that did not make it, or while it is borrowed.
    pub fn replace(&self, value: T) -> T {
        self.writes();
        self.cell.replace(value)
    }

    /// Its value, written through its one owner: the engine's alone (no
    /// reference reaches app code, ADR-0035, D4; engine code opens no
    /// check while it holds one).
    ///
    /// # Panics
    /// From a world that did not make it.
    pub(crate) fn get_mut(&mut self) -> &mut T {
        self.writes();
        self.cell.get_mut()
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
        Self::new(self.what, self.with(T::clone))
    }
}

impl<T: Default> Default for WorldRefCell<T> {
    /// Of the running world, named by its type.
    fn default() -> Self {
        Self::new(std::any::type_name::<T>(), T::default())
    }
}

/// By its name and world alone: formatting runs no payload code (nor the
/// caller's writer) while the value is borrowed (ADR-0035 D4).
impl<T> fmt::Debug for WorldRefCell<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorldRefCell")
            .field("what", &self.what)
            .field("world", &self.world)
            .finish_non_exhaustive()
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
        list.with_mut(|l| l.push(2));
        let wrote = crate::engine::in_cold_frame(|| {
            assert_eq!((live.get(), list.with(Vec::len)), (2, 2), "a check reads live");
            let own = WorldCell::new("the check's", 0_u32);
            own.set(1);
            let cell = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| live.set(9)));
            let vec = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| list.with_mut(|l| l.push(9))));
            (own.get(), cell.is_err(), vec.is_err(), live.is_here())
        });
        assert_eq!(
            wrote,
            (1, true, true, false),
            "the check wrote its own and was refused the live"
        );
        assert_eq!(
            (live.get(), list.with(Clone::clone)),
            (2, vec![1, 2]),
            "nothing live changed"
        );
    }

    /// A check opened inside a lend is refused, its body never run.
    #[test]
    fn a_check_inside_a_lend_is_refused() {
        let list = WorldRefCell::new("live list", vec![1]);
        let ended = list.with_mut(|l| crate::engine::fork(|| l.push(9)));
        assert!(
            matches!(ended, crate::engine::Ended::Dropped(why) if why == super::super::frame::BORROWED),
            "refused"
        );
        assert_eq!(list.with(Clone::clone), vec![1]);
        assert!(
            matches!(crate::engine::fork(|| 1), crate::engine::Ended::Kept(1)),
            "free again"
        );
    }

    /// The world follows the check (one source, ADR-0035, D5).
    #[test]
    fn the_world_follows_the_check() {
        assert_eq!(super::super::world(), super::super::LIVE);
        let inside = crate::engine::in_cold_frame(|| (super::super::world(), crate::engine::is_cold()));
        assert!(inside.0 != super::super::LIVE && inside.1);
        assert_eq!(super::super::world(), super::super::LIVE);
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
