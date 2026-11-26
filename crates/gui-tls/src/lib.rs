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
    /// What uses of the slots are noted: [`WATCHING`], [`NOTING`] and
    /// [`NOTED`].
    static FLAGS: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    /// The slots used since [`watch`], each once.
    static WATCHED: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The slots whose pieces' inits run now ([`initing`]).
    static INITING: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Run `init`, the init of slot `index`'s piece, unless that init already
/// runs on this thread: `None` then (an init that reaches its own piece,
/// which would recurse until the stack overflows: `OnceCell` tells
/// reentrance only once the init returns).
pub fn initing<R>(index: usize, init: impl FnOnce() -> R) -> Option<R> {
    /// The slot's init no longer runs, however it ends.
    struct Done(usize);
    impl Drop for Done {
        fn drop(&mut self) {
            INITING
                .try_with(|i| {
                    let mut i = i.borrow_mut();
                    if let Some(at) = i.iter().rposition(|s| *s == self.0) {
                        i.swap_remove(at);
                    }
                })
                .ok();
        }
    }
    let running = INITING
        .try_with(|i| {
            let mut i = i.borrow_mut();
            let running = i.contains(&index);
            if !running {
                i.push(index);
            }
            running
        })
        .unwrap_or(false);
    if running {
        return None;
    }
    let done = Done(index);
    let out = init();
    drop(done);
    Some(out)
}

/// The slots used are listed ([`watch`]).
const WATCHING: u8 = 1;
/// Whether any slot is used is noted ([`noting`]).
const NOTING: u8 = 2;
/// A slot was used while [`NOTING`].
const NOTED: u8 = 4;

/// Note slot `index`'s use while [`watch`]ing or [`noting`]; otherwise one
/// flag read, the whole cost a use of engine state pays here.
#[inline]
fn touch(index: usize) {
    let flags = FLAGS.try_with(std::cell::Cell::get).unwrap_or(0);
    if flags == 0 {
        return;
    }
    if flags & NOTING != 0 {
        FLAGS.try_with(|f| f.set(flags | NOTED)).ok();
    }
    if flags & WATCHING != 0 {
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

/// Set or clear `bit` of the flags.
fn flag(bit: u8, on: bool) {
    FLAGS
        .try_with(|f| f.set(if on { f.get() | bit } else { f.get() & !bit }))
        .ok();
}

/// Note which slots this thread uses from here, until [`watched`] (empty:
/// the stretch of code between used no engine state).
pub fn watch() {
    WATCHED.try_with(|w| w.borrow_mut().clear()).ok();
    flag(WATCHING, true);
}

/// The slots used since [`watch`], each once, in the order first used;
/// the noting stops.
#[must_use]
pub fn watched() -> Vec<usize> {
    flag(WATCHING, false);
    WATCHED
        .try_with(|w| std::mem::take(&mut *w.borrow_mut()))
        .unwrap_or_default()
}

/// Run `f`, and say whether it used any slot. Nested, a use inside counts
/// for every `noting` around it too; on unwind the flags are put back.
pub fn noting<R>(f: impl FnOnce() -> R) -> (R, bool) {
    /// The flags outside, put back however `f` ends (a use inside kept
    /// for an outer `noting`).
    struct Outer(u8);
    impl Drop for Outer {
        fn drop(&mut self) {
            FLAGS
                .try_with(|f| {
                    let now = f.get();
                    let noted = if self.0 & NOTING == 0 {
                        0
                    } else {
                        (self.0 | now) & NOTED
                    };
                    f.set((now & WATCHING) | (self.0 & NOTING) | noted);
                })
                .ok();
        }
    }
    let outer = Outer(FLAGS.try_with(std::cell::Cell::get).unwrap_or(0));
    FLAGS.try_with(|f| f.set((f.get() | NOTING) & !NOTED)).ok();
    let out = f();
    let used = FLAGS.try_with(std::cell::Cell::get).unwrap_or(0) & NOTED != 0;
    drop(outer);
    (out, used)
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

/// [`try_with`], unnoted: the engine's own moves as a check switches back to
/// the live world, so a [`watch`] over the switch sees only what other code
/// reached (gui's ADR-0034, S1).
pub fn try_with_quiet<R>(index: usize, f: impl FnOnce(Option<&Slot>) -> R) -> Result<R, AccessError> {
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
