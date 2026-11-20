//! Every gallery page, in the order the gallery lists them.

use crate::Page;

mod accordion;
mod alert;
mod alert_dialog;
mod aspect_ratio;
mod attachment;
mod avatar;
mod badge;
mod breadcrumb;
mod bubble;
mod button;
mod button_group;
mod calendar;
mod card;
mod carousel;
mod chart;
mod chat;
mod checkbox;
mod code_block;
mod collapsible;
mod combobox;
mod command;
mod context_menu;
mod demo_chat;
mod dialog;
mod drawer;
mod dropdown_menu;
mod empty;
mod field;
mod file_tree;
mod form;
mod form_cards;
mod form_formisch;
mod form_stepper;
mod form_tanstack;
mod form_vee;
mod hover_card;
mod input;
mod input_group;
mod input_otp;
mod item;
mod kbd;
mod label;
mod markdown;
mod marker;
mod menubar;
mod message;
mod message_scroller;
mod native_select;
mod navigation_menu;
mod number_field;
mod pagination;
mod pin_input;
mod plan;
mod popover;
mod progress;
mod prompt_input;
mod questionnaire;
mod radio_group;
mod range_calendar;
mod reasoning;
mod resizable;
mod scroll_area;
mod scroll_fade;
mod select;
mod separator;
mod sheet;
mod shimmer;
mod sidebar;
mod skeleton;
mod slider;
mod sonner;
mod spinner;
mod stack_trace;
mod stepper;
mod subagent;
mod switch;
mod table;
mod tabs;
mod tags_input;
mod terminal;
mod test_results;
mod textarea;
mod thread;
mod toggle;
mod toggle_group;
mod tool;
mod tooltip;
mod trace;
mod typography;

pub const ALL: &[Page] = &[
    Page {
        name: "accordion",
        render: accordion::render,
        script: accordion::script,
    },
    Page {
        name: "alert",
        render: alert::render,
        script: alert::script,
    },
    Page {
        name: "alert-dialog",
        render: alert_dialog::render,
        script: alert_dialog::script,
    },
    Page {
        name: "aspect-ratio",
        render: aspect_ratio::render,
        script: aspect_ratio::script,
    },
    Page {
        name: "attachment",
        render: attachment::render,
        script: attachment::script,
    },
    Page {
        name: "avatar",
        render: avatar::render,
        script: avatar::script,
    },
    Page {
        name: "badge",
        render: badge::render,
        script: badge::script,
    },
    Page {
        name: "breadcrumb",
        render: breadcrumb::render,
        script: breadcrumb::script,
    },
    Page {
        name: "bubble",
        render: bubble::render,
        script: bubble::script,
    },
    Page {
        name: "button",
        render: button::render,
        script: button::script,
    },
    Page {
        name: "button-group",
        render: button_group::render,
        script: button_group::script,
    },
    Page {
        name: "calendar",
        render: calendar::render,
        script: calendar::script,
    },
    Page {
        name: "card",
        render: card::render,
        script: card::script,
    },
    Page {
        name: "carousel",
        render: carousel::render,
        script: carousel::script,
    },
    Page {
        name: "chart",
        render: chart::render,
        script: chart::script,
    },
    Page {
        name: "chat",
        render: chat::render,
        script: chat::script,
    },
    Page {
        name: "checkbox",
        render: checkbox::render,
        script: checkbox::script,
    },
    Page {
        name: "code-block",
        render: code_block::render,
        script: code_block::script,
    },
    Page {
        name: "collapsible",
        render: collapsible::render,
        script: collapsible::script,
    },
    Page {
        name: "combobox",
        render: combobox::render,
        script: combobox::script,
    },
    Page {
        name: "command",
        render: command::render,
        script: command::script,
    },
    Page {
        name: "context-menu",
        render: context_menu::render,
        script: context_menu::script,
    },
    Page {
        name: "dialog",
        render: dialog::render,
        script: dialog::script,
    },
    Page {
        name: "drawer",
        render: drawer::render,
        script: drawer::script,
    },
    Page {
        name: "dropdown-menu",
        render: dropdown_menu::render,
        script: dropdown_menu::script,
    },
    Page {
        name: "empty",
        render: empty::render,
        script: empty::script,
    },
    Page {
        name: "field",
        render: field::render,
        script: field::script,
    },
    Page {
        name: "file-tree",
        render: file_tree::render,
        script: file_tree::script,
    },
    Page {
        name: "form",
        render: form::render,
        script: form::script,
    },
    Page {
        name: "form_formisch",
        render: form_formisch::render,
        script: form_formisch::script,
    },
    Page {
        name: "form_stepper",
        render: form_stepper::render,
        script: form_stepper::script,
    },
    Page {
        name: "form_tanstack",
        render: form_tanstack::render,
        script: form_tanstack::script,
    },
    Page {
        name: "form_vee",
        render: form_vee::render,
        script: form_vee::script,
    },
    Page {
        name: "hover-card",
        render: hover_card::render,
        script: hover_card::script,
    },
    Page {
        name: "input",
        render: input::render,
        script: input::script,
    },
    Page {
        name: "input-group",
        render: input_group::render,
        script: input_group::script,
    },
    Page {
        name: "input-otp",
        render: input_otp::render,
        script: input_otp::script,
    },
    Page {
        name: "item",
        render: item::render,
        script: item::script,
    },
    Page {
        name: "kbd",
        render: kbd::render,
        script: kbd::script,
    },
    Page {
        name: "label",
        render: label::render,
        script: label::script,
    },
    Page {
        name: "marker",
        render: marker::render,
        script: marker::script,
    },
    Page {
        name: "menubar",
        render: menubar::render,
        script: menubar::script,
    },
    Page {
        name: "message",
        render: message::render,
        script: message::script,
    },
    Page {
        name: "message-scroller",
        render: message_scroller::render,
        script: message_scroller::script,
    },
    Page {
        name: "native-select",
        render: native_select::render,
        script: native_select::script,
    },
    Page {
        name: "navigation-menu",
        render: navigation_menu::render,
        script: navigation_menu::script,
    },
    Page {
        name: "number-field",
        render: number_field::render,
        script: number_field::script,
    },
    Page {
        name: "pagination",
        render: pagination::render,
        script: pagination::script,
    },
    Page {
        name: "pin-input",
        render: pin_input::render,
        script: pin_input::script,
    },
    Page {
        name: "plan",
        render: plan::render,
        script: plan::script,
    },
    Page {
        name: "popover",
        render: popover::render,
        script: popover::script,
    },
    Page {
        name: "progress",
        render: progress::render,
        script: progress::script,
    },
    Page {
        name: "prompt-input",
        render: prompt_input::render,
        script: prompt_input::script,
    },
    Page {
        name: "questionnaire",
        render: questionnaire::render,
        script: questionnaire::script,
    },
    Page {
        name: "radio-group",
        render: radio_group::render,
        script: radio_group::script,
    },
    Page {
        name: "range-calendar",
        render: range_calendar::render,
        script: range_calendar::script,
    },
    Page {
        name: "reasoning",
        render: reasoning::render,
        script: reasoning::script,
    },
    Page {
        name: "resizable",
        render: resizable::render,
        script: resizable::script,
    },
    Page {
        name: "scroll-area",
        render: scroll_area::render,
        script: scroll_area::script,
    },
    Page {
        name: "scroll-fade",
        render: scroll_fade::render,
        script: scroll_fade::script,
    },
    Page {
        name: "select",
        render: select::render,
        script: select::script,
    },
    Page {
        name: "separator",
        render: separator::render,
        script: separator::script,
    },
    Page {
        name: "sheet",
        render: sheet::render,
        script: sheet::script,
    },
    Page {
        name: "shimmer",
        render: shimmer::render,
        script: shimmer::script,
    },
    Page {
        name: "sidebar",
        render: sidebar::render,
        script: sidebar::script,
    },
    Page {
        name: "skeleton",
        render: skeleton::render,
        script: skeleton::script,
    },
    Page {
        name: "slider",
        render: slider::render,
        script: slider::script,
    },
    Page {
        name: "sonner",
        render: sonner::render,
        script: sonner::script,
    },
    Page {
        name: "spinner",
        render: spinner::render,
        script: spinner::script,
    },
    Page {
        name: "stack-trace",
        render: stack_trace::render,
        script: stack_trace::script,
    },
    Page {
        name: "stepper",
        render: stepper::render,
        script: stepper::script,
    },
    Page {
        name: "subagent",
        render: subagent::render,
        script: subagent::script,
    },
    Page {
        name: "switch",
        render: switch::render,
        script: switch::script,
    },
    Page {
        name: "table",
        render: table::render,
        script: table::script,
    },
    Page {
        name: "tabs",
        render: tabs::render,
        script: tabs::script,
    },
    Page {
        name: "tags-input",
        render: tags_input::render,
        script: tags_input::script,
    },
    Page {
        name: "terminal",
        render: terminal::render,
        script: terminal::script,
    },
    Page {
        name: "test-results",
        render: test_results::render,
        script: test_results::script,
    },
    Page {
        name: "textarea",
        render: textarea::render,
        script: textarea::script,
    },
    Page {
        name: "toggle",
        render: toggle::render,
        script: toggle::script,
    },
    Page {
        name: "toggle-group",
        render: toggle_group::render,
        script: toggle_group::script,
    },
    Page {
        name: "tool",
        render: tool::render,
        script: tool::script,
    },
    Page {
        name: "tooltip",
        render: tooltip::render,
        script: tooltip::script,
    },
    Page {
        name: "trace",
        render: trace::render,
        script: trace::script,
    },
    Page {
        name: "typography",
        render: typography::render,
        script: typography::script,
    },
];
