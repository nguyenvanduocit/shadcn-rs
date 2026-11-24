//! A row measured while scrolling costs O(log n) (ADR-0014), even when it
//! moves the estimate the unmeasured rows take: frontiers 7.3 item 1 found
//! the estimate's change rebuilding every run. Counted on the test's own
//! thread (ADR-0023).

use gui::list::ExtentTree;
use gui::list::extent_tree::units;

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

fn allocations() -> u64 {
    gui::heap::thread().map_or(0, |h| h.allocations)
}

#[test]
fn a_row_measured_that_moves_the_estimate_costs_no_pass_over_the_runs() {
    // Ten thousand measured rows apart from each other: twenty thousand runs.
    let mut t = ExtentTree::new(1_000_000, units(30.0));
    for k in 0..10_000_u64 {
        t.set(k * 2, units(if k % 2 == 0 { 20.0 } else { 21.0 }));
    }
    assert!(t.runs() >= 20_000);
    let estimate = t.estimate();
    let before = allocations();
    t.set(500_001, units(400.0));
    let spent = allocations() - before;
    assert_ne!(t.estimate(), estimate, "the row moved the estimate");
    assert!(spent < 64, "{spent} allocations for one row of {} runs", t.runs());
}
