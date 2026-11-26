//! What ending a 10,000-node interactive tree costs (ADR-0035,
//! Verification): dropped live, and dropped inside a check (its engine
//! work done live once the check has switched back). Ignored: a
//! measurement, run with a release profile that keeps debug assertions
//! (the check is debug only):
//!
//! ```text
//! cargo test --config 'profile.measure.inherits="release"' \
//!   --config 'profile.measure.debug-assertions=true' --config 'profile.measure.debug=false' \
//!   --profile measure -p gui --test teardown_cost -- --ignored --nocapture
//! ```
#![cfg(debug_assertions)]

use std::time::{Duration, Instant};

use gui::layout::{LayoutTree, UiState};
use gui::prelude::*;

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

const NODES: usize = 10_000;
const ROUNDS: usize = 7;

/// A tree of `NODES` interactive leaves, each with a bound style, laid out.
fn laid_out() -> LayoutTree {
    let root = gui::div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(1000.0)
        .children((0..NODES).map(|_| {
            let i = gui::Interaction::new();
            gui::div().width(4.0).height(4.0).interaction(i).background(move || {
                if i.hovered() {
                    gui::Color::BLACK
                } else {
                    gui::Color::WHITE
                }
            })
        }))
        .into_element();
    let mut tree = LayoutTree::new();
    tree.layout(
        &root,
        1000.,
        1000.,
        &UiState::default(),
        &mut gui::Motion::default(),
        &mut gui::text::TextSystem::new(),
    );
    tree
}

/// Best-of-`ROUNDS` time and the allocations of `end` ending a fresh tree.
fn measure(end: impl Fn(LayoutTree)) -> (Duration, u64) {
    let mut best = Duration::MAX;
    let mut allocs = 0;
    for _ in 0..ROUNDS {
        let tree = laid_out();
        gui::engine::shutdown();
        let before = gui::heap::thread();
        let start = Instant::now();
        end(tree);
        let took = start.elapsed();
        let after = gui::heap::thread();
        if took < best {
            best = took;
            allocs = match (before, after) {
                (Some(b), Some(a)) => a.since(b).allocations,
                _ => 0,
            };
        }
    }
    (best, allocs)
}

#[test]
#[ignore = "a measurement: run with --ignored on a release profile with debug assertions"]
fn teardown_of_a_ten_thousand_node_interactive_tree() {
    let signals = gui::reactive::graph().signals;
    let (live, live_allocs) = measure(|t| {
        drop(t);
        gui::engine::shutdown();
    });
    let (check, check_allocs) = measure(|t| {
        gui::testing::in_check(move || drop(t));
        gui::engine::shutdown();
    });
    println!(
        "teardown {NODES} interactive nodes: live {live:?} ({live_allocs} allocations), in a check then live {check:?} ({check_allocs} allocations); signals before {signals} after {}",
        gui::reactive::graph().signals
    );
}
