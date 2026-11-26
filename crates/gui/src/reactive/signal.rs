//! `Signal<T>`: a reactive cell, a `Copy` handle to a node in the arena.

use std::cell::RefCell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::panic::Location;
use std::rc::Rc;

use super::runtime::{self, Kind, NodeId, flag};

/// A signal's value, with what it held before it was first written since it
/// was last checked: a value written and written back within one batch wakes
/// nothing (alien-signals' current and pending values).
struct Slot<T> {
    value: T,
    before: Option<T>,
    /// Written by `update`, `replace` or `trigger`: changed, whatever it holds.
    forced: bool,
}

/// `Check` for a `T`: whether the value differs from the one before.
fn changed<T: PartialEq + 'static>(value: &dyn std::any::Any) -> bool {
    let Some(cell) = value.downcast_ref::<RefCell<Slot<T>>>() else {
        return true;
    };
    let Ok(mut slot) = cell.try_borrow_mut() else {
        return true;
    };
    let forced = std::mem::take(&mut slot.forced);
    let before = slot.before.take();
    // `T::eq` is app code run with the slot borrowed: a lend; and the old
    // value's drop (app code too) runs once the slot is let go.
    let changed = forced || crate::engine::lend(|| before.as_ref().is_none_or(|b| *b != slot.value));
    drop(slot);
    drop(before);
    changed
}

/// A reactive cell. Reading it in an effect, a computed or a render
/// subscribes the reader; a write that changes the value runs what read it.
///
/// A `Signal` is a `Copy` handle. Its value lives as long as the scope that
/// was running when it was made (or forever, made outside any). Using it
/// after that scope ended is an error that names where it was made.
///
/// It names a node of this thread's arena, so it is neither `Send` nor
/// `Sync`, nor is any reactive handle: on another thread its id would name
/// that thread's node.
///
/// ```compile_fail
/// fn send<T: Send>() {}
/// send::<gui::Signal<u32>>();
/// ```
///
/// ```compile_fail
/// fn sync<T: Sync>() {}
/// sync::<gui::Signal<u32>>();
/// ```
///
/// ```compile_fail
/// fn send<T: Send>() {}
/// send::<gui::Scope>();
/// ```
///
/// ```compile_fail
/// fn send<T: Send>() {}
/// send::<gui::Computed<u32>>();
/// ```
///
/// ```compile_fail
/// fn send<T: Send>() {}
/// send::<gui::Effect>();
/// ```
pub struct Signal<T> {
    id: NodeId,
    created_at: &'static Location<'static>,
    ty: PhantomData<fn() -> T>,
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Signal<T> {}

impl<T> PartialEq for Signal<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> Eq for Signal<T> {}

impl<T> Hash for Signal<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: fmt::Debug + 'static> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_alive() {
            self.with_untracked(|v| f.debug_tuple("Signal").field(v).finish())
        } else {
            write!(f, "Signal(disposed, made at {})", self.created_at)
        }
    }
}

impl<T: Clone + Default + 'static> Default for Signal<T> {
    #[track_caller]
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Clone + fmt::Debug + 'static> Signal<T> {
    /// A signal that traces and debug reports show by `label` instead of its
    /// id, and whose value the devtools read ([`super::probes`]) and, for a
    /// plain value (a number, `bool`, `char` or `String`), write.
    #[track_caller]
    pub fn labeled(label: &'static str, value: T) -> Self {
        let signal = Self::create(value, Some(label), Location::caller());
        super::trace::register_probe::<T>(signal.id);
        signal
    }
}

impl<T: Clone + 'static> Signal<T> {
    /// A signal holding `value`. `T: Clone` (ADR-0026, ADR-0028 "What
    /// lands at K4a"): a signal holds a value, not a handle, so devtools
    /// and stepping back can keep a copy of it.
    ///
    /// ```compile_fail
    /// struct Handle;
    /// let _ = gui::reactive::Signal::new(Handle);
    /// ```
    #[track_caller]
    pub fn new(value: T) -> Self {
        Self::create(value, None, Location::caller())
    }
}

impl<T: 'static> Signal<T> {
    fn create(value: T, label: Option<&'static str>, created_at: &'static Location<'static>) -> Self {
        let cell: Rc<RefCell<Slot<T>>> = Rc::new(RefCell::new(Slot {
            value,
            before: None,
            forced: false,
        }));
        // Making a handle: the arena's own bookkeeping ([`crate::engine::allocating`]).
        let id = crate::engine::allocating(|| {
            runtime::with(|rt| {
                let id = rt.alloc(Kind::Signal, flag::MUTABLE, created_at);
                if let Some(n) = rt.live_mut(id) {
                    n.value = Some(cell);
                    n.label = label;
                }
                rt.adopt(id);
                id
            })
        });
        Self {
            id,
            created_at,
            ty: PhantomData,
        }
    }

    /// The handle of the live signal `id` holding a `T`.
    pub(crate) fn from_id(id: NodeId) -> Option<Self> {
        runtime::with(|rt| {
            let n = rt.live(id)?;
            let typed = n.value.as_ref()?.is::<RefCell<Slot<T>>>();
            (n.kind == Kind::Signal && typed).then_some(Self {
                id,
                created_at: n.created_at,
                ty: PhantomData,
            })
        })
    }

    /// Its slot and generation: which signal it is, as a map keys it.
    pub(crate) const fn key(&self) -> (usize, u32) {
        (self.id.index, self.id.generation)
    }

    /// Whether its scope still lives.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        runtime::with(|rt| rt.live(self.id).is_some())
    }

    /// Where it was made.
    #[must_use]
    pub fn created_at(&self) -> &'static Location<'static> {
        self.created_at
    }

    /// The slot, and whether the signal was written since it was last
    /// checked. A Shared init reads none: [`runtime::with`] refuses it
    /// (ADR-0028, R4).
    fn slot(&self) -> (Rc<RefCell<Slot<T>>>, bool) {
        let found = runtime::with(|rt| {
            let value = rt.live(self.id)?.value.clone()?;
            Some((value, runtime::is_dirty(rt, self.id.index)))
        });
        match found.map(|(v, dirty)| (Rc::downcast::<RefCell<Slot<T>>>(v), dirty)) {
            Some((Ok(cell), dirty)) => (cell, dirty),
            _ => runtime::disposed("signal", self.created_at),
        }
    }

    /// The slot to read, checked first, subscribing the running reader if `track`.
    fn read(&self, track: bool) -> Rc<RefCell<Slot<T>>> {
        let (cell, dirty) = self.slot();
        let i = self.id.index;
        if dirty {
            runtime::settle_signal(i);
        }
        if track {
            runtime::with(|rt| {
                rt.track(i);
                rt.record_read(i);
            });
        }
        cell
    }

    /// Borrow the value, subscribing the running reader.
    ///
    /// What `f` writes runs its effects after `f` returns, never while the
    /// value is borrowed.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let cell = self.read(true);
        let _batch = runtime::BatchGuard::new();
        let slot = borrow(&cell, self.created_at);
        // A lend to user code: no check opens inside it (ADR-0035, D4).
        crate::engine::lend(|| f(&slot.value))
    }

    /// Borrow the value without subscribing anything.
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let cell = self.read(false);
        let _batch = runtime::BatchGuard::new();
        let slot = borrow(&cell, self.created_at);
        // A lend to user code: no check opens inside it (ADR-0035, D4).
        crate::engine::lend(|| f(&slot.value))
    }

    /// The value, subscribing the running reader.
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    /// The value, without subscribing anything.
    pub fn get_untracked(&self) -> T
    where
        T: Clone,
    {
        self.with_untracked(T::clone)
    }

    /// Write `value`; what read the signal runs only if it differs from what
    /// it held (and, at the end of a batch, from what it held before it).
    pub fn set(&self, value: T)
    where
        T: PartialEq,
    {
        let (cell, dirty) = self.slot();
        let old = {
            let mut slot = borrow_mut(&cell, self.created_at);
            // `T::eq` is app code run with the slot borrowed: a lend.
            if crate::engine::lend(|| slot.value == value) {
                return;
            }
            runtime::check_write(self.id.index);
            if runtime::cold_refuses(self.id) {
                return;
            }
            let old = std::mem::replace(&mut slot.value, value);
            if dirty { Some(old) } else { slot.before.replace(old) }
        };
        drop(old);
        runtime::with(|rt| {
            if let Some(n) = rt.live_mut(self.id) {
                n.check = Some(changed::<T>);
            }
        });
        self.notify();
    }

    /// Write `value` and run what read the signal, equal or not.
    pub fn replace(&self, value: T) {
        runtime::check_write(self.id.index);
        if runtime::cold_refuses(self.id) {
            return;
        }
        let (cell, _) = self.slot();
        let old = {
            let mut slot = borrow_mut(&cell, self.created_at);
            slot.forced = true;
            std::mem::replace(&mut slot.value, value)
        };
        drop(old);
        self.notify();
    }

    /// Change the value in place; what read it runs, once `f` has returned
    /// and the value is no longer borrowed (so do effects of what `f` writes).
    /// Store `value` without waking anything: only for a value equal to the
    /// current one (a component's props whose `Callback`s are new).
    pub(crate) fn store_quiet(&self, value: T) {
        if runtime::cold_refuses(self.id) {
            return;
        }
        let (cell, _) = self.slot();
        borrow_mut(&cell, self.created_at).value = value;
    }

    /// The engine passes `value` (a component's props) in a cold frame: the
    /// frame's own copy of the signal holds it, the live value untouched,
    /// and what read it is told, as by a write.
    #[cfg(debug_assertions)]
    pub(crate) fn store_cold(&self, value: T) {
        runtime::cold_owns(self.id);
        runtime::replace_value(
            self.id,
            Rc::new(RefCell::new(Slot {
                value,
                before: None,
                forced: true,
            })),
        );
        runtime::with(|rt| {
            if let Some(n) = rt.live_mut(self.id) {
                n.check = None;
            }
        });
        self.notify();
    }

    pub fn update(&self, f: impl FnOnce(&mut T)) {
        runtime::check_write(self.id.index);
        if runtime::cold_refuses(self.id) {
            return;
        }
        let (cell, _) = self.slot();
        let _batch = runtime::BatchGuard::new();
        {
            let mut slot = borrow_mut(&cell, self.created_at);
            slot.forced = true;
            // A lend to user code: no check opens inside it (ADR-0035, D4).
            crate::engine::lend(|| f(&mut slot.value));
        }
        self.notify();
    }

    /// Run what read the signal, as if it had changed.
    pub fn trigger(&self) {
        runtime::check_write(self.id.index);
        if runtime::cold_refuses(self.id) {
            return;
        }
        let (cell, _) = self.slot();
        borrow_mut(&cell, self.created_at).forced = true;
        self.notify();
    }

    /// Whether a fork made it (tests).
    #[cfg(test)]
    pub(crate) fn made_in_fork(self) -> bool {
        self.id.made_in_fork()
    }

    /// Its node's id (tests: the id streams of ADR-0034, S6).
    #[cfg(test)]
    pub(crate) fn node_id(self) -> NodeId {
        self.id
    }

    /// Dispose it if the runtime can be reached now (from a `Drop`, which
    /// may run while the thread ends). While the thread unwinds, what
    /// leaving runs waits for the next settle.
    pub(crate) fn dispose_quietly(self) {
        if runtime::try_with(|rt| rt.dispose(self.id)).is_some() && !std::thread::panicking() {
            runtime::settle();
        }
    }

    /// Take the signal out of the graph now, before its scope ends.
    pub fn dispose(self) {
        runtime::with(|rt| rt.dispose(self.id));
        runtime::settle();
    }

    fn notify(&self) {
        let i = self.id.index;
        let flush = runtime::with(|rt| {
            rt.record_write(i);
            rt.wrote_signal(i)
        });
        if flush {
            runtime::flush();
        }
    }
}

fn borrow<'a, T>(cell: &'a RefCell<T>, created_at: &'static Location<'static>) -> std::cell::Ref<'a, T> {
    match cell.try_borrow() {
        Ok(value) => value,
        Err(_) => runtime::misuse(&format!(
            "the signal made at {created_at} was read while its own `update` changes it"
        )),
    }
}

fn borrow_mut<'a, T>(cell: &'a RefCell<T>, created_at: &'static Location<'static>) -> std::cell::RefMut<'a, T> {
    match cell.try_borrow_mut() {
        Ok(value) => value,
        Err(_) => runtime::misuse(&format!(
            "the signal made at {created_at} was written while it is being read (inside its own `with`) or changed"
        )),
    }
}
