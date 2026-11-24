//! A part that cannot work without its root, made outside it, reports the
//! error: it names the part, the root it needs and the place in the
//! caller's code that made it (`primitives::context::use_root_at`; K6a).
//!
//! Each test drains the reports, mounts one orphan part (or a part in its
//! outer root and without the inner one it also needs), and asserts the
//! exact list this mount reported.

#![expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]

use gui::prelude::*;
use gui::{Element, Scenario, div};
use primitives::avatar::ImageLoad;
use primitives::context::{MissingRoot, take_missing_roots};
use shadcn::{
    MenuItem, MessageScrollerDirection, QuestionnaireItem, SidebarMenuButtonSize, avatar_fallback, avatar_image,
    carousel_content, carousel_item, carousel_next, carousel_previous, collapsible_content, collapsible_trigger,
    collapsible_trigger_as, combobox_input, combobox_item_indicator, combobox_items, combobox_list, combobox_trigger,
    command_input, command_list, context_menu_checkbox_item, context_menu_content, context_menu_item,
    context_menu_radio_item, context_menu_sub, context_menu_sub_content, context_menu_sub_trigger,
    context_menu_trigger, dropdown_menu, dropdown_menu_checkbox_item, dropdown_menu_content, dropdown_menu_item,
    dropdown_menu_radio_item, dropdown_menu_sub, dropdown_menu_sub_content, dropdown_menu_sub_trigger,
    dropdown_menu_trigger, menubar_checkbox_item, menubar_content, menubar_item, menubar_radio_item, menubar_sub,
    menubar_sub_content, menubar_sub_trigger, menubar_trigger, message_scroller_button, message_scroller_viewport,
    navigation_menu, navigation_menu_content, navigation_menu_item, navigation_menu_trigger, questionnaire,
    questionnaire_choice, questionnaire_error, questionnaire_error_with, questionnaire_input, questionnaire_item,
    questionnaire_next, questionnaire_previous, questionnaire_progress, questionnaire_reset, questionnaire_skip,
    questionnaire_submit, resizable_handle, resizable_panel, select_content, select_item, select_value, sidebar,
    sidebar_content, sidebar_group_action, sidebar_group_label, sidebar_inset, sidebar_menu_action, sidebar_menu_badge,
    sidebar_menu_sub, sidebar_rail, sidebar_trigger, tags_input_input, tags_input_item, tags_input_item_delete,
    toggle_group_item, trace_collapsible_content, trace_collapsible_trigger,
};

/// What `make` reported when mounted: drained before, so only this mount's.
fn reports(make: impl Fn() -> Element + 'static) -> Vec<MissingRoot> {
    take_missing_roots();
    let mut s = Scenario::measuring(make, 400, 300, 1.0).expect("a scenario");
    s.settle().expect("settled");
    take_missing_roots()
}

/// The (part, root) pairs of `got`.
fn pairs(got: &[MissingRoot]) -> Vec<(&str, &str)> {
    got.iter().map(|m| (m.part, m.root)).collect()
}

/// One test per part: made outside its root, the mount reports exactly
/// these (part, root) pairs, and each names this file as where it was made.
macro_rules! needs_root {
    ($($test:ident: [$(($part:expr, $root:expr)),+] => $make:expr;)*) => {$(
        #[test]
        fn $test() {
            let got = reports(|| ($make).into_element());
            assert_eq!(pairs(&got), vec![$(($part, $root)),+]);
            for m in &got {
                assert!(m.at.file().ends_with("parts_need_roots.rs"), "made at the caller's code, not {}", m.at);
            }
        }
    )*};
}

needs_root! {
    an_avatar_image_outside_an_avatar_reports_it: [("avatar_image", "avatar")] => avatar_image(ImageLoad::Pending);
    an_avatar_fallback_outside_an_avatar_reports_it: [("avatar_fallback", "avatar")] => avatar_fallback("A");

    a_carousel_content_outside_a_carousel_reports_it: [("carousel_content", "carousel")] => carousel_content();
    a_carousel_item_outside_a_carousel_reports_it: [("carousel_item", "carousel")] => carousel_item();
    a_carousel_previous_outside_a_carousel_reports_it: [("carousel_previous", "carousel")] => carousel_previous();
    a_carousel_next_outside_a_carousel_reports_it: [("carousel_next", "carousel")] => carousel_next();

    a_combobox_trigger_outside_a_combobox_reports_it: [("combobox_trigger", "combobox")] => combobox_trigger(div());
    a_combobox_list_outside_a_combobox_reports_it: [("combobox_list", "combobox")] => combobox_list();
    a_combobox_item_indicator_outside_a_combobox_reports_it: [("combobox_item_indicator", "combobox")] => combobox_item_indicator("a");

    a_combobox_input_outside_a_combobox_reports_it: [("combobox_input", "combobox")] => combobox_input("");
    a_combobox_items_outside_a_combobox_reports_it: [("combobox_items", "combobox")] => combobox_items(Vec::<shadcn::CommandSection>::new(), |_, d| d);

    a_collapsible_trigger_outside_a_collapsible_reports_it: [("collapsible_trigger", "collapsible")] => collapsible_trigger();
    a_collapsible_trigger_as_outside_a_collapsible_reports_it: [("collapsible_trigger_as", "collapsible")] => collapsible_trigger_as(div());
    a_collapsible_content_outside_a_collapsible_reports_it: [("collapsible_content", "collapsible")] => collapsible_content();

    a_questionnaire_error_with_alone_reports_its_own_name: [("questionnaire_error_with", "questionnaire"), ("questionnaire_error_with", "questionnaire_item")] => questionnaire_error_with("x");

    a_command_input_outside_a_command_reports_it: [("command_input", "command")] => command_input("");
    a_command_list_outside_a_command_reports_it: [("command_list", "command")] => command_list(Vec::<shadcn::CommandSection>::new(), |_, d| d);

    a_trace_trigger_outside_a_collapsible_reports_it: [("trace_collapsible_trigger", "collapsible")] => trace_collapsible_trigger(false, div());
    a_trace_content_outside_a_collapsible_reports_it: [("trace_collapsible_content", "collapsible")] => trace_collapsible_content();

    a_message_scroller_viewport_outside_a_provider_reports_it: [("message_scroller_viewport", "message_scroller_provider")] => message_scroller_viewport(Vec::<u32>::new(), |_| ("", false), |_, _| div());
    a_message_scroller_button_outside_a_provider_reports_it: [("message_scroller_button", "message_scroller_provider")] => message_scroller_button(MessageScrollerDirection::End);

    a_navigation_item_outside_a_menu_reports_it: [("navigation_menu_item", "navigation_menu")] => navigation_menu_item("a");

    a_questionnaire_progress_outside_a_questionnaire_reports_it: [("questionnaire_progress", "questionnaire")] => questionnaire_progress();
    a_questionnaire_item_outside_a_questionnaire_reports_it: [("questionnaire_item", "questionnaire")] => questionnaire_item("a");
    a_questionnaire_choice_alone_reports_both_roots: [("questionnaire_choice", "questionnaire"), ("questionnaire_choice", "questionnaire_item")] => questionnaire_choice("a", div(), None);
    a_questionnaire_input_alone_reports_both_roots: [("questionnaire_input", "questionnaire"), ("questionnaire_input", "questionnaire_item")] => questionnaire_input();
    a_questionnaire_error_alone_reports_both_roots: [("questionnaire_error", "questionnaire"), ("questionnaire_error", "questionnaire_item")] => questionnaire_error();
    a_questionnaire_previous_outside_a_questionnaire_reports_it: [("questionnaire_previous", "questionnaire")] => questionnaire_previous("a");
    a_questionnaire_skip_outside_a_questionnaire_reports_it: [("questionnaire_skip", "questionnaire")] => questionnaire_skip("a");
    a_questionnaire_next_outside_a_questionnaire_reports_it: [("questionnaire_next", "questionnaire")] => questionnaire_next("a");
    a_questionnaire_submit_outside_a_questionnaire_reports_it: [("questionnaire_submit", "questionnaire")] => questionnaire_submit("a");
    a_questionnaire_reset_outside_a_questionnaire_reports_it: [("questionnaire_reset", "questionnaire")] => questionnaire_reset(div());

    a_resizable_panel_outside_a_group_reports_it: [("resizable_panel", "resizable_panel_group")] => resizable_panel(0);
    a_resizable_handle_outside_a_group_reports_it: [("resizable_handle", "resizable_panel_group")] => resizable_handle(0, false);

    a_select_value_outside_a_select_reports_it: [("select_value", "select")] => select_value("a");
    a_select_content_outside_a_select_reports_it: [("select_content", "select")] => select_content();
    a_select_item_outside_a_select_reports_it: [("select_item", "select")] => select_item("a", "a");

    a_sidebar_outside_a_provider_reports_it: [("sidebar", "sidebar_provider")] => sidebar();
    a_sidebar_trigger_outside_a_provider_reports_it: [("sidebar_trigger", "sidebar_provider")] => sidebar_trigger();
    a_sidebar_rail_outside_a_provider_reports_it: [("sidebar_rail", "sidebar_provider")] => sidebar_rail();
    a_sidebar_inset_outside_a_provider_reports_it: [("sidebar_inset", "sidebar_provider")] => sidebar_inset();
    a_sidebar_content_outside_a_provider_reports_it: [("sidebar_content", "sidebar_provider")] => sidebar_content();
    a_sidebar_group_label_outside_a_provider_reports_it: [("sidebar_group_label", "sidebar_provider")] => sidebar_group_label();
    a_sidebar_group_action_outside_a_provider_reports_it: [("sidebar_group_action", "sidebar_provider")] => sidebar_group_action();
    a_sidebar_menu_sub_outside_a_provider_reports_it: [("sidebar_menu_sub", "sidebar_provider")] => sidebar_menu_sub();
    a_sidebar_menu_action_outside_a_provider_reports_it: [("sidebar_menu_action", "sidebar_provider")] => sidebar_menu_action(SidebarMenuButtonSize::Default, false);
    a_sidebar_menu_badge_outside_a_provider_reports_it: [("sidebar_menu_badge", "sidebar_provider")] => sidebar_menu_badge(SidebarMenuButtonSize::Default);

    a_tags_input_item_outside_a_tags_input_reports_it: [("tags_input_item", "tags_input")] => tags_input_item("a");
    a_tags_input_item_delete_outside_a_tags_input_reports_it: [("tags_input_item_delete", "tags_input")] => tags_input_item_delete("a");
    a_tags_input_input_outside_a_tags_input_reports_it: [("tags_input_input", "tags_input")] => tags_input_input();

    a_toggle_group_item_outside_a_group_reports_it: [("toggle_group_item", "toggle_group")] => toggle_group_item("a");

    a_dropdown_trigger_outside_its_menu_reports_it: [("dropdown_menu_trigger", "dropdown_menu")] => dropdown_menu_trigger(div());
    a_context_menu_trigger_outside_its_menu_reports_it: [("context_menu_trigger", "context_menu")] => context_menu_trigger(div());
    a_menubar_trigger_outside_a_menubar_menu_reports_it: [("menubar_trigger", "menubar_menu")] => menubar_trigger("a");
}

/// The shared menu parts, made as each public menu: the report names that
/// menu's part and root, not the shared implementation's.
macro_rules! menu_family {
    ($family:ident, $root:literal, $group:literal, $content:ident, $sub_content:ident, $item:ident, $check:ident,
     $radio:ident, $sub:ident, $sub_trigger:ident, $p:literal) => {
        mod $family {
            use super::*;

            needs_root! {
                content_outside_its_menu_reports_it: [(concat!($p, "_content"), $root)] => $content();
                sub_content_outside_its_menu_reports_it: [(concat!($p, "_sub_content"), $root)] => $sub_content();
                item_outside_its_menu_reports_it: [(concat!($p, "_item"), $root)] => $item("a", MenuItem::default());
                checkbox_item_outside_its_menu_reports_it: [(concat!($p, "_checkbox_item"), $root)] => $check("a", false);
                sub_outside_its_menu_reports_it: [(concat!($p, "_sub"), $root)] => $sub();
                sub_trigger_outside_its_menu_reports_it: [(concat!($p, "_sub_trigger"), $root)] => $sub_trigger("a", MenuItem::default());
                radio_item_alone_reports_its_menu_and_its_group: [(concat!($p, "_radio_item"), $root), (concat!($p, "_radio_item"), $group)] => $radio("a", "a", false);
            }
        }
    };
}

menu_family!(
    dropdown,
    "dropdown_menu",
    "dropdown_menu_radio_group",
    dropdown_menu_content,
    dropdown_menu_sub_content,
    dropdown_menu_item,
    dropdown_menu_checkbox_item,
    dropdown_menu_radio_item,
    dropdown_menu_sub,
    dropdown_menu_sub_trigger,
    "dropdown_menu"
);
menu_family!(
    context,
    "context_menu",
    "context_menu_radio_group",
    context_menu_content,
    context_menu_sub_content,
    context_menu_item,
    context_menu_checkbox_item,
    context_menu_radio_item,
    context_menu_sub,
    context_menu_sub_trigger,
    "context_menu"
);
menu_family!(
    menubar,
    "menubar_menu",
    "menubar_radio_group",
    menubar_content,
    menubar_sub_content,
    menubar_item,
    menubar_checkbox_item,
    menubar_radio_item,
    menubar_sub,
    menubar_sub_trigger,
    "menubar"
);

/// A part that needs an inner root, made in its outer root without it: only
/// the inner root is reported.
#[test]
fn a_radio_item_in_a_menu_without_its_group_reports_the_group() {
    let got = reports(|| {
        dropdown_menu()
            .child(dropdown_menu_radio_item("a", "a", false))
            .into_element()
    });
    assert_eq!(
        pairs(&got),
        vec![("dropdown_menu_radio_item", "dropdown_menu_radio_group")]
    );
}

#[test]
fn questionnaire_parts_in_a_questionnaire_without_an_item_report_the_item() {
    let got = reports(|| {
        questionnaire(vec![QuestionnaireItem::new("q", &["a"])])
            .child(questionnaire_choice("a", div(), None))
            .child(questionnaire_input())
            .child(questionnaire_error())
            .into_element()
    });
    assert_eq!(
        pairs(&got),
        vec![
            ("questionnaire_choice", "questionnaire_item"),
            ("questionnaire_input", "questionnaire_item"),
            ("questionnaire_error", "questionnaire_item"),
        ]
    );
}

#[test]
fn navigation_parts_in_a_navigation_menu_without_an_item_report_the_item() {
    let got = reports(|| {
        navigation_menu()
            .child(navigation_menu_trigger("a"))
            .child(navigation_menu_content())
            .into_element()
    });
    assert_eq!(
        pairs(&got),
        vec![
            ("navigation_menu_trigger", "navigation_menu_item"),
            ("navigation_menu_content", "navigation_menu_item"),
        ]
    );
}

/// Two different orphans on one thread are two reports (one per part), and
/// two orphans of one part made at two places are two reports.
#[test]
fn different_orphans_and_different_places_are_each_reported() {
    let got = reports(|| {
        div()
            .child(carousel_next())
            .child(carousel_previous())
            .child(carousel_next())
            .into_element()
    });
    assert_eq!(
        pairs(&got),
        vec![
            ("carousel_next", "carousel"),
            ("carousel_previous", "carousel"),
            ("carousel_next", "carousel"),
        ]
    );
    assert_ne!(got[0].at, got[2].at, "one part, two places in the caller's code");
    let again = reports(|| carousel_content().into_element());
    assert_eq!(again.len(), 1, "a later mount reports only what it did");
}

/// The report is kept once per (part, root, place): a mount that runs the
/// same orphan's setup again does not report it again.
#[test]
fn an_orphan_is_reported_once_however_often_it_is_set_up() {
    let got = reports(|| {
        let mut parts = div();
        for _ in 0..3 {
            // One place in the source, three instances.
            parts = parts.child(carousel_item());
        }
        parts.into_element()
    });
    assert_eq!(pairs(&got), vec![("carousel_item", "carousel")]);
}
