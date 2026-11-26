//! F1b round 3: the round-2 reproducers (Codex dff0fd38), each a check
//! opened from user code the engine reached while live state was lent
//! (ADR-0035 D4). A check's body is `'static`, so the originals, which
//! carried the lent borrow into the check, no longer compile: they are the
//! `compile_fail` examples on `gui::testing::in_check` and `check`. What
//! stays reachable is a `'static` check opened at the same point; each
//! test here opens one and shows the live value whole.

#![cfg(debug_assertions)]

use gui::{Signal, engine::WorldRefCell, testing::check};
use std::{cell::Cell, fmt, rc::Rc};

/// Formatting a world cell runs no payload code: its `Debug` names it.
#[test]
fn debug_lend() {
    struct Probe(Cell<u8>, Rc<Cell<bool>>);
    impl fmt::Debug for Probe {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.1.set(true);
            write!(f, "{:?}", check(|| ()))
        }
    }
    let ran = Rc::new(Cell::new(false));
    let live = WorldRefCell::new("live", Probe(Cell::new(7), Rc::clone(&ran)));
    let said = format!("{live:?}");
    assert!(!ran.get(), "the payload's Debug ran: {said}");
    assert!(said.contains("live"), "{said}");
    assert_eq!(live.with(|p| p.0.get()), 7);
}

/// `Signal::set`'s comparison is a lend: a check it opens is refused,
/// its body never run.
#[test]
fn equality_lend() {
    #[derive(Clone)]
    struct Probe(Cell<u8>, Rc<Cell<Option<String>>>);
    impl PartialEq for Probe {
        fn eq(&self, _: &Self) -> bool {
            let said = match check(|| ()) {
                Ok(()) => String::from("admitted"),
                Err(why) => format!("{why:?}"),
            };
            self.1.set(Some(said));
            true
        }
    }
    let said = Rc::new(Cell::new(None));
    let value = Signal::new(Probe(Cell::new(7), Rc::clone(&said)));
    value.set(Probe(Cell::new(7), Rc::clone(&said)));
    let said = said.take().expect("the comparison ran");
    assert!(said.contains("InUserCode"), "{said}");
    assert_eq!(value.with(|p| p.0.get()), 7);
}

/// A worker thread's check, opened while this thread holds a mutable lend,
/// can carry no borrow of it: it runs in the worker's own engine and
/// leaves the lent value as it was.
#[test]
fn scoped_thread_lend() {
    let live = WorldRefCell::new("counter", 7);
    let result = live.with_mut(|slot| {
        let result = std::thread::scope(|scope| scope.spawn(|| check(|| 9)).join().expect("the worker"));
        assert_eq!(*slot, 7, "{result:?}");
        result
    });
    assert_eq!(live.take(), 7, "{result:?}");
}

/// Formatting a snapshot takes no page lock: the caller's writer never
/// runs while a page is held.
#[test]
fn page_formatter_lend() {
    use fmt::Write;
    struct Writer(bool, String);
    impl Write for Writer {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            if s.contains("PageData") {
                self.0 = true;
            }
            self.1.push_str(s);
            Ok(())
        }
    }
    let owner = gui::Scope::new();
    let plot = owner.run(gui::chunk::plot::series);
    plot.push(1, 1.);
    let mut writer = Writer(false, String::new());
    write!(writer, "{:?}", plot.snapshot_untracked()).expect("formatting");
    owner.dispose();
    assert!(!writer.0, "a page's data formatted under its lock: {}", writer.1);
}

// ── F1b round 4 (Opus r3, Codex r3) ──────────────────────────────────────

/// What a refused check said, or `admitted`.
fn said<R: fmt::Debug>(result: &Result<R, gui::engine::CheckReport>) -> String {
    match result {
        Ok(out) => format!("admitted: {out:?}"),
        Err(why) => format!("{why:?}"),
    }
}

/// Opus r3 P1: a computed's getter is user code the engine calls (the
/// runtime's `update`): a check it opens is refused, as one in any run the
/// runtime switches its subscriber or owner for (ADR-0035 D4's one gate).
#[test]
fn a_check_inside_a_live_getter() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
    let scope = gui::Scope::new();
    let s = Rc::clone(&seen);
    let doubled = scope.run(|| {
        gui::reactive::Computed::new(move || {
            s.borrow_mut().push(said(&check(|| 1)));
            2_u8
        })
    });
    // Read at the top level, outside `untracked`: the getter's run is the
    // runtime's `update` alone.
    assert_eq!(doubled.get(), 2);
    let s = Rc::clone(&seen);
    gui::untracked(move || s.borrow_mut().push(said(&check(|| 2))));
    let s = Rc::clone(&seen);
    scope.run(move || s.borrow_mut().push(said(&check(|| 3))));
    let got = seen.borrow().clone();
    assert_eq!(got.len(), 3, "{got:?}");
    assert!(got.iter().all(|r| r.contains("InUserCode")), "{got:?}");
    assert!(check(|| 4).is_ok(), "checks open again outside");
    scope.dispose();
}

/// Opus r3 P2: a check opened inside a check's body is refused, its body
/// never run, rather than panicking the outer check.
#[test]
fn a_check_inside_a_check() {
    let outer = check(|| said(&check(|| 1)));
    let inner = outer.expect("the outer check stands");
    assert!(inner.contains("InCheck"), "{inner}");
    assert!(check(|| 2).is_ok(), "checks open again after");
}

/// Codex r3 `scoped_worker`, kept as D016 classes it: thread A lends its
/// payload `&mut`, a Rayon job carries it to worker B, and B's `'static`
/// check body runs the job through `yield_local`. The write is A's live
/// code writing what A's own lend handed it, on a borrow of A's world no
/// switch on B swaps; the job runs with or without the check, so live ends
/// at 9 either way. T1 holds by counterfactual; a check running foreign
/// work is its body's impurity (T3; ADR-0035 D6, "A check owns what its
/// body runs").
#[test]
fn scoped_worker() {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("a pool");
    let live = WorldRefCell::new("payload", 7);
    let checked = live.with_mut(|slot| {
        pool.scope(|s| {
            s.spawn(move |_| *slot = 9);
            check(|| rayon::yield_local().is_some())
        })
    });
    assert_eq!(checked, Ok(true), "the check ran the job");
    assert_eq!(live.take(), 9, "D016: the job's write is live code's, check or not");
}
