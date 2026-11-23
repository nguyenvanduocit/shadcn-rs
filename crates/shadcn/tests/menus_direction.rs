//! A submenu opens with the arrow towards the inline end and closes with the
//! other, so under RTL ArrowLeft opens it and ArrowRight closes it (reka's
//! `SUB_OPEN_KEYS` and `SUB_CLOSE_KEYS`; research §4 item 13).

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a scenario that cannot run, or a node not found, is a failed test"
)]

use gui::events::{Modifiers, NamedKey};
use gui::prelude::*;
use gui::{Direction, Scenario, Signal, component, div, text};
use primitives::provide_direction;
use shadcn::dropdown_menu::{
    MenuItem, dropdown_menu, dropdown_menu_content, dropdown_menu_item, dropdown_menu_sub, dropdown_menu_sub_content,
    dropdown_menu_sub_trigger, dropdown_menu_trigger,
};

fn menu(direction: Signal<Direction>) -> Scenario {
    let mut s = Scenario::measuring(
        move || {
            component((), move |_| {
                let scope = provide_direction(direction);
                move || {
                    let sub = dropdown_menu_sub()
                        .child(dropdown_menu_sub_trigger("More", MenuItem::default()))
                        .child(
                            dropdown_menu_sub_content()
                                .child(dropdown_menu_item("Child", MenuItem::default()).child("Child")),
                        );
                    scope
                        .root(div())
                        .child(
                            dropdown_menu()
                                .child(dropdown_menu_trigger(
                                    div().width(120.0).height(30.0).child(text("Open")),
                                ))
                                .child(dropdown_menu_content().child(sub)),
                        )
                        .into_element()
                }
            })
            .into_element()
        },
        800,
        600,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("settled");
    let (x, y) = s.locate("Open").unwrap_or_else(|_| panic!("the trigger is on screen"));
    s.click(x, y);
    s.settle().expect("settled");
    assert!(s.locate("More").is_ok(), "the menu opened");
    press(&mut s, NamedKey::ArrowDown);
    s
}

fn press(s: &mut Scenario, key: NamedKey) {
    s.press(key, Modifiers::default());
    s.settle().expect("settled");
}

fn child_shown(s: &Scenario) -> bool {
    s.locate("Child").is_ok()
}

#[test]
fn a_submenu_opens_towards_the_inline_end() {
    let mut s = menu(Signal::new(Direction::Ltr));
    press(&mut s, NamedKey::ArrowLeft);
    assert!(!child_shown(&s), "ArrowLeft does not open a submenu left to right");
    press(&mut s, NamedKey::ArrowRight);
    assert!(child_shown(&s), "ArrowRight opens it");
    press(&mut s, NamedKey::ArrowLeft);
    assert!(!child_shown(&s), "ArrowLeft closes it");

    let mut s = menu(Signal::new(Direction::Rtl));
    press(&mut s, NamedKey::ArrowRight);
    assert!(!child_shown(&s), "ArrowRight does not open a submenu right to left");
    press(&mut s, NamedKey::ArrowLeft);
    assert!(child_shown(&s), "ArrowLeft opens it");
    press(&mut s, NamedKey::ArrowRight);
    assert!(!child_shown(&s), "ArrowRight closes it");
}

#[test]
fn switching_direction_at_run_time_turns_the_keys_of_an_open_menu() {
    let direction = Signal::new(Direction::Ltr);
    let mut s = menu(direction);
    s.act("right to left", || direction.set(Direction::Rtl));
    s.settle().expect("settled");
    press(&mut s, NamedKey::ArrowRight);
    assert!(!child_shown(&s), "the open menu reads the new direction at the key");
    press(&mut s, NamedKey::ArrowLeft);
    assert!(child_shown(&s), "ArrowLeft opens it now");
}
