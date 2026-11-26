//! ADR-0034 after review r15 (Codex 1 to 5): the fork machinery owns only
//! what the fork made. Live handles, live owners and undo state never pass
//! through a box the fork drops, forgets or replaces. Each test is a
//! reviewer's reproducer, run in a process of its own test thread.
#![cfg(debug_assertions)]
#![expect(clippy::panic, reason = "values whose drop panics, as the reproducers need")]
#![expect(clippy::expect_used, reason = "a reproducer that cannot run is a failed test")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

// ── Codex r15-1: a user destructor runs during the switch ──────────────────

static RESTORE_DROPS: AtomicUsize = AtomicUsize::new(0);

/// Records whether it was dropped inside the switch back to live.
struct Marker;

impl Drop for Marker {
    fn drop(&mut self) {
        let stack = std::backtrace::Backtrace::force_capture().to_string();
        if stack.contains("engine::frame::restore") {
            RESTORE_DROPS.fetch_add(1, SeqCst);
        }
    }
}

/// An app's hasher whose drop, in a check, writes a registered cell.
#[derive(Default)]
struct AppHasher;

impl std::hash::BuildHasher for AppHasher {
    type Hasher = std::collections::hash_map::DefaultHasher;
    fn build_hasher(&self) -> Self::Hasher {
        std::collections::hash_map::DefaultHasher::new()
    }
}

impl Drop for AppHasher {
    fn drop(&mut self) {
        if gui::engine::forking() {
            COPY.set(Some(Rc::new(Marker)));
        }
    }
}

gui::engine_local! {
    [Fresh] static MAP: RefCell<HashMap<u8, u8, AppHasher>> = RefCell::default();
    [Fork] static COPY: RefCell<Option<Rc<Marker>>> = RefCell::default();
}

#[test]
fn the_switch_runs_no_destructor_a_drain_wrote() {
    MAP.with(|_| ());
    COPY.with(|_| ());
    let ended = std::panic::catch_unwind(|| gui::testing::in_check(|| MAP.with(|_| ())));
    assert!(ended.is_ok(), "the check ends: the drain reached a fixed point");
    assert_eq!(RESTORE_DROPS.load(SeqCst), 0, "a user Drop ran inside restore");
}

// ── Codex r15-2: taking a logged map commits a check's writes ──────────────

gui::engine_local! {
    [Fork(log)] static ROWS: RefCell<gui::engine::ForkMap<u64, u8>> = RefCell::default();
    [Fork(log)] static SPARE: RefCell<gui::engine::ForkMap<u64, u8>> = RefCell::default();
}

/// The reviewer's `drop(ROWS.take())` no longer compiles: a logged key has
/// no whole-value `take`, `set` or cell, and its write borrow no `&mut`
/// (the compile-fail fixtures of `gui::engine::LogKey`). What a caller can
/// still write to empty the map or put another in its place: swap the
/// views of two logged maps, then forget every row through the swapped one,
/// then write. Each goes through the overlay: live rows stay, and none of
/// the check's is published.
#[test]
fn a_check_that_takes_a_logged_map_leaves_live_rows() {
    ROWS.with_borrow_mut(|rows| rows.replace(1, 7));
    SPARE.with_borrow_mut(|spare| spare.replace(5, 3));
    gui::testing::in_check(|| {
        ROWS.with(|rows| {
            SPARE.with(|spare| {
                let (mut rows, mut spare) = (rows, spare);
                std::mem::swap(&mut rows, &mut spare);
                let mut spare = spare.borrow_mut();
                let keys: Vec<u64> = spare.iter().map(|(k, _)| *k).collect();
                for k in keys {
                    spare.forget(&k);
                }
            });
        });
        assert_eq!(
            ROWS.with_borrow(gui::engine::ForkMap::len),
            0,
            "the check sees the map empty"
        );
        ROWS.with_borrow_mut(|rows| rows.replace(2, 9));
    });
    assert_eq!(
        ROWS.with(|r| (r.borrow().get(&1).copied(), r.borrow().get(&2).copied())),
        (Some(7), None)
    );
    assert_eq!(SPARE.with_borrow(|s| s.get(&5).copied()), Some(3));
}

// ── Codex r15-3: an unrelated disposal fault pins an untouched live owner ──

struct Bomb;

impl gui::engine::FreshValue for Bomb {}

impl Drop for Bomb {
    fn drop(&mut self) {
        panic!("bomb");
    }
}

gui::engine_local! {
    [Fork] static OWNER: RefCell<Option<Rc<gui::Root>>> = RefCell::default();
    [Fresh] static BOMBS: RefCell<Vec<Bomb>> = RefCell::default();
}

#[test]
fn an_unrelated_fault_pins_no_live_owner() {
    let owner = Rc::new(gui::Scope::detached().run(|| gui::Root::new(|| gui::Element::Empty)));
    let weak = Rc::downgrade(&owner);
    OWNER.set(Some(owner));
    BOMBS.with(|_| ());
    let failed = std::panic::catch_unwind(|| {
        gui::testing::in_check(|| BOMBS.with_borrow_mut(|v| v.push(Bomb)));
    });
    assert!(failed.is_err(), "the check faults at the bomb");
    drop(OWNER.take());
    assert!(weak.upgrade().is_none(), "the check pinned the live owner");
}

// ── Codex r15-4: a live root dropped while the body unwinds keeps its unmount ──

#[test]
fn a_live_root_dropped_as_a_check_unwinds_is_unmounted_live() {
    let (root, signal) = root_in(gui::Scope::detached());
    assert!(signal.is_alive());
    let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gui::testing::in_check(move || {
            let _root = root;
            panic!("body");
        });
    }));
    assert!(ended.is_err(), "the body's panic is raised");
    gui::engine::shutdown();
    assert!(!signal.is_alive(), "the root is gone and its signal still lives");
}

#[test]
fn a_live_root_dropped_in_a_check_is_unmounted_live() {
    let (root, signal) = root_in(gui::Scope::detached());
    let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gui::testing::in_check(move || drop(root));
    }));
    assert!(ended.is_ok(), "the check ends");
    assert!(signal.is_alive(), "not in the check: the unmount is live work");
    gui::engine::shutdown();
    assert!(!signal.is_alive(), "the root is gone and its signal still lives");
}

// ── Codex r15-5: a map value's destructor runs after its key's failed ──────

#[derive(PartialEq, Eq, Hash)]
struct Key;

impl gui::engine::FreshValue for Key {}

impl Drop for Key {
    fn drop(&mut self) {
        panic!("key fails");
    }
}

static VALUE_DROPS: AtomicUsize = AtomicUsize::new(0);

struct Value;

impl gui::engine::FreshValue for Value {}

impl Drop for Value {
    fn drop(&mut self) {
        VALUE_DROPS.fetch_add(1, SeqCst);
    }
}

gui::engine_local! {
    [Fresh] static PAIRS: RefCell<HashMap<Key, Value>> = RefCell::default();
}

/// Since review r16 (S2 revised): the key's drop faults, the value is
/// still dropped, once, on its own (the two panics never meet in one
/// unwind), as `drop_in_place` drops every field past a panicking one.
#[test]
fn a_keys_failed_drop_still_drops_its_value_once() {
    let fault = std::panic::catch_unwind(|| {
        gui::testing::in_check(|| {
            PAIRS.with_borrow_mut(|m| {
                m.insert(Key, Value);
            });
        });
    });
    assert!(fault.is_err());
    assert_eq!(
        VALUE_DROPS.load(SeqCst),
        1,
        "the value is dropped once though its key's drop panicked"
    );
}

// ── Review r16: a disposal fault pins nothing live; obligations end with ──
// ── their owner; a logged read lends no live value to take ──────────────

/// Sets its flag as it drops.
struct Flag(Rc<std::cell::Cell<bool>>);

impl Drop for Flag {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

/// A live root rendering one signal, made in `scope`: the root and the
/// signal its render made.
fn root_in(scope: gui::Scope) -> (gui::Root, gui::Signal<i32>) {
    let seen = Rc::new(std::cell::Cell::new(None));
    let publish = Rc::clone(&seen);
    let root = scope.run(|| {
        gui::Root::new(move || {
            publish.set(Some(gui::Signal::new(17)));
            gui::Element::Empty
        })
    });
    let _ = root.resolve();
    let signal = seen.get().expect("the root rendered");
    (root, signal)
}

gui::engine_local! {
    [Fresh] static BOMBS16: RefCell<Vec<Bomb>> = RefCell::default();
    [Fork(log)] static HELD: RefCell<gui::engine::ForkMap<u64, gui::engine::AppHandle<dyn std::any::Any>>> = RefCell::default();
    [Fork] static POINTED: RefCell<Option<Rc<Flag>>> = RefCell::default();
    [Fresh] static JOBS: RefCell<Vec<Box<dyn Fn()>>> = RefCell::default();
}

/// Opus r16 I1 (1): the engine's own copy of a live value, made by a
/// logged map's first write in a check, is dropped though an unrelated
/// value's drop faults: the live value ends with its live owner.
#[test]
fn a_logged_maps_copy_of_a_live_value_ends_after_a_fault() {
    let dropped = Rc::new(std::cell::Cell::new(false));
    let live: Rc<dyn std::any::Any> = Rc::new(Flag(Rc::clone(&dropped)));
    HELD.with_borrow_mut(|h| h.replace(1, gui::engine::AppHandle::new(live)));
    BOMBS16.with(|_| ());
    let failed = std::panic::catch_unwind(|| {
        gui::testing::in_check(|| {
            HELD.with_borrow_mut(|h| {
                if let Some(copied) = h.get(&1).cloned() {
                    h.insert(1, copied);
                }
            });
            BOMBS16.with_borrow_mut(|b| b.push(Bomb));
        });
    });
    assert!(failed.is_err(), "the check faults at the bomb");
    HELD.with_borrow_mut(|h| h.forget(&1));
    assert!(
        dropped.get(),
        "the fork's copy pinned the live value past its live owner"
    );
}

/// Opus r16 I1 (2): a scroll handle's shadow shares the live callbacks; a
/// fault after the engine's write pins none of them.
#[test]
fn a_scroll_shadow_pins_no_live_callback_after_a_fault() {
    let handle = gui::ScrollHandle::new();
    let dropped = Rc::new(std::cell::Cell::new(false));
    let flag = Flag(Rc::clone(&dropped));
    handle.on_reveal(move |_| {
        let _held = &flag;
        false
    });
    BOMBS16.with(|_| ());
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gui::testing::in_check(|| {
            handle.scroll_to((0.0, 5.0));
            BOMBS16.with_borrow_mut(|b| b.push(Bomb));
        });
    }));
    assert!(failed.is_err(), "the check faults at the bomb");
    handle.dispose();
    assert!(
        dropped.get(),
        "the fork's shadow pinned the live callback past its handle"
    );
}

/// Opus r16 I1 (3): a `[Fork]` cell a check pointed at a live value lets
/// it go though an unrelated drop faults.
#[test]
fn a_fork_cell_pointed_at_a_live_value_lets_it_go_after_a_fault() {
    let dropped = Rc::new(std::cell::Cell::new(false));
    let live = Rc::new(Flag(Rc::clone(&dropped)));
    POINTED.with(|_| ());
    BOMBS16.with(|_| ());
    let also = Rc::clone(&live);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gui::testing::in_check(move || {
            POINTED.set(Some(also));
            BOMBS16.with_borrow_mut(|b| b.push(Bomb));
        });
    }));
    assert!(failed.is_err(), "the check faults at the bomb");
    drop(live);
    assert!(
        dropped.get(),
        "the fork's cell pinned the live value past its live owner"
    );
}

/// Codex r16 2: a closure a check made holds a live root; an unrelated
/// drop faults first. The closure is still dropped, so the root unmounts
/// as its owner lets go of it.
#[test]
fn a_standalone_root_a_faulted_check_held_unmounts_with_its_owner() {
    let scope = gui::Scope::new();
    let (root, signal) = root_in(scope);
    let root = Rc::new(root);
    JOBS.with(|_| ());
    let fault = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gui::testing::in_check(|| {
            let bomb = Bomb;
            let held = Rc::clone(&root);
            JOBS.with_borrow_mut(|q| {
                q.push(Box::new(move || {
                    let _held = &bomb;
                }));
                q.push(Box::new(move || {
                    let _held = &held;
                }));
            });
        });
    }));
    assert!(fault.is_err(), "the check faults at the bomb");
    drop(root);
    gui::engine::shutdown();
    assert!(!signal.is_alive(), "the root's signal outlived the root's owner");
    scope.dispose();
}

/// Rescheduled by its own drop, holding a live root: a runaway.
struct Again(Rc<gui::Root>);

impl gui::engine::FreshValue for Again {}

impl Drop for Again {
    fn drop(&mut self) {
        let root = Rc::clone(&self.0);
        AGAIN.with_borrow_mut(|a| a.push(Again(root)));
    }
}

gui::engine_local! {
    [Fresh] static AGAIN: RefCell<Vec<Again>> = RefCell::default();
}

/// Codex r16 2, under a runaway: what is left is set aside, the root's
/// last handle among it; the root unmounts all the same as the scope that
/// made it ends (an obligation never ends on its last `Rc`).
#[test]
fn a_root_a_runaway_set_aside_unmounts_with_its_scope() {
    let scope = gui::Scope::new();
    let (root, signal) = root_in(scope);
    let root = Rc::new(root);
    AGAIN.with(|_| ());
    let held = Rc::clone(&root);
    let fault = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gui::testing::in_check(move || AGAIN.with_borrow_mut(|a| a.push(Again(held))));
    }));
    let said = fault.expect_err("the runaway faults the check");
    let said = said.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(said.contains("Runaway"), "{said}");
    drop(root);
    gui::engine::shutdown();
    assert!(signal.is_alive(), "set aside: the root's last handle is pinned");
    scope.dispose();
    gui::engine::shutdown();
    assert!(!signal.is_alive(), "the root outlived the scope that owns it");
}

/// Decision 2 with no scope running: a root made in a window's code is
/// that window's. A handle something pins (as a runaway's set-aside does)
/// keeps the count up, and the root unmounts as the window closes.
#[test]
#[expect(clippy::mem_forget, reason = "a pinned handle, as a runaway's set-aside leaves one")]
fn a_root_made_in_a_window_unmounts_as_it_closes_though_pinned() {
    let mut window = gui::app::Pipeline::new(|| gui::Element::Empty, false);
    let seen = Rc::new(std::cell::Cell::new(None));
    let publish = Rc::clone(&seen);
    let made = Rc::new(RefCell::new(None));
    let into = Rc::clone(&made);
    window.act("make a root", move || {
        let root = gui::Root::new(move || {
            publish.set(Some(gui::Signal::new(17)));
            gui::Element::Empty
        });
        let _ = root.resolve();
        *into.borrow_mut() = Some(root);
    });
    let root = Rc::new(made.borrow_mut().take().expect("made"));
    let signal = seen.get().expect("the root rendered");
    std::mem::forget(Rc::clone(&root));
    drop(root);
    gui::engine::shutdown();
    assert!(signal.is_alive(), "pinned: its last handle never drops");
    window.close().expect("closed");
    gui::engine::shutdown();
    assert!(!signal.is_alive(), "the root outlived the window that owns it");
}

// Codex r16 1a (a logged read lends a `Cell<Option<Root>>` a check takes
// the live root out of) and 1b (a `ForkSlab` nested in a `ForkMap` keeps a
// check's write for the next) no longer compile: a logged value is a
// `gui::engine::ForkValue`, which neither is. Their compile-fail fixtures,
// with positive controls, are on `ForkValue`.

/// A logged value the engine copies (`Rc<dyn Any>`, a context's) is read
/// back, by every check, as live has it.
#[test]
fn a_logged_handle_starts_each_check_from_live() {
    let live: Rc<dyn std::any::Any> = Rc::new(7_u8);
    HELD.with_borrow_mut(|h| h.replace(2, gui::engine::AppHandle::new(live)));
    let read = || HELD.with_borrow(|h| h.get(&2).and_then(|v| v.downcast_ref::<u8>().copied()));
    gui::testing::in_check(|| {
        HELD.with_borrow_mut(|h| h.insert(2, gui::engine::AppHandle::new(Rc::new(9_u8))));
    });
    assert_eq!(read(), Some(7));
    assert_eq!(
        gui::testing::in_check(read),
        Some(7),
        "the next check read the last one's write"
    );
}
