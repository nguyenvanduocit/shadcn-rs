//! Review r18's reproducers (ADR-0034): a live owner's drop inside a check
//! taking the explicit close's path, a tree laid out in a check publishing
//! into live members, and a check's read writing a live registry's count.
#![cfg(debug_assertions)]

use std::cell::RefCell;
use std::rc::Rc;

use gui::layout::{LayoutTree, UiState};
use gui::prelude::*;

// ── Codex r18 1: a live owner dropped in a check ─────────────────────────

/// A live pipeline a check drops closes live once the check has switched
/// back: its window's mailbox is retired, as its live close retires it.
#[test]
fn a_live_pipeline_dropped_in_a_check_retires_its_mail() {
    let mut window = gui::app::Pipeline::new(|| gui::Element::Empty, false);
    let mut sender = None;
    window.act("register", || sender = Some(gui::mailbox(|_: u8| {})));
    let sender = sender.expect("made in the window");
    assert!(sender.send(1).is_ok(), "open: the window takes mail");
    gui::testing::in_check(move || drop(window));
    gui::engine::shutdown();
    assert!(
        sender.send(42).is_err(),
        "the window closed live: its mailbox is retired"
    );
}

/// A live root a check drops ends live once the check has switched back:
/// the mailbox its tree made, owned by its scope, is retired with it.
#[test]
fn a_live_root_dropped_in_a_check_retires_its_mail() {
    let scope = gui::Scope::new();
    let made = Rc::new(RefCell::new(None));
    let m = Rc::clone(&made);
    let root = scope.run(|| {
        gui::Root::new(move || {
            if m.borrow().is_none() {
                m.replace(Some(gui::mailbox(|_: u8| {})));
            }
            gui::Element::Empty
        })
    });
    let _ = root.resolve();
    let sender = made.take().expect("rendered");
    assert!(sender.send(1).is_ok(), "mounted: its mailbox takes mail");
    gui::testing::in_check(move || drop(root));
    assert!(sender.send(2).is_ok(), "the check's drop retires nothing live");
    gui::engine::shutdown();
    assert!(sender.send(42).is_err(), "the root ended live: its mailbox is retired");
    scope.dispose();
}

// ── Codex r18 2: a tree laid out in a check and live members ─────────────

fn lay_out(tree: &mut LayoutTree, members: &gui::Members<i32>, value: i32) {
    tree.layout(
        &gui::div().member(members, value).into_element(),
        100.,
        100.,
        &UiState::default(),
        &mut gui::Motion::default(),
        &mut gui::text::TextSystem::new(),
    );
}

/// A tree a check makes and lays out is the check's: it publishes nothing
/// into a live list of members, whichever constructor made it.
#[test]
fn a_tree_laid_out_in_a_check_leaves_live_members() {
    let members = gui::Members::new();
    let mut live = LayoutTree::new();
    lay_out(&mut live, &members, 1);
    assert_eq!(members.get(), [1]);
    let held = members.clone();
    gui::testing::in_check(move || lay_out(&mut LayoutTree::new(), &held, 2));
    assert_eq!(members.get(), [1], "the check's tree took the live list over");
    lay_out(&mut live, &members, 3);
    assert_eq!(members.get(), [3], "the live tree still fills it");
}

// ── Codex r18 nit: a check's read and a live registry's count ────────────

/// A check reading a live registry writes nothing of it: its work count
/// included.
#[test]
fn a_check_reading_a_live_registry_leaves_its_count() {
    let scope = gui::Scope::new();
    let registry = scope.run(gui::Registry::<u8>::new);
    let root = scope.run(|| {
        gui::Root::new(move || {
            registry.register(7);
            gui::Element::Empty
        })
    });
    let _ = root.resolve();
    gui::engine::shutdown();
    assert_eq!(registry.to_vec_untracked(), [7]);
    let before = registry.visits();
    let seen = gui::testing::in_check(move || registry.to_vec_tracked());
    assert_eq!(seen, [7], "the check reads the live entries");
    assert_eq!(
        registry.visits(),
        before,
        "the check's read counted on the live registry"
    );
    drop(root);
    scope.dispose();
}
