//! `Computed<T>`: a value derived from others, lazy and cached.

use std::cell::RefCell;
use std::fmt;
use std::marker::PhantomData;
use std::panic::Location;
use std::rc::Rc;

use super::runtime::{self, Kind, NodeId, Update, flag};

/// A value computed from signals and other computeds. It computes when read
/// and is cached until something it read changes; a recomputation that gives
/// an equal value does not wake its readers. With no reader left it lets go
/// of what it read, and computes again when next read.
pub struct Computed<T> {
    id: NodeId,
    created_at: &'static Location<'static>,
    ty: PhantomData<fn() -> T>,
}

impl<T> Clone for Computed<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Computed<T> {}

impl<T> PartialEq for Computed<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> Eq for Computed<T> {}

impl<T> fmt::Debug for Computed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Computed(made at {})", self.created_at)
    }
}

type Cell<T> = RefCell<Option<T>>;

impl<T: PartialEq + 'static> Computed<T> {
    #[track_caller]
    pub fn new(f: impl Fn() -> T + 'static) -> Self {
        let created_at = Location::caller();
        let cell: Rc<Cell<T>> = Rc::new(RefCell::new(None));
        let store = Rc::clone(&cell);
        let update: Update = Rc::new(move || {
            let new = f();
            let Ok(mut slot) = store.try_borrow_mut() else {
                runtime::misuse(&format!("the computed made at {created_at} read itself"));
            };
            if slot.as_ref() == Some(&new) {
                return false;
            }
            let old = slot.replace(new);
            drop(slot);
            drop(old);
            true
        });
        let id = runtime::with(|rt| {
            let id = rt.alloc(Kind::Computed, flag::NONE, created_at);
            if let Some(n) = rt.live_mut(id) {
                n.value = Some(cell);
                n.update = Some(update);
            }
            rt.adopt(id);
            id
        });
        Self {
            id,
            created_at,
            ty: PhantomData,
        }
    }
}

impl<T: 'static> Computed<T> {
    /// Up to date, and subscribed by the running reader.
    fn fresh(&self) -> Rc<Cell<T>> {
        let i = self.id.index;
        let value = runtime::with(|rt| rt.live(self.id).and_then(|n| n.value.clone()));
        let Some(Ok(cell)) = value.map(Rc::downcast::<Cell<T>>) else {
            runtime::disposed("computed", self.created_at);
        };
        runtime::refresh_computed(i);
        runtime::with(|rt| rt.track(i));
        cell
    }

    /// Borrow the value, subscribing the running reader.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let cell = self.fresh();
        let _batch = runtime::BatchGuard::new();
        let value = cell.try_borrow().ok();
        match value.as_deref() {
            Some(Some(v)) => f(v),
            _ => runtime::misuse(&format!(
                "the computed made at {} was read while it computes (it reads itself)",
                self.created_at
            )),
        }
    }

    /// The value, subscribing the running reader.
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    /// The value without subscribing anything.
    pub fn get_untracked(&self) -> T
    where
        T: Clone,
    {
        super::untracked(|| self.get())
    }

    /// Whether its scope still lives.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        runtime::with(|rt| rt.live(self.id).is_some())
    }

    /// Take it out of the graph now, before its scope ends.
    pub fn dispose(self) {
        runtime::with(|rt| rt.dispose(self.id));
        runtime::settle();
    }
}
