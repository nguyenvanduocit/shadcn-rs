//! Code block (`ui/code-block`): monospaced code in a bordered box, with an
//! optional header (title, actions), numbered lines, added, removed and
//! highlighted lines, wrapping, and a copy button.
//!
//! Vue's `group-data-[size|variant]/code-block` and `has-[…]` selectors
//! become [`CodeBlockProps`], which every part takes. Each line is one
//! paragraph of tokens, coloured as Vue's `CodeBlockToken` paints Shiki's
//! (see [`highlighted_lines`]).

mod highlighter;

pub use highlighter::{HighlightedToken, TokenColors, highlight_code, highlighted_lines};

use std::time::Duration;

use gui::clock::{self, TimerId};
use gui::prelude::*;
use gui::{Div, Icon, Signal, Track, component, div, icon, span, text};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::focus::StateLook;
use crate::part::Part;
use crate::theme::{Colors, theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CodeBlockVariant {
    #[default]
    Default,
    /// Error output: red border and text.
    Destructive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CodeBlockSize {
    #[default]
    Default,
    Sm,
}

/// What a code block's parts read from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CodeBlockProps {
    pub variant: CodeBlockVariant,
    pub size: CodeBlockSize,
    /// Whether it has a [`code_block_header`]: the code then sits closer to it.
    pub header: bool,
    /// Whether it has a [`code_block_footer`].
    pub footer: bool,
}

impl CodeBlockProps {
    fn sm(self) -> bool {
        self.size == CodeBlockSize::Sm
    }

    fn destructive(self) -> bool {
        self.variant == CodeBlockVariant::Destructive
    }
}

/// The box; its children are a header, the [`code_block_content`], and a
/// footer or [`code_block_floating_actions`].
#[must_use]
pub fn code_block(p: CodeBlockProps) -> Div {
    let t = theme();
    let d = div()
        .named("code-block")
        .position(Position::Relative)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .min_width(0.0)
        .overflow(Overflow::Hidden)
        .radius(10.0)
        .border(1.0);
    let d = if p.sm() {
        d.text_size(12.0).line_height(16.0)
    } else {
        d.text_size(14.0).line_height(20.0)
    };
    match p.variant {
        CodeBlockVariant::Default => d
            .border_color(t.input)
            .text_color(t.foreground)
            .when(t.dark, |d| d.background(t.input.opacity(0.3))),
        CodeBlockVariant::Destructive => d
            .border_color(t.destructive.opacity(if t.dark { 0.4 } else { 0.3 }))
            .text_color(t.destructive),
    }
}

/// The row above the code: a [`code_block_title`] and [`code_block_actions`].
#[must_use]
pub fn code_block_header(p: CodeBlockProps) -> Div {
    let t = theme();
    div()
        .named("code-block-header")
        .display(Display::Flex)
        .align_items(Align::Center)
        .flex_shrink(0.0)
        .min_width(0.0)
        .gap(8.0)
        .padding_x(if p.sm() { 10.0 } else { 12.0 })
        .padding_top(if p.sm() { 10.0 } else { 12.0 })
        .text_color(if p.destructive() {
            t.destructive
        } else {
            t.muted_foreground
        })
}

/// The file name or label, with an optional 14px icon before it.
///
/// Vue's title is one truncated line: the icon an inline `align-middle` box
/// with `me-1.5`, then the white space the template leaves before the text.
/// The icon's top sits where `align-middle` puts it on Inter's line (measured
/// in Chrome: 4.18px down a 20px line, 2.08px down a 16px one).
#[must_use]
pub fn code_block_title(p: CodeBlockProps, leading: Option<Icon>, title: impl Into<String>) -> Div {
    let t = theme();
    let d = div()
        .named("code-block-title")
        .display(Display::Flex)
        .align_items(Align::Start)
        .min_width(0.0)
        .font_family(FontFamily::INTER)
        .font_weight(500)
        .text_color(if p.destructive() { t.destructive } else { t.foreground });
    let (d, middle) = if p.sm() {
        (d.text_size(12.0).line_height(16.0), 2.08)
    } else {
        (d.text_size(14.0).line_height(20.0), 4.18)
    };
    let title = title.into();
    match leading {
        Some(i) => d
            .child(icon(i).size(14.0).flex_shrink(0.0).margin_end(6.0).margin_top(middle))
            .child(
                text(format!(" {title}"))
                    .truncate(true)
                    .nowrap(true)
                    .overflow_x(Overflow::Hidden)
                    .min_width(0.0),
            ),
        None => d.child(
            text(title)
                .truncate(true)
                .nowrap(true)
                .overflow_x(Overflow::Hidden)
                .min_width(0.0),
        ),
    }
}

/// The buttons at the end of the header.
#[must_use]
pub fn code_block_actions() -> Div {
    div()
        .named("code-block-actions")
        .display(Display::Flex)
        .align_items(Align::Center)
        .flex_shrink(0.0)
        .gap(4.0)
        .margin_start(Length::Auto)
        .margin_y(-4.0)
        .margin_end(-6.0)
}

/// Buttons floating over the code's top-right corner, each on the page's
/// background with a faint ring so the code does not show through. Put it
/// after the content so it paints above. Vue shows them only while the block
/// is hovered or focused; `gui` has no hover state an ancestor passes to its
/// children, so they always show, as Vue does on touch screens.
///
/// The buttons are ghost buttons, as shadcn's are here (the copy button
/// is): their rest fill becomes the page's background, and over it they
/// keep a ghost button's hover fill and focus ring.
#[must_use]
pub fn code_block_floating_actions<B: Styled + IntoElement>(
    p: CodeBlockProps,
    buttons: impl IntoIterator<Item = B>,
) -> Div {
    let t = theme();
    let inset = if p.sm() { 6.0 } else { 8.0 };
    div()
        .named("code-block-actions")
        .position(Position::Absolute)
        .top(inset)
        .right(inset)
        .display(Display::Flex)
        .align_items(Align::Center)
        .flex_shrink(0.0)
        .gap(4.0)
        .children(buttons.into_iter().map(|b| {
            // `bg-background ring-1 ring-foreground/10` over the button's
            // rest; its hover fill and focus ring stay over them.
            let i = b.node_interaction();
            let lit = if t.dark { t.accent.opacity(0.5) } else { t.accent };
            let b = b
                .background(t.background)
                .ring(Ring::new(1.0, t.foreground.opacity(0.1)));
            match i {
                Some(i) => b.background_while(move || i.hovered(), lit).focus_ring_on(i, &t),
                None => b,
            }
        }))
}

/// How a line is marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CodeBlockLineVariant {
    #[default]
    Default,
    /// A `+` in the gutter on a muted band.
    Added,
    /// A red `−` in the gutter on a red band.
    Removed,
    /// A muted band.
    Highlighted,
}

/// One line of code, its tokens, and how it is marked.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CodeBlockLine {
    pub tokens: Vec<HighlightedToken>,
    pub variant: CodeBlockLineVariant,
}

impl CodeBlockLine {
    /// A line of plain text, in the block's text colour.
    #[must_use]
    pub fn new(text: impl Into<String>, variant: CodeBlockLineVariant) -> Self {
        let content = text.into();
        let tokens = if content.is_empty() {
            Vec::new()
        } else {
            vec![HighlightedToken { content, colors: None }]
        };
        Self { tokens, variant }
    }

    /// A line of highlighted `tokens`.
    #[must_use]
    pub fn tokens(tokens: Vec<HighlightedToken>, variant: CodeBlockLineVariant) -> Self {
        Self { tokens, variant }
    }

    /// The line's text.
    #[must_use]
    pub fn text(&self) -> String {
        self.tokens.iter().map(|t| t.content.as_str()).collect()
    }
}

/// `code` as plain lines; a trailing newline does not make an empty last line.
#[must_use]
pub fn code_block_lines(code: &str) -> Vec<CodeBlockLine> {
    code.strip_suffix('\n')
        .unwrap_or(code)
        .split('\n')
        .map(|l| CodeBlockLine::new(l, CodeBlockLineVariant::Default))
        .collect()
}

/// How the code is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeBlockContentProps {
    /// Wrap long lines instead of scrolling sideways.
    pub wrap: bool,
    pub line_numbers: bool,
    /// The number of the first line.
    pub start_line: usize,
}

impl Default for CodeBlockContentProps {
    fn default() -> Self {
        Self {
            wrap: false,
            line_numbers: false,
            start_line: 1,
        }
    }
}

/// The code: `lines` in a monospaced grid that scrolls when it overflows
/// (give it `.max_height(…)` to cap its height).
#[must_use]
pub fn code_block_content(
    p: CodeBlockProps,
    c: CodeBlockContentProps,
    lines: impl IntoIterator<Item = CodeBlockLine>,
) -> Div {
    let t = theme();
    let pad = if p.sm() { 10.0 } else { 12.0 };
    let top = match (p.header, p.sm()) {
        (true, true) => 6.0,
        (true, false) => 8.0,
        (false, _) => pad,
    };
    let bottom = if p.footer { 8.0 } else { pad };
    let columns = if c.line_numbers {
        vec![Track::Auto, Track::Fr(1.0)]
    } else {
        vec![Track::Fr(1.0)]
    };
    let cells = lines.into_iter().enumerate().flat_map(|(i, line)| {
        let number = c.line_numbers.then(|| c.start_line.saturating_add(i));
        line_cells(&t, pad, c.wrap, number, line)
    });
    div()
        .named("code-block-content")
        .min_height(0.0)
        .min_width(0.0)
        .padding_x(pad)
        .padding_top(top)
        .padding_bottom(bottom)
        .font_family(FontFamily::GEIST_MONO)
        .line_height(LineHeight::Relative(1.625))
        // Code reads left to right in a right to left page (Vue's `dir="ltr"`).
        .direction(Direction::Ltr)
        .text_align(TextAlign::Start)
        .focusable()
        // Vue's `scrollbar-thin` under macOS's overlay scrollbars shows no
        // thumb at rest; the fade marks the overflow (`scroll-fade-x`,
        // `data-wrap:scroll-fade-y`).
        .scrollbar_hidden(true)
        .when(c.wrap, |d| {
            d.overflow_y(Overflow::Scroll)
                .overflow_x(Overflow::Hidden)
                .mask(ScrollFade {
                    axis: Axis::Vertical,
                    start: true,
                    end: true,
                    size: None,
                })
        })
        .when(!c.wrap, |d| {
            d.overflow(Overflow::Scroll).mask(ScrollFade {
                axis: Axis::Horizontal,
                start: true,
                end: true,
                size: None,
            })
        })
        .child(
            div()
                .named("code-block-code")
                .display(Display::Grid)
                .grid_columns(columns)
                .align_content(Justify::Start)
                .min_width(0.0)
                .children(cells),
        )
}

/// A line's grid cells: its number (when numbered) and its text, each
/// carrying the line's band so the band runs edge to edge (Vue's
/// `-mx-3 px-3` line on a subgrid).
fn line_cells(t: &Colors, pad: f32, wrap: bool, number: Option<usize>, line: CodeBlockLine) -> Vec<Div> {
    let band = match line.variant {
        CodeBlockLineVariant::Default => None,
        CodeBlockLineVariant::Added | CodeBlockLineVariant::Highlighted => Some(t.muted),
        CodeBlockLineVariant::Removed => Some(t.destructive.opacity(0.1)),
    };
    let sign = match line.variant {
        CodeBlockLineVariant::Added => Some(("+", t.muted_foreground)),
        CodeBlockLineVariant::Removed => Some(("\u{2212}", t.destructive)),
        CodeBlockLineVariant::Default | CodeBlockLineVariant::Highlighted => None,
    };
    let cell = || {
        div()
            .position(Position::Relative)
            .min_width(0.0)
            .when_some(band, |d, c| d.background(c))
    };
    let gutter = |d: Div| {
        d.when_some(sign, |d, (s, color)| {
            d.child(
                text(s)
                    .position(Position::Absolute)
                    .top(0.0)
                    .left(4.0)
                    .text_color(color),
            )
        })
    };
    // An empty line still holds a line's height.
    let empty = line.tokens.iter().all(|tk| tk.content.is_empty());
    let dark = t.dark;
    let code =
        text(if empty { " " } else { "" })
            .spans(line.tokens.into_iter().map(|tk| {
                span(tk.content).when_some(tk.colors, |s, c| s.text_color(if dark { c.dark } else { c.light }))
            }))
            .when(!wrap, |e| e.nowrap(true));
    match number {
        // The number is Vue's `before:min-w-6 before:text-end`, the column gap
        // its `gap-x-3`: the minimum holds the number, not the band's padding.
        Some(n) => vec![
            gutter(
                cell()
                    .margin_left(-pad)
                    .padding_left(pad)
                    .padding_right(12.0)
                    .text_color(t.muted_foreground)
                    .child(
                        div()
                            .display(Display::Flex)
                            .justify_content(Justify::End)
                            .min_width(24.0)
                            .child(text(n.to_string())),
                    ),
            ),
            cell().margin_right(-pad).padding_right(pad).child(code),
        ],
        None => vec![gutter(cell().margin_x(-pad).padding_x(pad).child(code))],
    }
}

/// The row under the code (a caption, a "show more").
#[must_use]
pub fn code_block_footer(p: CodeBlockProps) -> Div {
    div()
        .named("code-block-footer")
        .display(Display::Flex)
        .align_items(Align::Center)
        .flex_shrink(0.0)
        .min_width(0.0)
        .gap(8.0)
        .padding_x(if p.sm() { 10.0 } else { 12.0 })
        .padding_bottom(if p.sm() { 10.0 } else { 12.0 })
        .text_color(theme().muted_foreground)
}

/// How long the copy button shows its check (Vue's `copiedDuring: 2000`).
const COPIED_FOR: Duration = Duration::from_secs(2);

/// What [`code_block_copy_button`] is given: the text it copies.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeBlockCopyProps {
    value: String,
}

/// A ghost icon button that copies `value`, showing a check for two seconds
/// once it has (its own state, as `CodeBlockCopyButton.vue`'s `useClipboard`).
#[must_use]
pub fn code_block_copy_button(value: impl Into<String>) -> Part<CodeBlockCopyProps> {
    let root = button(ButtonVariant::Ghost, ButtonSize::IconXs)
        .named("code-block-copy-button")
        .into_div();
    Part::new(CodeBlockCopyProps { value: value.into() }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let copied = Signal::labeled("code-block copied", false);
            let reset: Signal<Option<TimerId>> = Signal::new(None);
            // The reset timer belongs to the instance: it is cancelled with it.
            let owner = gui::Scope::new();
            let copy = move || {
                let value = props.with_untracked(|p| p.props.value.clone());
                if !gui::clipboard::copy(&value) {
                    return;
                }
                if let Some(timer) = reset.get_untracked() {
                    clock::cancel(timer);
                }
                copied.set(true);
                reset.set(Some(owner.run(|| clock::after(COPIED_FOR, move || copied.set(false)))));
            };
            move || {
                let shown = if copied.get() { Icon::Check } else { Icon::Copy };
                props
                    .with(|p| p.slot.0.clone())
                    .child(icon(shown).size(12.0))
                    .on_click(move |_| copy())
                    .into_element()
            }
        })
    })
}

impl Part<CodeBlockCopyProps> {
    /// Greyed and not pressable (a block still streaming).
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.map_root(|d| d.disabled(disabled))
    }
}
