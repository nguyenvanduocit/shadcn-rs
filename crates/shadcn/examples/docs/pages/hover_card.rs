use std::time::Duration;

use gui::prelude::*;
use gui::{Icon, ScenarioError, div, icon, text};
use shadcn::{
    ButtonSize, ButtonVariant, avatar, avatar_fallback, avatar_image, button, hover_card, hover_card_content,
    hover_card_trigger, theme,
};

use crate::{Render, Shot, demo, page, photo};

pub fn render() -> Render {
    let vercel = photo("vercel");
    Box::new(move || {
        let t = theme();
        page(
            "Hover Card",
            demo(
                "HoverCardDemo.vue",
                hover_card()
                    .child(hover_card_trigger(
                        button(ButtonVariant::Link, ButtonSize::Default).child("@nuxt"),
                    ))
                    .child(
                        hover_card_content().width(320.0).child(
                            div()
                                .display(Display::Flex)
                                .justify_content(Justify::Between)
                                .gap(16.0)
                                .child(
                                    avatar()
                                        .child(avatar_image(vercel.as_ref()))
                                        .child(avatar_fallback("VC")),
                                )
                                .child(
                                    div()
                                        .display(Display::Flex)
                                        .flex_direction(FlexDirection::Column)
                                        .gap(4.0)
                                        .child(text("@nuxt").text_size(14.0).line_height(20.0).font_weight(600))
                                        .child(
                                            text("The Vue Framework – The Progressive Web Framework.")
                                                .text_size(14.0)
                                                .line_height(20.0),
                                        )
                                        .child(
                                            div()
                                                .display(Display::Flex)
                                                .align_items(Align::Center)
                                                .padding_top(8.0)
                                                .child(icon(Icon::CalendarDays).margin_right(8.0).opacity(0.7))
                                                .child(
                                                    text("Joined December 2021")
                                                        .text_size(12.0)
                                                        .line_height(16.0)
                                                        .text_color(t.muted_foreground),
                                                ),
                                        ),
                                ),
                        ),
                    ),
            ),
        )
    })
}

/// Let a hover delay pass, running the timers it started.
fn wait(s: &mut Shot, ms: u64) {
    s.wait(Duration::from_millis(ms));
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("@nuxt")?;
    s.hover(x, y);
    wait(s, 100);
    s.frame("hovered 100ms, still closed")?;
    wait(s, 650);
    s.frame("open after 700ms")?;
    s.hover(x, y + 60.0);
    s.settle()?;
    wait(s, 400);
    s.frame("pointer on card keeps it open")?;
    s.hover(800.0, 500.0);
    wait(s, 350);
    s.frame("closed 300ms after leaving")?;
    Ok(())
}
