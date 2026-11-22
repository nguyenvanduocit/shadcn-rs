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
    Box::new(move || {
        let t = theme();
        // Tailwind's blue-500 (blue-600 in dark) for the checked track, blue-600 for the card.
        let blue = if t.dark {
            Color::oklch(0.546, 0.245, 262.881)
        } else {
            Color::oklch(0.623, 0.214, 259.815)
        };
        let blue_600 = Color::oklch(0.546, 0.245, 262.881);
        let fm = focus_mode.get();
        let a = airplane;
        let f = focus_mode;
        page(
            "Switch",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "SwitchDemo.vue",
                    div()
                        .display(Display::Flex)
                        .align_items(Align::Center)
                        .gap(8.0)
                        .child(switch().model(airplane))
                        .child(label("Airplane Mode").on_click(move |_| a.update(|v| *v = !*v))),
                ))
                .child(section(
                    "Coloured, disabled and in a card",
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(24.0)
                        .child(
                            div()
                                .display(Display::Flex)
                                .align_items(Align::Center)
                                .gap(8.0)
                                .child(
                                    switch()
                                        .model(bluetooth)
                                        .class(move |d| d.when(bluetooth.get(), |d| d.background(blue))),
                                )
                                .child(label("Bluetooth")),
                        )
                        .child(
                            div()
                                .display(Display::Flex)
                                .align_items(Align::Center)
                                .gap(8.0)
                                .child(switch().default_checked(true).disabled(true))
                                .child(label("Disabled")),
                        )
                        .child(
                            div()
                                .named("switch-card")
                                .display(Display::Flex)
                                .align_items(Align::Center)
                                .gap(24.0)
                                .max_width(384.0)
                                .radius(10.0)
                                .border(1.0)
                                .border_color(if fm { blue_600 } else { t.border })
                                .padding(16.0)
                                .text_size(14.0)
                                .line_height(20.0)
                                .line_height(LineHeight::Relative(1.0))
                                .font_weight(500)
                                .on_click(move |_| f.update(|v| *v = !*v))
                                .child(
                                    div()
                                        .display(Display::Flex)
                                        .flex_direction(FlexDirection::Column)
                                        .gap(4.0)
                                        .child(text("Share across devices").font_weight(500))
                                        .child(
                                            text(
                                                "Focus is shared across devices, and turns off when you leave the app.",
                                            )
                                            .text_size(14.0)
                                            .line_height(20.0)
                                            .font_weight(400)
                                            .text_color(t.muted_foreground),
                                        ),
                                )
                                .child(
                                    switch()
                                        .model(focus_mode)
                                        .class(move |d| d.when(focus_mode.get(), |d| d.background(blue))),
                                ),
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
