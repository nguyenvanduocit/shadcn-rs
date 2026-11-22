//! ADR-0013's dirtiness per node and phase, end to end: a signal read in one
//! phase of one node redoes that phase of that node, and the passes after it
//! visit only the paths that lead to it.
//!
//! The cold check (`GUI_COLD_CHECK`) renders every instance again each
//! frame, which is work these tests count: they measure with it off.

use crate::app::Scenario;
use crate::color::Color;
use crate::element::{IntoElement, div, text, text_bound};
use crate::reactive::{self, Phase, RenderNode, Signal};
use crate::styled::Styled;

/// A 50 x 50 grid of buttons, each lit on hover, as the stress `grid` scene.
fn grid(n: usize) -> crate::element::Element {
    div()
        .flex_col()
        .children((0..n).map(|r| {
            div().flex_row().children((0..n).map(move |c| {
                div()
                    .w_px(12.0)
                    .h_px(8.0)
                    .bg(Color::hex(0xf4f4f5))
                    .hover(|s| s.bg(Color::hex(0x18181b)))
                    .on_click(|_| {})
                    .child(text(format!("{r},{c}")).text_size(4.0, 6.0))
            }))
        }))
        .into_element()
}

#[test]
fn a_steady_frame_visits_nothing() {
    let mut s = Scenario::measuring(|| grid(50), 800, 600, 1.0).unwrap();
    s.check_cold(false);
    s.settle().unwrap();
    s.frame("warm").unwrap();
    let r = s.frame("steady").unwrap();
    assert_eq!(
        r.visits.resolved + r.visits.walked + r.visits.placed + r.visits.rebuilt,
        0,
        "{}",
        r.visits
    );
    assert_eq!(r.layout.calls, 0, "nothing is laid out");
    assert_eq!(r.paint.visited, 0, "the painter visits no node: {}", r.paint);
    assert_eq!(r.paint.recorded, 0);
    assert!(r.rendered.is_empty());
}

#[test]
fn a_hover_visits_the_path_to_the_node_it_changed() {
    let mut s = Scenario::measuring(|| grid(50), 800, 600, 1.0).unwrap();
    s.check_cold(false);
    s.settle().unwrap();
    s.hover(6.0, 4.0);
    s.settle().unwrap();
    s.take_reports();
    // From the button at row 0, column 0 to the one at row 30, column 20.
    s.hover(20.0 * 12.0 + 6.0, 30.0 * 8.0 + 4.0);
    let r = s.frame("moved").unwrap();
    assert_eq!(
        r.visits.resolved, 2,
        "the button left and the button entered: {}",
        r.visits
    );
    assert!(r.visits.walked <= 8, "the paths to them (depth 3): {}", r.visits);
    assert!(r.visits.placed <= 4, "{}", r.visits);
    assert_eq!(r.visits.rebuilt, 0);
    assert_eq!(r.layout.misses, 0, "a colour lays out nothing");
    assert_eq!(r.paint.recorded, 2, "the two buttons record again: {}", r.paint);
    assert!(r.paint.visited <= 4, "and the painter visits them alone: {}", r.paint);
}

#[test]
fn a_paint_binding_records_its_chunk_alone() {
    let colour = Signal::new(Color::hex(0xf4f4f5));
    let mut s = Scenario::measuring(
        move || {
            div()
                .flex_col()
                .children((0..40).map(move |i| {
                    let row = div().h_px(10.0).child(text(format!("row {i}")));
                    if i == 17 {
                        row.bind(move |st| st.bg(colour.get()))
                    } else {
                        row
                    }
                }))
                .into_element()
        },
        300,
        500,
        1.0,
    )
    .unwrap();
    s.check_cold(false);
    s.settle().unwrap();
    s.take_reports();
    s.act("colour", || colour.set(Color::hex(0x18181b)));
    let r = s.frame("coloured").unwrap();
    assert!(r.rendered.is_empty(), "no component renders: {:?}", r.rendered);
    assert_eq!(r.visits.resolved, 1, "the bound node alone resolves: {}", r.visits);
    assert!(r.visits.walked <= 2, "on the path to it: {}", r.visits);
    assert_eq!(r.visits.placed, 1, "{}", r.visits);
    assert_eq!(r.layout.misses, 0, "a colour lays out nothing");
    assert_eq!(r.paint.recorded, 1, "its chunk alone records: {}", r.paint);
    assert_eq!(r.paint.visited, 1, "and no clean subtree is walked: {}", r.paint);
}

#[test]
fn a_layout_binding_lays_out_and_renders_nothing() {
    let width = Signal::new(40.0_f32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .flex_col()
                .children((0..40).map(move |i| {
                    let row = div().h_px(10.0).child(text(format!("row {i}")));
                    if i == 17 {
                        row.bind(move |st| st.w_px(width.get()))
                    } else {
                        row
                    }
                }))
                .into_element()
        },
        300,
        500,
        1.0,
    )
    .unwrap();
    s.check_cold(false);
    s.settle().unwrap();
    s.take_reports();
    s.act("width", || width.set(80.0));
    let r = s.frame("wider").unwrap();
    assert!(r.rendered.is_empty(), "no component renders: {:?}", r.rendered);
    assert_eq!(r.visits.resolved, 1, "the bound node alone resolves: {}", r.visits);
    assert!(r.layout.misses > 0, "and is laid out again: {:?}", r.layout);
    assert!(r.visits.placed <= 3, "the node and what its size moves: {}", r.visits);
    assert!(r.paint.recorded <= 2, "{}", r.paint);
    let node = crate::inspect::find_text(&s.pipeline().frame().nodes, "row 17").unwrap();
    let parent = node.parent.unwrap();
    assert_eq!(s.pipeline().frame().nodes[parent].bounds.width, 80.0);
}

#[test]
fn accessibility_sends_and_reads_only_the_node_that_changed() {
    let label = Signal::new("row 17".to_owned());
    let mut s = Scenario::measuring(
        move || {
            div()
                .flex_col()
                .children((0..200).map(move |i| {
                    let row = div().h_px(10.0).on_click(|_| {});
                    if i == 17 {
                        row.child(text_bound(move || label.get()))
                    } else {
                        row.child(text(format!("row {i}")))
                    }
                }))
                .into_element()
        },
        300,
        2400,
        1.0,
    )
    .unwrap();
    s.check_cold(false);
    s.settle().unwrap();
    let whole = s.pipeline_mut().access_update(1.0, "page");
    assert!(whole.nodes.len() > 200);
    s.act("label", || label.set("row seventeen".to_owned()));
    s.frame("relabelled").unwrap();
    let update = s.pipeline_mut().access_update(1.0, "page");
    let stats = s.pipeline().access_stats();
    // The text node: its button is named by AccessKit from it, not by us.
    assert_eq!(update.nodes.len(), 1, "{:?}", update.nodes);
    assert!(stats.visited <= 2, "the changed node alone is read: {stats:?}");
    assert!(!stats.full);
    let steady = s.pipeline_mut().access_update(1.0, "page");
    assert!(steady.nodes.is_empty());
    assert_eq!(s.pipeline().access_stats().visited, 0);
}

#[test]
fn a_signal_written_after_a_later_phase_read_it_is_reported() {
    let read_in_layout = Signal::labeled("read in layout", 0_u32);
    let read_in_paint = Signal::labeled("read in paint", 0_u32);
    // What a node's layout and paint subscriptions read (an untracked read
    // depends on nothing, so no write after it is late).
    let reader = RenderNode::new(|| {});
    drop(reactive::take_late_writes());
    reactive::begin_frame();
    reactive::in_phase(Phase::Layout, false, || reader.track(|| read_in_layout.get()));
    reactive::in_phase(Phase::Paint, false, || {
        let _ = reader.track(|| read_in_paint.get());
        read_in_paint.set(1);
    });
    assert!(
        reactive::take_late_writes().is_empty(),
        "a write in the phase that read it is not late"
    );
    reactive::in_phase(Phase::Paint, false, || read_in_layout.set(1));
    let late = reactive::take_late_writes();
    assert_eq!(late.len(), 1, "{late:?}");
    let l = &late[0];
    assert_eq!((l.read_in, l.written_in), (Phase::Layout, Phase::Paint));
    let message = l.to_string();
    assert!(message.contains("read in layout"), "it names the signal: {message}");
    assert!(message.contains("layout") && message.contains("paint"), "{message}");
    // A layout pass that settles (one more follows) may write what layout read.
    reactive::begin_frame();
    reactive::in_phase(Phase::Layout, true, || reader.track(|| read_in_layout.get()));
    reactive::in_phase(Phase::Layout, true, || read_in_layout.set(2));
    assert!(reactive::take_late_writes().is_empty());
    // The next frame starts afresh.
    reactive::begin_frame();
    reactive::in_phase(Phase::Paint, false, || read_in_layout.set(3));
    assert!(reactive::take_late_writes().is_empty());
    reader.dispose();
}

#[test]
fn a_late_write_in_a_frame_shows_in_its_report() {
    let seen = Signal::labeled("seen width", 0.0_f32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .bind(move |st| st.w_px(40.0 + seen.get()))
                .h_px(10.0)
                .into_element()
        },
        300,
        100,
        1.0,
    )
    .unwrap();
    s.check_cold(false);
    s.settle().unwrap();
    s.take_reports();
    // Written while paint runs, after layout of this frame read it.
    let reader = RenderNode::new(|| {});
    reactive::begin_frame();
    reactive::in_phase(Phase::Layout, false, || reader.track(|| seen.get()));
    reactive::in_phase(Phase::Paint, false, || seen.set(4.0));
    reader.dispose();
    let r = s.frame("late").unwrap();
    assert_eq!(r.late.len(), 1, "{:?}", r.late);
    assert!(r.to_string().contains("seen width"), "{r}");
}
