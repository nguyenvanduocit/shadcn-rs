//! Review r16's reproducers, kept as regression tests (ADR-0015): a turn is
//! a capability every entry from outside holds, the live UI loop and the
//! app's setup included; a window owns the work scheduled in it, and its
//! close is a turn of that window in a fixed order; the arbiter keeps the
//! panic whose unwind began first; a contained getter runs once per pass;
//! holds are kept per root and never switch the cold check off; the
//! harness finds the stored kinds by type.

#![expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gui::app::{Pipeline, Scenario};
use gui::compositor::input_log::{Event, Told};
use gui::environment as env;
use gui::prelude::*;
use gui::{Callback, Effect, Signal, clock, div};

fn pipe() -> Pipeline {
    Pipeline::new(|| div().into_element(), false)
}

fn window() -> Scenario {
    let mut s = Scenario::measuring(|| div().into_element(), 100, 100, 1.).expect("scenario");
    s.settle().expect("frame");
    s
}

/// What a panic said.
fn said(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&'static str>()
        .map(|s| (*s).to_owned())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

/// Codex r16 1: a bound width whose getter panics on its first use is
/// called once, contributes nothing, and the rest of the tree is laid out.
#[test]
fn a_first_use_binding_that_panics_runs_once_and_the_rest_draws() {
    let calls = Rc::new(Cell::new(0));
    let c = Rc::clone(&calls);
    let mut s = Scenario::measuring(
        move || {
            let c = Rc::clone(&c);
            div()
                .child(div().width(move || -> f32 {
                    c.set(c.get() + 1);
                    panic!("width")
                }))
                .child(div().size(10.))
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    assert!(catch_unwind(AssertUnwindSafe(|| s.settle())).is_err());
    assert_eq!((calls.get(), s.last().nodes.len()), (1, 3));
}

/// Decision 4 over the other bound kinds a node's layout reads: a bound
/// text and a drawing whose first run panics run once in the pass, give
/// nothing, and the rest of the tree is laid out.
#[test]
fn every_first_use_binding_that_panics_runs_once_and_the_rest_draws() {
    for kind in ["text", "drawing"] {
        let calls = Rc::new(Cell::new(0));
        let c = Rc::clone(&calls);
        let mut s = Scenario::measuring(
            move || {
                let c = Rc::clone(&c);
                let failing = match kind {
                    "text" => gui::element::text_bound(move || -> String {
                        c.set(c.get() + 1);
                        panic!("text")
                    })
                    .into_element(),
                    _ => gui::element::drawing(move || -> gui::chunk::Drawing {
                        c.set(c.get() + 1);
                        panic!("drawing")
                    })
                    .size(10.)
                    .into_element(),
                };
                div().child(failing).child(div().size(10.)).into_element()
            },
            100,
            100,
            1.,
        )
        .expect("scenario");
        assert!(catch_unwind(AssertUnwindSafe(|| s.settle())).is_err(), "{kind}");
        assert_eq!((calls.get(), s.last().nodes.len()), (1, 3), "{kind}");
    }
}

/// Codex r16 2: a destructor that panics while the first panic unwinds
/// loses to it: the caller gets the first.
#[test]
fn a_drop_panicking_during_an_unwind_loses_to_the_unwind() {
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            Callback::<()>::new(|()| panic!("second")).call(());
        }
    }
    let mut a = pipe();
    let result = catch_unwind(AssertUnwindSafe(|| {
        a.act("work", || {
            let _cleanup = Cleanup;
            panic!("first");
        });
    }));
    assert_eq!(said(&result.expect_err("the act panics")), "first");
}

/// The same in a nested turn: the inner unwind began first, so it wins
/// over a later panic of a destructor the outer turn's unwind drops.
#[test]
fn a_drop_panicking_during_a_nested_unwind_loses_to_it() {
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            Callback::<()>::new(|()| panic!("second")).call(());
        }
    }
    let mut a = pipe();
    let result = catch_unwind(AssertUnwindSafe(|| {
        a.act("work", || {
            let _cleanup = Cleanup;
            Callback::<()>::new(|()| {
                let _inner = Cleanup;
                panic!("first");
            })
            .call(());
            panic!("after");
        });
    }));
    assert_eq!(said(&result.expect_err("the act panics")), "first");
}

/// The same inside a fork (a check, which has no turn of its own): a
/// destructor run by the first panic's unwind disposes a scope whose
/// cleanup would panic. A fork runs no user cleanup (its writes are the
/// outside world's), so no second panic exists there to order: the check
/// ends with the first, and nothing aborts. (A fork's own drain orders its
/// faults first-to-last: `engine::r14`, F1.)
#[cfg(debug_assertions)]
#[test]
fn a_drop_panicking_during_an_unwind_in_a_fork_loses_to_it() {
    struct Cleanup(gui::Scope);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.0.dispose();
        }
    }
    let ran = Rc::new(Cell::new(false));
    let r = Rc::clone(&ran);
    let result = catch_unwind(AssertUnwindSafe(|| {
        gui::testing::in_check(|| {
            let scope = gui::Scope::detached();
            scope.run(|| {
                gui::on_cleanup(move || {
                    r.set(true);
                    panic!("second")
                });
            });
            let _cleanup = Cleanup(scope);
            panic!("first");
        });
    }));
    let said = said(&result.expect_err("the check panics"));
    assert!(
        said.contains("first") && !ran.get(),
        "{said}, the cleanup ran: {}",
        ran.get()
    );
}

/// Codex r16 3: the platform telling the window it hid wakes effects in a
/// turn, so a nested callback's panic does not cut the effect short.
#[test]
fn a_visibility_change_wakes_effects_in_a_turn() {
    // From outside any turn, the write and the effect are the thread
    // window's (ADR-0015, the thread-window rule).
    let after = Rc::new(Cell::new(false));
    let out = Rc::clone(&after);
    let e = Effect::new(move || {
        if !env::window_visible() {
            Callback::<()>::new(|()| panic!("nested")).call(());
            out.set(true);
        }
    });
    // A setter called from outside, with no turn.
    assert!(catch_unwind(AssertUnwindSafe(|| env::set_window_visible(false))).is_err());
    e.dispose();
    env::set_window_visible(true);
    assert!(after.get(), "visibility delivery lost the continuation");
}

/// Opus r16 I1 and I2, on the live loop: the platform's resize wakes the
/// component's effect inside a turn, and the app's setup ran in its window
/// so its effect sees the resize.
#[test]
fn the_live_loop_resizes_in_a_turn_and_setup_runs_in_its_window() {
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let l = Arc::clone(&log);
    let mut off = gui::offscreen(
        move || {
            let l1 = Arc::clone(&l);
            let _setup = Effect::new(move || {
                let s = env::window_size();
                l1.lock().expect("log").push(format!("setup effect: {s:?}"));
            });
            let l = Arc::clone(&l);
            move || {
                let l = Arc::clone(&l);
                div()
                    .child(gui::component((), move |_| {
                        let l = Arc::clone(&l);
                        let _owned_by_the_instance = Effect::new(move || {
                            let Some(s) = env::window_size() else { return };
                            if s.0 < 299.0 {
                                return;
                            }
                            // Inside a turn the gate answers None; outside,
                            // the panic goes on.
                            let r = catch_unwind(AssertUnwindSafe(|| {
                                Callback::<()>::new(|()| panic!("probe")).call(());
                            }));
                            let at = if r.is_err() { "OUTSIDE a turn" } else { "inside a turn" };
                            l.lock().expect("log").push(format!("component effect: {s:?} {at}"));
                        });
                        || div().size(10.).into_element()
                    }))
                    .into_element()
            }
        },
        gui::RunOptions::default(),
        (200, 100),
        1.0,
        Duration::from_millis(8),
    )
    .expect("offscreen");
    std::thread::sleep(Duration::from_millis(300));
    off.send(Event::Told(Told::Resized {
        size: (300, 150),
        scale: 1.0,
    }));
    let until = Instant::now() + Duration::from_secs(3);
    let seen = |log: &[String]| {
        let in_turn = log.iter().any(|s| s.contains("inside a turn"));
        let outside = log.iter().any(|s| s.contains("OUTSIDE"));
        let setup_saw = log.iter().any(|s| s.starts_with("setup effect: Some((300.0, 150.0))"));
        (in_turn, outside, setup_saw)
    };
    loop {
        let (in_turn, outside, setup_saw) = seen(&log.lock().expect("log"));
        if (in_turn || outside) && setup_saw || Instant::now() > until {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(off);
    let log = log.lock().expect("log").clone();
    let (in_turn, outside, setup_saw) = seen(&log);
    assert!(in_turn && !outside && setup_saw, "{log:?}");
}

/// Codex r16 4: closing a window runs its unmount hooks in a turn, so a
/// nested callback's panic does not cut the hook short.
#[test]
fn closing_a_window_runs_its_unmount_hooks_in_a_turn() {
    let after = Rc::new(Cell::new(false));
    let seen = Rc::clone(&after);
    let mut a = Scenario::measuring(
        move || {
            let seen = Rc::clone(&seen);
            gui::component::on_unmounted(move || {
                Callback::<()>::new(|()| panic!("shutdown")).call(());
                seen.set(true);
            });
            div().into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    a.settle().expect("frame");
    drop(a);
    assert!(after.get(), "shutdown lost the continuation");
}

/// Opus r16 I3: a window's unmount hooks run in that window, not in the
/// window that ran last.
#[test]
fn closing_a_window_runs_its_unmount_hooks_in_that_window() {
    let seen = Rc::new(Cell::new(None));
    let out = Rc::clone(&seen);
    let mut a = Scenario::measuring(
        move || {
            let out = Rc::clone(&out);
            div()
                .child(gui::component((), move |_| {
                    let out = Rc::clone(&out);
                    gui::on_unmounted(move || out.set(Some(env::window_visible())));
                    move || div().size(10.).into_element()
                }))
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    a.settle().expect("frame");
    let mut b = window();
    b.act("hide b", || env::set_window_visible(false));
    drop(a);
    assert_eq!(seen.get(), Some(true), "a's cleanup ran in b's window");
}

/// Opus r16 I4: a timer of a closed window never runs.
#[test]
fn a_timer_of_a_closed_window_never_runs() {
    let mut a = window();
    let mut b = window();
    let seen = Rc::new(Cell::new(None));
    let out = Rc::clone(&seen);
    a.act("schedule", move || {
        clock::after(Duration::from_millis(5), move || out.set(Some(env::window_visible())));
    });
    drop(a);
    b.act("hide b", || env::set_window_visible(false));
    let r = catch_unwind(AssertUnwindSafe(|| b.at(Duration::from_millis(10))));
    assert!(r.is_ok(), "b's turn panicked running a dead window's timer");
    assert_eq!(seen.get(), None, "a closed window's timer ran");
}

/// Codex r16 5: the same with pipelines and the scripted clock.
#[test]
fn a_timer_of_a_closed_pipeline_never_runs() {
    let (mut a, mut b) = (pipe(), pipe());
    clock::script_fresh(Duration::ZERO);
    let ran = Rc::new(Cell::new(None));
    let out = Rc::clone(&ran);
    a.act("schedule", || {
        clock::after(Duration::ZERO, move || out.set(Some(env::window_visible())));
    });
    drop(a);
    b.act("advance", || {
        clock::advance_to(Duration::ZERO);
    });
    assert_eq!(ran.get(), None, "a closed window's timer ran");
}

/// A window closed while its pipeline (and so its `Rc`) still lives: its
/// close cancels its timers, not the last handle's drop.
#[test]
fn a_timer_of_a_window_closed_but_still_held_never_runs() {
    let (mut a, mut b) = (pipe(), pipe());
    clock::script_fresh(Duration::ZERO);
    let ran = Rc::new(Cell::new(false));
    let out = Rc::clone(&ran);
    a.act("schedule", || {
        clock::after(Duration::ZERO, move || out.set(true));
    });
    a.close().expect("closed");
    // Cancelled as it closed, not found dead when it would fire: the work
    // and what it captured are gone now.
    assert_eq!(Rc::strong_count(&ran), 1, "the closed window's timer is still queued");
    b.act("advance", || {
        clock::advance_to(Duration::ZERO);
    });
    assert!(!ran.get(), "a closed window's timer ran");
    drop(a);
}

/// Codex r16 6: a layout hook runs in the window it was scheduled in.
#[test]
fn a_layout_hook_runs_in_its_window() {
    let (mut a, mut b) = (pipe(), pipe());
    let scope = gui::Scope::detached();
    let seen = Rc::new(Cell::new(None));
    a.act("queue", || {
        env::set_window_visible(false);
        let seen = Rc::clone(&seen);
        scope.run(|| gui::component::after_layout(move || seen.set(Some(env::window_visible()))));
    });
    b.act("layout", || {
        gui::component::run_post();
    });
    a.act("layout", || {
        gui::component::run_post();
    });
    assert_eq!(seen.get(), Some(false));
}

/// Codex r16 6: an effect's cleanup runs in the window the effect ran in.
#[test]
fn an_effect_cleanup_runs_in_its_window() {
    let (mut a, mut b) = (pipe(), pipe());
    let seen = Rc::new(Cell::new(None));
    let mut e = None;
    a.act("make", || {
        env::set_window_visible(false);
        let seen = Rc::clone(&seen);
        e = Some(Effect::new(move || {
            let seen = Rc::clone(&seen);
            gui::on_cleanup(move || seen.set(Some(env::window_visible())));
        }));
    });
    b.act("dispose", || e.expect("made").dispose());
    assert_eq!(seen.get(), Some(false));
}

/// Codex r16 7: a fault held in one window leaves another window's cold
/// check running.
#[test]
fn a_fault_held_in_one_window_leaves_anothers_cold_check_on() {
    let boom = Signal::new(false);
    let mut a = Scenario::measuring(
        move || {
            assert!(!boom.get(), "held");
            div().into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    let sampled = Rc::new(Cell::new(false));
    let probe = Rc::clone(&sampled);
    let mut b = Scenario::measuring(
        move || {
            if gui::component::is_cold_render() {
                probe.set(true);
            }
            div().into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    b.check_cold(true);
    b.settle().expect("frame");
    assert!(sampled.replace(false));
    a.settle().expect("frame");
    a.act("fail", || boom.set(true));
    assert!(catch_unwind(AssertUnwindSafe(|| a.settle())).is_err());
    b.settle().expect("frame");
    assert!(sampled.get(), "independent root was never checked");
}

/// What a sibling of the stale node holds over a panic.
#[derive(Clone, Copy, PartialEq)]
enum Held {
    Nothing,
    /// Its render.
    Render,
    /// Its bound text.
    Text,
    /// Its bound width.
    Style,
}

/// A frame with a missed dependency, which the cold check must catch
/// (`stale`; without, a frame the check passes).
fn stale_frame(held: Held) -> Result<(), String> {
    frame_after(held, true)
}

fn frame_after(held: Held, stale: bool) -> Result<(), String> {
    let width = Rc::new(Cell::new(10.0_f32));
    let other = Signal::new(0_i32);
    let boom = Signal::new(false);
    let w = Rc::clone(&width);
    let mut s = Scenario::measuring(
        move || {
            let w = Rc::clone(&w);
            let d = div()
                .child(gui::component((), move |_| {
                    let w = Rc::clone(&w);
                    move || div().width(w.get()).height(10.).into_element()
                }))
                .child(gui::component((), move |_| {
                    move || gui::text(format!("o {}", other.get())).into_element()
                }));
            match held {
                Held::Nothing => d,
                Held::Render => d.child(gui::component((), move |_| {
                    move || {
                        assert!(!boom.get(), "held");
                        gui::text("held").into_element()
                    }
                })),
                Held::Text => d.child(gui::element::text_bound(move || {
                    assert!(!boom.get(), "held");
                    "held".to_owned()
                })),
                Held::Style => d.child(div().height(5.).width(move || -> f32 {
                    assert!(!boom.get(), "held");
                    20.
                })),
            }
            .into_element()
        },
        100,
        100,
        1.,
    )
    .map_err(|e| format!("{e:?}"))?;
    s.check_cold(true);
    s.settle().map_err(|e| format!("{e:?}"))?;
    if held != Held::Nothing {
        s.act("boom", || boom.set(true));
        if catch_unwind(AssertUnwindSafe(|| s.settle())).is_ok() {
            return Err("the held sibling did not fault".to_owned());
        }
    }
    s.act("stale", || {
        if stale {
            width.set(40.0);
        }
        other.set(1);
    });
    match catch_unwind(AssertUnwindSafe(|| s.settle())) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(format!("{e:?}")),
        Err(p) => Err(said(&p)),
    }
}

/// Opus r16 I5: a fault held in window A does not hide window B's missed
/// dependency.
#[test]
fn a_fault_held_in_one_window_hides_no_missed_dependency_in_another() {
    let control = stale_frame(Held::Nothing);
    let boom = Signal::new(false);
    let mut a = Scenario::measuring(
        move || {
            div()
                .child(gui::component((), move |_| {
                    move || {
                        assert!(!boom.get(), "a");
                        gui::text("a").into_element()
                    }
                }))
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    a.settle().expect("frame");
    a.act("boom", || boom.set(true));
    assert!(catch_unwind(AssertUnwindSafe(|| a.settle())).is_err());
    let held = stale_frame(Held::Nothing);
    assert!(control.is_err(), "the control frame was not caught: {control:?}");
    assert!(held.is_err(), "B's cold check was skipped because A holds a fault");
}

/// Within one tree: a sibling's missed dependency is caught while another
/// sibling is held.
#[test]
fn a_held_sibling_hides_no_missed_dependency_in_another() {
    for kind in [Held::Render, Held::Text, Held::Style] {
        let held = stale_frame(kind);
        assert!(
            held.as_ref().is_err_and(|e| e.contains("differs from a cold one")),
            "the held sibling switched the check off: {held:?}"
        );
        // And with nothing missed, the held sibling is reused as the live
        // tree holds it: the cold frame equals the live one.
        let clean = frame_after(kind, false);
        assert!(clean.is_ok(), "a held sibling failed the cold check: {clean:?}");
    }
}

/// A render held with no dependencies never runs again; the check stays
/// on for the rest of its tree.
#[test]
fn a_held_render_with_no_dependencies_leaves_the_check_on() {
    let sampled = Rc::new(Cell::new(0));
    let probe = Rc::clone(&sampled);
    let n = Signal::new(0);
    let first = Rc::new(Cell::new(true));
    let mut s = Scenario::measuring(
        move || {
            let first = Rc::clone(&first);
            let probe = Rc::clone(&probe);
            div()
                .child(gui::component((), move |_| {
                    let first = Rc::clone(&first);
                    move || {
                        // Panics once, having read nothing.
                        assert!(!first.replace(false) || gui::component::is_cold_render(), "held");
                        gui::text("held").into_element()
                    }
                }))
                .child(gui::component((), move |_| {
                    let probe = Rc::clone(&probe);
                    move || {
                        if gui::component::is_cold_render() {
                            probe.set(probe.get() + 1);
                        }
                        gui::text(format!("n {}", n.get())).into_element()
                    }
                }))
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    s.check_cold(true);
    assert!(
        catch_unwind(AssertUnwindSafe(|| s.settle())).is_err(),
        "the render panics once"
    );
    sampled.set(0);
    for i in 1..4 {
        s.act("n", || n.set(i));
        s.settle().expect("frame");
    }
    assert_eq!(sampled.get(), 3, "the check stayed off");
}

/// Codex r16 8: every stored kind has a harness case, Derive included.
#[test]
fn the_stored_kinds_include_derive() {
    let d = gui::component::Derive::new(|x: usize| x + 1);
    assert_eq!(d.call(1), 2);
    assert!(
        gui::engine::Site::ALL.iter().any(|s| format!("{s:?}") == "Derive"),
        "stored Derive has no harness kind"
    );
}

// ── Review r17: a window's life is one state machine ─────────────────────

/// A value whose drop says which window's visibility it read then.
struct Says(Rc<Cell<Option<bool>>>);

impl Drop for Says {
    fn drop(&mut self) {
        self.0.set(Some(env::window_visible()));
    }
}

/// Codex r17 1: B closes hidden A; A's pending timer is dropped in A's close
/// turn, so its capture's drop reads A (hidden), not B.
#[test]
fn cancellation_drops_timer_capture_in_its_window() {
    let (mut a, mut b) = (pipe(), pipe());
    let seen = Rc::new(Cell::new(None));
    a.act("hide and schedule", || {
        env::set_window_visible(false);
        let says = Says(Rc::clone(&seen));
        clock::after(Duration::from_secs(60), move || {
            std::hint::black_box(&says);
        });
    });
    b.act("close A", || {
        env::set_window_visible(true);
        a.close().expect("closed");
    });
    assert_eq!(seen.get(), Some(false));
}

/// Codex r17 2 / Opus r17 I2: an effect a cleanup makes as the window
/// disposes its effects is disposed by the same close.
#[test]
fn close_disposes_effect_made_by_cleanup() {
    let mut a = pipe();
    let made = Rc::new(Cell::new(None));
    a.act("install", || {
        let made = Rc::clone(&made);
        Effect::new(move || {
            let made = Rc::clone(&made);
            gui::on_cleanup(move || made.set(Some(Effect::new(|| {}))));
        });
    });
    a.close().expect("closed");
    assert!(
        !made.get().expect("the cleanup made one").is_alive(),
        "effect survived close"
    );
}

/// Decision 1: a cleanup that schedules a timer, mail and a hook after
/// layout as its window closes: each is dropped by that close, in it.
#[test]
fn work_scheduled_as_a_window_closes_is_dropped_by_the_close_in_it() {
    let mut a = pipe();
    let seen: Vec<Rc<Cell<Option<bool>>>> = (0..3).map(|_| Rc::new(Cell::new(None))).collect();
    let probes = seen.clone();
    // The mailbox's sender, kept past the close: its handler goes with the
    // window, not with its last sender.
    let senders: Rc<std::cell::RefCell<Vec<gui::Sender<()>>>> = Rc::default();
    let keep = Rc::clone(&senders);
    a.act("install", || {
        env::set_window_visible(false);
        let probes = probes.clone();
        Effect::new(move || {
            let probes = probes.clone();
            let keep = Rc::clone(&keep);
            gui::on_cleanup(move || {
                let [t, m, h] = [0, 1, 2].map(|k| Says(Rc::clone(&probes[k])));
                clock::after(Duration::from_secs(60), move || std::hint::black_box(&t).0.set(None));
                let mail = gui::mailbox(move |(): ()| std::hint::black_box(&m).0.set(None));
                keep.borrow_mut().push(mail);
                // A cleanup runs with no owner: the hook's is a scope.
                gui::Scope::detached().run(|| {
                    gui::component::after_layout(move || std::hint::black_box(&h).0.set(None));
                });
            });
        });
    });
    drop(probes);
    a.close().expect("closed");
    for (k, s) in seen.iter().enumerate() {
        assert_eq!(s.get(), Some(false), "work {k} not dropped in its window's close");
        assert_eq!(Rc::strong_count(s), 1, "work {k} outlived the close");
    }
    assert_eq!(senders.borrow().len(), 1, "its sender is still held");
}

/// Decision 1: work that keeps making work as it closes cannot keep the
/// close going: past the bound it is a fault of the close, reported.
#[test]
fn a_close_that_never_settles_is_a_fault_not_a_hang() {
    fn again(n: Rc<Cell<u32>>) {
        n.set(n.get() + 1);
        Effect::new(move || {
            let n = Rc::clone(&n);
            gui::on_cleanup(move || again(n));
        });
    }
    let mut a = pipe();
    let n = Rc::new(Cell::new(0));
    let m = Rc::clone(&n);
    a.act("install", move || again(m));
    let closed = a.close();
    assert!(closed.as_ref().is_err_and(|e| e.contains("settle")), "{closed:?}");
    assert!(n.get() < 1000, "ran {} rounds", n.get());
}

/// Codex r17 3: a callback at the top runs in the thread window; a nested
/// turn of B leaves the callback's own continuation in the thread window.
#[test]
fn nested_turn_restores_thread_window() {
    let b = std::cell::RefCell::new(pipe());
    env::set_window_visible(true);
    let still = Rc::new(Cell::new(None));
    let s = Rc::clone(&still);
    Callback::new(move |()| {
        b.borrow_mut().act("hide B", || env::set_window_visible(false));
        s.set(Some(env::window_visible()));
    })
    .call(());
    assert_eq!(still.get(), Some(true), "still reading B");
}

/// Decision 2, the release path: on a closed pipeline `act` and `read`
/// (each a `turn_with`) do nothing: no panic, the work dropped unrun,
/// `read` answering `None`.
#[cfg(not(debug_assertions))]
#[test]
fn a_closed_pipeline_does_nothing_in_release() {
    let mut a = pipe();
    a.close().expect("closed");
    let ran = Rc::new(Cell::new(false));
    let r = Rc::clone(&ran);
    let late = catch_unwind(AssertUnwindSafe(|| {
        a.act("late", move || r.set(true));
        let r = Rc::clone(&ran);
        a.read(move || r.set(true))
    }));
    assert!(matches!(late, Ok(None)), "no panic and no answer in release");
    assert!(!ran.get(), "nothing ran in the closed window");
    assert_eq!(Rc::strong_count(&ran), 1, "its work was dropped");
}

/// Codex r17 4 and decision 2: a closed pipeline runs no new work. Its work
/// is dropped at once, outside the closed window (here, the thread
/// window's code), and a debug build panics: using a closed window is a
/// caller's bug.
#[test]
fn closed_pipeline_rejects_new_work() {
    let mut a = pipe();
    a.act("hide", || env::set_window_visible(false));
    a.close().expect("closed");
    let ran = Rc::new(Cell::new(false));
    let seen = Rc::new(Cell::new(None));
    let (r, says) = (Rc::clone(&ran), Says(Rc::clone(&seen)));
    let late = catch_unwind(AssertUnwindSafe(|| {
        a.act("late", move || {
            std::hint::black_box(&says);
            r.set(true);
        });
    }));
    assert!(!ran.get(), "ran after close");
    assert_eq!(late.is_err(), cfg!(debug_assertions), "a debug build panics");
    assert_eq!(
        seen.get(),
        Some(true),
        "its work was dropped where its caller runs (the thread window), never in the closed one"
    );
}

/// Opus r17 I1: a window closed inside another window's scheduled work is
/// never current again; an effect made at the top runs.
#[test]
fn a_window_closed_inside_a_scheduled_entry_is_put_back_current() {
    let y = std::cell::RefCell::new(Some(pipe()));
    let x: Rc<std::cell::RefCell<Option<Pipeline>>> = Rc::default();
    let s = Signal::new(0);
    {
        let x = Rc::clone(&x);
        y.borrow_mut().as_mut().expect("y").act("y", move || {
            let x = Rc::clone(&x);
            Effect::new(move || {
                if s.get() == 1 {
                    drop(x.borrow_mut().take());
                }
            });
        });
    }
    *x.borrow_mut() = Some(pipe());
    x.borrow_mut().as_mut().expect("x").act("x", || {});
    s.set(1);
    assert!(x.borrow().is_none(), "X closed");
    let ran = Rc::new(Cell::new(false));
    let r = Rc::clone(&ran);
    Effect::new(move || r.set(true));
    assert!(ran.get(), "an effect made at the top after X closed never ran");
    drop(y);
}

/// Opus r17 I1, with the closed window still held: closed (not dropped)
/// inside another window's turn, it is struck off the stack, so the code
/// after it never runs in it; an effect made there runs.
#[test]
fn a_window_closed_but_held_inside_another_turn_is_never_current_again() {
    let mut y = pipe();
    let x: Rc<std::cell::RefCell<Option<Pipeline>>> = Rc::new(std::cell::RefCell::new(Some(pipe())));
    let kept: Rc<std::cell::RefCell<Option<Pipeline>>> = Rc::default();
    let ran = Rc::new(Cell::new(false));
    let s = Signal::new(0);
    {
        let (slot, kept, ran) = (Rc::clone(&x), Rc::clone(&kept), Rc::clone(&ran));
        x.borrow_mut().as_mut().expect("x").act("x", move || {
            // X's effect: run in X, entered from Y's turn when Y writes.
            Effect::new(move || {
                if s.get() == 1 {
                    let mut closing = slot.borrow_mut().take().expect("x");
                    closing.close().expect("closed");
                    // Held, not dropped: its handle still upgrades.
                    *kept.borrow_mut() = Some(closing);
                    let r = Rc::clone(&ran);
                    Effect::new(move || r.set(true));
                }
            });
        });
    }
    y.act("y writes", || s.set(1));
    assert!(kept.borrow().is_some(), "X closed and held");
    assert!(ran.get(), "an effect made after X closed ran, never in X");
    drop(kept.borrow_mut().take());
    drop(y);
}

/// Decision 4: a write from outside any turn names no window, so it is the
/// thread window's: neither window, the last one to run included, sees it.
#[test]
fn an_outside_write_lands_in_the_thread_window() {
    let (mut a, mut b) = (pipe(), pipe());
    let saw: Rc<std::cell::RefCell<Vec<&str>>> = Rc::default();
    for (p, name) in [(&mut a, "a"), (&mut b, "b")] {
        let saw = Rc::clone(&saw);
        p.act("watch", move || {
            Effect::new(move || {
                if !env::window_visible() {
                    saw.borrow_mut().push(name);
                }
            });
        });
    }
    env::set_window_visible(false);
    assert!(saw.borrow().is_empty(), "{:?} saw the outside write", saw.borrow());
    assert!(!env::window_visible(), "the thread window has it");
    env::set_window_visible(true);
    drop((a, b));
}

/// Decision 5: close reports its own panic as `Err` at the top, and nested
/// in another window's turn too, where the outer turn also goes on with it.
#[test]
fn close_reports_its_own_panic_at_the_top_and_nested() {
    fn panicky() -> Pipeline {
        let mut p = pipe();
        p.act("install", || {
            Effect::new(|| gui::on_cleanup(|| panic!("cleanup")));
        });
        p
    }
    let mut top = panicky();
    assert!(top.close().is_err_and(|e| e.contains("cleanup")));
    let mut b = pipe();
    let mut inner = Some(panicky());
    let mut said = None;
    let outer = catch_unwind(AssertUnwindSafe(|| {
        b.act("close inside", || said = inner.as_mut().map(Pipeline::close));
    }));
    assert!(
        matches!(&said, Some(Err(e)) if e.contains("cleanup")),
        "nested close said {said:?}"
    );
    assert_eq!(outer.map_err(|p| said_of(&p)).err().as_deref(), Some("cleanup"));
}

fn said_of(p: &Box<dyn std::any::Any + Send>) -> String {
    said(p)
}

/// Codex r17 plausible (decision 7): a node with a held width and a bound
/// height whose dependency was missed: the cold check still sees the stale
/// height, as the held width holds only itself.
#[test]
fn a_held_width_hides_no_stale_height_on_the_same_node() {
    let height = Rc::new(Cell::new(10.0_f32));
    let boom = Signal::new(false);
    let other = Signal::new(0_i32);
    let h = Rc::clone(&height);
    let mut s = Scenario::measuring(
        move || {
            let h = Rc::clone(&h);
            div()
                .child(
                    div()
                        .width(move || -> f32 {
                            assert!(!boom.get(), "held");
                            20.
                        })
                        .height(move || h.get()),
                )
                .child(gui::component((), move |_| {
                    move || gui::text(format!("o {}", other.get())).into_element()
                }))
                .into_element()
        },
        100,
        100,
        1.,
    )
    .expect("scenario");
    s.check_cold(true);
    s.settle().expect("frame");
    s.act("boom", || boom.set(true));
    assert!(
        catch_unwind(AssertUnwindSafe(|| s.settle())).is_err(),
        "the width faults"
    );
    s.act("stale", || {
        height.set(40.0);
        other.set(1);
    });
    let r = match catch_unwind(AssertUnwindSafe(|| s.settle())) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(format!("{e:?}")),
        Err(p) => Err(said(&p)),
    };
    assert!(
        r.as_ref().is_err_and(|e| e.contains("differs from a cold one")),
        "the held width hid the stale height: {r:?}"
    );
}
