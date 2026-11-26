//! A node's `Interaction` (ADR-0022): one signal per flag, written by hit
//! testing and focus only when it changes, read by bindings with tracking
//! per flag. Hovering a part reruns that node's binding alone: no instance
//! renders, nothing lays out, a binding of the node that reads another
//! flag does not run, and the UI thread allocates nothing for the whole
//! frame: taking the pointer in, running the bindings, laying out,
//! recording the two chunks again (into displays the renderer let go of),
//! committing them (in the commit the renderer gave back) and reporting
//! the frame, whatever the size of the tree.

use std::cell::Cell;
use std::rc::Rc;

use gui::app::Scenario;
use gui::events::{Modifiers, NamedKey};
use gui::prelude::*;
use gui::testing::Pass;
use gui::{Color, Interaction, div, span, text, text_bound};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

const REST: Color = Color::WHITE;
const LIT: Color = Color::BLACK;

/// A column of `n` rows, each lit while hovered, and a probe counting how
/// often a binding of row 7 that reads only `focus_visible` runs.
fn rows(n: usize, runs: Rc<Cell<u32>>) -> gui::Element {
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .children((0..n).map(move |r| {
            let i = Interaction::new();
            let runs = Rc::clone(&runs);
            let row = div()
                .interaction(i)
                .height(20.0)
                .padding_x(8.0)
                .background(move || if i.hovered() { LIT } else { REST })
                .child(text(format!("row {r}")));
            if r == 7 {
                row.ring(move || {
                    runs.set(runs.get().saturating_add(1));
                    i.focus_visible().then_some(Ring::new(3.0, LIT))
                })
            } else {
                row
            }
        }))
        .into_element()
}

/// Hover rows 7, 8 and 9 in turn over a column of `n` rows: each hover
/// frame reruns two bindings, records two chunks, and allocates nothing on
/// the UI thread.
fn hover_rows(n: usize) {
    gui::testing::both_passes(|pass| hover_rows_pass(pass, n));
}

#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn hover_rows_pass(pass: Pass, n: usize) {
    let runs = Rc::new(Cell::new(0));
    let probe = Rc::clone(&runs);
    let mut s = Scenario::measuring(move || rows(n, Rc::clone(&probe)), 300, 700, 1.0).unwrap();
    // The reactive trace a scenario keeps for its reports records each
    // write: an app's frames keep none.
    gui::reactive::stop_trace();
    pass.scenario(&mut s);
    s.settle().unwrap();
    // Warm: the first hovers grow what a hover grows once (the hovered
    // set, the chains of nodes it compares, the painter's buffers).
    for row in ["row 3", "row 4"] {
        let (x, y) = s.locate(row).unwrap();
        s.hover(x, y);
        s.settle().unwrap();
        s.frame("warm").unwrap();
    }
    let before = runs.get();
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    for (step, row) in ["row 7", "row 8", "row 7", "row 8", "row 9", "row 8", "row 7"]
        .into_iter()
        .enumerate()
    {
        let (x, y) = s.locate(row).unwrap();
        let ((), input) = gui::heap::measure(|| s.hover(x, y));
        let taken = input.map_or(u64::MAX, |t| t.allocated);
        pass.measured(|| {
            assert_eq!(taken, 0, "{taken} bytes allocated taking hover {step} in");
        });
        let r = s.frame(&format!("hover {step}")).unwrap();
        pass.measured(|| {
            assert!(r.rendered.is_empty(), "no component renders: {:?}", r.rendered);
            assert_eq!(r.layout.calls, 0, "nothing lays out");
            assert_eq!(
                r.visits.resolved, 2,
                "the row the pointer left and the row it came to resolve: {}",
                r.visits
            );
            assert_eq!(r.bindings, 2, "their bindings alone run");
            assert_eq!(r.paint.recorded, 2, "their chunks alone record again: {}", r.paint);
            let bytes = |t: Option<gui::heap::ThreadHeap>| t.map_or(u64::MAX, |t| t.allocated);
            // From the frame's start to its paint: the input state taken in,
            // the bindings run, the tree laid out.
            let built = bytes(r.footprint.built);
            assert_eq!(built, 0, "{built} bytes allocated building and laying out hover {step}");
            // The two chunks recorded again and the commit that carries them.
            let recorded = bytes(r.footprint.recorded);
            assert_eq!(recorded, 0, "{recorded} bytes allocated recording hover {step}");
            // The whole frame, its report written. The headless target
            // composes and draws the commit on this thread: in an app that is
            // the render thread's, so it is left out.
            let (thread, committed) = (bytes(r.footprint.thread), bytes(r.footprint.committed));
            let ui = thread.saturating_sub(committed);
            assert_eq!(
                ui, 0,
                "{ui} bytes allocated on the UI thread for hover {step} (rows {n})"
            );
        });
    }
    // Row 7's ring reads only `focus_visible`: its node resolved again on
    // each hover, and the ring replayed from what it last gave.
    pass.measured(|| assert_eq!(runs.get(), before, "the binding that reads focus_visible never ran"));
    let frame = s.frame("rest").unwrap();
    pass.measured(|| assert_eq!(frame.visits.resolved, 0, "a steady frame resolves nothing"));
}

/// Hovering a part: no instance renders, nothing lays out, only the two
/// rows' paint bindings run, and the UI thread allocates nothing for the
/// frame, end to end, at 30 rows and at 600.
#[test]
fn hovering_a_part_reruns_only_its_paint_binding() {
    hover_rows(30);
    hover_rows(600);
}

#[test]
fn a_flag_is_tracked_alone() {
    let i = Interaction::new();
    let (hover_runs, focus_runs) = (Rc::new(Cell::new(0_u32)), Rc::new(Cell::new(0_u32)));
    let (h, f) = (Rc::clone(&hover_runs), Rc::clone(&focus_runs));
    let _hover = gui::Effect::new(move || {
        let _read = i.hovered();
        h.set(h.get().saturating_add(1));
    });
    let _focus = gui::Effect::new(move || {
        let _read = i.focus_visible();
        f.set(f.get().saturating_add(1));
    });
    let mut s = Scenario::measuring(
        move || {
            div()
                .padding(20.0)
                .child(
                    div()
                        .interaction(i)
                        .size(40.0)
                        .focusable()
                        .on_click(|_| {})
                        .child(text("part")),
                )
                .into_element()
        },
        200,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let base = (hover_runs.get(), focus_runs.get());
    let (x, y) = s.locate("part").unwrap();
    s.hover(x, y);
    s.settle().unwrap();
    assert!(i.hovered());
    assert_eq!(
        hover_runs.get(),
        base.0.saturating_add(1),
        "hover reran what reads hovered"
    );
    assert_eq!(focus_runs.get(), base.1, "and not what reads focus_visible");
    s.hover(150.0, 150.0);
    s.settle().unwrap();
    assert!(!i.hovered());
    assert_eq!(focus_runs.get(), base.1);
}

#[test]
fn hit_testing_and_focus_write_each_flag() {
    let (outer, inner, off) = (Interaction::new(), Interaction::new(), Interaction::new());
    let field = gui::TextField::new("");
    let mut s = Scenario::measuring(
        move || {
            div()
                .padding(10.0)
                .display(Display::Flex)
                .gap(10.0)
                .child(
                    div().interaction(outer).padding(10.0).child(
                        div()
                            .interaction(inner)
                            .focusable()
                            .on_click(|_| {})
                            .child(text("inner")),
                    ),
                )
                .child(div().interaction(off).disabled(true).child(text("off")))
                .child(gui::text_input(&field).width(80.0))
                .into_element()
        },
        400,
        120,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    assert!(off.disabled(), "the layout tree writes disabled");
    assert!(!inner.disabled());

    // Hover reaches the node and its ancestors.
    let (x, y) = s.locate("inner").unwrap();
    s.hover(x, y);
    s.settle().unwrap();
    assert!(inner.hovered() && outer.hovered());

    // A disabled node is never hovered.
    let (ox, oy) = s.locate("off").unwrap();
    s.hover(ox, oy);
    s.settle().unwrap();
    assert!(!off.hovered() && !inner.hovered() && !outer.hovered());

    // A click focuses without showing it; focus-within reaches up.
    s.click(x, y);
    s.settle().unwrap();
    assert!(inner.focused() && !inner.focus_visible());
    assert!(outer.focus_within() && !outer.focused());
    assert!(!inner.pressed(), "the press ends with the release");

    // Tab moves focus by keyboard: the text input shows it; then back to
    // the part, which shows it now.
    s.press(NamedKey::Tab, Modifiers::default());
    s.settle().unwrap();
    assert!(!inner.focused() && !outer.focus_within());
    s.press(
        NamedKey::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    s.settle().unwrap();
    assert!(inner.focused() && inner.focus_visible());
    assert!(outer.focus_within() && !outer.focus_visible());
}

#[test]
fn a_handle_a_render_makes_starts_as_its_node_is() {
    let tick = gui::Signal::new(0_u32);
    let seen = Rc::new(Cell::new(None::<Interaction>));
    let keep = Rc::clone(&seen);
    let mut s = Scenario::measuring(
        move || {
            let _ = tick.get();
            let i = Interaction::new();
            // The cold check's render of every instance is not one of its own.
            if !gui::component::is_cold_render() {
                keep.set(Some(i));
            }
            div()
                .padding(10.0)
                .child(
                    div()
                        .interaction(i)
                        .size(40.0)
                        .background(move || if i.hovered() { LIT } else { REST })
                        .child(text("part")),
                )
                .into_element()
        },
        200,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let (x, y) = s.locate("part").unwrap();
    s.hover(x, y);
    s.settle().unwrap();
    // A render under the resting pointer makes a new handle: the tree
    // writes it hovered before its binding reads it.
    s.act("tick", || tick.set(1));
    s.settle().unwrap();
    let i = seen.get().expect("a handle");
    assert!(i.hovered(), "the new handle starts hovered");
    s.hover(150.0, 150.0);
    s.settle().unwrap();
    assert!(!i.hovered(), "and hit testing writes it from then on");
}

#[test]
fn a_node_runs_each_binding_on_its_own_flags() {
    let i = Interaction::new();
    let (hover_runs, ring_runs) = (Rc::new(Cell::new(0_u32)), Rc::new(Cell::new(0_u32)));
    let (h, f) = (Rc::clone(&hover_runs), Rc::clone(&ring_runs));
    let mut s = Scenario::measuring(
        move || {
            let (h, f) = (Rc::clone(&h), Rc::clone(&f));
            div()
                .padding(20.0)
                .child(
                    div()
                        .interaction(i)
                        .size(40.0)
                        .focusable()
                        .on_click(|_| {})
                        // The cold check's layout runs every binding: not runs of their own.
                        .background(move || {
                            if !gui::component::is_cold_render() {
                                h.set(h.get().saturating_add(1));
                            }
                            if i.hovered() { LIT } else { REST }
                        })
                        .ring(move || {
                            if !gui::component::is_cold_render() {
                                f.set(f.get().saturating_add(1));
                            }
                            i.focus_visible().then_some(Ring::new(3.0, LIT))
                        })
                        .child(text("part")),
                )
                .into_element()
        },
        200,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let base = (hover_runs.get(), ring_runs.get());
    let (x, y) = s.locate("part").unwrap();
    s.hover(x, y);
    s.settle().unwrap();
    assert_eq!(hover_runs.get(), base.0.saturating_add(1), "hover ran the background");
    assert_eq!(ring_runs.get(), base.1, "and not the ring");
    s.press(NamedKey::Tab, Modifiers::default());
    s.settle().unwrap();
    assert!(i.focus_visible());
    assert_eq!(ring_runs.get(), base.1.saturating_add(1), "keyboard focus ran the ring");
    assert_eq!(hover_runs.get(), base.0.saturating_add(1), "and not the background");
}

/// A part a render makes again with a new handle each time (a table row,
/// a checkbox's box) makes no signal: its node's flags were made once, and
/// each new handle is bound to them, reading the node as it is.
#[test]
fn a_render_makes_no_signal_for_its_handles() {
    let tick = gui::Signal::new(0_u32);
    let seen = Rc::new(Cell::new(None::<Interaction>));
    let keep = Rc::clone(&seen);
    let mut s = Scenario::measuring(
        move || {
            let _ = tick.get();
            div()
                .padding(10.0)
                .children((0..20).map(|r| {
                    div()
                        .with_interaction(|d, i| {
                            // The cold check's render of every instance is not one of its own.
                            if r == 0 && !gui::component::is_cold_render() {
                                keep.set(Some(i));
                            }
                            d.background(move || if i.hovered() { LIT } else { REST })
                        })
                        .height(30.0)
                        .child(text(format!("part {r}")))
                }))
                .into_element()
        },
        200,
        700,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let (x, y) = s.locate("part 0").unwrap();
    s.hover(x, y);
    s.settle().unwrap();
    let mut signals = None;
    for n in 1..=10 {
        let before = seen.get();
        s.act("tick", || tick.set(n));
        s.settle().unwrap();
        let i = seen.get().expect("a handle");
        assert_ne!(Some(i), before, "each render makes a new handle");
        assert!(i.hovered(), "bound to the node's flags, as the node is");
        assert!(before.is_some_and(|b| !b.hovered()), "the last one reads off");
        let now = gui::reactive::graph().signals;
        assert_eq!(*signals.get_or_insert(now), now, "render {n} made no signal");
    }
}

/// A column of rows, each lit while hovered; row 4 casts `shadows` box
/// shadows, so its chunk holds many more items than the others'.
fn shadowed_rows(shadows: usize) -> gui::Element {
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .children((0..12).map(move |r| {
            let i = Interaction::new();
            let row = div()
                .interaction(i)
                .height(20.0)
                .padding_x(8.0)
                .background(move || if i.hovered() { LIT } else { REST })
                .child(text(format!("row {r}")));
            if r == 4 {
                let cast = Shadow {
                    x: 0.0,
                    y: 1.0,
                    blur: 2.0,
                    spread: 0.0,
                    color: Color::BLACK,
                };
                row.shadow(Shadows::repeat(shadows, cast))
            } else {
                row
            }
        }))
        .into_element()
}

/// Hovering back and forth between a plain row and one with many box
/// shadows records both rows' chunks each frame, a small one and a big
/// one: each keeps a display of its own size to record into, and the
/// spilled shadow list it records from is written over in place, so no
/// frame allocates on the UI thread once warm.
fn hover_small_and_big(shadows: usize) {
    gui::testing::both_passes(|pass| hover_small_and_big_pass(pass, shadows));
}

#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn hover_small_and_big_pass(pass: Pass, shadows: usize) {
    let mut s = Scenario::measuring(move || shadowed_rows(shadows), 300, 400, 1.0).unwrap();
    gui::reactive::stop_trace();
    pass.scenario(&mut s);
    s.settle().unwrap();
    let (small, big) = (s.locate("row 3").unwrap(), s.locate("row 4").unwrap());
    for _ in 0..4 {
        for (x, y) in [small, big] {
            s.hover(x, y);
            s.settle().unwrap();
            s.frame("warm").unwrap();
        }
    }
    for step in 0..12 {
        let (x, y) = if step % 2 == 0 { small } else { big };
        s.hover(x, y);
        let r = s.frame(&format!("hover {step}")).unwrap();
        pass.measured(|| {
            assert_eq!(r.paint.recorded, 2, "both rows record again: {}", r.paint);
            let bytes = |t: Option<gui::heap::ThreadHeap>| t.map_or(u64::MAX, |t| t.allocated);
            let built = bytes(r.footprint.built);
            assert_eq!(
                built, 0,
                "{built} bytes allocated building hover {step} ({shadows} shadows)"
            );
            let recorded = bytes(r.footprint.recorded);
            assert_eq!(
                recorded, 0,
                "{recorded} bytes allocated recording hover {step} ({shadows} shadows)"
            );
            let ui = bytes(r.footprint.thread).saturating_sub(bytes(r.footprint.committed));
            assert_eq!(
                ui, 0,
                "{ui} bytes allocated on the UI thread, hover {step} ({shadows} shadows)"
            );
        });
    }
}

#[test]
fn hovering_between_a_small_and_a_big_chunk_allocates_nothing() {
    hover_small_and_big(10);
    hover_small_and_big(30);
}

/// Two plain rows over a box casting 20,000 shadows (an item each, some
/// megabytes of them) whose background shows which row is hovered, so
/// each hover records the two rows' small chunks and the box's huge one
/// again.
fn rows_over_a_huge_chunk() -> gui::Element {
    let (a, b) = (Interaction::new(), Interaction::new());
    let row = |i: Interaction, label: &str| {
        div()
            .interaction(i)
            .height(20.0)
            .background(move || if i.hovered() { LIT } else { REST })
            .child(text(label.to_owned()))
    };
    let cast = Shadow {
        x: 0.0,
        y: 1.0,
        blur: 2.0,
        spread: 0.0,
        color: Color::BLACK,
    };
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .child(row(a, "row a"))
        .child(row(b, "row b"))
        .child(
            div()
                .height(40.0)
                .shadow(Shadows::repeat(20_000, cast))
                .background(move || if a.hovered() { REST } else { LIT }),
        )
        .into_element()
}

/// A chunk of 20,000 items (megabytes), recorded again each frame, takes back a
/// large display of its own (kept apart from the small ones, outside their
/// bytes), and the small rows hovered beside it keep theirs: once warm, no
/// frame allocates on the UI thread.
#[test]
fn a_huge_chunk_recorded_each_frame_beside_small_ones_allocates_nothing() {
    gui::testing::both_passes(huge_chunk);
}

#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn huge_chunk(pass: Pass) {
    let mut s = Scenario::measuring(rows_over_a_huge_chunk, 300, 400, 1.0).unwrap();
    gui::reactive::stop_trace();
    pass.scenario(&mut s);
    s.settle().unwrap();
    let paint = |s: &Scenario| {
        s.owners()
            .into_iter()
            .find(|o| o.name == "paint")
            .map_or(0, |o| o.bytes)
    };
    assert!(
        paint(&s) > (2 << 20),
        "the box's chunk is huge: paint holds {}",
        paint(&s)
    );
    let (a, b) = (s.locate("row a").unwrap(), s.locate("row b").unwrap());
    // Warm past the frame's first rebuild after the scenario settles,
    // which records it whole once.
    for _ in 0..16 {
        for (x, y) in [a, b] {
            s.hover(x, y);
            s.settle().unwrap();
            s.frame("warm").unwrap();
        }
    }
    for step in 0..24 {
        let (x, y) = if step % 2 == 0 { a } else { b };
        s.hover(x, y);
        let r = s.frame(&format!("hover {step}")).unwrap();
        pass.measured(|| {
            assert_eq!(r.paint.recorded, 3, "both rows and the box record again: {}", r.paint);
            let bytes = |t: Option<gui::heap::ThreadHeap>| t.map_or(u64::MAX, |t| t.allocated);
            let recorded = bytes(r.footprint.recorded);
            assert_eq!(recorded, 0, "{recorded} bytes allocated recording hover {step}");
            let ui = bytes(r.footprint.thread).saturating_sub(bytes(r.footprint.committed));
            assert_eq!(ui, 0, "{ui} bytes allocated on the UI thread, hover {step}");
        });
    }
}

/// A column of rows, each lit while hovered; row 4 casts 40 shadows while
/// hovered and none at rest, so its chunk holds 40 more items each time it
/// is hovered than the last time it recorded.
fn a_row_that_grows_when_hovered() -> gui::Element {
    let cast = Shadow {
        x: 0.0,
        y: 1.0,
        blur: 2.0,
        spread: 0.0,
        color: Color::BLACK,
    };
    let many = Shadows::repeat(40, cast);
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .children((0..8).map(move |r| {
            let i = Interaction::new();
            let row = div()
                .interaction(i)
                .height(20.0)
                .background(move || if i.hovered() { LIT } else { REST })
                .child(text(format!("row {r}")));
            if r == 4 {
                let many = many.clone();
                row.shadow(move || if i.hovered() { many.clone() } else { Shadows::new() })
            } else {
                row
            }
        }))
        .into_element()
}

/// A chunk whose size changes each time it records (a row that casts
/// shadows only while hovered) records through scratch into a display of
/// the size it records, not into one sized by what it held the last time
/// (which would grow): once warm, no frame allocates on the UI thread.
#[test]
fn a_chunk_that_changes_size_each_record_allocates_nothing() {
    gui::testing::both_passes(changing_chunk);
}

#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn changing_chunk(pass: Pass) {
    let mut s = Scenario::measuring(a_row_that_grows_when_hovered, 300, 400, 1.0).unwrap();
    gui::reactive::stop_trace();
    pass.scenario(&mut s);
    s.settle().unwrap();
    let (rest, grows) = (s.locate("row 3").unwrap(), s.locate("row 4").unwrap());
    for _ in 0..8 {
        for (x, y) in [rest, grows] {
            s.hover(x, y);
            s.settle().unwrap();
            s.frame("warm").unwrap();
        }
    }
    for step in 0..16 {
        let (x, y) = if step % 2 == 0 { rest } else { grows };
        s.hover(x, y);
        let r = s.frame(&format!("hover {step}")).unwrap();
        pass.measured(|| {
            assert_eq!(r.paint.recorded, 2, "both rows record again: {}", r.paint);
            let bytes = |t: Option<gui::heap::ThreadHeap>| t.map_or(u64::MAX, |t| t.allocated);
            let recorded = bytes(r.footprint.recorded);
            assert_eq!(recorded, 0, "{recorded} bytes allocated recording hover {step}");
            let ui = bytes(r.footprint.thread).saturating_sub(bytes(r.footprint.committed));
            assert_eq!(ui, 0, "{ui} bytes allocated on the UI thread, hover {step}");
        });
    }
}

/// A column of rows, each a paragraph lit while hovered, so hovering a
/// row draws its text again: `label` gives row `r`'s text, `underline`
/// underlines every label.
fn labelled_rows(label: fn(usize) -> String, underline: bool) -> gui::Element {
    lit_rows(move |r, _| text(label(r)).underline(underline))
}

/// A column of eight rows made by `row` (given the row's interaction),
/// each lit while hovered.
fn lit_rows(row: impl Fn(usize, Interaction) -> gui::Text) -> gui::Element {
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .children((0..8).map(move |r| {
            let i = Interaction::new();
            row(r, i)
                .interaction(i)
                .height(24.0)
                .padding_x(8.0)
                .background(move || if i.hovered() { LIT } else { REST })
        }))
        .into_element()
}

/// Hover rows 3 and 4 by turns once warm: each frame draws both labels
/// again, and nothing on the UI thread allocates.
fn hover_labels(label: fn(usize) -> String, underline: bool) {
    hover_rows_allocating_nothing(move || labelled_rows(label, underline), label);
}

/// Hover rows 3 and 4 of `ui`, found by `label`, by turns once warm: each
/// frame draws both again, and nothing on the UI thread allocates.
fn hover_rows_allocating_nothing(ui: impl Fn() -> gui::Element + 'static, label: fn(usize) -> String) {
    hover_rows_checking(ui, label, |_, _| {});
}

/// [`hover_rows_allocating_nothing`], with `check` run after each frame
/// once warm (its allocations measured), given whether row 3 (else row
/// 4) is hovered.
fn hover_rows_checking(
    ui: impl Fn() -> gui::Element + 'static,
    label: fn(usize) -> String,
    check: impl Fn(&mut Scenario, bool),
) {
    let ui = Rc::new(ui);
    gui::testing::both_passes(|pass| {
        let ui = Rc::clone(&ui);
        hover_rows_pass_checking(pass, move || ui(), label, &check);
    });
}

#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn hover_rows_pass_checking(
    pass: Pass,
    ui: impl Fn() -> gui::Element + 'static,
    label: fn(usize) -> String,
    check: &impl Fn(&mut Scenario, bool),
) {
    let mut s = Scenario::measuring(ui, 300, 400, 1.0).unwrap();
    gui::reactive::stop_trace();
    pass.scenario(&mut s);
    s.settle().unwrap();
    let (a, b) = (s.locate(&label(3)).unwrap(), s.locate(&label(4)).unwrap());
    for _ in 0..8 {
        for (x, y) in [a, b] {
            s.hover(x, y);
            s.settle().unwrap();
            s.frame("warm").unwrap();
        }
    }
    for step in 0..16 {
        let (x, y) = if step % 2 == 0 { a } else { b };
        s.hover(x, y);
        let r = s.frame(&format!("hover {step}")).unwrap();
        pass.measured(|| {
            assert!(r.paint.recorded >= 2, "both labels record again: {}", r.paint);
            let bytes = |t: Option<gui::heap::ThreadHeap>| t.map_or(u64::MAX, |t| t.allocated);
            let recorded = bytes(r.footprint.recorded);
            assert_eq!(recorded, 0, "{recorded} bytes allocated recording hover {step}");
            // A hover resolves the paragraph again (its background binds) and
            // writes its frame node anew: both share the content and runs it
            // held, and layout copies nothing.
            let built = bytes(r.footprint.built);
            assert_eq!(built, 0, "{built} bytes allocated building hover {step}");
            // The whole frame on the UI thread: layout, the text drawn again,
            // the commit, the frame's end and its report.
            let ui = bytes(r.footprint.thread).saturating_sub(bytes(r.footprint.committed));
            assert_eq!(ui, 0, "{ui} bytes allocated on the UI thread, hover {step}");
        });
        check(&mut s, step % 2 == 0);
    }
}

/// Hovering a row draws its plain label again, allocating nothing.
#[test]
fn drawing_a_label_again_allocates_nothing() {
    hover_labels(|r| format!("label {r}"), false);
}

/// An underlined label with descenders is drawn again (its glyphs, the
/// skip-ink gaps cut from its line) allocating nothing.
#[test]
fn drawing_an_underlined_label_again_allocates_nothing() {
    hover_labels(|r| format!("jumpy grapes {r}"), true);
}

/// A label whose text a binding gives, hovered: its background resolves it
/// again, its unchanged text is read and shared, and nothing allocates.
#[test]
fn hovering_a_bound_label_allocates_nothing() {
    fn label(r: usize) -> String {
        format!("bound {r}")
    }
    hover_rows_allocating_nothing(|| lit_rows(|r, _| text_bound(move || label(r))), label);
}

/// A paragraph of static spans, hovered: its runs, resolved again equal,
/// are compared where they lie and shared, and nothing allocates.
#[test]
fn hovering_a_paragraph_of_spans_allocates_nothing() {
    fn label(r: usize) -> String {
        format!("plain bold {r}")
    }
    hover_rows_allocating_nothing(
        || {
            lit_rows(|r, _| {
                text("plain ")
                    .span(span("bold").font_weight(700))
                    .span(span(format!(" {r}")).underline(true))
            })
        },
        label,
    )
}

/// A span's colour while its row is hovered.
fn hover_red() -> Color {
    Color::hex(0x00ff_0000)
}

/// A span's colour while its row is not.
fn rest_blue() -> Color {
    Color::hex(0x0000_00ff)
}

/// The red and the blue pixels the GPU drew inside the paragraph showing
/// `label`, read back.
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a frame that cannot be read is a failed test"
)]
fn red_and_blue(s: &mut Scenario, label: &str) -> (usize, usize) {
    let b = s
        .last()
        .nodes
        .iter()
        .find(|n| matches!(&n.content, gui::layout::Content::Text { content, .. } if &**content == label))
        .map(|n| n.bounds(gui::inspect::OUTSIDE))
        .expect("the paragraph is a node");
    let shot = s.shot(false).unwrap();
    let scale = shot.scale();
    let logical = |v: u32| f32::from(u16::try_from(v).unwrap_or(u16::MAX)) / scale;
    let (mut red, mut blue) = (0_usize, 0_usize);
    for y in 0..shot.height() {
        for x in 0..shot.width() {
            if !b.contains(logical(x), logical(y)) {
                continue;
            }
            let Some([r, g, bl, _]) = shot.straight(x, y) else {
                continue;
            };
            if r > 150 && g < 90 && bl < 90 {
                red = red.saturating_add(1);
            } else if bl > 150 && r < 90 && g < 90 {
                blue = blue.saturating_add(1);
            }
        }
    }
    (red, blue)
}

/// The hovered row's span is drawn red and the other's blue, as the GPU
/// drew them: what is written in place is what paints.
fn spans_painted(s: &mut Scenario, label: fn(usize) -> String, third: bool) {
    let (on, off) = if third { (3, 4) } else { (4, 3) };
    let (red, blue) = red_and_blue(s, &label(on));
    assert!(
        red > 0 && blue == 0,
        "the hovered row's span is red: {red} red, {blue} blue"
    );
    let (red, blue) = red_and_blue(s, &label(off));
    assert!(
        red == 0 && blue > 0,
        "the row that lost hover's is blue: {red} red, {blue} blue"
    );
}

/// A paragraph whose span's colour follows its hover: each transition
/// resolves its runs again, into the ones it showed (taken back from the
/// frame node they were shared with), and nothing allocates. Each frame
/// draws the hovered row's span in its hover colour, the other's in its
/// resting one.
#[test]
fn hovering_a_paragraph_that_recolours_a_span_allocates_nothing() {
    fn label(r: usize) -> String {
        format!("plain lit {r}")
    }
    hover_rows_checking(
        || {
            lit_rows(|r, i| {
                text("plain ")
                    .span(span("lit").text_color(move || if i.hovered() { hover_red() } else { rest_blue() }))
                    .span(format!(" {r}"))
            })
        },
        label,
        |s, third| spans_painted(s, label, third),
    );
}

/// A paragraph whose link (a span that takes clicks) changes colour on
/// hover: its text lies where it did, so where its link lies is not
/// looked for again, and nothing allocates. Both the hovered row's link
/// and the one that just lost hover are drawn in their colours.
#[test]
fn hovering_a_paragraph_that_recolours_its_link_allocates_nothing() {
    fn label(r: usize) -> String {
        format!("go to link {r}")
    }
    hover_rows_checking(
        || {
            lit_rows(|r, i| {
                text("go to ")
                    .span(
                        span("link")
                            .on_click(|_| {})
                            .text_color(move || if i.hovered() { hover_red() } else { rest_blue() }),
                    )
                    .span(format!(" {r}"))
            })
        },
        label,
        |s, third| spans_painted(s, label, third),
    );
}

/// A paragraph whose span's colour a signal gives, written twice between
/// paints (as a frame's two layout passes may): the runs paint shares are
/// not written, two others are, by turns, and nothing allocates.
#[test]
fn runs_written_twice_before_each_paint_allocate_nothing() {
    use gui::layout::{Content, LayoutTree, UiState};
    gui::reactive::stop_trace();
    let lit = gui::reactive::Signal::new(false);
    let el = div()
        .child(
            text("plain ")
                .span(span("lit").text_color(move || if lit.get() { LIT } else { REST }))
                .span(" end"),
        )
        .into_element();
    let mut tree = LayoutTree::new();
    let mut ts = gui::text::TextSystem::new();
    let state = UiState::default();
    let mut motion = gui::Motion::default();
    // What paint's chunk holds: the runs of the frame it last recorded.
    let mut painted = None;
    let mut frame = |painted: &mut Option<std::rc::Rc<[gui::text::Run]>>| {
        for on in [true, false] {
            lit.set(on);
            tree.layout(&el, 300.0, 200.0, &state, &mut motion, &mut ts);
        }
        let runs = tree.frame().nodes.iter().find_map(|n| match &n.content {
            Content::Text { runs, .. } if !runs.is_empty() => Some(std::rc::Rc::clone(runs)),
            _ => None,
        });
        *painted = runs;
    };
    for _ in 0..8 {
        frame(&mut painted);
    }
    for step in 0..16 {
        let ((), heap) = gui::heap::measure(|| frame(&mut painted));
        let bytes = heap.map_or(u64::MAX, |t| t.allocated);
        assert_eq!(bytes, 0, "{bytes} bytes allocated writing the runs twice, frame {step}");
    }
    assert!(painted.is_some(), "the paragraph shows its runs");
}

/// A button in a paragraph clamped to one line, on the line the clamp
/// hides: it is neither painted nor clicked; on a line that shows, it is.
#[test]
fn a_button_on_a_line_a_clamp_hides_is_not_painted_or_clicked() {
    for clamp in [true, false] {
        let clicks = Rc::new(Cell::new(0_u32));
        let c = Rc::clone(&clicks);
        let ui = move || {
            let c = Rc::clone(&c);
            let p = text("words that fill the first line and wrap ")
                .child(
                    div()
                        .inline(true)
                        .padding_x(4.0)
                        .on_click(move |_| c.set(c.get() + 1))
                        .child(text("the button")),
                )
                .width(120.0);
            div().child(if clamp { p.line_clamp(1) } else { p }).into_element()
        };
        let mut s = Scenario::measuring(ui, 300, 200, 1.0).unwrap();
        s.settle().unwrap();
        let node = s
            .last()
            .nodes
            .iter()
            .find(|n| matches!(&n.content, gui::layout::Content::Text { content, .. } if &**content == "the button"))
            .cloned()
            .expect("the button's label is a node");
        let b = node.bounds(gui::inspect::OUTSIDE);
        s.click(b.x + b.width / 2.0, b.y + b.height / 2.0);
        s.settle().unwrap();
        if clamp {
            assert!(node.look.invisible, "a clamp hides the button's label");
            assert_eq!(clicks.get(), 0, "a hidden button takes no click");
        } else {
            assert!(!node.look.invisible, "unclamped, it shows");
            assert_eq!(clicks.get(), 1, "and takes the click");
        }
    }
}

/// A paragraph holding an inline icon and an inline box, hovered: its own
/// background resolves it again, its inline objects are unchanged and
/// compared where they lie, and nothing allocates.
#[test]
fn hovering_a_paragraph_with_inline_objects_allocates_nothing() {
    fn label(r: usize) -> String {
        let o = gui::text::OBJECT;
        format!("icon {o} box {o}{r}")
    }
    hover_rows_allocating_nothing(
        || {
            lit_rows(|r, _| {
                text("icon ")
                    .child(gui::icon(gui::Icon::Activity))
                    .span(" box ")
                    .child(div().inline(true).width(6.0).height(6.0))
                    .span(format!("{r}"))
            })
        },
        label,
    );
}

/// A row, and one unwrapped paragraph of about 50,000 glyphs whose
/// background follows the row's hover: each hover draws all of it again.
fn a_row_over_a_long_paragraph() -> gui::Element {
    let i = Interaction::new();
    let long = "Sphinx of black quartz, judge my vow; ".repeat(1_350);
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .child(
            div()
                .interaction(i)
                .height(24.0)
                .background(move || if i.hovered() { LIT } else { REST })
                .child(text("the row")),
        )
        .child(div().height(24.0).child(text("the rest")))
        .child(
            text(long)
                .nowrap(true)
                .underline(true)
                .background(move || if i.hovered() { REST } else { LIT }),
        )
        .into_element()
}

/// A paragraph of about 50,000 glyphs, underlined, drawn again each frame
/// (megabytes of glyphs and skip-ink gaps): once warm, no frame allocates
/// on the UI thread.
#[test]
fn a_long_paragraph_drawn_each_frame_allocates_nothing() {
    gui::testing::both_passes(long_paragraph);
}

#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn long_paragraph(pass: Pass) {
    let mut s = Scenario::measuring(a_row_over_a_long_paragraph, 300, 200, 1.0).unwrap();
    gui::reactive::stop_trace();
    pass.scenario(&mut s);
    s.settle().unwrap();
    let paint = s
        .owners()
        .into_iter()
        .find(|o| o.name == "paint")
        .map_or(0, |o| o.bytes);
    assert!(
        paint > (1 << 20),
        "its glyphs are megabytes of items: paint holds {paint}"
    );
    let (row, rest) = (s.locate("the row").unwrap(), s.locate("the rest").unwrap());
    for _ in 0..12 {
        for (x, y) in [row, rest] {
            s.hover(x, y);
            s.settle().unwrap();
            s.frame("warm").unwrap();
        }
    }
    for step in 0..12 {
        let (x, y) = if step % 2 == 0 { row } else { rest };
        s.hover(x, y);
        let r = s.frame(&format!("hover {step}")).unwrap();
        pass.measured(|| {
            assert!(
                r.paint.recorded >= 2,
                "the row and the paragraph record again: {}",
                r.paint
            );
            let bytes = |t: Option<gui::heap::ThreadHeap>| t.map_or(u64::MAX, |t| t.allocated);
            let recorded = bytes(r.footprint.recorded);
            assert_eq!(recorded, 0, "{recorded} bytes allocated recording hover {step}");
            // A hover resolves the paragraph again (its background binds) and
            // writes its frame node anew: both share the content and runs it
            // held, and layout copies nothing.
            let built = bytes(r.footprint.built);
            assert_eq!(built, 0, "{built} bytes allocated building hover {step}");
            // The whole frame on the UI thread: layout, the text drawn again,
            // the commit, the frame's end and its report.
            let ui = bytes(r.footprint.thread).saturating_sub(bytes(r.footprint.committed));
            assert_eq!(ui, 0, "{ui} bytes allocated on the UI thread, hover {step}");
        });
    }
}

/// The button of [`a_button_a_clamp_hides_and_shows_again_on_one_tree`]
/// in the accessibility tree: the node a screen reader can press.
fn button_in_access_tree(s: &mut Scenario) -> Option<accesskit::NodeId> {
    let mut ts = gui::text::TextSystem::new();
    let focus = s.read(gui::focus::focused);
    let tree = gui::access::tree(s.last(), focus, 1.0, "test", &mut ts);
    let pressable: Vec<_> = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.supports_action(accesskit::Action::Click))
        .map(|(id, _)| *id)
        .collect();
    assert!(pressable.len() <= 1, "one pressable node at most: {pressable:?}");
    pressable.first().copied()
}

/// One tree, a button in a paragraph clamped to one line: shown, then
/// moved to the line the clamp hides (the paragraph narrowed), then shown
/// again. Hidden, it loses focus (Chrome's focus fixup), leaves the
/// accessibility tree, and neither Enter nor a screen reader's press
/// queued for its old node runs it; shown again, it is back in the tree
/// and takes a click.
#[test]
fn a_button_a_clamp_hides_and_shows_again_on_one_tree() {
    let clicks = Rc::new(Cell::new(0_u32));
    let narrow = gui::reactive::Signal::new(false);
    let focus = gui::FocusHandle::new();
    let c = Rc::clone(&clicks);
    let ui = move || {
        let c = Rc::clone(&c);
        div()
            .child(
                text("words before ")
                    .child(
                        div()
                            .inline(true)
                            .padding_x(4.0)
                            .track_focus(focus)
                            .on_click(move |_| c.set(c.get() + 1))
                            .child(text("the button")),
                    )
                    .line_clamp(1)
                    .width(move || if narrow.get() { 100.0 } else { 400.0 }),
            )
            .into_element()
    };
    let mut s = Scenario::measuring(ui, 500, 200, 1.0).unwrap();
    s.settle().unwrap();
    let label = |s: &Scenario| {
        s.last()
            .nodes
            .iter()
            .find(|n| matches!(&n.content, gui::layout::Content::Text { content, .. } if &**content == "the button"))
            .cloned()
            .expect("the button's label is a node")
    };
    let click = |s: &mut Scenario| {
        let b = label(s).bounds(gui::inspect::OUTSIDE);
        s.click(b.x + b.width / 2.0, b.y + b.height / 2.0);
        s.settle().unwrap();
    };
    // Shown.
    assert!(!label(&s).look.invisible, "on the first line it shows");
    let id = button_in_access_tree(&mut s).expect("a screen reader finds the button");
    click(&mut s);
    assert_eq!(clicks.get(), 1, "and takes a click");
    s.act("focus", || focus.focus());
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), Some(focus.id()), "it holds focus");
    // Hidden.
    s.act("narrow", || narrow.set(true));
    s.settle().unwrap();
    assert!(label(&s).look.invisible, "on the clamped line it is hidden");
    assert_eq!(s.read(gui::focus::focused), None, "and loses focus");
    assert_eq!(button_in_access_tree(&mut s), None, "and leaves the accessibility tree");
    s.press(NamedKey::Enter, Modifiers::default());
    s.settle().unwrap();
    assert_eq!(clicks.get(), 1, "Enter does not press it");
    s.send(gui::window::Input::Act(accesskit::ActionRequest {
        action: accesskit::Action::Click,
        target_node: id,
        target_tree: accesskit::TreeId::ROOT,
        data: None,
    }));
    s.settle().unwrap();
    assert_eq!(clicks.get(), 1, "nor does a press queued for its old node");
    click(&mut s);
    assert_eq!(clicks.get(), 1, "nor a click where it was");
    // Shown again.
    s.act("widen", || narrow.set(false));
    s.settle().unwrap();
    assert!(!label(&s).look.invisible, "back on the first line it shows");
    assert_eq!(
        button_in_access_tree(&mut s),
        Some(id),
        "back in the accessibility tree"
    );
    click(&mut s);
    assert_eq!(clicks.get(), 2, "and takes a click again");
}

/// A focusable 120x30 button showing `name`, counting its clicks.
fn focus_button(name: &'static str, focus: gui::FocusHandle, clicks: &Rc<Cell<u32>>) -> gui::Div {
    let clicks = Rc::clone(clicks);
    div()
        .track_focus(focus)
        .width(120.0)
        .height(30.0)
        .child(text(name))
        .on_click(move |_| clicks.set(clicks.get().saturating_add(1)))
}

/// A trapped modal dialog opened from a trigger, holding D1 and D2; `hide`
/// hides D2, `gone` the whole dialog (both `invisible`).
#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn hiding_dialog(
    open: gui::Signal<bool>,
    hide: gui::Signal<bool>,
    gone: gui::Signal<bool>,
    [opener, d1, d2]: [gui::FocusHandle; 3],
) -> Scenario {
    let clicks = Rc::new(Cell::new(0_u32));
    let mut s = Scenario::measuring(
        move || {
            let d = open.get().then(|| {
                let layer = gui::Layer::new()
                    .modal(true)
                    .dismiss(gui::DismissPolicy::DEFAULT.focus_outside(false))
                    .focus_scope(gui::FocusScope::trapped());
                let region = div()
                    .layer(layer)
                    .invisible(move || gone.get())
                    .child(focus_button("D1", d1, &clicks))
                    .child(focus_button("D2", d2, &clicks).invisible(move || hide.get()));
                gui::deferred(gui::Positioning::At { x: 300.0, y: 100.0 }, region)
            });
            div()
                .child(focus_button("Open", opener, &clicks).on_click(move |_| open.set(true)))
                .child(d)
                .into_element()
        },
        800,
        600,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s
}

/// A focused node hidden inside a trapped dialog is as one removed: the
/// trap takes focus back inside (its region), rather than focus being
/// dropped before the trap could see it.
#[test]
fn hiding_the_focused_node_of_a_trapped_dialog_keeps_focus_inside() {
    let (open, hide, gone) = (
        gui::Signal::new(false),
        gui::Signal::new(false),
        gui::Signal::new(false),
    );
    let f = [
        gui::FocusHandle::new(),
        gui::FocusHandle::new(),
        gui::FocusHandle::new(),
    ];
    let mut s = hiding_dialog(open, hide, gone, f);
    s.act("open", || open.set(true));
    s.settle().unwrap();
    s.act("focus D2", || f[2].focus());
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), Some(f[2].id()), "D2 holds focus");
    s.act("hide D2", || hide.set(true));
    s.settle().unwrap();
    // The trap's rule for removed content: its region takes focus.
    assert_eq!(
        s.read(gui::focus::focused),
        Some(region(&mut s, 0)),
        "the dialog's region holds focus"
    );
}

/// The focus id of the region of layer `layer` in the last frame: a
/// path's id is its window's, so it is asked in a turn of that window.
#[expect(clippy::expect_used, reason = "a layer not open is a failed test")]
fn region(s: &mut Scenario, layer: usize) -> gui::focus::FocusId {
    let f = s.last();
    let root = f.layers.get(layer).expect("the layer is open").root;
    let path = f.nodes.get(root).expect("its root is a node").path.clone();
    s.read(|| gui::FocusHandle::for_path(&path).id())
}

/// A focused node hidden with its dialog in one pass: the dialog closes
/// as one removed does, and focus goes back to its trigger.
#[test]
fn hiding_a_dialog_with_its_focused_node_gives_focus_back_to_its_trigger() {
    let (open, hide, gone) = (
        gui::Signal::new(false),
        gui::Signal::new(false),
        gui::Signal::new(false),
    );
    let f = [
        gui::FocusHandle::new(),
        gui::FocusHandle::new(),
        gui::FocusHandle::new(),
    ];
    let mut s = hiding_dialog(open, hide, gone, f);
    s.act("focus the trigger", || f[0].focus());
    s.settle().unwrap();
    s.act("open", || open.set(true));
    s.settle().unwrap();
    s.act("focus D2", || f[2].focus());
    s.settle().unwrap();
    s.act("hide all", || {
        hide.set(true);
        gone.set(true);
    });
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), Some(f[0].id()), "back to the trigger");
}

/// Code focusing a hidden node while a shown one holds focus does nothing,
/// as `focus()` on an element not rendered does nothing in Chrome: the
/// shown one keeps focus, and what watches it blur sees nothing.
#[test]
fn focusing_a_hidden_node_keeps_focus_where_it_is() {
    let blurs = Rc::new(Cell::new(0_u32));
    let (shown, hidden) = (gui::FocusHandle::new(), gui::FocusHandle::new());
    let b = Rc::clone(&blurs);
    let mut s = Scenario::measuring(
        move || {
            let b = Rc::clone(&b);
            let clicks = Rc::new(Cell::new(0));
            // What watches the shown one blur: a binding of its own.
            let watch = move || {
                if !shown.is_focused() {
                    b.set(b.get() + 1);
                }
                REST
            };
            div()
                .child(focus_button("shown", shown, &clicks).background(watch))
                .child(focus_button("hidden", hidden, &clicks).invisible(true))
                .into_element()
        },
        400,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus the shown one", || shown.focus());
    s.settle().unwrap();
    let before = blurs.get();
    s.act("focus the hidden one", || hidden.focus());
    s.settle().unwrap();
    assert_eq!(
        s.read(gui::focus::focused),
        Some(shown.id()),
        "the shown one keeps focus"
    );
    assert_eq!(blurs.get(), before, "it never blurred");
}

/// A button its paragraph's clamp hides, under a root shown, takes no
/// window key: its shortcut does not run.
#[test]
fn a_hidden_node_takes_no_key() {
    let shortcut = Rc::new(Cell::new(0_u32));
    let sc = Rc::clone(&shortcut);
    let mut s = Scenario::measuring(
        move || {
            let sc = Rc::clone(&sc);
            div()
                .child(
                    text("words that fill the first line and wrap ")
                        .child(
                            div()
                                .inline(true)
                                .on_window_key(move |_| {
                                    sc.set(sc.get() + 1);
                                    true
                                })
                                .child(text("the button")),
                        )
                        .line_clamp(1)
                        .width(120.0),
                )
                .into_element()
        },
        400,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.press(NamedKey::Enter, Modifiers::default());
    s.settle().unwrap();
    assert_eq!(shortcut.get(), 0, "a hidden button's shortcut does not run");
}

/// With nothing focused, a key goes to the top root shown: a deferred
/// root above it that is hidden does not swallow it.
#[test]
fn a_hidden_deferred_root_leaves_the_key_to_the_root_below() {
    let (main_keys, layer_keys) = (Rc::new(Cell::new(0_u32)), Rc::new(Cell::new(0_u32)));
    let (mk, lk) = (Rc::clone(&main_keys), Rc::clone(&layer_keys));
    let mut s = Scenario::measuring(
        move || {
            let (mk, lk) = (Rc::clone(&mk), Rc::clone(&lk));
            let hidden = div()
                .layer(gui::Layer::new())
                .invisible(true)
                .width(50.0)
                .height(50.0)
                .on_key_down(move |_| {
                    lk.set(lk.get() + 1);
                    true
                });
            div()
                .on_key_down(move |_| {
                    mk.set(mk.get() + 1);
                    true
                })
                .child(text("page"))
                .child(gui::deferred(gui::Positioning::At { x: 10.0, y: 10.0 }, hidden))
                .into_element()
        },
        400,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), None);
    s.press(NamedKey::Enter, Modifiers::default());
    s.settle().unwrap();
    assert_eq!(layer_keys.get(), 0, "the hidden root takes no key");
    assert_eq!(main_keys.get(), 1, "the root shown below takes it");
}

/// Two buttons, the second `inert` while `inert` holds.
#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn inert_pair(inert: gui::Signal<bool>, [a, b]: [gui::FocusHandle; 2]) -> Scenario {
    let clicks = Rc::new(Cell::new(0_u32));
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(focus_button("A", a, &clicks))
                .child(focus_button("B", b, &clicks).inert(inert.get()))
                .into_element()
        },
        400,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s
}

/// A focused node made inert outside any scope loses focus, as one hidden.
#[test]
fn a_focused_node_made_inert_loses_focus() {
    let inert = gui::Signal::new(false);
    let f = [gui::FocusHandle::new(), gui::FocusHandle::new()];
    let mut s = inert_pair(inert, f);
    s.act("focus B", || f[1].focus());
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), Some(f[1].id()));
    s.act("make B inert", || inert.set(true));
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), None, "inert, it holds no focus");
}

/// Code focusing an inert node does not take focus from a shown one.
#[test]
fn focusing_an_inert_node_keeps_focus_where_it_is() {
    let inert = gui::Signal::new(true);
    let f = [gui::FocusHandle::new(), gui::FocusHandle::new()];
    let mut s = inert_pair(inert, f);
    s.act("focus A", || f[0].focus());
    s.settle().unwrap();
    s.act("focus the inert B", || f[1].focus());
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), Some(f[0].id()), "A keeps focus");
}

/// What runs after layout sees the frame just laid out: a button that
/// pass revealed takes the focus asked for it.
#[test]
fn a_node_revealed_takes_focus_asked_after_layout() {
    let hidden = gui::Signal::new(true);
    let f = [gui::FocusHandle::new(), gui::FocusHandle::new()];
    let clicks = Rc::new(Cell::new(0_u32));
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(focus_button("A", f[0], &clicks))
                .child(focus_button("B", f[1], &clicks).invisible(move || hidden.get()))
                .into_element()
        },
        400,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    // The work is a part's: queued in the scope it lives in.
    let part = gui::Scope::new();
    s.act("reveal B and focus it after layout", || {
        hidden.set(false);
        part.run(|| gui::after_layout(move || f[1].focus()));
    });
    s.settle().unwrap();
    assert_eq!(
        s.read(gui::focus::focused),
        Some(f[1].id()),
        "the revealed B holds focus"
    );
}

/// A focused node removed outside any scope leaves no id behind: laid out
/// again, it does not take focus back unasked.
#[test]
fn a_focused_node_removed_and_laid_out_again_is_not_focused() {
    let shown = gui::Signal::new(true);
    let h = gui::FocusHandle::new();
    let clicks = Rc::new(Cell::new(0_u32));
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(text("page"))
                .child(shown.get().then(|| focus_button("B", h, &clicks)))
                .into_element()
        },
        400,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus B", || h.focus());
    s.settle().unwrap();
    s.act("remove B", || shown.set(false));
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), None, "removed, nothing holds focus");
    s.act("lay B out again", || shown.set(true));
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), None, "and B does not take it back");
}

/// An inner trapped dialog removed with its opener while the outer one
/// stays open: nowhere to give focus back to, the outer trap takes it
/// back inside (its region).
#[test]
fn an_inner_dialog_gone_with_its_opener_leaves_focus_in_the_outer() {
    let (outer, inner) = (gui::Signal::new(false), gui::Signal::new(false));
    let f = [
        gui::FocusHandle::new(),
        gui::FocusHandle::new(),
        gui::FocusHandle::new(),
    ];
    let clicks = Rc::new(Cell::new(0_u32));
    let trapped = || {
        gui::Layer::new()
            .modal(true)
            .dismiss(gui::DismissPolicy::DEFAULT.focus_outside(false))
            .focus_scope(gui::FocusScope::trapped())
    };
    let mut s = Scenario::measuring(
        move || {
            let d = outer.get().then(|| {
                // The opener and its dialog, removed together.
                let opening = inner.get().then(|| {
                    let dialog = div().layer(trapped()).child(focus_button("I1", f[2], &clicks));
                    div()
                        .child(focus_button("Opener", f[1], &clicks))
                        .child(gui::deferred(gui::Positioning::At { x: 400.0, y: 300.0 }, dialog))
                });
                let region = div()
                    .layer(trapped())
                    .child(focus_button("O1", f[0], &clicks))
                    .child(opening);
                gui::deferred(gui::Positioning::At { x: 100.0, y: 100.0 }, region)
            });
            div().child(text("page")).child(d).into_element()
        },
        800,
        600,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("open both", || {
        outer.set(true);
        inner.set(true);
    });
    s.settle().unwrap();
    s.act("focus I1", || f[2].focus());
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), Some(f[2].id()));
    s.act("remove the inner with its opener", || inner.set(false));
    s.settle().unwrap();
    assert_eq!(
        s.read(gui::focus::focused),
        Some(region(&mut s, 0)),
        "the outer's region holds focus"
    );
}

/// A list's members ([`gui::Members`]) are the members shown: one
/// `invisible`, one its paragraph's clamp hides, are not among them, so
/// arrow keys and typeahead never reach them; shown again, they are.
#[test]
fn a_list_holds_only_its_members_that_are_shown() {
    let list: gui::Members<u32> = gui::Members::new();
    let hide = gui::reactive::Signal::new(true);
    let l = list.clone();
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(div().member(&l, 1).child(text("one")))
                .child(div().member(&l, 2).invisible(move || hide.get()).child(text("two")))
                .child(
                    text("words that fill the first line and wrap ")
                        .child(div().inline(true).member(&l, 3).child(text("three")))
                        .width(move || if hide.get() { 120.0 } else { 600.0 })
                        .line_clamp(1),
                )
                .child(div().member(&l, 4).child(text("four")))
                .into_element()
        },
        700,
        300,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    assert_eq!(list.get(), [1, 4], "the hidden members are not in the list");
    s.act("show", || hide.set(false));
    s.settle().unwrap();
    assert_eq!(list.get(), [1, 2, 3, 4], "shown, they are, in tree order");
}

/// A modal dialog opened from a node that goes to sleep sleeps with it: it
/// is not shown, takes no key and blocks nothing below it.
#[test]
fn a_dialog_opened_from_a_dormant_node_sleeps_with_it() {
    let sleep = gui::Signal::new(false);
    let (page, keys) = (Rc::new(Cell::new(0_u32)), Rc::new(Cell::new(0_u32)));
    let (p, k) = (Rc::clone(&page), Rc::clone(&keys));
    let inside = gui::FocusHandle::new();
    let mut s = Scenario::measuring(
        move || {
            let k = Rc::clone(&k);
            let clicks = Rc::new(Cell::new(0_u32));
            let region = div()
                .layer(
                    gui::Layer::new()
                        .modal(true)
                        .dismiss(gui::DismissPolicy::DEFAULT.focus_outside(false))
                        .focus_scope(gui::FocusScope::trapped()),
                )
                .on_key_down(move |_| {
                    k.set(k.get() + 1);
                    true
                })
                .child(focus_button("Inside", inside, &clicks));
            div()
                .child(focus_button("Page", gui::FocusHandle::new(), &p))
                .child(
                    div()
                        .dormant(sleep.get())
                        .child(gui::deferred(gui::Positioning::At { x: 300.0, y: 200.0 }, region)),
                )
                .into_element()
        },
        800,
        600,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    assert_eq!(
        s.read(gui::focus::focused),
        Some(inside.id()),
        "awake, the dialog takes focus"
    );
    s.act("sleep", || sleep.set(true));
    s.settle().unwrap();
    let root = s.last().layers.first().expect("the layer is laid out").root;
    assert!(s.last().nodes[root].look.invisible, "its region is not shown");
    assert_ne!(s.read(gui::focus::focused), Some(inside.id()), "nor holds focus");
    s.press(NamedKey::Enter, Modifiers::default());
    s.settle().unwrap();
    assert_eq!(keys.get(), 0, "it takes no key");
    s.click(60.0, 15.0);
    s.settle().unwrap();
    assert_eq!(page.get(), 1, "and blocks nothing below it");
}

/// Two windows on one thread keep their lists and what they hide apart:
/// laying one out leaves the other's as it was.
#[test]
fn two_trees_on_one_thread_keep_their_lists_and_hidden_focus_apart() {
    let list: gui::Members<u32> = gui::Members::new();
    let (shown, hidden) = (gui::FocusHandle::new(), gui::FocusHandle::new());
    let l = list.clone();
    let mut a = Scenario::measuring(
        move || {
            let clicks = Rc::new(Cell::new(0_u32));
            div()
                .child(focus_button("shown", shown, &clicks).member(&l, 1))
                .child(focus_button("hidden", hidden, &clicks).invisible(true))
                .into_element()
        },
        400,
        200,
        1.0,
    )
    .unwrap();
    a.settle().unwrap();
    assert_eq!(list.get(), [1]);
    let mut b = Scenario::measuring(|| div().child(text("another window")).into_element(), 400, 200, 1.0).unwrap();
    b.settle().unwrap();
    assert_eq!(list.get(), [1], "the other tree's pass keeps this tree's list");
    a.act("focus the shown one", || shown.focus());
    a.act("focus the hidden one", || hidden.focus());
    assert_eq!(
        a.read(gui::focus::focused),
        Some(shown.id()),
        "this tree still hides it"
    );
}

/// A list's watcher runs on the frame as it is: a member that pass
/// revealed takes the focus the watcher asks for it.
#[test]
fn a_list_watcher_focuses_a_member_revealed_that_pass() {
    let list: gui::Members<u32> = gui::Members::new();
    let hidden = gui::Signal::new(true);
    let f = [gui::FocusHandle::new(), gui::FocusHandle::new()];
    let l = list.clone();
    let mut s = Scenario::measuring(
        move || {
            let clicks = Rc::new(Cell::new(0_u32));
            div()
                .child(focus_button("A", f[0], &clicks).member(&l, 1))
                .child(
                    focus_button("B", f[1], &clicks)
                        .member(&l, 2)
                        .invisible(move || hidden.get()),
                )
                .into_element()
        },
        400,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    // It asks once, the first pass B is a member.
    let asked = Rc::new(Cell::new(false));
    list.on_laid_out(move |v| {
        if v.contains(&2) && !asked.replace(true) {
            f[1].focus();
        }
    });
    s.act("reveal B", || hidden.set(false));
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), Some(f[1].id()), "the watcher focused B");
}

/// A pass that hides the focused node, and a hook after it that shows it
/// again in the next pass of the same frame: focus stays on it, as it is
/// dropped only once the frame's last pass is laid out.
#[test]
fn a_node_hidden_and_shown_again_within_a_frame_keeps_focus() {
    let hidden = gui::Signal::new(false);
    let f = [gui::FocusHandle::new(), gui::FocusHandle::new()];
    let clicks = Rc::new(Cell::new(0_u32));
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(focus_button("A", f[0], &clicks))
                .child(focus_button("B", f[1], &clicks).invisible(move || hidden.get()))
                .into_element()
        },
        400,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus B", || f[1].focus());
    s.settle().unwrap();
    // The work is a part's: queued in the scope it lives in.
    let part = gui::Scope::new();
    s.act("hide B, and show it after layout", || {
        hidden.set(true);
        part.run(|| gui::after_layout(move || hidden.set(false)));
    });
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), Some(f[1].id()), "B keeps focus");
}

/// A node made inert with nothing else changing (no box moves) is known
/// hidden at once: focus asked for it leaves the shown one focused.
#[test]
fn focusing_a_node_just_made_inert_keeps_focus_where_it_is() {
    let inert = gui::Signal::new(false);
    let f = [gui::FocusHandle::new(), gui::FocusHandle::new()];
    let mut s = inert_pair(inert, f);
    s.act("focus A", || f[0].focus());
    s.settle().unwrap();
    s.act("make B inert", || inert.set(true));
    s.settle().unwrap();
    s.act("focus B", || f[1].focus());
    s.settle().unwrap();
    assert_eq!(s.read(gui::focus::focused), Some(f[0].id()), "A keeps focus");
}

/// Focus is each window's own: laying out another window, whose node at
/// the same place is hidden, leaves this window's focus where it is, and
/// the same place in two windows is two nodes.
#[test]
fn laying_out_another_window_keeps_this_window_s_focus() {
    let ui = |hidden: bool| {
        move || {
            div()
                .child(
                    div()
                        .focusable()
                        .width(120.0)
                        .height(30.0)
                        .invisible(hidden)
                        .child(text("node")),
                )
                .into_element()
        }
    };
    let mut a = Scenario::measuring(ui(false), 400, 200, 1.0).unwrap();
    a.settle().unwrap();
    a.click(60.0, 15.0);
    a.settle().unwrap();
    let held = a.read(gui::focus::focused);
    assert!(held.is_some(), "the node takes focus in the first window");
    let mut b = Scenario::measuring(ui(true), 400, 200, 1.0).unwrap();
    b.settle().unwrap();
    assert_eq!(b.read(gui::focus::focused), None, "the second window holds none");
    a.settle().unwrap();
    assert_eq!(a.read(gui::focus::focused), held, "the first window keeps its focus");
}

/// The cold check lays the frame out again on a tree of its own: the
/// window's lists stay as the window's tree filled them.
#[test]
fn a_cold_check_leaves_the_window_s_lists_alone() {
    let list: gui::Members<u32> = gui::Members::new();
    let l = list.clone();
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(div().member(&l, 1).child(text("one")))
                .child(div().member(&l, 2).child(text("two")))
                .into_element()
        },
        400,
        200,
        1.0,
    )
    .unwrap();
    s.check_cold(true);
    s.act("again", || {});
    s.frame("checked cold").unwrap();
    assert_eq!(list.get(), [1, 2], "the window's list holds its members");
}

/// A focused field's caret blinks on and off across frames that change
/// nothing else. Each frame paints as a cold one does (the cold check is
/// on): a blink re-records the content chunk that draws the caret, the one
/// a blink is checked against too, so it never keeps a caret that went out.
#[test]
fn a_blinking_caret_paints_as_a_cold_frame_does_through_every_blink() {
    let field = gui::TextField::new("abc");
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .padding(10.0)
                .child(gui::text_input(&f).width(80.0))
                .into_element()
        },
        200,
        60,
        1.0,
    )
    .unwrap();
    s.check_cold(true);
    s.settle().unwrap();
    let (x, y) = s.locate("abc").unwrap();
    s.click(x, y);
    s.settle().unwrap();
    // Two of the blink's half periods (0.53 s) do not divide 0.2 s evenly:
    // frames land on both phases, and on each change of phase.
    for _ in 0..12 {
        s.wait(std::time::Duration::from_millis(200));
        s.frame("a blink").unwrap();
    }
}
