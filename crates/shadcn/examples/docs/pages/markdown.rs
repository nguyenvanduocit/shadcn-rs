//! Markdown as shadcn-vue's docs render an agent's reply (`components/Markdown.vue`:
//! comark in a `typeset` box, fenced code as a [`code_block`]). Only what
//! agent replies use is drawn: paragraphs, lists, strong, emphasis, inline
//! code, links and fenced code; anything else shows as its text.

use gui::prelude::*;
use gui::{Div, Element, Span, Text, div, span, text};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use shadcn::{
    CodeBlockContentProps, CodeBlockProps, code_block, code_block_actions, code_block_content, code_block_copy_button,
    code_block_header, code_block_title, highlighted_lines, theme,
};

/// typeset's `--typeset-leading`.
const LEADING: f32 = 1.75;
/// typeset's `--typeset-flow`, in ems: the space above every block but the first.
const FLOW: f32 = 1.25;

/// A block being built: its kind, and the inline runs gathered so far.
enum Open {
    Paragraph(Vec<Span>),
    List { ordered: Option<u64>, items: Vec<Element> },
    Item(Vec<Element>, Vec<Span>),
    Fence { language: String, code: String },
}

/// The inline style in force: strong, emphasis, link depth.
#[derive(Clone, Copy, Default)]
struct Inline {
    strong: u8,
    emphasis: u8,
    link: u8,
}

/// `source` in a `typeset` column whose text is `size` px. Each fence's copy
/// button keeps its own state, so a reply streamed in keeps it as it grows.
pub fn markdown(source: &str, size: f32) -> Div {
    let mut blocks: Vec<Element> = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    let mut inline = Inline::default();
    for event in Parser::new(source) {
        match event {
            Event::Start(Tag::Paragraph) => stack.push(Open::Paragraph(Vec::new())),
            Event::Start(Tag::List(first)) => stack.push(Open::List {
                ordered: first,
                items: Vec::new(),
            }),
            Event::Start(Tag::Item) => stack.push(Open::Item(Vec::new(), Vec::new())),
            Event::Start(Tag::CodeBlock(kind)) => stack.push(Open::Fence {
                language: match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                },
                code: String::new(),
            }),
            Event::Start(Tag::Strong) => inline.strong = inline.strong.saturating_add(1),
            Event::Start(Tag::Emphasis) => inline.emphasis = inline.emphasis.saturating_add(1),
            Event::Start(Tag::Link { .. }) => inline.link = inline.link.saturating_add(1),
            Event::End(TagEnd::Strong) => inline.strong = inline.strong.saturating_sub(1),
            Event::End(TagEnd::Emphasis) => inline.emphasis = inline.emphasis.saturating_sub(1),
            Event::End(TagEnd::Link) => inline.link = inline.link.saturating_sub(1),
            Event::Text(t) => match stack.last_mut() {
                Some(Open::Fence { code, .. }) => code.push_str(&t),
                Some(Open::Paragraph(runs) | Open::Item(_, runs)) => runs.push(styled(span(t.to_string()), inline)),
                _ => {}
            },
            Event::Code(c) => {
                if let Some(Open::Paragraph(runs) | Open::Item(_, runs)) = stack.last_mut() {
                    runs.push(code_span(&c, size));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(Open::Paragraph(runs) | Open::Item(_, runs)) = stack.last_mut() {
                    runs.push(span(" "));
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if let Some(Open::Paragraph(runs)) = stack.pop() {
                    place(&mut stack, &mut blocks, paragraph(runs).into_element());
                }
            }
            Event::End(TagEnd::Item) => {
                if let Some(Open::Item(mut children, runs)) = stack.pop() {
                    // A tight list item's text comes without a paragraph.
                    if !runs.is_empty() {
                        children.insert(0, paragraph(runs).into_element());
                    }
                    if let Some(Open::List { items, .. }) = stack.last_mut() {
                        items.push(item(children, size));
                    }
                }
            }
            Event::End(TagEnd::List(_)) => {
                if let Some(Open::List { ordered, items }) = stack.pop() {
                    place(&mut stack, &mut blocks, list(ordered, items, size).into_element());
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(Open::Fence { language, code }) = stack.pop() {
                    place(&mut stack, &mut blocks, fence(&language, &code).into_element());
                }
            }
            _ => {}
        }
    }
    // A stream cut mid-block: show what has arrived.
    while let Some(open) = stack.pop() {
        let el = match open {
            Open::Paragraph(runs) | Open::Item(_, runs) => paragraph(runs).into_element(),
            Open::List { ordered, items } => list(ordered, items, size).into_element(),
            Open::Fence { language, code } => fence(&language, &code).into_element(),
        };
        place(&mut stack, &mut blocks, el);
    }
    flow(
        div()
            .named("markdown")
            .width(pct(100.0))
            .min_width(0.0)
            .overflow(Overflow::Hidden),
        blocks,
        size,
    )
    .text_size(size)
    .line_height(size * LEADING)
    .text_color(theme().foreground)
}

/// Put a finished block in the list item it belongs to, or at the top level.
fn place(stack: &mut [Open], blocks: &mut Vec<Element>, el: Element) {
    match stack.last_mut() {
        Some(Open::Item(children, runs)) => {
            if !runs.is_empty() {
                children.push(paragraph(std::mem::take(runs)).into_element());
            }
            children.push(el);
        }
        _ => blocks.push(el),
    }
}

/// `blocks` stacked with the flow between them (typeset's
/// `margin-block-start`, trimmed on the first).
fn flow(column: Div, blocks: Vec<Element>, size: f32) -> Div {
    let gap = FLOW * size;
    column
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .children(
            blocks
                .into_iter()
                .enumerate()
                .map(|(i, b)| div().min_width(0.0).when(i > 0, |d| d.margin_top(gap)).child(b)),
        )
}

fn styled(s: Span, inline: Inline) -> Span {
    s.when(inline.strong > 0, |s| s.font_weight(600))
        .when(inline.emphasis > 0, |s| s.italic(true))
        .when(inline.link > 0, |s| s.font_weight(500).underline(true))
}

/// Markdown.vue's inline code: `rounded-sm bg-muted px-1 py-0.5 font-mono text-[0.9em]`.
fn code_span(code: &str, size: f32) -> Span {
    let em = 0.9 * size;
    span(code.to_owned())
        .font_family(FontFamily::GEIST_MONO)
        .text_size(em)
        .line_height(em * LEADING)
        .background(theme().muted)
        .radius(6.0)
        .padding_x(4.0)
        .padding_y(2.0)
}

fn paragraph(runs: Vec<Span>) -> Text {
    text("").spans(runs)
}

/// A list item: its blocks, `0.5em` below the one before.
fn item(children: Vec<Element>, size: f32) -> Element {
    let half = 0.5 * size;
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .min_width(0.0)
        .children(
            children
                .into_iter()
                .enumerate()
                .map(|(i, c)| div().min_width(0.0).when(i > 0, |d| d.margin_top(half)).child(c)),
        )
        .into_element()
}

/// typeset's list: `padding-inline-start: 1.5em`, each item `0.5em` apart with
/// its marker (muted) outside it and `0.4em` of room.
fn list(ordered: Option<u64>, items: Vec<Element>, size: f32) -> Div {
    let t = theme();
    let (indent, room, half) = (1.5 * size, 0.4 * size, 0.5 * size);
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .padding_left(indent)
        .children(items.into_iter().enumerate().map(|(i, el)| {
            let marker = match ordered {
                Some(first) => format!("{}.", first.saturating_add(u64::try_from(i).unwrap_or(u64::MAX))),
                None => "•".to_owned(),
            };
            div()
                .position(Position::Relative)
                .min_width(0.0)
                .margin_top(half)
                .padding_left(room)
                .child(
                    div()
                        .position(Position::Absolute)
                        .top(0.0)
                        .left(gui::px(-indent))
                        .width(indent)
                        .display(Display::Flex)
                        .justify_content(Justify::End)
                        .padding_right(room)
                        .text_color(t.muted_foreground)
                        .child(text(marker)),
                )
                .child(el)
        }))
}

/// MarkdownCodeBlock.vue: the language as title, a copy button, numbered lines.
fn fence(language: &str, code: &str) -> Div {
    let p = CodeBlockProps {
        header: true,
        ..CodeBlockProps::default()
    };
    let content = CodeBlockContentProps {
        line_numbers: true,
        ..CodeBlockContentProps::default()
    };
    let title = if language.is_empty() { "text" } else { language };
    code_block(p)
        .child(
            code_block_header(p)
                .child(code_block_title(p, None, title))
                .child(code_block_actions().child(code_block_copy_button(code))),
        )
        .child(code_block_content(p, content, highlighted_lines(code, language)))
}
