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
                        hover_card_content().w(80.0).child(
                            div()
                                .flex()
                                .justify_between()
                                .gap(4.0)
                                .child(
                                    avatar()
                                        .child(avatar_image(vercel.as_ref()))
                                        .child(avatar_fallback(vercel.as_ref(), "VC")),
                                )
                                .child(
                                    div()
                                        .flex_col()
                                        .gap(1.0)
                                        .child(text("@nuxt").text_sm().font_semibold())
                                        .child(text("The Vue Framework – The Progressive Web Framework.").text_sm())
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .pt(2.0)
                                                .child(icon(Icon::CalendarDays).mr(2.0).opacity(0.7))
                                                .child(
                                                    text("Joined December 2021")
                                                        .text_xs()
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
