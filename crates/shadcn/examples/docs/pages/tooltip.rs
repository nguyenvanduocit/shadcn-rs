use std::time::Duration;

use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, Side, SideAlign, div, text};
use shadcn::{ButtonSize, ButtonVariant, button, tooltip, tooltip_content, tooltip_trigger};

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page, section};

const SIDES: [(&str, Side); 4] = [
    ("Left", Side::Left),
    ("Top", Side::Top),
    ("Bottom", Side::Bottom),
    ("Right", Side::Right),
];

pub fn render() -> Render {
    Box::new(move || {
        let hover_button =
            |label: &str| tooltip_trigger(button(ButtonVariant::Outline, ButtonSize::Default).child(label.to_owned()));
        page(
            "Tooltip",
            div()
                .flex_col()
                .gap(10.0)
                .child(demo(
                    "TooltipDemo.vue",
                    tooltip()
                        .child(hover_button("Hover"))
                        .child(tooltip_content().child(text("Add to library"))),
                ))
                .child(section(
                    "Sides",
                    div()
                        .flex()
                        .gap(2.0)
                        .pl(28.0)
                        .children(SIDES.iter().map(|(name, side)| {
                            tooltip().child(hover_button(name)).child(
                                tooltip_content()
                                    .side(*side)
                                    .align(SideAlign::Center)
                                    .child(text("Add to library")),
                            )
                        })),
                ))
                .child(section(
                    "700ms delay",
                    div().flex().child(
                        tooltip()
                            .delay_duration(Duration::from_millis(700))
                            .child(hover_button("Wait for it"))
                            .child(tooltip_content().child(text("Took its time"))),
                    ),
                )),
        )
    })
}

/// Let a delay pass, running the timers it started.
fn wait(s: &mut Shot, ms: u64) {
    s.wait(Duration::from_millis(ms));
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    motion_frames(s, "opening", &MOTION_MS, |s| {
        let (x, y) = s.locate("Hover")?;
        s.hover(x, y);
        Ok(())
    })?;
    motion_frames(s, "closing", &MOTION_MS, |s| {
        s.press(NamedKey::Escape, Modifiers::default());
        Ok(())
    })?;
    let (x, y) = s.locate("Hover")?;
    s.hover(x, y);
    s.frame("hover shows it at once")?;
    s.press(NamedKey::Escape, Modifiers::default());
    s.frame("escape hides it")?;
    for (name, _) in SIDES {
        let (x, y) = s.locate(name)?;
        s.hover(x, y);
        s.frame(&format!("{name} side"))?;
    }
    s.hover(800.0, 600.0);
    s.frame("pointer left")?;
    let (x, y) = s.locate("Wait for it")?;
    s.hover(x, y);
    wait(s, 200);
    s.frame("delayed, 200ms: hidden")?;
    wait(s, 550);
    s.frame("delayed, 750ms: shown")?;
    s.click(x, y);
    s.frame("press hides it")?;
    s.hover(800.0, 600.0);
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab focus shows it")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab on: the next one's")?;
    Ok(())
}
