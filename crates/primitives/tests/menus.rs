//! Group 4, the menus, in a window by behaviour (research §8, ADR-0025
//! "Group 4"): the menu engine's keyboard table, its highlight as focus,
//! submenus' 100 ms and the 300 ms grace area, picking and its veto,
//! checkbox and radio requests; the dropdown's trigger and focus return;
//! the context menu's point, its 700 ms long press and its keys; the
//! menubar's one Tab stop and its moves between menus; the navigation
//! menu's 200 / 150 / 300 ms and its keys; the toolbar's arrows. Each has
//! what a screen reader is told as a snapshot.

#![expect(
    clippy::disallowed_types,
    reason = "test and example state the test reads back, not a component's"
)]
#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a scenario that cannot run, or a node not found, is a failed test; counters are small"
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use accesskit::{Node, NodeId, TreeUpdate};
use gui::app::Scenario;
use gui::events::{Modifiers, NamedKey};
use gui::prelude::*;
use gui::window::Input;
use gui::{
    Axis, Dismiss, Div, Element, FocusHandle, PointerKind, Positioning, Side, SideAlign, Signal, Toggled, TouchEvent,
    TouchPhase, component, deferred, div, text,
};
use primitives::context_menu::ContextMenu;
use primitives::dropdown_menu::DropdownMenu;
use primitives::menu::{GRACE, Menu, MenuOptions, SUB_OPEN_DELAY};
use primitives::menubar::{Menubar, MenubarMenu};
use primitives::navigation_menu::{
    CLOSE_DELAY, NavOptions, NavigationMenu, NavigationMenuItem, OPEN_DELAY, SKIPPED_DELAY,
};
use primitives::toolbar::Toolbar;
use primitives::{Availability, Item, Wrap};

type Log = Rc<RefCell<Vec<String>>>;

/// Hand `v`, made in a setup, out to the test: in an effect, as a setup
/// (as a render) writes no live signal it did not make.
fn hand_out<T: Clone + PartialEq + 'static>(to: Signal<T>, v: T) {
    gui::Effect::new(move || to.set(v.clone()));
}

/// [`hand_out`], one more of a list.
fn hand_out_push<T: Clone + 'static>(to: Signal<Vec<T>>, v: T) {
    gui::Effect::new(move || gui::untracked(|| to.update(|t| t.push(v.clone()))));
}

fn scenario(f: impl Fn() -> Element + 'static) -> Scenario {
    let mut s = Scenario::measuring(f, 800, 600, 1.0).expect("a scenario");
    // Every frame is checked against a cold one (a render is pure),
    // with `GUI_COLD_CHECK` set or not, so the menus' virtual lists
    // cannot regress unseen.
    s.check_cold(true);
    s.settle().expect("settled");
    s
}

fn focused(h: FocusHandle) -> bool {
    gui::focus::focused() == Some(h.id())
}

fn key(s: &mut Scenario, k: NamedKey, modifiers: Modifiers) {
    s.press(k, modifiers);
    s.settle().expect("settled");
}

fn plain(s: &mut Scenario, k: NamedKey) {
    key(s, k, Modifiers::default());
}

fn shows(s: &Scenario, name: &str) -> bool {
    s.locate(name).is_ok()
}

fn at(s: &Scenario, name: &str) -> (f32, f32) {
    s.locate(name).unwrap_or_else(|_| panic!("{name} is on screen"))
}

fn click(s: &mut Scenario, name: &str) {
    let (x, y) = at(s, name);
    s.click(x, y);
    s.settle().expect("settled");
}

fn hover(s: &mut Scenario, x: f32, y: f32) {
    s.hover(x, y);
    s.settle().expect("settled");
}

fn wait(s: &mut Scenario, ms: u64) {
    s.wait(Duration::from_millis(ms));
    s.settle().expect("settled");
}

fn type_char(s: &mut Scenario, c: &str) {
    s.type_text(c);
    s.settle().expect("settled");
}

fn row(name: &str) -> Div {
    div().width(140.0).height(24.0).child(text(name.to_owned()))
}

fn below() -> Positioning {
    Positioning::Anchored {
        side: Side::Bottom,
        align: SideAlign::Start,
        offset: 0.0,
        match_width: false,
        fit_height: false,
    }
}

/// The items of a menu: `name` and whether it is disabled.
const ITEMS: [(&str, bool); 5] = [
    ("Apple", false),
    ("Banana", false),
    ("Cherry", true),
    ("Date", false),
    ("Elder", false),
];

/// A level's plain items, each logging its pick; `veto` keeps the menu open.
fn items(menu: Menu, foci: &[FocusHandle; 5], picked: &Log, veto: bool) -> Vec<Element> {
    ITEMS
        .iter()
        .zip(foci)
        .map(|((name, disabled), focus)| {
            let picked = Rc::clone(picked);
            let entry = Item::new((), *focus)
                .label(*name)
                .availability(Availability::from_disabled(*disabled));
            menu.item(row(name), entry, move |()| {
                picked.borrow_mut().push((*name).to_owned());
                if veto { Dismiss::Veto } else { Dismiss::Close }
            })
            .into_element()
        })
        .collect()
}

fn content(menu: Menu, positioning: Positioning, children: Vec<Element>) -> Option<Element> {
    menu.is_open().then(|| {
        deferred(positioning, menu.content(div().width(160.0).children(children)))
            .placement(menu.placed())
            .into_element()
    })
}

struct Dropdown {
    open: Signal<bool>,
    trigger_focus: Signal<Option<FocusHandle>>,
    foci: [FocusHandle; 5],
    picked: Log,
    /// A focusable box below.
    page: FocusHandle,
}

/// A dropdown menu of [`ITEMS`] below an "Open" trigger, a page box below.
fn dropdown(options: MenuOptions, veto: bool, availability: Availability) -> (Scenario, Dropdown) {
    let open = Signal::new(false);
    let trigger_focus = Signal::new(None);
    let foci = [(); 5].map(|()| FocusHandle::new());
    let picked: Log = Rc::default();
    let page = FocusHandle::new();
    let log = Rc::clone(&picked);
    let s = scenario(move || {
        let log = Rc::clone(&log);
        component((), move |_| {
            let dd = DropdownMenu::new(open, options);
            hand_out(trigger_focus, Some(dd.menu().trigger_focus()));
            let log = Rc::clone(&log);
            move || {
                let menu = dd.menu();
                div()
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .gap(300.0)
                    .child(
                        div()
                            .child(dd.trigger(div().width(80.0).height(30.0).child(text("Open")), availability))
                            .child(content(menu, below(), items(menu, &foci, &log, veto))),
                    )
                    .child(div().track_focus(page).width(200.0).height(30.0).child(text("Page")))
                    .into_element()
            }
        })
        .into_element()
    });
    (
        s,
        Dropdown {
            open,
            trigger_focus,
            foci,
            picked,
            page,
        },
    )
}

fn trigger_of(d: &Dropdown) -> FocusHandle {
    d.trigger_focus.get_untracked().expect("the trigger's focus")
}

#[test]
fn the_menu_keyboard_table() {
    let (mut s, d) = dropdown(MenuOptions::DEFAULT, false, Availability::Enabled);
    let trigger = trigger_of(&d);
    s.act("focus the trigger", || trigger.focus());
    s.settle().expect("settled");

    plain(&mut s, NamedKey::Enter);
    assert!(d.open.get_untracked(), "Enter on the trigger opens it");
    assert!(focused(d.foci[0]), "on its first item");
    plain(&mut s, NamedKey::ArrowUp);
    assert!(focused(d.foci[0]), "no wrap past the first");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(d.foci[1]));
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(d.foci[3]), "the disabled item is passed");
    plain(&mut s, NamedKey::End);
    assert!(focused(d.foci[4]), "End: the last");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(d.foci[4]), "no wrap past the last");
    plain(&mut s, NamedKey::Home);
    assert!(focused(d.foci[0]), "Home: the first");
    plain(&mut s, NamedKey::PageDown);
    assert!(focused(d.foci[4]), "PageDown: the last, from an item too");
    plain(&mut s, NamedKey::PageUp);
    assert!(focused(d.foci[0]), "PageUp: the first");

    type_char(&mut s, "d");
    assert!(focused(d.foci[3]), "typeahead reaches Date");
    type_char(&mut s, "c");
    assert!(focused(d.foci[3]), "Dc matches nothing: the highlight stays");
    wait(&mut s, 1100);
    type_char(&mut s, "c");
    assert!(focused(d.foci[3]), "a disabled item is not typed to");

    plain(&mut s, NamedKey::Escape);
    assert!(!d.open.get_untracked(), "Escape closes");
    assert!(focused(trigger), "and focus is back on the trigger");
    assert!(gui::focus::focus_visible(), "with its ring");

    plain(&mut s, NamedKey::ArrowDown);
    assert!(d.open.get_untracked(), "ArrowDown on the trigger opens it");
    assert!(focused(d.foci[0]));
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::Space);
    assert_eq!(*d.picked.borrow(), ["Banana"], "Space picks");
    assert!(!d.open.get_untracked(), "picking closes");
    assert!(focused(trigger), "focus back on the trigger");

    plain(&mut s, NamedKey::Space);
    assert!(d.open.get_untracked(), "Space on the trigger opens");
    assert!(focused(d.foci[0]));
    plain(&mut s, NamedKey::Tab);
    assert!(d.open.get_untracked(), "Tab does nothing in a modal menu");
    assert!(focused(d.foci[0]), "focus stays on the item");
    key(
        &mut s,
        NamedKey::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert!(focused(d.foci[0]), "Shift+Tab too");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(d.picked.borrow().len(), 2, "Enter picks the focused item");
}

#[test]
fn an_arrow_from_the_content_itself_reaches_an_end() {
    let (mut s, d) = dropdown(MenuOptions::DEFAULT, false, Availability::Enabled);
    s.act("open", || d.open.set(true));
    s.settle().expect("settled");
    assert!(
        d.foci.iter().all(|f| !focused(*f)),
        "opened by its caller, the content holds focus, no item"
    );
    plain(&mut s, NamedKey::ArrowUp);
    assert!(
        focused(d.foci[4]),
        "ArrowUp from the content reaches the last item, though the menu does not wrap"
    );

    let (mut s, d) = dropdown(MenuOptions::DEFAULT, false, Availability::Enabled);
    s.act("open", || d.open.set(true));
    s.settle().expect("settled");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(d.foci[0]), "ArrowDown from the content reaches the first");
}

#[test]
fn space_continues_a_typeahead_search() {
    let (mut s, d) = dropdown(MenuOptions::DEFAULT, false, Availability::Enabled);
    let trigger = trigger_of(&d);
    s.act("focus the trigger", || trigger.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Enter);
    type_char(&mut s, "b");
    assert!(focused(d.foci[1]));
    type_char(&mut s, " ");
    assert!(
        d.open.get_untracked(),
        "Space within a search is part of it, not a pick"
    );
    assert!(d.picked.borrow().is_empty());
}

#[test]
fn the_highlight_is_focus_and_the_pointer_moves_it() {
    let (mut s, d) = dropdown(MenuOptions::DEFAULT, false, Availability::Enabled);
    click(&mut s, "Open");
    assert!(d.open.get_untracked(), "a click opens it");
    assert!(d.foci.iter().all(|f| !focused(*f)), "with no item highlighted");
    let (x, y) = at(&s, "Banana");
    hover(&mut s, x, y);
    assert!(focused(d.foci[1]), "the item under the mouse takes focus");
    let (x, y) = at(&s, "Cherry");
    hover(&mut s, x, y);
    assert!(
        d.foci.iter().all(|f| !focused(*f)),
        "a disabled item takes none: the content holds it"
    );
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(d.foci[0]), "the next arrow starts from an end");
    click(&mut s, "Date");
    assert_eq!(*d.picked.borrow(), ["Date"], "a click picks");
    assert!(!d.open.get_untracked());

    click(&mut s, "Open");
    click(&mut s, "Open");
    assert!(
        !d.open.get_untracked(),
        "a press on the trigger closes it (outside a modal menu), and no click follows"
    );
    click(&mut s, "Open");
    assert!(d.open.get_untracked(), "the next click opens it");
}

#[test]
fn an_item_can_keep_the_menu_open() {
    let (mut s, d) = dropdown(MenuOptions::DEFAULT, true, Availability::Enabled);
    click(&mut s, "Open");
    click(&mut s, "Apple");
    assert_eq!(*d.picked.borrow(), ["Apple"]);
    assert!(
        d.open.get_untracked(),
        "Dismiss::Veto keeps it open (reka's prevented select)"
    );
}

#[test]
fn a_disabled_trigger_opens_nothing() {
    let (mut s, d) = dropdown(MenuOptions::DEFAULT, false, Availability::Disabled);
    click(&mut s, "Open");
    assert!(!d.open.get_untracked());
    assert!(!focused(trigger_of(&d)), "and takes no focus");
}

#[test]
fn the_open_state_is_the_callers() {
    let (mut s, d) = dropdown(MenuOptions::DEFAULT, false, Availability::Enabled);
    s.act("open from outside", || d.open.set(true));
    s.settle().expect("settled");
    assert!(shows(&s, "Apple"), "a write from the caller opens it");
    s.act("close from outside", || d.open.set(false));
    s.settle().expect("settled");
    assert!(!shows(&s, "Apple"));
}

#[test]
fn a_menu_that_is_not_modal_closes_on_focus_outside() {
    let (mut s, d) = dropdown(MenuOptions::DEFAULT.modal(false), false, Availability::Enabled);
    let trigger = trigger_of(&d);
    s.act("focus the trigger", || trigger.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Enter);
    assert!(focused(d.foci[0]));
    plain(&mut s, NamedKey::Tab);
    assert!(focused(d.page), "Tab leaves a menu that is not modal");
    assert!(!d.open.get_untracked(), "and it closes: focus is outside");
}

#[test]
fn a_press_on_the_trigger_closes_a_menu_that_is_not_modal() {
    let (mut s, d) = dropdown(MenuOptions::DEFAULT.modal(false), false, Availability::Enabled);
    click(&mut s, "Open");
    assert!(d.open.get_untracked(), "a click opens it");
    click(&mut s, "Open");
    assert!(
        !d.open.get_untracked(),
        "the trigger is inside the layer by ownership: the press is not outside, the click toggles it closed"
    );
    assert!(focused(trigger_of(&d)), "focus is on the trigger");
    click(&mut s, "Open");
    assert!(d.open.get_untracked(), "the next click opens it, once each");
}

/// A root menu of one item and a sub trigger "More" whose submenu holds
/// "Child" and "Kid"; "Below" under the sub trigger.
struct Subs {
    open: Signal<bool>,
    sub_open: Signal<bool>,
    root_items: [FocusHandle; 3],
    sub_items: [FocusHandle; 2],
}

fn with_submenu() -> (Scenario, Subs) {
    let open = Signal::new(true);
    let sub_open = Signal::new(false);
    let root_items = [(); 3].map(|()| FocusHandle::new());
    let sub_items = [(); 2].map(|()| FocusHandle::new());
    let s = scenario(move || {
        component((), move |_| {
            let menu = Menu::new(open, MenuOptions::DEFAULT);
            move || {
                let sub = component((), move |_| {
                    let sub = menu.submenu(sub_open);
                    move || {
                        let kids = ["Child", "Kid"]
                            .iter()
                            .zip(sub_items)
                            .map(|(n, f)| menu_item(sub, n, f))
                            .collect();
                        let side = Positioning::Anchored {
                            side: sub.sub_side(),
                            align: SideAlign::Start,
                            offset: 0.0,
                            match_width: false,
                            fit_height: false,
                        };
                        div()
                            .child(sub.sub_trigger(row("More"), "More", Availability::Enabled))
                            .child(content(sub, side, kids))
                            .into_element()
                    }
                })
                .into_element();
                let children = vec![
                    menu_item(menu, "First", root_items[0]),
                    sub,
                    menu_item(menu, "Below", root_items[1]),
                    menu_item(menu, "Last", root_items[2]),
                ];
                div()
                    .padding(20.0)
                    .child(content(menu, below(), children))
                    .into_element()
            }
        })
        .into_element()
    });
    (
        s,
        Subs {
            open,
            sub_open,
            root_items,
            sub_items,
        },
    )
}

fn menu_item(menu: Menu, name: &str, focus: FocusHandle) -> Element {
    menu.item(row(name), Item::new((), focus).label(name.to_owned()), |()| {
        Dismiss::Close
    })
    .into_element()
}

#[test]
fn a_submenu_opens_after_the_mouse_rests_100_ms() {
    let (mut s, m) = with_submenu();
    let (x, y) = at(&s, "More");
    hover(&mut s, x, y);
    assert!(!m.sub_open.get_untracked(), "not at once");
    wait(&mut s, 60);
    hover(&mut s, x + 1.0, y);
    assert!(
        !m.sub_open.get_untracked(),
        "a further move does not restart nor hurry it"
    );
    wait(&mut s, u64::try_from(SUB_OPEN_DELAY.as_millis()).expect("ms") - 50);
    assert!(m.sub_open.get_untracked(), "100 ms after the first move it opens");
    assert!(m.sub_items.iter().all(|f| !focused(*f)), "focus stays on its trigger");

    // Leaving before the delay cancels the opening.
    let (mut s, m) = with_submenu();
    let (x, y) = at(&s, "More");
    hover(&mut s, x, y);
    let (bx, by) = at(&s, "First");
    wait(&mut s, 50);
    hover(&mut s, bx, by);
    wait(&mut s, 200);
    assert!(!m.sub_open.get_untracked(), "leaving first cancels it");
    assert!(focused(m.root_items[0]));
}

#[test]
fn keys_open_and_close_a_submenu() {
    let (mut s, m) = with_submenu();
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(m.sub_open.get_untracked(), "the inline-end arrow opens it");
    assert!(focused(m.sub_items[0]), "on its first item");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(m.sub_items[1]), "its keys are its own");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(
        focused(m.sub_items[1]),
        "no wrap, and the parent's items are not reached"
    );
    plain(&mut s, NamedKey::ArrowLeft);
    assert!(!m.sub_open.get_untracked(), "the other arrow closes it");
    plain(&mut s, NamedKey::Enter);
    assert!(m.sub_open.get_untracked(), "Enter on the sub trigger opens it");
    assert!(focused(m.sub_items[0]));
    plain(&mut s, NamedKey::Escape);
    assert!(!m.open.get_untracked(), "Escape in a submenu closes the whole menu");
    assert!(!m.sub_open.get_untracked());
}

#[test]
fn the_grace_area_keeps_a_submenu_on_the_way_to_it() {
    let (mut s, m) = with_submenu();
    let (x, y) = at(&s, "More");
    hover(&mut s, x, y);
    wait(&mut s, 120);
    assert!(m.sub_open.get_untracked());
    // Down and to the right, over "Below", toward the submenu.
    let (_, by) = at(&s, "Below");
    hover(&mut s, x + 60.0, by);
    assert!(m.sub_open.get_untracked(), "in the grace area the submenu stays");
    assert!(!focused(m.root_items[1]), "and the item passed takes no highlight");
    wait(&mut s, u64::try_from(GRACE.as_millis()).expect("ms"));
    hover(&mut s, x + 61.0, by);
    assert!(
        focused(m.root_items[1]),
        "after 300 ms the item under the pointer is highlighted"
    );
    assert!(!m.sub_open.get_untracked(), "and the submenu closes");

    // Moving away from the submenu is no grace.
    let (mut s, m) = with_submenu();
    let (x, y) = at(&s, "More");
    hover(&mut s, x, y);
    wait(&mut s, 120);
    let (bx, by) = at(&s, "Below");
    hover(&mut s, bx - 40.0, by);
    assert!(
        focused(m.root_items[1]),
        "moving away: the item takes the highlight at once"
    );
    assert!(!m.sub_open.get_untracked());
}

#[test]
fn checkbox_and_radio_items_ask_for_their_values() {
    let asked: Log = Rc::default();
    let log = Rc::clone(&asked);
    let open = Signal::new(true);
    let checks = Signal::new([Toggled::False, Toggled::True, Toggled::Mixed]);
    let mut s = scenario(move || {
        let log = Rc::clone(&log);
        component((), move |_| {
            let menu = Menu::new(open, MenuOptions::DEFAULT);
            let foci = [(); 5].map(|()| FocusHandle::new());
            let log = Rc::clone(&log);
            move || {
                let now = checks.get();
                let mut kids: Vec<Element> = ["Off", "On", "Mixed"]
                    .iter()
                    .enumerate()
                    .map(|(i, name)| {
                        let log = Rc::clone(&log);
                        menu.checkbox_item(row(name), Item::new((), foci[i]).label(*name), now[i], move |next| {
                            log.borrow_mut().push(format!("{name}={next}"));
                            Dismiss::Veto
                        })
                        .into_element()
                    })
                    .collect();
                for (i, name) in ["Red", "Blue"].iter().enumerate() {
                    let log = Rc::clone(&log);
                    kids.push(
                        menu.radio_item(row(name), Item::new((), foci[i + 3]).label(*name), i == 0, move |()| {
                            log.borrow_mut().push((*name).to_owned());
                            Dismiss::Veto
                        })
                        .into_element(),
                    );
                }
                div().child(content(menu, below(), kids)).into_element()
            }
        })
        .into_element()
    });
    for name in ["Off", "On", "Mixed", "Red", "Blue"] {
        click(&mut s, name);
    }
    assert_eq!(
        *asked.borrow(),
        ["Off=true", "On=false", "Mixed=true", "Red", "Blue"],
        "a checkbox asks for the flip (mixed: checked); a radio for itself, chosen or not"
    );
    assert!(open.get_untracked(), "each said Veto");
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    let dumped = dump(&tree);
    assert!(
        dumped.contains("MenuItemCheckBox checked=Mixed\n      Label \"Mixed\""),
        "{dumped}"
    );
    assert!(
        dumped.contains("MenuItemRadio checked=True\n      Label \"Red\""),
        "{dumped}"
    );
}

#[test]
fn what_a_screen_reader_is_told_of_a_dropdown_menu() {
    let (mut s, _d) = dropdown(MenuOptions::DEFAULT, false, Availability::Enabled);
    click(&mut s, "Open");
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("menus_dropdown_a11y", dump(&tree));
}

#[test]
fn what_a_screen_reader_is_told_of_a_submenu() {
    let (mut s, _m) = with_submenu();
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::ArrowRight);
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("menus_submenu_a11y", dump(&tree));
}

#[test]
fn a_group_is_named_by_its_label_only_when_it_has_one() {
    let foci = [(); 5].map(|()| FocusHandle::new());
    let picked: Log = Rc::default();
    let s = scenario(move || {
        let picked = Rc::clone(&picked);
        component((), move |_| {
            let dd = DropdownMenu::new(Signal::new(true), MenuOptions::DEFAULT);
            let label = gui::NodeRef::new();
            let picked = Rc::clone(&picked);
            move || {
                let menu = dd.menu();
                let mut rows = items(menu, &foci, &picked, false).into_iter();
                let named = primitives::menu::group(div(), Some(label))
                    .child(primitives::menu::label(row("Fruit"), label))
                    .children(rows.by_ref().take(2));
                let unnamed = primitives::menu::group(div(), None).children(rows);
                div()
                    .child(dd.trigger(row("Open"), Availability::Enabled))
                    .children(content(
                        menu,
                        below(),
                        vec![named.into_element(), unnamed.into_element()],
                    ))
                    .into_element()
            }
        })
        .into_element()
    });
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    let nodes: HashMap<NodeId, &Node> = tree.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let groups: Vec<&Node> = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == accesskit::Role::Group)
        .map(|(_, n)| n)
        .collect();
    assert_eq!(groups.len(), 2);
    let label = groups[0].labelled_by();
    assert_eq!(label.len(), 1, "the group with a label is named by it");
    let label = nodes.get(&label[0]).expect("the label's node");
    let texts: Vec<String> = std::iter::once(*label)
        .chain(label.children().iter().filter_map(|c| nodes.get(c).copied()))
        .filter_map(name)
        .collect();
    assert_eq!(texts, ["Fruit"]);
    assert!(
        label.role() != accesskit::Role::MenuItem,
        "the label is no item: keys and typeahead pass it"
    );
    assert!(groups[1].labelled_by().is_empty(), "a group without a label is unnamed");
}

// ── Context menu ──

struct Context {
    open: Signal<bool>,
    picked: Log,
    foci: [FocusHandle; 5],
    area: FocusHandle,
}

/// An area "Area" (300×200 at 100, 100), its context menu of [`ITEMS`].
fn context_menu() -> (Scenario, Context) {
    let open = Signal::new(false);
    let picked: Log = Rc::default();
    let foci = [(); 5].map(|()| FocusHandle::new());
    let area = FocusHandle::new();
    let log = Rc::clone(&picked);
    let s = scenario(move || {
        let log = Rc::clone(&log);
        component((), move |_| {
            let cm = ContextMenu::new(open, MenuOptions::DEFAULT);
            let log = Rc::clone(&log);
            move || {
                let menu = cm.menu();
                let field = div().track_focus(area).width(60.0).height(20.0).child(text("Field"));
                div()
                    .padding(100.0)
                    .child(
                        cm.area(
                            div().width(300.0).height(200.0).child(text("Area")).child(field),
                            Availability::Enabled,
                        )
                        .child(content(
                            menu,
                            cm.positioning(),
                            items(menu, &foci, &log, false),
                        )),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    (
        s,
        Context {
            open,
            picked,
            foci,
            area,
        },
    )
}

fn content_box(s: &Scenario) -> gui::Bounds {
    let (x, y) = at(s, "Apple");
    gui::Bounds {
        x,
        y,
        width: 0.0,
        height: 0.0,
    }
}

#[test]
fn a_context_menu_opens_at_the_pointer() {
    let (mut s, c) = context_menu();
    s.secondary_click(150.0, 150.0);
    s.settle().expect("settled");
    assert!(c.open.get_untracked(), "a secondary press in the area opens it");
    let first = content_box(&s);
    assert!(
        first.x > 150.0 && first.y > 150.0,
        "to the right of and below the point"
    );

    s.secondary_click(380.0, 280.0);
    s.settle().expect("settled");
    assert!(c.open.get_untracked(), "a secondary press in the area moves it");
    let moved = content_box(&s);
    assert!((moved.x - first.x - 230.0).abs() < 0.5, "by the pointer's move");
    assert!((moved.y - first.y - 130.0).abs() < 0.5);

    s.click(120.0, 120.0);
    s.settle().expect("settled");
    assert!(
        !c.open.get_untracked(),
        "a primary press in the area is outside the menu"
    );
    let _ = c.picked.borrow().len();
}

#[test]
fn a_secondary_press_on_another_area_closes_one_menu_and_opens_the_other() {
    // Areas "Left" (200×200 at 50, 50) and "Right" (200×200 at 400, 50).
    let opens = [Signal::new(false), Signal::new(false)];
    let mut s = scenario(move || {
        component((), move |_| {
            let menus = opens.map(|open| ContextMenu::new(open, MenuOptions::DEFAULT));
            move || {
                let areas = menus.iter().zip(["Left", "Right"]).map(|(cm, name)| {
                    let menu = cm.menu();
                    cm.area(
                        div().width(200.0).height(200.0).child(text(name)),
                        Availability::Enabled,
                    )
                    .child(content(
                        menu,
                        cm.positioning(),
                        vec![row(&format!("{name} item")).into_element()],
                    ))
                    .into_element()
                });
                div()
                    .padding(50.0)
                    .display(Display::Flex)
                    .gap(150.0)
                    .children(areas)
                    .into_element()
            }
        })
        .into_element()
    });
    s.secondary_click(100.0, 200.0);
    s.settle().expect("settled");
    assert!(opens[0].get_untracked(), "Left's menu opens");
    s.secondary_click(500.0, 100.0);
    s.settle().expect("settled");
    assert!(
        !opens[0].get_untracked(),
        "a secondary press on Right closes Left's menu"
    );
    assert!(
        opens[1].get_untracked(),
        "and opens Right's in the same press (reka's `ContextMenu.browser.test.ts`)"
    );
    let (x, y) = at(&s, "Right item");
    assert!(x > 500.0 && y > 100.0, "at the point");
    s.secondary_click(700.0, 500.0);
    s.settle().expect("settled");
    assert!(
        !opens[0].get_untracked() && !opens[1].get_untracked(),
        "a secondary press on no area closes the menu and opens nothing"
    );
}

#[test]
fn a_context_menu_opens_on_a_700_ms_long_press() {
    let (mut s, c) = context_menu();
    let touch = |phase, x, y| {
        Input::Touch(TouchEvent {
            id: 1,
            kind: PointerKind::Touch,
            phase,
            x,
            y,
        })
    };
    s.send(touch(TouchPhase::Started, 150.0, 150.0));
    wait(&mut s, 650);
    assert!(!c.open.get_untracked(), "not before 700 ms");
    wait(&mut s, 60);
    assert!(c.open.get_untracked(), "a touch held 700 ms opens it");
    s.send(touch(TouchPhase::Ended, 150.0, 150.0));
    s.settle().expect("settled");

    let (mut s, c) = context_menu();
    s.send(touch(TouchPhase::Started, 150.0, 150.0));
    wait(&mut s, 100);
    s.send(touch(TouchPhase::Moved, 165.0, 150.0));
    wait(&mut s, 700);
    assert!(!c.open.get_untracked(), "a touch moved past the slop is no long press");
}

#[test]
fn a_context_menu_opens_over_an_unavailable_item_on_right_click_and_long_press() {
    // An area wrapping an unavailable item (100×40 at its top left, 100,
    // 100 in the window): a right click and a long press on the item are
    // the area's gestures, as on the web; the item's own click does not run.
    let open = Signal::new(false);
    let item_clicks = Rc::new(std::cell::Cell::new(0_u32));
    let clicks = Rc::clone(&item_clicks);
    let focus = FocusHandle::new();
    let mut s = scenario(move || {
        let clicks = Rc::clone(&clicks);
        component((), move |_| {
            let cm = ContextMenu::new(open, MenuOptions::DEFAULT);
            let clicks = Rc::clone(&clicks);
            move || {
                let clicks = Rc::clone(&clicks);
                let menu = cm.menu();
                let item = Availability::Unavailable.apply(
                    div()
                        .width(100.0)
                        .height(40.0)
                        .track_focus(focus)
                        .on_click(move |_| clicks.set(clicks.get() + 1))
                        .child(text("Locked")),
                );
                div()
                    .padding(100.0)
                    .child(
                        cm.area(div().width(300.0).height(200.0).child(item), Availability::Enabled)
                            .child(content(menu, cm.positioning(), vec![text("Menu").into_element()])),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    s.secondary_click(120.0, 120.0);
    s.settle().expect("settled");
    assert!(open.get_untracked(), "a right click on the unavailable item opens it");
    s.act("close", || open.set(false));
    s.settle().expect("settled");
    assert!(!open.get_untracked());

    let touch = |phase, x, y| {
        Input::Touch(TouchEvent {
            id: 1,
            kind: PointerKind::Touch,
            phase,
            x,
            y,
        })
    };
    s.send(touch(TouchPhase::Started, 120.0, 120.0));
    wait(&mut s, 750);
    assert!(open.get_untracked(), "a long press on the unavailable item opens it");
    s.send(touch(TouchPhase::Ended, 120.0, 120.0));
    s.settle().expect("settled");
    s.act("close", || open.set(false));
    s.settle().expect("settled");
    s.click(120.0, 120.0);
    s.settle().expect("settled");
    assert!(focused(focus), "a press focuses it");
    assert_eq!(item_clicks.get(), 0, "the unavailable item is never activated");
}

#[test]
fn the_context_menu_key_opens_it_on_its_first_item() {
    let (mut s, c) = context_menu();
    s.act("focus in the area", || c.area.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::ContextMenu);
    assert!(c.open.get_untracked());
    assert!(focused(c.foci[0]), "on its first item");
    plain(&mut s, NamedKey::Escape);
    assert!(focused(c.area), "closing gives focus back to what held it");
    key(
        &mut s,
        NamedKey::F10,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert!(c.open.get_untracked(), "Shift+F10 too");
}

#[test]
fn what_a_screen_reader_is_told_of_a_context_menu() {
    let (mut s, _c) = context_menu();
    s.secondary_click(150.0, 150.0);
    s.settle().expect("settled");
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("menus_context_a11y", dump(&tree));
}

// ── Menubar ──

struct Bar {
    value: Signal<String>,
    triggers: Signal<Vec<FocusHandle>>,
}

/// A bar of File, Edit (disabled when `edit_disabled`), View, each with
/// two items named after it.
fn menubar(edit_disabled: bool) -> (Scenario, Bar) {
    let value = Signal::new(String::new());
    let triggers = Signal::new(Vec::new());
    let s = scenario(move || {
        component((), move |_| {
            let bar = Menubar::new(value, Wrap::Stop);
            move || {
                let menus = ["File", "Edit", "View"].map(|name| {
                    component((), move |_| {
                        let mm = MenubarMenu::new(bar, name.to_lowercase(), MenuOptions::DEFAULT);
                        hand_out_push(triggers, mm.menu().trigger_focus());
                        let foci = [(); 2].map(|()| FocusHandle::new());
                        move || {
                            let menu = mm.menu();
                            let usable = if edit_disabled && name == "Edit" {
                                Availability::Disabled
                            } else {
                                Availability::Enabled
                            };
                            let kids = foci
                                .iter()
                                .enumerate()
                                .map(|(i, f)| menu_item(menu, &format!("{name} {i}"), *f))
                                .collect();
                            div()
                                .child(mm.trigger(div().width(60.0).height(24.0).child(text(name)), name, usable))
                                .child(content(menu, below(), kids))
                                .into_element()
                        }
                    })
                    .into_element()
                });
                div()
                    .padding(20.0)
                    .child(bar.root(div().display(Display::Flex).gap(10.0).children(menus)))
                    .into_element()
            }
        })
        .into_element()
    });
    (s, Bar { value, triggers })
}

fn bar_trigger(b: &Bar, i: usize) -> FocusHandle {
    b.triggers.get_untracked()[i]
}

#[test]
fn a_menubar_switches_menus_by_pointer_and_keys() {
    let (mut s, b) = menubar(false);
    let (fx, fy) = at(&s, "File");
    s.hover(fx, fy);
    s.send(Input::MouseDown { x: fx, y: fy });
    s.settle().expect("settled");
    assert_eq!(
        b.value.get_untracked(),
        "file",
        "a press opens its menu as it goes down"
    );
    s.send(Input::MouseUp { x: fx, y: fy });
    s.settle().expect("settled");
    assert_eq!(b.value.get_untracked(), "file", "the release does nothing");

    let (ex, ey) = at(&s, "Edit");
    hover(&mut s, ex, ey);
    assert_eq!(b.value.get_untracked(), "edit", "pointing at another trigger switches");

    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(
        b.value.get_untracked(),
        "view",
        "the inline-end arrow in a content opens the next menu"
    );
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(b.value.get_untracked(), "view", "no wrap");
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(b.value.get_untracked(), "edit");
    plain(&mut s, NamedKey::Escape);
    assert_eq!(b.value.get_untracked(), "");
    assert!(focused(bar_trigger(&b, 1)), "Escape: focus back on its trigger");

    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(bar_trigger(&b, 2)), "the arrows move between triggers");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(bar_trigger(&b, 2)), "stopping at the end");
    plain(&mut s, NamedKey::Home);
    assert!(focused(bar_trigger(&b, 0)));
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(b.value.get_untracked(), "file", "ArrowDown opens on its first item");
    plain(&mut s, NamedKey::Escape);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(b.value.get_untracked(), "file", "Enter opens");

    let (fx, fy) = at(&s, "File");
    hover(&mut s, fx, fy);
    s.send(Input::MouseDown { x: fx, y: fy });
    s.settle().expect("settled");
    assert_eq!(
        b.value.get_untracked(),
        "",
        "a press on the open menu's trigger closes it"
    );
}

#[test]
fn a_disabled_menubar_trigger_is_passed_and_not_opened_by_hover() {
    let (mut s, b) = menubar(true);
    click(&mut s, "File");
    let (ex, ey) = at(&s, "Edit");
    hover(&mut s, ex, ey);
    assert_eq!(
        b.value.get_untracked(),
        "file",
        "hover does not open a disabled trigger (reka does)"
    );
    plain(&mut s, NamedKey::Escape);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(bar_trigger(&b, 2)), "the arrows pass it");
}

#[test]
fn what_a_screen_reader_is_told_of_a_menubar() {
    let (mut s, _b) = menubar(true);
    click(&mut s, "File");
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("menus_menubar_a11y", dump(&tree));
}

// ── Navigation menu ──

struct Nav {
    value: Signal<Option<String>>,
    menu: Signal<Option<NavigationMenu>>,
    triggers: Signal<Vec<FocusHandle>>,
    links: [FocusHandle; 2],
    followed: Log,
}

/// Items "One" and "Two" with panels of two links, and a list link "Docs".
fn navigation(options: NavOptions) -> (Scenario, Nav) {
    let value = Signal::new(None);
    let menu = Signal::new(None);
    let triggers = Signal::new(Vec::new());
    let links = [(); 2].map(|()| FocusHandle::new());
    let followed: Log = Rc::default();
    let log = Rc::clone(&followed);
    let s = scenario(move || {
        let log = Rc::clone(&log);
        component((), move |_| {
            let nav = NavigationMenu::new(value, options);
            hand_out(menu, Some(nav));
            let docs = FocusHandle::new();
            let log = Rc::clone(&log);
            move || {
                let items =
                    ["One", "Two"].map(|name| {
                        let log = Rc::clone(&log);
                        component((), move |_| {
                            let item = NavigationMenuItem::new(nav, name.to_lowercase());
                            hand_out_push(triggers, item.trigger_focus());
                            let own = [(); 2].map(|()| FocusHandle::new());
                            let log = Rc::clone(&log);
                            move || {
                                let foci = if name == "One" { links } else { own };
                                let panel = item.is_open().then(|| {
                                    let kids: Vec<Element> = foci
                                        .iter()
                                        .enumerate()
                                        .map(|(i, f)| {
                                            let label = format!("{name} link {i}");
                                            let log = Rc::clone(&log);
                                            let shown = label.clone();
                                            item.link(row(&label), *f, i == 0, move |()| {
                                                log.borrow_mut().push(shown.clone());
                                                Dismiss::Close
                                            })
                                            .into_element()
                                        })
                                        .collect();
                                    deferred(below(), item.content(div().width(160.0).children(kids))).into_element()
                                });
                                item.root(div())
                                    .child(item.trigger(
                                        div().width(60.0).height(24.0).child(text(name)),
                                        Availability::Enabled,
                                    ))
                                    .child(panel)
                                    .into_element()
                            }
                        })
                        .into_element()
                    });
                let log = Rc::clone(&log);
                let docs_link = nav.link(
                    div().width(60.0).height(24.0).child(text("Docs")),
                    docs,
                    false,
                    move |()| {
                        log.borrow_mut().push("Docs".to_owned());
                        Dismiss::Close
                    },
                );
                div()
                    .padding(20.0)
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .gap(300.0)
                    .child(
                        nav.root(div()).child(
                            nav.list(div().display(Display::Flex).gap(10.0))
                                .children(items)
                                .child(docs_link),
                        ),
                    )
                    .child(div().width(200.0).height(30.0).child(text("Page")))
                    .into_element()
            }
        })
        .into_element()
    });
    (
        s,
        Nav {
            value,
            menu,
            triggers,
            links,
            followed,
        },
    )
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).expect("ms")
}

fn open_one(n: &Nav) -> bool {
    n.value.get_untracked().as_deref() == Some("one")
}

#[test]
fn a_navigation_menu_opens_on_hover_after_200_ms_and_closes_150_ms_after_leaving() {
    let (mut s, n) = navigation(NavOptions::DEFAULT);
    let (x, y) = at(&s, "One");
    hover(&mut s, x, y);
    wait(&mut s, ms(OPEN_DELAY) - 20);
    assert!(n.value.get_untracked().is_none(), "not before 200 ms");
    wait(&mut s, 30);
    assert!(open_one(&n), "200 ms after the pointer moved on the trigger");

    let (tx, ty) = at(&s, "Two");
    hover(&mut s, tx, ty);
    assert_eq!(
        n.value.get_untracked().as_deref(),
        Some("two"),
        "while one is open, another opens at once"
    );

    hover(&mut s, 700.0, 500.0);
    wait(&mut s, ms(CLOSE_DELAY) - 20);
    assert!(n.value.get_untracked().is_some(), "not closed before 150 ms");
    wait(&mut s, 30);
    assert!(n.value.get_untracked().is_none(), "150 ms after leaving");

    hover(&mut s, x, y);
    wait(&mut s, ms(SKIPPED_DELAY) + 10);
    assert!(open_one(&n), "just after one closed, the next opens after 150 ms");
}

#[test]
fn the_pointer_in_the_panel_holds_it_open() {
    let (mut s, n) = navigation(NavOptions::DEFAULT);
    let (x, y) = at(&s, "One");
    hover(&mut s, x, y);
    wait(&mut s, 210);
    let (lx, ly) = at(&s, "One link 1");
    hover(&mut s, lx, ly);
    wait(&mut s, 400);
    assert!(open_one(&n), "entering the panel drops the close");
    hover(&mut s, 700.0, 500.0);
    wait(&mut s, 160);
    assert!(n.value.get_untracked().is_none(), "leaving the panel closes it");
}

#[test]
fn a_click_toggles_but_not_right_after_the_hover_opened_it() {
    let (mut s, n) = navigation(NavOptions::DEFAULT);
    click(&mut s, "One");
    assert!(
        n.value.get_untracked().is_none(),
        "a click within 300 ms of the move that asked to open is ignored"
    );
    wait(&mut s, 210);
    assert!(open_one(&n), "the hover opened it");
    wait(&mut s, 150);
    click(&mut s, "One");
    assert!(n.value.get_untracked().is_none(), "past 300 ms a click closes it");
    let (x, y) = at(&s, "One");
    hover(&mut s, x + 1.0, y);
    wait(&mut s, 300);
    assert!(
        n.value.get_untracked().is_none(),
        "after a click closed it, the hover does not open it again"
    );
    hover(&mut s, 700.0, 500.0);
    hover(&mut s, x, y);
    wait(&mut s, 210);
    assert!(open_one(&n), "until the pointer leaves and comes back");
}

#[test]
fn escape_closes_a_panel_and_focuses_its_trigger() {
    let (mut s, n) = navigation(NavOptions::DEFAULT);
    let one = n.triggers.get_untracked()[0];
    s.act("focus One", || one.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Enter);
    assert!(open_one(&n), "Enter toggles");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(n.links[0]), "ArrowDown on the open trigger enters the panel");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(n.links[1]));
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(n.links[1]), "no wrap in a panel");
    plain(&mut s, NamedKey::Escape);
    assert!(n.value.get_untracked().is_none());
    assert!(focused(one), "Escape focuses the trigger");

    // The pointer on the trigger as Escape closes it does not open it again.
    let (mut s, n) = navigation(NavOptions::DEFAULT);
    let (x, y) = at(&s, "One");
    hover(&mut s, x, y);
    wait(&mut s, 210);
    assert!(open_one(&n));
    plain(&mut s, NamedKey::Escape);
    assert!(n.value.get_untracked().is_none(), "Escape closes it wherever focus is");
    hover(&mut s, x + 1.0, y);
    wait(&mut s, 300);
    assert!(
        n.value.get_untracked().is_none(),
        "the pointer moving on it does not open it again"
    );
    hover(&mut s, 700.0, 500.0);
    hover(&mut s, x, y);
    wait(&mut s, 210);
    assert!(open_one(&n), "until it leaves and comes back");
}

#[test]
fn arrows_move_between_triggers_and_links_of_the_list() {
    let (mut s, n) = navigation(NavOptions::DEFAULT);
    let [one, two] = [0, 1].map(|i| n.triggers.get_untracked()[i]);
    s.act("focus One", || one.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(two));
    plain(&mut s, NamedKey::ArrowDown);
    assert!(
        n.value.get_untracked().is_none(),
        "ArrowDown on a closed trigger moves on"
    );
    plain(&mut s, NamedKey::ArrowLeft);
    plain(&mut s, NamedKey::ArrowUp);
    assert!(focused(one), "no wrap at the start");
    plain(&mut s, NamedKey::End);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(*n.followed.borrow(), ["Docs"], "a link of the list follows");
}

#[test]
fn a_press_outside_or_focus_outside_closes_but_not_on_the_menu() {
    let (mut s, n) = navigation(NavOptions::DEFAULT);
    let one = n.triggers.get_untracked()[0];
    s.act("focus One", || one.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Enter);
    let two = n.triggers.get_untracked()[1];
    s.act("focus Two", || two.focus());
    s.settle().expect("settled");
    assert!(open_one(&n), "focus on another trigger of the menu is not outside");
    click(&mut s, "Page");
    assert!(n.value.get_untracked().is_none(), "a press outside closes it");

    let (mut s, n) = navigation(NavOptions::DEFAULT);
    let one = n.triggers.get_untracked()[0];
    s.act("focus One", || one.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Enter);
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(*n.followed.borrow(), ["One link 0"]);
    assert!(n.value.get_untracked().is_none(), "following a link closes the menu");
    assert!(focused(one), "and focus goes back to the trigger");
}

#[test]
fn what_a_screen_reader_is_told_of_a_navigation_menu() {
    let (mut s, n) = navigation(NavOptions::DEFAULT);
    let one = n.triggers.get_untracked()[0];
    s.act("focus One", || one.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Enter);
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("menus_navigation_a11y", dump(&tree));
}

#[test]
fn the_indicator_follows_the_open_trigger_and_keeps_its_place_while_hidden() {
    let (mut s, n) = navigation(NavOptions::DEFAULT);
    let nav = n.menu.get_untracked().expect("the menu");
    let place = || gui::untracked(|| nav.active_trigger());
    assert!(place().is_none(), "no place before anything opened");
    let [one, two] = [0, 1].map(|i| n.triggers.get_untracked()[i]);
    s.act("focus One", || one.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Enter);
    let at_one = place().expect("One's box");
    let (ox, _) = at(&s, "One");
    assert!(ox >= at_one.x && ox <= at_one.right(), "at the open trigger");
    s.act("focus Two", || two.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Enter);
    let at_two = place().expect("Two's box");
    let (tx, _) = at(&s, "Two");
    assert!(
        tx >= at_two.x && tx <= at_two.right(),
        "it follows the trigger now open"
    );
    assert!(at_two.x > at_one.x);
    plain(&mut s, NamedKey::Enter);
    assert!(n.value.get_untracked().is_none());
    assert_eq!(
        place(),
        Some(at_two),
        "closed, it keeps its last place (it hides where it was)"
    );
}

// ── Toolbar ──

fn toolbar(orientation: Axis, wrap: Wrap, direction: Option<gui::Direction>) -> (Scenario, [FocusHandle; 4], Log) {
    let foci = [(); 4].map(|()| FocusHandle::new());
    let followed: Log = Rc::default();
    let log = Rc::clone(&followed);
    let s = scenario(move || {
        let log = Rc::clone(&log);
        component((), move |_| {
            let bar = Toolbar::new(orientation, wrap, direction);
            let log = Rc::clone(&log);
            move || {
                let button = |i: usize, name: &str, availability| {
                    bar.item(
                        div().width(50.0).height(24.0).child(text(name.to_owned())),
                        foci[i],
                        availability,
                    )
                    .into_element()
                };
                let log = Rc::clone(&log);
                let direction = match orientation {
                    Axis::Horizontal => FlexDirection::Row,
                    Axis::Vertical => FlexDirection::Column,
                };
                div()
                    .padding(20.0)
                    .child(
                        bar.root(div().display(Display::Flex).flex_direction(direction).gap(4.0))
                            .child(button(0, "Bold", Availability::Enabled))
                            .child(button(1, "Italic", Availability::Disabled))
                            .child(bar.separator(div().width(1.0).height(24.0)))
                            .child(button(2, "Left", Availability::Enabled))
                            .child(bar.link(
                                div().width(50.0).height(24.0).child(text("Help")),
                                foci[3],
                                Availability::Enabled,
                                move || log.borrow_mut().push("Help".to_owned()),
                            )),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    (s, foci, followed)
}

#[test]
fn a_toolbar_is_one_tab_stop_with_arrows_along_it() {
    let (mut s, f, followed) = toolbar(Axis::Horizontal, Wrap::Stop, None);
    plain(&mut s, NamedKey::Tab);
    assert!(focused(f[0]), "Tab enters at the first enabled item");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(f[0]), "the other axis does nothing");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(f[2]), "a disabled item is passed");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(f[3]));
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(f[3]), "no wrap: reka's loop is off");
    plain(&mut s, NamedKey::Space);
    assert_eq!(*followed.borrow(), ["Help"], "Space follows a link");
    plain(&mut s, NamedKey::Home);
    assert!(focused(f[0]));

    let (mut s, f, _) = toolbar(Axis::Vertical, Wrap::Around, None);
    plain(&mut s, NamedKey::Tab);
    plain(&mut s, NamedKey::ArrowUp);
    assert!(focused(f[3]), "wrapping when asked, along a vertical toolbar");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(f[3]), "the horizontal arrows do nothing there");
}

#[test]
fn a_horizontal_toolbar_swaps_its_arrows_right_to_left() {
    let (mut s, f, _) = toolbar(Axis::Horizontal, Wrap::Stop, Some(gui::Direction::Rtl));
    plain(&mut s, NamedKey::Tab);
    assert!(focused(f[0]));
    plain(&mut s, NamedKey::ArrowLeft);
    assert!(focused(f[2]), "ArrowLeft moves on right to left");
    plain(&mut s, NamedKey::ArrowLeft);
    assert!(focused(f[3]));
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(f[2]), "ArrowRight moves back");
    plain(&mut s, NamedKey::Home);
    assert!(focused(f[0]), "Home is the first item still");

    let (mut s, f, _) = toolbar(Axis::Vertical, Wrap::Stop, Some(gui::Direction::Rtl));
    plain(&mut s, NamedKey::Tab);
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(f[2]), "a vertical toolbar's arrows do not swap");
}

/// A toolbar of "Bold", a dropdown trigger "Open" (made an item before or
/// after it is made the trigger, as `trigger_last` says) and "After".
struct MenuInToolbar {
    buttons: [FocusHandle; 2],
    items: [FocusHandle; 5],
    open: Signal<bool>,
    trigger: Signal<Option<FocusHandle>>,
}

fn menu_in_toolbar(orientation: Axis, trigger_last: bool) -> (Scenario, MenuInToolbar) {
    let buttons = [(); 2].map(|()| FocusHandle::new());
    let foci = [(); 5].map(|()| FocusHandle::new());
    let open = Signal::new(false);
    let trigger = Signal::new(None);
    let s = scenario(move || {
        component((), move |_| {
            let bar = Toolbar::new(orientation, Wrap::Stop, None);
            let dd = DropdownMenu::new(open, MenuOptions::DEFAULT);
            hand_out(trigger, Some(dd.menu().trigger_focus()));
            let log: Log = Rc::default();
            move || {
                let menu = dd.menu();
                let button =
                    |i: usize, name: &str| bar.item(row(name), buttons[i], Availability::Enabled).into_element();
                let face = div().width(80.0).height(24.0).child(text("Open"));
                let trigger = if trigger_last {
                    dd.trigger(
                        bar.item(face, menu.trigger_focus(), Availability::Enabled),
                        Availability::Enabled,
                    )
                } else {
                    bar.item(
                        dd.trigger(face, Availability::Enabled),
                        menu.trigger_focus(),
                        Availability::Enabled,
                    )
                };
                let direction = match orientation {
                    Axis::Horizontal => FlexDirection::Row,
                    Axis::Vertical => FlexDirection::Column,
                };
                div()
                    .padding(20.0)
                    .child(
                        bar.root(div().display(Display::Flex).flex_direction(direction).gap(4.0))
                            .child(button(0, "Bold"))
                            .child(trigger)
                            .child(button(1, "After")),
                    )
                    .children(content(menu, below(), items(menu, &foci, &log, false)))
                    .into_element()
            }
        })
        .into_element()
    });
    (
        s,
        MenuInToolbar {
            buttons,
            items: foci,
            open,
            trigger,
        },
    )
}

#[test]
fn a_menu_trigger_in_a_toolbar_keeps_its_keys() {
    for trigger_last in [false, true] {
        let (mut s, m) = menu_in_toolbar(Axis::Horizontal, trigger_last);
        let trigger = m.trigger.get_untracked().expect("the trigger's focus");
        plain(&mut s, NamedKey::Tab);
        assert!(focused(m.buttons[0]));
        plain(&mut s, NamedKey::ArrowRight);
        assert!(focused(trigger), "the toolbar's arrows reach the trigger");
        plain(&mut s, NamedKey::ArrowDown);
        assert!(m.open.get_untracked(), "ArrowDown opens the menu ({trigger_last})");
        assert!(focused(m.items[0]), "on its first item");
        plain(&mut s, NamedKey::ArrowRight);
        assert!(focused(m.items[0]), "the menu's keys are not the toolbar's");
        plain(&mut s, NamedKey::Escape);
        assert!(!m.open.get_untracked());
        assert!(focused(trigger), "focus comes back to the trigger");
        plain(&mut s, NamedKey::ArrowRight);
        assert!(focused(m.buttons[1]), "the arrows it leaves move along the toolbar");
        plain(&mut s, NamedKey::ArrowLeft);
        plain(&mut s, NamedKey::Enter);
        assert!(m.open.get_untracked(), "Enter opens it");
        plain(&mut s, NamedKey::Escape);

        let (mut s, m) = menu_in_toolbar(Axis::Vertical, trigger_last);
        let trigger = m.trigger.get_untracked().expect("the trigger's focus");
        plain(&mut s, NamedKey::Tab);
        plain(&mut s, NamedKey::ArrowDown);
        assert!(focused(trigger), "down a vertical toolbar to the trigger");
        plain(&mut s, NamedKey::ArrowDown);
        assert!(
            m.open.get_untracked(),
            "ArrowDown opens the menu in a vertical toolbar too ({trigger_last})"
        );
        assert!(!focused(m.buttons[1]), "instead of moving focus");
        plain(&mut s, NamedKey::Escape);
        assert!(focused(trigger));
        plain(&mut s, NamedKey::ArrowUp);
        assert!(
            focused(m.buttons[0]),
            "ArrowUp, which the trigger leaves, moves up the toolbar"
        );
    }
}

#[test]
fn what_a_screen_reader_is_told_of_a_toolbar() {
    let (s, _, _) = toolbar(Axis::Horizontal, Wrap::Stop, None);
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("menus_toolbar_a11y", dump(&tree));
}

fn name(n: &Node) -> Option<String> {
    n.label().or_else(|| n.value()).map(str::to_owned)
}

/// The tree from the window down, a line per node: role, name, flags and
/// relations by their targets' names.
fn dump(update: &TreeUpdate) -> String {
    let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let named = |id: &NodeId| {
        nodes
            .get(id)
            .and_then(|n| name(n).or_else(|| Some(format!("{:?}", n.role()))))
            .unwrap_or_else(|| "?".to_owned())
    };
    let mut out = String::new();
    let mut stack = vec![(NodeId(0), 0_usize)];
    while let Some((id, depth)) = stack.pop() {
        let Some(n) = nodes.get(&id) else { continue };
        let mut line = format!("{}{:?}", "  ".repeat(depth), n.role());
        if let Some(nm) = name(n) {
            line.push_str(&format!(" {nm:?}"));
        }
        if let Some(o) = n.orientation() {
            line.push_str(&format!(" {o:?}"));
        }
        if let Some(c) = n.toggled() {
            line.push_str(&format!(" checked={c:?}"));
        }
        if let Some(e) = n.is_expanded() {
            line.push_str(&format!(" expanded={e}"));
        }
        if let Some(p) = n.has_popup() {
            line.push_str(&format!(" haspopup={p:?}"));
        }
        if let Some(c) = n.aria_current() {
            line.push_str(&format!(" current={c:?}"));
        }
        if n.is_disabled() {
            line.push_str(" disabled");
        }
        if n.is_hidden() {
            line.push_str(" hidden");
        }
        for (what, ids) in [
            ("labelled-by", n.labelled_by()),
            ("controls", n.controls()),
            ("owns", n.owns()),
        ] {
            if !ids.is_empty() {
                let names: Vec<String> = ids.iter().map(named).collect();
                line.push_str(&format!(" {what}={}", names.join(",")));
            }
        }
        out.push_str(&line);
        out.push('\n');
        for c in n.children().iter().rev() {
            stack.push((*c, depth + 1));
        }
    }
    out
}

#[test]
fn focus_comes_back_to_the_trigger() {
    let (mut s, d) = dropdown(MenuOptions::DEFAULT, false, Availability::Enabled);
    gui::focus::set_focus(Some(d.page.id()), false);
    click(&mut s, "Open");
    click(&mut s, "Apple");
    assert!(
        focused(trigger_of(&d)),
        "a pick gives focus to the trigger, not to what held it before (reka's `onCloseAutoFocus`)"
    );

    click(&mut s, "Open");
    let (x, y) = at(&s, "Banana");
    hover(&mut s, x, y);
    plain(&mut s, NamedKey::Escape);
    assert!(focused(trigger_of(&d)));
    plain(&mut s, NamedKey::Enter);
    assert!(
        focused(d.foci[0]),
        "Enter opens on the first item, the mouse resting on another"
    );
}

// ── Review fixes (ADR-0025 "Group 4: review fixes") ──

#[test]
fn a_submenu_stays_open_when_the_highlight_merely_goes_away() {
    let (mut s, m) = with_submenu();
    let (x, y) = at(&s, "More");
    hover(&mut s, x, y);
    wait(&mut s, 120);
    assert!(m.sub_open.get_untracked(), "the rest opened it");
    // Into the parent content's empty right edge (the rows are 140 px of
    // its 160), past the grace period, then a further move.
    let (_, fy) = at(&s, "First");
    hover(&mut s, x + 75.0, fy);
    wait(&mut s, u64::try_from(GRACE.as_millis()).expect("ms") + 50);
    hover(&mut s, x + 76.0, fy);
    assert!(
        m.root_items.iter().all(|f| !focused(*f)),
        "no item is highlighted: the content holds focus"
    );
    assert!(
        m.sub_open.get_untracked(),
        "the highlight going away keeps the submenu open: it may cover its trigger"
    );
    // Focus leaving the submenu does close it.
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(m.sub_items[0]));
    s.act("focus the first item", || m.root_items[0].focus());
    s.settle().expect("settled");
    assert!(
        !m.sub_open.get_untracked(),
        "focus leaving it for the parent's item closes it"
    );
}

#[test]
fn a_press_elsewhere_released_on_an_item_picks_it() {
    // A press on one item released on another picks the other (reka's
    // `pointerup`); a press and release on the same item is its click.
    let (mut s, d) = dropdown(MenuOptions::DEFAULT, false, Availability::Enabled);
    click(&mut s, "Open");
    let (ax, ay) = at(&s, "Apple");
    let (bx, by) = at(&s, "Banana");
    s.hover(ax, ay);
    s.send(Input::MouseDown { x: ax, y: ay });
    s.settle().expect("settled");
    s.hover(bx, by);
    s.send(Input::MouseUp { x: bx, y: by });
    s.settle().expect("settled");
    assert_eq!(
        *d.picked.borrow(),
        ["Banana"],
        "released on Banana, it picks Banana once"
    );
    assert!(!d.open.get_untracked());

    // A menubar trigger pressed, dragged to an item and released picks it.
    let (mut s, b) = menubar(false);
    let (fx, fy) = at(&s, "File");
    s.hover(fx, fy);
    s.send(Input::MouseDown { x: fx, y: fy });
    s.settle().expect("settled");
    assert_eq!(b.value.get_untracked(), "file");
    let (ix, iy) = at(&s, "File 1");
    s.hover(ix, iy);
    s.settle().expect("settled");
    s.send(Input::MouseUp { x: ix, y: iy });
    s.settle().expect("settled");
    assert_eq!(
        b.value.get_untracked(),
        "",
        "released on an item, it picks it and closes"
    );
}

#[test]
fn a_long_press_lifted_on_an_item_picks_it_and_the_delay_is_the_callers() {
    let (mut s, c) = context_menu();
    let touch = |phase, x, y| {
        Input::Touch(TouchEvent {
            id: 1,
            kind: PointerKind::Touch,
            phase,
            x,
            y,
        })
    };
    s.send(touch(TouchPhase::Started, 150.0, 150.0));
    wait(&mut s, 710);
    assert!(c.open.get_untracked());
    let (bx, by) = at(&s, "Banana");
    s.send(touch(TouchPhase::Moved, bx, by));
    s.settle().expect("settled");
    assert!(
        c.open.get_untracked(),
        "moving after the long press scrolls and closes nothing"
    );
    s.send(touch(TouchPhase::Ended, bx, by));
    s.settle().expect("settled");
    assert_eq!(
        *c.picked.borrow(),
        ["Banana"],
        "the same contact lifted on an item picks it"
    );
    assert!(!c.open.get_untracked());

    // reka's `pressOpenDelay`: a caller's 300 ms.
    let open = Signal::new(false);
    let mut s = scenario(move || {
        component((), move |_| {
            let cm = ContextMenu::new(open, MenuOptions::DEFAULT).press_open_delay(Duration::from_millis(300));
            move || {
                let menu = cm.menu();
                div()
                    .padding(100.0)
                    .child(
                        cm.area(
                            div().width(300.0).height(200.0).child(text("Area")),
                            Availability::Enabled,
                        )
                        .child(content(
                            menu,
                            cm.positioning(),
                            vec![text("Menu").into_element()],
                        )),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    s.send(touch(TouchPhase::Started, 150.0, 150.0));
    wait(&mut s, 250);
    assert!(!open.get_untracked(), "not before the caller's delay");
    wait(&mut s, 60);
    assert!(open.get_untracked(), "a touch held 300 ms opens it");
}

/// A dropdown of [`ITEMS`] opened, a filter field first in its content.
fn filtered() -> (Scenario, Dropdown, gui::TextField, Signal<Option<Menu>>) {
    filtered_in(None)
}

/// [`filtered`], its items in a scroll container two rows high tracked by
/// `scroll` when given (the content's `Menu::scroll_with`).
fn filtered_in(scroll: Option<gui::ScrollHandle>) -> (Scenario, Dropdown, gui::TextField, Signal<Option<Menu>>) {
    use primitives::dropdown_menu::DropdownMenuFilter;
    let open = Signal::new(true);
    let trigger_focus = Signal::new(None);
    let foci = [(); 5].map(|()| FocusHandle::new());
    let picked: Log = Rc::default();
    let page = FocusHandle::new();
    let field = gui::TextField::new("");
    let level = Signal::new(None);
    let log = Rc::clone(&picked);
    let s = scenario(move || {
        let log = Rc::clone(&log);
        let scroll = scroll.clone();
        component((), move |_| {
            let dd = DropdownMenu::new(open, MenuOptions::DEFAULT);
            hand_out(trigger_focus, Some(dd.menu().trigger_focus()));
            hand_out(level, Some(dd.menu()));
            if let Some(handle) = &scroll {
                dd.menu().scroll_with(handle.clone());
            }
            let log = Rc::clone(&log);
            let scroll = scroll.clone();
            move || {
                let menu = dd.menu();
                let search = component((), move |_| {
                    let filter = DropdownMenuFilter::new(menu, field, true, false);
                    move || filter.field(gui::text_input(&field).width(140.0)).into_element()
                })
                .into_element();
                let mut kids = vec![search];
                let rows = items(menu, &foci, &log, false);
                match &scroll {
                    Some(handle) => kids.push(
                        div()
                            .height(48.0)
                            .overflow_y(Overflow::Scroll)
                            .track_scroll(handle)
                            .children(rows)
                            .into_element(),
                    ),
                    None => kids.extend(rows),
                }
                div()
                    .child(dd.trigger(
                        div().width(80.0).height(30.0).child(text("Open")),
                        Availability::Enabled,
                    ))
                    .child(content(menu, below(), kids))
                    .into_element()
            }
        })
        .into_element()
    });
    (
        s,
        Dropdown {
            open,
            trigger_focus,
            foci,
            picked,
            page,
        },
        field,
        level,
    )
}

#[test]
fn a_filter_keeps_focus_while_the_keys_move_the_highlight() {
    let (mut s, d, field, level) = filtered();
    let menu = level.get_untracked().expect("the menu");
    assert!(focused(field.focus), "asked to, it takes focus once laid out");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(field.focus), "focus stays in the field");
    assert!(
        gui::untracked(|| menu.is_highlighted(d.foci[0])),
        "the first item is highlighted"
    );
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::ArrowDown);
    assert!(
        gui::untracked(|| menu.is_highlighted(d.foci[3])),
        "the disabled item is passed"
    );
    plain(&mut s, NamedKey::Home);
    assert!(gui::untracked(|| menu.is_highlighted(d.foci[0])));
    plain(&mut s, NamedKey::End);
    assert!(gui::untracked(|| menu.is_highlighted(d.foci[4])));
    // The pointer highlights too, focus kept in the field.
    let (bx, by) = at(&s, "Banana");
    hover(&mut s, bx, by);
    assert!(focused(field.focus));
    assert!(gui::untracked(|| menu.is_highlighted(d.foci[1])));
    // Typing types, Space included: no typeahead, no pick.
    type_char(&mut s, "b");
    type_char(&mut s, " ");
    assert_eq!(field.edit.get_untracked().text(), "b ");
    assert!(d.picked.borrow().is_empty());
    // Escape empties the field and the menu stays; once empty it closes.
    plain(&mut s, NamedKey::Escape);
    assert_eq!(field.edit.get_untracked().text(), "");
    assert!(d.open.get_untracked(), "Escape on a field with text only empties it");
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(d.picked.borrow().len(), 1, "Enter picks the highlight");
    assert!(!d.open.get_untracked());

    let (mut s, d, _field, _level) = filtered();
    plain(&mut s, NamedKey::Escape);
    assert!(!d.open.get_untracked(), "Escape on an empty field closes the menu");
}

#[test]
fn a_highlight_moved_from_the_filter_is_scrolled_into_view_the_nearest_way() {
    let scroll = gui::ScrollHandle::new();
    let (mut s, d, field, level) = filtered_in(Some(scroll.clone()));
    let menu = level.get_untracked().expect("the menu");
    let y = || gui::untracked(|| scroll.offset()).1;
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::ArrowDown);
    assert!(gui::untracked(|| menu.is_highlighted(d.foci[1])));
    assert!(y().abs() < 0.5, "a highlight in view scrolls nothing");
    plain(&mut s, NamedKey::End);
    assert!(gui::untracked(|| menu.is_highlighted(d.foci[4])));
    assert!(
        (y() - 72.0).abs() < 0.5,
        "the last row is brought in at the bottom edge: {}",
        y()
    );
    plain(&mut s, NamedKey::ArrowUp);
    assert!(gui::untracked(|| menu.is_highlighted(d.foci[3])));
    assert!((y() - 72.0).abs() < 0.5, "Date is in view already: {}", y());
    plain(&mut s, NamedKey::Home);
    assert!(y().abs() < 0.5, "the first row is brought in at the top edge: {}", y());
    assert!(focused(field.focus), "focus stays in the field throughout");
}

#[test]
fn a_filter_leaves_the_keys_to_a_composing_input_method() {
    let (mut s, _d, field, level) = filtered();
    let menu = level.get_untracked().expect("the menu");
    s.send(Input::Ime(gui::events::ImeEvent::Preedit(gui::Preedit {
        text: "ka".to_owned(),
        cursor: Some((2, 2)),
    })));
    s.settle().expect("settled");
    let before = gui::untracked(|| menu.active_node());
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(
        gui::untracked(|| menu.active_node()),
        before,
        "the arrow is the input method's"
    );
    assert!(focused(field.focus));
}

#[test]
fn what_a_screen_reader_is_told_of_a_menu_filter() {
    let (mut s, _d, _field, _level) = filtered();
    plain(&mut s, NamedKey::ArrowDown);
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    let nodes: HashMap<NodeId, &Node> = tree.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let search = tree
        .nodes
        .iter()
        .find(|(_, n)| n.role() == accesskit::Role::SearchInput)
        .expect("a searchbox");
    let active = search.1.active_descendant().expect("it names the highlight");
    let item = nodes.get(&active).expect("the highlighted node");
    assert_eq!(item.role(), accesskit::Role::MenuItem);
    let texts: Vec<String> = item
        .children()
        .iter()
        .filter_map(|c| nodes.get(c).and_then(|n| name(n)))
        .collect();
    assert_eq!(texts, ["Apple"], "the first item, highlighted");
    insta::assert_snapshot!("menus_filter_a11y", dump(&tree));
}

/// A right-to-left menubar of File (a plain item, then a sub trigger "Share"
/// with one item) and Edit.
fn menubar_rtl() -> (Scenario, Signal<String>, Signal<bool>, [FocusHandle; 3]) {
    let value = Signal::new(String::new());
    let sub_open = Signal::new(false);
    let foci = [(); 3].map(|()| FocusHandle::new());
    let s = scenario(move || {
        component((), move |_| {
            let scope = primitives::direction::provide_direction(Signal::new(gui::Direction::Rtl));
            let bar = Menubar::new(value, Wrap::Stop);
            move || {
                let file = component((), move |_| {
                    let mm = MenubarMenu::new(bar, "file", MenuOptions::DEFAULT);
                    move || {
                        let menu = mm.menu();
                        let share = component((), move |_| {
                            let sub = menu.submenu(sub_open);
                            move || {
                                let side = Positioning::Anchored {
                                    side: sub.sub_side(),
                                    align: SideAlign::Start,
                                    offset: 0.0,
                                    match_width: false,
                                    fit_height: false,
                                };
                                div()
                                    .child(sub.sub_trigger(row("Share"), "Share", Availability::Enabled))
                                    .child(content(sub, side, vec![menu_item(sub, "Mail", foci[2])]))
                                    .into_element()
                            }
                        })
                        .into_element();
                        div()
                            .child(mm.trigger(
                                div().width(60.0).height(24.0).child(text("File")),
                                "File",
                                Availability::Enabled,
                            ))
                            .child(content(menu, below(), vec![menu_item(menu, "New", foci[0]), share]))
                            .into_element()
                    }
                })
                .into_element();
                let edit = component((), move |_| {
                    let mm = MenubarMenu::new(bar, "edit", MenuOptions::DEFAULT);
                    move || {
                        let menu = mm.menu();
                        div()
                            .child(mm.trigger(
                                div().width(60.0).height(24.0).child(text("Edit")),
                                "Edit",
                                Availability::Enabled,
                            ))
                            .child(content(menu, below(), vec![menu_item(menu, "Undo", foci[1])]))
                            .into_element()
                    }
                })
                .into_element();
                scope
                    .root(div().padding(200.0))
                    .child(bar.root(div().display(Display::Flex).gap(10.0)).child(file).child(edit))
                    .into_element()
            }
        })
        .into_element()
    });
    (s, value, sub_open, foci)
}

#[test]
fn a_right_to_left_menubar_moves_on_with_the_inline_end_key_from_a_submenu() {
    let (mut s, value, sub_open, foci) = menubar_rtl();
    s.act("open File", || value.set("file".to_owned()));
    s.settle().expect("settled");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(foci[0]));
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::ArrowLeft);
    assert!(sub_open.get_untracked(), "ArrowLeft, the inline end, opens the submenu");
    assert!(focused(foci[2]));
    plain(&mut s, NamedKey::ArrowRight);
    assert!(
        !sub_open.get_untracked(),
        "ArrowRight, the inline start, closes the submenu"
    );
    assert_eq!(value.get_untracked(), "file", "and only that: the bar stays on File");
    plain(&mut s, NamedKey::ArrowLeft);
    assert!(sub_open.get_untracked());
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(
        value.get_untracked(),
        "edit",
        "from the submenu's plain item, the inline-end key opens the next menu"
    );
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(value.get_untracked(), "file", "the inline-start key the previous one");
}

#[test]
fn a_menubar_wraps_when_asked_is_one_tab_stop_and_its_value_is_the_callers() {
    // Wrap::Around: past the last menu, the first; the triggers wrap too.
    let value = Signal::new(String::new());
    let triggers = Signal::new(Vec::new());
    let page = FocusHandle::new();
    let mut s = scenario(move || {
        component((), move |_| {
            let bar = Menubar::new(value, Wrap::Around);
            move || {
                let menus = ["File", "Edit"].map(|name| {
                    component((), move |_| {
                        let mm = MenubarMenu::new(bar, name.to_lowercase(), MenuOptions::DEFAULT);
                        hand_out_push(triggers, mm.menu().trigger_focus());
                        let f = FocusHandle::new();
                        move || {
                            let menu = mm.menu();
                            div()
                                .child(mm.trigger(
                                    div().width(60.0).height(24.0).child(text(name)),
                                    name,
                                    Availability::Enabled,
                                ))
                                .child(content(menu, below(), vec![menu_item(menu, &format!("{name} 0"), f)]))
                                .into_element()
                        }
                    })
                    .into_element()
                });
                div()
                    .padding(20.0)
                    .child(bar.root(div().display(Display::Flex).gap(10.0)).children(menus))
                    .child(div().track_focus(page).width(60.0).height(24.0).child(text("Page")))
                    .into_element()
            }
        })
        .into_element()
    });
    let t = |i: usize| triggers.get_untracked()[i];
    plain(&mut s, NamedKey::Tab);
    assert!(focused(t(0)), "Tab enters the bar at its first trigger");
    plain(&mut s, NamedKey::ArrowRight);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(t(0)), "the triggers wrap");
    plain(&mut s, NamedKey::ArrowLeft);
    assert!(focused(t(1)));
    plain(&mut s, NamedKey::Tab);
    assert!(focused(page), "Tab leaves the bar: it is one Tab stop");
    key(
        &mut s,
        NamedKey::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert!(focused(t(1)), "Shift+Tab comes back to the trigger focused last");

    s.act("the caller opens Edit", || value.set("edit".to_owned()));
    s.settle().expect("settled");
    assert!(shows(&s, "Edit 0"), "the caller's value opens its menu");
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(value.get_untracked(), "file", "past the last menu, the first");
    s.act("the caller closes", || value.set(String::new()));
    s.settle().expect("settled");
    assert!(!shows(&s, "File 0"));
}

#[test]
fn a_toolbar_remembers_its_item_and_an_all_disabled_one_is_no_stop() {
    let foci = [(); 3].map(|()| FocusHandle::new());
    let [before, after] = [(); 2].map(|()| FocusHandle::new());
    let mut s = scenario(move || {
        component((), move |_| {
            let bar = Toolbar::new(Axis::Horizontal, Wrap::Stop, None);
            move || {
                div()
                    .child(div().track_focus(before).width(40.0).height(20.0).child(text("Before")))
                    .child(
                        bar.root(div().display(Display::Flex))
                            .child(bar.item(row("A"), foci[0], Availability::Enabled))
                            .child(bar.item(row("B"), foci[1], Availability::Enabled))
                            .child(bar.item(row("C"), foci[2], Availability::Enabled)),
                    )
                    .child(div().track_focus(after).width(40.0).height(20.0).child(text("After")))
                    .into_element()
            }
        })
        .into_element()
    });
    let shift_tab = |s: &mut Scenario| {
        key(
            s,
            NamedKey::Tab,
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        );
    };
    plain(&mut s, NamedKey::Tab);
    plain(&mut s, NamedKey::Tab);
    assert!(focused(foci[0]), "Tab enters at the first enabled item");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(foci[1]));
    shift_tab(&mut s);
    assert!(focused(before), "Shift+Tab leaves the toolbar");
    plain(&mut s, NamedKey::Tab);
    assert!(focused(foci[1]), "Tab enters at the item focused last");
    plain(&mut s, NamedKey::Tab);
    assert!(focused(after), "Tab leaves at once");
    shift_tab(&mut s);
    assert!(focused(foci[1]), "Shift+Tab comes back to the item focused last");

    // Every item disabled: the toolbar is no Tab stop.
    let after = FocusHandle::new();
    let foci = [(); 2].map(|()| FocusHandle::new());
    let mut s = scenario(move || {
        component((), move |_| {
            let bar = Toolbar::new(Axis::Horizontal, Wrap::Stop, None);
            move || {
                div()
                    .child(
                        bar.root(div().display(Display::Flex))
                            .child(bar.item(row("A"), foci[0], Availability::Disabled))
                            .child(bar.item(row("B"), foci[1], Availability::Disabled)),
                    )
                    .child(div().track_focus(after).width(40.0).height(20.0).child(text("After")))
                    .into_element()
            }
        })
        .into_element()
    });
    plain(&mut s, NamedKey::Tab);
    assert!(focused(after), "Tab passes a toolbar of disabled items");
    let (ax, ay) = at(&s, "A");
    s.click(ax, ay);
    s.settle().expect("settled");
    assert!(
        !focused(foci[0]),
        "a disabled item takes no focus, the pointer's either"
    );
}

#[test]
fn a_toolbar_toggle_group_is_part_of_the_toolbar() {
    use primitives::toggle_group::ToggleGroup;
    let value: Signal<Vec<String>> = Signal::new(Vec::new());
    let foci = [(); 3].map(|()| FocusHandle::new());
    let after = FocusHandle::new();
    let mut s = scenario(move || {
        component(gui::Bind::model(value), move |cx| {
            let bar = Toolbar::new(Axis::Horizontal, Wrap::Stop, None);
            let group = ToggleGroup::new(
                gui::use_model(cx.props(), |b: &gui::Bind<Vec<String>>| b),
                gui::Direction::Ltr,
            );
            move || {
                let toggle = |i: usize, name: &str| {
                    bar.toggle_item(row(name), group, name.to_owned(), foci[i], Availability::Enabled)
                        .into_element()
                };
                div()
                    .child(
                        bar.root(div().display(Display::Flex))
                            .child(bar.item(row("Cut"), foci[0], Availability::Enabled))
                            .child(
                                bar.toggle_group(div().display(Display::Flex), group)
                                    .child(toggle(1, "Bold"))
                                    .child(toggle(2, "Italic")),
                            ),
                    )
                    .child(div().track_focus(after).width(40.0).height(20.0).child(text("After")))
                    .into_element()
            }
        })
        .into_element()
    });
    plain(&mut s, NamedKey::Tab);
    assert!(focused(foci[0]));
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(foci[1]), "the toolbar's arrows reach the group's items");
    plain(&mut s, NamedKey::Space);
    assert_eq!(value.get_untracked(), ["Bold"], "Space presses the toggle");
    plain(&mut s, NamedKey::ArrowRight);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), ["Bold", "Italic"]);
    plain(&mut s, NamedKey::Tab);
    assert!(focused(after), "the whole toolbar is one Tab stop");
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("menus_toolbar_toggle_a11y", dump(&tree));
}

#[test]
fn disable_click_turns_off_the_mouse_click_only() {
    let options = NavOptions {
        disable_click: true,
        disable_hover: true,
        ..NavOptions::DEFAULT
    };
    let (mut s, n) = navigation(options);
    let one = n.triggers.get_untracked()[0];
    s.act("focus One", || one.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Enter);
    assert!(open_one(&n), "Enter still toggles");
    plain(&mut s, NamedKey::Enter);
    assert!(n.value.get_untracked().is_none());
    click(&mut s, "One");
    assert!(n.value.get_untracked().is_none(), "a mouse click does nothing");
    let (x, y) = at(&s, "One");
    let touch = |phase| {
        Input::Touch(TouchEvent {
            id: 1,
            kind: PointerKind::Touch,
            phase,
            x,
            y,
        })
    };
    s.send(touch(TouchPhase::Started));
    s.send(touch(TouchPhase::Ended));
    s.settle().expect("settled");
    assert!(open_one(&n), "a tap still toggles");
}

#[test]
fn disable_hover_and_pointer_leave_close() {
    let hoverless = NavOptions {
        disable_hover: true,
        ..NavOptions::DEFAULT
    };
    let (mut s, n) = navigation(hoverless);
    let (x, y) = at(&s, "One");
    hover(&mut s, x, y);
    wait(&mut s, 400);
    assert!(n.value.get_untracked().is_none(), "the pointer opens nothing");
    click(&mut s, "One");
    assert!(open_one(&n), "a click does");
    hover(&mut s, 700.0, 500.0);
    wait(&mut s, 300);
    assert!(open_one(&n), "leaving the trigger closes nothing");
    let (lx, ly) = at(&s, "One link 1");
    hover(&mut s, lx, ly);
    hover(&mut s, 700.0, 500.0);
    wait(&mut s, ms(CLOSE_DELAY) + 10);
    assert!(
        n.value.get_untracked().is_none(),
        "but leaving the panel still closes it after 150 ms (reka's rule)"
    );

    let kept = NavOptions {
        disable_pointer_leave_close: true,
        ..NavOptions::DEFAULT
    };
    let (mut s, n) = navigation(kept);
    hover(&mut s, x, y);
    wait(&mut s, 210);
    assert!(open_one(&n));
    let (lx, ly) = at(&s, "One link 1");
    hover(&mut s, lx, ly);
    hover(&mut s, 700.0, 500.0);
    wait(&mut s, 400);
    assert!(open_one(&n), "leaving the panel does not close it");
}

#[test]
fn a_held_space_toggles_a_trigger_once_and_tab_enters_the_panel() {
    let (mut s, n) = navigation(NavOptions::DEFAULT);
    let [one, two] = [0, 1].map(|i| n.triggers.get_untracked()[i]);
    s.act("focus One", || one.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Space);
    assert!(open_one(&n), "Space toggles");
    let repeat = || {
        Input::KeyRepeat(gui::KeyEvent {
            key: gui::events::Key::Named(NamedKey::Space),
            text: Some(" ".to_owned()),
            modifiers: Modifiers::default(),
        })
    };
    s.send(repeat());
    s.send(repeat());
    s.settle().expect("settled");
    assert!(open_one(&n), "its auto-repeat is ignored");
    plain(&mut s, NamedKey::Tab);
    assert!(focused(n.links[0]), "Tab from the open trigger enters its panel");
    plain(&mut s, NamedKey::Tab);
    assert!(focused(n.links[1]));
    plain(&mut s, NamedKey::Tab);
    assert!(focused(two), "past its last link, the next item");
    key(
        &mut s,
        NamedKey::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert!(focused(n.links[1]), "Shift+Tab from the next item, its last link");
}

#[test]
fn a_right_to_left_navigation_menu_swaps_its_arrows() {
    let value: Signal<Option<String>> = Signal::new(None);
    let triggers = Signal::new(Vec::new());
    let links = [(); 2].map(|()| FocusHandle::new());
    let mut s = scenario(move || {
        component((), move |_| {
            let scope = primitives::direction::provide_direction(Signal::new(gui::Direction::Rtl));
            let nav = NavigationMenu::new(value, NavOptions::DEFAULT);
            move || {
                let items =
                    ["One", "Two"].map(|name| {
                        component((), move |_| {
                            let item = NavigationMenuItem::new(nav, name.to_lowercase());
                            hand_out_push(triggers, item.trigger_focus());
                            move || {
                                let panel = item.is_open().then(|| {
                                    let kids: Vec<Element> = links
                                        .iter()
                                        .enumerate()
                                        .map(|(i, f)| {
                                            item.link(row(&format!("{name} {i}")), *f, false, |()| Dismiss::Close)
                                                .into_element()
                                        })
                                        .collect();
                                    deferred(below(), item.content(div().display(Display::Flex).children(kids)))
                                        .into_element()
                                });
                                item.root(div())
                                    .child(item.trigger(
                                        div().width(60.0).height(24.0).child(text(name)),
                                        Availability::Enabled,
                                    ))
                                    .child(panel)
                                    .into_element()
                            }
                        })
                        .into_element()
                    });
                scope
                    .root(div().padding(20.0))
                    .child(
                        nav.root(div())
                            .child(nav.list(div().display(Display::Flex)).children(items)),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    let [one, two] = [0, 1].map(|i| triggers.get_untracked()[i]);
    s.act("focus One", || one.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::ArrowLeft);
    assert!(focused(two), "right to left, ArrowLeft goes to the next trigger");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(one), "and ArrowRight to the previous");
    plain(&mut s, NamedKey::Enter);
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(links[0]));
    plain(&mut s, NamedKey::ArrowLeft);
    assert!(focused(links[1]), "in a panel too");
}

/// Items "One" and "Two" whose panels a viewport below the list shows,
/// its Tab stops after the open trigger; "After" below.
fn navigation_with_viewport() -> (
    Scenario,
    Signal<Option<String>>,
    [FocusHandle; 2],
    [FocusHandle; 4],
    NavigationMenu,
) {
    let value = Signal::new(None);
    let triggers = Signal::new(Vec::new());
    let links = [(); 4].map(|()| FocusHandle::new());
    let menu: Rc<std::cell::Cell<Option<NavigationMenu>>> = Rc::default();
    let out = Rc::clone(&menu);
    let s = scenario(move || {
        let out = Rc::clone(&out);
        component((), move |_| {
            let nav = NavigationMenu::new(value, NavOptions::DEFAULT);
            out.set(Some(nav));
            move || {
                let items =
                    ["One", "Two"].map(|name| {
                        component((), move |_| {
                            let item = NavigationMenuItem::new(nav, name.to_lowercase());
                            hand_out_push(triggers, item.trigger_focus());
                            move || {
                                item.root(div())
                                    .child(item.trigger(
                                        div().width(60.0).height(24.0).child(text(name)),
                                        Availability::Enabled,
                                    ))
                                    .into_element()
                            }
                        })
                        .into_element()
                    });
                let viewport = component((), move |_| {
                    nav.use_viewport();
                    move || {
                        let Some(item) = nav.open_item() else {
                            return Element::Empty;
                        };
                        let at = if nav.item("two") == Some(item) { 2 } else { 0 };
                        let kids: Vec<Element> = (at..at + 2)
                            .map(|i| {
                                item.link(row(&format!("Link {i}")), links[i], false, |()| Dismiss::Close)
                                    .into_element()
                            })
                            .collect();
                        let panel = item.content(div().width(160.0).children(kids));
                        deferred(below(), nav.viewport(div().child(panel)))
                            .tab_after(item.trigger_key())
                            .into_element()
                    }
                })
                .into_element();
                div()
                    .padding(20.0)
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .gap(200.0)
                    .child(
                        nav.root(div())
                            .child(nav.list(div().display(Display::Flex).gap(10.0)).children(items))
                            .child(viewport),
                    )
                    .child(div().focusable().width(60.0).height(24.0).child(text("After")))
                    .into_element()
            }
        })
        .into_element()
    });
    let nav = menu.get().expect("the menu");
    let triggers = [0, 1].map(|i| triggers.get_untracked()[i]);
    (s, value, triggers, links, nav)
}

#[test]
fn a_viewport_shows_the_open_panel_and_its_tab_stops_follow_the_trigger() {
    let (mut s, value, t, links, nav) = navigation_with_viewport();
    s.act("focus One", || t[0].focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked().as_deref(), Some("one"));
    assert!(shows(&s, "Link 0"), "the viewport shows One's panel");
    assert!(
        gui::untracked(|| nav.viewport_size()).is_some_and(|(w, h)| w > 0.0 && h > 0.0),
        "its size is known"
    );
    plain(&mut s, NamedKey::Tab);
    assert!(
        focused(links[0]),
        "Tab from the open trigger enters the panel, drawn in the viewport after the list"
    );
    plain(&mut s, NamedKey::Tab);
    assert!(focused(links[1]));
    plain(&mut s, NamedKey::Tab);
    assert!(focused(t[1]), "past the panel's end, the next item's trigger");
    let back = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    key(&mut s, NamedKey::Tab, back);
    assert!(focused(links[1]), "Shift+Tab from the next item, the panel's last link");
    key(&mut s, NamedKey::Tab, back);
    key(&mut s, NamedKey::Tab, back);
    assert!(focused(t[0]), "and before its first, its trigger");

    // Switching moves the viewport's content and its place in Tab order.
    plain(&mut s, NamedKey::ArrowRight);
    s.act("open Two", || value.set(Some("two".to_owned())));
    s.settle().expect("settled");
    assert!(shows(&s, "Link 2") && !shows(&s, "Link 0"));
    plain(&mut s, NamedKey::Tab);
    assert!(focused(links[2]), "Tab from Two enters Two's panel");

    // The pointer in the viewport holds it open; leaving closes it.
    let (lx, ly) = at(&s, "Link 3");
    hover(&mut s, lx, ly);
    wait(&mut s, 400);
    assert_eq!(value.get_untracked().as_deref(), Some("two"));
    hover(&mut s, 700.0, 590.0);
    wait(&mut s, ms(CLOSE_DELAY) + 10);
    assert!(
        value.get_untracked().is_none(),
        "leaving the viewport closes it after 150 ms"
    );
}

struct Nested {
    value: Signal<Option<String>>,
    sub_value: Signal<Option<String>>,
    /// The root item's trigger.
    one: Signal<Option<FocusHandle>>,
    sub_triggers: Signal<Vec<FocusHandle>>,
    /// The link in each nested panel.
    sub_links: [FocusHandle; 2],
    followed: Log,
}

/// Item "One", open, whose panel holds a vertical nested menu of "A" and
/// "B", each a panel with a link "A link" / "B link".
fn nested() -> (Scenario, Nested) {
    let value = Signal::new(Some("one".to_owned()));
    let sub_value: Signal<Option<String>> = Signal::new(None);
    let one_trigger = Signal::new(None);
    let sub_triggers = Signal::new(Vec::new());
    let sub_links = [(); 2].map(|()| FocusHandle::new());
    let followed: Log = Rc::default();
    let log = Rc::clone(&followed);
    let s = scenario(move || {
        let log = Rc::clone(&log);
        component((), move |_| {
            let nav = NavigationMenu::new(value, NavOptions::DEFAULT);
            let log = Rc::clone(&log);
            move || {
                let log = Rc::clone(&log);
                let one = component((), move |_| {
                    let item = NavigationMenuItem::new(nav, "one");
                    hand_out(one_trigger, Some(item.trigger_focus()));
                    let log = Rc::clone(&log);
                    move || {
                        let log = Rc::clone(&log);
                        let panel = item.is_open().then(|| {
                            let sub = component((), move |_| {
                                let sub = nav.sub(sub_value, Axis::Vertical);
                                let log = Rc::clone(&log);
                                move || {
                                    let kids = [("A", sub_links[0]), ("B", sub_links[1])].map(|(name, link)| {
                                        let log = Rc::clone(&log);
                                        component((), move |_| {
                                            let si = NavigationMenuItem::new(sub, name.to_lowercase());
                                            hand_out_push(sub_triggers, si.trigger_focus());
                                            let log = Rc::clone(&log);
                                            move || {
                                                let log = Rc::clone(&log);
                                                let panel = si.is_open().then(|| {
                                                    let label = format!("{name} link");
                                                    let shown = label.clone();
                                                    let link = si.link(row(&label), link, false, move |()| {
                                                        log.borrow_mut().push(shown.clone());
                                                        Dismiss::Close
                                                    });
                                                    deferred(
                                                        Positioning::Anchored {
                                                            side: Side::Right,
                                                            align: SideAlign::Start,
                                                            offset: 0.0,
                                                            match_width: false,
                                                            fit_height: false,
                                                        },
                                                        si.content(div().child(link)),
                                                    )
                                                    .into_element()
                                                });
                                                si.root(div())
                                                    .child(si.trigger(
                                                        div().width(60.0).height(24.0).child(text(name)),
                                                        Availability::Enabled,
                                                    ))
                                                    .child(panel)
                                                    .into_element()
                                            }
                                        })
                                        .into_element()
                                    });
                                    sub.root(div()).child(sub.list(div()).children(kids)).into_element()
                                }
                            })
                            .into_element();
                            deferred(
                                below(),
                                item.content(div().width(200.0).height(120.0).child(sub).child(text("Pad"))),
                            )
                            .into_element()
                        });
                        item.root(div())
                            .child(
                                item.trigger(div().width(60.0).height(24.0).child(text("One")), Availability::Enabled),
                            )
                            .child(panel)
                            .into_element()
                    }
                })
                .into_element();
                div()
                    .padding(20.0)
                    .child(nav.root(div()).child(nav.list(div()).child(one)))
                    .into_element()
            }
        })
        .into_element()
    });
    (
        s,
        Nested {
            value,
            sub_value,
            one: one_trigger,
            sub_triggers,
            sub_links,
            followed,
        },
    )
}

#[test]
fn a_nested_navigation_menu_switches_at_once_and_stays() {
    let (mut s, n) = nested();
    let Nested {
        value,
        sub_value,
        sub_triggers,
        ..
    } = n;
    let (ax, ay) = at(&s, "A");
    hover(&mut s, ax, ay);
    assert_eq!(
        sub_value.get_untracked().as_deref(),
        Some("a"),
        "a nested trigger opens at once on hover"
    );
    let (bx, by) = at(&s, "B");
    hover(&mut s, bx, by);
    assert_eq!(sub_value.get_untracked().as_deref(), Some("b"), "and switches at once");
    let (px, py) = at(&s, "Pad");
    hover(&mut s, px, py);
    wait(&mut s, 400);
    assert_eq!(
        sub_value.get_untracked().as_deref(),
        Some("b"),
        "leaving it closes nothing"
    );
    s.click(px, py);
    s.settle().expect("settled");
    assert_eq!(
        sub_value.get_untracked().as_deref(),
        Some("b"),
        "a press outside its panel does not close it"
    );
    assert_eq!(value.get_untracked().as_deref(), Some("one"));
    let b_trigger = sub_triggers.get_untracked()[1];
    s.act("focus B", || b_trigger.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Escape);
    assert!(sub_value.get_untracked().is_none(), "Escape closes the nested panel");
    assert_eq!(value.get_untracked().as_deref(), Some("one"), "and only it");
    assert!(focused(b_trigger), "focus on its trigger");
}

#[test]
fn a_link_picked_in_a_nested_panel_closes_the_whole_menu() {
    // By the keyboard: focus goes back to the root item's trigger.
    let (mut s, n) = nested();
    let a = n.sub_triggers.get_untracked()[0];
    s.act("focus A", || a.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(n.sub_value.get_untracked().as_deref(), Some("a"));
    plain(&mut s, NamedKey::ArrowRight);
    assert!(
        focused(n.sub_links[0]),
        "ArrowRight enters a vertical nested menu's panel"
    );
    plain(&mut s, NamedKey::Enter);
    assert_eq!(*n.followed.borrow(), ["A link"]);
    assert!(
        n.value.get_untracked().is_none(),
        "a link picked at any depth closes the root menu (reka's dismiss goes up to the root content)"
    );
    assert!(n.sub_value.get_untracked().is_none(), "and the nested one with it");
    let one = n.one.get_untracked().expect("One's trigger");
    assert!(focused(one), "focus goes back to the root item's trigger");

    // By the mouse.
    let (mut s, n) = nested();
    let (ax, ay) = at(&s, "A");
    hover(&mut s, ax, ay);
    click(&mut s, "A link");
    assert_eq!(*n.followed.borrow(), ["A link"]);
    assert!(
        n.value.get_untracked().is_none(),
        "a click on a nested link closes the whole menu"
    );
    assert!(n.sub_value.get_untracked().is_none());
}

/// The viewport part replaced in one update (keyed anew): the new one
/// joins before the old one leaves, and the old one's leaving does not
/// take the viewport away from the new one.
#[test]
fn a_navigation_viewport_replaced_in_one_update_still_shows_the_panels() {
    let value = Signal::new(None);
    let which = Signal::new(false);
    let menu: Rc<std::cell::Cell<Option<NavigationMenu>>> = Rc::default();
    let out = Rc::clone(&menu);
    let mut s = scenario(move || {
        let out = Rc::clone(&out);
        component((), move |_| {
            let nav = NavigationMenu::new(value, NavOptions::DEFAULT);
            out.set(Some(nav));
            move || {
                let viewport = component((), move |_| {
                    nav.use_viewport();
                    || Element::Empty
                });
                nav.root(div())
                    .child(nav.list(div()))
                    .child(
                        div()
                            .key(gui::key_of(if which.get() { "second" } else { "first" }))
                            .child(viewport),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    let nav = menu.get().expect("the menu");
    assert!(gui::untracked(|| nav.has_viewport()));
    s.act("replace the viewport", || which.set(true));
    s.settle().expect("settled");
    assert!(
        gui::untracked(|| nav.has_viewport()),
        "the new viewport shows the panels"
    );
}

/// The filter part replaced in one update (keyed anew), both editing the
/// same field (the same focus handle): the new part holds the level's
/// filter from its mount, and the old one's leaving lets go only of its
/// own registration, not of the handle, so the keys in the field keep
/// focus there and move the level's highlight.
#[test]
fn a_filter_registered_again_with_the_same_field_stays_the_levels_filter() {
    use primitives::dropdown_menu::DropdownMenuFilter;
    let open = Signal::new(true);
    let which = Signal::new(false);
    let foci = [(); 5].map(|()| FocusHandle::new());
    let picked: Log = Rc::default();
    let field = gui::TextField::new("");
    let level = Signal::new(None);
    let mut s = scenario(move || {
        let picked = Rc::clone(&picked);
        component((), move |_| {
            let dd = DropdownMenu::new(open, MenuOptions::DEFAULT);
            hand_out(level, Some(dd.menu()));
            let picked = Rc::clone(&picked);
            move || {
                let menu = dd.menu();
                let search = div()
                    .key(gui::key_of(if which.get() { "second" } else { "first" }))
                    .child(component((), move |_| {
                        let filter = DropdownMenuFilter::new(menu, field, true, false);
                        move || filter.field(gui::text_input(&field).width(140.0)).into_element()
                    }))
                    .into_element();
                let mut kids = vec![search];
                kids.extend(items(menu, &foci, &picked, false));
                div()
                    .child(dd.trigger(
                        div().width(80.0).height(30.0).child(text("Open")),
                        Availability::Enabled,
                    ))
                    .child(content(menu, below(), kids))
                    .into_element()
            }
        })
        .into_element()
    });
    let menu = level.get_untracked().expect("the menu");
    s.act("register the filter again", || which.set(true));
    s.settle().expect("settled");
    s.act("focus the field", || field.focus.focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(field.focus), "focus stays in the level's filter");
    assert!(
        gui::untracked(|| menu.is_highlighted(foci[0])),
        "the first item is highlighted"
    );
}

/// `part`, set up by the cold frame alone: a cold re-render's setup that
/// never mounts. It draws nothing, so the frames compare equal.
fn cold_only(part: impl Fn() + Copy + 'static) -> Option<Element> {
    gui::component::is_cold_render().then(|| {
        component((), move |_| {
            part();
            || Element::Empty
        })
        .into_element()
    })
}

fn checked_frames(s: &mut Scenario) {
    for name in ["one", "two", "three"] {
        s.frame(name).expect("a checked frame");
    }
}

/// A submenu only a cold frame set up never mounted: it is none of the
/// level's submenus, which closing the level walks.
#[test]
fn a_submenu_a_cold_frame_alone_set_up_is_not_the_levels() {
    let open = Signal::new(true);
    let level = Signal::new(None);
    let mut s = scenario(move || {
        component((), move |_| {
            let menu = Menu::new(open, MenuOptions::DEFAULT);
            hand_out(level, Some(menu));
            move || {
                let ghost = cold_only(move || {
                    let _sub = menu.submenu(Signal::new(false));
                });
                div().children(ghost).into_element()
            }
        })
        .into_element()
    });
    checked_frames(&mut s);
    let menu = level.get_untracked().expect("the menu");
    s.act("close the menu", || menu.close());
    s.settle().expect("settled");
    assert!(!open.get_untracked(), "closed, its submenus closed with it");
}

/// A navigation item only a cold frame set up never mounted: the menu has
/// no item of its value.
#[test]
fn a_navigation_item_a_cold_frame_alone_set_up_is_not_the_menus() {
    let value = Signal::new(None);
    let menu = Signal::new(None);
    let mut s = scenario(move || {
        component((), move |_| {
            let nav = NavigationMenu::new(value, NavOptions::default());
            hand_out(menu, Some(nav));
            move || {
                let ghost = cold_only(move || {
                    let _item = NavigationMenuItem::new(nav, "ghost");
                });
                div().children(ghost).into_element()
            }
        })
        .into_element()
    });
    checked_frames(&mut s);
    let nav = menu.get_untracked().expect("the menu");
    assert!(
        gui::untracked(|| nav.item("ghost")).is_none(),
        "no item of a part never mounted"
    );
}

/// A menubar menu only a cold frame set up never mounted: it is none of
/// the bar's menus.
#[test]
fn a_menubar_menu_a_cold_frame_alone_set_up_is_not_the_bars() {
    let value = Signal::new(String::new());
    let held = Signal::new(None);
    let mut s = scenario(move || {
        component((), move |_| {
            let bar = Menubar::new(value, Wrap::Stop);
            hand_out(held, Some(bar));
            move || {
                let ghost = cold_only(move || {
                    let _menu = MenubarMenu::new(bar, "ghost", MenuOptions::DEFAULT);
                });
                div().children(ghost).into_element()
            }
        })
        .into_element()
    });
    checked_frames(&mut s);
    let bar = held.get_untracked().expect("the bar");
    assert!(
        gui::untracked(|| bar.menu("ghost")).is_none(),
        "no menu of a part never mounted"
    );
}

/// A filtered menu's rows come and go a thousand times, each time with
/// focus handles never seen before: the level keeps nothing of a row beyond
/// those laid out (no table of nodes or boxes by focus that only grows).
#[test]
fn rows_of_a_filtered_menu_that_come_and_go_leave_nothing_behind() {
    use primitives::dropdown_menu::DropdownMenuFilter;
    let open = Signal::new(true);
    let field = gui::TextField::new("");
    let level = Signal::new(None);
    let round = Signal::new(0_u32);
    let mut s = scenario(move || {
        component((), move |_| {
            let dd = DropdownMenu::new(open, MenuOptions::DEFAULT);
            hand_out(level, Some(dd.menu()));
            move || {
                let menu = dd.menu();
                let search = component((), move |_| {
                    let filter = DropdownMenuFilter::new(menu, field, true, false);
                    move || filter.field(gui::text_input(&field).width(140.0)).into_element()
                })
                .into_element();
                let shown = if round.get().is_multiple_of(2) { 3 } else { 1 };
                let mut kids = vec![search];
                kids.extend((0..shown).map(|i| {
                    let entry = Item::new((), FocusHandle::new()).label(format!("Row {i}"));
                    menu.item(row("Row"), entry, |()| Dismiss::Close).into_element()
                }));
                div()
                    .child(dd.trigger(
                        div().width(80.0).height(30.0).child(text("Open")),
                        Availability::Enabled,
                    ))
                    .child(content(menu, below(), kids))
                    .into_element()
            }
        })
        .into_element()
    });
    let menu = level.get_untracked().expect("the menu");
    let mut most = 0;
    for n in 1..=1000 {
        s.act("rows come and go", || round.set(n));
        plain(&mut s, NamedKey::ArrowDown);
        most = most.max(menu.kept_rows());
    }
    assert!(most <= 3, "kept {most} rows, 3 at most shown");
}

/// A submenu's open model whose closing ends its part (it leaves as it
/// closes), and counts nothing else.
struct LeavesAsItCloses {
    open: Signal<bool>,
    part: std::cell::Cell<Option<gui::Scope>>,
}

impl primitives::menu::OpenModel for LeavesAsItCloses {
    fn get(&self) -> bool {
        self.open.get()
    }

    fn set(&self, open: bool) {
        if !open && let Some(part) = self.part.take() {
            part.dispose();
        }
        if self.open.get_untracked() != open {
            self.open.set(open);
        }
    }
}

/// Opening a menu again closes its 500 submenus, walking each once, a few
/// links apiece; the first one leaving as it closes does not make the walk
/// skip the second.
#[test]
fn a_menus_walk_over_its_submenus_reaches_each_once_though_one_leaves_as_it_closes() {
    let open = Signal::new(true);
    let level = Signal::new(None);
    let _s = scenario(move || {
        component((), move |_| {
            let dd = DropdownMenu::new(open, MenuOptions::DEFAULT);
            hand_out(level, Some(dd.menu()));
            || div().into_element()
        })
        .into_element()
    });
    let menu: Menu = level.get_untracked().expect("the menu");
    let mut opens = Vec::new();
    let mut parts = Vec::new();
    for i in 0..500 {
        let open = Signal::new(true);
        let part = gui::Scope::new();
        let model = LeavesAsItCloses {
            open,
            part: std::cell::Cell::new(None),
        };
        if i == 0 {
            model.part.set(Some(part));
        } else {
            parts.push(part);
        }
        part.run(|| {
            let _sub = menu.submenu(model);
        });
        opens.push(open);
    }
    let before = menu.sub_visits();
    menu.open(primitives::menu::OpenFocus::Content);
    let walked = menu.sub_visits() - before;
    let still: Vec<usize> = (0..500).filter(|&i| opens[i].get_untracked()).collect();
    assert!(still.is_empty(), "submenus left open: {still:?}");
    assert!(
        walked <= 4 * 500,
        "closing stepped through {walked} links for 500 submenus"
    );
    for p in parts {
        p.dispose();
    }
}
