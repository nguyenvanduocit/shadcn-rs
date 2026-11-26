//! The ways out of the UI thread's state (ADR-0028, "The outbox"): a
//! [`Port`] is a typed way out whose `deliver` runs on the UI thread, and
//! [`spawn`] is work off the UI thread, owned by the scope that started it
//! and cancelled with it.
//!
//! At K4a the ports deliver at once and nothing is held back: outside any
//! fork, [`Port::post`] calls `deliver` now. Inside a fork (a check) a
//! post reaches nothing and a job never starts, so a check has no effect
//! on the outside world (ADR-0028, rule 5); F3 routes the same calls
//! through the outbox, with no change to component code.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::engine::{Site, Stored};

/// A typed way out, owned by the scope that made it ([`port`]). `Copy`.
pub struct Port<M: 'static> {
    id: u64,
    ty: PhantomData<fn(M)>,
}

impl<M: 'static> Clone for Port<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M: 'static> Copy for Port<M> {}

impl<M: 'static> PartialEq for Port<M> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<M: 'static> Eq for Port<M> {}

impl<M: 'static> fmt::Debug for Port<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Port").field(&self.id).finish()
    }
}

/// A port's `deliver`, the caller's: kept in a [`Stored`].
type Deliver<M> = Box<dyn FnMut(M)>;

/// One port: its `deliver` (kept in the window it was made in), the posts
/// made while it runs, and whether it runs.
struct Entry<M: 'static> {
    deliver: Stored<RefCell<Deliver<M>>>,
    /// Posts made while `deliver` runs: delivered as it returns, never by
    /// entering it again. Empty, it holds no allocation.
    queued: RefCell<VecDeque<M>>,
    running: Cell<bool>,
}

crate::engine::engine_local! {
    /// Each open port, by id. A fork sees none of the live ones (what it
    /// posts reaches nothing) and what it opens goes with it.
    [Fresh] static PORTS: RefCell<Vec<(u64, Rc<dyn Any>)>> = const { RefCell::new(Vec::new()) };
    /// The window each port was made in, by id: retired as it closes
    /// ([`close_window`]).
    [Fresh] static MADE_IN: RefCell<Vec<(u64, Stored<()>)>> = const { RefCell::new(Vec::new()) };
}

/// The ports made, numbered.
static NEXT: crate::engine::IdKind = crate::engine::IdKind::new("port");

/// A port whose `deliver` runs on the UI thread, with no fork open, for
/// each message posted to it ([`Port::post`]).
///
/// Made while a scope runs, it closes as that scope ends; made in a window's
/// code, as the window closes. A post to a closed port is dropped.
///
/// # Panics
/// In a debug build, with neither a scope running nor a window's code (a
/// test's top level): nothing would close it.
#[track_caller]
pub fn port<M: 'static>(deliver: impl FnMut(M) + 'static) -> Port<M> {
    let owner = crate::engine::owner("gui::port");
    let id = crate::engine::unique_id(&NEXT);
    let deliver: Deliver<M> = Box::new(deliver);
    let entry: Rc<dyn Any> = Rc::new(Entry {
        deliver: Stored::new(Site::Mail, RefCell::new(deliver)),
        queued: RefCell::new(VecDeque::new()),
        running: Cell::new(false),
    });
    PORTS.with(|p| p.borrow_mut().push((id, entry)));
    MADE_IN.with(|m| m.borrow_mut().push((id, Stored::new(Site::Mail, ()))));
    match owner {
        crate::engine::Owner::Scope => crate::reactive::on_release(move || retire(id)),
        crate::engine::Owner::Window | crate::engine::Owner::Nothing => {}
    }
    Port { id, ty: PhantomData }
}

/// Port `id`'s entry, while it is open.
fn entry(id: u64) -> Option<Rc<dyn Any>> {
    PORTS
        .try_with(|p| p.borrow().iter().find(|(of, _)| *of == id).map(|(_, e)| Rc::clone(e)))
        .ok()
        .flatten()
}

/// Close port `id` and let go of its `deliver` and what waits for it, once
/// the lists are let go of (a drop that makes or closes a port finds them
/// free).
fn retire(id: u64) {
    let entry = PORTS
        .try_with(|p| {
            let mut p = p.borrow_mut();
            let i = p.iter().position(|(of, _)| *of == id)?;
            Some(p.remove(i))
        })
        .ok()
        .flatten();
    let made = MADE_IN
        .try_with(|m| {
            let mut m = m.borrow_mut();
            let i = m.iter().position(|(of, _)| *of == id)?;
            Some(m.remove(i))
        })
        .ok()
        .flatten();
    drop((entry, made));
}

/// `window` closes: the ports made in it are retired. How many were.
pub(crate) fn close_window(window: &crate::focus::FocusRoot) -> usize {
    let ids: Vec<u64> = MADE_IN
        .try_with(|m| {
            m.borrow()
                .iter()
                .filter(|(_, made)| made.is_of(window))
                .map(|(id, _)| *id)
                .collect()
        })
        .unwrap_or_default();
    let n = ids.len();
    for id in ids {
        crate::engine::contained(|| retire(id));
    }
    n
}

impl<M: 'static> Port<M> {
    /// Deliver `message`: now, outside any fork. A post made while this
    /// port's `deliver` runs is delivered as it returns, never by entering
    /// it again. Inside a fork, or to a closed port, it is dropped.
    pub fn post(self, message: M) {
        if crate::engine::forking() {
            drop(message);
            return;
        }
        let Some(open) = entry(self.id) else {
            return;
        };
        let Ok(open) = open.downcast::<Entry<M>>() else {
            return;
        };
        if open.running.get() {
            open.queued.borrow_mut().push_back(message);
            return;
        }
        /// Clears `running` however the delivery ends.
        struct Running<'a>(&'a Cell<bool>);
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        open.running.set(true);
        let _running = Running(&open.running);
        let mut next = Some(message);
        while let Some(message) = next {
            open.deliver.call(|d| {
                if let Ok(mut d) = d.try_borrow_mut() {
                    d(message);
                }
            });
            // Closed by what it delivered: what waits is dropped with it.
            if entry(self.id).is_none() {
                break;
            }
            next = open.queued.borrow_mut().pop_front();
        }
    }

    /// Whether it is open: its owner lives, its window too.
    #[must_use]
    pub fn is_open(self) -> bool {
        entry(self.id).is_some()
    }
}

/// A job's cancel token: set as its owner ends ([`spawn`]) or by
/// [`Task::cancel`]. A job reads it where it can stop early.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// Whether the job was cancelled: its result will be dropped.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn set(&self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Sets the token as it drops: kept by the job's answer, so a job whose
/// answer can never run (its owner ended, its window closed) is cancelled.
struct CancelOnDrop(Cancel);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.set();
    }
}

/// A job [`spawn`] started.
#[derive(Clone, Debug)]
pub struct Task {
    cancel: Cancel,
}

impl Task {
    /// Cancel it: its result is dropped, and `then` never runs.
    pub fn cancel(&self) {
        self.cancel.set();
    }

    /// Whether it was cancelled (by its owner ending too), or never started
    /// (inside a fork).
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// Work off the UI thread, owned by the current scope and cancelled with it
/// (ADR-0026, ADR-0028): `job` runs on a thread of its own, given its
/// cancel token; `then` runs on the UI thread with its result, as a
/// mailbox's handler does, unless the job was cancelled first.
///
/// Inside a fork the job never starts (a check has no effect on the
/// outside world) and the task returned is cancelled. A job that panics
/// ends its thread; its `then` never runs.
///
/// # Panics
/// In a debug build, with neither a scope running nor a window's code, as
/// [`crate::mailbox`].
#[track_caller]
pub fn spawn<T: Send + 'static>(
    job: impl FnOnce(&Cancel) -> T + Send + 'static,
    then: impl FnOnce(T) + 'static,
) -> Task {
    let cancel = Cancel::default();
    if crate::engine::forking() {
        cancel.set();
        return Task { cancel };
    }
    let mut then = Some(then);
    let ended = CancelOnDrop(cancel.clone());
    let answer = crate::mailbox::mailbox(move |value: T| {
        if ended.0.is_cancelled() {
            return;
        }
        if let Some(then) = then.take() {
            then(value);
        }
    });
    let token = cancel.clone();
    let started = std::thread::Builder::new().name("gui spawn".to_owned()).spawn(move || {
        let out = job(&token);
        if !token.is_cancelled() {
            // Closed: its owner ended meanwhile; the result is dropped.
            answer.send(out).ok();
        }
    });
    if started.is_err() {
        cancel.set();
    }
    Task { cancel }
}

#[cfg(test)]
mod tests;
