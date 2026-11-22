//! A steady update allocates nothing (ADR-0012): links are reused when a run
//! reads in the same order, and the stacks and queues keep their capacity.
//! Counted on the test's own thread (ADR-0023), so the harness's threads
//! do not add to it.

use std::cell::Cell;
use std::rc::Rc;

use gui::{Computed, Effect, Signal, batch};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

fn allocations() -> u64 {
    gui::heap::thread().map_or(0, |h| h.allocations)
}

#[test]
fn a_steady_update_allocates_nothing() {
    let a = Signal::new(0_u64);
    let b = Signal::new(0_u64);
    let sum = Computed::new(move || a.get().wrapping_add(b.get()));
    let double = Computed::new(move || sum.get().wrapping_mul(2));
    let seen = Rc::new(Cell::new(0));
    let s = Rc::clone(&seen);
    Effect::new(move || s.set(double.get().wrapping_add(a.get())));
    let fan: Vec<Effect> = (0..64)
        .map(|_| {
            let s = Rc::clone(&seen);
            Effect::new(move || s.set(s.get().max(sum.get())))
        })
        .collect();
    // Warm: every stack and queue grows to what an update needs.
    for v in 1..10 {
        a.set(v);
        batch(|| {
            a.set(v.wrapping_add(1));
            b.set(v);
        });
    }
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    let before = allocations();
    for v in 10..10_000 {
        a.set(v);
        batch(|| {
            a.set(v.wrapping_add(1));
            b.set(v);
        });
    }
    let made = allocations().saturating_sub(before);
    assert_eq!(made, 0, "{made} allocations over 19,980 steady updates");
    assert!(seen.get() > 0);
    drop(fan);
}
