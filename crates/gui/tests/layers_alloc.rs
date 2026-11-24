//! A frame with a layer open (ADR-0024) costs what one without does: the
//! layer stack is read again into its own buffers and the focus manager
//! settles without allocating while no focus scope opens or closes, so
//! hovering rows of an open layer allocates nothing on the UI thread.
//!
//! The layer's region is in the main tree here, so what is measured is the
//! layer stack and the focus manager alone: a hover inside a deferred
//! subtree allocates in building and layout, apart from layers.

use gui::app::Scenario;
use gui::prelude::*;
use gui::{Color, FocusHandle, FocusScope, Interaction, Layer, Signal, div, text};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

const REST: Color = Color::WHITE;
const LIT: Color = Color::BLACK;

/// A page with a focusable button and, open, a layer of `n` rows, each
/// lit while hovered, in a focus scope that took focus as it opened.
fn page(n: usize, open: Signal<bool>) -> gui::Element {
    let pop = open.get().then(|| {
        div()
            .layer(Layer::new().focus_scope(FocusScope::new()))
            .track_focus(FocusHandle::new())
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .width(200.0)
            .children((0..n).map(|r| {
                let i = Interaction::new();
                div()
                    .interaction(i)
                    .height(20.0)
                    .padding_x(8.0)
                    .background(move || if i.hovered() { LIT } else { REST })
                    .child(text(format!("row {r}")))
            }))
    });
    div()
        .child(
            div()
                .track_focus(FocusHandle::new())
                .on_click(|_| {})
                .child(text("Trigger")),
        )
        .child(pop)
        .into_element()
}

#[test]
fn hovering_in_an_open_layer_allocates_nothing() {
    gui::testing::both_passes(hover);
}

#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn hover(pass: gui::testing::Pass) {
    let open = Signal::new(true);
    let mut s = Scenario::measuring(move || page(30, open), 600, 700, 1.0).unwrap();
    gui::reactive::stop_trace();
    pass.scenario(&mut s);
    s.settle().unwrap();
    for row in ["row 3", "row 4"] {
        let (x, y) = s.locate(row).unwrap();
        s.hover(x, y);
        s.settle().unwrap();
        s.frame("warm").unwrap();
    }
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    let bytes = |t: Option<gui::heap::ThreadHeap>| t.map_or(u64::MAX, |t| t.allocated);
    for (step, row) in ["row 7", "row 8", "row 7", "row 9"].into_iter().enumerate() {
        let (x, y) = s.locate(row).unwrap();
        let ((), input) = gui::heap::measure(|| s.hover(x, y));
        let taken = bytes(input);
        pass.measured(|| assert_eq!(taken, 0, "{taken} bytes allocated taking hover {step} in"));
        let r = s.frame(&format!("hover {step}")).unwrap();
        let built = bytes(r.footprint.built);
        pass.measured(|| {
            assert_eq!(
                built, 0,
                "{built} bytes allocated building hover {step} with a layer open"
            );
        });
        let ui = bytes(r.footprint.thread).saturating_sub(bytes(r.footprint.committed));
        pass.measured(|| {
            assert_eq!(
                ui, 0,
                "{ui} bytes allocated on the UI thread for hover {step} with a layer open"
            );
        });
    }
    let r = s.frame("rest").unwrap();
    let ui = bytes(r.footprint.thread).saturating_sub(bytes(r.footprint.committed));
    pass.measured(|| assert_eq!(ui, 0, "a steady frame with a layer open allocates nothing"));
}
