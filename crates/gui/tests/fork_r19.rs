//! Review r19's reproducers (ADR-0035), as the reviewers wrote them.
#![cfg(debug_assertions)]
#![expect(clippy::expect_used, reason = "a failed precondition of a reproducer fails the test")]

use std::cell::RefCell;

use gui::layout::{LayoutTree, UiState};
use gui::prelude::*;

// ── Opus r19 I1 ──────────────────────────────────────────────────────────

fn lay_out(tree: &mut LayoutTree, i: gui::Interaction) {
    tree.layout(
        &gui::div().width(10.0).height(10.0).interaction(i).into_element(),
        100.,
        100.,
        &UiState::default(),
        &mut gui::Motion::default(),
        &mut gui::text::TextSystem::new(),
    );
}

fn signals() -> usize {
    gui::reactive::graph().signals
}

#[test]
fn a_live_tree_dropped_in_a_check_ends_its_flags_as_a_live_drop() {
    {
        let mut t = LayoutTree::new();
        lay_out(&mut t, gui::Interaction::new());
        drop(t);
        gui::engine::shutdown();
    }
    let before = signals();
    let mut t = LayoutTree::new();
    lay_out(&mut t, gui::Interaction::new());
    drop(t);
    gui::engine::shutdown();
    let after_live = signals();
    let before_check = signals();
    let mut t = LayoutTree::new();
    lay_out(&mut t, gui::Interaction::new());
    gui::testing::in_check(move || drop(t));
    gui::engine::shutdown();
    let after_check = signals();
    assert_eq!(
        after_check - before_check,
        after_live.saturating_sub(before),
        "a live tree dropped in a check left signals behind that a live drop ends"
    );
}

fn hovered_scenario(i: gui::Interaction) -> gui::Scenario {
    let mut s = gui::Scenario::measuring(
        move || gui::div().width(50.0).height(50.0).interaction(i).into_element(),
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("settled");
    s.hover(10.0, 10.0);
    s.settle().expect("settled");
    s
}

#[test]
fn a_live_window_dropped_in_a_check_leaves_no_hover_live() {
    let i = gui::Interaction::new();
    let s = hovered_scenario(i);
    assert!(i.hovered());
    drop(s);
    gui::engine::shutdown();
    let live_after = i.hovered();
    let j = gui::Interaction::new();
    let s = hovered_scenario(j);
    assert!(j.hovered());
    gui::testing::in_check(move || drop(s));
    gui::engine::shutdown();
    assert_eq!(
        j.hovered(),
        live_after,
        "the handle reads the window's flags after the window ended"
    );
}

// ── D3 (D008, revised): a misclassified plain `Drop` ─────────────────────

/// An app value whose plain `Drop` disposes the live root scope it holds:
/// the lint cannot see app code, so the sink refuses.
struct Misclassified(gui::Scope);

impl Drop for Misclassified {
    fn drop(&mut self) {
        self.0.dispose();
    }
}

/// Dropped inside a check, its disposal of a live root scope is refused
/// and the check faults: rolled back, the scope would come back with no
/// Rust owner left (ADR-0035, D3). The live scope and its signal stand.
#[test]
fn a_plain_drop_disposing_a_live_root_scope_in_a_check_faults() {
    let root = gui::Scope::new();
    let kept = root.run(|| gui::Signal::new(7_u8));
    let held = Misclassified(root);
    let ended = gui::testing::check(move || drop(held));
    assert!(
        matches!(&ended, Err(gui::engine::CheckReport::Faulted(why)) if why.contains("Foreign")),
        "{ended:?}"
    );
    assert!(root.is_alive() && kept.is_alive(), "the live scope stands");
    assert_eq!(kept.get_untracked(), 7);
    root.dispose();
}

// ── Codex r19 1: a forgotten guard of a logged piece ─────────────────────

gui::engine_local! {
    [Fork(log)] static ROWS: RefCell<gui::engine::ForkMap<u64, u8>> = RefCell::default();
}

/// The reproducer's guard no longer exists (`LogCell` lends through closures
/// alone: the compile-fail doctests in `engine/log_key.rs`). Its runtime
/// form: a check opened inside a live write lend is refused, its body never
/// run, and the live row reads as live code left it.
#[test]
fn a_check_inside_a_logged_lend_is_refused() {
    ROWS.with_borrow_mut(|rows| rows.replace(1, 7));
    let ended = ROWS.with_borrow_mut(|_| gui::testing::check(|| ROWS.with_borrow_mut(|rows| rows.replace(1, 9))));
    assert!(
        matches!(&ended, Err(gui::engine::CheckReport::Skipped(why)) if why.contains("InUserCode")),
        "{ended:?}"
    );
    assert_eq!(ROWS.with_borrow(|rows| rows.get(&1).copied()), Some(7));
    assert!(ROWS.with(|cell| cell.try_with_mut(|_| ()).is_some()), "live table free");
    assert!(gui::testing::check(|| 1).is_ok(), "checks open again");
}

// ── Codex r19 2: a chart snapshot lends a writable page ──────────────────

/// The write no longer compiles (`Page::data` is private: the compile-fail
/// doctest on `gui::chunk::series::Page`). A snapshot's prefix reads the
/// same after a live append, and a check's read of it leaves live `2.0`.
#[test]
fn live_chart_survives_a_check_reading_its_snapshot() {
    let owner = gui::Scope::new();
    let plot = owner.run(gui::chunk::plot::series);
    plot.push(5, 2.0);
    let sampled = plot.snapshot_untracked();
    plot.push(6, 3.0);
    assert_eq!(sampled.get(0), Some((5, 2.0)), "the prefix is as it was");
    let read = gui::testing::in_check(move || {
        let s = plot.snapshot_untracked();
        s.get(0).map(|(_, y)| y)
    });
    assert_eq!(read, Some(2.0));
    let point = plot.snapshot_untracked().get(0);
    owner.dispose();
    assert_eq!(point, Some((5, 2.0)));
}

// ── Codex r19 3: a world cell's guard spans the switch ───────────────────

/// No guard exists to carry into a check (the compile-fail doctests in
/// `engine/world_cell.rs`), nor can a check's `'static` body borrow the
/// lent queue. A check entered inside a live `with_mut` is refused, and
/// the live action list gets no `Close`.
#[test]
fn check_cannot_queue_a_live_window_close() {
    use gui::window::decoration::{Actions, WindowAction};
    let actions = Actions::default();
    let handle = std::rc::Rc::clone(&actions);
    let ended =
        actions.with_mut(|_| gui::testing::check(move || handle.with_mut(|queue| queue.push(WindowAction::Close))));
    assert!(
        matches!(&ended, Err(gui::engine::CheckReport::Skipped(why)) if why.contains("InUserCode")),
        "{ended:?}"
    );
    assert!(actions.take().is_empty(), "live queue contains a close");
}

/// A check's read lends nothing past it: the live list is writable after.
#[test]
fn check_read_leaves_the_live_action_queue_writable() {
    use gui::window::decoration::Actions;
    let actions = Actions::default();
    let handle = std::rc::Rc::clone(&actions);
    let sampled = gui::testing::in_check(move || handle.with(Vec::len));
    assert_eq!(sampled, 0);
    assert!(actions.try_with_mut(|_| ()).is_some(), "check still owns a live borrow");
}

// ── Codex r19 4: world ids per thread ────────────────────────────────────

/// Two threads' first checks have different worlds.
#[test]
fn two_threads_first_checks_differ() {
    let made = || {
        std::thread::spawn(|| gui::testing::in_check(|| gui::engine::WorldCell::new("a check's", 1u8)))
            .join()
            .expect("ran")
    };
    let (a, b) = (made(), made());
    let seen = std::thread::spawn(move || gui::testing::in_check(move || (a.is_here(), b.is_here())))
        .join()
        .expect("ran");
    assert_eq!(seen, (false, false), "no check matches another thread's");
}

#[test]
fn distinct_threads_checks_have_distinct_worlds() {
    let cell = std::thread::spawn(|| gui::testing::in_check(|| gui::engine::WorldCell::new("from A", 1u8)))
        .join()
        .unwrap();
    let claimed = std::thread::spawn(move || {
        gui::testing::in_check(move || {
            if cell.is_here() {
                cell.set(9);
            }
            cell.get()
        })
    })
    .join()
    .unwrap();
    assert_eq!(claimed, 1);
}
