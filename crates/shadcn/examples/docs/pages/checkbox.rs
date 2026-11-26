use gui::prelude::*;
use gui::{Color, Modifiers, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{checkbox, label, theme};

use crate::{Render, Shot, demo, page};
use shadcn::focus::StateLook;

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

pub fn render() -> Render {
    let terms = Signal::labeled("terms", false);
    let terms2 = Signal::labeled("terms-2", true);
    let notify2 = Signal::labeled("toggle-2", true);
    Box::new(move || {
        let t = theme();
        // Tailwind's blue-600/700 and blue-50/900/950.
        let blue = if t.dark {
            Color::oklch(0.488, 0.243, 264.376)
        } else {
            Color::oklch(0.546, 0.245, 262.881)
        };
        let card_border = if t.dark {
            Color::oklch(0.379, 0.146, 265.522)
        } else {
            blue
        };
        let card_bg = if t.dark {
            Color::oklch(0.282, 0.091, 267.935)
        } else {
            Color::oklch(0.97, 0.014, 254.604)
        };
        let (a, b, c) = (terms, terms2, notify2);
        page(
            "Checkbox",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "CheckboxDemo.vue",
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(24.0)
                        .child(
                            div()
                                .display(Display::Flex)
                                .align_items(Align::Center)
                                .gap(12.0)
                                .child(checkbox().model(terms))
                                .child(label("Accept terms and conditions").on_click(move |_| a.update(|v| *v = !*v))),
                        )
                        .child(
                            div()
                                .display(Display::Flex)
                                .align_items(Align::Start)
                                .gap(12.0)
                                .child(checkbox().model(terms2))
                                .child(
                                    div()
                                        .display(Display::Grid)
                                        .gap(8.0)
                                        .child(
                                            label("Accept terms and conditions")
                                                .on_click(move |_| b.update(|v| *v = !*v)),
                                        )
                                        .child(
                                            text("By clicking this checkbox, you agree to the terms and conditions.")
                                                .text_size(14.0)
                                                .line_height(20.0)
                                                .text_color(t.muted_foreground),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .display(Display::Flex)
                                .align_items(Align::Start)
                                .gap(12.0)
                                .child(checkbox().disabled(true))
                                .child(label("Enable notifications").disabled(true)),
                        )
                        .child(
                            div()
                                .named("checkbox-card")
                                .display(Display::Flex)
                                .align_items(Align::Start)
                                .gap(12.0)
                                .radius(10.0)
                                .border(1.0)
                                .border_color(t.border)
                                .padding(12.0)
                                .text_size(14.0)
                                .line_height(20.0)
                                .line_height(LineHeight::Relative(1.0))
                                .font_weight(500)
                                .when(notify2.get(), |d| d.border_color(card_border).background(card_bg))
                                .when(!notify2.get(), |d| {
                                    d.with_interaction(|d, i| {
                                        d.background_while(move || i.hovered(), t.accent.opacity(0.5))
                                    })
                                })
                                .on_click(move |_| c.update(|v| *v = !*v))
                                .child(checkbox().model(notify2).class(move |d| {
                                    d.when(notify2.get(), |d| {
                                        d.border_color(blue).background(blue).text_color(Color::WHITE)
                                    })
                                }))
                                .child(
                                    div()
                                        .display(Display::Grid)
                                        .gap(6.0)
                                        .font_weight(400)
                                        .child(
                                            text("Enable notifications")
                                                .text_size(14.0)
                                                .line_height(20.0)
                                                .line_height(LineHeight::Relative(1.0))
                                                .font_weight(500),
                                        )
                                        .child(
                                            text("You can enable or disable notifications at any time.")
                                                .text_size(14.0)
                                                .line_height(20.0)
                                                .text_color(t.muted_foreground),
                                        ),
                                ),
                        ),
                ))
                .child(demo(
                    "CheckboxDisabled.vue",
                    // `space-x-2`: the label 0.5rem after the box.
                    div()
                        .display(Display::Flex)
                        .align_items(Align::Center)
                        .gap(8.0)
                        .child(checkbox().disabled(true))
                        .child(label("Accept terms and conditions").disabled(true)),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = spot(s, "checkbox", 0)?;
    s.click(x, y);
    s.frame("clicked first")?;
    s.click_text("Accept terms and conditions")?;
    s.frame("label click unchecks")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab focus")?;
    s.press(NamedKey::Space, Modifiers::default());
    s.frame("space checks")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab to checked")?;
    let (x, y) = spot(s, "checkbox", 2)?;
    s.click(x, y);
    s.frame("disabled ignores click")?;
    let (x, y) = spot(s, "checkbox-card", 0)?;
    s.click(x, y);
    s.frame("card click unchecks")?;
    Ok(())
}
