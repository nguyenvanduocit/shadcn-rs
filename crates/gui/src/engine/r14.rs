//! Review 14's reproducers (Codex 1 to 7, Opus's runaway) and ADR-0034's
//! own checks, each closed by a decision of that record, not at its site.

use super::{ForkSlab, IdKind, IdLeases, in_cold_frame, run_releases, unique_id_in, when_live};
use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, panic_any};
use std::rc::Rc;

crate::engine_local! {
    [Fork] static CONFIG: Cell<bool> = Cell::new(true);
    [Fork(log)] static IDS: RefCell<ForkSlab<u8>> = {
        let _ = CONFIG.get();
        RefCell::default()
    };
}

/// Codex r14-1, closed by S6 and S8: a slab whose init read another piece
/// was not entered by the check, and allocated in live generation space;
/// its id then read a later live entry. An init that reads engine state is
/// now a debug panic at first touch (S8), so every piece is entered; and
/// allocation is routed by the running world, so an id of a world that
/// ended names nothing (S6: the next test).
#[test]
fn escaped_id() {
    let p = catch_unwind(|| in_cold_frame(|| IDS.with_borrow_mut(|s| s.insert(1)))).unwrap_err();
    let said = fault_of(&p);
    assert!(
        said.contains("the init of the engine state IDS read other engine state"),
        "{said}"
    );
    let p = catch_unwind(|| IDS.with(|_| ())).unwrap_err();
    assert!(fault_of(&p).contains("ADR-0034, S8"), "live too");
}

/// A fork's slab id names nothing in a later fork either, though both
/// number their own entries from the same index.
#[test]
fn a_fork_id_names_nothing_in_the_next_fork() {
    crate::engine_local! {
        [Fork(log)] static SLAB: RefCell<ForkSlab<u8>> = RefCell::default();
    }
    SLAB.with(|_| ());
    let first = in_cold_frame(|| SLAB.with_borrow_mut(|s| s.insert(1)));
    let seen = in_cold_frame(|| {
        let second = SLAB.with_borrow_mut(|s| s.insert(2));
        (second != first, SLAB.with_borrow(|s| s.read(first, |v| *v)))
    });
    assert_eq!(seen, (true, None));
}

struct Bomb;

impl Drop for Bomb {
    #[expect(clippy::panic, reason = "a payload whose drop panics")]
    fn drop(&mut self) {
        panic!("payload drop");
    }
}

/// A value whose drop says so, and panics for one key.
#[derive(Clone)]
struct Said(u8, std::sync::mpsc::Sender<u8>);

impl Drop for Said {
    #[expect(clippy::panic, reason = "a value whose drop panics")]
    fn drop(&mut self) {
        let _sent = self.1.send(self.0);
        if self.0 == 3 {
            panic!("value 3's drop");
        }
    }
}

/// The message a check that faulted panics with.
fn fault_of(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default()
}

/// Codex r14-2, closed by S9 and S2: one panicking value of an overlay
/// made its map's drop skip the other seven, silently. Each value is now
/// disposed of on its own; the first that panics is a fault, reported with
/// its piece; the rest are still dropped, each once (S2 revised in review
/// r16); the live map is whole; and no check opens on the thread again.
#[test]
fn overlay_drops() {
    crate::engine_local! {
        [Fork(log)] static MAP: RefCell<super::ForkMap<u8, Said>> = RefCell::default();
    }
    let (tx, rx) = std::sync::mpsc::channel();
    MAP.with_borrow_mut(|m| m.insert(9, Said(9, tx.clone())));
    let p = catch_unwind(|| {
        in_cold_frame(|| {
            for k in 0..8 {
                MAP.with_borrow_mut(|m| m.insert(k, Said(k, tx.clone())));
            }
        });
    })
    .unwrap_err();
    let said = fault_of(&p);
    assert!(said.contains("a check faulted (Disposal of MAP"), "{said}");
    let dropped: Vec<u8> = rx.try_iter().collect();
    let mut once = dropped.clone();
    once.sort_unstable();
    once.dedup();
    assert_eq!(once.len(), dropped.len(), "each dropped once: {dropped:?}");
    // S2 revised (review r16): the disposal goes on past the fault, so all
    // eight of the check's values are dropped.
    assert_eq!(once, (0..8).collect::<Vec<u8>>(), "every one dropped: {dropped:?}");
    assert!(!dropped.contains(&9), "the live value is not the check's to drop");
    MAP.with_borrow(|m| {
        assert_eq!((m.len(), m.get(&9).map(|s| s.0)), (1, Some(9)), "the live map is whole");
    });
    let next = catch_unwind(|| in_cold_frame(|| ())).unwrap_err();
    assert!(
        fault_of(&next).contains("an earlier one faulted"),
        "{}",
        fault_of(&next)
    );
}

/// ADR-0034, S9 for `[Fresh]` pieces: a check's `Vec` of values whose
/// drops panic was dropped whole by the drain, so the second panic ran
/// while the first unwound the `Vec`'s drop glue: an abort, which takes
/// the test process with it. The drain takes a `[Fresh]` value apart, one
/// box per element ([`super::Parts`]): the first panic is the fault, the
/// other value is still dropped under its own catch, a later fault (S2,
/// review r16), and the live value is moved back whole.
#[test]
fn a_fresh_vec_whose_two_drops_panic_faults_without_an_abort() {
    struct Panics(std::sync::mpsc::Sender<u8>, u8);
    impl super::FreshValue for Panics {}
    impl Drop for Panics {
        fn drop(&mut self) {
            let _sent = self.0.send(self.1);
            panic!("value {}'s drop", self.1);
        }
    }
    crate::engine_local! {
        [Fresh] static PENDING: RefCell<Vec<Panics>> = RefCell::new(Vec::new());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    PENDING.with_borrow_mut(|v| v.clear());
    let p = catch_unwind(|| {
        in_cold_frame(|| {
            PENDING.with_borrow_mut(|v| {
                v.push(Panics(tx.clone(), 1));
                v.push(Panics(tx.clone(), 2));
            });
        });
    })
    .unwrap_err();
    let said = fault_of(&p);
    assert!(said.contains("a check faulted (Disposal of PENDING"), "{said}");
    assert!(said.contains("then (Disposal of PENDING): value 2's drop"), "{said}");
    assert_eq!(
        rx.try_iter().collect::<Vec<_>>(),
        [1, 2],
        "both drops ran, each under its own catch"
    );
    assert_eq!(PENDING.with(|v| v.borrow().len()), 0, "the live value is back");
}

/// Made in a check, it makes another as it drops: a destructor that
/// schedules work again.
#[derive(Clone)]
struct Again;

crate::engine_local! {
    [Fork(log)] static AGAIN: RefCell<ForkSlab<Again>> = RefCell::default();
}

impl Drop for Again {
    fn drop(&mut self) {
        AGAIN.with_borrow_mut(|s| s.insert(Again));
    }
}

/// The check's result: says, as it drops, whether a check ran then.
struct Payload(std::sync::mpsc::Sender<bool>);

impl Drop for Payload {
    fn drop(&mut self) {
        let _sent = self.0.send(crate::reactive::is_cold());
    }
}

/// Opus r14, closed by S1 and S2: a disposal that ran away (what it made
/// still made more after the bound) left the check's result to be dropped
/// live, after the switch: user code in the live world. A runaway is a
/// fault: the result is quarantined with the rest, and the switch runs no
/// destructor.
#[test]
fn runaway_disposal_quarantines_the_result() {
    let (tx, rx) = std::sync::mpsc::channel();
    let p = catch_unwind(move || {
        in_cold_frame(move || {
            AGAIN.with_borrow_mut(|s| s.insert(Again));
            Payload(tx)
        })
    })
    .map(drop)
    .unwrap_err();
    let said = fault_of(&p);
    assert!(said.contains("a check faulted (Runaway of AGAIN"), "{said}");
    assert_eq!(rx.try_iter().collect::<Vec<_>>(), [], "the result was never dropped");
    assert_eq!(AGAIN.with_borrow(|s| s.len()), 0, "the live slab is whole");
}

/// Codex r14-3, closed by S3 and S5: a losing release payload's drop
/// replaced the winner and discarded the jobs after it. A release's payload
/// is a string as it is caught (another type is forgotten unread), and
/// each job runs on its own.
#[test]
fn release_failures() {
    let ran = Rc::new(Cell::new(false));
    in_cold_frame(|| {
        when_live(|| panic!("first"));
        when_live(|| panic_any(Bomb));
        let ran = Rc::clone(&ran);
        when_live(move || ran.set(true));
    });
    let p = catch_unwind(run_releases).unwrap_err();
    assert_eq!(
        (p.downcast_ref::<String>().map(String::as_str), ran.get()),
        (Some("first"), true)
    );
}

/// Codex r14-4, closed by S5 (the owner drains the queue): a release a
/// check left was dropped uncalled as the thread ended. The pipeline, the
/// live owner, drains it as it ends, while the engine is reachable.
#[test]
fn shutdown_release() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    let closed = Arc::new(AtomicBool::new(false));
    let worker = Arc::clone(&closed);
    let left = std::thread::spawn(move || {
        // A check leaves a release, and nothing else on the thread drains:
        // no pipeline, no live release, no settle. Only its owner's end does.
        in_cold_frame(|| when_live(move || worker.store(true, SeqCst)));
        let left = super::releases_pending();
        super::shutdown();
        left
    })
    .join()
    .unwrap();
    assert_eq!(left, 1, "queued, and nothing drained it before the end");
    assert!(
        closed.load(SeqCst),
        "the owner's shutdown ran it before the thread ended"
    );
}

/// S5, the pipeline as the owner: its drop shuts the engine down.
#[test]
fn shutdown_release_through_the_pipeline() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    let closed = Arc::new(AtomicBool::new(false));
    let worker = Arc::clone(&closed);
    std::thread::spawn(move || {
        use crate::element::IntoElement;
        let pipeline = crate::app::Pipeline::new(|| crate::element::div().into_element(), false);
        in_cold_frame(|| when_live(move || worker.store(true, SeqCst)));
        drop(pipeline);
    })
    .join()
    .unwrap();
    assert!(closed.load(SeqCst), "the release ran before the thread ended");
}

/// S5 and S10: a check of the pipeline that panics still releases what it
/// left, live, after the switch and before `check_cold` returns: nothing
/// waits on a later, unrelated drain point. Its own panic goes on.
#[test]
fn a_check_that_panics_releases_what_it_left_as_it_ends() {
    use crate::element::{IntoElement, div};
    let ran = Rc::new(Cell::new(false));
    let armed = Rc::new(Cell::new(false));
    let (r, a) = (Rc::clone(&ran), Rc::clone(&armed));
    let ui = move || {
        if a.get() && crate::engine::forking() {
            let r = Rc::clone(&r);
            when_live(move || r.set(true));
            panic!("the check fails");
        }
        div().into_element()
    };
    let mut s = crate::app::Scenario::measuring(ui, 100, 100, 1.0).unwrap();
    s.check_cold(false);
    s.settle().unwrap();
    armed.set(true);
    let p = catch_unwind(std::panic::AssertUnwindSafe(|| s.check_now())).unwrap_err();
    assert_eq!(fault_of(&p), "the check fails");
    assert!(ran.get(), "released as the check ended");
    assert_eq!(super::releases_pending(), 0);
}

/// A value that says each time it is cloned.
#[derive(Default)]
struct Counted(Option<std::sync::mpsc::Sender<()>>);

impl Clone for Counted {
    fn clone(&self) -> Self {
        if let Some(s) = &self.0 {
            let _sent = s.send(());
        }
        Self(self.0.clone())
    }
}

crate::engine::fork_clone_by_clone!(Counted, Said, Again);

/// Codex r14-7, closed by S7 (the overlay is looked at first): 1,000
/// writes to one key in a check cloned the live value 1,000 times; now
/// once.
#[test]
fn repeated_write() {
    crate::engine_local! {
        [Fork(log)] static MAP: RefCell<super::ForkMap<u64, Counted>> = RefCell::default();
    }
    let (tx, rx) = std::sync::mpsc::channel();
    MAP.with_borrow_mut(|m| m.insert(1, Counted(Some(tx))));
    in_cold_frame(|| {
        for _ in 0..1_000 {
            MAP.with_borrow_mut(|m| {
                let _written: &mut Counted = m.entry_mut(1);
            });
        }
    });
    assert_eq!(rx.try_iter().count(), 1);
}

/// S7: a value a check writes over in its own overlay is not dropped in
/// the writer's borrow. By `insert`, it is handed back to the caller as a
/// live map hands it back (moved, not copied); by `replace`, which hands
/// nothing back, the drain drops it with the rest of what the check made.
#[test]
fn a_value_written_over_in_a_check_drops_in_its_drain() {
    crate::engine_local! {
        [Fork(log)] static MAP: RefCell<super::ForkMap<u64, Said>> = RefCell::default();
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let was = in_cold_frame(|| {
        MAP.with_borrow_mut(|m| {
            let none = m.insert(1, Said(1, tx.clone()));
            let was = m.insert(1, Said(2, tx.clone()));
            assert!(none.is_none());
            m.replace(1, Said(4, tx.clone()));
            assert_eq!(rx.try_iter().count(), 0, "nothing dropped while the map is borrowed");
            was
        })
    });
    let mut dropped: Vec<u8> = rx.try_iter().collect();
    dropped.sort_unstable();
    assert_eq!(
        dropped,
        [2, 4],
        "what the check made and kept no hold of: dropped inside it"
    );
    assert!(MAP.with_borrow(|m| m.is_empty()));
    assert_eq!(
        was.as_ref().map(|s| s.0),
        Some(1),
        "what it wrote over, moved out to the caller, uncopied"
    );
}

/// S7: a value with no [`super::ForkClone`] goes in a map by replacement
/// only, so a check never copies it: a live value is shadowed by the
/// check's, and a tombstone hides it, with the live map untouched.
#[test]
fn a_replace_only_value_is_never_copied() {
    struct Uncopied(u8);
    crate::engine::fork_value_audited!(Uncopied);
    crate::engine::parts_whole!(Uncopied);
    crate::engine_local! {
        [Fork(log)] static MAP: RefCell<super::ForkMap<u64, Uncopied>> = RefCell::default();
    }
    MAP.with_borrow_mut(|m| {
        m.replace(1, Uncopied(1));
        m.replace(2, Uncopied(2));
    });
    let seen = in_cold_frame(|| {
        MAP.with_borrow_mut(|m| {
            m.replace(1, Uncopied(10));
            m.forget(&2);
            (m.get(&1).map(|v| v.0), m.contains_key(&2), m.len())
        })
    });
    assert_eq!(seen, (Some(10), false, 1));
    MAP.with_borrow(|m| {
        assert_eq!(
            (m.get(&1).map(|v| v.0), m.get(&2).map(|v| v.0)),
            (Some(1), Some(2)),
            "live is whole"
        );
    });
}

// Codex r14-6 (a key equal to the live one but not the same) is closed by
// S7 at compile time: a `ForkMap`'s key is a sealed `ForkKey`, which no
// type of the caller's implements (the compile-fail example on `ForkKey`).

/// Codex r14-5, closed by S5 (one drainer, FIFO): a job that opens a check
/// which leaves a job, and drains, ran that job before the older one.
#[test]
fn release_order() {
    let order = Rc::new(RefCell::new(Vec::new()));
    let record = |order: &Rc<RefCell<Vec<u8>>>, v: u8| {
        let order = Rc::clone(order);
        move || order.borrow_mut().push(v)
    };
    let (one, two, three) = (record(&order, 1), record(&order, 2), record(&order, 3));
    in_cold_frame(|| {
        when_live(move || {
            one();
            in_cold_frame(|| when_live(three));
            run_releases();
        });
        when_live(two);
    });
    run_releases();
    assert_eq!(*order.borrow(), [1, 2, 3]);
}

/// S5: outside a check, a job asked for runs at once, after what was
/// queued before it.
#[test]
fn a_live_release_runs_at_once_after_those_queued() {
    let order = Rc::new(RefCell::new(Vec::new()));
    let o = Rc::clone(&order);
    in_cold_frame(|| when_live(move || o.borrow_mut().push(1)));
    let o = Rc::clone(&order);
    when_live(move || o.borrow_mut().push(2));
    assert_eq!(*order.borrow(), [1, 2]);
    assert_eq!(super::releases_pending(), 0);
}

/// S6: live ids are the same with and without checks, in two engines run
/// from the same start (each its own thread and id allocator): unique ids,
/// slab ids and reactive node ids, a check between each live step that
/// allocates of every kind, first registers a slab, and is dropped part way.
#[test]
fn live_ids_are_the_same_with_and_without_checks() {
    fn run(check: bool, leases: &IdLeases) -> Vec<String> {
        static KIND: IdKind = IdKind::new("r14 stream");
        crate::engine_local! {
            [Fork(log)] static SLAB: RefCell<ForkSlab<u32>> = RefCell::default();
            [Fork(log)] static LATE: RefCell<ForkSlab<u32>> = RefCell::default();
        }
        let mut seen = Vec::new();
        for step in 0..12_u32 {
            if check {
                let ended = catch_unwind(|| {
                    in_cold_frame(|| {
                        for _ in 0..5 {
                            let _ = unique_id_in(&KIND, leases);
                            SLAB.with_borrow_mut(|s| s.insert(step));
                            LATE.with_borrow_mut(|s| s.insert(step));
                            let _ = crate::Signal::new(step);
                        }
                        if step % 3 == 0 {
                            panic!("a check interrupted after its writes");
                        }
                    })
                });
                assert_eq!(ended.is_err(), step % 3 == 0);
            }
            seen.push(format!("id {}", unique_id_in(&KIND, leases)));
            let id = SLAB.with_borrow_mut(|s| s.insert(step));
            seen.push(format!("slab {id:?}"));
            if step % 2 == 0 {
                SLAB.with_borrow_mut(|s| s.remove(id));
            }
            let signal = crate::Signal::new(step);
            seen.push(format!("node {:?}", signal.node_id()));
            if step % 4 == 0 {
                signal.dispose();
            }
        }
        seen
    }
    let plain = std::thread::spawn(|| run(false, &IdLeases::new(4))).join().unwrap();
    let checked = std::thread::spawn(|| run(true, &IdLeases::new(4))).join().unwrap();
    assert_eq!(checked, plain);
}

/// Drops of a [`Marker`], counted.
static MARKERS_DROPPED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Whether the writing drain hook has written (once).
static HOOK_WROTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A value a drain hook writes into another piece.
struct Marker;

impl Drop for Marker {
    fn drop(&mut self) {
        MARKERS_DROPPED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn writer_enter() -> Box<dyn std::any::Any> {
    Box::new(())
}

/// A drain hook that reaches another piece (as a destructor it ran would):
/// it writes a new handle into `COPY`, once.
fn writer_drain(_: &dyn std::any::Any) -> Vec<Box<dyn std::any::Any>> {
    if !HOOK_WROTE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        COPY.set(Some(Rc::new(Marker)));
    }
    Vec::new()
}

fn writer_exit(_: Box<dyn std::any::Any>) {}

crate::engine_local! {
    [Fork(writer_enter, writer_drain, writer_exit)] static WRITER: Cell<u8> = Cell::new(0);
    [Fork] static COPY: RefCell<Option<Rc<Marker>>> = RefCell::new(None);
}

/// Codex r15-1 (decision 6): the drain reaches a fixed point before the
/// switch. A drain hook that writes a piece the check already drained this
/// round (a destructor it ran) leaves it for the next round: the value it
/// wrote is drained and dropped inside the check, and the switch finds the
/// live value in place and moves only. Without the next round, the switch
/// would meet the new handle (quarantined: the check faults).
#[test]
fn a_piece_a_drain_hook_wrote_is_drained_again_before_the_switch() {
    WRITER.with(|_| ());
    COPY.with(|_| ());
    in_cold_frame(|| {
        // Entered in this order, drained in the other: COPY first.
        WRITER.set(1);
        COPY.with(|_| ());
    });
    assert!(HOOK_WROTE.load(std::sync::atomic::Ordering::SeqCst), "the hook wrote");
    assert_eq!(
        MARKERS_DROPPED.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "dropped in the check, once"
    );
    assert!(COPY.with(|c| c.borrow().is_none()), "live is as it was");
}

/// Opus r15 P1: a cancelled check unwinds from a cancellation point, and a
/// value dropped on the way reaches another (a render a guard's drop
/// runs). Unwinding again from inside an unwind aborts the process, so a
/// point does nothing while the thread panics. Run in a process of its
/// own: without the guard the abort takes the process down, which the
/// parent sees as a failed exit.
#[test]
fn a_point_reached_while_a_cancelled_check_unwinds_does_nothing() {
    const ME: &str = "engine::r14::a_point_reached_while_a_cancelled_check_unwinds_does_nothing";
    if std::env::var_os("GUI_R14_CHILD").is_some() {
        struct ReachesAPoint;
        impl Drop for ReachesAPoint {
            fn drop(&mut self) {
                super::cancel_point();
            }
        }
        let ended = catch_unwind(|| {
            in_cold_frame(|| {
                let _guard = ReachesAPoint;
                super::cancel_check();
                super::cancel_point();
            })
        });
        let said = ended
            .map_err(|p| fault_of(&p))
            .expect_err("the check does not end normally");
        assert!(said.contains("cancelled"), "{said}");
        return;
    }
    let exe = std::env::current_exe().expect("the test binary");
    let out = std::process::Command::new(exe)
        .args([ME, "--exact", "--test-threads=1", "--nocapture"])
        .env("GUI_R14_CHILD", "1")
        .output()
        .expect("the child runs");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the child aborted or failed: {:?}\n{said}",
        out.status
    );
}

/// A logged piece whose drain hook fails (a bug in gui, injected).
#[derive(Default)]
struct DrainFails;

impl super::sealed::Sealed for DrainFails {}

impl super::Logged for DrainFails {
    fn begin(&mut self) {}
    #[expect(clippy::panic, reason = "an injected drain fault")]
    fn drain(&mut self) -> Vec<Box<dyn std::any::Any>> {
        panic!("a drain hook fails");
    }
    fn rollback(&mut self) {}
}

crate::engine_local! {
    [Fork(log)] static DRAIN_A: RefCell<DrainFails> = RefCell::default();
    [Fork(log)] static DRAIN_B: RefCell<DrainFails> = RefCell::default();
}

/// S2 (review r16): every fault is reported, the first and then each
/// later one in order; two drain hooks failing in one round are two
/// faults, not one (the drain once kept only the first of a round).
#[test]
fn two_failing_drains_are_two_faults_in_one_report() {
    std::thread::spawn(|| {
        DRAIN_A.with(|_| ());
        DRAIN_B.with(|_| ());
        let p = catch_unwind(|| in_cold_frame(|| ())).unwrap_err();
        let said = fault_of(&p);
        assert!(
            said.contains("a check faulted (Drain of DRAIN_B): a drain hook fails"),
            "{said}"
        );
        assert!(said.contains("then (Drain of DRAIN_A): a drain hook fails"), "{said}");
    })
    .join()
    .expect("the thread ends");
}
