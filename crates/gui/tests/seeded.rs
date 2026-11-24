//! Seeded random element trees (ADR-0023), edited step by step: every frame
//! equals one built cold (ADR-0013), for every seed. `SEED=n ITERATIONS=1`
//! repeats one run; `ITERATIONS=200` searches wider.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a test's body, run on its own thread: a frame that fails is a failed test"
)]

use std::cell::RefCell;
use std::rc::Rc;

use gui::testing::{RandomTree, Rng, Topology, run_seeded};
use gui::{Scenario, Signal};

/// A test thread's stack is 2 MB; a deep tree (48 nested boxes) needs more
/// in a debug build, so each seed runs on a thread with a main thread's 8 MB.
const STACK: usize = 8 << 20;

/// Trees a local run checks; `ITERATIONS` (the one knob of every seeded test,
/// `testing::seeds`) sets how many, and CI sets it higher.
const SEEDS: u64 = 16;

#[test]
fn random_trees_edited_by_seed_equal_a_cold_build_every_frame() {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(edit_random_trees)
        .expect("a thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

fn edit_random_trees() {
    run_seeded(SEEDS, &[], |seed| {
        let tree = Signal::new(RandomTree::new(seed, 160, Topology::of_seed(seed)));
        let mut s = Scenario::measuring(move || tree.with(RandomTree::element), 800, 600, 1.0).expect("a scenario");
        s.check_cold(true);
        s.settle().expect("the first frame");
        let rng = Rc::new(RefCell::new(Rng::new(seed)));
        for step in 0..12 {
            let r = Rc::clone(&rng);
            let mut what = String::new();
            s.act(format!("edit {step}"), || {
                tree.update(|t| what = t.mutate(&mut r.borrow_mut()));
            });
            s.settle()
                .unwrap_or_else(|e| panic!("seed {seed} step {step} ({what}): {e}"));
        }
        let built = s.last().nodes.len();
        assert!(built > 1, "seed {seed}: the tree was laid out");
    });
}
