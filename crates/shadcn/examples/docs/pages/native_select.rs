use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{NativeSelectEntry, native_select, native_select_opt_group, native_select_option as option, theme};

use crate::{Render, Shot, demo, page};

fn statuses() -> Vec<NativeSelectEntry> {
    vec![
        option("", "Select status"),
        option("todo", "Todo"),
        option("in-progress", "In Progress"),
        option("done", "Done"),
        option("cancelled", "Cancelled"),
    ]
}

fn departments() -> Vec<NativeSelectEntry> {
    vec![
        option("", "Select department"),
        native_select_opt_group(
            "Engineering",
            [
                option("frontend", "Frontend"),
                option("backend", "Backend"),
                option("devops", "DevOps"),
            ],
        ),
        native_select_opt_group(
            "Sales",
            [
                option("sales-rep", "Sales Rep"),
                option("account-manager", "Account Manager"),
                option("sales-director", "Sales Director"),
            ],
        ),
        native_select_opt_group(
            "Operations",
            [
                option("support", "Customer Support"),
                option("product-manager", "Product Manager"),
                option("ops-manager", "Operations Manager"),
            ],
        ),
    ]
}

fn priorities() -> Vec<NativeSelectEntry> {
    vec![
        option("", "Select priority"),
        option("low", "Low"),
        option("medium", "Medium"),
        option("high", "High"),
        option("critical", "Critical"),
    ]
}

fn roles() -> Vec<NativeSelectEntry> {
    vec![
        option("", "Select role"),
        option("admin", "Admin"),
        option("editor", "Editor"),
        option("viewer", "Viewer"),
        option("guest", "Guest"),
    ]
}

pub fn render() -> Render {
    let status = Signal::labeled("native select status", String::new());
    let department = Signal::labeled("native select department", String::new());
    Box::new(move || {
        let t = theme();
        page(
            "Native Select",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(24.0)
                .child(demo("NativeSelectDemo.vue", native_select(statuses()).model(status)))
                .child(demo(
                    "NativeSelectGroupsDemo.vue",
                    native_select(departments()).model(department),
                ))
                .child(demo(
                    "NativeSelectDisabledDemo.vue",
                    native_select(priorities()).disabled(true),
                ))
                .child(demo(
                    "NativeSelectInvalidDemo.vue",
                    native_select(roles()).invalid(true),
                ))
                // shadcn-vue has not written this one yet: its demo is the
                // line below, in the page's text.
                .child(demo("NativeSelectInputGroupDemo.vue", text("not yet ready")))
                .child(
                    text(format!(
                        "Status: {:?}. Department: {:?}.",
                        status.get(),
                        department.get()
                    ))
                    .text_size(14.0)
                    .line_height(20.0)
                    .text_color(t.muted_foreground),
                ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Select status")?;
    s.click(x, y);
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.frame("arrow down twice")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.frame("tab, focus ring, next department")?;
    Ok(())
}
