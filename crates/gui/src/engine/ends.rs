//! The one engine `Drop` that does engine work (ADR-0035, D1): a value
//! whose end disposes reactive nodes, writes a registry, closes a window or
//! a worker is an [`Ending`], held in an [`Ends`], which records the world
//! that made it and ends it there, whoever lets it go:
//!
//! - in the running world, when that is its own: `end` runs in place;
//! - live, while a check runs: the whole value is handed to the live
//!   release queue (ADR-0034, S5), so `end` and every field's drop run
//!   after the switch back, in live, in order with the other releases;
//! - in an expired world (a check's value that escaped into app heap): its
//!   memory alone is dropped, as that world's state went with its rollback.
//!
//! No other code chooses a world for a cleanup. A plain `impl Drop` does no
//! engine work (the obligations lint, `crates/gui/tests/obligations.rs`).
//! Leptos's sandboxed owners install their own arena in their `Drop`
//! (`references/leptos/reactive_graph/src/owner.rs:582-600`); its failure
//! report (fields dropping after the restore, `:586`) is why the whole value
//! moves here, not a cleanup beside it.

use std::ops::{Deref, DerefMut};

/// A value's end: engine work run in the world that made it ([`Ends`]).
/// `end` acts on the running world, which is its own by construction, so it
/// may read, decide and write in one step (a window's close, a node's flags
/// with their `BOUND` row).
pub(crate) trait Ending: 'static {
    /// End it, in its own world.
    fn end(self);
}

/// An [`Ending`] value and the world that made it ([`super::world`]): its
/// drop ends it there (the [module](self)'s rule). Reads and writes go
/// through it as through the value.
pub(crate) struct Ends<T: Ending> {
    world: u64,
    value: Option<T>,
}

impl<T: Ending> Ends<T> {
    /// `value`, of the running world.
    pub(crate) fn new(value: T) -> Self {
        Self {
            world: super::world(),
            value: Some(value),
        }
    }

    /// The value. Present from `new` until the drop takes it.
    fn get(&self) -> &T {
        match &self.value {
            Some(v) => v,
            None => unreachable_taken(),
        }
    }

    fn get_mut(&mut self) -> &mut T {
        match &mut self.value {
            Some(v) => v,
            None => unreachable_taken(),
        }
    }
}

/// The value is taken out by the drop alone, after which nothing reads it.
#[expect(clippy::panic, reason = "an `Ends` read after its drop took the value: unreachable")]
#[cold]
fn unreachable_taken() -> ! {
    panic!("gui: an `Ends` read after its drop")
}

impl<T: Ending> Deref for Ends<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.get()
    }
}

impl<T: Ending> DerefMut for Ends<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.get_mut()
    }
}

impl<T: Ending + Default> Default for Ends<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Ending> Drop for Ends<T> {
    /// The only routing (ADR-0035, D1).
    fn drop(&mut self) {
        let Some(value) = self.value.take() else { return };
        route(self.world, super::world(), value);
    }
}

/// End `value`, made in `made`, from world `now`.
fn route<T: Ending>(made: u64, now: u64, value: T) {
    if made == now {
        value.end();
    } else if made == super::LIVE {
        super::release::when_live(move || value.end());
    } else {
        // Its world expired: its state went with the rollback. Its fields
        // drop here, each `Ends` among them routed by its own world.
        drop(value);
    }
}

/// A node one value owns alone (a layout node's tracker, a hub's): disposed
/// as that value ends.
impl Ending for crate::reactive::RenderNode {
    fn end(self) {
        self.dispose();
    }
}

#[cfg(test)]
#[cfg(debug_assertions)]
mod tests {
    use super::{Ending, Ends};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Records the world each end ran in.
    struct Noted(Rc<RefCell<Vec<u64>>>);

    impl Ending for Noted {
        fn end(self) {
            self.0.borrow_mut().push(crate::engine::world());
        }
    }

    /// Live, dropped live: in place. Live, dropped in a check: live, after
    /// the switch. A check's, dropped in it: in it. A check's that escaped,
    /// dropped live: never.
    #[test]
    fn a_value_ends_in_the_world_that_made_it() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        drop(Ends::new(Noted(Rc::clone(&seen))));
        assert_eq!(*seen.borrow(), [0], "live, in place");
        let live = Ends::new(Noted(Rc::clone(&seen)));
        let s = Rc::clone(&seen);
        let (in_check, escaped) = crate::engine::in_cold_frame(move || {
            drop(live);
            assert_eq!(s.borrow().len(), 1, "a live value's end waits for live");
            drop(Ends::new(Noted(Rc::clone(&s))));
            let own = s.borrow().last().copied();
            (own, Ends::new(Noted(Rc::clone(&s))))
        });
        assert!(in_check.is_some_and(|w| w != 0), "a check's own, in it");
        assert_eq!(seen.borrow().len(), 2, "queued live");
        crate::engine::run_releases();
        assert_eq!(seen.borrow().len(), 3, "the live one, ended live after the switch");
        assert_eq!(seen.borrow().get(2), Some(&0));
        drop(escaped);
        assert_eq!(seen.borrow().len(), 3, "an expired world's value ends nowhere");
    }
}
