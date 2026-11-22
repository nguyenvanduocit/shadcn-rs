//! A node's `Interaction` (ADR-0022): one signal per flag, written by hit
//! testing and focus only when it changes, read by bindings with tracking
//! per flag. Hovering a part reruns that node's binding alone: no instance
//! renders, nothing lays out, a binding of the node that reads another
//! flag does not run, and the UI thread allocates nothing taking the
//! pointer in, nor building and laying out the frame; recording the two
//! chunks again allocates the same whatever the size of the tree.

use std::cell::Cell;
use std::rc::Rc;

use gui::app::Scenario;
use gui::events::{Modifiers, NamedKey};
use gui::prelude::*;
use gui::{Color, Interaction, div, text};

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

/// What one hover frame allocated on the UI thread, by part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HoverBytes {
    /// Recording the two chunks again and the commit carrying them.
    recorded: u64,
    /// The rest of the frame beyond input, build, layout and recording:
    /// the frame report's description of the two nodes that changed.
    reported: u64,
}

/// Hover rows 7, 8 and 9 in turn over a column of `n` rows; what each
/// hover frame allocated on the UI thread past its layout.
#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn hover_rows(n: usize) -> Vec<HoverBytes> {
    let runs = Rc::new(Cell::new(0));
    let probe = Rc::clone(&runs);
    let mut s = Scenario::measuring(move || rows(n, Rc::clone(&probe)), 300, 700, 1.0).unwrap();
    // The reactive trace a scenario keeps for its reports records each
    // write: an app's frames keep none.
    gui::reactive::stop_trace();
    s.check_cold(false);
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
    let mut out = Vec::new();
    for (step, row) in ["row 7", "row 8", "row 7", "row 8", "row 9", "row 8", "row 7"]
        .into_iter()
        .enumerate()
    {
        let (x, y) = s.locate(row).unwrap();
        let ((), input) = gui::heap::measure(|| s.hover(x, y));
        let taken = input.map_or(u64::MAX, |t| t.allocated);
        assert_eq!(taken, 0, "{taken} bytes allocated taking hover {step} in");
        let r = s.frame(&format!("hover {step}")).unwrap();
        assert!(r.rendered.is_empty(), "no component renders: {:?}", r.rendered);
        assert_eq!(r.layout.calls, 0, "nothing lays out");
        assert_eq!(
            r.visits.resolved, 2,
            "the row the pointer left and the row it came to resolve: {}",
            r.visits
        );
        assert_eq!(r.bindings, 2, "their bindings alone run");
        assert_eq!(r.paint.recorded, 2, "their chunks alone record again: {}", r.paint);
        // From the frame's start to its paint: the input state taken in,
        // the bindings run, the tree laid out.
        let bytes = |t: Option<gui::heap::ThreadHeap>| t.map_or(u64::MAX, |t| t.allocated);
        let built = bytes(r.footprint.built);
        assert_eq!(built, 0, "{built} bytes allocated building and laying out hover {step}");
        // The headless target composes and draws the commit on this
        // thread: in an app that is the render thread's.
        let (thread, recorded, committed) = (
            bytes(r.footprint.thread),
            bytes(r.footprint.recorded),
            bytes(r.footprint.committed),
        );
        let reported = thread.saturating_sub(recorded).saturating_sub(committed);
        out.push(HoverBytes { recorded, reported });
    }
    // Row 7's ring reads only `focus_visible`: its node resolved again on
    // each hover, and the ring replayed from what it last gave.
    assert_eq!(runs.get(), before, "the binding that reads focus_visible never ran");
    let frame = s.frame("rest").unwrap();
    assert_eq!(frame.visits.resolved, 0, "a steady frame resolves nothing");
    out
}

/// Hovering a part: no instance renders, nothing lays out, only the two
/// rows' paint bindings run, and nothing is allocated taking the pointer
/// in, running the bindings or laying out. What the frame allocates past
/// that is what any repaint of two chunks allocates (their display lists,
/// handed to the render thread in the commit, and the report naming the
/// two nodes): the same on every hover, whatever the number of rows.
#[test]
fn hovering_a_part_reruns_only_its_paint_binding() {
    let few = hover_rows(30);
    let many = hover_rows(600);
    let first = few.first().copied().expect("hover frames");
    assert!(
        few.iter().chain(&many).all(|b| *b == first),
        "each hover frame allocates what two chunks recorded again do, whatever the rows: {few:?} {many:?}"
    );
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
            keep.set(Some(i));
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
                        .background(move || {
                            h.set(h.get().saturating_add(1));
                            if i.hovered() { LIT } else { REST }
                        })
                        .ring(move || {
                            f.set(f.get().saturating_add(1));
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
