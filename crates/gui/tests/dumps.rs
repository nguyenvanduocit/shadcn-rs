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
fn settings() -> impl FnMut() -> Element + 'static {
    let remember = Signal::labeled("remember", false);
    let name = TextField::new("");
    move || {
        let r = remember;
        let checked = remember.get();
        div()
            .size_full()
            .p(4.0)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(3.0)
                    .w_px(320.0)
                    .p(6.0)
                    .border()
                    .border_color(Color::BLACK)
                    .child(text("Account settings").text_xl().font_semibold())
                    .child(text("Choose how the app greets you and what it keeps between visits.").text_sm())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(1.5)
                            .child(text("Display name").text_sm().font_medium())
                            .child(
                                text_input(&name)
                                    .placeholder("Ada Lovelace")
                                    .aria_label("Display name")
                                    .h(9.0)
                                    .px(3.0)
                                    .border(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(2.0)
                            .role(Role::CheckBox)
                            .aria_label("Remember me")
                            .aria_checked(checked)
                            .on_click(move |_| r.update(|v| *v = !*v))
                            .child(div().size(4.0).border().when(checked, |d| d.bg(Color::BLACK)))
                            .child(text("Remember me").text_sm()),
                    )
                    .child(
                        div()
                            .role(Role::List)
                            .children((1..).zip(["Profile", "Notifications", "Privacy"]).map(|(i, label)| {
                                div()
                                    .role(Role::ListItem)
                                    .aria_pos_in_set(i, 3)
                                    .py(1.0)
                                    .child(text(label).text_sm())
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(2.0)
                            .child(
                                div()
                                    .role(Role::Button)
                                    .disabled(true)
                                    .px(4.0)
                                    .py(2.0)
                                    .child(text("Reset").text_sm()),
                            )
                            .child(
                                div()
                                    .role(Role::Button)
                                    .on_click(|_| {})
                                    .px(4.0)
                                    .py(2.0)
                                    .bg(Color::BLACK)
                                    .child(text("Save").text_sm().text_color(Color::WHITE)),
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
