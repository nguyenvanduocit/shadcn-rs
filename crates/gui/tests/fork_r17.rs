//! Review r17's reproducers (ADR-0034): state outside the registry that a
//! check reached through `&` (engine handles' fields, sealed values that
//! were not data, lifecycle work a check queued live), a drain that boxed
//! a container whole, and constructors that ignored their owner.
#![cfg(debug_assertions)]
#![expect(clippy::panic, reason = "values whose drop panics, as the reproducers need")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// What a panic said.
fn said(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&'static str>()
        .map(|s| (*s).to_owned())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

// ── Codex r17 1: two logged callbacks dropped under one catch abort ──────

struct Bomb;

impl Drop for Bomb {
    fn drop(&mut self) {
        panic!("job capture");
    }
}

/// A logged list of callbacks, by key.
type Jobs = gui::engine::ForkMap<u64, Vec<gui::engine::AppHandle<dyn Fn()>>>;

gui::engine_local! {
    [Fork(log)] static JOBS: RefCell<Jobs> = RefCell::default();
}

/// Two callbacks a check logs in one `Vec`, each capturing a value whose
/// drop panics: each is its own box, so both faults are reported and the
/// process does not abort. Run in a child process (an abort kills it).
#[test]
fn separate_logged_jobs() {
    if std::env::var_os("GUI_R17_CHILD").is_some() {
        JOBS.with(|_| ());
        let result = std::panic::catch_unwind(|| {
            gui::testing::in_check(|| {
                let jobs = (0..2)
                    .map(|_| {
                        let b = Bomb;
                        let job: Rc<dyn Fn()> = Rc::new(move || {
                            std::hint::black_box(&b);
                        });
                        gui::engine::AppHandle::new(job)
                    })
                    .collect();
                JOBS.with_borrow_mut(|m| m.replace(1, jobs));
            });
        });
        let said = said(&result.expect_err("the check faults"));
        assert_eq!(said.matches("job capture").count(), 2, "{said}");
        return;
    }
    let child = std::process::Command::new(std::env::current_exe().expect("the test binary"))
        .args(["--exact", "separate_logged_jobs", "--nocapture", "--test-threads=1"])
        .env("GUI_R17_CHILD", "1")
        .output()
        .expect("the child runs");
    assert!(
        child.status.success(),
        "{}: {}",
        child.status,
        String::from_utf8_lossy(&child.stderr)
    );
}

// ── Coordinator r17: a list state's app callback outside its split ───────

struct ListBomb;

impl Drop for ListBomb {
    fn drop(&mut self) {
        panic!("list hook capture");
    }
}

/// Two live lists a check gives layout hooks (app closures), each holding a
/// value whose drop panics: the check's copies of the lists' state drain
/// with each hook on a box of its own, so both faults are reported and
/// the process does not abort. Run in a child process (an abort kills it).
#[test]
fn a_check_drains_each_list_hook_on_its_own() {
    if std::env::var_os("GUI_R17_LIST_CHILD").is_some() {
        let lists = Rc::new([gui::ListState::new(24.0), gui::ListState::new(24.0)]);
        let result = std::panic::catch_unwind(|| {
            let lists = Rc::clone(&lists);
            gui::testing::in_check(move || {
                for list in lists.iter() {
                    let b = ListBomb;
                    list.on_layout(move |_| {
                        std::hint::black_box(&b);
                        false
                    });
                }
            });
        });
        let said = said(&result.expect_err("the check faults"));
        assert_eq!(said.matches("list hook capture").count(), 2, "{said}");
        return;
    }
    let child = std::process::Command::new(std::env::current_exe().expect("the test binary"))
        .args([
            "--exact",
            "a_check_drains_each_list_hook_on_its_own",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("GUI_R17_LIST_CHILD", "1")
        .output()
        .expect("the child runs");
    assert!(
        child.status.success(),
        "{}: {}",
        child.status,
        String::from_utf8_lossy(&child.stderr)
    );
}

// ── Codex r17 4, Opus r17 I1: a check resolving a poisoned live root ─────

/// A check that resolves a live root a panic poisoned leaves it poisoned,
/// its remount still pending: the live resolve after mounts and renders.
#[test]
fn a_check_leaves_a_poisoned_live_root_to_remount_live() {
    let fail = Rc::new(Cell::new(true));
    let renders = Rc::new(Cell::new(0_u32));
    let seen = Rc::new(Cell::new(None));
    let scope = gui::Scope::new();
    let (f, r, s) = (Rc::clone(&fail), Rc::clone(&renders), Rc::clone(&seen));
    let root = scope.run(|| {
        gui::Root::new(move || {
            r.set(r.get() + 1);
            assert!(!f.get(), "render fails");
            s.set(Some(gui::Signal::new(17)));
            gui::Element::Empty
        })
    });
    let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| root.resolve()));
    assert!(first.is_err() && root.is_poisoned(), "poisoned live");
    fail.set(false);
    let root = Rc::new(root);
    let held = Rc::clone(&root);
    gui::testing::in_check(move || {
        let _ = held.resolve();
    });
    assert!(root.is_poisoned(), "the check cleared the live remount");
    let before = renders.get();
    seen.set(None);
    let _ = root.resolve();
    assert_eq!(renders.get(), before + 1, "the live resolve rendered");
    assert!(seen.get().is_some(), "the live root mounted again");
    drop(root);
    scope.dispose();
}

/// Opus r17 I1, its second path: a render that panics inside the check
/// poisons the check's world, never the live root. The live root is not
/// rendered yet, so the check's resolve renders it first and that render
/// panics (review r18: the earlier form never rendered in the check).
#[test]
fn a_render_panicking_in_a_check_leaves_the_live_root_whole() {
    let fail = Rc::new(Cell::new(true));
    let renders = Rc::new(Cell::new(0_u32));
    let seen = Rc::new(Cell::new(None));
    let scope = gui::Scope::new();
    let (f, r, s) = (Rc::clone(&fail), Rc::clone(&renders), Rc::clone(&seen));
    let root = scope.run(|| {
        gui::Root::new(move || {
            r.set(r.get() + 1);
            assert!(!f.get(), "render fails");
            s.set(Some(gui::Signal::new(17)));
            gui::Element::Empty
        })
    });
    let root = Rc::new(root);
    let held = Rc::clone(&root);
    let in_check = gui::testing::in_check(move || {
        let resolved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| held.resolve()));
        (resolved.is_err(), held.is_poisoned())
    });
    assert_eq!(renders.get(), 1, "the check's resolve did the first render");
    assert_eq!(
        in_check,
        (true, true),
        "the render panicked and poisoned the check's root"
    );
    assert!(!root.is_poisoned(), "the check's panic poisoned the live root");
    fail.set(false);
    let _ = root.resolve();
    assert_eq!(renders.get(), 2, "the live resolve rendered");
    let live = seen.get().expect("rendered live");
    assert!(live.is_alive(), "the live tree is mounted");
    drop(root);
    scope.dispose();
}

// ── Codex r17 3: a check closing a window ────────────────────────────────

/// A check that closes a live window is refused, a fault (ADR-0035, D3:
/// a value's end outside the world that made it): the live root stands,
/// and the live close after still unmounts the root the window owns.
#[test]
fn window_owns_root_after_check_close() {
    let mut w = gui::app::Pipeline::new(|| gui::Element::Empty, false);
    let seen = Rc::new(Cell::new(None));
    let output = Rc::clone(&seen);
    let mut held = None;
    w.act("make", || {
        let r = gui::Root::new(move || {
            output.set(Some(gui::Signal::new(1)));
            gui::Element::Empty
        });
        let _ = r.resolve();
        held = Some(r);
    });
    let s = seen.get().expect("rendered");
    let w = Rc::new(std::cell::RefCell::new(w));
    let handle = Rc::clone(&w);
    let checked = gui::testing::check(move || drop(handle.borrow_mut().close()));
    assert!(
        matches!(&checked, Err(gui::engine::CheckReport::Faulted(why)) if why.contains("Foreign")),
        "{checked:?}"
    );
    gui::engine::shutdown();
    assert!(s.is_alive(), "the check's close unmounted the live root");
    w.borrow_mut().close().expect("closed");
    gui::engine::shutdown();
    assert!(!s.is_alive(), "live window close missed its registered root");
    drop(held);
}

/// A check that disposes of a live scope is refused, a fault (ADR-0035,
/// D3): the scope's roots and signals live on, and its live dispose ends
/// them.
#[test]
fn a_check_disposing_a_live_scope_ends_nothing_live() {
    let scope = gui::Scope::new();
    let seen = Rc::new(Cell::new(None));
    let s = Rc::clone(&seen);
    let root = scope.run(|| {
        gui::Root::new(move || {
            s.set(Some(gui::Signal::new(1)));
            gui::Element::Empty
        })
    });
    let _ = root.resolve();
    let signal = seen.get().expect("rendered");
    let checked = gui::testing::check(move || scope.dispose());
    assert!(
        matches!(&checked, Err(gui::engine::CheckReport::Faulted(why)) if why.contains("Foreign")),
        "{checked:?}"
    );
    gui::engine::shutdown();
    assert!(signal.is_alive(), "the check's dispose unmounted the live root");
    scope.dispose();
    gui::engine::shutdown();
    assert!(!signal.is_alive(), "the live dispose ended it");
    drop(root);
}

// ── Codex r17 5: views made with no owner ────────────────────────────────

/// Views with no scope and no window would end on their last handle
/// alone, which a runaway can pin: refused (debug).
#[test]
fn ownerless_views_are_refused() {
    let made = std::panic::catch_unwind(|| {
        drop(gui::list::Views::new(
            &gui::list::ListState::new(24.0),
            gui::list::View::identity(1),
            |_| {},
        ));
    });
    let said = said(&made.expect_err("no owner: refused"));
    assert!(said.contains("no scope running"), "{said}");
}

/// A mailbox made with no scope and no window would let go of its handler
/// only as its senders go, which a runaway can pin: refused (debug).
#[test]
fn ownerless_mailboxes_are_refused() {
    let made = std::panic::catch_unwind(|| drop(gui::mailbox(|_: u8| {})));
    let said = said(&made.expect_err("no owner: refused"));
    assert!(said.contains("no scope running"), "{said}");
}

// ── Opus r17 I2/I3, Codex r17 2: a sealed value that is not data ─────────

// `Rc<dyn Any>` and `&'static Cell` are no logged values any more (their
// compile-fail fixtures are on `gui::engine::ForkValue`); an app object
// goes in by `AppHandle`, whose contract is the render-purity rule.

gui::engine_local! {
    [Fork(log)] static OWNER: RefCell<gui::engine::ForkMap<u64, gui::engine::AppHandle<dyn std::any::Any>>> = RefCell::default();
}

/// Codex r17 2 / Opus r17 I2, against the contract: a check isolates which
/// handles a logged map holds, so its replace and forget are undone; the
/// object behind a handle is app heap, as an `Rc` a closure captured is.
/// Taking a live root out of it in a check takes it out live, and its
/// unmount follows, consistently (ADR-0034 S7, "What a check isolates").
#[test]
fn an_app_handle_isolates_the_map_not_the_app_object() {
    let scope = gui::Scope::new();
    let seen = Rc::new(Cell::new(None));
    let s = Rc::clone(&seen);
    let root = scope.run(|| {
        gui::Root::new(move || {
            s.set(Some(gui::Signal::new(1)));
            gui::Element::Empty
        })
    });
    let _ = root.resolve();
    let signal = seen.get().expect("rendered");
    let object: Rc<dyn std::any::Any> = Rc::new(Cell::new(Some(root)));
    OWNER.with_borrow_mut(|m| m.replace(1, gui::engine::AppHandle::new(object)));
    // The map's structure: a check's replace and forget are its own.
    gui::testing::in_check(|| {
        OWNER.with_borrow_mut(|m| {
            m.replace(2, gui::engine::AppHandle::new(Rc::new(7_u8)));
            m.forget(&1);
        });
    });
    OWNER.with_borrow(|m| assert!(m.contains_key(&1) && !m.contains_key(&2), "the map as live had it"));
    // The object behind a handle: app heap, the check's write is live.
    gui::testing::in_check(|| {
        OWNER.with_borrow(|m| {
            let cell = m.get(&1).expect("logged").downcast_ref::<Cell<Option<gui::Root>>>();
            drop(cell.expect("a cell").take());
        });
    });
    gui::engine::shutdown();
    let still = OWNER.with_borrow(|m| {
        m.get(&1)
            .and_then(|h| h.downcast_ref::<Cell<Option<gui::Root>>>().map(|c| c.take().is_some()))
    });
    assert_eq!(still, Some(false), "taken from app heap, as the contract says");
    assert!(!signal.is_alive(), "and unmounted live, consistently");
    scope.dispose();
}
