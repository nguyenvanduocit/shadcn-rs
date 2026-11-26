//! Review r14's reproducers, kept as regression tests of the dispatch
//! rule (ADR-0015): the routing primitive owns the transaction, whatever
//! path input takes (platform, scenario, devtools); every call of user
//! code inside it is caught at the one gate; one arbiter keeps the first
//! panic in time; a window's ambient state is put back after a nested
//! dispatch to another; a commit publishes all its signal writes at once.

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use gui::app::Scenario;
use gui::events::{Modifiers, NamedKey};
use gui::prelude::*;
use gui::window::Input;
use gui::{Effect, FocusHandle, Interaction, Layer, Positioning, deferred, div};

/// The `&str` a panic carried.
fn said(p: &Box<dyn std::any::Any + Send>) -> Option<&'static str> {
    p.downcast_ref::<&'static str>().copied()
}

/// Review r14, Codex 1 = Opus I2: the devtools' hover goes through the
/// same transaction as platform input: the pointer move is still told
/// after the enter handler panics.
#[test]
fn a_devtools_hover_tells_the_move_after_a_panicking_enter() {
    let moved = Rc::new(Cell::new(false));
    let out = Rc::clone(&moved);
    let s = Scenario::measuring(
        move || {
            let out = Rc::clone(&out);
            div()
                .size(100.)
                .on_hover(|on| {
                    if on {
                        panic!("enter")
                    }
                })
                .on_pointer_move(move |_| out.set(true))
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    let mut session = gui::devtools::Session::new(s).expect("session");
    let r = catch_unwind(AssertUnwindSafe(|| session.run("hover 5,5")));
    assert_eq!(r.as_ref().err().and_then(said), Some("enter"));
    assert!(moved.get(), "devtools skipped pointer-move");
}

/// Review r14, Codex 2 = Opus I1: a layer's dismiss answer that panics
/// counts as no answer; the press outside it goes on to its target.
#[test]
fn a_panicking_dismiss_answer_leaves_the_press_to_go_on() {
    let it = Interaction::new();
    let mut s = Scenario::measuring(
        move || {
            let layer = Layer::new().on_dismiss(|_| panic!("dismiss"));
            div()
                .child(div().size(10.).child(deferred(
                    Positioning::At { x: 70., y: 70. },
                    div().size(10.).layer(layer),
                )))
                .child(div().size(40.).interaction(it))
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    let r = catch_unwind(AssertUnwindSafe(|| s.send(Input::MouseDown { x: 5., y: 20. })));
    assert_eq!(r.as_ref().err().and_then(said), Some("dismiss"));
    assert!(it.pressed(), "outside target never became pressed");
}

/// Review r14, Codex 3: input sent to another window from inside a
/// handler leaves the outer window's focus root current when it returns.
#[test]
fn a_nested_input_to_another_window_leaves_this_windows_focus() {
    let mut other = Scenario::measuring(|| div().into_element(), 100, 100, 1.).expect("scenario");
    other.settle().expect("frame");
    let other = Rc::new(RefCell::new(other));
    let h = FocusHandle::new();
    let kept = Rc::new(Cell::new(None));
    let seen = Rc::clone(&kept);
    let mut s = Scenario::measuring(
        move || {
            let other = Rc::clone(&other);
            let seen = Rc::clone(&seen);
            div()
                .track_focus(h)
                .size(100.)
                .on_mouse_down(move |_| {
                    let before = h.is_focused();
                    other.borrow_mut().send(Input::Modifiers(Modifiers::default()));
                    seen.set(Some((before, h.is_focused())));
                })
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    s.send(Input::MouseDown { x: 5., y: 5. });
    assert_eq!(kept.get(), Some((true, true)), "outer handler lost its window");
}

/// Review r14, Codex 4: Tab publishes focus and its ring together: an
/// effect reading both sees one change, never the pair half written.
#[test]
fn tab_publishes_focus_and_its_ring_at_once() {
    let h = FocusHandle::new();
    let mut s =
        Scenario::measuring(move || div().track_focus(h).size(100.).into_element(), 100, 100, 1.).expect("scenario");
    s.settle().expect("frame");
    let pairs = Rc::new(RefCell::new(Vec::new()));
    let out = Rc::clone(&pairs);
    // Made in the window, so it reads that window's focus (ADR-0015, the
    // thread-window rule: at the top level it would read the thread's).
    let _e = s.read(|| {
        Effect::new(move || {
            out.borrow_mut().push((h.is_focused(), gui::focus::focus_visible()));
        })
    });
    pairs.borrow_mut().clear();
    s.press(NamedKey::Tab, Modifiers::default());
    assert_eq!(&*pairs.borrow(), &[(true, true)]);
}

/// Review r14, Codex 5: the first panic in time goes on, though a later
/// one (a nested dispatch's, run by a later effect of the same flush)
/// reached the dispatch first.
#[test]
fn the_first_panic_in_time_goes_on() {
    let mut s = Scenario::measuring(
        || div().size(50.).on_mouse_down(|_| panic!("second")).into_element(),
        100,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    let window = Rc::new(RefCell::new(s));
    let weak = Rc::downgrade(&window);
    let start = gui::Signal::new(false);
    let next = gui::Signal::new(false);
    let _a = Effect::new(move || {
        if start.get() {
            next.set(true);
            panic!("first");
        }
    });
    let _b = Effect::new(move || {
        if next.get()
            && let Some(w) = weak.upgrade()
        {
            w.borrow_mut().send(Input::MouseDown { x: 5., y: 5. });
        }
    });
    // A turn of another window: the write that starts both effects.
    let mut other = Scenario::measuring(|| div().into_element(), 100, 100, 1.).expect("scenario");
    let r = catch_unwind(AssertUnwindSafe(|| other.act("start", || start.set(true))));
    assert_eq!(r.as_ref().err().and_then(said), Some("first"));
}

/// Review r14 E: a handler that panics inside its own batch leaves the
/// effects its writes woke queued as the batch unwinds; the dispatch runs
/// them before it ends, as the web's microtask checkpoint runs after a
/// listener that threw.
#[test]
fn effects_a_panicking_batch_left_run_within_the_dispatch() {
    let n = gui::Signal::new(0_u32);
    let seen = Rc::new(Cell::new(0_u32));
    let out = Rc::clone(&seen);
    let _e = Effect::new(move || out.set(n.get()));
    let mut s = Scenario::measuring(
        move || {
            div()
                .size(50.)
                .on_mouse_down(move |_| {
                    gui::batch(|| {
                        n.set(7);
                        panic!("in its batch")
                    })
                })
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    let r = catch_unwind(AssertUnwindSafe(|| s.send(Input::MouseDown { x: 5., y: 5. })));
    assert_eq!(r.as_ref().err().and_then(said), Some("in its batch"));
    assert_eq!(seen.get(), 7, "the effect its write woke ran");
}

/// Review r14 C: two windows, input sent to the second from a handler of
/// the first. Each keeps its own pointer and focus, the first's handler
/// finds its window's own after the nested dispatch, and the nested one
/// shares the outer's arbiter: the inner window's panic, first in time,
/// is kept (the send returns), and the outer handler's later one dropped.
#[test]
fn a_nested_dispatch_keeps_each_windows_own_and_the_first_panic() {
    let mut other = Scenario::measuring(
        || div().size(50.).on_mouse_down(|_| panic!("inner")).into_element(),
        100,
        100,
        1.,
    )
    .expect("scenario");
    other.settle().expect("frame");
    let other = Rc::new(RefCell::new(other));
    let seen = Rc::new(Cell::new(None));
    let out = Rc::clone(&seen);
    let h = FocusHandle::new();
    let mut s = Scenario::measuring(
        move || {
            let (other, out) = (Rc::clone(&other), Rc::clone(&out));
            div()
                .track_focus(h)
                .size(100.)
                .on_mouse_down(move |_| {
                    let sent = catch_unwind(AssertUnwindSafe(|| {
                        let mut o = other.borrow_mut();
                        o.send(Input::PointerMoved { x: 30., y: 30. });
                        o.send(Input::MouseDown { x: 5., y: 5. });
                    }));
                    out.set(Some((sent.is_ok(), gui::interact::pointer_position(), h.is_focused())));
                    panic!("outer")
                })
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    s.hover(10., 10.);
    let r = catch_unwind(AssertUnwindSafe(|| s.send(Input::MouseDown { x: 10., y: 10. })));
    assert_eq!(
        seen.get(),
        Some((true, Some((10., 10.)), true)),
        "the nested send returned; the outer window's pointer and focus after it"
    );
    assert_eq!(
        r.as_ref().err().and_then(said),
        Some("inner"),
        "the first panic in time"
    );
}
