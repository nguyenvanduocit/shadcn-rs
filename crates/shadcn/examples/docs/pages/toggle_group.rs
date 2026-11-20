use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, Signal, div, icon};
use shadcn::{ToggleGroupSelection, ToggleGroupState, ToggleSize, ToggleVariant, toggle_group, toggle_group_item};

use crate::{Render, Shot, demo, page, section};

/// The centre of the `nth` node named `name` in the last frame.
fn spot(s: &Shot, name: &str, nth: usize) -> Result<(f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .filter(|n| n.name == Some(name))
        .nth(nth)
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound(format!("{name} #{nth}")))
}

const BIU: [(&str, Icon); 3] = [
    ("bold", Icon::Bold),
    ("italic", Icon::Italic),
    ("underline", Icon::Underline),
];

fn multiple() -> ToggleGroupSelection {
    ToggleGroupSelection::Multiple(Signal::new(Vec::new()))
}

fn single(value: Option<&str>) -> ToggleGroupSelection {
    ToggleGroupSelection::Single(Signal::new(value.map(str::to_owned)))
}

fn icons(state: &ToggleGroupState) -> gui::Div {
    toggle_group(state).children(BIU.map(|(v, i)| toggle_group_item(state, v).child(icon(i))))
}

pub fn render() -> Render {
    let values = BIU.map(|(v, _)| v);
    let outline = ToggleGroupState::new(multiple(), &values).variant(ToggleVariant::Outline);
    let one = ToggleGroupState::new(single(None), &values);
    let plain = ToggleGroupState::new(multiple(), &values);
    let disabled = ToggleGroupState::new(multiple(), &values);
    disabled.disabled.set(true);
    let large = ToggleGroupState::new(multiple(), &values).size(ToggleSize::Lg);
    let small = ToggleGroupState::new(single(None), &values).size(ToggleSize::Sm);
    let missed = ToggleGroupState::new(single(Some("all")), &["all", "missed"]).variant(ToggleVariant::Outline);
    let spaced = ToggleGroupState::new(single(Some("last-24-hours")), &["last-24-hours", "last-7-days"])
        .variant(ToggleVariant::Outline)
        .size(ToggleSize::Sm)
        .spacing(2.0);
    Box::new(move || {
        page(
            "Toggle Group",
            div()
                .flex_col()
                .gap(5.0)
                .child(demo("ToggleGroupDemo.vue", icons(&outline)))
                .child(demo("ToggleGroupSingleDemo.vue", icons(&one)))
                .child(demo("ToggleGroupDefaultDemo.vue", icons(&plain)))
                .child(demo("ToggleGroupDisabledDemo.vue", icons(&disabled)))
                .child(demo("ToggleGroupLargeDemo.vue", icons(&large)))
                .child(demo("ToggleGroupSmallDemo.vue", icons(&small)))
                .child(section(
                    "Text items, joined and spaced",
                    div()
                        .flex()
                        .items_start()
                        .gap(4.0)
                        .child(
                            toggle_group(&missed)
                                .child(toggle_group_item(&missed, "all").w(20.0).child("All"))
                                .child(toggle_group_item(&missed, "missed").w(20.0).child("Missed")),
                        )
                        .child(
                            toggle_group(&spaced)
                                .child(toggle_group_item(&spaced, "last-24-hours").child("Last 24 hours"))
                                .child(toggle_group_item(&spaced, "last-7-days").child("Last 7 days")),
                        ),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    // Multiple: bold and underline both stay pressed.
    let (x, y) = spot(s, "toggle-group-item", 0)?;
    s.click(x, y);
    let (x, y) = spot(s, "toggle-group-item", 2)?;
    s.click(x, y);
    s.hover(0.0, 0.0);
    s.frame("multiple bold and underline")?;
    // Single: pressing italic then underline leaves only underline.
    let (x, y) = spot(s, "toggle-group-item", 4)?;
    s.click(x, y);
    let (x, y) = spot(s, "toggle-group-item", 5)?;
    s.click(x, y);
    s.hover(0.0, 0.0);
    s.frame("single underline only")?;
    // Disabled group ignores clicks.
    let (x, y) = spot(s, "toggle-group-item", 9)?;
    s.click(x, y);
    s.frame("disabled ignores click")?;
    // Keyboard: Tab into the first group, arrows rove, Space presses.
    s.click(0.0, 0.0);
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab focus")?;
    s.press(NamedKey::ArrowRight, Modifiers::default());
    s.press(NamedKey::Space, Modifiers::default());
    s.frame("arrow right then space")?;
    s.click_text("Missed")?;
    s.frame("missed chosen")?;
    Ok(())
}
