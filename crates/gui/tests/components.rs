//! ADR-0013 end to end: what a frame renders, and what bindings change
//! without rendering.

use gui::prelude::*;
use gui::{Color, Scenario, Signal, component, div, text, text_bound};

fn texts(s: &Scenario) -> Vec<String> {
    s.last()
        .nodes
        .iter()
        .filter_map(|n| match &n.content {
            gui::layout::Content::Text { content, .. } => Some(content.clone()),
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
