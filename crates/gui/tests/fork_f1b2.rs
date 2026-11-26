//! F1b review round 1's reproducers (Codex): a check opened while user code
//! holds a live lend (ADR-0035, D4), and a retirement that let an app value
//! dispose of a live scope (D3).
#![cfg(debug_assertions)]

use gui::testing::check;

/// Codex r1-1. Its form, `WorldRefCell::get_mut` held into a check, no
/// longer compiles (the compile-fail doctest in `engine/world_cell.rs`),
/// nor does a check body borrowing the lent queue (a check's body is
/// `'static`, F1b r3): a check opened inside the lend, reaching the list
/// by handle, is refused, the live list given no `Close`.
#[test]
fn action_lend() {
    use gui::window::decoration::{Actions, WindowAction};
    let actions = Actions::default();
    let handle = std::rc::Rc::clone(&actions);
    let result = actions.with_mut(|_| check(move || handle.with_mut(|queue| queue.push(WindowAction::Close))));
    assert!(actions.take().is_empty(), "live Close queued, {result:?}");
    assert!(
        matches!(&result, Err(gui::engine::CheckReport::Skipped(why)) if why.contains("InUserCode")),
        "{result:?}"
    );
}

/// Codex r1-2. Its form, `Page::with` lending a live page into a check, no
/// longer compiles (the compile-fail doctest on `chunk::series::Page`): app
/// code reads points by copy, and a check reading the snapshot sees the
/// prefix it holds.
#[test]
fn page_lend() {
    let scope = gui::Scope::new();
    let series = scope.run(gui::chunk::plot::series);
    series.push(1, 1.0);
    let snapshot = series.snapshot_untracked();
    let held = snapshot.clone();
    let result = check(move || held.get(0));
    series.push(2, 2.0);
    assert_eq!(snapshot.get(1), None, "the prefix is the snapshot's");
    scope.dispose();
    assert_eq!(result.ok().flatten(), Some((1, 1.0)));
}

/// A check opened inside any user code the engine calls is refused: a
/// render, an effect, a handler (ADR-0035, D4).
#[test]
fn a_check_inside_user_code_is_refused() {
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let s = std::rc::Rc::clone(&seen);
    let scope = gui::Scope::new();
    let root = scope.run(|| {
        gui::Root::new(move || {
            s.borrow_mut().push(format!("{:?}", check(|| 1)));
            gui::Element::Empty
        })
    });
    let _ = root.resolve();
    let s = std::rc::Rc::clone(&seen);
    let _effect = scope.run(|| gui::reactive::Effect::new(move || s.borrow_mut().push(format!("{:?}", check(|| 2)))));
    let s = std::rc::Rc::clone(&seen);
    let computed = scope.run(|| gui::reactive::Computed::new(|| 3_u8));
    computed.with(|_| s.borrow_mut().push(format!("{:?}", check(|| 3))));
    let got = seen.borrow().clone();
    assert_eq!(got.len(), 3, "{got:?}");
    assert!(got.iter().all(|r| r.contains("InUserCode")), "{got:?}");
    assert!(check(|| 4).is_ok(), "checks open again outside");
    drop(root);
    scope.dispose();
}

#[test]
fn retirement() {
    struct Dispose(gui::Scope);
    impl Drop for Dispose {
        fn drop(&mut self) {
            self.0.dispose();
        }
    }
    let victim = gui::Scope::new();
    let result = check(move || {
        let owner = gui::Scope::new();
        let root = owner.run(|| {
            gui::Root::new(move || {
                let guard = Dispose(victim);
                gui::reactive::on_cleanup(move || drop(guard));
                panic!("render");
            })
        });
        drop(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            root.resolve()
        })));
        victim.is_alive()
    });
    assert!(victim.is_alive(), "rollback restored the live scope");
    victim.dispose();
    assert!(
        matches!(&result, Err(gui::engine::CheckReport::Faulted(s)) if s.contains("Foreign")),
        "{result:?}"
    );
}

/// Codex r1-4. Its form, the lent `&mut` written in a check, no longer
/// compiles (a check's body is `'static`); a check opened inside the
/// update, writing the signal by handle, is refused.
#[test]
fn signal_lend() {
    let value = gui::Signal::new(7);
    let mut result = None;
    value.update(|_| result = Some(check(move || value.set(9))));
    let result = result.expect("the update ran");
    assert!(
        matches!(&result, Err(gui::engine::CheckReport::Skipped(why)) if why.contains("InUserCode")),
        "{result:?}"
    );
    assert_eq!(value.get_untracked(), 7);
}
