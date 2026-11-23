//! Text snapshots (insta, M9 I4) of what gui's devtools print about a page:
//! the layout dump (`gui::inspect::dump`, one node a line with its box) and
//! the accessibility tree a screen reader is told (the `a11y` command of a
//! `gui::devtools::Session`), before and after input.
//!
//! Snapshots live in `tests/snapshots/`; review a change with `cargo insta
//! review`, or rewrite them with `INSTA_UPDATE=always cargo test -p gui
//! --test dumps`.
//!
//! The same on every OS: the pages use Latin text only, which gui's own
//! Inter has every glyph of, so no system font is ever chosen; layout
//! measures text with Parley's unhinted advances (hinting and pixel-snapped
//! glyph origins are paint-time and per-OS, not in a box); the dump rounds
//! boxes to whole pixels; and the a11y dump prints roles, names, states,
//! actions and node paths, never a time or a coordinate.

#![expect(clippy::expect_used, reason = "a test: a page that cannot be drawn is a failed test")]

use gui::devtools::Session;
use gui::element::Role;
use gui::prelude::*;
use gui::{Color, Element, Scenario, Signal, TextField, div, text, text_input};

/// A settings card: a heading, a labelled field, a checkbox, a list of
/// three, and two buttons, one disabled. Clicking "Remember me" toggles it.
fn settings() -> impl Fn() -> Element + 'static {
    let remember = Signal::labeled("remember", false);
    let name = TextField::new("");
    move || {
        let r = remember;
        let checked = remember.get();
        div()
            .size(pct(100.0))
            .padding(16.0)
            .child(
                div()
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .gap(12.0)
                    .width(320.0)
                    .padding(24.0)
                    .border(1.0)
                    .border_color(Color::BLACK)
                    .child(
                        text("Account settings")
                            .text_size(20.0)
                            .line_height(28.0)
                            .font_weight(600),
                    )
                    .child(
                        text("Choose how the app greets you and what it keeps between visits.")
                            .text_size(14.0)
                            .line_height(20.0),
                    )
                    .child(
                        div()
                            .display(Display::Flex)
                            .flex_direction(FlexDirection::Column)
                            .gap(6.0)
                            .child(text("Display name").text_size(14.0).line_height(20.0).font_weight(500))
                            .child(
                                text_input(&name)
                                    .placeholder("Ada Lovelace")
                                    .aria_label("Display name")
                                    .height(36.0)
                                    .padding_x(12.0)
                                    .border(1.0),
                            ),
                    )
                    .child(
                        div()
                            .display(Display::Flex)
                            .align_items(Align::Center)
                            .gap(8.0)
                            .role(Role::CheckBox)
                            .aria_label("Remember me")
                            .aria_checked(checked)
                            .on_click(move |_| r.update(|v| *v = !*v))
                            .child(
                                div()
                                    .size(16.0)
                                    .border(1.0)
                                    .when(checked, |d| d.background(Color::BLACK)),
                            )
                            .child(text("Remember me").text_size(14.0).line_height(20.0)),
                    )
                    .child(
                        div()
                            .role(Role::List)
                            .children((1..).zip(["Profile", "Notifications", "Privacy"]).map(|(i, label)| {
                                div()
                                    .role(Role::ListItem)
                                    .aria_pos_in_set(i, 3)
                                    .padding_y(4.0)
                                    .child(text(label).text_size(14.0).line_height(20.0))
                            })),
                    )
                    .child(
                        div()
                            .display(Display::Flex)
                            .justify_content(Justify::End)
                            .gap(8.0)
                            .child(
                                div()
                                    .role(Role::Button)
                                    .disabled(true)
                                    .padding_x(16.0)
                                    .padding_y(8.0)
                                    .child(text("Reset").text_size(14.0).line_height(20.0)),
                            )
                            .child(
                                div()
                                    .role(Role::Button)
                                    .on_click(|_| {})
                                    .padding_x(16.0)
                                    .padding_y(8.0)
                                    .background(Color::BLACK)
                                    .child(text("Save").text_size(14.0).line_height(20.0).text_color(Color::WHITE)),
                            ),
                    ),
            )
            .into_element()
    }
}

fn session() -> Session {
    let scenario = Scenario::measuring(settings(), 400, 420, 1.0).expect("a scenario");
    Session::new(scenario).expect("a session")
}

#[test]
fn layout_dump_of_a_settings_card() {
    let mut s = session();
    insta::assert_snapshot!(gui::inspect::dump(&s.scenario().last().nodes));
}

#[test]
fn accessibility_tree_of_a_settings_card() {
    let mut s = session();
    insta::assert_snapshot!("before", s.run("a11y").expect("a11y"));
    s.run("click \"Remember me\"").expect("a click");
    insta::assert_snapshot!("after_checking", s.run("a11y").expect("a11y"));
}
