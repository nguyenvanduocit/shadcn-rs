use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, div, icon};
use shadcn::{
    Part, ToggleGroupModel, ToggleGroupProps, ToggleSize, ToggleVariant, toggle_group, toggle_group_item,
    toggle_group_multiple,
};

use crate::{Render, Shot, demo, page, section};

/// The centre of the `nth` node named `name` in the last frame.
fn spot(s: &Shot, name: &str, nth: usize) -> Result<(f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .filter(|n| n.name == Some(name))
        .nth(nth)
        .map(|n| n.bounds(gui::inspect::OUTSIDE))
        .map(|b| (b.x + b.width / 2.0, b.y + b.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound(format!("{name} #{nth}")))
}

const BIU: [(&str, Icon); 3] = [
    ("bold", Icon::Bold),
    ("italic", Icon::Italic),
    ("underline", Icon::Underline),
];

/// The bold, italic and underline items with their icons.
fn icons<M: ToggleGroupModel>(group: Part<ToggleGroupProps<M>>) -> Part<ToggleGroupProps<M>> {
    group.children(BIU.map(|(v, i)| toggle_group_item(v).child(icon(i))))
}

pub fn render() -> Render {
    Box::new(move || {
        page(
            "Toggle Group",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(20.0)
                .child(demo(
                    "ToggleGroupDemo.vue",
                    icons(toggle_group_multiple().variant(ToggleVariant::Outline)),
                ))
                .child(demo("ToggleGroupSingleDemo.vue", icons(toggle_group())))
                .child(demo("ToggleGroupDefaultDemo.vue", icons(toggle_group_multiple())))
                .child(demo(
                    "ToggleGroupDisabledDemo.vue",
                    icons(toggle_group_multiple().disabled(true)),
                ))
                .child(demo(
                    "ToggleGroupLargeDemo.vue",
                    icons(toggle_group_multiple().size(ToggleSize::Lg)),
                ))
                .child(demo(
                    "ToggleGroupSmallDemo.vue",
                    icons(toggle_group().size(ToggleSize::Sm)),
                ))
                .child(section(
                    "Text items, joined and spaced",
                    div()
                        .display(Display::Flex)
                        .align_items(Align::Start)
                        .gap(16.0)
                        .child(
                            toggle_group()
                                .variant(ToggleVariant::Outline)
                                .default_value(Some("all".to_owned()))
                                .child(toggle_group_item("all").class(|d| d.width(80.0)).child("All"))
                                .child(toggle_group_item("missed").class(|d| d.width(80.0)).child("Missed")),
                        )
                        .child(
                            toggle_group()
                                .variant(ToggleVariant::Outline)
                                .size(ToggleSize::Sm)
                                .spacing(2.0)
                                .default_value(Some("last-24-hours".to_owned()))
                                .child(toggle_group_item("last-24-hours").child("Last 24 hours"))
                                .child(toggle_group_item("last-7-days").child("Last 7 days")),
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
