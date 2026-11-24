//! What one bound setter costs on the heap (review 12.5, level 1): its
//! definition is one allocation, the closure and the setter's write
//! together behind one pointer; each node it is mounted on adds one more,
//! the typed cell of what it last gave there, made once and kept when the
//! element is built again. Counted on the test's own thread (ADR-0023).

use gui::layout::{LayoutTree, UiState};
use gui::prelude::*;
use gui::reactive::Signal;
use gui::{Div, Element, div};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

fn allocations(f: impl FnOnce()) -> u64 {
    let ((), heap) = gui::heap::measure(f);
    // Not counted (no counting allocator): a count no assertion meets.
    heap.map_or(u64::MAX, |h| h.allocations)
}

/// A node with `n` (2 or 3) setters bound to `w`, each of other fields.
fn node(n: usize, w: Signal<f32>) -> Div {
    let d = div().width(move || w.get()).height(move || w.get() / 2.0);
    if n > 2 { d.min_width(move || w.get() / 4.0) } else { d }
}

/// Ten such nodes under a root.
fn tree(n: usize, w: Signal<f32>) -> Element {
    div().children((0..10).map(|_| node(n, w))).into_element()
}

fn lay_out(tree: &mut LayoutTree, root: &Element) {
    let mut ts = gui::text::TextSystem::new();
    tree.layout(
        root,
        400.0,
        300.0,
        &UiState::default(),
        &mut gui::Motion::default(),
        &mut ts,
    );
}

/// The definition of one bound setter is one allocation.
#[test]
fn a_bound_setter_is_one_allocation() {
    let w = Signal::new(40.0_f32);
    // Warm: whatever a first element makes once.
    drop(node(3, w));
    let two = allocations(|| drop(node(2, w)));
    let three = allocations(|| drop(node(3, w)));
    assert_eq!(three - two, 1, "the third bound setter: {two} then {three} allocations");
}

/// Mounted on a node, a bound setter adds one allocation, its cell; built
/// again with a new closure of the same setter, it adds none on the node
/// (the new definition's own is the one above).
#[test]
fn a_bound_setter_mounts_in_one_allocation_and_rebinds_in_none() {
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    let w = Signal::new(40.0_f32);
    let (r2, r3) = (tree(2, w), tree(3, w));
    // Warm: the reactive arena's slots and links, the layout's buffers,
    // made by as many trees as are measured at once, kept once they go.
    let warm: Vec<LayoutTree> = [&r2, &r3, &r2, &r3]
        .into_iter()
        .map(|r| {
            let mut t = LayoutTree::new();
            lay_out(&mut t, r);
            t
        })
        .collect();
    drop(warm);
    let mut t2 = LayoutTree::new();
    let mut t3 = LayoutTree::new();
    let mount2 = allocations(|| lay_out(&mut t2, &r2));
    let mount3 = allocations(|| lay_out(&mut t3, &r3));
    assert_eq!(
        mount3 - mount2,
        10,
        "ten nodes mount a third binding each: {mount2} then {mount3} allocations"
    );
    // Built again: new closures, the same setters. The elements are made
    // outside the count; laying them out rebinds each node's cells.
    let (again2, again3) = (tree(2, w), tree(3, w));
    let rebind2 = allocations(|| lay_out(&mut t2, &again2));
    let rebind3 = allocations(|| lay_out(&mut t3, &again3));
    assert_eq!(
        rebind3, rebind2,
        "a third binding built again costs its nodes nothing: {rebind2} then {rebind3} allocations"
    );
}
