//! The docs' type: headings, the lead under a title, paragraphs whose
//! `code` reads as code, and Markdown (the crate's README) in the same look,
//! after ui.shadcn.com's docs.

use gui::prelude::*;
use gui::{Div, Element, Text, Track, div, span, text};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use shadcn::{
    CodeBlockContentProps, CodeBlockProps, code_block, code_block_content, highlighted_lines, table, table_body,
    table_head, table_header, table_row, theme,
};

/// A page's title: `scroll-m-20 text-3xl font-semibold tracking-tight`.
pub fn h1(title: &str) -> Text {
    text(title)
        .text_size(30.0)
        .line_height(36.0)
        .font_weight(600)
        .tracking(-0.025)
}

/// A section's heading: `text-xl font-semibold tracking-tight`.
pub fn h2(title: &str) -> Text {
    text(title)
        .text_size(20.0)
        .line_height(28.0)
        .font_weight(600)
        .tracking(-0.025)
}

/// A demo's heading: `text-lg font-medium tracking-tight`.
pub fn h3(title: &str) -> Text {
    text(title)
        .text_size(18.0)
        .line_height(28.0)
        .font_weight(500)
        .tracking(-0.025)
}

/// The sentence under a title: `text-muted-foreground text-[1.05rem] text-balance`.
pub fn lead(s: &str) -> Text {
    let t = theme();
    inline(s)
        .text_size(16.8)
        .line_height(27.0)
        .text_color(t.muted_foreground)
        .line_height(LineHeight::Relative(1.625))
}

/// A paragraph: `leading-7`.
pub fn paragraph(s: &str) -> Text {
    inline(s).line_height(LineHeight::Relative(1.625))
}

/// `s` with each `` `code` `` as inline code; a rustdoc link (`` [`Name`] ``)
/// reads as its code.
pub fn inline(s: &str) -> Text {
    let s = s.replace("[`", "`").replace("`]", "`");
    s.split('`').enumerate().fold(text(""), |p, (i, part)| {
        if i.is_multiple_of(2) {
            p.span(part)
        } else {
            p.span(code_span(part))
        }
    })
}

/// Inline code: `bg-muted relative rounded-md px-[0.3rem] py-[0.2rem] font-mono text-[0.8rem]`.
fn code_span(s: &str) -> gui::Span {
    let t = theme();
    span(s.to_owned())
        .font_family(FontFamily::GEIST_MONO)
        .text_size(12.8)
        .line_height(20.0)
        .background(t.muted)
        .radius(8.0)
        .padding_x(4.8)
}

/// A block of code in `language`, as the docs show one.
pub fn code(source: &str, language: &str) -> Div {
    let p = CodeBlockProps::default();
    code_block(p).child(code_block_content(
        p,
        CodeBlockContentProps::default(),
        highlighted_lines(source, language),
    ))
}

/// One run of inline Markdown.
#[derive(Clone, Default)]
struct Run {
    text: String,
    code: bool,
    strong: bool,
    em: bool,
}

/// A Markdown block, ready to draw.
enum Block {
    Heading(HeadingLevel, Vec<Run>),
    Paragraph(Vec<Run>),
    /// A list item: its depth (1 at the top) and its runs.
    Item(usize, Vec<Run>),
    Code(String, String),
    /// Rows of cells; the first row is the head.
    Table(Vec<Vec<Vec<Run>>>),
}

fn runs_text(runs: &[Run]) -> Text {
    runs.iter().fold(text(""), |p, r| {
        if r.code {
            p.span(code_span(&r.text))
        } else {
            let s = span(r.text.clone());
            let s = if r.strong { s.font_weight(600) } else { s };
            p.span(if r.em { s.italic(true) } else { s })
        }
    })
}

/// Read `md` into blocks.
fn blocks(md: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut runs: Vec<Run> = Vec::new();
    let mut style = Run::default();
    let mut depth = 0_usize;
    let mut code: Option<(String, String)> = None;
    let mut table: Option<Vec<Vec<Vec<Run>>>> = None;
    let flush_item = |runs: &mut Vec<Run>, out: &mut Vec<Block>, depth: usize| {
        if !runs.is_empty() {
            out.push(Block::Item(depth, std::mem::take(runs)));
        }
    };
    for event in Parser::new_ext(md, Options::ENABLE_TABLES) {
        match event {
            Event::Start(Tag::List(_)) => {
                flush_item(&mut runs, &mut out, depth);
                depth = depth.saturating_add(1);
            }
            Event::End(TagEnd::List(_)) => {
                flush_item(&mut runs, &mut out, depth);
                depth = depth.saturating_sub(1);
            }
            Event::End(TagEnd::Item) => flush_item(&mut runs, &mut out, depth),
            Event::End(TagEnd::Paragraph) if depth == 0 && table.is_none() => {
                out.push(Block::Paragraph(std::mem::take(&mut runs)));
            }
            Event::End(TagEnd::Heading(level)) => out.push(Block::Heading(level, std::mem::take(&mut runs))),
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(l) => l.to_string(),
                    pulldown_cmark::CodeBlockKind::Indented => String::new(),
                };
                code = Some((lang, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((lang, body)) = code.take() {
                    out.push(Block::Code(lang, body));
                }
            }
            Event::Start(Tag::Table(_)) => table = Some(Vec::new()),
            Event::End(TagEnd::Table) => {
                if let Some(rows) = table.take() {
                    out.push(Block::Table(rows));
                }
            }
            Event::Start(Tag::TableHead | Tag::TableRow) => {
                if let Some(rows) = table.as_mut() {
                    rows.push(Vec::new());
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(row) = table.as_mut().and_then(|rows| rows.last_mut()) {
                    row.push(std::mem::take(&mut runs));
                }
            }
            Event::Start(Tag::Strong) => style.strong = true,
            Event::End(TagEnd::Strong) => style.strong = false,
            Event::Start(Tag::Emphasis) => style.em = true,
            Event::End(TagEnd::Emphasis) => style.em = false,
            Event::Text(s) => match code.as_mut() {
                Some((_, body)) => body.push_str(&s),
                None => runs.push(Run {
                    text: s.to_string(),
                    ..style.clone()
                }),
            },
            Event::Code(s) => runs.push(Run {
                text: s.to_string(),
                code: true,
                ..Run::default()
            }),
            Event::SoftBreak | Event::HardBreak => runs.push(Run {
                text: " ".to_owned(),
                ..style.clone()
            }),
            _ => {}
        }
    }
    out
}

/// `md` as the docs' prose; each `##` heading starts a new part, so a page
/// can file each under "On this page". The first part is what comes before
/// any `##`, its first paragraph read as the lead; a part is its heading (if
/// any) and its body.
pub fn markdown(md: &str) -> Vec<(Option<String>, Element)> {
    let t = theme();
    let mut parts: Vec<(Option<String>, Vec<Element>)> = vec![(None, Vec::new())];
    for block in blocks(md) {
        let el = match block {
            Block::Heading(HeadingLevel::H1, _) => continue,
            Block::Heading(HeadingLevel::H2, runs) => {
                let title: String = runs.iter().map(|r| r.text.as_str()).collect();
                parts.push((Some(title.clone()), vec![h2(&title).into_element()]));
                continue;
            }
            Block::Heading(_, runs) => runs_text(&runs)
                .text_size(18.0)
                .line_height(28.0)
                .font_weight(600)
                .margin_top(8.0)
                .into_element(),
            Block::Paragraph(runs) if parts.len() == 1 && parts.first().is_some_and(|(_, e)| e.is_empty()) => {
                runs_text(&runs)
                    .text_size(16.8)
                    .line_height(27.0)
                    .text_color(t.muted_foreground)
                    .line_height(LineHeight::Relative(1.625))
                    .into_element()
            }
            Block::Paragraph(runs) => runs_text(&runs).line_height(LineHeight::Relative(1.625)).into_element(),
            Block::Item(depth, runs) => div()
                .display(Display::Flex)
                .gap(8.0)
                .padding_left((f32::from(u16::try_from(depth).unwrap_or(1)) * 5.0 - 5.0) * 4.0)
                .child(text("•").text_color(t.muted_foreground))
                .child(
                    runs_text(&runs)
                        .line_height(LineHeight::Relative(1.625))
                        .flex(Flex::grow(1.0))
                        .min_width(0.0),
                )
                .into_element(),
            Block::Code(lang, body) => code(&body, if lang.is_empty() { "bash" } else { &lang }).into_element(),
            Block::Table(rows) => markdown_table(rows).into_element(),
        };
        if let Some((_, els)) = parts.last_mut() {
            els.push(el);
        }
    }
    parts
        .into_iter()
        .filter(|(_, els)| !els.is_empty())
        .map(|(title, els)| {
            (
                title,
                div()
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .gap(16.0)
                    .children(els)
                    .into_element(),
            )
        })
        .collect()
}

fn markdown_table(rows: Vec<Vec<Vec<Run>>>) -> Div {
    let width = rows.first().map_or(1, Vec::len);
    let tracks: Vec<Track> = (0..width).map(|_| Track::Fr(1.0)).collect();
    let mut rows = rows.into_iter();
    let head = rows.next().unwrap_or_default();
    let t = theme();
    div()
        .radius(10.0)
        .border(1.0)
        .border_color(t.border)
        .overflow(Overflow::Hidden)
        .child(
            table()
                .child(table_header([
                    table_row(&tracks, false).children(head.iter().map(|cell| table_head(runs_text(cell))))
                ]))
                .child(table_body(rows.map(|row| {
                    table_row(&tracks, false).children(row.iter().map(|cell| {
                        div()
                            .padding(8.0)
                            .child(runs_text(cell).line_height(LineHeight::Relative(1.625)))
                    }))
                }))),
        )
}
