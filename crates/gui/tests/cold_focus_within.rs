//! Where focus is, as each frame shows it: `focus_within` of a node's own
//! `Interaction` (the one a render makes), and which child a scroll
//! container tells its handle holds focus.
//!
//! - A frame built cold paints the same as the live one (ADR-0013): the
//!   cold tree starts with no focused path, so an ancestor visited before
//!   the focused node must still be told focus is within it.
//! - The first frame after the focused node moved in a render shows it
//!   where it is, not where it was: the pass that finds it elsewhere than
//!   it was built over is followed by one built over where it is.
//!
//! Run with `GUI_COLD_CHECK=1` or not: the scenarios check their frames
//! themselves.

use gui::app::Scenario;
use gui::interact::Flag;
use gui::prelude::*;
use gui::reactive::Signal;
use gui::{
    Color, Element, FocusHandle, Interaction, ListState, ScrollHandle, component, div, text, text_input, virtual_list,
};

/// The background of the node named `name` as the last frame painted it.
fn look(s: &Scenario, name: &str) -> Option<Color> {
    s.last()
        .nodes
        .iter()
        .find(|n| n.name == Some(name))
        .and_then(|n| n.look.background)
}

/// A box around a field, lit while focus is within it by `lit`, both
/// frames checked against cold ones: before focus and after a click moved
/// it into the field.
#[expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]
fn check(lit: fn(Interaction) -> gui::Div) {
    let field = gui::TextField::new("");
    let mut s = Scenario::measuring(
        move || -> Element {
            let i = Interaction::new();
            lit(i)
                .named("box")
                .interaction(i)
                .padding(10.0)
                .child(text_input(&field).width(80.0).height(24.0))
                .into_element()
        },
        400,
        120,
        1.0,
    )
    .expect("a scenario");
    s.check_cold(true);
    s.settle().expect("the first frame");
    assert_eq!(look(&s, "box"), Some(Color::WHITE));
    // The field is at 10..90 by 10..34, inside the box's padding.
    s.click(40.0, 20.0);
    s.settle().expect("the frame after focus moved inside the box");
    assert_eq!(look(&s, "box"), Some(Color::BLACK), "focus is within the box");
}

/// Read by a closure, under the node's own tracker.
#[test]
fn an_ancestors_focus_within_is_the_same_in_a_cold_frame() {
    check(|i| div().background(move || if i.focus_within() { Color::BLACK } else { Color::WHITE }));
}

/// Read by a flag source (the handle's own waiter list, no tracker).
#[test]
fn an_ancestors_focus_within_flag_source_is_the_same_in_a_cold_frame() {
    check(|i| div().background(i.when(Flag::FocusWithin, Color::WHITE, Color::BLACK)));
}

/// The focused node moves from box A to box B in one render: the first
/// frame after shows B lit and A not, live and cold.
#[test]
fn a_focused_node_moved_to_another_box_lights_that_box_in_the_first_frame() {
    let focus = FocusHandle::new();
    let in_b = Signal::new(false);
    // The boxes' handles outlive the renders: the boxes stay, and only
    // what their flags say moves.
    let (ia, ib) = (Interaction::new(), Interaction::new());
    let mut s = Scenario::measuring(
        move || -> Element {
            let boxed = |name: &'static str, holds: bool| {
                let i = if name == "a" { ia } else { ib };
                div()
                    .named(name)
                    .interaction(i)
                    .padding(10.0)
                    .height(44.0)
                    .background(i.when(Flag::FocusWithin, Color::WHITE, Color::BLACK))
                    .children(holds.then(|| div().track_focus(focus).size(24.0)))
            };
            let b = in_b.get();
            div().child(boxed("a", !b)).child(boxed("b", b)).into_element()
        },
        400,
        200,
        1.0,
    )
    .unwrap();
    s.check_cold(true);
    s.settle().unwrap();
    s.act("focus the node in A", || focus.focus());
    s.settle().unwrap();
    assert_eq!(look(&s, "a"), Some(Color::BLACK), "focus is within A");
    s.act("move the node to B", || in_b.set(true));
    s.frame("moved").unwrap();
    assert_eq!(s.read(gui::focus::focused), Some(focus.id()), "it keeps focus");
    assert_eq!(look(&s, "b"), Some(Color::BLACK), "B holds focus from the first frame");
    assert_eq!(look(&s, "a"), Some(Color::WHITE), "and A no more");
}

/// One focus handle moves from keyed row A to keyed row B of a scroll
/// container (no node comes or goes): the container's handle reports B as
/// the child holding focus.
#[test]
fn a_scroll_handle_reports_the_row_a_focus_handle_moved_to() {
    let scroll = ScrollHandle::new();
    let focus = FocusHandle::new();
    // The row without `focus` holds this one: the rows stay as they are
    // but for which handle they hold.
    let other = FocusHandle::new();
    let on_b = Signal::new(false);
    let sh = scroll;
    // Each row holds a component reading `on_b`, which holds the handle or
    // not: moving it renders those again, not the rows nor the container.
    let row = move |key: u64, on: bool| {
        div().key(key).height(30.0).child(component((), move |_| {
            move || {
                let holds = on_b.get() == on;
                div()
                    .size(20.0)
                    .child(text(format!("row {key}")))
                    .track_focus(if holds { focus } else { other })
                    .into_element()
            }
        }))
    };
    let mut s = Scenario::measuring(
        move || -> Element {
            div()
                .child(
                    div()
                        .height(100.0)
                        .overflow_y(gui::style::Overflow::Scroll)
                        .track_scroll(&sh)
                        .child(row(1, false))
                        .child(row(2, true)),
                )
                .into_element()
        },
        200,
        200,
        1.0,
    )
    .unwrap();
    s.check_cold(true);
    s.settle().unwrap();
    s.act("focus row 1", || focus.focus());
    s.settle().unwrap();
    let holding = |scroll: &ScrollHandle| {
        scroll.layout().map(|l| {
            l.children
                .iter()
                .filter(|c| c.focus_within)
                .map(|c| c.key)
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(holding(&scroll), Some(vec![Some(1)]));
    s.act("move the handle to row 2", || on_b.set(true));
    s.frame("moved").unwrap();
    assert_eq!(s.read(gui::focus::focused), Some(focus.id()));
    assert_eq!(holding(&scroll), Some(vec![Some(2)]), "the handle reports row 2");
}

/// A virtual list's focused row, scrolled out of its window and kept
/// built, stays built and focused when an unkeyed sibling comes before an
/// ancestor of the list in the same frame (every path under it moves).
#[test]
fn a_pinned_focused_row_stays_when_a_sibling_comes_before_its_list() {
    let state = ListState::new(30.0);
    let focus = FocusHandle::new();
    let before = Signal::new(false);
    let st = state;
    let mut s = Scenario::measuring(
        move || -> Element {
            let list = virtual_list(
                &st,
                10_000,
                |i| i,
                move |i| {
                    let d = div().height(24.0).child(text(format!("row {i}")));
                    if i == 3 { d.track_focus(focus) } else { d }.into_element()
                },
            )
            .height(300.0);
            let page = div().width(300.0);
            let page = if before.get() {
                page.child(div().height(20.0).child(text("before")))
            } else {
                page
            };
            page.child(div().child(list)).into_element()
        },
        300,
        400,
        1.0,
    )
    .unwrap();
    s.check_cold(true);
    s.settle().unwrap();
    s.act("focus row 3", || focus.focus());
    s.settle().unwrap();
    s.act("scroll far away", || state.scroll_to_row(5_000));
    s.settle().unwrap();
    assert_eq!(state.focused_row(), Some(3), "kept built, pinned");
    s.act("a sibling before the list", || before.set(true));
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), Some(focus.id()), "row 3 keeps focus");
    assert_eq!(state.focused_row(), Some(3), "and stays built");
}
