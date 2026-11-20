use gui::prelude::*;
use gui::{Color, Modifiers, NamedKey, ScenarioError, div, text};
use shadcn::{RadioGroupState, label, radio_group, radio_group_item, theme};

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

const PLANS: [(&str, &str, &str); 2] = [
    (
        "starter",
        "Starter Plan",
        "Perfect for small businesses getting started with our platform",
    ),
    (
        "pro",
        "Pro Plan",
        "Advanced features for growing businesses with higher demands",
    ),
];

pub fn render() -> Render {
    let density = RadioGroupState::new(&["default", "comfortable", "compact"], Some("comfortable"));
    let plan = RadioGroupState::new(&["starter", "pro"], Some("starter"));
    let off = RadioGroupState::new(&["on", "off"], Some("on"));
    off.disabled.set(true);
    Box::new(move || {
        let t = theme();
        // Tailwind's green-600 (green-900 in dark) and green-50 (green-950).
        let green = if t.dark {
            Color::oklch(0.393, 0.095, 152.535)
        } else {
            Color::oklch(0.627, 0.194, 149.214)
        };
        let green_bg = if t.dark {
            Color::oklch(0.266, 0.065, 152.934)
        } else {
            Color::oklch(0.982, 0.018, 155.826)
        };
        let row = |state: &RadioGroupState, value: &str, name: &str| {
            let pick = state.value.clone();
            let owned = value.to_owned();
            div()
                .flex()
                .items_center()
                .gap(2.0)
                .child(radio_group_item(state, value))
                .child(label(name).on_click(move |_| pick.set(Some(owned.clone()))))
        };
        let chosen = plan.value.get();
        page(
            "Radio Group",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "RadioGroupDemo.vue",
                    radio_group(&density)
                        .child(row(&density, "default", "Default"))
                        .child(row(&density, "comfortable", "Comfortable"))
                        .child(row(&density, "compact", "Compact")),
                ))
                .child(section(
                    "Plans",
                    radio_group(&plan)
                        .max_w_px(384.0)
                        .children(PLANS.map(|(id, name, description)| {
                            let on = chosen.as_deref() == Some(id);
                            let pick = plan.value.clone();
                            div()
                                .named("plan")
                                .flex()
                                .items_start()
                                .gap(3.0)
                                .rounded_lg()
                                .border()
                                .border_color(if on { green } else { t.border })
                                .when(on, |d| d.bg(green_bg))
                                .when(!on, |d| d.hover(move |s| s.bg(t.accent.opacity(0.5))))
                                .p(4.0)
                                .text_sm()
                                .on_click(move |_| pick.set(Some(id.to_owned())))
                                .child(radio_group_item(&plan, id).shadow_none())
                                .child(
                                    div()
                                        .grid()
                                        .gap(1.0)
                                        .font_normal()
                                        .child(text(name).font_medium())
                                        .child(text(description).leading_snug().text_color(t.muted_foreground)),
                                )
                        })),
                ))
                .child(section(
                    "Disabled",
                    radio_group(&off)
                        .child(row(&off, "on", "On"))
                        .child(row(&off, "off", "Off")),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = spot(s, "radio-group-item", 0)?;
    s.click(x, y);
    s.frame("clicked default")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab focus")?;
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.frame("arrow down checks next")?;
    s.press(NamedKey::ArrowUp, Modifiers::default());
    s.press(NamedKey::ArrowUp, Modifiers::default());
    s.press(NamedKey::ArrowUp, Modifiers::default());
    s.frame("arrow up wraps to last")?;
    s.click_text("Comfortable")?;
    s.frame("label click")?;
    s.click_text("Pro Plan")?;
    s.frame("plan card click")?;
    let (x, y) = spot(s, "radio-group-item", 6)?;
    s.click(x, y);
    s.frame("disabled ignores click")?;
    Ok(())
}
