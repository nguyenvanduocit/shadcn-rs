use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, Signal, div, icon};
use shadcn::{ToggleSize, ToggleVariant, toggle};

use crate::{Render, Shot, demo, page};

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

pub fn render() -> Render {
    let states: [Signal<bool>; 6] = std::array::from_fn(|_| Signal::new(false));
    Box::new(move || {
        let [bold, outline, disabled, small, large, text] = &states;
        page(
            "Toggle",
            div()
                .flex_col()
                .gap(6.0)
                .child(demo(
                    "ToggleDemo.vue",
                    toggle(ToggleVariant::Default, ToggleSize::Default, bold).child(icon(Icon::Bold)),
                ))
                .child(demo(
                    "ToggleOutlineDemo.vue",
                    toggle(ToggleVariant::Outline, ToggleSize::Default, outline).child(icon(Icon::Bold)),
                ))
                .child(demo(
                    "ToggleDisabledDemo.vue",
                    toggle(ToggleVariant::Default, ToggleSize::Default, disabled)
                        .disabled(true)
                        .child(icon(Icon::Underline)),
                ))
                .child(demo(
                    "ToggleSmallDemo.vue",
                    toggle(ToggleVariant::Default, ToggleSize::Sm, small).child(icon(Icon::Italic)),
                ))
                .child(demo(
                    "ToggleLargeDemo.vue",
                    toggle(ToggleVariant::Default, ToggleSize::Lg, large).child(icon(Icon::Italic)),
                ))
                .child(demo(
                    "ToggleTextDemo.vue",
                    toggle(ToggleVariant::Default, ToggleSize::Default, text)
                        .child(icon(Icon::Italic))
                        .child("Italic"),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = spot(s, "toggle", 0)?;
    s.hover(x, y);
    s.frame("hover default")?;
    s.click(x, y);
    s.frame("pressed default")?;
    let (x, y) = spot(s, "toggle", 1)?;
    s.hover(x, y);
    s.frame("hover outline")?;
    s.click(x, y);
    s.hover(0.0, 0.0);
    s.frame("pressed outline")?;
    let (x, y) = spot(s, "toggle", 2)?;
    s.click(x, y);
    s.frame("disabled ignores click")?;
    s.click(0.0, 0.0);
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab focus")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.press(NamedKey::Tab, Modifiers::default());
    s.press(NamedKey::Space, Modifiers::default());
    s.frame("space presses small")?;
    s.click_text("Italic")?;
    s.frame("text toggle pressed")?;
    Ok(())
}
