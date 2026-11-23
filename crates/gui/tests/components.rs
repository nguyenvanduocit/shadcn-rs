//! ADR-0013 end to end: what a frame renders, and what bindings change
//! without rendering.

use std::cell::Cell;
use std::rc::Rc;

use gui::prelude::*;
use gui::{Color, Computed, Effect, Scenario, Signal, component, div, text, text_bound};

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

#[test]
fn a_frame_with_nothing_changed_renders_nothing() {
    let n = Signal::new(0_i32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(component((), move |_| {
                    move || text(format!("n {}", n.get())).into_element()
                }))
                .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    let again = s.frame("again").expect("a frame");
    assert!(again.rendered.is_empty(), "{:?}", again.rendered);
    s.act("write", || n.set(1));
    let written = s.frame("written").expect("a frame");
    assert_eq!(written.rendered.len(), 1, "{:?}", written.rendered);
    assert_eq!(texts(&s), ["n 1"]);
}

#[test]
fn a_binding_changes_the_frame_without_rendering() {
    let label = Signal::new("a".to_owned());
    let wide = Signal::new(false);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(component((), move |_| {
                    move || {
                        div()
                            .named("box")
                            .width(move || if wide.get() { 160.0 } else { 40.0 })
                            .background(Color::BLACK)
                            .child(text_bound(move || label.get()))
                            .into_element()
                    }
                }))
                .into_element()
        },
        400,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    s.act("write", || {
        label.set("b".to_owned());
        wide.set(true);
    });
    let r = s.frame("bound").expect("a frame");
    assert!(r.rendered.is_empty(), "no instance renders: {:?}", r.rendered);
    assert!(r.bindings >= 2, "the bindings ran: {}", r.bindings);
    assert_eq!(texts(&s), ["b"]);
    let w = s
        .last()
        .nodes
        .iter()
        .find(|n| n.name == Some("box"))
        .map(|n| n.bounds.width);
    assert_eq!(w, Some(160.0));
}

#[test]
fn a_slot_hidden_and_shown_again_mounts_its_components_again() {
    let shown = Signal::new(true);
    let mut s = Scenario::measuring(
        move || {
            // The page builds the slot once; the part shows it or not.
            let slot = div().child(component((), |_| || text("inner").into_element()));
            div()
                .child(component(gui::Slot(slot), move |cx| {
                    move || {
                        if shown.get() {
                            cx.props().with(|s| s.0.clone()).into_element()
                        } else {
                            gui::Element::Empty
                        }
                    }
                }))
                .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    assert_eq!(texts(&s), ["inner"]);
    s.act("hide", || shown.set(false));
    s.frame("hidden").expect("a frame");
    assert!(texts(&s).is_empty());
    s.act("show", || shown.set(true));
    s.frame("shown").expect("a frame");
    assert_eq!(texts(&s), ["inner"]);
}

#[test]
fn content_moved_onto_a_layer_and_back_keeps_its_instances() {
    let floating = Signal::new(false);
    let bump = Signal::new(0_u32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(component((), move |_| {
                    move || {
                        // A keyed counter: in the flow, or lifted onto a layer
                        // at the same place (Vue's `Teleport`).
                        let content = div().key(7).child(counter(gui::Bind::default(), bump));
                        if floating.get() {
                            gui::deferred(gui::Positioning::Fill, content).into_element()
                        } else {
                            content.into_element()
                        }
                    }
                }))
                .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    s.act("bump", || bump.set(1));
    s.frame("bumped").expect("a frame");
    assert_eq!(texts(&s), ["n 1"]);
    let before = gui::component::stats();
    for (i, up) in [true, false, true, false].into_iter().enumerate() {
        s.act("move", || floating.set(up));
        s.frame(&format!("moved {i}")).expect("a frame");
        assert_eq!(texts(&s), ["n 1"], "the counter keeps its state");
    }
    let after = gui::component::stats();
    assert_eq!(after.setups, before.setups, "nothing mounted again");
    assert_eq!(after.unmounts, before.unmounts, "nothing unmounted");
}

#[derive(Clone, PartialEq)]
struct Count {
    n: gui::Bind<i32>,
}

/// A component holding a `v-model`: it bumps its value on each `bump`.
fn counter(n: gui::Bind<i32>, bump: Signal<u32>) -> gui::Element {
    component(Count { n }, move |cx| {
        let n = gui::use_model(cx.props(), |p: &Count| &p.n);
        let _bumps = gui::watch(move || bump.get(), move |_, _| n.update(|v| *v = v.saturating_add(1)));
        move || text(format!("n {}", n.get())).into_element()
    })
    .into_element()
}

#[test]
fn a_model_is_kept_by_the_component_or_bound_to_the_caller() {
    let bump = Signal::new(0_u32);
    let bound = Signal::new(10_i32);
    let heard = Signal::new(0_i32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(counter(gui::Bind::default_value(1), bump))
                .child(counter(gui::Bind::model(bound), bump))
                .child(counter(
                    gui::Bind::value(bound.get().saturating_mul(2)).on_change(move |v| heard.set(v)),
                    bump,
                ))
                .into_element()
        },
        300,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    assert_eq!(texts(&s), ["n 1", "n 10", "n 20"]);
    s.act("bump", || bump.set(1));
    s.frame("bumped").expect("a frame");
    // Kept: its own value moves. Bound: the caller's moves. Shown as a
    // value: only `on_change` hears it, and it shows what the caller keeps.
    assert_eq!(bound.get_untracked(), 11);
    assert_eq!(heard.get_untracked(), 21);
    assert_eq!(texts(&s), ["n 2", "n 11", "n 22"]);
}

#[test]
fn a_parent_restyles_a_components_root_through_every_render() {
    let n = Signal::new(0_i32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(
                    component((), move |_| {
                        move || {
                            div()
                                .named("own")
                                .width(40.0)
                                .child(text(format!("n {}", n.get())))
                                .into_element()
                        }
                    })
                    .into_element()
                    .restyle(|st| st.width(120.0)),
                )
                .into_element()
        },
        400,
        100,
        1.0,
    )
    .expect("a scenario");
    let width = |s: &Scenario| {
        s.last()
            .nodes
            .iter()
            .find(|n| n.name == Some("own"))
            .map(|n| n.bounds.width)
    };
    s.frame("first").expect("a frame");
    assert_eq!(
        width(&s),
        Some(120.0),
        "the parent's width wins over the component's own"
    );
    s.act("write", || n.set(1));
    let r = s.frame("rendered again").expect("a frame");
    assert_eq!(r.rendered.len(), 1, "{:?}", r.rendered);
    assert_eq!(texts(&s), ["n 1"]);
    assert_eq!(width(&s), Some(120.0), "a new output keeps the parent's style");
}

fn own_width(s: &Scenario) -> Option<f32> {
    s.last()
        .nodes
        .iter()
        .find(|n| n.name == Some("own"))
        .map(|n| n.bounds.width)
}

#[test]
fn a_parent_restyle_wins_over_a_property_the_root_bound() {
    let w = Signal::new(40.0_f32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(
                    component((), move |_| {
                        move || div().named("own").width(move || w.get()).height(10.0).into_element()
                    })
                    .into_element()
                    .restyle(|st| st.width(120.0)),
                )
                .into_element()
        },
        400,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    assert_eq!(own_width(&s), Some(120.0), "the parent's width wins over the bound one");
    s.act("write", || w.set(60.0));
    s.frame("bound value moved").expect("a frame");
    assert_eq!(
        own_width(&s),
        Some(120.0),
        "and keeps winning when the binding runs again"
    );
}

#[test]
fn a_new_parent_restyle_reaches_a_clean_root_with_bindings() {
    let wide = Signal::new(false);
    let h = Signal::new(20.0_f32);
    let mut s = Scenario::measuring(
        move || {
            let wide = wide.get();
            div()
                .child(
                    component((), move |_| {
                        move || div().named("own").height(move || h.get()).into_element()
                    })
                    .into_element()
                    .restyle(move |st| st.width(if wide { 200.0 } else { 50.0 })),
                )
                .into_element()
        },
        400,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    assert_eq!(own_width(&s), Some(50.0));
    s.act("widen", || wide.set(true));
    s.frame("parent rendered").expect("a frame");
    assert_eq!(
        own_width(&s),
        Some(200.0),
        "the parent's new restyle reaches the clean root"
    );
    s.act("narrow", || wide.set(false));
    s.frame("narrowed").expect("a frame");
    assert_eq!(own_width(&s), Some(50.0));
}

#[test]
fn a_node_restyled_through_style_mut_resolves_its_new_base() {
    let h = Signal::new(20.0_f32);
    let wide = Signal::new(false);
    // One node, its bindings shared by every clone of it.
    let own = div().named("own").height(move || h.get()).into_element();
    let mut s = Scenario::measuring(
        move || {
            let mut el = own.clone();
            if wide.get()
                && let Some(st) = el.style_mut()
            {
                st.width = gui::Length::Px(200.0);
            }
            div().child(el).into_element()
        },
        400,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    s.act("widen", || wide.set(true));
    s.frame("widened").expect("a frame");
    assert_eq!(
        own_width(&s),
        Some(200.0),
        "the new fixed style, not the memo of the old one"
    );
}

/// A render that writes live state fails at once, in any debug build and
/// whether or not the cold check is on: the write itself is the bug (a
/// render is pure, as in Compose), named with where the signal was made.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "a render wrote live state")]
fn a_render_that_writes_live_state_fails() {
    let n = Signal::new(0_i32);
    let seen = Signal::new(0_u32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(component((), move |_| {
                    move || {
                        seen.set(seen.get_untracked().wrapping_add(1));
                        text(format!("n {}", n.get())).into_element()
                    }
                }))
                .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
}

/// A render that writes live state only when its input changed (as the
/// message scroller's once did, splicing its list when the thread grew)
/// passes a cold re-render, which sees no change and so writes nothing,
/// but not the render check: the frame that renders the change fails.
#[cfg(debug_assertions)]
#[test]
fn a_render_that_writes_only_when_its_input_changes_fails() {
    let n = Signal::new(0_i32);
    let changes = Signal::new(0_u32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(component((), move |_| {
                    let last = Rc::new(Cell::new(0_i32));
                    move || {
                        let now = n.get();
                        if last.replace(now) != now {
                            changes.set(changes.get_untracked().wrapping_add(1));
                        }
                        text(format!("n {now}")).into_element()
                    }
                }))
                .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    s.check_cold(true);
    s.settle().expect("nothing changed: nothing written");
    s.act("change", || n.set(1));
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.settle()))
        .expect_err("the render that saw the change wrote live state");
    let message = failed
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| failed.downcast_ref::<&str>().map(|m| (*m).to_owned()))
        .unwrap_or_default();
    assert!(message.contains("a render wrote live state"), "{message}");
}

/// The same instance with its write moved out of render (to the input that
/// changes `n`) passes the cold check, through a change that renders it.
#[test]
fn a_pure_render_passes_the_cold_check() {
    let n = Signal::new(0_i32);
    let seen = Signal::new(0_u32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(component((), move |_| {
                    move || text(format!("n {}", n.get())).into_element()
                }))
                .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    s.check_cold(true);
    s.settle().expect("a frame");
    s.act("write", || {
        n.set(1);
        seen.set(seen.get_untracked().wrapping_add(1));
    });
    s.settle().expect("a frame");
    assert_eq!(texts(&s), ["n 1"]);
    assert_eq!(seen.get_untracked(), 1);
}

/// The panic's message, as text.
#[cfg(debug_assertions)]
fn message(failed: &(dyn std::any::Any + Send)) -> String {
    failed
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| failed.downcast_ref::<&str>().map(|m| (*m).to_owned()))
        .unwrap_or_default()
}

/// A render whose input changed hands the write to an effect: the render
/// writes nothing (the cold check, on, re-renders it and compares), and the
/// effect runs once the render returns, before layout, so the frame that
/// renders the change shows what the effect wrote.
#[test]
fn an_effect_a_render_makes_writes_before_the_same_frames_layout() {
    let n = Signal::new(0_i32);
    let seen = Signal::new(0_i32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(component((), move |_| {
                    let last = Rc::new(Cell::new(0_i32));
                    move || {
                        let now = n.get();
                        if last.replace(now) != now {
                            Effect::new(move || seen.set(now));
                        }
                        text(format!("n {now}")).into_element()
                    }
                }))
                .child(text_bound(move || format!("seen {}", seen.get())))
                .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    s.check_cold(true);
    s.settle().expect("a frame");
    assert_eq!(texts(&s), ["n 0", "seen 0"]);
    s.act("change", || n.set(1));
    s.frame("the change").expect("one frame");
    assert_eq!(
        texts(&s),
        ["n 1", "seen 1"],
        "the effect's write shows in the frame that rendered it"
    );
}

/// The effects a cold re-render makes never run: they go with its scope.
#[test]
fn a_cold_re_renders_effects_never_run() {
    let n = Signal::new(0_i32);
    let renders = Rc::new(Cell::new(0_u32));
    let runs = Rc::new(Cell::new(0_u32));
    let (r, e) = (Rc::clone(&renders), Rc::clone(&runs));
    let mut s = Scenario::measuring(
        move || {
            let (r, e) = (Rc::clone(&r), Rc::clone(&e));
            div()
                .child(component((), move |_| {
                    move || {
                        if !gui::component::is_cold_render() {
                            r.set(r.get() + 1);
                        }
                        let e = Rc::clone(&e);
                        Effect::new(move || e.set(e.get() + 1));
                        text(format!("n {}", n.get())).into_element()
                    }
                }))
                .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    s.check_cold(true);
    s.settle().expect("a frame");
    s.act("change", || n.set(1));
    s.settle().expect("a frame");
    assert_eq!(renders.get(), 2, "two renders of its own");
    assert_eq!(
        runs.get(),
        renders.get(),
        "one effect run for each, none for the cold check's"
    );
}

/// A computed's getter only reads: one that writes a live signal panics.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "a computed's getter wrote live state")]
fn a_computed_getter_that_writes_fails() {
    let other = Signal::new(0_i32);
    let c = Computed::new(move || {
        other.set(1);
        5_i32
    });
    let _ = c.get();
}

/// A write a render may not make is refused before it lands: the signal
/// keeps its value.
#[cfg(debug_assertions)]
#[test]
fn a_refused_write_leaves_the_signal_as_it_was() {
    let seen = Signal::new(0_u32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(component((), move |_| {
                    move || {
                        seen.set(5);
                        text("x").into_element()
                    }
                }))
                .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.settle())).expect_err("refused");
    assert!(message(&*failed).contains("a render wrote live state"));
    assert_eq!(seen.get_untracked(), 0, "the refused value never landed");
}

/// Siblings A, B and C under one parent: B renders on a change and its
/// effect writes what A read, after A resolved. C's turn takes A's mark
/// in, and the parent must keep the work below it marked: A renders again
/// within the frame, not left dirty behind the parent's output.
#[test]
fn a_sibling_marked_by_a_later_siblings_effect_renders_in_the_same_frame() {
    let read_by_a = Signal::new(0_i32);
    let trigger = Signal::new(0_i32);
    let mut s = Scenario::measuring(
        move || {
            component((), move |_| {
                move || {
                    div()
                        .child(component((), move |_| {
                            move || text(format!("a {}", read_by_a.get())).into_element()
                        }))
                        .child(component((), move |_| {
                            let last = Rc::new(Cell::new(0_i32));
                            move || {
                                let now = trigger.get();
                                if last.replace(now) != now {
                                    Effect::new(move || read_by_a.set(now));
                                }
                                text(format!("b {now}")).into_element()
                            }
                        }))
                        .child(component((), move |_| move || text("c").into_element()))
                        .into_element()
                }
            })
            .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    assert_eq!(texts(&s), ["a 0", "b 0", "c"]);
    s.act("change B", || trigger.set(7));
    s.frame("one frame").expect("a frame");
    assert_eq!(
        texts(&s),
        ["a 7", "b 7", "c"],
        "A shows what B's effect wrote, in the same frame"
    );
}

/// A child's effect, made by its render, panics as it runs after that
/// render (inside its parent's resolve); the caller catches the panic.
/// The parent's retained output stays in place: the child then invalidated
/// alone renders again under its parent's output, not into nothing.
#[test]
fn a_childs_panicking_effect_leaves_its_parents_output_in_place() {
    let boom = Signal::new(false);
    let n = Signal::new(0_i32);
    let fired = Rc::new(Cell::new(false));
    let mut s = Scenario::measuring(
        move || {
            let fired = Rc::clone(&fired);
            component((), move |_| {
                let fired = Rc::clone(&fired);
                move || {
                    let fired = Rc::clone(&fired);
                    let on = boom.get();
                    div()
                        .child(text(if on { "parent on" } else { "parent off" }))
                        .child(component((), move |_| {
                            let fired = Rc::clone(&fired);
                            move || {
                                if boom.get() && !fired.replace(true) {
                                    let _boom = Effect::new(|| panic!("the child's effect fails"));
                                }
                                text(format!("child {}", n.get())).into_element()
                            }
                        }))
                        .into_element()
                }
            })
            .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("settled");
    s.act("the child's effect will fail", || boom.set(true));
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.settle()));
    assert!(failed.is_err(), "the panic reaches the caller");
    s.act("the child alone changes", || n.set(1));
    s.settle().expect("settled");
    let shown = texts(&s);
    assert!(
        shown.contains(&"parent on".to_owned()),
        "the parent's output is there: {shown:?}"
    );
    assert!(
        shown.contains(&"child 1".to_owned()),
        "the child renders under it: {shown:?}"
    );
}

/// Props whose comparison (user code, run as the parent passes them)
/// panics for 13.
#[derive(Clone)]
struct Touchy(i32);

impl PartialEq for Touchy {
    fn eq(&self, other: &Self) -> bool {
        assert!(other.0 != 13 && self.0 != 13, "13 is not compared");
        self.0 == other.0
    }
}

/// Passing a child new props runs user code (their comparison) that
/// panics; the caller catches it. The child keeps its props in place: the
/// next props passed reach it and it renders them.
#[test]
fn a_panic_as_props_are_passed_leaves_the_childs_props_in_place() {
    let n = Signal::new(1_i32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(component(Touchy(n.get()), |cx| {
                    let props = cx.props();
                    move || text(format!("child {}", props.with(|p| p.0))).into_element()
                }))
                .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("settled");
    s.act("13", || n.set(13));
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.settle()));
    assert!(failed.is_err(), "the panic reaches the caller");
    s.act("14", || n.set(14));
    s.settle().expect("settled");
    assert_eq!(texts(&s), ["child 14"]);
}

/// Two hooks due after layout, the first panicking: the second runs all
/// the same, and the panic reaches the caller.
#[test]
fn a_panicking_after_layout_hook_leaves_none_after_it_unrun() {
    let ran = Rc::new(Cell::new(false));
    let r = Rc::clone(&ran);
    let mut s = Scenario::measuring(
        move || {
            let r = Rc::clone(&r);
            component((), move |_| {
                gui::after_layout(|| panic!("the first hook fails"));
                let r = Rc::clone(&r);
                gui::after_layout(move || r.set(true));
                || text("x").into_element()
            })
            .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.settle()));
    assert!(failed.is_err(), "the panic reaches the caller");
    assert!(ran.get(), "the hook after it ran");
}
