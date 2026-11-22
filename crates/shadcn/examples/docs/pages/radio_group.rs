use gui::prelude::*;
use gui::{Color, Modifiers, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{label, radio_group, radio_group_item, theme};

use crate::{Render, Shot, demo, page, section};
use shadcn::focus::StateLook;

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
    let density = Signal::labeled("density", Some("comfortable".to_owned()));
    let plan = Signal::labeled("plan", Some("starter".to_owned()));
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
        let row = |pick: Option<Signal<Option<String>>>, value: &str, name: &str| {
            let owned = value.to_owned();
            div()
                .display(Display::Flex)
                .align_items(Align::Center)
                .gap(8.0)
                .child(radio_group_item(value))
                .child(label(name).when_some(pick, |l, pick| l.on_click(move |_| pick.set(Some(owned.clone())))))
        };
        let chosen = plan.get();
        page(
            "Radio Group",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "RadioGroupDemo.vue",
                    radio_group()
                        .model(density)
                        .child(row(Some(density), "default", "Default"))
                        .child(row(Some(density), "comfortable", "Comfortable"))
                        .child(row(Some(density), "compact", "Compact")),
                ))
                .child(section(
                    "Plans",
                    radio_group()
                        .model(plan)
                        .max_width(384.0)
                        .children(PLANS.map(|(id, name, description)| {
                            let on = chosen.as_deref() == Some(id);
                            let pick = plan;
                            div()
                                .named("plan")
                                .display(Display::Flex)
                                .align_items(Align::Start)
                                .gap(12.0)
                                .radius(10.0)
                                .border(1.0)
                                .border_color(if on { green } else { t.border })
                                .when(on, |d| d.background(green_bg))
                                .when(!on, |d| {
                                    d.with_interaction(|d, i| {
                                        d.background_while(move || i.hovered(), t.accent.opacity(0.5))
                                    })
                                })
                                .padding(16.0)
                                .text_size(14.0)
                                .line_height(20.0)
                                .on_click(move |_| pick.set(Some(id.to_owned())))
                                .child(radio_group_item(id).shadow(Shadows::new()))
                                .child(
                                    div()
                                        .display(Display::Grid)
                                        .gap(4.0)
                                        .font_weight(400)
                                        .child(text(name).font_weight(500))
                                        .child(
                                            text(description)
                                                .line_height(LineHeight::Relative(1.375))
                                                .text_color(t.muted_foreground),
                                        ),
                                )
                        })),
                ))
                .child(section(
                    "Disabled",
                    radio_group()
                        .default_value("on")
                        .disabled(true)
                        .child(row(None, "on", "On"))
                        .child(row(None, "off", "Off")),
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
