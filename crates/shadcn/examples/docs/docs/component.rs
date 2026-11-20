//! A component's docs page, as ui.shadcn.com lays one out: its title and what
//! its module doc says, then each shadcn-vue demo it reproduces in a
//! Preview | Code block (the live demo, and the Rust that renders it), then
//! the rest of its gallery page.

use std::collections::HashMap;

use gui::prelude::*;
use gui::{Div, Element, Icon, div, icon, text};
use shadcn::{
    BadgeVariant, CodeBlockContentProps, CodeBlockCopy, CodeBlockProps, TabsState, badge, code_block,
    code_block_actions, code_block_content, code_block_copy_button, code_block_header, code_block_title,
    highlighted_lines, tabs, tabs_content, tabs_list, tabs_trigger, theme,
};

use super::Part;
use super::catalog::Entry;
use super::fidelity::{Verdicts, grade_badge};
use super::lift::Lifted;
use super::prose::{h1, h2, h3, inline, lead};
use super::source::{self, DemoCode};

/// The name of the box holding the rest of the gallery page.
pub const PLAYGROUND: &str = "docs-playground";

/// What a demo's block keeps between frames.
pub struct DemoState {
    tabs: TabsState,
    copy: CodeBlockCopy,
    /// The code its Code tab shows, cut from the page's source once.
    code: Option<DemoCode>,
}

impl DemoState {
    fn new(entry: &Entry, file: &str) -> Self {
        Self {
            tabs: TabsState::new("preview"),
            copy: CodeBlockCopy::new(),
            code: source::demo_code(&entry.source, file),
        }
    }
}

/// Demo states by page and demo file.
pub type DemoStates = HashMap<(&'static str, &'static str), DemoState>;

/// The doc's first paragraph split as the module docs open, `Badge
/// (`ui/badge`): a small label…`: the registry item, and the sentence after
/// it, capitalised; the other paragraphs follow as notes.
pub(super) struct Doc {
    pub(super) registry: Option<String>,
    pub(super) lead: String,
    pub(super) notes: Vec<String>,
}

pub(super) fn parse_doc(doc: &str) -> Doc {
    let mut paragraphs = doc
        .split("\n\n")
        .map(|p| p.lines().map(str::trim).collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty());
    let first = paragraphs.next().unwrap_or_default();
    let registry = first
        .split_once("(`")
        .and_then(|(_, rest)| rest.split_once("`):"))
        .filter(|(item, _)| item.starts_with("ui/"));
    let (registry, lead) = match registry {
        Some((item, rest)) => {
            let rest = rest.trim();
            let mut c = rest.chars();
            let lead = c
                .next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default();
            (Some(item.to_owned()), lead)
        }
        None => (None, first),
    };
    Doc {
        registry,
        lead,
        notes: paragraphs.collect(),
    }
}

/// `ButtonWithIcon.vue` on the Button page → `With Icon`; `ButtonDemo.vue` → `Demo`.
pub fn demo_title(page_title: &str, file: &str) -> String {
    let stem = file.trim_end_matches(".vue");
    let prefix: String = page_title.split_whitespace().collect();
    let rest = stem
        .strip_prefix(prefix.as_str())
        .filter(|r| !r.is_empty())
        .unwrap_or(stem);
    let rest = rest.strip_suffix("Demo").filter(|r| !r.is_empty()).unwrap_or(rest);
    let mut words = String::new();
    for (i, c) in rest.char_indices() {
        let prev = rest.get(..i).and_then(|p| p.chars().last());
        if c.is_uppercase() && prev.is_some_and(char::is_lowercase) {
            words.push(' ');
        }
        words.push(c);
    }
    words
}

fn header(entry: &Entry) -> Div {
    let t = theme();
    let doc = parse_doc(&entry.doc);
    let tags = div()
        .flex()
        .flex_wrap()
        .gap(2.0)
        .when_some(doc.registry, |d, item| {
            d.child(
                badge(BadgeVariant::Secondary)
                    .child(icon(Icon::Layers))
                    .child(format!("shadcn-vue {item}")),
            )
        })
        .child(
            badge(BadgeVariant::Outline)
                .child(icon(Icon::FileCode))
                .child(entry.path.clone()),
        );
    div()
        .flex_col()
        .gap(3.0)
        .child(h1(&entry.title))
        .when(!doc.lead.is_empty(), |d| d.child(lead(&doc.lead)))
        .child(tags)
        .children(
            doc.notes
                .iter()
                .map(|n| inline(n).text_sm().leading_relaxed().text_color(t.muted_foreground)),
        )
}

/// The demo's preview: `flex min-h-[350px] items-center justify-center p-10`
/// in a bordered box, at the height a demo of a few controls reads well at.
fn preview(demo: Element) -> Div {
    let t = theme();
    div()
        .named("docs-preview")
        .w_full()
        .min_h_px(240.0)
        .flex()
        .items_center()
        .justify_center()
        .rounded_lg()
        .border()
        .border_color(t.border)
        .p(6.0)
        .child(demo)
}

fn code_view(entry: &Entry, file: &str, state: &DemoState) -> Div {
    let t = theme();
    let Some(found) = &state.code else {
        return div().child(
            inline(&format!(
                "`{file}` reaches `demo()` from outside `{}`, so there is no call here to show.",
                entry.path
            ))
            .text_sm()
            .text_color(t.muted_foreground),
        );
    };
    let p = CodeBlockProps {
        header: true,
        ..CodeBlockProps::default()
    };
    let block = code_block(p)
        .child(
            code_block_header(p)
                .child(code_block_title(p, Some(Icon::FileCode), entry.path.clone()))
                .child(code_block_actions().child(code_block_copy_button(&state.copy, found.code.clone()))),
        )
        .child(
            code_block_content(
                p,
                CodeBlockContentProps::default(),
                highlighted_lines(&found.code, "rust"),
            )
            .max_h_px(520.0),
        );
    div()
        .flex_col()
        .gap(2.0)
        .when(!found.exact, |d| {
            d.child(
                inline(&format!(
                    "`{file}` is named in a list; this is the call that maps it through `demo()`."
                ))
                .text_sm()
                .text_color(t.muted_foreground),
            )
        })
        .child(block)
}

/// A demo's block: its heading, file and verdict, then Preview | Code.
fn demo_block(entry: &Entry, file: &'static str, demo: Element, state: &DemoState, verdicts: &Verdicts) -> Div {
    let t = theme();
    let (light, dark) = verdicts.of(entry.page.name, file);
    let heading = div()
        .flex()
        .items_center()
        .gap(3.0)
        .child(h3(&demo_title(&entry.title, file)))
        .child(text(file).font_mono().text_xs().text_color(t.muted_foreground))
        .child(
            div()
                .ml_auto()
                .flex()
                .gap(1.5)
                .when_some(light, |d, j| d.child(grade_badge(Some("Light"), j)))
                .when_some(dark, |d, j| d.child(grade_badge(Some("Dark"), j))),
        );
    let switch = tabs_list()
        .child(
            tabs_trigger(&state.tabs, "preview")
                .child(icon(Icon::Eye))
                .child("Preview"),
        )
        .child(tabs_trigger(&state.tabs, "code").child(icon(Icon::Code2)).child("Code"));
    let shown = tabs()
        .gap(3.0)
        .child(switch)
        .child(tabs_content(&state.tabs, "preview").map(|c| c.child(preview(demo))))
        .child(tabs_content(&state.tabs, "code").map(|c| c.child(code_view(entry, file, state))));
    div().flex_col().gap(4.0).child(heading).child(shown)
}

/// The page of `entry`, from its lifted gallery page.
pub fn parts(entry: &Entry, lifted: Lifted, states: &mut DemoStates, verdicts: &Verdicts) -> Vec<Part> {
    let t = theme();
    let mut parts = vec![Part::new("top", None, header(entry))];
    for (file, demo) in lifted.demos {
        let state = states
            .entry((entry.page.name, file))
            .or_insert_with(|| DemoState::new(entry, file));
        parts.push(Part::keyed(
            gui::key_of(file),
            Some(demo_title(&entry.title, file)),
            demo_block(entry, file, demo, state, verdicts),
        ));
    }
    if let Some(rest) = lifted.rest {
        parts.push(Part::new(
            "playground",
            Some("Playground"),
            div()
                .flex_col()
                .gap(4.0)
                .child(h2("Playground"))
                .child(
                    text("The gallery page's own examples, beyond shadcn-vue's demos.")
                        .text_sm()
                        .text_color(t.muted_foreground),
                )
                // The gallery lays its page out at its own widths: wider than
                // the column, it scrolls sideways rather than run under "On this page".
                .child(div().named(PLAYGROUND).w_full().overflow_x_scroll().child(rest)),
        ));
    }
    parts
}
