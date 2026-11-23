//! The per-thread slots gui's engine state lives in.
//!
//! gui bans raw `thread_local!` (its `clippy.toml`): each piece of its
//! per-thread state is an `engine_local!`, which gives it the policy a cold
//! frame applies to it, and which stores it in one of these slots. This
//! crate holds the one `thread_local!` that does, outside gui, so the ban
//! has no exception inside it.

use std::any::Any;
use std::cell::OnceCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread::AccessError;

/// How many pieces of engine state a thread can hold.
pub const CAPACITY: usize = 256;

/// One piece of engine state, made on first use.
pub type Slot = OnceCell<Box<dyn Any>>;

thread_local! {
    static SLOTS: Box<[Slot]> = (0..CAPACITY).map(|_| OnceCell::new()).collect();
    /// How many times this thread's slots were used.
    static TOUCHES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Whether the slots used are being noted ([`watch`]).
    static WATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The slots used since [`watch`], each once.
    static WATCHED: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn touch(index: usize) {
    TOUCHES.try_with(|t| t.set(t.get().wrapping_add(1))).ok();
    if WATCHING.try_with(std::cell::Cell::get).unwrap_or(false) {
        WATCHED
            .try_with(|w| {
                let mut w = w.borrow_mut();
                if !w.contains(&index) {
                    w.push(index);
                }
            })
            .ok();
    }
}

/// How many times this thread's slots were used so far: unchanged over a
/// stretch of code, it used no engine state.
#[must_use]
pub fn touches() -> u64 {
    TOUCHES.try_with(std::cell::Cell::get).unwrap_or(0)
}

/// Note which slots this thread uses from here, until [`watched`].
pub fn watch() {
    WATCHED.try_with(|w| w.borrow_mut().clear()).ok();
    WATCHING.try_with(|w| w.set(true)).ok();
}

/// The slots used since [`watch`], each once, in the order first used;
/// the noting stops.
#[must_use]
pub fn watched() -> Vec<usize> {
    WATCHING.try_with(|w| w.set(false)).ok();
    WATCHED
        .try_with(|w| std::mem::take(&mut *w.borrow_mut()))
        .unwrap_or_default()
}

/// Run `f` on slot `index` of this thread, `None` past [`CAPACITY`].
pub fn with<R>(index: usize, f: impl FnOnce(Option<&Slot>) -> R) -> R {
    touch(index);
    SLOTS.with(|s| f(s.get(index)))
}

/// [`with`], unless the thread's slots are gone (it is ending).
pub fn try_with<R>(index: usize, f: impl FnOnce(Option<&Slot>) -> R) -> Result<R, AccessError> {
    touch(index);
    SLOTS.try_with(|s| f(s.get(index)))
}

/// A key's slot before it is given one.
pub const UNSET: usize = usize::MAX;

/// The slots given so far, the same on every thread.
static GIVEN: Mutex<usize> = Mutex::new(0);

/// The slot of the piece whose key is `key` (holding [`UNSET`] until its
/// first use): given once, under a lock, so threads racing to a key's
/// first use agree on one slot and the losers take none.
pub fn slot_of(key: &AtomicUsize) -> usize {
    let at = key.load(Ordering::Acquire);
    if at != UNSET {
        return at;
    }
    let mut given = GIVEN.lock().unwrap_or_else(PoisonError::into_inner);
    let at = key.load(Ordering::Acquire);
    if at != UNSET {
        return at;
    }
    let at = *given;
    *given = at.saturating_add(1);
    key.store(at, Ordering::Release);
    at
}

/// How many slots were given (tests).
#[must_use]
pub fn given() -> usize {
    *GIVEN.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    /// Threads racing to one key's first use take one slot between them.
    #[test]
    fn threads_racing_to_a_keys_first_use_take_one_slot() {
        const THREADS: usize = 16;
        const KEYS: usize = 200;
        let before = given();
        for _ in 0..KEYS {
            let key = Arc::new(AtomicUsize::new(UNSET));
            let start = Arc::new(Barrier::new(THREADS));
            let slots: Vec<usize> = (0..THREADS)
                .map(|_| {
                    let (key, start) = (Arc::clone(&key), Arc::clone(&start));
                    std::thread::spawn(move || {
                        start.wait();
                        slot_of(&key)
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|t| t.join().unwrap())
                .collect();
            assert!(slots.iter().all(|s| *s == slots[0]), "{slots:?}");
        }
        assert_eq!(given() - before, KEYS, "a slot a key");
    }
}
