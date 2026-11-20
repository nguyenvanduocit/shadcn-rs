use std::time::Duration;

use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, Side, SideAlign, div, text};
use shadcn::{
    ButtonSize, ButtonVariant, TooltipState, button, tooltip, tooltip_content, tooltip_content_placed, tooltip_trigger,
};

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page, section};

const SIDES: [(&str, Side); 4] = [
    ("Left", Side::Left),
    ("Top", Side::Top),
    ("Bottom", Side::Bottom),
    ("Right", Side::Right),
];

pub fn render() -> Render {
    let first = TooltipState::new();
    let sides: Vec<TooltipState> = SIDES.iter().map(|_| TooltipState::new()).collect();
    let delayed = TooltipState::with_delay(Duration::from_millis(700));
    Box::new(move || {
        let hover_button = |state: &TooltipState, label: &str| {
            tooltip_trigger(
                state,
                button(ButtonVariant::Outline, ButtonSize::Default).child(label.to_owned()),
            )
        };
        page(
            "Tooltip",
            div()
                .flex_col()
                .gap(10.0)
                .child(demo(
                    "TooltipDemo.vue",
                    tooltip()
                        .child(hover_button(&first, "Hover"))
                        .child(tooltip_content(&first, |c| c.child(text("Add to library")))),
                ))
                .child(section(
                    "Sides",
                    div()
                        .flex()
                        .gap(2.0)
                        .pl(28.0)
                        .children(SIDES.iter().zip(&sides).map(|((name, side), state)| {
                            tooltip().child(hover_button(state, name)).child(tooltip_content_placed(
                                state,
                                *side,
                                SideAlign::Center,
                                |c| c.child(text("Add to library")),
                            ))
                        })),
                ))
                .child(section(
                    "700ms delay",
                    div().flex().child(
                        tooltip()
                            .child(hover_button(&delayed, "Wait for it"))
                            .child(tooltip_content(&delayed, |c| c.child(text("Took its time")))),
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
    Ok(())
}
