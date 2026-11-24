//! A row measured while scrolling costs O(log n) (ADR-0014), even when it
//! moves the estimate the unmeasured rows take: frontiers 7.3 item 1 found
//! the estimate's change rebuilding every run. A sort's heights, reordered
//! on the worker, go in place on the UI thread for nothing. Counted on the
//! test's own thread (ADR-0023).

use gui::list::extent_tree::units;
use gui::list::{ExtentTree, ListState};

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

const MILLION: u64 = 1_000_000;

/// A list whose million rows are all measured, two heights alternating (a
/// million runs).
fn measured_list() -> ListState {
    let list = ListState::new(30.0);
    let mut lent = list.lend_heights();
    let ext = lent.extents_mut();
    *ext = ExtentTree::new(MILLION, units(30.0));
    for r in 0..MILLION {
        ext.set(r, units(if r % 2 == 0 { 20.0 } else { 21.0 }));
    }
    assert!(list.reorder_with(lent, MILLION, Some).is_some());
    assert_eq!(list.runs(), 1_000_000);
    list
}

/// Codex (M15 review 1): a sort of a million measured rows, its heights
/// reordered on the worker, is put in place on the UI thread allocating
/// (almost) nothing, warm: the work is the worker's.
#[test]
fn a_sort_of_a_million_measured_rows_allocates_nothing_on_the_ui_thread() {
    let list = measured_list();
    let reverse = |p: u64| Some(MILLION - 1 - p);
    let mut spent = Vec::new();
    for _ in 0..3 {
        let mut lent = list.lend_heights();
        list.adjust_row_height(7, 1.0);
        // The worker's part.
        lent.reorder(MILLION, reverse);
        let before = allocations();
        let replaced = list.reorder_with(lent, MILLION, reverse);
        spent.push(allocations() - before);
        assert!(replaced.is_some());
        // Dropped elsewhere (the worker), as `Views` does.
        std::thread::spawn(move || drop(replaced)).join().unwrap();
    }
    eprintln!("allocations per sort put in place: {spent:?}");
    assert!(spent[2] <= 4, "allocations per sort put in place: {spent:?}");
}

/// What the worker spends reordering a million measured rows (a line for
/// the bench: run it with `--release --nocapture`).
#[test]
fn the_worker_reorders_a_million_measured_rows() {
    let list = measured_list();
    let mut lent = list.lend_heights();
    let at = std::time::Instant::now();
    lent.reorder(MILLION, |p| Some(MILLION - 1 - p));
    let took = at.elapsed();
    eprintln!("worker reorder, 1 000 000 measured rows in 1 000 000 runs: {took:?}");
    assert_eq!(lent.extents().len(), MILLION);
}
