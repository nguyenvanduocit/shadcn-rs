//! Review r15's reproducers, kept as regression tests of the turn rule
//! (ADR-0015): one turn owns every entry from outside (input with its
//! compositor hand-off, mail, timers, frames, the devtools, a window made,
//! an accessibility action); one arbiter, total over unwinds, keeps the
//! first panic in time and drops the others outside its borrow; work
//! scheduled in a window runs in that window; a render that panics never
//! drops the input routed after it.

#![expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind, panic_any};
use std::rc::Rc;
use std::time::Duration;

use gui::app::Scenario;
use gui::environment as env;
use gui::prelude::*;
use gui::{Effect, Signal, clock, div};

/// The `&str` a panic carried.
fn said(p: &Box<dyn std::any::Any + Send>) -> Option<&'static str> {
    p.downcast_ref::<&'static str>().copied()
}

fn window() -> Scenario {
    let mut s = Scenario::measuring(|| div().into_element(), 100, 100, 1.).expect("scenario");
    s.settle().expect("frame");
    s
}

/// Codex r15 1: the compositor's scroll handed on with a move, and the
/// move, are one turn: a dismiss answer that panics as the scroll is taken
/// in leaves the move to reach its target.
#[test]
fn a_pulled_scroll_whose_dismiss_panics_still_moves_the_pointer() {
    let calls = Rc::new(Cell::new(0));
    let out = Rc::clone(&calls);
    let h = gui::ScrollHandle::new();
    let mut s = Scenario::measuring(
        move || {
            let layer = gui::Layer::new()
                .dismiss(gui::DismissPolicy::NONE.scroll(true))
                .on_dismiss(|_| panic!("dismiss"));
            let out = Rc::clone(&out);
            div()
                .width(200.)
                .height(100.)
                .overflow_y(gui::Overflow::Scroll)
                .track_scroll(&h)
                .on_pointer_move(move |_| out.set(out.get() + 1))
                .child(div().size(10.).child(gui::deferred(
                    gui::Positioning::At { x: 5., y: 5. },
                    div().size(5.).layer(layer),
                )))
                .child(div().height(1000.))
                .into_element()
        },
        200,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    s.hover(150., 80.);
    assert!(s.wheel(40.));
    let before = calls.get();
    let r = catch_unwind(AssertUnwindSafe(|| s.hover(151., 80.)));
    assert_eq!(r.as_ref().err().and_then(said), Some("dismiss"));
    assert_eq!(calls.get(), before + 1, "the move was told");
}

/// Codex r15 2 = Opus r15 I1: mail is one turn whatever path delivers it:
/// a handler that panics leaves the next value to be delivered, and an
/// earlier panic of the turn delivering it (an effect's) stays first.
#[test]
fn mail_drains_past_a_panicking_handler_and_the_first_panic_stays_first() {
    let b = Rc::new(RefCell::new(window()));
    let seen = Rc::new(Cell::new(0));
    let out = Rc::clone(&seen);
    // B's mailbox, made in a turn of B (the top level owns none: ADR-0015,
    // the thread-window rule); B's frame delivers it.
    let tx = b.borrow_mut().read(|| {
        gui::mailbox(move |n: u32| {
            out.set(n);
            if n == 1 {
                panic!("mail")
            }
        })
    });
    let mut a = window();
    let go = Signal::new(false);
    let first = Effect::new(move || {
        if go.get() {
            panic!("first")
        }
    });
    let w = Rc::clone(&b);
    let second = Effect::new(move || {
        if go.get() {
            w.borrow_mut().settle().expect("frame");
        }
    });
    assert!(tx.send(1).is_ok() && tx.send(2).is_ok());
    let r = catch_unwind(AssertUnwindSafe(|| a.act("go", || go.set(true))));
    let got = (r.as_ref().err().and_then(said), seen.get());
    // The effects hold the second window: gone before the thread's end.
    first.dispose();
    second.dispose();
    drop(b);
    assert_eq!(
        got,
        (Some("first"), 2),
        "the first panic in time, every value delivered"
    );
}

/// Opus r15 I1: a scenario's frame delivers mail in a turn, as the
/// platform's does: a gated call inside a handler that panics answers no
/// answer, and the handler goes on.
#[test]
fn a_frame_delivers_mail_in_a_turn() {
    let mut s = window();
    let after = Rc::new(Cell::new(false));
    let out = Rc::clone(&after);
    let fails: gui::Callback<(), ()> = gui::Callback::new(|()| panic!("gated"));
    let tx = s.read(|| {
        gui::mailbox(move |(): ()| {
            fails.call(());
            out.set(true);
        })
    });
    assert!(tx.send(()).is_ok());
    let r = catch_unwind(AssertUnwindSafe(|| s.settle()));
    assert_eq!(r.as_ref().err().and_then(said), Some("gated"));
    assert!(after.get(), "the handler went on past the gated call");
}

/// Codex r15 3: a timer runs in the window that scheduled it, whichever
/// window advances the clock.
#[test]
fn a_timer_runs_in_the_window_that_scheduled_it() {
    let mut a = window();
    let mut b = window();
    let observed = Rc::new(Cell::new(None));
    let seen = Rc::clone(&observed);
    a.act("schedule", move || {
        env::set_window_visible(false);
        clock::after(Duration::ZERO, move || seen.set(Some(env::window_visible())));
    });
    b.at(Duration::from_millis(1));
    a.at(Duration::from_millis(1));
    assert_eq!(observed.get(), Some(false));
}

/// Codex r15 4: an effect runs in the window it was made in, whichever
/// window's write wakes it.
#[test]
fn an_effect_runs_in_the_window_it_was_made_in() {
    let mut a = window();
    let mut b = window();
    let trigger = Signal::new(0);
    let seen = Rc::new(Cell::new(None));
    let out = Rc::clone(&seen);
    let mut effect = None;
    a.act("observe", || {
        env::set_window_visible(false);
        effect = Some(Effect::new(move || {
            let _ = trigger.get();
            out.set(Some(env::window_visible()));
        }));
    });
    b.act("publish", || trigger.set(1));
    assert_eq!(seen.get(), Some(false));
    assert!(effect.is_some_and(|e| e.is_alive()));
}

/// Codex r15 5: making a window leaves the caller's window current.
#[test]
fn making_a_window_leaves_the_callers_current() {
    let mut a = window();
    let still = Rc::new(Cell::new(None));
    let out = Rc::clone(&still);
    a.act("make a window", move || {
        env::set_window_visible(false);
        let _b = window();
        out.set(Some(env::window_visible()));
    });
    assert_eq!(still.get(), Some(false), "the caller switched to the new window");
}

/// Codex r15 6: the arbiter drops a later panic's payload outside its
/// borrow: a payload whose drop reaches the gate neither aborts the turn
/// nor replaces the first panic.
#[test]
fn a_later_payload_that_reaches_the_gate_as_it_drops_is_dropped_quietly() {
    struct Payload;
    impl Drop for Payload {
        fn drop(&mut self) {
            gui::engine::call_out(gui::engine::Site::Callback, || ());
        }
    }
    let finished = Rc::new(Cell::new(false));
    let done = Rc::clone(&finished);
    let mut s = Scenario::measuring(
        move || {
            let done = Rc::clone(&done);
            div()
                .size(100.)
                .on_pointer_move(move |_| done.set(true))
                .child(
                    div()
                        .size(50.)
                        .on_hover(|on| {
                            if on {
                                panic!("first")
                            }
                        })
                        .on_pointer_move(|_| panic_any(Payload)),
                )
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    let r = catch_unwind(AssertUnwindSafe(|| s.hover(5., 5.)));
    assert_eq!(r.as_ref().err().and_then(said), Some("first"));
    assert!(finished.get(), "the move went on to the parent");
}

/// Opus r15 I2 (A): a layout hook's panic, then a render's in the same
/// frame: the hook's, first in time, goes on.
#[test]
fn a_hooks_panic_before_a_renders_stays_first() {
    let boom = Signal::new(false);
    let mut s = Scenario::measuring(
        move || {
            if boom.get() {
                panic!("render")
            }
            div()
                .size(10.)
                .on_measured(move |_| {
                    if !boom.get_untracked() {
                        boom.set(true);
                        panic!("hook")
                    }
                })
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    let r = catch_unwind(AssertUnwindSafe(|| s.settle()));
    assert_eq!(r.as_ref().err().and_then(said), Some("hook"));
}

/// Opus r15 I2 (ungated user code in layout): a binding that panics as
/// input lays the frame out goes to the arbiter, the binding holds what it
/// last gave (the coordinator's r15 ask: nothing blanks), and the input is
/// routed all the same.
#[test]
fn a_binding_panicking_as_input_lays_out_drops_no_input() {
    let boom = Signal::new(false);
    let armed = Rc::new(Cell::new(true));
    let a = Rc::clone(&armed);
    let mut s = Scenario::measuring(
        move || {
            let a = Rc::clone(&a);
            div()
                .height(100.)
                .width(move || {
                    if boom.get() && a.replace(false) {
                        panic!("bound")
                    }
                    100.
                })
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    s.hover(1., 1.);
    s.act("boom", || boom.set(true));
    let r = catch_unwind(AssertUnwindSafe(|| s.hover(7., 7.)));
    assert_eq!(r.as_ref().err().and_then(said), Some("bound"));
    let mut at = None;
    s.act("read", || at = gui::interact::pointer_position());
    assert_eq!(at, Some((7., 7.)), "the move was routed");
    let widths: Vec<f32> = s
        .last()
        .nodes
        .iter()
        .map(|n| n.bounds(gui::inspect::OUTSIDE).width)
        .collect();
    assert_eq!(widths, [100.], "the node as it was: the binding holds what it gave");
}

/// The coordinator's r15 ask, for what a layout runs: a bound text whose
/// closure panics keeps the text it gave, the rest of the frame drawn
/// anew; once what it read changes it runs again.
#[test]
fn a_bound_text_that_panics_keeps_what_it_gave_and_the_rest_draws() {
    let (n, m) = (Signal::new(0_i32), Signal::new(0_i32));
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(gui::element::text_bound(move || {
                    let v = n.get();
                    assert!(v != 1, "n is not 1");
                    format!("n {v}")
                }))
                .child(gui::element::text_bound(move || format!("m {}", m.get())))
                .into_element()
        },
        200,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    s.act("both", || {
        n.set(1);
        m.set(1);
    });
    let r = catch_unwind(AssertUnwindSafe(|| s.settle()));
    assert!(r.is_err(), "the panic reaches the caller");
    assert_eq!(texts(&s), ["n 0", "m 1"]);
    s.act("n", || n.set(2));
    s.settle().expect("frame");
    assert_eq!(texts(&s), ["n 2", "m 1"]);
}

/// The texts the last frame shows, in order.
fn texts(s: &Scenario) -> Vec<String> {
    s.last()
        .nodes
        .iter()
        .filter_map(|n| match &n.content {
            gui::layout::Content::Text { content, .. } => Some(content.to_string()),
            _ => None,
        })
        .collect()
}

/// The coordinator's r15 ask (a failing component does not blank the
/// window): the app's root render panicking on every try keeps the
/// window as it was (its last output, and the input routed into it), and
/// renders again once what it read before the panic changes.
#[test]
fn a_root_render_that_panics_keeps_the_window_as_it_was() {
    let boom = Signal::new(false);
    let mut s = Scenario::measuring(
        move || {
            if boom.get() {
                panic!("render")
            }
            div().size(100.).child(gui::text("shown")).into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    s.act("boom", || boom.set(true));
    let r = catch_unwind(AssertUnwindSafe(|| s.hover(7., 7.)));
    assert_eq!(r.as_ref().err().and_then(said), Some("render"));
    assert_eq!(texts(&s), ["shown"], "the window as it was");
    let r = catch_unwind(AssertUnwindSafe(|| s.settle()));
    assert!(r.is_ok(), "not tried again until what it read changes");
    s.act("calm", || boom.set(false));
    s.hover(8., 8.);
    assert_eq!(texts(&s), ["shown"]);
    assert_eq!(s.read(gui::interact::pointer_position), Some((8., 8.)));
}

/// Opus r15 I2 (render in routing), under the coordinator's r15 ask: a
/// render that panics as input makes the frame current keeps its last
/// output, and the input is routed all the same, into the window as it
/// was: the move moves the pointer, and the node it is over is there.
#[test]
fn a_render_panicking_as_input_lays_out_drops_no_input() {
    let boom = Signal::new(false);
    let armed = Rc::new(Cell::new(true));
    let a = Rc::clone(&armed);
    let mut s = Scenario::measuring(
        move || {
            if boom.get() && a.replace(false) {
                panic!("render")
            }
            div().size(100.).into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    s.hover(1., 1.);
    s.act("boom", || boom.set(true));
    let r = catch_unwind(AssertUnwindSafe(|| s.hover(7., 7.)));
    assert_eq!(r.as_ref().err().and_then(said), Some("render"));
    let mut at = None;
    s.act("read", || at = gui::interact::pointer_position());
    assert_eq!(at, Some((7., 7.)), "the move was routed");
    assert_eq!(s.last().nodes.len(), 1, "the window as it was");
}

/// The coordinator's r15 ask: one component whose render panics keeps its
/// last output while the rest of the window renders what changed with it,
/// in the same frame (Flutter's error widget stands for the one element
/// whose build threw); it renders again once what it read changes.
#[test]
fn a_failing_component_keeps_its_last_output_and_the_rest_draws() {
    let (a, b) = (Signal::new(0_i32), Signal::new(0_i32));
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(gui::component((), move |_| {
                    move || {
                        let v = a.get();
                        assert!(v != 1, "a is not 1");
                        gui::text(format!("a {v}")).into_element()
                    }
                }))
                .child(gui::component((), move |_| {
                    move || gui::text(format!("b {}", b.get())).into_element()
                }))
                .into_element()
        },
        200,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    s.act("both", || {
        a.set(1);
        b.set(1);
    });
    let r = catch_unwind(AssertUnwindSafe(|| s.settle()));
    assert!(r.is_err(), "the panic reaches the caller");
    assert_eq!(texts(&s), ["a 0", "b 1"], "a as it was, b as it is now");
    s.act("a", || a.set(2));
    s.settle().expect("frame");
    assert_eq!(texts(&s), ["a 2", "b 1"]);
}

/// The coordinator's r15 report (components.rs, props compared): props
/// whose comparison panics leave the child as it was, its instance, its
/// props and its output; the next props passed reach it, with no mount
/// again (a mount would compare nothing, and keep what was refused).
#[test]
fn a_props_comparison_that_panics_leaves_the_child_as_it_was() {
    /// Props whose comparison panics for 13.
    #[derive(Clone)]
    struct Touchy(i32);
    impl PartialEq for Touchy {
        fn eq(&self, other: &Self) -> bool {
            assert!(other.0 != 13 && self.0 != 13, "13 is not compared");
            self.0 == other.0
        }
    }
    let n = Signal::new(1_i32);
    let setups = Rc::new(Cell::new(0_u32));
    let counted = Rc::clone(&setups);
    let mut s = Scenario::measuring(
        move || {
            let counted = Rc::clone(&counted);
            div()
                .child(gui::text(format!("parent {}", n.get())))
                .child(gui::component(Touchy(n.get()), move |cx| {
                    counted.set(counted.get() + 1);
                    let props = cx.props();
                    move || gui::text(format!("child {}", props.with(|p| p.0))).into_element()
                }))
                .into_element()
        },
        200,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    s.act("13", || n.set(13));
    let r = catch_unwind(AssertUnwindSafe(|| s.settle()));
    assert!(r.is_err(), "the panic reaches the caller");
    assert_eq!(texts(&s), ["parent 13", "child 1"], "the child as it was");
    s.act("14", || n.set(14));
    s.settle().expect("frame");
    assert_eq!(texts(&s), ["parent 14", "child 14"]);
    assert_eq!(setups.get(), 1, "never mounted again");
}

/// The coordinator's r15 ask: a child whose setup panics is torn down
/// alone, its slot empty; its siblings and its parent draw.
#[test]
fn a_child_whose_setup_panics_leaves_its_slot_empty_and_the_rest_drawn() {
    let n = Signal::new(0_i32);
    let armed = Rc::new(Cell::new(false));
    let a = Rc::clone(&armed);
    let mut s = Scenario::measuring(
        move || {
            let a = Rc::clone(&a);
            let v = n.get();
            let failing = (v > 0).then(|| {
                gui::component(v, move |_| {
                    assert!(!a.replace(false), "setup");
                    || gui::text("late").into_element()
                })
            });
            div()
                .child(gui::text(format!("n {v}")))
                .children(failing)
                .child(gui::component((), |_| || gui::text("sibling").into_element()))
                .into_element()
        },
        200,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    armed.set(true);
    s.act("1", || n.set(1));
    let r = catch_unwind(AssertUnwindSafe(|| s.settle()));
    assert!(r.is_err(), "the panic reaches the caller");
    assert_eq!(texts(&s), ["n 1", "sibling"]);
    s.act("2", || n.set(2));
    s.settle().expect("frame");
    assert_eq!(
        texts(&s),
        ["n 2", "late", "sibling"],
        "mounted as its parent rendered again"
    );
}

/// Opus r15 I2's list of ungated user code, under the coordinator's r15
/// ask: a bound Tab stop whose closure panics as Tab moves focus answers
/// as no closure would (a stop): the Tab is not dropped.
#[test]
fn a_bound_tab_stop_that_panics_drops_no_tab() {
    let (a, b) = (gui::FocusHandle::new(), gui::FocusHandle::new());
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(
                    div()
                        .track_focus(a)
                        .size(20.)
                        .tab_stop(|| -> bool { panic_any("stop") }),
                )
                .child(div().track_focus(b).size(20.))
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    s.settle().expect("frame");
    let r = catch_unwind(AssertUnwindSafe(|| {
        s.press(gui::events::NamedKey::Tab, gui::events::Modifiers::default());
    }));
    assert_eq!(r.as_ref().err().and_then(said), Some("stop"));
    assert!(s.read(|| a.is_focused()), "Tab moved focus to the first stop");
}
