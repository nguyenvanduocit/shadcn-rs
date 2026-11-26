//! Effects, watchers and cleanups.

use std::cell::RefCell;
use std::panic::Location;
use std::rc::Rc;

use super::runtime::{self, Kind, NodeId, Run, flag};

/// Code that runs now and again whenever something it read changes. It
/// lives as long as the scope (or effect, or computed) running when it was
/// made; one made outside any lives until [`Effect::dispose`].
///
/// An effect made while another runs belongs to it: the owner runs first,
/// and disposes the effects of its last run before it runs again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect {
    id: NodeId,
    created_at: &'static Location<'static>,
}

impl Effect {
    #[track_caller]
    pub fn new(f: impl Fn() + 'static) -> Self {
        let created_at = Location::caller();
        // A Shared init makes no effect, live or in a fork alike: its run
        // is code that reads state, and a fork never runs it, so allowed it
        // would differ between the two (ADR-0028, R4, F1 review 13).
        #[cfg(debug_assertions)]
        crate::engine::no_forked_read("an effect (its run reads state)");
        // It runs in the window current where it is made, as it runs as
        // its owner, whichever window's write wakes it (ADR-0015, the turn
        // rule).
        // Kept in its window ([`crate::engine::Stored`]): each run there,
        // in a turn, and none once that window closed.
        let f = crate::engine::Stored::new(crate::engine::Site::Effect, f);
        let run: Run = Rc::new(move || {
            f.run(|f| f());
        });
        let owned = crate::reactive::has_owner();
        // Making a handle: the arena's own bookkeeping
        // ([`crate::engine::allocating`]); its run below is checked.
        let id = crate::engine::allocating(|| {
            runtime::with(|rt| {
                let id = rt.alloc(Kind::Effect, flag::WATCHING, created_at);
                if let Some(n) = rt.live_mut(id) {
                    n.run = Some(run);
                }
                rt.adopt(id);
                id
            })
        });
        let effect = Self { id, created_at };
        // One no scope owns is its window's: disposed as it closes. With
        // none current it is the thread's own, which never closes.
        if !owned && let Some(window) = crate::focus::entered_window() {
            window.own_effect(effect);
        }
        runtime::first_run_or_defer(id);
        effect
    }

    /// Whether it still runs.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        runtime::with(|rt| rt.live(self.id).is_some())
    }

    /// Where it was made.
    #[must_use]
    pub fn created_at(&self) -> &'static Location<'static> {
        self.created_at
    }

    /// Stop it: its cleanups run and what it owns is disposed.
    pub fn dispose(self) {
        runtime::with(|rt| rt.dispose(self.id));
        runtime::settle();
    }
}

/// Call `callback(new, old)` whenever `source` gives a value unequal to the
/// last. `source` is tracked, `callback` is not; it is not called for the
/// first value. Returns the effect, owned like any.
#[track_caller]
pub fn watch<T: PartialEq + 'static>(
    source: impl Fn() -> T + 'static,
    callback: impl FnMut(&T, &T) + 'static,
) -> Effect {
    let last: RefCell<Option<T>> = RefCell::new(None);
    let callback = RefCell::new(callback);
    Effect::new(move || {
        let new = source();
        let old = last.borrow_mut().take();
        if let Some(old) = &old
            && *old != new
        {
            super::untracked(|| {
                if let Ok(mut cb) = callback.try_borrow_mut() {
                    cb(&new, old);
                }
            });
        }
        *last.borrow_mut() = Some(new);
    })
}

/// Run `f` before the running effect or computed runs again or is disposed,
/// or when the running scope is disposed. Cleanups run last first.
///
/// It runs in the window current where it was registered, in a turn
/// (ADR-0015): user code, reached through the gate. A window's own scopes
/// end, their cleanups run, as it closes and before it is gone; a cleanup
/// of a scope that outlived its window (a detached one a caller kept) is
/// cancelled with the window, never run in another.
#[track_caller]
pub fn on_cleanup(f: impl FnOnce() + 'static) {
    let f = crate::engine::Stored::new(crate::engine::Site::Cleanup, f);
    register(Location::caller(), move || {
        f.run_once(|f| f());
    });
}

/// [`on_cleanup`] for the engine's own bookkeeping (a timer cancelled, a
/// mailbox retired, a memo freed): no user code, run as the owner ends,
/// whatever window it was made in.
#[track_caller]
pub(crate) fn on_release(f: impl FnOnce() + 'static) {
    register(Location::caller(), f);
}

fn register(at: &'static Location<'static>, f: impl FnOnce() + 'static) {
    let registered = runtime::with(|rt| {
        let owner = rt.active_owner?;
        rt.live_mut(owner)?.cleanups.push(Box::new(f));
        Some(())
    });
    if registered.is_none() {
        eprintln!("gui: `on_cleanup` at {at} has no effect, computed or scope running; it will never run");
    }
}
