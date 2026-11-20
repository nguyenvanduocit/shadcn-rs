use gui::prelude::*;
use gui::{Color, Div, ScenarioError, Signal, Text, div, text};
use shadcn::{ButtonSize, ButtonVariant, Shimmer, ShimmerExt, button, theme};

use crate::{Render, Shot, demo, page};

const LABEL: &str = "Generating response\u{2026}";

pub fn render() -> Render {
    let replayed = Signal::labeled("replayed", 0_u64);
    Box::new(move || {
        page(
            "Shimmer",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "ShimmerDemo.vue",
                    div().flex().items_center().justify_center().h_full().w_full().child(
                        text(LABEL)
                            .text_sm()
                            .text_color(theme().muted_foreground)
                            .shimmer(Shimmer::default()),
                    ),
                ))
                .child(demo(
                    "ShimmerAngleDemo.vue",
                    pair(
                        (Shimmer::default(), "shimmer"),
                        (Shimmer::default().angle(45.0), "shimmer-angle-45"),
                    ),
                ))
                .child(demo(
                    "ShimmerColorDemo.vue",
                    muted(div().flex_col().items_center().gap(2.0))
                        .child(text(LABEL).shimmer(Shimmer::default().color(blue_500().opacity(0.6))))
                        .child(text(LABEL).shimmer(Shimmer::default().color(Color::hex(0x0037_8add)))),
                ))
                .child(demo(
                    "ShimmerDisablingDemo.vue",
                    // `md:shimmer-none`: the verdict's 1280px viewport is past `md`.
                    muted(div().flex_col().items_center().gap(3.0))
                        .child(text(LABEL))
                        .child(caption("shimmer md:shimmer-none")),
                ))
                .child(demo(
                    "ShimmerDurationDemo.vue",
                    pair(
                        (Shimmer::default(), "shimmer"),
                        (Shimmer::default().duration_ms(1000.0), "shimmer-duration-1000"),
                    ),
                ))
                .child(demo(
                    "ShimmerPlayOnceDemo.vue",
                    div()
                        .flex_col()
                        .items_center()
                        .gap(4.0)
                        .child(
                            // Vue's `:key="key"`: a new node, a new sweep.
                            text(LABEL)
                                .key(replayed.get())
                                .text_sm()
                                .text_color(theme().muted_foreground)
                                .shimmer(Shimmer::default().duration_ms(1100.0).once()),
                        )
                        .child(
                            button(ButtonVariant::Outline, ButtonSize::Sm)
                                .on_click({
                                    let replayed = replayed.clone();
                                    move |_| replayed.update(|k| k.wrapping_add(1))
                                })
                                .child("Replay"),
                        ),
                ))
                .child(demo(
                    "ShimmerRTLDemo.vue",
                    muted(
                        div()
                            .mx_auto()
                            .grid()
                            .w_full()
                            .max_w_px(512.0)
                            .gap(6.0)
                            .text_center()
                            .grid_cols(2),
                    )
                    .child(
                        div()
                            .flex_col()
                            .gap(3.0)
                            .child(text(LABEL).dir_ltr().shimmer(Shimmer::default()))
                            .child(caption("dir=\"ltr\"")),
                    )
                    .child(
                        div()
                            .flex_col()
                            .gap(3.0)
                            .child(
                                text("جارٍ إنشاء الرد\u{2026}")
                                    .dir_rtl()
                                    .shimmer(Shimmer::default().reverse()),
                            )
                            .child(caption("dir=\"rtl\"")),
                    ),
                ))
                .child(demo(
                    "ShimmerSpreadDemo.vue",
                    pair(
                        (Shimmer::default().spread(4.0), "shimmer-spread-4"),
                        (Shimmer::default().spread(24.0), "shimmer-spread-24"),
                    ),
                )),
        )
    })
}

/// Tailwind's `blue-500`.
fn blue_500() -> Color {
    Color::oklch(0.623, 0.214, 259.815)
}

/// `text-sm text-muted-foreground`, which the demos set on their root.
fn muted(d: Div) -> Div {
    d.text_sm().text_color(theme().muted_foreground)
}

/// `<p class="font-mono text-xs">`: the class a column demonstrates.
fn caption(label: &str) -> Text {
    text(label).font_mono().text_xs()
}

/// Two shimmers side by side, each over the class it shows
/// (`mx-auto grid w-full max-w-lg gap-6 text-center sm:grid-cols-2`).
fn pair(left: (Shimmer, &str), right: (Shimmer, &str)) -> Div {
    let column = |(shimmer, label): (Shimmer, &str)| {
        div()
            .flex_col()
            .gap(3.0)
            .child(text(LABEL).shimmer(shimmer))
            .child(caption(label))
    };
    muted(
        div()
            .mx_auto()
            .grid()
            .w_full()
            .max_w_px(512.0)
            .gap(6.0)
            .text_center()
            .grid_cols(2),
    )
    .child(column(left))
    .child(column(right))
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("sweeps start left of the text")?;
    s.at(1.0);
    s.frame("highlight mid-sweep")?;
    s.click_text("Replay")?;
    s.at(1.55);
    s.frame("replayed sweep mid-way")?;
    Ok(())
}
