//! ADR-0024's return-focus rule as menus meet it: a menu closed by a press
//! or by focus outside leaves focus to the engine, which gives it back to
//! the trigger only after a primary press outside a modal menu, and never
//! takes back focus that something else has taken. Escape focuses the
//! trigger.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a scenario that cannot run, or a node not found, is a failed test"
)]

use gui::events::{Modifiers, NamedKey};
use gui::prelude::*;
use gui::{Element, FocusHandle, PointerButton, Scenario, div, text};
use shadcn::dropdown_menu::{
    MenuItem, dropdown_menu, dropdown_menu_content, dropdown_menu_item, dropdown_menu_trigger,
};
use shadcn::menubar::{menubar, menubar_content, menubar_item, menubar_menu, menubar_trigger};

fn scenario(f: impl Fn() -> Element + 'static) -> Scenario {
    let mut s = Scenario::measuring(f, 800, 600, 1.0).expect("a scenario");
    s.settle().expect("settled");
    s
}

fn at(s: &Scenario, name: &str) -> (f32, f32) {
    s.locate(name).unwrap_or_else(|_| panic!("{name} is on screen"))
}

fn shows(s: &Scenario, name: &str) -> bool {
    s.locate(name).is_ok()
}

fn click(s: &mut Scenario, name: &str) {
    let (x, y) = at(s, name);
    s.click(x, y);
    s.settle().expect("settled");
}

/// A focusable 120×30 box showing `name`.
fn focusable(name: &'static str, focus: FocusHandle) -> gui::Div {
    div().track_focus(focus).width(120.0).height(30.0).child(text(name))
}

#[test]
fn a_menubar_menu_closed_by_focus_outside_leaves_focus_where_it_went() {
    let elsewhere = FocusHandle::new();
    let mut s = scenario(move || {
        let file = menubar_content().child(menubar_item("New", MenuItem::default()).child("New"));
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(200.0)
            .child(menubar().child(menubar_menu().child(menubar_trigger("File")).child(file)))
            .child(focusable("Elsewhere", elsewhere))
            .into_element()
    });
    click(&mut s, "File");
    assert!(shows(&s, "New"), "the menu opened");

    s.act("focus elsewhere", || elsewhere.focus());
    s.settle().expect("settled");
    assert!(!shows(&s, "New"), "focus outside a menubar's menu closes it");
    // Focus is the window's: read in a turn of it (ADR-0015).
    assert_eq!(
        s.read(gui::focus::focused),
        Some(elsewhere.id()),
        "focus taken elsewhere is not taken back"
    );
}

#[test]
fn a_modal_menu_gives_focus_back_after_a_primary_press_outside_and_not_after_a_secondary_one() {
    let mut s = scenario(move || {
        let content =
            dropdown_menu_content().child(dropdown_menu_item("Profile", MenuItem::default()).child("Profile"));
        let menu = dropdown_menu()
            .child(dropdown_menu_trigger(
                div().width(120.0).height(30.0).child(text("Open")),
            ))
            .child(content);
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(300.0)
            .child(menu)
            .child(div().width(200.0).height(30.0).child(text("Page")))
            .into_element()
    });
    // Focus is the window's: read in a turn of it (ADR-0015).
    let held = |s: &mut Scenario| s.read(gui::focus::focused);

    click(&mut s, "Open");
    assert!(shows(&s, "Profile"));
    let inside = held(&mut s);
    s.press(NamedKey::Escape, Modifiers::default());
    s.settle().expect("settled");
    assert!(!shows(&s, "Profile"));
    let trigger = held(&mut s);
    assert!(trigger.is_some() && trigger != inside, "Escape focuses the trigger");
    assert!(
        s.read(gui::focus::focus_visible),
        "with its ring: the keyboard closed it"
    );

    click(&mut s, "Open");
    assert!(shows(&s, "Profile"));
    assert_ne!(held(&mut s), trigger, "focus went into the menu");
    let (x, y) = at(&s, "Page");
    s.secondary_click(x, y);
    s.settle().expect("settled");
    assert!(!shows(&s, "Profile"), "a secondary press outside closes it");
    assert_eq!(
        held(&mut s),
        None,
        "and gives no focus back (reka's hasInteractedOutside)"
    );

    click(&mut s, "Open");
    assert!(shows(&s, "Profile"));
    s.press_button(x, y, PointerButton::Middle);
    s.settle().expect("settled");
    assert!(!shows(&s, "Profile"), "so does the middle button");
    assert_eq!(held(&mut s), None);

    click(&mut s, "Open");
    assert!(shows(&s, "Profile"));
    s.click(x, y);
    s.settle().expect("settled");
    assert!(!shows(&s, "Profile"));
    assert_eq!(
        held(&mut s),
        trigger,
        "a primary press outside a modal menu gives focus back"
    );
    assert!(
        !s.read(gui::focus::focus_visible),
        "without a ring: a pointer closed it"
    );
}

#[test]
fn a_menubar_opens_its_default_menu_and_wraps_when_asked() {
    use primitives::Wrap;
    let bar = |wrap: Wrap| {
        move || {
            let menu = |name: &'static str, item: &'static str| {
                menubar_menu()
                    .value(name)
                    .child(menubar_trigger(name))
                    .child(menubar_content().child(menubar_item(item, MenuItem::default()).child(item)))
            };
            div()
                .child(
                    menubar()
                        .default_value("Edit")
                        .wrap(wrap)
                        .child(menu("File", "New"))
                        .child(menu("Edit", "Undo")),
                )
                .into_element()
        }
    };
    let mut s = scenario(bar(Wrap::Around));
    assert!(shows(&s, "Undo"), "the default menu is open at first");
    assert!(!shows(&s, "New"));
    s.press(NamedKey::ArrowRight, Modifiers::default());
    s.settle().expect("settled");
    assert!(shows(&s, "New"), "past the last menu it wraps to the first");

    let mut s = scenario(bar(Wrap::Stop));
    s.press(NamedKey::ArrowRight, Modifiers::default());
    s.settle().expect("settled");
    assert!(shows(&s, "Undo"), "reka's loop is off: the last menu stays");
    assert!(!shows(&s, "New"));
}
