//! A fork is a transaction that never commits (ADR-0028): what it makes is
//! its own, its undo restores "never made", and its verdict is read once
//! its work is done. Codex's PoCs from F1 review 9, each a regression test
//! (public API); 5 and 6 rewritten for the rule that replaced the rerun.

#![cfg(debug_assertions)]
#![expect(
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "tests: small counters, and panics are what they check"
)]

use gui::engine::{IdKind, unique_id};
use gui::testing::in_check as in_cold_frame;
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU32, Ordering};

gui::engine_local! {
    [Fresh] static PENDING: RefCell<Vec<Touch>> = RefCell::new(Vec::new());
    [Shared("immutable")] static UNIT: () = ();
    [Fork] static WRITES: Cell<u32> = Cell::new(0);
}

/// Touches a Shared piece first as the fork's drain drops it.
struct Touch;

impl gui::engine::FreshValue for Touch {}

impl Drop for Touch {
    fn drop(&mut self) {
        UNIT.with(|()| ());
    }
}

/// A result whose drop writes engine state.
struct ResultGuard;

impl Drop for ResultGuard {
    fn drop(&mut self) {
        WRITES.set(WRITES.get() + 1);
    }
}

/// Codex 1: a fork whose drain touches a Shared piece first (a destructor)
/// keeps its result, and no drop of the result happens live in its place:
/// the verdict is read once the drain is done.
#[test]
fn late_doom() {
    let kept = in_cold_frame(|| {
        PENDING.with_borrow_mut(|p| p.push(Touch));
        ResultGuard
    });
    assert_eq!(WRITES.get(), 0, "a doomed fork's result was dropped live");
    drop(kept);
}

gui::engine_local! {
    [Shared("handle")] static NODE: gui::Signal<u32> = gui::Signal::new(7);
    [Fork] static ALIAS: Cell<gui::Signal<u32>> = Cell::new(NODE.with(|s| *s));
}

/// Codex 2, now closed by ADR-0034 (S8): a Fork piece whose init reads
/// another piece (here a Shared piece's stand-in) would be the fork's
/// alone if a fork made it. Such an init panics at its first touch, in a
/// check and live, naming the piece.
#[test]
fn cached_stand_in() {
    let forked = std::panic::catch_unwind(|| in_cold_frame(|| ALIAS.get())).unwrap_err();
    let live = std::panic::catch_unwind(|| ALIAS.get()).unwrap_err();
    for p in [forked, live] {
        let said = said(&*p);
        assert!(said.contains("ALIAS read other engine state"), "{said}");
    }
}

/// Codex 3: a panic while a fork is entered leaves no fork open.
#[test]
fn failed_enter() {
    PENDING.with(|p| {
        let _b = p.borrow();
        assert!(std::panic::catch_unwind(|| in_cold_frame(|| ())).is_err());
    });
    assert!(!gui::engine::forking(), "a fork stayed open");
}

static IDS: IdKind = IdKind::new("review");

gui::engine_local! {
    [Shared("immutable id")] static INNER: u64 = unique_id(&IDS);
    [Shared("immutable ids")] static OUTER: (u64, u64) = (unique_id(&IDS), INNER.with(|id| *id));
}

/// Codex 4: nested first touches in a fork leave live code to make the
/// pieces in the order a run with no check makes them.
#[test]
fn nested_order() {
    fn order(check: bool) -> bool {
        std::thread::spawn(move || {
            if check {
                in_cold_frame(|| OUTER.with(|_| ()));
            }
            OUTER.with(|(a, b)| a < b)
        })
        .join()
        .unwrap()
    }
    assert_eq!(order(true), order(false));
}

gui::engine_local! {
    [Fork] static FAIL: Cell<bool> = Cell::new(false);
    [Shared("immutable")] static SEVEN: u32 = {
        assert!(!FAIL.get());
        7
    };
}

/// The text of a panic.
fn said(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default()
}

/// Codex 5, under the rule that replaced the rerun (F1 review 9): a Shared
/// init that reads a `[Fork]` cell would make a stand-in that differs from
/// what live code makes, so the read itself is refused, naming the piece,
/// in a fork and live.
#[test]
fn caught_stand_in_panic() {
    let forked = in_cold_frame(|| {
        FAIL.set(true);
        std::panic::catch_unwind(|| SEVEN.with(|x| *x)).expect_err("the init's read is refused")
    });
    let live = std::panic::catch_unwind(|| SEVEN.with(|x| *x)).expect_err("live too");
    for p in [forked, live] {
        let said = said(&*p);
        assert!(
            said.contains("`SEVEN`") && said.contains("state a fork changes"),
            "{said}"
        );
    }
}

/// How many times [`CHILD`]'s init ran past its read of `forking()`.
static ATTEMPTS: AtomicU32 = AtomicU32::new(0);

gui::engine_local! {
    [Shared("immutable")] static PARENT: () = CHILD.with(|()| ());
    [Shared("immutable")] static CHILD: () = {
        if !gui::engine::forking() {
            ATTEMPTS.fetch_add(1, Ordering::Relaxed);
            panic!("init failed");
        }
    };
}

/// Codex 6, under the same rule: a nested Shared init that reads whether a
/// fork runs is refused at the read, naming the piece; nothing of it runs,
/// and nothing is made at the fork's end (a stand-in lives and dies with
/// the fork), so no init runs twice.
#[test]
fn nested_failure() {
    let p = std::panic::catch_unwind(|| in_cold_frame(|| PARENT.with(|()| ()))).expect_err("refused");
    let said = said(&*p);
    assert!(said.contains("`CHILD`") && said.contains("forking()"), "{said}");
    assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 0);
}

/// Codex's PoCs from F1 review 10: a value's life in a fork has one owner.
mod codex_r10 {
    use super::said;
    use gui::testing::in_check as in_cold_frame;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    gui::engine_local! {
        [Fork] static SOURCE: Cell<u32> = Cell::new(0);
        [Fork] static COPY: Cell<u32> = {
            let value = SOURCE.get();
            assert_ne!(value, 1, "live init failed");
            Cell::new(value)
        };
        [Shared("signal handle")] static SIGNAL: gui::Signal<u32> = gui::Signal::new(0);
        [Shared("immutable snapshot")] static VALUE: u32 = SIGNAL.with(|s| s.get());
        [Fork] static WRITES: Cell<u32> = Cell::new(0);
        [Fork] static OWNED: RefCell<Option<Rc<OnDrop>>> = RefCell::new(Some(Rc::new(OnDrop(SOURCE.get()))));
        [Fresh] static PENDING: RefCell<Vec<Explodes>> = RefCell::default();
    }

    /// Writes engine state as it drops, when made from 19.
    struct OnDrop(u32);

    impl Drop for OnDrop {
        fn drop(&mut self) {
            if self.0 == 19 {
                WRITES.set(self.0);
            }
        }
    }

    /// Panics as it drops.
    struct Explodes;

    impl gui::engine::FreshValue for Explodes {}

    impl Drop for Explodes {
        fn drop(&mut self) {
            panic!("drain failed");
        }
    }

    /// R3, now ADR-0034 (S8): a slot whose init reads engine state is never
    /// made, in a fork or live: its first touch panics, naming it, each
    /// time (no slot is left half made).
    #[test]
    fn retry_remake() {
        let forked = std::panic::catch_unwind(|| {
            in_cold_frame(|| {
                SOURCE.set(9);
                COPY.get()
            })
        });
        SOURCE.set(2);
        for p in [
            forked,
            std::panic::catch_unwind(|| COPY.get()),
            std::panic::catch_unwind(|| COPY.get()),
        ] {
            let said = said(&*p.unwrap_err());
            assert!(said.contains("COPY read other engine state"), "{said}");
        }
        assert_eq!(SOURCE.get(), 2);
    }

    /// R4: a Shared init that reads a reactive value is refused, naming the
    /// piece, in a fork and live (a fork's write to the signal would
    /// otherwise stand in the Shared value live).
    #[test]
    fn shared_signal_read() {
        let forked = std::panic::catch_unwind(|| {
            in_cold_frame(|| {
                SIGNAL.with(|s| s.set(7));
                assert_eq!(SIGNAL.with(|s| s.get()), 7);
                VALUE.with(|v| *v)
            })
        });
        let live = std::panic::catch_unwind(|| VALUE.with(|v| *v));
        for p in [forked, live] {
            let p = p.expect_err("the init's read is refused");
            let said = said(&*p);
            assert!(said.contains("`VALUE`"), "{said}");
        }
    }

    /// R1, now ADR-0034 (S8): a slot made from a fork's write would be the
    /// fork's alone; its init reads engine state, so it is refused at its
    /// first touch and nothing of it is dropped live.
    #[test]
    fn drop_side() {
        WRITES.set(0);
        let forked = std::panic::catch_unwind(|| {
            in_cold_frame(|| {
                SOURCE.set(19);
                OWNED.with(|_| ());
            })
        });
        assert!(said(&*forked.unwrap_err()).contains("OWNED read other engine state"));
        assert!(std::panic::catch_unwind(|| OWNED.with(|_| ())).is_err());
        assert_eq!(WRITES.get(), 0);
    }

    /// R2: a panic in the drain keeps no result: it is dropped in the fork,
    /// then the panic resumes.
    #[test]
    fn failed_drain_result() {
        WRITES.set(0);
        let failed = std::panic::catch_unwind(|| {
            in_cold_frame(|| {
                PENDING.with_borrow_mut(|v| v.push(Explodes));
                OnDrop(19)
            })
        });
        assert!(failed.is_err());
        assert_eq!(WRITES.get(), 0);
    }
}

/// Opus's PoC from F1 review 10 (R1): one stand-in per piece per fork,
/// its drain included.
mod opus_r10 {
    use gui::testing::in_check as in_cold_frame;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU32, Ordering};

    static INITS: AtomicU32 = AtomicU32::new(0);
    static SAME: AtomicU32 = AtomicU32::new(0);
    static DIFF: AtomicU32 = AtomicU32::new(0);

    gui::engine_local! {
        [Shared("a handle")] static NODE: gui::Signal<u32> = {
            INITS.fetch_add(1, Ordering::Relaxed);
            gui::Signal::new(7)
        };
        [Fresh] static PENDING: RefCell<Vec<Holder>> = RefCell::new(Vec::new());
    }

    /// Compares the piece as its drop sees it with the one it holds.
    struct Holder(gui::Signal<u32>);

    impl gui::engine::FreshValue for Holder {}

    impl Drop for Holder {
        fn drop(&mut self) {
            if NODE.with(|s| *s) == self.0 {
                SAME.fetch_add(1, Ordering::Relaxed);
            } else {
                DIFF.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    #[test]
    fn one_stand_in_per_fork_through_its_drain() {
        std::thread::spawn(|| {
            in_cold_frame(|| {
                let h = NODE.with(|s| *s);
                PENDING.with_borrow_mut(|p| p.push(Holder(h)));
            });
            assert_eq!(INITS.load(Ordering::Relaxed), 1);
            assert_eq!(DIFF.load(Ordering::Relaxed), 0);
        })
        .join()
        .unwrap();
    }
}

/// Codex's PoCs from F1 review 11: one teardown sequence, forks scoped by a
/// closure, and R4 where state is read.
mod codex_r11 {
    use super::said;
    use gui::testing::in_check as in_cold_frame;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    gui::engine_local! {
        [Fork] static FAIL: Cell<bool> = Cell::new(false);
        [Fresh] static VALUE: Cell<u32> = Cell::new(0);
    }

    /// T0, T1, T5: the drain runs no init and leaves the undo records be;
    /// the body's panic goes on, and the live value is put back. (Its init
    /// once read `FAIL`; an init that reads engine state is refused since
    /// ADR-0034, S8.)
    #[test]
    fn drain_error() {
        VALUE.set(42);
        let panic = std::panic::catch_unwind(|| {
            in_cold_frame(|| {
                VALUE.set(9);
                FAIL.set(true);
                panic!("body failed");
            })
        })
        .unwrap_err();
        assert_eq!(
            (said(&*panic).as_str(), VALUE.get(), FAIL.get()),
            ("body failed", 42, false)
        );
    }

    gui::engine_local! {
        [Fork] static SOURCE: Cell<u32> = Cell::new(0);
        [Fork] static COPY: Cell<u32> = Cell::new(SOURCE.get());
    }

    /// A piece made from another piece's value would be the fork's alone
    /// when a fork makes it: under ADR-0034 (S8) an init that reads engine
    /// state panics at its first touch, in a fork or live, so none is.
    #[test]
    fn borrowed_at_close() {
        SOURCE.set(1);
        let made = std::panic::catch_unwind(|| {
            in_cold_frame(|| {
                SOURCE.set(9);
                COPY.with(|c| c.get())
            })
        });
        assert!(made.is_err(), "a reading init is refused");
        assert_eq!(SOURCE.get(), 1);
        assert!(std::panic::catch_unwind(|| COPY.get()).is_err(), "live too");
    }

    /// Writes live state as it drops.
    struct Writes;

    impl Drop for Writes {
        fn drop(&mut self) {
            SOURCE.set(88);
        }
    }

    gui::engine_local! {
        [Shared("immutable marker")] static MARKER: Writes = Writes;
    }

    /// Closure scoping: no stand-in outlives its fork.
    #[test]
    fn borrowed_stand_in() {
        SOURCE.set(1);
        // The stand-in's use ends inside the fork: it drops at T3, while
        // the fork is open, so its write is the fork's.
        in_cold_frame(|| MARKER.with(|_| ()));
        assert_eq!(SOURCE.get(), 1);
    }

    /// Panics as it drops.
    struct Fails;

    impl gui::engine::FreshValue for Fails {}

    impl Drop for Fails {
        fn drop(&mut self) {
            panic!("queued drop failed");
        }
    }

    gui::engine_local! {
        [Shared("signal handle")] static S: gui::Signal<u32> = gui::Signal::new(0);
        [Fresh] static QUEUE: RefCell<Vec<Fails>> = RefCell::default();
    }

    /// Whether its drop ran (`Some`), and whether the piece it reached then
    /// is the one it was made from.
    struct ResultGuard(gui::Signal<u32>, Rc<Cell<Option<bool>>>);

    impl Drop for ResultGuard {
        fn drop(&mut self) {
            self.1.set(Some(S.with(|s| *s == self.0)));
        }
    }

    /// T2 under ADR-0034 (S2 as revised in review r16): a disposal that
    /// faults goes on, and the result, not kept once the check faulted, is
    /// dropped in the check, before its stand-ins go: it reaches the very
    /// stand-in it was made from, never a later one nor the live piece.
    /// The check still faults. (A result not kept with no fault is dropped
    /// the same way:
    /// `engine::tests::a_fork_its_drain_dooms_drops_its_result_inside_it`.)
    #[test]
    fn result_before_stand_in() {
        let matched = Rc::new(Cell::new(None));
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let matched = Rc::clone(&matched);
            in_cold_frame(move || {
                QUEUE.with_borrow_mut(|q| q.push(Fails));
                ResultGuard(S.with(|s| *s), matched)
            })
        }));
        let said = super::said(&*failed.map(drop).expect_err("the check faulted"));
        assert!(said.contains("queued drop failed"), "{said}");
        assert_eq!(
            matched.get(),
            Some(true),
            "the result was dropped once, in the check, while its stand-in lived"
        );
    }

    gui::engine_local! {
        [Shared("signal handle")] static LIVES: gui::Signal<u32> = gui::Signal::new(0);
        [Shared("snapshot")] static ALIVE: bool = LIVES.with(|s| s.is_alive());
    }

    /// R4 at the source: a Shared init may not read a handle's liveness.
    #[test]
    fn liveness() {
        let s = LIVES.with(|s| *s);
        let result = std::panic::catch_unwind(|| {
            in_cold_frame(move || {
                s.dispose();
                ALIVE.with(|v| *v)
            })
        });
        assert!(result.is_err(), "accepted {result:?}");
    }
}

/// Opus's PoCs from F1 review 11.
mod opus_r11 {
    use gui::testing::in_check as in_cold_frame;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SAME: AtomicU32 = AtomicU32::new(0);
    static DIFF: AtomicU32 = AtomicU32::new(0);

    /// Panics as it drops.
    struct Explodes;

    impl gui::engine::FreshValue for Explodes {}

    impl Drop for Explodes {
        fn drop(&mut self) {
            panic!("drain failed");
        }
    }

    gui::engine_local! {
        [Shared("a handle")] static NODE: gui::Signal<u32> = gui::Signal::new(7);
        [Fresh] static PENDING: RefCell<Vec<Explodes>> = RefCell::new(Vec::new());
        [Shared("a test of a Shared init reading whether a render is cold")]
        static COLD: bool = gui::component::is_cold_render();
    }

    /// Compares the piece as its drop sees it with the one it holds.
    struct Res(gui::Signal<u32>);

    impl Drop for Res {
        fn drop(&mut self) {
            if NODE.with(|s| *s) == self.0 {
                SAME.fetch_add(1, Ordering::Relaxed);
            } else {
                DIFF.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Opus I1 (= Codex 3), under ADR-0034 (S2 as revised in review r16):
    /// after a disposal fault the result, not kept, is still dropped, once,
    /// in the check and before the stand-ins go: it reaches the stand-in
    /// it was made from, never one made after the stand-ins went, nor the
    /// live piece.
    #[test]
    fn result_not_kept_dropped_after_stand_ins() {
        std::thread::spawn(|| {
            let failed = std::panic::catch_unwind(|| {
                in_cold_frame(|| {
                    PENDING.with_borrow_mut(|p| p.push(Explodes));
                    Res(NODE.with(|s| *s))
                })
            });
            assert!(failed.is_err());
            assert_eq!(
                (SAME.load(Ordering::Relaxed), DIFF.load(Ordering::Relaxed)),
                (1, 0),
                "dropped once, against its own stand-in"
            );
        })
        .join()
        .unwrap();
    }

    /// Opus P1: a Shared init may not read whether a render is cold (it
    /// differs in a fork and live).
    #[test]
    fn a_shared_init_reading_cold_render_panics() {
        assert!(std::panic::catch_unwind(|| COLD.with(|v| *v)).is_err());
    }
}

/// Codex's PoCs from F1 review 12: entering and leaving a fork run no user
/// code (E1, E2), and R4 has one gate (E3).
mod codex_r12 {
    use gui::testing::in_check as in_cold_frame;
    use std::cell::{Cell, RefCell};
    use std::panic::catch_unwind;

    gui::engine_local! {
        [Fresh] static VALUE: Cell<u32> = Cell::new(0);
    }

    /// E1: a fork opened inside a use of a piece is refused before any
    /// change, naming the piece; nothing changed, and the next fork opens.
    #[test]
    fn outer_borrow() {
        VALUE.set(42);
        let refused = catch_unwind(|| VALUE.with(|_| in_cold_frame(|| VALUE.set(9)))).unwrap_err();
        assert!(super::said(&*refused).contains("`VALUE`"), "{}", super::said(&*refused));
        assert_eq!(VALUE.get(), 42);
        in_cold_frame(|| VALUE.set(9));
        assert_eq!(VALUE.get(), 42);
    }

    gui::engine_local! {
        [Shared("marker")] static STAND: Stand = Stand;
        [Fresh] static Q: RefCell<Vec<Bomb>> = RefCell::new(Vec::new());
        [Fork] static WRITES: Cell<u32> = Cell::new(0);
    }

    struct Stand;
    struct Bomb;
    struct ResultGuard;

    impl gui::engine::FreshValue for Bomb {}

    impl Drop for Stand {
        fn drop(&mut self) {
            Q.with_borrow_mut(|q| q.push(Bomb));
        }
    }

    impl Drop for Bomb {
        fn drop(&mut self) {
            panic!("late drain");
        }
    }

    impl Drop for ResultGuard {
        fn drop(&mut self) {
            WRITES.set(88);
        }
    }

    /// E2: a panic in what the last stand-in's drop queued drops the
    /// result inside the fork.
    #[test]
    fn late_failure() {
        WRITES.set(1);
        assert!(
            catch_unwind(|| in_cold_frame(|| {
                STAND.with(|_| ());
                ResultGuard
            }))
            .is_err()
        );
        assert_eq!(WRITES.get(), 1);
    }

    gui::engine_local! {
        [Fork] static COUNTER: Cell<u32> = Cell::new(0);
        [Fresh] static FIRST: Cell<()> = {
            if gui::engine::forking() {
                COUNTER.set(COUNTER.get() + 1);
            }
            Cell::new(())
        };
    }

    /// E1, now ADR-0034 S8: an init that reads engine state (here, to
    /// write it) panics at its first touch, so none runs in an entry.
    #[test]
    fn early_init() {
        assert!(catch_unwind(|| FIRST.get()).is_err());
        COUNTER.set(1);
        in_cold_frame(|| ());
        assert_eq!(COUNTER.get(), 1);
    }

    // `map_keys` (a key whose drop runs user code) cannot be written any
    // more: a `ForkMap`'s key is a sealed, `Copy` `ForkKey` (ADR-0034, S7).

    gui::engine_local! {
        [Shared("immutable count")] static COUNT: usize = gui::reactive::graph().signals;
    }

    /// E3: an aggregate of the runtime is a read a Shared init may not make.
    #[test]
    fn aggregate_read() {
        let before = gui::reactive::graph().signals;
        let cold = catch_unwind(|| {
            in_cold_frame(|| {
                let _signal = gui::Signal::new(7);
                COUNT.with(|n| *n)
            })
        });
        assert!(cold.is_err(), "fork={cold:?}");
        assert_eq!(before, gui::reactive::graph().signals);
    }
}

/// Opus's PoC from F1 review 12 (Codex 5, broader): every aggregate of the
/// runtime is behind the one R4 gate.
mod opus_r12 {
    use gui::testing::in_check as in_cold_frame;
    use std::panic::catch_unwind;

    gui::engine_local! {
        [Shared("a test of a Shared init reading the graph")] static SIGNALS: usize = gui::reactive::graph().signals;
        [Shared("a test of a Shared init reading the cleanups")] static CLEANUPS: usize = gui::reactive::cleanups();
        [Shared("a test of a Shared init reading the bytes held")] static BYTES: u64 = gui::reactive::held_bytes();
    }

    #[test]
    fn aggregates_read_by_a_shared_init_in_a_fork_panic() {
        let signals = catch_unwind(|| in_cold_frame(|| SIGNALS.with(|n| *n)));
        let cleanups = catch_unwind(|| in_cold_frame(|| CLEANUPS.with(|n| *n)));
        let bytes = catch_unwind(|| in_cold_frame(|| BYTES.with(|n| *n)));
        assert!(
            signals.is_err() && cleanups.is_err() && bytes.is_err(),
            "{signals:?} {cleanups:?} {bytes:?}"
        );
    }
}

/// Codex's PoCs from F1 review 13: after the drain loop no user-defined
/// value is dropped, hashed, compared or cloned, and nothing that can panic
/// runs while the fork holds its result; the R4 exemption is unforgeable.
mod codex_r13 {
    use gui::testing::in_check as in_cold_frame;
    use std::cell::{Cell, RefCell};
    use std::panic::{catch_unwind, panic_any};

    gui::engine_local! {
        [Fork] static LIVE: Cell<u32> = Cell::new(1);
        [Fresh] static Q: RefCell<Vec<Bomb>> = RefCell::default();
    }

    // `hash_during_restore` (a key whose `Hash` runs user code) cannot be
    // written any more: a `ForkMap`'s key is a sealed `ForkKey` (ADR-0034,
    // S7), closed by the compile-fail example on `ForkKey`.

    struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            LIVE.set(88);
        }
    }

    struct Bomb;

    impl gui::engine::FreshValue for Bomb {}

    impl Drop for Bomb {
        fn drop(&mut self) {
            panic_any(Guard);
        }
    }

    /// Under ADR-0034 (S2, S3): the body panics, then a disposal panics
    /// with a payload that has a `Drop`. The check faults, naming the body's
    /// panic too; the payload is forgotten unread, its drop never run, and
    /// the live state is whole.
    #[test]
    fn suppressed_payload() {
        LIVE.set(1);
        let error = catch_unwind(|| {
            in_cold_frame(|| {
                Q.with_borrow_mut(|q| q.push(Bomb));
                panic!("body");
            })
        })
        .unwrap_err();
        let said = error.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(
            said.contains("Disposal of Q") && said.contains("its body had panicked first: body"),
            "{said}"
        );
        assert_eq!(LIVE.get(), 1);
    }

    /// A winning panic whose payload has a `Drop` leaves the fork as a
    /// string; the payload is forgotten unread (S3), its drop never run.
    #[test]
    fn non_string_winner() {
        LIVE.set(1);
        let error = catch_unwind(|| in_cold_frame(|| panic_any(Guard))).unwrap_err();
        assert!(error.is::<String>(), "a plain payload");
        drop(error);
        assert_eq!(LIVE.get(), 1);
    }

    // `failed_release` moved into gui (`engine::r14`): the release queue is
    // engine-private (ADR-0034, S5).

    gui::engine_local! {
        [Shared("cached value")] static CACHED: u32 = gui::engine::shared_init("", || LIVE.get());
    }

    /// No name passed to `shared_init` turns its read check off.
    #[test]
    fn empty_name() {
        LIVE.set(1);
        let cold = catch_unwind(|| {
            in_cold_frame(|| {
                LIVE.set(9);
                CACHED.with(|v| *v)
            })
        });
        assert!(cold.is_err(), "Shared init read {cold:?}");
    }
}

/// Opus's nit from F1 review 13: a Shared init that makes an effect is
/// refused alike live and in a fork (an effect's run is code that reads
/// state, which a Shared init may not run).
mod opus_r13 {
    use gui::testing::in_check as in_cold_frame;
    use std::panic::catch_unwind;

    gui::engine_local! {
        [Shared("a test of a Shared init making an effect, live")]
        static LIVE_EFFECT: gui::Effect = gui::Effect::new(|| ());
        [Shared("a test of a Shared init making an effect, in a fork")]
        static FORK_EFFECT: gui::Effect = gui::Effect::new(|| ());
    }

    #[test]
    fn a_shared_init_making_an_effect_is_refused_live_and_in_a_fork() {
        let live = catch_unwind(|| LIVE_EFFECT.with(|_| ()));
        let fork = catch_unwind(|| in_cold_frame(|| FORK_EFFECT.with(|_| ())));
        assert_eq!((live.is_err(), fork.is_err()), (true, true));
    }
}
