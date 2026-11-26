use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use super::*;
use crate::reactive::Scope;

/// A post is delivered now, in order; one made while `deliver` runs waits
/// for it to return and is never delivered by entering it again.
#[test]
fn a_post_is_delivered_at_once_and_a_post_inside_deliver_waits() {
    let scope = Scope::detached();
    let got: Rc<RefCell<Vec<String>>> = Rc::default();
    let into = Rc::clone(&got);
    let me: Rc<RefCell<Option<Port<u32>>>> = Rc::default();
    let again = Rc::clone(&me);
    let p = scope.run(|| {
        port(move |n: u32| {
            into.borrow_mut().push(format!("in {n}"));
            if n == 1
                && let Some(p) = *again.borrow()
            {
                p.post(2);
            }
            into.borrow_mut().push(format!("out {n}"));
        })
    });
    *me.borrow_mut() = Some(p);
    p.post(1);
    assert_eq!(*got.borrow(), ["in 1", "out 1", "in 2", "out 2"], "never re-entered");
    p.post(3);
    assert_eq!(got.borrow().len(), 6, "delivered at once");
    scope.dispose();
}

/// A port closes with its scope: a post after is dropped, its `deliver`
/// let go of.
#[test]
fn a_port_closes_with_its_scope() {
    let scope = Scope::detached();
    let got = Rc::new(RefCell::new(0_u32));
    let into = Rc::clone(&got);
    let p = scope.run(|| port(move |n: u32| *into.borrow_mut() += n));
    p.post(1);
    assert!(p.is_open());
    scope.dispose();
    assert!(!p.is_open());
    p.post(10);
    assert_eq!(*got.borrow(), 1, "nothing delivered after its scope ended");
    assert_eq!(Rc::strong_count(&got), 1, "its deliver was let go of");
}

/// Inside a check a post reaches nothing (ADR-0028, rule 5), and a port the
/// check makes is gone with it.
#[cfg(debug_assertions)]
#[test]
fn a_check_posts_nothing_and_its_ports_go_with_it() {
    let scope = Scope::detached();
    let got = Rc::new(RefCell::new(Vec::new()));
    let into = Rc::clone(&got);
    let p = scope.run(|| port(move |n: u32| into.borrow_mut().push(n)));
    let made = crate::testing::in_check(move || {
        p.post(1);
        let inner = Scope::detached().run(|| port(|_: u32| {}));
        inner.post(2);
        inner
    });
    assert!(got.borrow().is_empty(), "the check's post reached nothing");
    assert!(!made.is_open(), "the check's own port went with it");
    p.post(3);
    assert_eq!(*got.borrow(), [3]);
    scope.dispose();
}

/// `then` runs on the UI thread, at a delivery, with the job's result.
#[test]
fn a_job_answers_on_the_ui_thread() {
    let scope = Scope::detached();
    let got = Rc::new(RefCell::new(None));
    let into = Rc::clone(&got);
    let ui = std::thread::current().id();
    let task = scope.run(|| {
        spawn(
            move |_| std::thread::current().id(),
            move |id| *into.borrow_mut() = Some(id),
        )
    });
    let mut waited = 0;
    while got.borrow().is_none() && waited < 500 {
        crate::mailbox::deliver();
        std::thread::sleep(Duration::from_millis(2));
        waited += 1;
    }
    let ran_on = got.borrow().expect("the answer came");
    assert_ne!(ran_on, ui, "the job ran off the UI thread");
    assert!(!task.is_cancelled());
    scope.dispose();
}

/// A job's owner ending cancels it: its token is set, its answer dropped.
#[test]
fn a_job_is_cancelled_with_its_scope() {
    let scope = Scope::detached();
    let (go, wait) = mpsc::channel::<()>();
    let (seen, saw) = mpsc::channel::<bool>();
    let ran = Rc::new(RefCell::new(false));
    let into = Rc::clone(&ran);
    let task = scope.run(|| {
        spawn(
            move |cancel| {
                wait.recv().ok();
                seen.send(cancel.is_cancelled()).ok();
            },
            move |()| *into.borrow_mut() = true,
        )
    });
    scope.dispose();
    assert!(task.is_cancelled(), "cancelled as its scope ended");
    go.send(()).ok();
    assert_eq!(saw.recv_timeout(Duration::from_secs(5)), Ok(true), "the job saw it");
    std::thread::sleep(Duration::from_millis(20));
    crate::mailbox::deliver();
    assert!(!*ran.borrow(), "its answer never ran");
}

/// Inside a check no job starts.
#[cfg(debug_assertions)]
#[test]
fn a_check_starts_no_job() {
    let (tx, rx) = mpsc::channel::<()>();
    let cancelled = crate::testing::in_check(move || {
        Scope::detached()
            .run(|| {
                spawn(
                    move |_| {
                        tx.send(()).ok();
                    },
                    |()| {},
                )
            })
            .is_cancelled()
    });
    assert!(cancelled, "the task never started");
    assert!(rx.recv_timeout(Duration::from_millis(50)).is_err(), "no job ran");
}
