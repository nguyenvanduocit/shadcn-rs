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
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "ScrollFadeDemo.vue",
                    frame()
                        .mx_auto()
                        .w_full()
                        .max_w_px(320.0)
                        .child(rows(numbered(12)).scroll_fade().h(72.0)),
                ))
                .child(demo(
                    "ScrollFadeEdgeDemo.vue",
                    div()
                        .mx_auto()
                        .flex_col()
                        .max_w_px(320.0)
                        .min_w_px(0.0)
                        .gap(6.0)
                        .child(labelled(
                            frame().child(rows(TASKS.map(String::from)).scroll_fade_t().h(36.0)),
                            "scroll-fade-t",
                        ))
                        .child(labelled(
                            frame().child(rows(TASKS.map(String::from)).scroll_fade_b().h(36.0)),
                            "scroll-fade-b",
                        ))
                        .child(labelled(
                            frame().child(tags(TEAMS.iter().take(8), Tag::Pill).scroll_fade_s()),
                            "scroll-fade-s",
                        ))
                        .child(labelled(
                            frame().child(tags(TEAMS.iter().take(8), Tag::Pill).scroll_fade_e()),
                            "scroll-fade-e",
                        )),
                ))
                .child(demo(
                    "ScrollFadeHorizontalDemo.vue",
                    frame()
                        .mx_auto()
                        .w_full()
                        .max_w_px(320.0)
                        .child(tags(TEAMS.iter(), Tag::Chip).scroll_fade_x()),
                ))
                .child(demo(
                    "ScrollFadeRTLDemo.vue",
                    div()
                        .mx_auto()
                        .grid()
                        .w_full()
                        .max_w_px(512.0)
                        .gap(6.0)
                        .grid_cols(2)
                        .child(labelled(
                            frame()
                                .dir_ltr()
                                .child(tags(TEAMS.iter().take(10), Tag::Chip).scroll_fade_x()),
                            "dir=\"ltr\"",
                        ))
                        .child(labelled(
                            frame()
                                .dir_rtl()
                                .child(tags(RTL_TEAMS.iter(), Tag::Chip).scroll_fade_x()),
                            "dir=\"rtl\"",
                        )),
                ))
                .child(demo(
                    "ScrollFadeNoneDemo.vue",
                    column()
                        .child(labelled(
                            frame().child(rows(numbered(8)).scroll_fade().h(48.0)),
                            "scroll-fade",
                        ))
                        .child(labelled(
                            frame().child(rows(numbered(8)).scroll_fade().scroll_fade_none().h(48.0)),
                            "scroll-fade scroll-fade-none",
                        )),
                ))
                .child(demo(
                    "ScrollFadeOverflowDemo.vue",
                    frame()
                        .mx_auto()
                        .w_full()
                        .max_w_px(320.0)
                        .child(rows(numbered(3)).scroll_fade()),
                ))
                .child(demo(
                    "ScrollFadeSizeDemo.vue",
                    column()
                        .child(labelled(
                            frame().child(rows(numbered(8)).scroll_fade().scroll_fade_size(4.0).h(48.0)),
                            "scroll-fade-4",
                        ))
                        .child(labelled(
                            frame().child(rows(numbered(8)).scroll_fade().scroll_fade_size(24.0).h(48.0)),
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
        .overflow_hidden()
        .rounded_2xl()
        .border()
        .border_color(theme().border)
}

/// `mx-auto flex w-full max-w-xs min-w-0 flex-col gap-6`.
fn column() -> Div {
    div()
        .mx_auto()
        .flex_col()
        .w_full()
        .max_w_px(320.0)
        .min_w_px(0.0)
        .gap(6.0)
}

/// A scrolling list of `rounded-lg bg-muted px-3 py-2.5 text-sm` rows
/// (`scrollbar-none overflow-y-auto` around `flex flex-col gap-1.5 p-1.5`).
fn rows(labels: impl IntoIterator<Item = String>) -> Div {
    let t = theme();
    div().scrollbar_none().overflow_y_scroll().child(
        div().flex_col().gap(1.5).p(1.5).children(
            labels
                .into_iter()
                .map(|label| div().rounded_lg().bg(t.muted).px(3.0).py(2.5).text_sm().child(label)),
        ),
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
    div().scrollbar_none().overflow_x_scroll().child(
        // `w-max` in block flow: an inline flex row of tags that do not
        // shrink is exactly as wide as they are.
        div()
            .inline_flex()
            .gap(1.5)
            .p(1.5)
            .children(labels.into_iter().map(|&label| {
                let tag = div().shrink_0().bg(t.muted).py(2.5).text_sm().child(label);
                match look {
                    Tag::Pill => tag.rounded_xl().px(4.0),
                    Tag::Chip => tag.rounded_lg().px(3.0),
                }
            })),
    )
}

/// `content` over the utility it shows (`flex flex-col gap-3`).
fn labelled(content: Div, label: &str) -> Div {
    div().flex_col().gap(3.0).child(content).child(caption(label))
}

/// `<p class="text-center font-mono text-xs text-muted-foreground">`.
fn caption(label: &str) -> Text {
    text(label)
        .text_center()
        .font_mono()
        .text_xs()
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
