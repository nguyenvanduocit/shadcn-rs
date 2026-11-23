//! Every component with an initially open, selected or active option,
//! mounted in that state: a setup writes nothing it did not make (a debug
//! build's purity check refuses a render that does), and each frame is the
//! one built cold. Each test asserts the state shows.

#![expect(
    clippy::expect_used,
    reason = "a scenario that cannot run, or a node not found, is a failed test"
)]

use std::sync::Arc;

use gui::prelude::*;
use gui::{Element, Scenario, Signal, div, text};
use shadcn::{
    ButtonSize, ButtonVariant, CalendarDate, CommandItem, CommandSection, DateRange, DrawerDirection, MenuItem,
    Orientation, SheetSide, SnapPoint, ToggleSize, ToggleVariant, accordion, accordion_content, accordion_item,
    accordion_multiple, accordion_trigger, alert_dialog, alert_dialog_action, alert_dialog_cancel,
    alert_dialog_content, alert_dialog_description, alert_dialog_footer, alert_dialog_header, alert_dialog_title,
    button, calendar, checkbox, collapsible, collapsible_content, collapsible_trigger_as, combobox, combobox_input,
    combobox_items, combobox_list, combobox_trigger, dialog, dialog_content, dialog_description, dialog_title,
    dialog_trigger, drawer, drawer_content, drawer_description, drawer_title, drawer_trigger, dropdown_menu,
    dropdown_menu_checkbox_item, dropdown_menu_content, dropdown_menu_item, dropdown_menu_radio_group,
    dropdown_menu_radio_item, dropdown_menu_trigger, hover_card, hover_card_content, hover_card_trigger, input_otp,
    input_otp_group, input_otp_slot, menubar, menubar_content, menubar_item, menubar_menu, menubar_trigger,
    navigation_menu, navigation_menu_content, navigation_menu_item, navigation_menu_list, navigation_menu_trigger,
    number_field, number_field_content, number_field_decrement, number_field_increment, number_field_input, pagination,
    pagination_content, pagination_list, pagination_next, pagination_previous, pin_input, pin_input_slot, popover,
    popover_content, popover_trigger, radio_group, radio_group_item, range_calendar, select, select_content,
    select_item, select_value, sheet, sheet_content, sheet_description, sheet_title, sidebar, sidebar_content,
    sidebar_inset, sidebar_provider, sidebar_trigger, slider, stepper, stepper_item, stepper_title, switch, tabs,
    tabs_content, tabs_list, tabs_trigger, tags_input, tags_input_input, tags_input_item, tags_input_item_delete,
    tags_input_item_text, toggle, toggle_group, toggle_group_item, toggle_group_multiple, tooltip, tooltip_content,
    tooltip_trigger,
};

/// `f` mounted under the cold check, settled, two frames drawn.
fn mounted(f: impl Fn() -> Element + 'static) -> Scenario {
    let mut s = Scenario::measuring(f, 800, 600, 1.0).expect("a scenario");
    s.check_cold(true);
    s.settle().expect("settled");
    s.frame("first").expect("a frame");
    s.frame("second").expect("a frame");
    s
}

fn shows(s: &Scenario, what: &str) -> bool {
    s.locate(what).is_ok()
}

/// What a screen reader is told of the last frame, a node at a time.
fn told(s: &Scenario) -> Vec<accesskit::Node> {
    let mut ts = gui::text::TextSystem::new();
    gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts)
        .nodes
        .into_iter()
        .map(|(_, n)| n)
        .collect()
}

fn named(n: &accesskit::Node) -> String {
    n.label().or_else(|| n.value()).unwrap_or_default().to_owned()
}

/// The node of `id` among `nodes`.
fn node(nodes: &[(accesskit::NodeId, accesskit::Node)], id: accesskit::NodeId) -> Option<&accesskit::Node> {
    nodes.iter().find(|(i, _)| *i == id).map(|(_, n)| n)
}

/// What a screen reader is told of the last frame, with each node's id.
fn told_by_id(s: &Scenario) -> Vec<(accesskit::NodeId, accesskit::Node)> {
    let mut ts = gui::text::TextSystem::new();
    gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts).nodes
}

/// The names of the children of the nodes `pick` chooses, in tree order.
fn names_under(s: &Scenario, pick: impl Fn(&accesskit::Node) -> bool) -> Vec<Vec<String>> {
    let nodes = told_by_id(s);
    nodes
        .iter()
        .filter(|(_, n)| pick(n))
        .map(|(_, n)| {
            n.children()
                .iter()
                .filter_map(|c| node(&nodes, *c))
                .map(named)
                .collect()
        })
        .collect()
}

/// The sliders' values, in tree order.
fn slider_values(s: &Scenario) -> Vec<f64> {
    told(s)
        .iter()
        .filter(|n| n.role() == accesskit::Role::Slider)
        .filter_map(accesskit::Node::numeric_value)
        .collect()
}

/// The names of the buttons that are the current page.
fn current_pages(s: &Scenario) -> Vec<String> {
    told(s)
        .iter()
        .filter(|n| n.aria_current() == Some(accesskit::AriaCurrent::Page))
        .map(named)
        .collect()
}

/// What each text input of the pin shows.
fn pin_cells(s: &Scenario) -> Vec<String> {
    names_under(s, |n| n.role() == accesskit::Role::TextInput)
        .into_iter()
        .map(|c| c.concat())
        .collect()
}

/// The names of the days selected (the buttons in the selected cells).
fn selected_days(s: &Scenario) -> Vec<String> {
    names_under(s, |n| {
        n.role() == accesskit::Role::GridCell && n.is_selected() == Some(true)
    })
    .into_iter()
    .map(|c| c.concat())
    .collect()
}

/// `today` as a day's button names it: "…, November 23, 2026".
fn day_name_ends(today: CalendarDate) -> String {
    format!(" {}, {}", today.day(), today.year())
}

fn outline(label: &str) -> gui::Div {
    button(ButtonVariant::Outline, ButtonSize::Default)
        .child(label.to_owned())
        .into()
}

#[test]
fn alert_dialog_open_from_the_start() {
    let s = mounted(|| {
        alert_dialog()
            .default_open(true)
            .child(dialog_trigger(outline("Show Dialog")))
            .child(
                alert_dialog_content()
                    .child(
                        alert_dialog_header()
                            .child(alert_dialog_title("Are you sure?"))
                            .child(alert_dialog_description("It cannot be undone.")),
                    )
                    .child(
                        alert_dialog_footer()
                            .child(alert_dialog_cancel().child("Cancel"))
                            .child(alert_dialog_action(|()| {}).child("Continue")),
                    ),
            )
            .into_element()
    });
    assert!(shows(&s, "Are you sure?"));
}

#[test]
fn dialog_open_from_the_start() {
    let s = mounted(|| {
        dialog()
            .default_open(true)
            .child(dialog_trigger(outline("Open Dialog")))
            .child(
                dialog_content()
                    .child(dialog_title("Edit profile"))
                    .child(dialog_description("Make changes.")),
            )
            .into_element()
    });
    assert!(shows(&s, "Edit profile"));
}

#[test]
fn sheet_open_from_the_start() {
    let s = mounted(|| {
        sheet()
            .default_open(true)
            .child(dialog_trigger(outline("Open")))
            .child(
                sheet_content(SheetSide::Right)
                    .child(sheet_title("Edit profile"))
                    .child(sheet_description("Make changes.")),
            )
            .into_element()
    });
    assert!(shows(&s, "Edit profile"));
}

#[test]
fn collapsible_open_from_the_start() {
    let s = mounted(|| {
        collapsible()
            .default_open(true)
            .child(collapsible_trigger_as(outline("Toggle")))
            .child(collapsible_content().child(text("Hidden part")))
            .into_element()
    });
    assert!(shows(&s, "Hidden part"));
}

#[test]
fn dropdown_menu_open_from_the_start_with_checked_and_chosen_items() {
    let s = mounted(|| {
        dropdown_menu()
            .default_open(true)
            .child(dropdown_menu_trigger(outline("Open")))
            .child(
                dropdown_menu_content()
                    .child(dropdown_menu_item("Profile", MenuItem::default()).child("Profile"))
                    .child(dropdown_menu_checkbox_item("Status Bar", false).default_checked(true))
                    .child(
                        dropdown_menu_radio_group()
                            .default_value("bottom")
                            .child(dropdown_menu_radio_item("Top", "top", false))
                            .child(dropdown_menu_radio_item("Bottom", "bottom", false)),
                    ),
            )
            .into_element()
    });
    assert!(shows(&s, "Profile"));
    assert!(shows(&s, "Status Bar"));
}

#[test]
fn drawer_open_from_the_start() {
    let s = mounted(|| {
        drawer()
            .default_open(true)
            .child(drawer_trigger(outline("Open Drawer")))
            .child(
                drawer_content(DrawerDirection::Down)
                    .child(drawer_title("Move Goal"))
                    .child(drawer_description("Set your daily goal.")),
            )
            .into_element()
    });
    assert!(shows(&s, "Move Goal"));
}

#[test]
fn drawer_open_from_the_start_at_a_snap_point() {
    let s = mounted(|| {
        drawer()
            .default_open(true)
            .snap_points([SnapPoint::Number(0.5), SnapPoint::Number(1.0)])
            .default_snap_point(Some(SnapPoint::Number(0.5)))
            .child(drawer_trigger(outline("Open Drawer")))
            .child(drawer_content(DrawerDirection::Down).child(drawer_title("Snapped")))
            .into_element()
    });
    assert!(shows(&s, "Snapped"));
}

#[test]
fn popover_open_from_the_start() {
    let s = mounted(|| {
        popover()
            .default_open(true)
            .child(popover_trigger(outline("Open popover")))
            .child(popover_content().child(text("Dimensions")))
            .into_element()
    });
    assert!(shows(&s, "Dimensions"));
}

#[test]
fn hover_card_open_from_the_start() {
    let s = mounted(|| {
        hover_card()
            .default_open(true)
            .child(hover_card_trigger(
                button(ButtonVariant::Link, ButtonSize::Default).child("@nuxt"),
            ))
            .child(hover_card_content().child(text("The Vue Framework")))
            .into_element()
    });
    assert!(shows(&s, "The Vue Framework"));
}

#[test]
fn tooltip_open_from_the_start() {
    let s = mounted(|| {
        tooltip()
            .default_open(true)
            .child(tooltip_trigger(outline("Hover")))
            .child(tooltip_content().child(text("Add to library")))
            .into_element()
    });
    assert!(shows(&s, "Add to library"));
}

#[test]
fn select_open_from_the_start_with_a_value() {
    let s = mounted(|| {
        select()
            .default_value(Some("banana".to_owned()))
            .default_open(true)
            .width(180.0)
            .child(select_value("Select a fruit"))
            .child(
                select_content()
                    .child(select_item("apple", "Apple"))
                    .child(select_item("banana", "Banana")),
            )
            .into_element()
    });
    assert!(shows(&s, "Apple"), "the list is open");
}

#[test]
fn sidebar_open_from_the_start() {
    let s = mounted(|| {
        sidebar_provider()
            .default_open(true)
            .child(sidebar().child(sidebar_content().child(text("Projects"))))
            .child(sidebar_inset().child(sidebar_trigger()))
            .into_element()
    });
    assert!(shows(&s, "Projects"));
}

#[test]
fn menubar_with_a_menu_open_from_the_start() {
    let s = mounted(|| {
        menubar()
            .default_value("file")
            .child(
                menubar_menu()
                    .value("file")
                    .child(menubar_trigger("File"))
                    .child(menubar_content().child(menubar_item("New Tab", MenuItem::default()).child("New Tab"))),
            )
            .child(
                menubar_menu()
                    .value("edit")
                    .child(menubar_trigger("Edit"))
                    .child(menubar_content().child(menubar_item("Undo", MenuItem::default()).child("Undo"))),
            )
            .into_element()
    });
    assert!(shows(&s, "New Tab"));
}

#[test]
fn navigation_menu_with_an_item_open_from_the_start() {
    let s = mounted(|| {
        navigation_menu()
            .default_value("one")
            .child(
                navigation_menu_list().child(
                    navigation_menu_item("one")
                        .child(navigation_menu_trigger("One"))
                        .child(navigation_menu_content().child(text("Panel one"))),
                ),
            )
            .into_element()
    });
    assert!(shows(&s, "Panel one"));
}

#[test]
fn navigation_menu_without_a_viewport_open_from_the_start() {
    let s = mounted(|| {
        navigation_menu()
            .viewport(false)
            .default_value("one")
            .child(
                navigation_menu_list().child(
                    navigation_menu_item("one")
                        .child(navigation_menu_trigger("One"))
                        .child(navigation_menu_content().child(text("Panel one"))),
                ),
            )
            .into_element()
    });
    assert!(shows(&s, "Panel one"));
}

#[test]
fn accordion_with_a_section_open_from_the_start() {
    let s = mounted(|| {
        accordion()
            .collapsible(true)
            .default_value("item-1")
            .child(
                accordion_item("item-1")
                    .child(accordion_trigger("First"))
                    .child(accordion_content().child(text("First body"))),
            )
            .child(
                accordion_item("item-2")
                    .child(accordion_trigger("Second"))
                    .child(accordion_content().child(text("Second body"))),
            )
            .into_element()
    });
    assert!(shows(&s, "First body"));
}

#[test]
fn accordion_of_several_with_sections_open_from_the_start() {
    let s = mounted(|| {
        accordion_multiple()
            .default_value(&["item-1", "item-2"])
            .child(
                accordion_item("item-1")
                    .child(accordion_trigger("First"))
                    .child(accordion_content().child(text("First body"))),
            )
            .child(
                accordion_item("item-2")
                    .child(accordion_trigger("Second"))
                    .child(accordion_content().child(text("Second body"))),
            )
            .into_element()
    });
    assert!(shows(&s, "First body") && shows(&s, "Second body"));
}

#[test]
fn tabs_with_a_tab_active_from_the_start() {
    let s = mounted(|| {
        tabs()
            .default_value("password")
            .child(
                tabs_list()
                    .child(tabs_trigger("account").child("Account"))
                    .child(tabs_trigger("password").child("Password")),
            )
            .child(tabs_content("account").child(text("Account body")))
            .child(tabs_content("password").child(text("Password body")))
            .into_element()
    });
    assert!(shows(&s, "Password body"));
}

#[test]
fn combobox_open_from_the_start_with_a_value() {
    let list = Arc::new(vec![
        CommandSection::empty("No framework found."),
        CommandSection::group(None, ["Next.js", "Nuxt.js"].map(CommandItem::new)),
    ]);
    let open = Signal::new(true);
    let s = mounted(move || {
        combobox()
            .default_value(Some("Nuxt.js".to_owned()))
            .open(open)
            .child(combobox_trigger(outline("Select framework...")))
            .child(
                combobox_list()
                    .child(combobox_input("Search framework..."))
                    .child(combobox_items(Arc::clone(&list), |item, row| {
                        row.child(item.value.clone())
                    })),
            )
            .into_element()
    });
    assert!(shows(&s, "Next.js"), "the list is open");
}

#[test]
fn toggle_group_with_an_item_on_from_the_start() {
    let s = mounted(|| {
        div()
            .child(
                toggle_group()
                    .default_value(Some("bold".to_owned()))
                    .child(toggle_group_item("bold").child("B"))
                    .child(toggle_group_item("italic").child("I")),
            )
            .child(
                toggle_group_multiple()
                    .default_value(vec!["bold".to_owned()])
                    .child(toggle_group_item("bold").child("B2"))
                    .child(toggle_group_item("italic").child("I2")),
            )
            .into_element()
    });
    assert!(shows(&s, "B") && shows(&s, "B2"));
}

#[test]
fn radio_group_with_an_option_chosen_from_the_start() {
    let s = mounted(|| {
        radio_group()
            .default_value("on")
            .child(div().child(radio_group_item("on")).child(text("On")))
            .child(div().child(radio_group_item("off")).child(text("Off")))
            .into_element()
    });
    assert!(shows(&s, "On"));
}

#[test]
fn stepper_at_a_step_from_the_start() {
    let s = mounted(|| {
        stepper()
            .default_value(2)
            .children(
                [(1, "Address"), (2, "Shipping"), (3, "Payment")]
                    .map(|(step, title)| stepper_item(step).fill(move |item, d| d.child(stepper_title(item, title)))),
            )
            .into_element()
    });
    assert!(shows(&s, "Shipping"));
}

#[test]
fn pagination_at_a_page_from_the_start() {
    let s = mounted(|| {
        pagination()
            .total(200, 10)
            .default_page(10)
            .child(
                pagination_content()
                    .child(pagination_previous())
                    .child(pagination_list())
                    .child(pagination_next()),
            )
            .into_element()
    });
    assert_eq!(current_pages(&s), ["Page 10"], "page 10 is the current page");
}

#[test]
fn calendar_with_a_day_chosen_from_the_start() {
    let today = CalendarDate::today();
    let s = mounted(move || calendar().today(today).default_value(Some(today)).into_element());
    let days = selected_days(&s);
    assert_eq!(days.len(), 1, "one day is selected: {days:?}");
    assert!(
        days[0].ends_with(&day_name_ends(today)),
        "today is the day selected: {days:?}"
    );
}

#[test]
fn range_calendar_with_a_range_chosen_from_the_start() {
    let today = CalendarDate::today();
    let s = mounted(move || {
        range_calendar()
            .today(today)
            .default_value(DateRange {
                start: Some(today),
                end: Some(today),
            })
            .into_element()
    });
    let days = selected_days(&s);
    assert_eq!(days.len(), 1, "the range is the one day: {days:?}");
    assert!(days[0].ends_with(&day_name_ends(today)), "today is the range: {days:?}");
}

#[test]
fn tags_input_with_tags_from_the_start() {
    let s = mounted(|| {
        tags_input()
            .default_value(["Apple", "Banana"])
            .width(300.0)
            .children(["Apple", "Banana"].map(|tag| {
                tags_input_item(tag)
                    .key(gui::key_of(tag))
                    .child(tags_input_item_text(tag))
                    .child(tags_input_item_delete(tag))
            }))
            .child(tags_input_input().placeholder("Fruits..."))
            .into_element()
    });
    assert!(shows(&s, "Apple"));
}

#[test]
fn pin_input_filled_from_the_start() {
    let s = mounted(|| {
        pin_input()
            .default_value(vec![Some("1".to_owned()), Some("2".to_owned()), None])
            .child(input_otp_group((0..3).map(pin_input_slot)))
            .into_element()
    });
    assert_eq!(pin_cells(&s), ["1", "2", ""], "the cells show the value given");
}

#[test]
fn input_otp_filled_from_the_start() {
    let s = mounted(|| {
        input_otp()
            .default_value("12")
            .child(input_otp_group((0..4).map(input_otp_slot)))
            .into_element()
    });
    assert!(shows(&s, "1"));
}

#[test]
fn slider_at_a_value_from_the_start() {
    let s = mounted(|| {
        div()
            .width(300.0)
            .child(slider(Orientation::Horizontal).default_value(vec![25.0, 50.0]))
            .into_element()
    });
    assert_eq!(slider_values(&s), [25.0, 50.0], "the thumbs are at the values given");
}

#[test]
fn number_field_at_a_value_from_the_start() {
    let s = mounted(|| {
        number_field()
            .default_value(18.0)
            .child(
                number_field_content()
                    .child(number_field_decrement())
                    .child(number_field_input())
                    .child(number_field_increment()),
            )
            .into_element()
    });
    assert!(shows(&s, "18"));
}

#[test]
fn switch_checkbox_and_toggle_on_from_the_start() {
    let s = mounted(|| {
        div()
            .child(switch().default_checked(true))
            .child(checkbox().default_checked(true))
            .child(
                toggle(ToggleVariant::Default, ToggleSize::Default)
                    .default_pressed(true)
                    .child("Bold"),
            )
            .into_element()
    });
    assert!(shows(&s, "Bold"));
}

/// A toaster only a cold frame set up never mounted is none of the
/// toasters: a toast goes to the one mounted last that is.
#[test]
fn a_toaster_a_cold_frame_alone_set_up_takes_no_toast() {
    let mut s = mounted(|| {
        let ghost = gui::component::is_cold_render().then(shadcn::toaster);
        div()
            .width(800.0)
            .height(600.0)
            .child(text("Page"))
            .child(shadcn::toaster())
            .children(ghost)
            .into_element()
    });
    s.frame("third").expect("a frame");
    s.act("toast", || {
        shadcn::toast("Saved");
    });
    s.settle().expect("settled");
    assert!(shows(&s, "Saved"), "the mounted toaster shows it");
}
