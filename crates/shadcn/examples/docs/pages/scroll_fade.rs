//! shadcn's `scroll-fade` utilities: a scrolling list or row whose edges fade
//! out where more content waits, on every side (`scroll-fade-t`, `-b`, `-s`,
//! `-e`) and right to left.

use gui::prelude::*;
use gui::{Div, ScenarioError, Text, div, text};
use shadcn::theme;

use crate::{Render, Shot, demo, page};

const TEAMS: [&str; 12] = [
    "Design",
    "Engineering",
    "Marketing",
    "Product",
    "Research",
    "Sales",
    "Support",
    "Operations",
    "Finance",
    "Legal",
    "People",
    "Security",
];

/// `ScrollFadeRTLDemo`'s teams, in Arabic.
const RTL_TEAMS: [&str; 10] = [
    "تصميم",
    "هندسة",
    "تسويق",
    "منتج",
    "أبحاث",
    "مبيعات",
    "دعم",
    "عمليات",
    "مالية",
    "قانوني",
];

const TASKS: [&str; 6] = [
    "Inbox triage",
    "Design review",
    "API contract",
    "QA pass",
    "Launch notes",
    "Metrics follow-up",
];

pub fn render() -> Render {
    Box::new(|| {
        page(
            "Scroll Fade",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "ScrollFadeDemo.vue",
                    frame().margin_x(Length::Auto).width(pct(100.0)).max_width(320.0).child(
                        rows(numbered(12))
                            .mask(ScrollFade {
                                axis: Axis::Vertical,
                                start: true,
                                end: true,
                                size: None,
                            })
                            .height(288.0),
                    ),
                ))
                .child(demo(
                    "ScrollFadeEdgeDemo.vue",
                    div()
                        .margin_x(Length::Auto)
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .max_width(320.0)
                        .min_width(0.0)
                        .gap(24.0)
                        .child(labelled(
                            frame().child(
                                rows(TASKS.map(String::from))
                                    .mask(ScrollFade {
                                        axis: Axis::Vertical,
                                        start: true,
                                        end: false,
                                        size: None,
                                    })
                                    .height(144.0),
                            ),
                            "scroll-fade-t",
                        ))
                        .child(labelled(
                            frame().child(
                                rows(TASKS.map(String::from))
                                    .mask(ScrollFade {
                                        axis: Axis::Vertical,
                                        start: false,
                                        end: true,
                                        size: None,
                                    })
                                    .height(144.0),
                            ),
                            "scroll-fade-b",
                        ))
                        .child(labelled(
                            frame().child(tags(TEAMS.iter().take(8), Tag::Pill).mask(ScrollFade {
                                axis: Axis::Horizontal,
                                start: true,
                                end: false,
                                size: None,
                            })),
                            "scroll-fade-s",
                        ))
                        .child(labelled(
                            frame().child(tags(TEAMS.iter().take(8), Tag::Pill).mask(ScrollFade {
                                axis: Axis::Horizontal,
                                start: false,
                                end: true,
                                size: None,
                            })),
                            "scroll-fade-e",
                        )),
                ))
                .child(demo(
                    "ScrollFadeHorizontalDemo.vue",
                    frame().margin_x(Length::Auto).width(pct(100.0)).max_width(320.0).child(
                        tags(TEAMS.iter(), Tag::Chip).mask(ScrollFade {
                            axis: Axis::Horizontal,
                            start: true,
                            end: true,
                            size: None,
                        }),
                    ),
                ))
                .child(demo(
                    "ScrollFadeRTLDemo.vue",
                    div()
                        .margin_x(Length::Auto)
                        .display(Display::Grid)
                        .width(pct(100.0))
                        .max_width(512.0)
                        .gap(24.0)
                        .grid_columns(Tracks::repeat(2, Track::Fr(1.0)))
                        .child(labelled(
                            frame()
                                .direction(Direction::Ltr)
                                .child(tags(TEAMS.iter().take(10), Tag::Chip).mask(ScrollFade {
                                    axis: Axis::Horizontal,
                                    start: true,
                                    end: true,
                                    size: None,
                                })),
                            "dir=\"ltr\"",
                        ))
                        .child(labelled(
                            frame()
                                .direction(Direction::Rtl)
                                .child(tags(RTL_TEAMS.iter(), Tag::Chip).mask(ScrollFade {
                                    axis: Axis::Horizontal,
                                    start: true,
                                    end: true,
                                    size: None,
                                })),
                            "dir=\"rtl\"",
                        )),
                ))
                .child(demo(
                    "ScrollFadeNoneDemo.vue",
                    column()
                        .child(labelled(
                            frame().child(
                                rows(numbered(8))
                                    .mask(ScrollFade {
                                        axis: Axis::Vertical,
                                        start: true,
                                        end: true,
                                        size: None,
                                    })
                                    .height(192.0),
                            ),
                            "scroll-fade",
                        ))
                        .child(labelled(
                            frame().child(
                                rows(numbered(8))
                                    .mask(ScrollFade {
                                        axis: Axis::Vertical,
                                        start: true,
                                        end: true,
                                        size: None,
                                    })
                                    .mask(None)
                                    .height(192.0),
                            ),
                            "scroll-fade scroll-fade-none",
                        )),
                ))
                .child(demo(
                    "ScrollFadeOverflowDemo.vue",
                    frame()
                        .margin_x(Length::Auto)
                        .width(pct(100.0))
                        .max_width(320.0)
                        .child(rows(numbered(3)).mask(ScrollFade {
                            axis: Axis::Vertical,
                            start: true,
                            end: true,
                            size: None,
                        })),
                ))
                .child(demo(
                    "ScrollFadeSizeDemo.vue",
                    column()
                        .child(labelled(
                            frame().child(
                                rows(numbered(8))
                                    .mask(ScrollFade {
                                        axis: Axis::Vertical,
                                        start: true,
                                        end: true,
                                        size: Some(16.0),
                                    })
                                    .height(192.0),
                            ),
                            "scroll-fade-4",
                        ))
                        .child(labelled(
                            frame().child(
                                rows(numbered(8))
                                    .mask(ScrollFade {
                                        axis: Axis::Vertical,
                                        start: true,
                                        end: true,
                                        size: Some(96.0),
                                    })
                                    .height(192.0),
                            ),
                            "scroll-fade-24",
                        )),
                )),
        )
    })
}

/// `Item 1` … `Item n`.
fn numbered(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("Item {i}")).collect()
}

/// `overflow-hidden rounded-2xl border`: the card a list scrolls in.
fn frame() -> Div {
    div()
        .overflow(Overflow::Hidden)
        .radius(18.0)
        .border(1.0)
        .border_color(theme().border)
}

/// `mx-auto flex w-full max-w-xs min-w-0 flex-col gap-6`.
fn column() -> Div {
    div()
        .margin_x(Length::Auto)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .max_width(320.0)
        .min_width(0.0)
        .gap(24.0)
}

/// A scrolling list of `rounded-lg bg-muted px-3 py-2.5 text-sm` rows
/// (`scrollbar-none overflow-y-auto` around `flex flex-col gap-1.5 p-1.5`).
fn rows(labels: impl IntoIterator<Item = String>) -> Div {
    let t = theme();
    div()
        .scrollbar_hidden(true)
        .overflow_y(Overflow::Scroll)
        .overflow_x(Overflow::Hidden)
        .child(
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(6.0)
                .padding(6.0)
                .children(labels.into_iter().map(|label| {
                    div()
                        .radius(10.0)
                        .background(t.muted)
                        .padding_x(12.0)
                        .padding_y(10.0)
                        .text_size(14.0)
                        .line_height(20.0)
                        .child(label)
                })),
        )
}

/// How a demo draws its tags.
#[derive(Clone, Copy)]
enum Tag {
    /// `rounded-xl px-4`.
    Pill,
    /// `rounded-lg px-3`.
    Chip,
}

/// A sideways-scrolling row of `shrink-0 bg-muted py-2.5 text-sm` tags
/// (`scrollbar-none overflow-x-auto` around `flex w-max gap-1.5 p-1.5`).
fn tags<'a>(labels: impl IntoIterator<Item = &'a &'a str>, look: Tag) -> Div {
    let t = theme();
    div()
        .scrollbar_hidden(true)
        .overflow_x(Overflow::Scroll)
        .overflow_y(Overflow::Hidden)
        .child(
            // `w-max` in block flow: an inline flex row of tags that do not
            // shrink is exactly as wide as they are.
            div()
                .display(Display::Flex)
                .inline(true)
                .gap(6.0)
                .padding(6.0)
                .children(labels.into_iter().map(|&label| {
                    let tag = div()
                        .flex_shrink(0.0)
                        .background(t.muted)
                        .padding_y(10.0)
                        .text_size(14.0)
                        .line_height(20.0)
                        .child(label);
                    match look {
                        Tag::Pill => tag.radius(14.0).padding_x(16.0),
                        Tag::Chip => tag.radius(10.0).padding_x(12.0),
                    }
                })),
        )
}

/// `content` over the utility it shows (`flex flex-col gap-3`).
fn labelled(content: Div, label: &str) -> Div {
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(12.0)
        .child(content)
        .child(caption(label))
}

/// `<p class="text-center font-mono text-xs text-muted-foreground">`.
fn caption(label: &str) -> Text {
    text(label)
        .text_align(TextAlign::Center)
        .font_family(FontFamily::GEIST_MONO)
        .text_size(12.0)
        .line_height(16.0)
        .text_color(theme().muted_foreground)
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("at the top: only the bottom edges fade")?;
    let (x, y) = s.locate("Item 3")?;
    s.hover(x, y);
    s.wheel(120.0);
    s.frame("scrolled into the list: both edges fade")?;
    s.wheel(2000.0);
    s.frame("at the bottom: only the top edge fades")?;
    Ok(())
}
