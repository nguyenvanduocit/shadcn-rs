use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{Orientation, label, slider, theme};

use crate::{Render, Shot, demo, page, section};

/// The `nth` node named `name` in the last frame: its centre and width.
fn spot(s: &Shot, name: &str, nth: usize) -> Result<(f32, f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .filter(|n| n.name == Some(name))
        .nth(nth)
        .map(|n| {
            (
                n.bounds.x + n.bounds.width / 2.0,
                n.bounds.y + n.bounds.height / 2.0,
                n.bounds.width,
            )
        })
        .ok_or_else(|| ScenarioError::TextNotFound(format!("{name} #{nth}")))
}

pub fn render() -> Render {
    let temperature = Signal::labeled("temperature", vec![0.3_f64, 0.7]);
    Box::new(move || {
        let t = theme();
        let shown = temperature
            .get()
            .iter()
            .map(f64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        page(
            "Slider",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "SliderDemo.vue",
                    div().width(pct(100.0)).child(
                        slider(Orientation::Horizontal)
                            .default_value(vec![50.0])
                            .width(Length::Fraction(0.6)),
                    ),
                ))
                .child(section(
                    "Range, steps of ten, vertical, labelled, disabled",
                    div()
                        .display(Display::Flex)
                        .gap(48.0)
                        .child(
                            div()
                                .display(Display::Flex)
                                .flex_direction(FlexDirection::Column)
                                .width(384.0)
                                .gap(24.0)
                                .child(slider(Orientation::Horizontal).default_value(vec![25.0, 50.0]))
                                .child(
                                    slider(Orientation::Horizontal)
                                        .default_value(vec![10.0, 20.0])
                                        .step(10.0),
                                )
                                .child(
                                    div()
                                        .display(Display::Grid)
                                        .width(pct(100.0))
                                        .gap(12.0)
                                        .child(
                                            div()
                                                .display(Display::Flex)
                                                .align_items(Align::Center)
                                                .justify_content(Justify::Between)
                                                .gap(8.0)
                                                .child(label("Temperature"))
                                                .child(
                                                    text(shown)
                                                        .text_size(14.0)
                                                        .line_height(20.0)
                                                        .text_color(t.muted_foreground),
                                                ),
                                        )
                                        .child(slider(Orientation::Horizontal).model(temperature).max(1.0).step(0.1)),
                                )
                                .child(slider(Orientation::Horizontal).default_value(vec![40.0]).disabled(true)),
                        )
                        .child(
                            div()
                                .display(Display::Flex)
                                .align_items(Align::Center)
                                .gap(24.0)
                                .child(slider(Orientation::Vertical).default_value(vec![50.0]))
                                .child(slider(Orientation::Vertical).default_value(vec![25.0])),
                        ),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    // Drag the first slider from its thumb to three quarters along.
    let (x, y, _) = spot(s, "slider-thumb", 0)?;
    let (cx, _, w) = spot(s, "slider", 0)?;
    s.drag((x, y), (cx - w / 2.0 + w * 0.75, y));
    s.frame("dragged to 75")?;
    s.hover(0.0, 0.0);
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab focus")?;
    s.press(NamedKey::ArrowRight, Modifiers::default());
    s.press(NamedKey::ArrowRight, Modifiers::default());
    s.frame("arrow right twice")?;
    s.press(NamedKey::Home, Modifiers::default());
    s.frame("home")?;
    // Click the range slider past its upper thumb: the nearer thumb follows.
    let (cx, y, w) = spot(s, "slider", 1)?;
    s.click(cx - w / 2.0 + w * 0.9, y);
    s.frame("range upper to 90")?;
    // Temperature: drag the lower thumb past the upper one; they swap.
    let (x, y, _) = spot(s, "slider-thumb", 5)?;
    let (cx, _, w) = spot(s, "slider", 3)?;
    s.drag((x, y), (cx - w / 2.0 + w * 0.95, y));
    s.frame("temperature crossed")?;
    // Vertical: drag up to the top.
    let (x, y, _) = spot(s, "slider-thumb", 8)?;
    let (_, cy, _) = spot(s, "slider", 5)?;
    s.drag((x, y), (x, cy - 200.0));
    s.frame("vertical to top")?;
    let (x, y, _) = spot(s, "slider-thumb", 7)?;
    s.drag((x, y), (x + 200.0, y));
    s.frame("disabled ignores drag")?;
    Ok(())
}
