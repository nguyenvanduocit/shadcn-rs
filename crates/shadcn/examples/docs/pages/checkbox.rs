use gui::prelude::*;
use gui::{Color, Modifiers, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{checkbox, label, theme};

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
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "CheckboxDemo.vue",
                    div()
                        .flex_col()
                        .gap(6.0)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(3.0)
                                .child(checkbox().model(terms))
                                .child(label("Accept terms and conditions").on_click(move |_| a.update(|v| *v = !*v))),
                        )
                        .child(
                            div()
                                .flex()
                                .items_start()
                                .gap(3.0)
                                .child(checkbox().model(terms2))
                                .child(
                                    div()
                                        .grid()
                                        .gap(2.0)
                                        .child(
                                            label("Accept terms and conditions")
                                                .on_click(move |_| b.update(|v| *v = !*v)),
                                        )
                                        .child(
                                            text("By clicking this checkbox, you agree to the terms and conditions.")
                                                .text_sm()
                                                .text_color(t.muted_foreground),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_start()
                                .gap(3.0)
                                .child(checkbox().disabled(true))
                                .child(label("Enable notifications").disabled(true)),
                        )
                        .child(
                            div()
                                .named("checkbox-card")
                                .flex()
                                .items_start()
                                .gap(3.0)
                                .rounded_lg()
                                .border()
                                .border_color(t.border)
                                .p(3.0)
                                .text_sm()
                                .leading_none()
                                .font_medium()
                                .when(notify2.get(), |d| d.border_color(card_border).bg(card_bg))
                                .when(!notify2.get(), |d| d.hover(move |s| s.bg(t.accent.opacity(0.5))))
                                .on_click(move |_| c.update(|v| *v = !*v))
                                .child(checkbox().model(notify2).class(move |d| {
                                    d.when(notify2.get(), |d| {
                                        d.border_color(blue).bg(blue).text_color(Color::WHITE)
                                    })
                                }))
                                .child(
                                    div()
                                        .grid()
                                        .gap(1.5)
                                        .font_normal()
                                        .child(text("Enable notifications").text_sm().leading_none().font_medium())
                                        .child(
                                            text("You can enable or disable notifications at any time.")
                                                .text_sm()
                                                .text_color(t.muted_foreground),
                                        ),
                                ),
                        ),
                ))
                .child(demo(
                    "CheckboxDisabled.vue",
                    // `space-x-2`: the label 0.5rem after the box.
                    div()
                        .flex()
                        .items_center()
                        .gap(2.0)
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
