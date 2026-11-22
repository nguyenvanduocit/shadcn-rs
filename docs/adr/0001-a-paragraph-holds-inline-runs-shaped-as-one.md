# 0001. A paragraph holds inline runs, shaped as one

Status: accepted, 2026-11-20; its cosmic-text basis superseded by ADR-0019 (a paragraph is still shaped as one); the inherited text properties a span honours superseded by ADR-0022 (text reads the `ContentStyle` its container provides).

## Context

A `gui` text node had one style. CSS lets one paragraph hold runs of other colours, weights, italics, underlines, fonts and
backgrounds, all wrapping as one: a link inside a sentence, `<code>` inside a line of prose, a Shiki token inside a line of
code. Without them shadcn-vue's demos could not be reproduced:

- `FieldGroupDemo` put its inline "Manage tasks" link on a line of its own where shadcn-vue wraps it inside the sentence
  (verdict 80 light, 79 dark, `/tmp/verdict-all.txt`).
- `TypographyDemo` has a link inside a paragraph; `CodeBlockTypesetDemo` and `CodeBlockMarkdownDemo` have padded inline
  `<code>` inside prose.
- Every highlighted code block (`CodeBlockHighlightingDemo`, `CodeBlockStartLineDemo` at 81 light) is a line of tokens of
  different colours, one `<span>` each, inside one `<pre>` line that wraps when asked to.

Text is shaped by cosmic-text 0.19.0 (`Cargo.lock`), whose `Buffer::set_rich_text` takes `(&str, Attrs)` spans
(`cosmic-text-0.19.0/src/buffer.rs:1102`); each `Attrs` carries family, weight, style, letter spacing, metrics and a
`metadata: usize` that every laid-out glyph keeps (`LayoutGlyph::metadata`, `src/layout.rs`).

## Options

1. **An inline formatting context in layout**: inline children of a block flow into line boxes. Taffy 0.15 has no inline
   layout, so this is a second layout engine beside it, with line breaking split between it and cosmic-text.
2. **A second element kind for rich text** beside `Text`: two paths that shape, measure, draw and hit-test text, and every
   rule on text (wrap, truncate, clamp, balance, align) owned twice.
3. **A paragraph is a string with runs**: `Text` holds its string and the spans styled inside it; the text system shapes
   them as one cosmic-text buffer through `set_rich_text`, and a plain text is a paragraph with no runs.

## Decision

Option 3. One text node is one paragraph, one owner of every text rule.

- `gui::element::Span` (`span("…")`, `Styled`, `on_click`) is appended with `Text::span` / `Text::spans`; a span with no
  style and no handler only adds text. Of its style a span honours the inherited text properties (colour, size, weight,
  family, italic, underline and its offset, line-through, tracking), a background with its radius and padding, and the
  cursor; layout properties have no effect on it.
- Layout resolves each span against the paragraph's text style into a `gui::text::Run` (byte range, `ResolvedText`,
  background, radius, padding) carried by `Content::Text { runs }`; `gui::text::Paragraph { content, runs }` is what
  `TextSystem::measure`, `selection` and `draw` take (`&str` converts into one with no runs, so inputs and labels read as
  before). The shape cache keys on what of a run changes shaping (range, size, weight, family, italic, tracking, padding).
- Each run's glyphs carry its index in `metadata`, so drawing colours, underlines, strikes and backgrounds each glyph by
  its run; a decoration or background spans a run's glyphs on each line, trailing white space left out.
- A run's left and right padding (`px-1` on inline `<code>`) are extra letter spacing on its first and last character, so
  lines wrap with the padding counted; the first glyph is drawn after the left padding. Top and bottom padding only grow
  the background, which covers the font's ascent and descent, as CSS paints an inline box.
- A span with a click handler or a cursor becomes a `SpanTarget` on its node's behaviour, its boxes found once the frame
  is placed (`TextSystem::selection` over the span's bytes); a click pressed and released on that paragraph inside a span
  calls the span's handler before any ancestor's, and the pointer shows the span's cursor.

## Consequences

- FieldGroupDemo's link, Typography's link and inline code, and token-coloured code lines are one paragraph each, wrapping
  as the browser wraps them.
- A run's own line height does not grow its line: a run takes the paragraph's lines (a larger inline font is the case CSS
  would grow). Its font size changes the glyphs only.
- Background corners are rounded on every line fragment; CSS slices a broken inline box (`box-decoration-break: slice`)
  and rounds only its outer ends.

## Revisit when

- A span needs **hover or focus state** (a link's `hover:underline`, a link reached by Tab): the seam is `SpanTarget`, which
  would gain a focus handle and hover refinement as `Common` has for nodes; links are not in the Tab order today.
- A span needs a **line height of its own**, an **inline box** (an icon or a badge inside a sentence), or **sliced
  decoration** across a line break: cosmic-text's per-span `metrics` and a placeholder glyph are where it goes.
- Right-to-left runs with padding: the padding letter spacing assumes the run's first character is drawn first.
