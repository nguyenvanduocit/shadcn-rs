//! Values other threads hand the UI thread: a [`mailbox`] runs its handler
//! on the UI thread with each value sent to it, before the next frame, and
//! the window wakes for it. A handler may write signals, as a click does; a
//! network thread that receives what a screen shows sends it here.

use std::cell::RefCell;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};

/// Wakes the window's event loop for one value sent to the mailbox at an
/// address, from any thread: a window's wake sends it through the one
/// producer of the UI's input (ADR-0033), which logs it where the UI takes
/// it, in order with the input.
pub(crate) type Wake = Arc<dyn Fn(Address) + Send + Sync>;

/// Which mailbox: the site its maker was called from (a stable hash of the
/// file, line and column) and how many mailboxes that site made before it
/// on this thread. The same app made the same way numbers its mailboxes
/// the same, live and in a replay, whatever else (the devtools, a
/// readback) made one meanwhile: what an input log names a delivery by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Address {
    pub(crate) site: u64,
    pub(crate) nth: u64,
}

/// How one mailbox wakes the window, once per value sent: set once the
/// window runs, with the values sent before that rung for then.
struct Bell {
    at: Address,
    wake: Mutex<(Option<Wake>, u64)>,
}

impl Bell {
    fn set(&self, wake: Wake) {
        let unrung = {
            let mut w = self.wake.lock().unwrap_or_else(PoisonError::into_inner);
            w.0 = Some(Arc::clone(&wake));
            std::mem::take(&mut w.1)
        };
        for _ in 0..unrung {
            wake(self.at);
        }
    }

    fn ring(&self) {
        let wake = {
            let mut w = self.wake.lock().unwrap_or_else(PoisonError::into_inner);
            if w.0.is_none() {
                w.1 = w.1.saturating_add(1);
            }
            w.0.clone()
        };
        if let Some(wake) = wake {
            wake(self.at);
        }
    }
}

/// Sends values to a mailbox's handler on the UI thread; clone it for each
/// thread that sends.
pub struct Sender<T> {
    tx: mpsc::Sender<T>,
    bell: Arc<Bell>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            bell: Arc::clone(&self.bell),
        }
    }
}

impl<T: Send> Sender<T> {
    /// Hand `value` to the UI thread and wake the window; `false` once the
    /// mailbox is gone (its scope ended, or its thread).
    ///
    /// Sent from the UI thread while a fork is open there (a check,
    /// ADR-0028), it reaches nothing and wakes nothing: what a fork sends
    /// is its own and goes with it, whichever sender it used (one made
    /// before the fork included), and it counts as sent. Another thread is
    /// the outside world, and sends at once.
    pub fn send(&self, value: T) -> bool {
        if crate::engine::forking() {
            drop(value);
            return true;
        }
        let sent = self.tx.send(value).is_ok();
        if sent {
            self.bell.ring();
        }
        sent
    }
}

/// What a drain takes: all that arrived, or the one value its wake was rung
/// for, waited for this long.
#[derive(Clone, Copy)]
enum Take {
    All,
    One(std::time::Duration),
}

/// A mailbox's drain: whether anything arrived, or `None` once it closed.
/// Each value's handler runs under the delivery's [`Gather`]: one that
/// panics leaves the next value to be delivered.
type Drain = Box<dyn FnMut(Take, &mut Gather) -> Option<bool>>;

use crate::engine::Gather;

crate::engine::engine_local! {
    /// Each mailbox's drain, by address and id: runs its handler on what
    /// arrived, says whether anything did. One closes as its scope ends,
    /// and is let go of then ([`retire`]).
    [Fresh] static BOXES: RefCell<Vec<(Address, u64, Drain)>> = const { RefCell::new(Vec::new()) };
    [Fresh] static BELLS: RefCell<Vec<(u64, Arc<Bell>)>> = const { RefCell::new(Vec::new()) };
    /// The window each mailbox was made in, by id: retired as it closes
    /// ([`close_window`]), with its handler.
    [Fresh] static MADE_IN: RefCell<Vec<(u64, crate::engine::Stored<()>)>> = const { RefCell::new(Vec::new()) };
    /// The running window's wake, for the mailboxes made after it started.
    [Fork] static WAKE: RefCell<Option<Wake>> = const { RefCell::new(None) };
    /// The mailboxes open, by id: one closes as its scope ends. A fork
    /// closes none (it runs no cleanup) and what it opens goes with it.
    [Fork(log)] static OPEN: RefCell<crate::engine::ForkMap<u64, ()>> = RefCell::default();
    /// How many mailboxes each site made ([`Address`]); what a fork makes
    /// is numbered again live.
    [Fork(log)] static MADE: RefCell<crate::engine::ForkMap<u64, u64>> = RefCell::default();
}

/// The mailboxes as the live registry holds them, with no id: what V2
/// compares between two engines (review r18, decision 6).
#[cfg(test)]
pub(crate) fn registry_summary() -> String {
    let mut made: Vec<(u64, u64)> = MADE.with_borrow(|m| m.iter().map(|(site, n)| (*site, *n)).collect());
    made.sort_unstable();
    format!(
        "open {} made {made:?} boxes {} bells {} made_in {}",
        OPEN.with_borrow(crate::engine::ForkMap::len),
        BOXES.with(|b| b.borrow().len()),
        BELLS.with(|b| b.borrow().len()),
        MADE_IN.with(|m| m.borrow().len()),
    )
}

/// A site's hash ([`Address`]): FNV-1a over its file, line and column,
/// the same in every build of the same source.
fn site_of(at: &std::panic::Location<'_>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let line = at.line().to_le_bytes();
    let column = at.column().to_le_bytes();
    for b in at.file().bytes().chain(line).chain(column) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The mailboxes made, numbered.
static NEXT: crate::engine::IdKind = crate::engine::IdKind::new("mailbox");

/// Whether mailbox `id` is open.
fn is_open(id: u64) -> bool {
    OPEN.try_with(|o| o.borrow().contains_key(&id)).unwrap_or(false)
}

/// Close mailbox `id` and let go of it at once (Codex r13 5): its drain
/// (its handler and all it captured) and its bell, not at a delivery to
/// its address that may never come. One being delivered is not in
/// [`BOXES`] then: its delivery finds it closed and drops it. What is let
/// go of is dropped after the lists are let go of, so a drop that makes
/// or closes a mailbox finds them free.
fn retire(id: u64) {
    OPEN.try_with(|o| o.borrow_mut().remove(&id)).ok();
    let drain = BOXES
        .try_with(|b| {
            let mut b = b.borrow_mut();
            let i = b.iter().position(|(_, of, _)| *of == id)?;
            // In the order made, as a delivery runs them.
            Some(b.remove(i))
        })
        .ok()
        .flatten();
    let bell = BELLS
        .try_with(|b| {
            let mut b = b.borrow_mut();
            let i = b.iter().position(|(of, _)| *of == id)?;
            Some(b.remove(i))
        })
        .ok()
        .flatten();
    MADE_IN
        .try_with(|m| {
            if let Ok(mut m) = m.try_borrow_mut() {
                m.retain(|(of, _)| *of != id);
            }
        })
        .ok();
    drop((drain, bell));
}

/// `window` closes: the mailboxes made in it are retired with their
/// handlers (ADR-0015, the close order), in its close turn: what arrives
/// after is dropped. How many were.
pub(crate) fn close_window(window: &crate::focus::FocusRoot) -> usize {
    let ids: Vec<u64> = MADE_IN
        .try_with(|m| {
            m.borrow()
                .iter()
                .filter(|(_, made)| made.is_of(window))
                .map(|(id, _)| *id)
                .collect()
        })
        .unwrap_or_default();
    let n = ids.len();
    for id in ids {
        crate::engine::contained(|| retire(id));
    }
    n
}

/// A mailbox on this (the UI) thread whose `handler` takes each value sent
/// through the returned [`Sender`], in the order sent.
///
/// Made while a scope (a component's, an effect's) runs, it closes when that
/// scope ends: what arrives after is dropped, and its handler, which may
/// write the scope's signals, never runs again.
///
/// The handler runs in the window current where the mailbox is made
/// (`engine::Stored`), whichever window's turn delivers it, as its scope
/// is the one it was made in; the window owns it: as it closes, the
/// mailbox closes, whatever still holds a sender.
///
/// # Panics
/// In a debug build, with neither a scope running nor a window's code (a
/// test's top level): nothing would close it but its senders, which any
/// thread can keep (ADR-0034, S4; review r17, decision 5).
#[track_caller]
pub fn mailbox<T: Send + 'static>(handler: impl FnMut(T) + 'static) -> Sender<T> {
    // Its owner first, before anything is made: the window it is made in
    // keeps it in `MADE_IN` ([`close_window`]); a scope retires it too.
    let owner = crate::engine::owner("gui::mailbox");
    let handler = crate::engine::Stored::new(crate::engine::Site::Mail, RefCell::new(handler));
    let here = crate::engine::Stored::new(crate::engine::Site::Mail, ());
    let id = crate::engine::unique_id(&NEXT);
    let site = site_of(std::panic::Location::caller());
    let nth = MADE.with_borrow_mut(|m| {
        let n = m.entry_mut(site);
        let nth = *n;
        *n = nth.saturating_add(1);
        nth
    });
    let at = Address { site, nth };
    OPEN.with_borrow_mut(|o| o.insert(id, ()));
    MADE_IN.with(|m| m.borrow_mut().push((id, here)));
    match owner {
        crate::engine::Owner::Scope => crate::reactive::on_release(move || retire(id)),
        crate::engine::Owner::Window | crate::engine::Owner::Nothing => {}
    }
    let (tx, rx) = mpsc::channel();
    let bell = Arc::new(Bell {
        at,
        wake: Mutex::new((None, 0)),
    });
    // One a fork makes goes with it and wakes nothing: its address is
    // numbered again live.
    if !crate::engine::forking()
        && let Some(wake) = WAKE.with(|w| w.borrow().clone())
    {
        bell.set(wake);
    }
    BELLS.with(|b| b.borrow_mut().push((id, Arc::clone(&bell))));
    BOXES.with(|b| {
        b.borrow_mut().push((
            at,
            id,
            Box::new(move |take, all: &mut Gather| {
                let mut any = false;
                // Each value under a catch of its own: one whose handler
                // panics leaves the next to be delivered (its mailbox is
                // asked whether it is open again, as the handler may close
                // it).
                loop {
                    if !is_open(id) {
                        return if any { Some(true) } else { None };
                    }
                    let value = match take {
                        Take::All => rx.try_recv().map_err(|e| e == mpsc::TryRecvError::Disconnected),
                        Take::One(_) if any => Err(false),
                        Take::One(wait) => rx
                            .recv_timeout(wait)
                            .map_err(|e| e == mpsc::RecvTimeoutError::Disconnected),
                    };
                    let value = match value {
                        Ok(value) => value,
                        // Every sender gone and nothing left: nothing can
                        // come again, so it is let go of (a mailbox no
                        // scope owns, whose senders all went).
                        Err(true) => {
                            OPEN.try_with(|o| o.borrow_mut().remove(&id)).ok();
                            return if any { Some(true) } else { None };
                        }
                        Err(false) => break,
                    };
                    any = true;
                    all.run(|| {
                        handler.call(|h| {
                            if let Ok(mut h) = h.try_borrow_mut() {
                                h(value);
                            }
                        })
                    });
                }
                Some(any)
            }),
        ));
    });
    Sender { tx, bell }
}

/// Run every handler on what arrived; whether anything did. A handler that
/// makes a mailbox adds it for the next call.
///
/// One delivery is one batch, however many mailboxes' handlers it runs
/// (each under its own catch): what they write runs each effect once, as
/// the batch ends.
pub(crate) fn deliver() -> bool {
    run(None)
}

/// Run the handler of the mailbox at `at` on the one value its wake was
/// rung for (waiting up to `wait` for it: a replay's sender may not have
/// sent it yet), as one delivery; whether one came. The window delivers
/// so, where its one queue hands the wake on in order with the input, and
/// a replay where the log took it (ADR-0033).
pub(crate) fn deliver_one(at: Address, wait: std::time::Duration) -> bool {
    run(Some((at, wait)))
}

/// One delivery: every mailbox's handler on all that arrived, or one
/// mailbox's on one value.
fn run(one: Option<(Address, std::time::Duration)>) -> bool {
    // What other threads sent waits in its channel for the next live turn
    // (rule 5 of ADR-0028): a fork runs no handler.
    if crate::engine::forking() {
        return false;
    }
    /// The mailboxes taken out, put back however `deliver` ends: an
    /// unwind from anywhere in it loses no receiver.
    struct Taken(Vec<(Address, u64, Drain)>);
    impl Drop for Taken {
        fn drop(&mut self) {
            let mut boxes = std::mem::take(&mut self.0);
            BOXES
                .try_with(|b| {
                    let mut b = b.borrow_mut();
                    boxes.append(&mut b);
                    *b = boxes;
                })
                .ok();
        }
    }
    let mut taken = Taken(BOXES.with(|b| std::mem::take(&mut *b.borrow_mut())));
    // Every mailbox drains, whatever the others held; a closed one is dropped.
    // One batch for the delivery (Opus nit 7, F1 review 8): an effect what
    // several handlers write wakes runs once, on all of it, never on a
    // state between two handlers. Each value's handler under a catch of its
    // own, and the effects the batch runs as it ends under one more: a
    // handler or effect that panics loses no other value or mailbox. Its
    // panic goes to the turn's arbiter as it is caught (ADR-0015, the turn
    // rule; outside a turn, the first goes on once all are drained).
    let mut any = false;
    let mut each = Gather::new();
    let mut end = Gather::new();
    end.run(|| {
        crate::reactive::batch(|| {
            taken.0.retain_mut(|(at, id, drain)| {
                let take = match one {
                    None => Take::All,
                    Some((only, wait)) if only == *at => Take::One(wait),
                    Some(_) => return true,
                };
                match drain(take, &mut each) {
                    Some(got) => {
                        any |= got;
                        true
                    }
                    // Closed: its bell goes with it.
                    None => {
                        let id = *id;
                        BELLS.try_with(|b| b.borrow_mut().retain(|(of, _)| *of != id)).ok();
                        false
                    }
                }
            });
        });
    });
    drop(taken);
    // The handlers' panics came before the batch's end's.
    each.finish();
    end.finish();
    any
}

/// Wake the window with `wake` for each value that arrives, from now on,
/// and for each sent before (rung for once it is set).
pub(crate) fn wake_with(wake: Wake) {
    BELLS.with(|b| b.borrow().iter().for_each(|(_, bell)| bell.set(Arc::clone(&wake))));
    WAKE.with(|w| *w.borrow_mut() = Some(wake));
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// [`super::mailbox`], made at a test's top level in a scope of the
    /// test's own ([`crate::engine::owned`]); in a scope or a window, by
    /// that. Its site is this one's, the same for every call.
    fn mailbox<T: Send + 'static>(handler: impl FnMut(T) + 'static) -> Sender<T> {
        crate::engine::owned(|| super::mailbox(handler))
    }

    /// A mailbox at a test's top level, with no scope or window to close
    /// it, is refused (review r17, decision 5).
    #[cfg(debug_assertions)]
    #[test]
    fn an_ownerless_mailbox_is_refused() {
        let made = std::panic::catch_unwind(|| drop(super::mailbox(|_: u32| {})));
        let said = made.expect_err("refused");
        let said = said.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(said.contains("no scope running"), "{said}");
    }

    #[test]
    fn what_another_thread_sends_reaches_the_ui_thread_in_order_and_wakes_the_window() {
        let got = Rc::new(RefCell::new(Vec::new()));
        let into = Rc::clone(&got);
        let sender = mailbox(move |n: u32| into.borrow_mut().push(n));
        let rings = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&rings);
        wake_with(Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        std::thread::spawn(move || (1..=3).all(|n| sender.send(n)))
            .join()
            .unwrap();
        assert!(got.borrow().is_empty(), "nothing runs off the UI thread");
        assert!(deliver());
        assert_eq!(*got.borrow(), [1, 2, 3]);
        assert_eq!(rings.load(Ordering::SeqCst), 3);
        assert!(!deliver(), "nothing new arrived");
    }

    /// Codex r12 7: each value rings its mailbox's address once, those sent
    /// before the window's wake once it is set, and a delivery by address
    /// runs that mailbox's handler on one value, leaving the rest and every
    /// other mailbox for their own rings. A site numbers its mailboxes.
    #[test]
    fn each_value_rings_its_address_once_and_is_delivered_by_it() {
        let got = Rc::new(RefCell::new(Vec::new()));
        let make = |tag: &'static str| {
            let into = Rc::clone(&got);
            mailbox(move |n: u32| into.borrow_mut().push((tag, n)))
        };
        let (a, b) = (make("a"), make("b"));
        assert_eq!(a.bell.at.site, b.bell.at.site, "one site");
        assert_eq!(b.bell.at.nth, a.bell.at.nth + 1, "numbered in order");
        assert!(a.send(1) && a.send(2) && b.send(3));
        let rung = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&rung);
        wake_with(Arc::new(move |at| into.lock().unwrap().push(at)));
        assert_eq!(
            *rung.lock().unwrap(),
            [a.bell.at, a.bell.at, b.bell.at],
            "what came before the wake rung once it is set"
        );
        assert!(deliver_one(b.bell.at, std::time::Duration::ZERO));
        assert!(deliver_one(a.bell.at, std::time::Duration::ZERO));
        assert_eq!(
            *got.borrow(),
            [("b", 3), ("a", 1)],
            "one value, the addressed mailbox's"
        );
        assert!(deliver_one(a.bell.at, std::time::Duration::ZERO));
        assert!(!deliver_one(a.bell.at, std::time::Duration::ZERO), "nothing left");
        assert_eq!(*got.borrow(), [("b", 3), ("a", 1), ("a", 2)]);
    }

    /// F1 review 6 (batch runners): a handler that panics loses no other
    /// mailbox; the panic is raised once every mailbox drained, and both
    /// stay open for what comes next.
    #[test]
    fn a_panicking_handler_loses_no_other_mailbox() {
        let got = Rc::new(RefCell::new(Vec::new()));
        let into = Rc::clone(&got);
        let fails = mailbox(|_: u32| panic!("a handler fails"));
        let works = mailbox(move |n: u32| into.borrow_mut().push(n));
        assert!(fails.send(1) && works.send(2));
        let delivered = std::panic::catch_unwind(deliver);
        assert!(delivered.is_err(), "the panic is raised");
        assert_eq!(*got.borrow(), [2], "the other mailbox drained");
        assert!(works.send(3));
        drop(fails);
        assert!(deliver());
        assert_eq!(*got.borrow(), [2, 3], "and stays open");
    }

    /// Codex 5 (F1 review 7): an effect a handler's write runs, panicking
    /// as the delivery's batch ends, loses no mailbox: each handler runs
    /// under a catch, and the batch's end under one more, and the boxes are
    /// put back by a guard however `deliver` ends.
    #[test]
    fn an_effect_panicking_as_a_handlers_batch_ends_loses_no_mailbox() {
        let s = crate::Signal::new(0u32);
        crate::Effect::new(move || assert!(s.get() != 1, "an effect fails"));
        let got = Rc::new(RefCell::new(Vec::new()));
        let into = Rc::clone(&got);
        let writes = mailbox(move |n: u32| s.set(n));
        let works = mailbox(move |n: u32| into.borrow_mut().push(n));
        assert!(writes.send(1) && works.send(2));
        let delivered = std::panic::catch_unwind(deliver);
        assert!(delivered.is_err(), "the effect's panic is raised");
        assert_eq!(*got.borrow(), [2], "the other mailbox drained");
        assert!(works.send(3) && writes.send(4));
        assert!(deliver(), "both still open");
        assert_eq!(*got.borrow(), [2, 3]);
        assert_eq!(s.get(), 4, "the writing one too");
    }

    /// Opus nit 7 (F1 review 8): one delivery is one batch, however many
    /// mailboxes it drains: an effect reading what two handlers write runs
    /// once for both, not once per handler.
    #[test]
    fn an_effect_two_handlers_wake_runs_once_per_delivery() {
        let (a, b) = (crate::Signal::new(0u32), crate::Signal::new(0u32));
        let runs = Rc::new(RefCell::new(Vec::new()));
        let r = Rc::clone(&runs);
        let e = crate::Effect::new(move || r.borrow_mut().push((a.get(), b.get())));
        let first = mailbox(move |n: u32| a.set(n));
        let second = mailbox(move |n: u32| b.set(n));
        assert!(first.send(1) && second.send(2));
        assert!(deliver());
        assert_eq!(*runs.borrow(), [(0, 0), (1, 2)], "one run for the delivery");
        e.dispose();
    }

    /// Async stays outside the graph: a status signal, fed through a mailbox.
    #[test]
    fn a_status_fed_from_another_thread_runs_an_effect_once_per_delivery() {
        #[derive(Clone, Debug, PartialEq)]
        enum Status {
            Loading,
            Ready(u32),
        }
        let status = crate::Signal::new(Status::Loading);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s = Rc::clone(&seen);
        crate::Effect::new(move || s.borrow_mut().push(status.get()));
        let sender = mailbox(move |n: u32| status.set(Status::Ready(n)));
        std::thread::spawn(move || (1..=3).all(|n| sender.send(n)))
            .join()
            .unwrap();
        assert!(deliver());
        assert_eq!(
            *seen.borrow(),
            [Status::Loading, Status::Ready(3)],
            "one batch, one run"
        );
    }

    #[test]
    fn a_mailbox_made_in_a_scope_closes_with_it() {
        let scope = crate::Scope::new();
        let (sender, got) = scope.run(|| {
            let got = crate::Signal::new(0_u32);
            (mailbox(move |n: u32| got.set(n)), got)
        });
        assert!(sender.send(1));
        assert!(deliver());
        assert_eq!(got.get(), 1);
        scope.dispose();
        assert!(!sender.send(2), "gone with its scope: nothing to send to");
        assert!(!deliver(), "closed: its handler would write a disposed signal");
    }

    /// Codex r13 5: mailboxes made and closed with their scopes, again and
    /// again, keep nothing: each is let go of as its scope ends, its drain
    /// and its bell, with no delivery to its address.
    #[test]
    fn mailboxes_made_and_closed_again_and_again_keep_nothing() {
        let boxes = || BOXES.with(|b| b.borrow().len());
        let bells = || BELLS.with(|b| b.borrow().len());
        let (held, rung) = (boxes(), bells());
        for _ in 0..1_000 {
            let scope = crate::Scope::new();
            let sender = scope.run(|| mailbox(|_: u32| {}));
            scope.dispose();
            drop(sender);
        }
        assert_eq!((boxes(), bells()), (held, rung), "none kept");
        // One no scope owns goes once its senders all went and nothing is
        // left for it.
        let sender = mailbox(|_: u32| {});
        drop(sender);
        deliver();
        assert_eq!((boxes(), bells()), (held, rung), "nor one whose senders went");
    }

    /// A value whose drop panics: what makes a check's teardown fault.
    #[cfg(debug_assertions)]
    struct Boom;

    #[cfg(debug_assertions)]
    impl Drop for Boom {
        fn drop(&mut self) {
            panic!("a boom's drop");
        }
    }

    crate::engine::engine_local! {
        /// What a check parks, opaque: a boom first, then a sender.
        #[cfg(debug_assertions)]
        [Fresh] static PARKED: RefCell<Vec<Box<dyn std::any::Any>>> = RefCell::new(Vec::new());
    }

    /// ADR-0015 (a window owns what is made in it), F1's handoff: a
    /// mailbox made with no scope, in a window, whose sender a faulted
    /// check forgot (parked in an opaque value a runaway set aside) keeps no
    /// drain or handler once its window closes: what is sent after is
    /// handled by nothing.
    #[cfg(debug_assertions)]
    #[test]
    fn a_scope_less_mailbox_a_faulted_check_pinned_ends_with_its_window() {
        let boxes = || BOXES.with(|b| b.borrow().len());
        let mut window = crate::app::Pipeline::new(|| crate::element::Element::Empty, false);
        let held = boxes();
        let got = Rc::new(std::cell::Cell::new(0_u32));
        let into = Rc::clone(&got);
        let mut made = None;
        window.act("make", || made = Some(mailbox(move |n: u32| into.set(into.get() + n))));
        let sender = made.expect("made");
        assert_eq!(boxes(), held + 1);
        let parked = sender.clone();
        let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::engine::in_cold_frame(|| {
                PARKED.with_borrow_mut(|v| {
                    v.push(Box::new(Boom));
                    v.push(Box::new(parked.clone()));
                });
            });
        }));
        assert!(p.is_err(), "the check faults at the boom");
        drop(parked);
        assert!(sender.send(1));
        assert!(deliver(), "open while its window is");
        assert_eq!(got.get(), 1);
        assert_eq!(window.close(), Ok(()));
        assert_eq!(boxes(), held, "its drain went with its window");
        assert!(!sender.send(2), "closed with its window: nothing to send to");
        assert!(!deliver());
        assert_eq!(got.get(), 1, "its handler never runs again");
    }

    /// A mailbox only a cold frame made goes with it: nothing drains it
    /// after, however often the check runs, and what is sent to it is
    /// never handled.
    #[cfg(debug_assertions)]
    #[test]
    fn a_mailbox_a_cold_frame_made_leaves_no_drain() {
        let live = mailbox(|_: u32| {});
        let boxes = || BOXES.with(|b| b.borrow().len());
        let bells = || BELLS.with(|b| b.borrow().len());
        let (held, rung) = (boxes(), bells());
        let got = Rc::new(std::cell::Cell::new(0_u32));
        let mut senders = Vec::new();
        for _ in 0..3 {
            let _cold = crate::engine::ColdFrame::begin();
            let into = Rc::clone(&got);
            senders.push(mailbox(move |n: u32| into.set(into.get() + n)));
        }
        assert_eq!((boxes(), bells()), (held, rung), "no drain kept, none piled up");
        assert!(
            senders.iter().all(|s| !s.send(1)),
            "sent to nothing: the cold mailboxes are gone"
        );
        assert!(live.send(1));
        assert!(deliver(), "the live mailbox delivers");
        assert_eq!(got.get(), 0, "the cold mailboxes' handlers never run");
    }

    /// A sender made before a fork and used in it (ADR-0028, the capture
    /// test): what the fork sends goes with it, delivered to nothing and
    /// waking nothing, and a delivery the fork asks for runs no handler.
    #[cfg(debug_assertions)]
    #[test]
    fn what_a_fork_sends_through_a_sender_it_captured_goes_with_it() {
        let got = Rc::new(RefCell::new(Vec::new()));
        let into = Rc::clone(&got);
        let sender = mailbox(move |n: u32| into.borrow_mut().push(n));
        let rings = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&rings);
        wake_with(Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        // Sent live, waiting for the next delivery.
        assert!(sender.send(1));
        let captured = sender.clone();
        {
            let _cold = crate::engine::ColdFrame::begin();
            assert!(captured.send(2), "counts as sent");
            assert!(!deliver(), "a fork delivers nothing");
        }
        assert_eq!(rings.load(Ordering::SeqCst), 1, "the fork's send woke nothing");
        assert!(got.borrow().is_empty(), "no handler ran in the fork");
        assert!(deliver());
        assert_eq!(*got.borrow(), [1], "what was sent live, not what the fork sent");
        assert!(sender.send(3));
        assert!(deliver());
        assert_eq!(*got.borrow(), [1, 3], "the mailbox works live after");
    }
}
