use std::time::Duration;

use gui::prelude::*;
use gui::{ScenarioError, Signal, div, text};
use shadcn::progress;

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page, section};

pub fn render() -> Render {
    // The demo's `onMounted` timer: 13 % now, 66 % half a second later.
    let value = Signal::labeled("progress", 13.0_f32);
    let later = value;
    gui::clock::after(Duration::from_millis(500), move || later.set(66.0));
    Box::new(move || {
        page(
            "Progress",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "ProgressDemo.vue",
                    progress(value.get()).width(Length::Fraction(0.6)),
                ))
                .child(section(
                    "Values",
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(16.0)
                        .width(480.0)
                        .children([0.0, 25.0, 66.0, 100.0].map(|v| {
                            div()
                                .display(Display::Flex)
                                .align_items(Align::Center)
                                .gap(16.0)
                                .child(text(format!("{v}%")).text_size(14.0).line_height(20.0).width(48.0))
                                .child(progress(v))
                        })),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default, 13%")?;
    // The demo's timer moves it to 66% at 500ms; the bar slides there.
    motion_frames(s, "to 66%", &MOTION_MS, |s| {
        s.at(0.5);
        Ok(())
    })
}
