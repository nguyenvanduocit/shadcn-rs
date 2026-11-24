//! A node that shows `focus_within` through its own `Interaction` (the one a
//! render makes) paints the same in a frame built cold as in the live one
//! (ADR-0013): the cold tree starts with no focused path, so an ancestor
//! visited before the focused node must still be told focus is within it.
//! Run with `GUI_COLD_CHECK=1` or not: the scenario checks its frames itself.

use gui::app::Scenario;
use gui::interact::Flag;
use gui::prelude::*;
use gui::{Color, Element, Interaction, div, text_input};

/// The box's background as the last frame painted it.
fn look(s: &Scenario) -> Option<Color> {
    s.last()
        .nodes
        .iter()
        .find(|n| n.name == Some("box"))
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
    assert_eq!(look(&s), Some(Color::WHITE));
    // The field is at 10..90 by 10..34, inside the box's padding.
    s.click(40.0, 20.0);
    s.settle().expect("the frame after focus moved inside the box");
    assert_eq!(look(&s), Some(Color::BLACK), "focus is within the box");
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
