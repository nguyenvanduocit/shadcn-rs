use gui::prelude::*;
use gui::{Color, Modifiers, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{label, switch, theme};

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page, section};

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
    let airplane = Signal::labeled("airplane-mode", false);
    let bluetooth = Signal::labeled("bluetooth", true);
    let focus_mode = Signal::labeled("focus-mode", false);
    let disabled = Signal::labeled("disabled", true);
    Box::new(move || {
        let t = theme();
        // Tailwind's blue-500 (blue-600 in dark) for the checked track, blue-600 for the card.
        let blue = if t.dark {
            Color::oklch(0.546, 0.245, 262.881)
        } else {
            Color::oklch(0.623, 0.214, 259.815)
        };
        let blue_600 = Color::oklch(0.546, 0.245, 262.881);
        let (bt, fm) = (bluetooth.get(), focus_mode.get());
        let a = airplane.clone();
        let f = focus_mode.clone();
        page(
            "Switch",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "SwitchDemo.vue",
                    div()
                        .flex()
                        .items_center()
                        .gap(2.0)
                        .child(switch(&airplane))
                        .child(label("Airplane Mode").on_click(move |_| a.update(|v| !v))),
                ))
                .child(section(
                    "Coloured, disabled and in a card",
                    div()
                        .flex_col()
                        .gap(6.0)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(2.0)
                                .child(switch(&bluetooth).when(bt, |d| d.bg(blue)))
                                .child(label("Bluetooth")),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(2.0)
                                .child(switch(&disabled).disabled(true))
                                .child(label("Disabled")),
                        )
                        .child(
                            div()
                                .named("switch-card")
                                .flex()
                                .items_center()
                                .gap(6.0)
                                .max_w_px(384.0)
                                .rounded_lg()
                                .border()
                                .border_color(if fm { blue_600 } else { t.border })
                                .p(4.0)
                                .text_sm()
                                .leading_none()
                                .font_medium()
                                .on_click(move |_| f.update(|v| !v))
                                .child(
                                    div()
                                        .flex_col()
                                        .gap(1.0)
                                        .child(text("Share across devices").font_medium())
                                        .child(
                                            text(
                                                "Focus is shared across devices, and turns off when you leave the app.",
                                            )
                                            .text_sm()
                                            .font_normal()
                                            .text_color(t.muted_foreground),
                                        ),
                                )
                                .child(switch(&focus_mode).when(fm, |d| d.bg(blue))),
                        ),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = spot(s, "switch", 0)?;
    s.click(x, y);
    s.frame("clicked on")?;
    s.click(0.0, 0.0);
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab focus")?;
    s.press(NamedKey::Space, Modifiers::default());
    s.frame("space turns off")?;
    let (x, y) = spot(s, "switch", 2)?;
    s.click(x, y);
    s.frame("disabled ignores click")?;
    let (x, y) = spot(s, "switch-card", 0)?;
    s.click(x, y);
    s.frame("card turns focus mode on")?;
    motion_frames(s, "turning on", &MOTION_MS, |s| {
        let (x, y) = spot(s, "switch", 0)?;
        s.click(x, y);
        Ok(())
    })?;
    Ok(())
}
