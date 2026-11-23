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
        let run: Run = Rc::new(f);
        let id = runtime::with(|rt| {
            let id = rt.alloc(Kind::Effect, flag::WATCHING, created_at);
            if let Some(n) = rt.live_mut(id) {
                n.run = Some(run);
            }
            rt.adopt(id);
            id
        });
        runtime::first_run_or_defer(id);
        Self { id, created_at }
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
#[track_caller]
pub fn on_cleanup(f: impl FnOnce() + 'static) {
    let at = Location::caller();
    let registered = runtime::with(|rt| {
        let owner = rt.active_owner?;
        rt.live_mut(owner)?.cleanups.push(Box::new(f));
        Some(())
    });
    if registered.is_none() {
        eprintln!("gui: `on_cleanup` at {at} has no effect, computed or scope running; it will never run");
    }
}
