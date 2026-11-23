//! A frame the compositor composes over a scene at rest allocates nothing
//! (ADR-0015): its sources, its working sets and the frame it presents
//! are last frames' memory.

use gui::app::Scenario;
use gui::prelude::*;
use gui::{div, pct};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

/// Two lists side by side, each with an overlay scrollbar.
fn two_lists() -> gui::Element {
    let list = |left: f32| {
        div()
            .position(Position::Absolute)
            .left(left)
            .top(0.0)
            .width(100.0)
            .height(100.0)
            .overflow_y(Overflow::Scroll)
            .child(div().height(400.0))
    };
    div()
        .size(pct(100.0))
        .child(list(0.0))
        .child(list(200.0))
        .into_element()
}

/// A commit's scroll bookkeeping allocates nothing: with its scroll
/// containers at rest (nothing held for any), and with a scroll owed (the
/// set of those shown is one kept), 100 commits each.
#[test]
fn a_commits_scroll_bookkeeping_allocates_nothing() {
    let mut s = Scenario::measuring(two_lists, 400, 100, 1.0).expect("a scenario");
    s.check_cold(false);
    s.frame("first").expect("a frame");
    let c = s.compositor();
    let ((), heap) = gui::heap::measure(|| {
        for _ in 0..100 {
            c.recommit_scrollers();
        }
    });
    assert_eq!(heap.expect("counted").allocations, 0, "at rest");
    let seen = c.presented().frame;
    let wheeled = c.wheel_input(gui::compositor::WheelInput {
        point: (50.0, 50.0),
        delta: (0.0, 30.0),
        at: 0.0,
        precise: false,
        began: false,
        ended: false,
        seen,
    });
    assert!(
        matches!(wheeled, gui::compositor::Wheeled::Scrolled),
        "the list scrolls: {wheeled:?}"
    );
    // Warm: the set of those shown grows once.
    c.recommit_scrollers();
    let ((), heap) = gui::heap::measure(|| {
        for _ in 0..100 {
            c.recommit_scrollers();
        }
    });
    assert_eq!(heap.expect("counted").allocations, 0, "with a scroll owed");
}

#[test]
fn a_frame_composed_over_a_scene_at_rest_allocates_nothing() {
    let mut s = Scenario::measuring(two_lists, 400, 100, 1.0).expect("a scenario");
    s.check_cold(false);
    s.frame("first").expect("a frame");
    let c = s.compositor();
    // Warm: the frames kept for late wheels filled, a frame retired.
    for n in 0..8_u8 {
        c.compose(f32::from(n) * 0.01);
    }
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    for n in 8..12_u8 {
        let at = f32::from(n) * 0.01;
        let ((), heap) = gui::heap::measure(|| {
            c.compose(at);
        });
        let heap = heap.expect("counted");
        assert_eq!(
            (heap.allocations, heap.allocated),
            (0, 0),
            "frame {n} composed at rest allocated"
        );
    }
}
