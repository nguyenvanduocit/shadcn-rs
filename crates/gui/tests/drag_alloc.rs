//! A drag under a scaled or turned ancestor costs what one without does:
//! mapping the pointer into the dragged node's space (its ancestors'
//! scales and turns undone) and the pressed state of every pointer down
//! allocate nothing, so each move of such a drag allocates nothing as the
//! UI takes it in.

use std::cell::Cell;
use std::rc::Rc;

use gui::app::Scenario;
use gui::prelude::*;
use gui::{Color, div};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

/// A box dragged inside a container scaled 1.5x and turned, counting its
/// drag events.
fn page(moves: &Rc<Cell<u32>>) -> gui::Element {
    let moves = Rc::clone(moves);
    div()
        .width(600.0)
        .height(400.0)
        .child(
            div().width(300.0).height(200.0).scale(1.5).rotate(10.0).child(
                div()
                    .width(200.0)
                    .height(100.0)
                    .background(Color::BLACK)
                    .on_drag(move |_| moves.set(moves.get().saturating_add(1))),
            ),
        )
        .into_element()
}

#[test]
fn a_drag_under_a_scaled_and_turned_ancestor_allocates_nothing_as_it_moves() {
    gui::testing::both_passes(drag);
}

#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn drag(pass: gui::testing::Pass) {
    let moves = Rc::new(Cell::new(0));
    let counted = Rc::clone(&moves);
    let mut s = Scenario::measuring(move || page(&counted), 600, 400, 1.0).unwrap();
    gui::reactive::stop_trace();
    pass.scenario(&mut s);
    s.settle().unwrap();
    s.hover(90.0, 15.0);
    s.send(gui::window::Input::MouseDown { x: 90.0, y: 15.0 });
    for x in [92.0, 94.0, 96.0] {
        s.hover(x, 15.0);
        s.settle().unwrap();
        s.frame("warm").unwrap();
    }
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    let bytes = |t: Option<gui::heap::ThreadHeap>| t.map_or(u64::MAX, |t| t.allocated);
    let before = moves.get();
    for (step, x) in [98.0, 100.0, 102.0, 104.0].into_iter().enumerate() {
        let ((), input) = gui::heap::measure(|| s.hover(x, 16.0));
        let taken = bytes(input);
        pass.measured(|| assert_eq!(taken, 0, "{taken} bytes allocated taking drag move {step} in"));
    }
    assert_eq!(moves.get(), before.saturating_add(4), "each move reached the drag");
    s.send(gui::window::Input::MouseUp { x: 104.0, y: 16.0 });
}
