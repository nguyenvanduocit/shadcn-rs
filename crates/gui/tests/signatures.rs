//! What component code sees at K4a (ADR-0026, ADR-0028 "What lands at
//! K4a"), from outside `gui`: a callback into a disposed owner answers
//! `None`, a send to a closed mailbox says `Closed`.

use gui::{Callback, Closed, Scope};

/// A callback whose owner was disposed does not run and gives `None` (it
/// once ran with no owner); a caller asking for a veto reads that as none.
#[test]
fn a_callback_into_a_disposed_owner_gives_none() {
    let scope = Scope::detached();
    let ran = std::rc::Rc::new(std::cell::Cell::new(0_u32));
    let r = std::rc::Rc::clone(&ran);
    let cb: Callback<u32, u32> = scope.run(|| {
        Callback::new(move |x: u32| {
            r.set(r.get().saturating_add(1));
            x.saturating_mul(2)
        })
    });
    assert_eq!(cb.call(3), Some(6));
    scope.dispose();
    assert_eq!(cb.call(3), None, "its owner is gone");
    assert_eq!(ran.get(), 1, "and it did not run");
    let vetoed = cb.call(0).is_some_and(|v| v == 0);
    assert!(!vetoed, "None is no veto");
}

/// A send to a mailbox whose scope ended says so.
#[test]
fn a_send_to_a_closed_mailbox_says_closed() {
    let scope = Scope::detached();
    let sender = scope.run(|| gui::mailbox(|_: u32| {}));
    assert_eq!(sender.send(1), Ok(()));
    scope.dispose();
    assert_eq!(sender.send(2), Err(Closed));
}
