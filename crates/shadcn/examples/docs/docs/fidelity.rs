//! How close each demo comes to shadcn-vue's own, as the last verdict run
//! (`crates/shadcn/verdict/run.sh`, light and `--dark`) scored it: a badge
//! per demo on its component's page, and the Fidelity page listing them all.

use std::path::Path;

use gui::prelude::*;
use gui::{Div, Element, Icon, Track, div, icon, key_of, text};
use shadcn::{
    BadgeVariant, ButtonSize, ButtonVariant, EmptyMediaVariant, badge, button, empty, empty_content, empty_description,
    empty_header, empty_media, empty_title, table, table_body, table_cell, table_head, table_header, table_row, theme,
};

use super::catalog::Catalog;
use super::prose::{code, h1, h2, inline, lead};
use super::{Nav, Part};
use crate::verdict::{self, Grade, Judged};

/// Where `run.sh` writes the verdict of every page in vega, the default style, light and dark.
const LIGHT: &str = "/tmp/shadcn-verdict/vega/all/verdict.json";
const DARK: &str = "/tmp/shadcn-verdict/vega/all-dark/verdict.json";

/// The last verdict runs, as found on disk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Verdicts {
    pub light: Option<Vec<Judged>>,
    pub dark: Option<Vec<Judged>>,
}

impl Verdicts {
    pub fn read() -> Self {
        Self {
            light: verdict::read_json(Path::new(LIGHT)),
            dark: verdict::read_json(Path::new(DARK)),
        }
    }

    fn find<'a>(run: Option<&'a Vec<Judged>>, page: &str, demo: &str) -> Option<&'a Judged> {
        run?.iter().find(|j| j.page == page && j.demo == demo)
    }

    /// `demo` of `page` in the light and the dark run.
    pub fn of(&self, page: &str, demo: &str) -> (Option<&Judged>, Option<&Judged>) {
        (
            Self::find(self.light.as_ref(), page, demo),
            Self::find(self.dark.as_ref(), page, demo),
        )
    }
}

fn grade_look(grade: Grade) -> (BadgeVariant, Icon) {
    match grade {
        Grade::Pass => (BadgeVariant::Secondary, Icon::CircleCheck),
        Grade::Better => (BadgeVariant::Secondary, Icon::Star),
        Grade::Warn => (BadgeVariant::Outline, Icon::TriangleAlert),
        Grade::Fail => (BadgeVariant::Destructive, Icon::CircleX),
    }
}

/// `PASS 99.7`, its look following the grade; `mode` (Light, Dark) leads it when given.
pub fn grade_badge(mode: Option<&str>, j: &Judged) -> Div {
    let (variant, glyph) = grade_look(j.grade);
    let label = match mode {
        Some(m) => format!("{m} {} {:.0}", j.grade, j.score),
        None => format!("{} {:.1}", j.grade, j.score),
    };
    badge(variant).child(icon(glyph)).child(label)
}

fn mean(run: &[Judged]) -> f64 {
    let n = u32::try_from(run.len()).unwrap_or(u32::MAX).max(1);
    run.iter().map(|j| j.score).sum::<f64>() / f64::from(n)
}

/// A figure over its label, in a bordered box.
fn stat(figure: String, label: &str) -> Div {
    let t = theme();
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(4.0)
        .flex(Flex::grow(1.0))
        .radius(10.0)
        .border(1.0)
        .border_color(t.border)
        .padding(16.0)
        .child(
            text(figure)
                .text_size(24.0)
                .line_height(32.0)
                .font_weight(600)
                .tracking(-0.025),
        )
        .child(
            text(label.to_owned())
                .text_size(14.0)
                .line_height(20.0)
                .text_color(t.muted_foreground),
        )
}

fn count(run: &[Judged], grade: Grade) -> usize {
    run.iter().filter(|j| j.grade == grade).count()
}

/// The figures of one run: its mean and how many of each grade.
fn summary(mode: &str, run: &[Judged]) -> Div {
    let grades = [Grade::Pass, Grade::Better, Grade::Warn, Grade::Fail];
    div()
        .display(Display::Flex)
        .gap(12.0)
        .child(stat(
            format!("{:.1}", mean(run)),
            &format!("{mode}: mean of {} demos", run.len()),
        ))
        .children(grades.map(|g| stat(count(run, g).to_string(), &format!("{mode}: {g}"))))
}

/// The message shown before any verdict has run.
fn none_yet() -> Div {
    empty()
        .border(1.0)
        .border_color(theme().border)
        .child(
            empty_header()
                .child(empty_media(EmptyMediaVariant::Icon).child(icon(Icon::FlaskConical).size(24.0)))
                .child(empty_title("No verdict yet"))
                .child(empty_description(
                    "Run crates/shadcn/verdict/run.sh to judge every demo against shadcn-vue's own; \
                     this page reads what it writes.",
                )),
        )
        .child(empty_content().width(pct(100.0)).child(code(
            "crates/shadcn/verdict/run.sh         # light\ncrates/shadcn/verdict/run.sh --dark  # dark",
            "bash",
        )))
}

/// Which file each run was read from, or that it has none yet.
fn read_from(runs: &[(&str, Option<&[Judged]>)]) -> String {
    runs.iter()
        .zip([LIGHT, DARK])
        .map(|((mode, run), path)| match run {
            Some(_) => format!("{mode} read from `{path}`"),
            None => format!("{mode}: no `{path}` yet"),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The Fidelity page.
pub fn parts(catalog: &Catalog, verdicts: &Verdicts, nav: &Nav, reload: impl Fn() + 'static) -> Vec<Part> {
    let t = theme();
    let intro = format!(
        "How close each demo comes to shadcn-vue's own. The verdict lays our render over theirs, cropped to the ink, and \
         scores the share that matches: `PASS` from {:.0}, `WARN` from {:.0}, `FAIL` below; `BETTER` differs on purpose \
         (`verdict/better.tsv` says why).",
        verdict::PASS,
        verdict::FAIL,
    );
    let header = div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(8.0)
        .child(h1("Fidelity"))
        .child(lead(&intro))
        .child(
            div()
                .display(Display::Flex)
                .align_items(Align::Center)
                .gap(8.0)
                .padding_top(8.0)
                .child(
                    button(ButtonVariant::Outline, ButtonSize::Sm)
                        .named("reload-verdict")
                        .on_click(move |_| reload())
                        .child(icon(Icon::RefreshCw))
                        .child("Read again"),
                ),
        );
    let mut parts = vec![Part::new("top", None, header)];
    let runs = [("Light", verdicts.light.as_deref()), ("Dark", verdicts.dark.as_deref())];
    if runs.iter().all(|(_, r)| r.is_none()) {
        parts.push(Part::new("none", None, none_yet()));
        return parts;
    }
    let summaries = runs.iter().filter_map(|(mode, run)| run.map(|r| summary(mode, r)));
    parts.push(Part::new(
        "summary",
        Some("Summary"),
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(12.0)
            .child(h2("Summary"))
            .children(summaries)
            .child(
                inline(&format!("{}.", read_from(&runs)))
                    .text_size(14.0)
                    .line_height(20.0)
                    .text_color(t.muted_foreground),
            ),
    ));
    parts.push(Part::new(
        "demos",
        Some("By demo"),
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(12.0)
            .child(h2("By demo"))
            .child(by_demo(catalog, verdicts, nav)),
    ));
    parts
}

/// One row per demo either run judged, under its component, A–Z.
fn by_demo(catalog: &Catalog, verdicts: &Verdicts, nav: &Nav) -> Element {
    let t = theme();
    let columns = [Track::Fr(1.0), Track::Fr(1.4), Track::Px(150.0), Track::Px(150.0)];
    let mut rows = Vec::new();
    for route in catalog.order() {
        let Some(entry) = catalog.entry(route) else {
            continue;
        };
        let mut demos: Vec<&str> = verdicts
            .light
            .iter()
            .chain(verdicts.dark.iter())
            .flatten()
            .filter(|j| j.page == entry.page.name)
            .map(|j| j.demo.as_str())
            .collect();
        let mut seen = std::collections::HashSet::new();
        demos.retain(|d| seen.insert(*d));
        for (i, demo) in demos.iter().enumerate() {
            let (light, dark) = verdicts.of(entry.page.name, demo);
            let cell = |j: Option<&Judged>| match j {
                Some(j) => table_cell(grade_badge(None, j).align_self(Align::Start)),
                None => table_cell(text("–").text_color(t.muted_foreground)),
            };
            let go = nav.clone();
            let component = (i == 0).then(|| {
                text(entry.title.clone())
                    .font_weight(500)
                    .cursor(Cursor::Pointer)
                    .underline_offset(4.0)
                    .with_interaction(|d, i| d.underline(move || i.hovered()))
            });
            rows.push(
                table_row(&columns, false)
                    .key(key_of(&format!("{}/{demo}", entry.page.name)))
                    .cursor(Cursor::Pointer)
                    .on_click(move |_| go.go(route))
                    .child(table_cell(component))
                    .child(table_cell(
                        text((*demo).to_owned())
                            .font_family(FontFamily::GEIST_MONO)
                            .text_size(12.0)
                            .line_height(16.0),
                    ))
                    .child(cell(light))
                    .child(cell(dark)),
            );
        }
    }
    let head = table_row(&columns, false)
        .child(table_head("Component"))
        .child(table_head("Demo"))
        .child(table_head("Light"))
        .child(table_head("Dark"));
    div()
        .radius(10.0)
        .border(1.0)
        .border_color(t.border)
        .overflow(Overflow::Hidden)
        .child(table().child(table_header([head])).child(table_body(rows)))
        .into_element()
}
