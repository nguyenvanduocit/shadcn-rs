//! Values other threads hand the UI thread: a [`mailbox`] runs its handler
//! on the UI thread with each value sent to it, before the next frame, and
//! the window wakes for it. A handler may write signals, as a click does; a
//! network thread that receives what a screen shows sends it here.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};

/// Wakes the window's event loop, from any thread.
pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

/// How one mailbox wakes the window: set once the window runs.
#[derive(Default)]
struct Bell(Mutex<Option<Wake>>);

impl Bell {
    fn set(&self, wake: Wake) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(wake);
    }

    fn ring(&self) {
        let wake = self.0.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Some(wake) = wake {
            wake();
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
    /// mailbox is gone (its thread ended).
    pub fn send(&self, value: T) -> bool {
        let sent = self.tx.send(value).is_ok();
        if sent {
            self.bell.ring();
        }
        sent
    }
}

/// A mailbox's drain: whether anything arrived, or `None` once it closed.
type Drain = Box<dyn FnMut() -> Option<bool>>;

crate::engine::engine_local! {
    /// Each mailbox's drain: runs its handler on what arrived, says whether anything did.
    [Fresh] static BOXES: RefCell<Vec<Drain>> = const { RefCell::new(Vec::new()) };
    [Fresh] static BELLS: RefCell<Vec<Arc<Bell>>> = const { RefCell::new(Vec::new()) };
    /// The running window's wake, for the mailboxes made after it started.
    [Fork] static WAKE: RefCell<Option<Wake>> = const { RefCell::new(None) };
}

/// A mailbox on this (the UI) thread whose `handler` takes each value sent
/// through the returned [`Sender`], in the order sent.
///
/// Made while a scope (a component's, an effect's) runs, it closes when that
/// scope ends: what arrives after is dropped, and its handler, which may
/// write the scope's signals, never runs again.
pub fn mailbox<T: Send + 'static>(mut handler: impl FnMut(T) + 'static) -> Sender<T> {
    let open = Rc::new(Cell::new(true));
    if crate::reactive::has_owner() {
        let closing = Rc::clone(&open);
        crate::reactive::on_cleanup(move || closing.set(false));
    }
    let (tx, rx) = mpsc::channel();
    let bell = Arc::new(Bell::default());
    if let Some(wake) = WAKE.with(|w| w.borrow().clone()) {
        bell.set(wake);
    }
    BELLS.with(|b| b.borrow_mut().push(Arc::clone(&bell)));
    BOXES.with(|b| {
        b.borrow_mut().push(Box::new(move || {
            if !open.get() {
                return None;
            }
            let mut any = false;
            while let Ok(value) = rx.try_recv() {
                handler(value);
                any = true;
            }
            Some(any)
        }));
    });
    Sender { tx, bell }
}

/// Run every handler on what arrived; whether anything did. A handler that
/// makes a mailbox adds it for the next call.
///
/// All of it is one batch: what the handlers write runs each effect once.
pub(crate) fn deliver() -> bool {
    let mut boxes = BOXES.with(|b| std::mem::take(&mut *b.borrow_mut()));
    // Every mailbox drains, whatever the others held; a closed one is dropped.
    let mut any = false;
    crate::reactive::batch(|| {
        boxes.retain_mut(|drain| match drain() {
            Some(got) => {
                any |= got;
                true
            }
            None => false,
        });
    });
    BOXES.with(|b| {
        let mut b = b.borrow_mut();
        boxes.append(&mut b);
        *b = boxes;
    });
    any
}

/// Wake the window with `wake` whenever a value arrives, from now on.
pub(crate) fn wake_with(wake: Wake) {
    BELLS.with(|b| b.borrow().iter().for_each(|bell| bell.set(Arc::clone(&wake))));
    WAKE.with(|w| *w.borrow_mut() = Some(wake));
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn what_another_thread_sends_reaches_the_ui_thread_in_order_and_wakes_the_window() {
        let got = Rc::new(RefCell::new(Vec::new()));
        let into = Rc::clone(&got);
        let sender = mailbox(move |n: u32| into.borrow_mut().push(n));
        let rings = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&rings);
        wake_with(Arc::new(move || {
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
        assert!(sender.send(2));
        assert!(!deliver(), "closed: its handler would write a disposed signal");
    }

    /// A mailbox only a cold frame made goes with it: nothing drains it
    /// after, however often the check runs, and what is sent to it is
    /// never handled.
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
}
