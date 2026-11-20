use std::time::Duration;

use gui::prelude::*;
use gui::{ScenarioError, Signal, div, relative, text};
use shadcn::progress;

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page, section};

pub fn render() -> Render {
    // The demo's `onMounted` timer: 13 % now, 66 % half a second later.
    let value = Signal::labeled("progress", 13.0_f32);
    let later = value.clone();
    gui::clock::after(Duration::from_millis(500), move || later.set(66.0));
    Box::new(move || {
        page(
            "Progress",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo("ProgressDemo.vue", progress(value.get()).w_len(relative(0.6))))
                .child(section(
                    "Values",
                    div()
                        .flex_col()
                        .gap(4.0)
                        .w_px(480.0)
                        .children([0.0, 25.0, 66.0, 100.0].map(|v| {
                            div()
                                .flex()
                                .items_center()
                                .gap(4.0)
                                .child(text(format!("{v}%")).text_sm().w(12.0))
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
