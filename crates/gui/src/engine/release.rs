//! The one release queue (ADR-0034, S5): what must happen live once a
//! check has switched back, and what live code releases while a drain runs.
//!
//! - Engine-private: no caller outside `gui` enqueues or drains.
//! - A job asked for while a check runs, or while the queue drains, is
//!   enqueued; outside both it is enqueued and the queue drained at once, so
//!   job starts are first in, first out.
//! - Each job is popped on its own and runs with the queue's borrow
//!   released, under its own catch: one that fails loses no other, and its
//!   payload becomes a string as it is caught ([`plain`], S3).
//! - One drainer at a time: a drain asked for inside a job returns at once,
//!   and the outer drain runs what the job added, in order.
//! - Its owner drains it as it ends ([`shutdown`]: the pipeline's drop),
//!   while the engine is reachable: never a thread-local destructor, whose
//!   order with the state a job needs is unspecified.

use super::EngineKey;
use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;

/// The queue and whether a drain runs. The registry's own, as `REGISTRY`
/// is: entered by no check (a rollback must not take back what it leaves to
/// do) and no Shared piece (a check touching it first would get a stand-in).
static RELEASES: EngineKey<RefCell<Releases>> = EngineKey::new(RefCell::default, None);

#[derive(Default)]
struct Releases {
    queue: VecDeque<Job>,
    draining: bool,
}

/// One job: a live obligation's transition.
type Job = Box<dyn FnOnce()>;

/// Run `f` live, after every job asked for before it: now, outside a check
/// and a drain; else once the check has switched back, or once the drain
/// reaches it. For the release of a live resource of the outside world (a
/// worker, a claim others wait on) whose live owner ends while a check runs.
///
/// # Panics
/// Outside a check and a drain, a job of the drain this starts panicked:
/// the first such panic, as a string, once the queue is empty.
pub(crate) fn when_live(f: impl FnOnce() + 'static) {
    let mut job: Option<Job> = Some(Box::new(f));
    RELEASES
        .try_with(|r| {
            if let Some(job) = job.take() {
                r.borrow_mut().queue.push_back(job);
            }
        })
        .ok();
    if let Some(job) = job {
        // The thread's engine is being torn down (a value its own state
        // held drops): no queue is left and no check runs, so the job runs
        // now, as the last live code of the thread.
        job();
        return;
    }
    if !super::is_cold() {
        raise(drain());
    }
}

/// The proof that a release is asked for by a `Drop` (review r17, decision
/// 3): only a value's drop reaches the live world from inside a check, as
/// a live value a check lets go has no other moment to end in. An explicit
/// operation inside a check (a close, a dispose, a retire) acts on the
/// check's world, never queues live work: it takes no token. Built only by
/// [`Self::in_drop`], which only the `Drop` impls of the S4 table call
/// (the obligations lint, `crates/gui/tests/obligations.rs`, checks it).
pub(crate) struct LiveDrop(());

impl LiveDrop {
    /// The token, in a `Drop` impl of the S4 table and nowhere else.
    pub(crate) const fn in_drop() -> Self {
        Self(())
    }
}

/// What a value's `Drop` releases of the engine or the outside world, by
/// its world: a check's own value is released now, with the check; a live
/// one goes through the queue ([`when_live`]), so it is released live and
/// in order, never by the check (ADR-0034, S4).
pub(crate) fn dropped(_drop: LiveDrop, made_in_fork: bool, f: impl FnOnce() + 'static) {
    if made_in_fork {
        f();
    } else {
        when_live(f);
    }
}

/// Run what is queued now (a test, or a live step that needs it done).
///
/// # Panics
/// A job panicked: the first, as a string, once the queue is empty.
#[cfg_attr(not(test), expect(dead_code, reason = "the live settle drains through `released`"))]
pub(crate) fn run_releases() {
    if !super::is_cold() {
        raise(drain());
    }
}

/// [`run_releases`], the first failure given back (the live settle raises
/// it with its cleanups'). Nothing runs while a check does.
pub(crate) fn released() -> Option<Box<dyn Any + Send>> {
    if super::is_cold() {
        return None;
    }
    drain().map(|s| -> Box<dyn Any + Send> { Box::new(s) })
}

/// End the engine's live work on this thread: every job queued runs, in
/// order, while the engine is still reachable. Its owner calls it as it ends
/// (the pipeline, so an app, a scenario, a window or an offscreen frame); a
/// job asked for after it runs at once, as ever outside a check.
///
/// # Panics
/// A job panicked (and no panic unwinds already): the first, as a string.
pub fn shutdown() {
    if super::is_cold() {
        return;
    }
    let failed = drain();
    if std::thread::panicking() {
        if let Some(s) = failed {
            eprintln!("gui: a release failed as the engine shut down: {s}");
        }
        return;
    }
    raise(failed);
}

/// How many jobs wait (tests).
#[cfg(test)]
pub(crate) fn pending() -> usize {
    RELEASES.try_with(|r| r.borrow().queue.len()).unwrap_or(0)
}

/// Drain the queue unless a drain runs: each job popped on its own and run
/// with the queue's borrow released, under its own catch. The first failure,
/// as a string.
fn drain() -> Option<String> {
    /// Ends the drain however it ends.
    struct Draining;
    impl Drop for Draining {
        fn drop(&mut self) {
            RELEASES.try_with(|r| r.borrow_mut().draining = false).ok();
        }
    }
    let first_drainer = RELEASES
        .try_with(|r| !std::mem::replace(&mut r.borrow_mut().draining, true))
        .unwrap_or(false);
    if !first_drainer {
        return None;
    }
    let _draining = Draining;
    let mut first = None;
    while let Some(job) = RELEASES.try_with(|r| r.borrow_mut().queue.pop_front()).ok().flatten() {
        if let Err(p) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)) {
            let said = plain(p);
            eprintln!("gui: a release panicked ({said}); the others still run");
            first.get_or_insert(said);
        }
    }
    first
}

/// Raise a drain's first failure; reported instead while a panic unwinds
/// (a release from a drop during unwinding), which a second would abort.
fn raise(failed: Option<String>) {
    if let Some(s) = failed {
        if std::thread::panicking() {
            eprintln!("gui: a release failed while a panic unwinds: {s}");
        } else {
            std::panic::resume_unwind(Box::new(s));
        }
    }
}

/// A caught panic payload as a string, at a check's or a release's catch
/// (ADR-0034, S3): a `String` moved, a `&'static str` copied, any other
/// payload forgotten unread (its `Drop` is user code, which runs nowhere
/// here) and a fixed diagnostic in its place.
pub(crate) fn plain(p: Box<dyn Any + Send>) -> String {
    let p = match p.downcast::<String>() {
        Ok(s) => return *s,
        Err(p) => p,
    };
    if let Some(s) = p.downcast_ref::<&'static str>() {
        return (*s).to_owned();
    }
    forget(p);
    String::from("a panic whose payload is not a string (forgotten unread: its drop is user code)")
}

/// Never drop `p`: a payload of a type whose `Drop` may run user code.
#[expect(
    clippy::mem_forget,
    reason = "a payload of an unknown type is forgotten by design (ADR-0034, S3): dropping it runs user code"
)]
fn forget(p: Box<dyn Any + Send>) {
    std::mem::forget(p);
}
