use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, TextField, Track, div, text};
use shadcn::{ButtonSize, ButtonVariant, button, input, label, popover, popover_content, popover_trigger, theme};

use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page};

pub fn render() -> Render {
    let fields = [
        ("Width", TextField::new("100%")),
        ("Max. width", TextField::new("300px")),
        ("Height", TextField::new("25px")),
        ("Max. height", TextField::new("none")),
    ];
    Box::new(move || {
        let t = theme();
        page(
            "Popover",
            demo(
                "PopoverDemo.vue",
                popover()
                    .child(popover_trigger(
                        button(ButtonVariant::Outline, ButtonSize::Default).child("Open popover"),
                    ))
                    .child(
                        popover_content().w(80.0).child(
                            div()
                                .grid()
                                .gap(4.0)
                                .child(
                                    div()
                                        .flex_col()
                                        .gap(2.0)
                                        .child(text("Dimensions").font_medium().leading_none())
                                        .child(
                                            text("Set the dimensions for the layer.")
                                                .text_sm()
                                                .text_color(t.muted_foreground),
                                        ),
                                )
                                .child(div().grid().gap(2.0).children(fields.iter().map(|(name, f)| {
                                    div()
                                        .grid()
                                        .grid_cols_tracks(vec![Track::Fr(1.0); 3])
                                        .items_center()
                                        .gap(4.0)
                                        .child(label(*name))
                                        .child(input().field(*f).col_span(2).h(8.0))
                                }))),
                        ),
                    ),
            ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    motion_frames(s, "opening", &MOTION_MS, |s| s.click_text("Open popover"))?;
    motion_frames(s, "closing", &MOTION_MS, |s| {
        s.press(NamedKey::Escape, Modifiers::default());
        Ok(())
    })?;
    s.click_text("Open popover")?;
    s.frame("open")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab into content")?;
    s.press(NamedKey::Escape, Modifiers::default());
    s.frame("escape closed")?;
    s.click_text("Open popover")?;
    s.settle()?;
    s.click(800.0, 500.0);
    s.frame("outside press closed")?;
    s.click_text("Open popover")?;
    s.settle()?;
    s.click_text("Open popover")?;
    s.frame("trigger press closed")?;
    Ok(())
}
