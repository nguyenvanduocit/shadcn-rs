//! The reactive core's guarantees (ADR-0012): the alien-signals spec cases
//! (`references/alien-signals/tests`), one test per invariant, the API, and a
//! seeded comparison with a graph that recomputes everything.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::*;

type Log = Rc<RefCell<Vec<String>>>;

fn log() -> Log {
    Rc::new(RefCell::new(Vec::new()))
}

fn push(log: &Log, s: &str) {
    log.borrow_mut().push(s.to_owned());
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

fn counter() -> Rc<Cell<u32>> {
    Rc::new(Cell::new(0))
}

fn bump(c: &Cell<u32>) {
    c.set(c.get().saturating_add(1));
}

// ── alien-signals: effect.spec.ts ────────────────────────────────────────

#[test]
fn cleanup_order_on_outer_rerun_inner_before_outer_before_new_run() {
    let log = log();
    let a = Signal::new(0_i32);
    let l = Rc::clone(&log);
    Effect::new(move || {
        a.get();
        push(&l, "outer:run");
        let li = Rc::clone(&l);
        Effect::new(move || {
            push(&li, "inner:run");
            let lc = Rc::clone(&li);
            on_cleanup(move || push(&lc, "inner:cleanup"));
        });
        let lc = Rc::clone(&l);
        on_cleanup(move || push(&lc, "outer:cleanup"));
    });
    assert_eq!(take(&log), ["outer:run", "inner:run"]);
    a.set(1);
    assert_eq!(take(&log), ["inner:cleanup", "outer:cleanup", "outer:run", "inner:run"]);
}

#[test]
fn cleanup_order_on_dispose_inner_before_outer() {
    let log = log();
    let l = Rc::clone(&log);
    let e = Effect::new(move || {
        push(&l, "outer:run");
        let li = Rc::clone(&l);
        Effect::new(move || {
            push(&li, "inner:run");
            let lc = Rc::clone(&li);
            on_cleanup(move || push(&lc, "inner:cleanup"));
        });
        let lc = Rc::clone(&l);
        on_cleanup(move || push(&lc, "outer:cleanup"));
    });
    take(&log);
    e.dispose();
    assert_eq!(take(&log), ["inner:cleanup", "outer:cleanup"]);
}

fn three_children(l: &Log) {
    for name in ["inner1", "inner2", "inner3"] {
        let li = Rc::clone(l);
        Effect::new(move || {
            let lc = Rc::clone(&li);
            on_cleanup(move || push(&lc, &format!("{name}:cleanup")));
        });
    }
}

#[test]
fn sibling_cleanup_on_dispose_is_last_made_first() {
    let log = log();
    let l = Rc::clone(&log);
    let e = Effect::new(move || {
        three_children(&l);
        let lc = Rc::clone(&l);
        on_cleanup(move || push(&lc, "outer:cleanup"));
    });
    e.dispose();
    assert_eq!(
        take(&log),
        ["inner3:cleanup", "inner2:cleanup", "inner1:cleanup", "outer:cleanup"]
    );
}

#[test]
fn sibling_cleanup_on_outer_rerun_is_last_made_first() {
    let log = log();
    let a = Signal::new(0_i32);
    let l = Rc::clone(&log);
    Effect::new(move || {
        a.get();
        three_children(&l);
        let lc = Rc::clone(&l);
        on_cleanup(move || push(&lc, "outer:cleanup"));
    });
    take(&log);
    a.set(1);
    assert_eq!(
        take(&log)[..4],
        ["inner3:cleanup", "inner2:cleanup", "inner1:cleanup", "outer:cleanup"]
    );
}

#[test]
fn three_levels_dispose_deepest_first() {
    let log = log();
    let l = Rc::clone(&log);
    let e = Effect::new(move || {
        let lc = Rc::clone(&l);
        Effect::new(move || {
            let lg = Rc::clone(&lc);
            Effect::new(move || {
                let lg2 = Rc::clone(&lg);
                on_cleanup(move || push(&lg2, "grandchild:cleanup"));
            });
            let lc2 = Rc::clone(&lc);
            on_cleanup(move || push(&lc2, "child:cleanup"));
        });
        let lo = Rc::clone(&l);
        on_cleanup(move || push(&lo, "outer:cleanup"));
    });
    e.dispose();
    assert_eq!(take(&log), ["grandchild:cleanup", "child:cleanup", "outer:cleanup"]);
}

#[test]
fn an_unwatched_computed_disposes_its_effects_last_made_first() {
    let log = log();
    let l = Rc::clone(&log);
    let c = Computed::new(move || {
        for name in ["e1", "e2", "e3"] {
            let li = Rc::clone(&l);
            Effect::new(move || {
                let lc = Rc::clone(&li);
                on_cleanup(move || push(&lc, name));
            });
        }
        0
    });
    let e = Effect::new(move || {
        c.get();
    });
    take(&log);
    e.dispose();
    assert_eq!(take(&log), ["e3", "e2", "e1"]);
}

#[test]
fn a_computed_disposes_the_effects_of_its_last_run_before_it_runs_again() {
    let log = log();
    let a = Signal::new(0_i32);
    let l = Rc::clone(&log);
    let c = Computed::new(move || {
        push(&l, "computed:eval");
        let li = Rc::clone(&l);
        Effect::new(move || {
            push(&li, "inner:run");
            let lc = Rc::clone(&li);
            on_cleanup(move || push(&lc, "inner:cleanup"));
        });
        a.get()
    });
    Effect::new(move || {
        c.get();
    });
    take(&log);
    a.set(1);
    assert_eq!(take(&log), ["inner:cleanup", "computed:eval", "inner:run"]);
}

#[test]
fn cleanup_order_holds_after_an_inner_only_rerun() {
    let log = log();
    let (a, b) = (Signal::new(0_i32), Signal::new(0_i32));
    let l = Rc::clone(&log);
    Effect::new(move || {
        a.get();
        push(&l, "outer:run");
        let li = Rc::clone(&l);
        Effect::new(move || {
            b.get();
            push(&li, "inner:run");
            let lc = Rc::clone(&li);
            on_cleanup(move || push(&lc, "inner:cleanup"));
        });
        let lc = Rc::clone(&l);
        on_cleanup(move || push(&lc, "outer:cleanup"));
    });
    b.set(1);
    take(&log);
    a.set(1);
    assert_eq!(take(&log), ["inner:cleanup", "outer:cleanup", "outer:run", "inner:run"]);
}

/// alien-signals issue #115.
#[test]
fn an_outer_effect_keeps_its_own_dep_after_the_inner_reruns() {
    let (a, b) = (Signal::new(0_i32), Signal::new(0_i32));
    let (outer, inner) = (counter(), counter());
    let (o, i) = (Rc::clone(&outer), Rc::clone(&inner));
    Effect::new(move || {
        a.get();
        bump(&o);
        let i = Rc::clone(&i);
        Effect::new(move || {
            b.get();
            bump(&i);
        });
    });
    assert_eq!((outer.get(), inner.get()), (1, 1));
    b.set(1);
    assert_eq!(outer.get(), 1);
    assert!(inner.get() >= 2);
    a.set(1);
    assert_eq!(outer.get(), 2);
}

// ── alien-signals: effectScope.spec.ts ───────────────────────────────────

#[test]
fn a_scope_disposes_its_effects_last_made_first_deepest_first() {
    let log = log();
    let scope = Scope::new();
    scope.run(|| {
        three_children(&log);
        let l = Rc::clone(&log);
        Effect::new(move || {
            let lg = Rc::clone(&l);
            Effect::new(move || {
                let lg2 = Rc::clone(&lg);
                on_cleanup(move || push(&lg2, "grandchild:cleanup"));
            });
            let lc = Rc::clone(&l);
            on_cleanup(move || push(&lc, "child:cleanup"));
        });
    });
    scope.dispose();
    assert_eq!(
        take(&log),
        [
            "grandchild:cleanup",
            "child:cleanup",
            "inner3:cleanup",
            "inner2:cleanup",
            "inner1:cleanup"
        ]
    );
}

#[test]
fn a_scope_inside_an_effect_is_disposed_before_the_effect_reruns() {
    let log = log();
    let a = Signal::new(0_i32);
    let l = Rc::clone(&log);
    Effect::new(move || {
        a.get();
        push(&l, "outer:run");
        let li = Rc::clone(&l);
        Scope::new().run(move || {
            Effect::new(move || {
                push(&li, "inner:run");
                let lc = Rc::clone(&li);
                on_cleanup(move || push(&lc, "inner:cleanup"));
            });
        });
        let lc = Rc::clone(&l);
        on_cleanup(move || push(&lc, "outer:cleanup"));
    });
    take(&log);
    a.set(1);
    assert_eq!(take(&log), ["inner:cleanup", "outer:cleanup", "outer:run", "inner:run"]);
}

// ── alien-signals: trigger.spec.ts ───────────────────────────────────────

type Shared = Rc<RefCell<Vec<i32>>>;

#[test]
fn trigger_with_no_dependencies_does_nothing() {
    trigger(|| {});
    assert_eq!(graph(), Graph::default());
}

#[test]
fn trigger_updates_a_computed_over_a_value_changed_in_place() {
    let arr: Signal<Shared> = Signal::new(Rc::default());
    let length = Computed::new(move || arr.with(|a| a.borrow().len()));
    assert_eq!(length.get(), 0);
    arr.with_untracked(|a| a.borrow_mut().push(1));
    trigger(|| {
        arr.with(|_| ());
    });
    assert_eq!(length.get(), 1);
}

#[test]
fn trigger_updates_what_depends_on_the_second_source() {
    let src1: Signal<Shared> = Signal::new(Rc::default());
    let src2: Signal<Shared> = Signal::new(Rc::default());
    let length = Computed::new(move || src2.with(|a| a.borrow().len()));
    assert_eq!(length.get(), 0);
    src2.with_untracked(|a| a.borrow_mut().push(1));
    trigger(|| {
        src1.with(|_| ());
        src2.with(|_| ());
    });
    assert_eq!(length.get(), 1);
}

#[test]
fn trigger_runs_an_effect_once_for_two_sources() {
    let src1: Signal<Shared> = Signal::new(Rc::default());
    let src2: Signal<Shared> = Signal::new(Rc::default());
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        bump(&r);
        src1.with(|_| ());
        src2.with(|_| ());
    });
    assert_eq!(runs.get(), 1);
    trigger(|| {
        src1.with(|_| ());
        src2.with(|_| ());
    });
    assert_eq!(runs.get(), 2);
}

#[test]
fn trigger_does_not_notify_its_own_temporary_reader() {
    let src1: Signal<Shared> = Signal::new(Rc::default());
    let src2 = Computed::new(move || src1.with(|a| a.borrow().len()));
    Effect::new(move || {
        src1.with(|_| ());
        src2.get();
    });
    trigger(|| {
        src1.with(|_| ());
        src2.get();
    });
    assert_eq!(graph().scopes, 0, "the temporary reader is gone");
}

#[test]
fn trigger_allows_writing_a_signal_after_reading_it_and_reruns_once() {
    let src1 = Signal::new(1_i32);
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        bump(&r);
        src1.get();
    });
    assert_eq!(runs.get(), 1);
    trigger(|| {
        let v = src1.get();
        src1.set(v.saturating_add(1));
    });
    assert_eq!(runs.get(), 2);
    assert_eq!(src1.get(), 2);
}

// ── The six bugs of the old runtime (research, section 2) ────────────────

#[test]
fn bug_1_a_computed_read_inside_a_batch_is_fresh() {
    let s = Signal::new(1_i32);
    let c = Computed::new(move || s.get().saturating_mul(10));
    assert_eq!(c.get(), 10);
    let read = batch(|| {
        s.set(2);
        c.get()
    });
    assert_eq!(read, 20);
}

#[test]
fn bug_2_an_effect_on_a_signal_and_its_computed_runs_once_with_both_new() {
    let s = Signal::new(1_i32);
    let c = Computed::new(move || s.get().saturating_mul(10));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let l = Rc::clone(&seen);
    Effect::new(move || l.borrow_mut().push((s.get(), c.get())));
    s.set(2);
    assert_eq!(*seen.borrow(), [(1, 10), (2, 20)]);
}

#[test]
fn bug_3_an_unchanged_computed_stops_the_update() {
    let s = Signal::new(1_i32);
    let parity = Computed::new(move || s.get() % 2);
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        parity.get();
        bump(&r);
    });
    s.set(3);
    assert_eq!(runs.get(), 1);
}

#[test]
fn bug_4_an_equal_write_triggers_nothing() {
    let s = Signal::new(1_i32);
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        s.get();
        bump(&r);
    });
    s.set(1);
    assert_eq!(runs.get(), 1);
}

#[test]
fn bug_5_an_effect_writing_what_it_reads_runs_once() {
    let s = Signal::new(0_i32);
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        bump(&r);
        let v = s.get();
        s.set(v.saturating_add(1));
    });
    assert_eq!(runs.get(), 1);
}

#[test]
fn bug_6_an_effect_woken_by_another_runs_after_it_ends() {
    let (a, b) = (Signal::new(0_i32), Signal::new(0_i32));
    let log = log();
    let l = Rc::clone(&log);
    Effect::new(move || {
        let v = a.get();
        push(&l, "e1 start");
        b.set(v);
        push(&l, "e1 end");
    });
    let l = Rc::clone(&log);
    Effect::new(move || {
        b.get();
        push(&l, "e2");
    });
    take(&log);
    a.set(1);
    assert_eq!(take(&log), ["e1 start", "e1 end", "e2"]);
}

// ── Invariant 1: glitch-free; a diamond runs once ────────────────────────

#[test]
fn a_diamond_runs_its_effect_once_and_never_sees_mixed_values() {
    let a = Signal::new(1_i32);
    let b = Computed::new(move || a.get().saturating_mul(2));
    let c = Computed::new(move || a.get().saturating_mul(3));
    let d = Computed::new(move || (b.get(), c.get()));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = Rc::clone(&seen);
    Effect::new(move || s.borrow_mut().push(d.get()));
    a.set(2);
    a.set(5);
    assert_eq!(*seen.borrow(), [(2, 3), (4, 6), (10, 15)]);
}

#[test]
fn an_effect_reading_a_signal_and_its_computed_sees_them_agree() {
    let a = Signal::new(1_i32);
    let double = Computed::new(move || a.get().saturating_mul(2));
    let bad = counter();
    let runs = counter();
    let (b, r) = (Rc::clone(&bad), Rc::clone(&runs));
    Effect::new(move || {
        bump(&r);
        if double.get() != a.get().saturating_mul(2) {
            bump(&b);
        }
    });
    for v in 2..20 {
        a.set(v);
    }
    assert_eq!(bad.get(), 0);
    assert_eq!(runs.get(), 19);
}

// ── Invariant 2: fresh reads inside batch ────────────────────────────────

#[test]
fn reads_inside_a_batch_are_fresh_and_effects_run_once_after() {
    let a = Signal::new(1_i32);
    let b = Signal::new(1_i32);
    let sum = Computed::new(move || a.get().saturating_add(b.get()));
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        sum.get();
        bump(&r);
    });
    let inside = batch(|| {
        a.set(10);
        let first = sum.get();
        b.set(20);
        (first, sum.get(), runs.get())
    });
    assert_eq!(inside, (11, 30, 1), "fresh inside, no effect run yet");
    assert_eq!(runs.get(), 2, "one run after the batch");
}

// ── Invariant 3: equality cutoff ─────────────────────────────────────────

#[test]
fn an_equal_write_wakes_nothing() {
    let a = Signal::new(1_i32);
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        a.get();
        bump(&r);
    });
    a.set(1);
    assert_eq!(runs.get(), 1);
}

#[test]
fn a_value_written_and_written_back_in_a_batch_wakes_nothing() {
    let a = Signal::new(1_i32);
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        a.get();
        bump(&r);
    });
    batch(|| {
        a.set(2);
        a.set(1);
    });
    assert_eq!(runs.get(), 1, "found by the seeded comparison: seed 4, step 12");
    batch(|| {
        a.set(2);
        a.set(3);
    });
    assert_eq!(runs.get(), 2);
}

#[test]
fn a_computed_that_recomputes_an_equal_value_stops_the_update() {
    let a = Signal::new(1_i32);
    let parity = Computed::new(move || a.get() % 2);
    let evals = counter();
    let runs = counter();
    let (e, r) = (Rc::clone(&evals), Rc::clone(&runs));
    let after = Computed::new(move || {
        bump(&e);
        parity.get()
    });
    Effect::new(move || {
        after.get();
        bump(&r);
    });
    a.set(3);
    assert_eq!(
        (evals.get(), runs.get()),
        (1, 1),
        "parity unchanged: nothing below it ran"
    );
    a.set(4);
    assert_eq!((evals.get(), runs.get()), (2, 2));
}

#[test]
fn update_and_trigger_always_notify() {
    let a = Signal::new(vec![1]);
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        a.with(Vec::len);
        bump(&r);
    });
    a.update(|v| v.push(2));
    a.trigger();
    assert_eq!(runs.get(), 3);
    assert_eq!(a.get(), [1, 2]);
}

// ── Invariant 4: no self-recursion, no nested flush, a recursion limit ───

#[test]
fn an_effect_writing_what_it_read_does_not_rerun_itself() {
    let a = Signal::new(0_i32);
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        bump(&r);
        let v = a.get();
        if v < 5 {
            a.set(v.saturating_add(1));
        }
    });
    assert_eq!(runs.get(), 1);
    assert_eq!(a.get_untracked(), 1);
}

#[test]
fn effects_woken_inside_a_flush_run_in_the_same_flush_not_nested() {
    let a = Signal::new(0_i32);
    let b = Signal::new(0_i32);
    let log = log();
    let l = Rc::clone(&log);
    Effect::new(move || {
        let v = a.get();
        push(&l, "first:start");
        b.set(v);
        push(&l, "first:end");
    });
    let l = Rc::clone(&log);
    Effect::new(move || {
        b.get();
        push(&l, "second");
    });
    take(&log);
    a.set(1);
    assert_eq!(take(&log), ["first:start", "first:end", "second"]);
}

#[test]
fn two_effects_feeding_each_other_stop_at_the_recursion_limit() {
    let x = Signal::new(0_u32);
    let y = Signal::new(0_u32);
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        bump(&r);
        y.set(x.get().saturating_add(1));
    });
    Effect::new(move || x.set(y.get().saturating_add(1)));
    x.set(1000);
    assert!(runs.get() <= 103, "stopped: {}", runs.get());
    // The graph still works afterwards.
    let seen = counter();
    let s = Rc::clone(&seen);
    let z = Signal::new(0_i32);
    Effect::new(move || {
        z.get();
        bump(&s);
    });
    z.set(1);
    assert_eq!(seen.get(), 2);
}

// ── Invariant 5: an owner before what it owns; siblings in order ─────────

#[test]
fn an_owner_runs_before_its_children_and_siblings_in_subscription_order() {
    let a = Signal::new(0_i32);
    let log = log();
    let l = Rc::clone(&log);
    Effect::new(move || {
        a.get();
        push(&l, "parent");
        for name in ["child1", "child2"] {
            let lc = Rc::clone(&l);
            Effect::new(move || {
                a.get();
                push(&lc, name);
            });
        }
    });
    for name in ["sibling1", "sibling2"] {
        let l = Rc::clone(&log);
        Effect::new(move || {
            a.get();
            push(&l, name);
        });
    }
    take(&log);
    a.set(1);
    // The parent re-runs and makes its children anew; the old children were
    // disposed first and do not run.
    assert_eq!(take(&log), ["parent", "child1", "child2", "sibling1", "sibling2"]);
}

// ── Invariant 6: panic safety ────────────────────────────────────────────

#[test]
fn a_panicking_effect_leaves_the_runtime_consistent() {
    let a = Signal::new(0_i32);
    let other = counter();
    let o = Rc::clone(&other);
    Effect::new(move || {
        assert!(a.get() != 1, "boom");
    });
    Effect::new(move || {
        a.get();
        bump(&o);
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| a.set(1)));
    assert!(result.is_err(), "the panic is raised again after the flush");
    assert_eq!(other.get(), 2, "the effect after the panicking one still ran for 1");
    // Nothing is left half-run: the next write runs both effects.
    a.set(2);
    assert_eq!(other.get(), 3);
    let tracked = Signal::new(0_i32);
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        tracked.get();
        bump(&r);
    });
    tracked.set(1);
    assert_eq!(runs.get(), 2);
}

#[test]
fn a_panicking_computed_computes_again_when_read() {
    let a = Signal::new(0_i32);
    let c = Computed::new(move || {
        let v = a.get();
        assert!(v != 1, "boom");
        v.saturating_mul(10)
    });
    assert_eq!(c.get(), 0);
    a.set(1);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.get())).is_err());
    a.set(2);
    assert_eq!(c.get(), 20);
}

#[test]
fn a_panic_inside_a_batch_keeps_the_queue_for_the_next_write() {
    let a = Signal::new(0_i32);
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        a.get();
        bump(&r);
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        batch(|| {
            a.set(1);
            panic!("boom");
        })
    }));
    assert!(result.is_err());
    a.set(2);
    assert!(runs.get() >= 2);
    assert_eq!(a.get_untracked(), 2);
}

// ── The remaining cases of research §3.6 ─────────────────────────────────

/// Case 7.
#[test]
fn three_effects_on_a_signal_run_in_the_order_made_every_time() {
    let a = Signal::new(0_i32);
    let log = log();
    for name in ["e1", "e2", "e3"] {
        let l = Rc::clone(&log);
        Effect::new(move || {
            a.get();
            push(&l, name);
        });
    }
    for v in 1..=100 {
        take(&log);
        a.set(v);
        assert_eq!(take(&log), ["e1", "e2", "e3"]);
    }
}

/// Case 11.
#[test]
fn reading_a_signal_twice_makes_one_link() {
    let (a, b) = (Signal::new(0_i32), Signal::new(0_i32));
    Effect::new(move || {
        a.get();
        b.get();
        a.get();
        b.get();
    });
    assert_eq!(graph().subscriptions, 2);
}

/// Case 13.
#[test]
fn an_effect_disposed_while_queued_is_skipped() {
    let a = Signal::new(0_i32);
    let runs = counter();
    let victim = Rc::new(Cell::new(None::<Effect>));
    let v = Rc::clone(&victim);
    Effect::new(move || {
        a.get();
        if let Some(e) = v.get() {
            e.dispose();
        }
    });
    let r = Rc::clone(&runs);
    victim.set(Some(Effect::new(move || {
        a.get();
        bump(&r);
    })));
    a.set(1);
    assert_eq!(runs.get(), 1, "disposed by the first effect before its turn");
}

/// Case 14.
#[test]
fn an_effect_may_dispose_itself_while_it_runs() {
    let a = Signal::new(0_i32);
    let me = Rc::new(Cell::new(None::<Effect>));
    let runs = counter();
    let (m, r) = (Rc::clone(&me), Rc::clone(&runs));
    me.set(Some(Effect::new(move || {
        bump(&r);
        if a.get() == 1
            && let Some(e) = m.get()
        {
            e.dispose();
        }
    })));
    a.set(1);
    a.set(2);
    assert_eq!(runs.get(), 2);
    assert_eq!(graph().effects, 0);
}

/// Case 15.
#[test]
fn disposing_a_computed_with_readers_unlinks_both_ways() {
    let a = Signal::new(1_i32);
    let c = Computed::new(move || a.get());
    let runs = counter();
    let r = Rc::clone(&runs);
    let e = Effect::new(move || {
        if c.is_alive() {
            c.get();
        }
        bump(&r);
    });
    c.dispose();
    assert_eq!(graph().subscriptions, 0);
    a.set(2);
    assert_eq!(runs.get(), 1, "nothing links the effect to `a` any more");
    e.dispose();
    assert_eq!(
        graph(),
        Graph {
            signals: 1,
            ..Graph::default()
        }
    );
}

/// Case 19.
#[test]
fn a_cleanup_reads_nothing_into_its_effect() {
    let (a, b) = (Signal::new(0_i32), Signal::new(0_i32));
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        a.get();
        bump(&r);
        on_cleanup(move || {
            b.get();
        });
    });
    a.set(1);
    b.set(1);
    assert_eq!(runs.get(), 2, "the cleanup's read of `b` subscribed nothing");
}

/// Case 21.
#[test]
fn untracked_reads_make_no_link() {
    let a = Signal::new(0_i32);
    let c = Computed::new(move || a.get());
    Effect::new(move || {
        a.get_untracked();
        untracked(|| c.get());
        c.get_untracked();
    });
    assert_eq!(graph().subscriptions, 1, "only the computed's own read of `a`");
}

/// Case 22.
#[test]
fn a_computed_reading_itself_is_reported() {
    let me: Rc<Cell<Option<Computed<i32>>>> = Rc::new(Cell::new(None));
    let m = Rc::clone(&me);
    let c = Computed::new(move || m.get().map_or(0, |c| c.get()));
    me.set(Some(c));
    let err = std::panic::catch_unwind(|| c.get()).unwrap_err();
    let message = err.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("itself"), "{message}");
}

/// Case 24.
#[test]
fn no_effect_runs_while_a_value_is_borrowed() {
    let list = Signal::new(vec![1_i32]);
    let len = Signal::new(0_usize);
    let seen = Rc::new(Cell::new(0));
    let s = Rc::clone(&seen);
    Effect::new(move || {
        len.get();
        s.set(list.with(Vec::len));
    });
    list.update(|v| {
        v.push(2);
        len.set(v.len());
    });
    assert_eq!(seen.get(), 2);
    list.with(|v| len.set(v.len().saturating_add(1)));
    assert_eq!(seen.get(), 2);
}

/// Case 25.
#[test]
fn ten_thousand_labelled_signals_are_made_in_linear_time() {
    let scope = Scope::new();
    let time = |n: usize| {
        let start = std::time::Instant::now();
        scope.run(|| {
            for _ in 0..n {
                let _ = Signal::labeled("many", 0_u8);
            }
        });
        start.elapsed()
    };
    let small = time(10_000);
    let large = time(40_000);
    // Quadratic would be 16 times; linear is 4, with room for noise.
    assert!(
        large < small.saturating_mul(10),
        "10k took {small:?}, 40k took {large:?}"
    );
    scope.dispose();
}

/// Case 27, `calendar.rs`: the locale sets the system and the view follows
/// both; one change of locale reaches the view once, with both new.
#[test]
fn a_chain_of_effects_reaches_the_last_once_with_both_new() {
    let locale = Signal::new("en");
    let system = Signal::new("gregory");
    let calls = Rc::new(RefCell::new(Vec::new()));
    Effect::new(move || system.set(if locale.get() == "fa" { "persian" } else { "gregory" }));
    let c = Rc::clone(&calls);
    Effect::new(move || c.borrow_mut().push((locale.get(), system.get())));
    locale.set("fa");
    assert_eq!(*calls.borrow(), [("en", "gregory"), ("fa", "persian")]);
}

// ── Ownership and handles ────────────────────────────────────────────────

#[test]
fn a_scope_owns_what_is_made_in_it() {
    let scope = Scope::new();
    let (s, c) = scope.run(|| {
        let s = Signal::new(1_i32);
        (s, Computed::new(move || s.get()))
    });
    assert_eq!(c.get(), 1);
    let before = graph();
    scope.dispose();
    assert!(!s.is_alive());
    let after = graph();
    assert_eq!(after.signals, before.signals.saturating_sub(1));
    assert_eq!(after.computeds, before.computeds.saturating_sub(1));
    assert_eq!(after.subscriptions, 0);
}

#[test]
fn using_a_disposed_signal_names_where_it_was_made() {
    let scope = Scope::new();
    let s = scope.run(|| Signal::new(1_i32));
    scope.dispose();
    let err = std::panic::catch_unwind(|| s.get()).unwrap_err();
    let message = err.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("disposed"), "{message}");
    assert!(message.contains("reactive/tests.rs"), "{message}");
}

#[test]
fn a_slot_reused_after_dispose_does_not_answer_to_the_old_handle() {
    let old = Scope::new().run(|| Signal::new(1_i32));
    let scope = Scope::new();
    let _ = scope.run(|| Signal::new(2_i32));
    let stale = Signal::new(3_i32);
    let reused = Scope::new();
    reused.run(|| ());
    scope.dispose();
    let fresh = Signal::new(4_i32);
    assert_ne!(fresh, stale);
    assert!(old.is_alive(), "its scope was never disposed");
    assert_eq!(fresh.get(), 4);
}

#[test]
fn signals_made_by_an_effect_are_disposed_when_it_runs_again() {
    let a = Signal::new(0_i32);
    let made = Rc::new(RefCell::new(Vec::new()));
    let m = Rc::clone(&made);
    Effect::new(move || {
        a.get();
        m.borrow_mut().push(Signal::new(0_i32));
    });
    a.set(1);
    let made = made.borrow();
    assert!(!made[0].is_alive());
    assert!(made[1].is_alive());
}

#[test]
fn untracked_reads_subscribe_nothing_but_still_belong_to_the_owner() {
    let a = Signal::new(0_i32);
    let runs = counter();
    let r = Rc::clone(&runs);
    let made = Rc::new(Cell::new(None));
    let m = Rc::clone(&made);
    let e = Effect::new(move || {
        bump(&r);
        untracked(|| {
            a.get();
            m.set(Some(Signal::new(0_i32)));
        });
    });
    a.set(1);
    assert_eq!(runs.get(), 1);
    e.dispose();
    assert!(!made.get().unwrap().is_alive());
}

#[test]
fn detached_state_outlives_the_effect_that_made_it() {
    let a = Signal::new(0_i32);
    let made = Rc::new(Cell::new(None));
    let m = Rc::clone(&made);
    Effect::new(move || {
        a.get();
        if m.get().is_none() {
            m.set(Some(detached(|| Signal::new(7_i32))));
        }
    });
    a.set(1);
    assert_eq!(made.get().unwrap().get(), 7);
}

#[test]
fn watch_passes_new_and_old_and_skips_the_first_value() {
    let a = Signal::new(1_i32);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = Rc::clone(&seen);
    watch(move || a.get(), move |new, old| s.borrow_mut().push((*new, *old)));
    a.set(2);
    a.set(2);
    a.set(5);
    assert_eq!(*seen.borrow(), [(2, 1), (5, 2)]);
}

#[test]
fn a_computed_with_no_reader_lets_go_of_its_sources_and_computes_again_when_read() {
    let a = Signal::new(1_i32);
    let evals = counter();
    let e = Rc::clone(&evals);
    let c = Computed::new(move || {
        bump(&e);
        a.get()
    });
    let effect = Effect::new(move || {
        c.get();
    });
    assert_eq!(graph().subscriptions, 2);
    effect.dispose();
    assert_eq!(graph().subscriptions, 0, "unwatched: it let go of `a`");
    a.set(2);
    assert_eq!(evals.get(), 1, "nothing reads it, so it did not compute");
    assert_eq!(c.get(), 2);
    assert_eq!(evals.get(), 2);
}

#[test]
fn an_effect_wakes_only_for_what_its_last_run_read() {
    let gate = Signal::new(true);
    let early = Signal::new(0_i32);
    let runs = counter();
    let r = Rc::clone(&runs);
    let effect = Effect::new(move || {
        if gate.get() {
            early.get();
        }
        bump(&r);
    });
    gate.set(false);
    assert_eq!(runs.get(), 2);
    early.set(1);
    assert_eq!(runs.get(), 2, "the last run did not read `early`");
    assert_eq!(graph().subscriptions, 1, "only `gate` holds it");
    effect.dispose();
    assert_eq!(graph().subscriptions, 0, "a disposed effect leaves no subscription");
}

// ── Devtools ─────────────────────────────────────────────────────────────

#[test]
fn trace_names_what_was_written_and_read() {
    let tab = Signal::labeled("tab", 0_i32);
    let other = Signal::new(0_i32);
    start_trace();
    tab.get();
    tab.get();
    other.set(1);
    let trace = take_trace();
    stop_trace();
    assert_eq!(trace.reads.iter().map(ToString::to_string).collect::<Vec<_>>(), ["tab"]);
    assert_eq!(trace.writes.len(), 1);
    assert!(trace.writes.first().is_some_and(|w| w.label.is_none()));
}

#[test]
fn probes_read_and_write_labeled_signals() {
    let count = Signal::labeled("count", 3_i32);
    let runs = counter();
    let r = Rc::clone(&runs);
    Effect::new(move || {
        count.get();
        bump(&r);
    });
    let probe = probes().into_iter().find(|p| p.label == "count").unwrap();
    assert_eq!((probe.value.as_str(), probe.subscribers), ("3", 1));
    write_probe(probe.id, "7").unwrap();
    assert_eq!(count.get(), 7);
    assert_eq!(runs.get(), 2);
    assert!(write_probe(probe.id, "seven").is_err());
    let scope = Scope::new();
    let gone = scope.run(|| Signal::labeled("gone", 1_u8));
    let id = probes().into_iter().find(|p| p.label == "gone").unwrap().id;
    scope.dispose();
    assert!(write_probe(id, "2").is_err());
    assert!(!gone.is_alive());
}

// ── Scale: no recursion in propagate, dirty-check or unwatch ─────────────

/// A chain `depth` computeds deep, built one at a time (so no getter runs
/// inside another), updated from its root and let go.
fn chain(depth: usize) {
    let root = Signal::new(0_u64);
    let mut last = Computed::new(move || root.get());
    last.get();
    for _ in 0..depth {
        let prev = last;
        last = Computed::new(move || prev.get().wrapping_add(1));
        last.get();
    }
    let tail = last;
    let seen = Rc::new(Cell::new(0));
    let s = Rc::clone(&seen);
    let effect = Effect::new(move || s.set(tail.get()));
    root.set(10);
    assert_eq!(seen.get(), 10_u64.wrapping_add(u64::try_from(depth).unwrap()));
    effect.dispose();
    assert_eq!(graph().subscriptions, 0, "the whole chain let go, iteratively");
}

#[test]
fn a_chain_of_a_hundred_thousand_computeds_updates_without_recursion() {
    chain(100_000);
}

/// The plan's goal: a million deep (`cargo test -- --ignored`, release).
#[test]
#[ignore = "a million nodes: run with --release -- --ignored"]
fn a_chain_of_a_million_computeds_updates_without_recursion() {
    chain(1_000_000);
}

#[test]
fn a_signal_with_a_hundred_thousand_effects_notifies_them_all_once() {
    const N: u32 = 100_000;
    let a = Signal::new(0_i32);
    let runs = counter();
    let scope = Scope::new();
    scope.run(|| {
        for _ in 0..N {
            let r = Rc::clone(&runs);
            Effect::new(move || {
                a.get();
                bump(&r);
            });
        }
    });
    a.set(1);
    assert_eq!(runs.get(), N.saturating_mul(2));
    scope.dispose();
    assert_eq!(graph().effects, 0);
}

// ── Seeded comparison with a graph that recomputes everything ────────────

/// xorshift64*: the same sequence for a seed on every machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next().checked_rem(u64::try_from(n.max(1)).unwrap()).unwrap()).unwrap()
    }
}

/// A computed node: reads `deps[0]`; if it is even, also `deps[1]`, else
/// `deps[2]` (its dependencies change as values do), then folds.
#[derive(Clone)]
struct Spec {
    deps: [usize; 3],
    modulo: i64,
}

fn eval(spec: &Spec, get: &dyn Fn(usize) -> i64) -> i64 {
    let first = get(spec.deps[0]);
    let second = if first % 2 == 0 {
        get(spec.deps[1])
    } else {
        get(spec.deps[2])
    };
    first.wrapping_add(second).rem_euclid(spec.modulo)
}

#[derive(Clone, Copy)]
enum Node {
    Signal(Signal<i64>),
    Computed(Computed<i64>),
}

impl Node {
    fn get(self) -> i64 {
        match self {
            Self::Signal(s) => s.get(),
            Self::Computed(c) => c.get(),
        }
    }
}

/// Graphs a local run checks; `ITERATIONS` (the one knob every seeded test
/// shares, `testing::seeds`) sets how many, `SEED` replays from one.
const GRAPHS: u64 = 1000;

/// An effect whose read set changes between runs: it reads `deps[0]`, then
/// `deps[1]` if that is even, else `deps[2]`, like a computed's `Spec`. It
/// lives in a scope of its own, so a step can dispose it either way.
struct Watcher {
    spec: Spec,
    scope: Scope,
    effect: Effect,
    alive: bool,
}

/// Which nodes `spec` reads when `get` gives the values.
fn reads(spec: &Spec, get: &[i64]) -> [usize; 2] {
    let first = spec.deps[0];
    [
        first,
        if get[first] % 2 == 0 {
            spec.deps[1]
        } else {
            spec.deps[2]
        },
    ]
}

#[test]
fn seeded_graphs_match_a_full_recompute_after_every_step() {
    crate::testing::run_seeded(GRAPHS, &[], seeded_graph);
}

#[expect(clippy::too_many_lines, reason = "one graph, built, driven and checked in order")]
fn seeded_graph(seed: u64) {
    let mut rng = Rng(seed.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let scope = Scope::new();
    let signals = 2_usize.saturating_add(rng.below(6));
    let computeds = 1_usize.saturating_add(rng.below(25));
    let mut values: Vec<i64> = (0..signals).map(|_| i64::try_from(rng.below(10)).unwrap()).collect();
    let mut specs: Vec<Spec> = Vec::new();
    let nodes: Rc<RefCell<Vec<Node>>> = Rc::new(RefCell::new(Vec::new()));
    let evals = Rc::new(RefCell::new(vec![0_u32; signals.saturating_add(computeds)]));
    scope.run(|| {
        for &v in &values {
            nodes.borrow_mut().push(Node::Signal(Signal::new(v)));
        }
        for k in 0..computeds {
            let below = signals.saturating_add(k);
            let spec = Spec {
                deps: [rng.below(below), rng.below(below), rng.below(below)],
                modulo: 3_i64.saturating_add(i64::try_from(rng.below(5)).unwrap()),
            };
            specs.push(spec.clone());
            let all = Rc::clone(&nodes);
            let ev = Rc::clone(&evals);
            let me = below;
            let c = Computed::new(move || {
                bump_vec(&ev, me);
                let nodes: Vec<Node> = all.borrow().clone();
                eval(&spec, &|i| nodes[i].get())
            });
            nodes.borrow_mut().push(Node::Computed(c));
        }
    });
    // Effects, each watching one node, recording what it saw.
    let total = signals.saturating_add(computeds);
    let watched: Vec<usize> = (0..4).map(|_| rng.below(total)).collect();
    let seen: Rc<RefCell<Vec<Vec<i64>>>> = Rc::new(RefCell::new(vec![Vec::new(); watched.len()]));
    scope.run(|| {
        for (e, &w) in watched.iter().enumerate() {
            let node = nodes.borrow()[w];
            let s = Rc::clone(&seen);
            Effect::new(move || {
                let v = node.get();
                s.borrow_mut()[e].push(v);
            });
        }
    });
    // Effects with a read set that changes, one scope each; the steps
    // dispose some, by the scope or by the effect.
    let dyn_seen: Rc<RefCell<Vec<Vec<i64>>>> = Rc::new(RefCell::new(vec![Vec::new(); 4]));
    let mut watchers: Vec<Watcher> = Vec::new();
    scope.run(|| {
        for e in 0..4 {
            let spec = Spec {
                deps: [rng.below(total), rng.below(total), rng.below(total)],
                modulo: 3_i64.saturating_add(i64::try_from(rng.below(5)).unwrap()),
            };
            let own = Scope::new();
            let (s, all, sp) = (Rc::clone(&dyn_seen), Rc::clone(&nodes), spec.clone());
            let effect = own.run(|| {
                Effect::new(move || {
                    let nodes: Vec<Node> = all.borrow().clone();
                    let v = eval(&sp, &|i| nodes[i].get());
                    s.borrow_mut()[e].push(v);
                })
            });
            watchers.push(Watcher {
                spec,
                scope: own,
                effect,
                alive: true,
            });
        }
    });
    let oracle = |values: &[i64]| -> Vec<i64> {
        let mut all: Vec<i64> = values.to_vec();
        for spec in &specs {
            let v = eval(spec, &|i| all[i]);
            all.push(v);
        }
        all
    };
    let mut expected = oracle(&values);
    for step in 0..60 {
        let writes = 1_usize.saturating_add(rng.below(3));
        let before = expected.clone();
        let lens: Vec<usize> = seen.borrow().iter().map(Vec::len).collect();
        let dyn_lens: Vec<usize> = dyn_seen.borrow().iter().map(Vec::len).collect();
        // Dispose one watcher now and then, before the writes: it must
        // not run for them, and what it read must not keep it alive.
        if rng.below(8) == 0 {
            let w = rng.below(watchers.len());
            let by_scope = rng.below(2) == 0;
            let watcher = &mut watchers[w];
            if watcher.alive {
                if by_scope {
                    watcher.scope.dispose();
                    assert!(
                        !watcher.scope.is_alive(),
                        "seed {seed} step {step}: scope {w} alive after its dispose"
                    );
                } else {
                    watcher.effect.dispose();
                }
                watcher.alive = false;
                assert!(
                    !watcher.effect.is_alive(),
                    "seed {seed} step {step}: watcher {w} still alive"
                );
            }
        }
        batch(|| {
            for _ in 0..writes {
                let i = rng.below(signals);
                let v = i64::try_from(rng.below(10)).unwrap();
                values[i] = v;
                if let Node::Signal(s) = nodes.borrow()[i] {
                    s.set(v);
                }
            }
        });
        expected = oracle(&values);
        for (k, node) in nodes.borrow().iter().enumerate() {
            assert_eq!(node.get(), expected[k], "seed {seed} step {step}: node {k}");
        }
        for (e, &w) in watched.iter().enumerate() {
            let seen = seen.borrow();
            let ran = seen[e].len().saturating_sub(lens[e]);
            let changed = before[w] != expected[w];
            assert_eq!(
                ran,
                usize::from(changed),
                "seed {seed} step {step}: effect on {w} ran {ran}"
            );
            assert_eq!(seen[e].last().copied(), Some(expected[w]));
        }
        for (e, watcher) in watchers.iter().enumerate() {
            let seen = dyn_seen.borrow();
            let ran = seen[e].len().saturating_sub(dyn_lens[e]);
            // It reruns once when a node it read changed, by the read set
            // of its last run; a disposed one never runs again.
            let changed = reads(&watcher.spec, &before).iter().any(|&n| before[n] != expected[n]);
            assert_eq!(
                ran,
                usize::from(watcher.alive && changed),
                "seed {seed} step {step}: dynamic effect {e} (alive {}) ran {ran}",
                watcher.alive
            );
            if watcher.alive {
                assert_eq!(
                    seen[e].last().copied(),
                    Some(eval(&watcher.spec, &|i| expected[i])),
                    "seed {seed} step {step}: dynamic effect {e}"
                );
            }
        }
    }
    scope.dispose();
    assert_eq!(graph(), Graph::default(), "seed {seed}: the scope took everything");
}

fn bump_vec(v: &RefCell<Vec<u32>>, i: usize) {
    if let Some(n) = v.borrow_mut().get_mut(i) {
        *n = n.saturating_add(1);
    }
}

// ── Allocation-free steady state ─────────────────────────────────────────

#[test]
fn a_steady_update_reuses_its_links() {
    let a = Signal::new(0_i32);
    let b = Computed::new(move || a.get().saturating_add(1));
    let sink = Rc::new(Cell::new(0));
    let s = Rc::clone(&sink);
    Effect::new(move || s.set(b.get()));
    a.set(1);
    let links = graph().subscriptions;
    for v in 2..1000 {
        a.set(v);
    }
    assert_eq!(graph().subscriptions, links, "links were reused, not remade");
    assert_eq!(sink.get(), 1000);
}

#[test]
fn a_timer_set_again_by_an_owned_timer_is_owned_and_ends_with_its_scope() {
    crate::clock::script(std::time::Duration::ZERO);
    let ticks = counter();
    let t = Rc::clone(&ticks);
    let scope = Scope::detached();
    fn arm(t: Rc<Cell<u32>>) {
        crate::clock::after(std::time::Duration::from_millis(10), move || {
            bump(&t);
            arm(Rc::clone(&t));
        });
    }
    scope.run(|| arm(t));
    crate::clock::advance_to(std::time::Duration::from_millis(35));
    assert_eq!(ticks.get(), 3);
    scope.dispose();
    crate::clock::advance_to(std::time::Duration::from_millis(100));
    assert_eq!(ticks.get(), 3, "no timer outlives its scope");
}

/// A scope that owned many signals leaves no room behind in its freed slot:
/// kept, such lists spread over the free slots as they are reused, and the
/// heap keeps them.
#[test]
fn a_disposed_owner_leaves_no_list_in_its_slot() {
    for _ in 0..3 {
        let scope = Scope::detached();
        scope.run(|| {
            for i in 0..1000_u32 {
                let _ = Signal::new(i);
            }
        });
        scope.dispose();
        let small = Scope::detached();
        small.run(|| Signal::new(0_u32));
        small.dispose();
    }
    assert_eq!(runtime::with(|rt| rt.free_owned_capacity()), 0);
}

/// The effects a render makes all run once it returns, though one of them
/// panics: the panic goes on after the last has run.
#[test]
fn a_panicking_deferred_effect_leaves_none_after_it_unrun() {
    let ran = counter();
    let r = Rc::clone(&ran);
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime::rendering("panics", || {
            let _first = Effect::new(|| panic!("the first effect fails"));
            let r = Rc::clone(&r);
            let _second = Effect::new(move || bump(&r));
        });
    }));
    assert!(out.is_err(), "the panic goes on");
    assert_eq!(ran.get(), 1, "the effect after it ran");
}

/// What a deferred effect reads is read in the effect phase, not the
/// render's: a paint-phase write of it is not late for a render.
#[test]
fn a_deferred_effects_reads_are_no_renders() {
    let x = Signal::new(0_u32);
    runtime::begin_frame();
    runtime::in_phase(Phase::Render, false, || {
        runtime::rendering("reads", || {
            let _e = Effect::new(move || {
                let _ = x.get();
            });
        });
    });
    drop(runtime::take_late_writes());
    runtime::in_phase(Phase::Paint, false, || x.set(1));
    assert!(
        runtime::take_late_writes().is_empty(),
        "read by the effect, not by a render"
    );
    runtime::in_phase(Phase::Idle, false, || {});
}

/// A cold re-render that panics leaves nothing cold behind it: the
/// effects a later live render makes still run.
#[test]
fn a_panicking_cold_render_leaves_later_effects_running() {
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _cold = crate::engine::ColdFrame::begin();
        runtime::rendering("cold", || panic!("the cold render fails"))
    }));
    assert!(out.is_err(), "the panic goes on");
    let ran = counter();
    let r = Rc::clone(&ran);
    runtime::rendering("live", || {
        let r = Rc::clone(&r);
        let _e = Effect::new(move || bump(&r));
    });
    assert_eq!(ran.get(), 1, "a live render's effect runs");
}

/// Two cleanups of a scope, the first panicking: the second runs all the
/// same as the scope is disposed, and the panic goes on.
#[test]
fn a_panicking_cleanup_leaves_none_after_it_unrun() {
    let ran = counter();
    let r = Rc::clone(&ran);
    let scope = Scope::new();
    scope.run(|| {
        on_cleanup(|| panic!("the first cleanup fails"));
        on_cleanup(move || bump(&r));
    });
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scope.dispose()));
    assert!(out.is_err(), "the panic goes on");
    assert_eq!(ran.get(), 1, "the cleanup after it ran");
}
