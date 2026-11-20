# 0004. Direction is an inherited property of `gui` nodes

Status: accepted, 2026-11-20. Extends ADR-0001 (inline runs) and ADR-0003 (`scroll-fade`).

## Context

Three shadcn-vue demos set HTML's `dir="rtl"` on part of the page: `CodeBlockRTLDemo` (an Arabic sentence and a code
block whose header runs right to left while its code stays `dir="ltr"`), `ShimmerRTLDemo` and `ScrollFadeRTLDemo` (a
row of Arabic tags that overflows to the left and fades on its inline end). CSS `direction` is inherited and changes:

- which way flex rows and grid columns run, and where an in-flow block child sits (against the right edge);
- what `text-align: start` (the initial value) and `end` mean;
- logical margins (`ms-*`, `me-*`), which the code block's actions and title use (`ms-auto -me-1.5`, `*:[svg]:me-1.5`);
- horizontal overflow: it lies to the left, scrolling starts at the right, and `scroll-fade-x` runs its gradient
  `to left` (`packages/cli/src/tailwind.css`, `&:where([dir="rtl"], [dir="rtl"] *)`).

Taffy 0.15 has `Style::direction` and honours it in flexbox and grid (`compute/flexbox.rs`, `compute/grid/alignment.rs`),
not in block layout (`compute/block.rs` reads no direction). cosmic-text picks a paragraph's base direction from its
first strong character and right-aligns a right-to-left line when given no alignment.

Arabic needs no font of its own: `TextSystem::new` loads the system fonts, and cosmic-text's macOS fallback for the
Arabic script is Geeza Pro, the face Chrome falls back to (measured: the demo sentence is 163.80px wide in Chrome with
`font-family: Inter, "Geeza Pro"` and as rendered).

## Options

1. **Mirror the finished layout** of an RTL subtree: wrong for code inside it that is `dir="ltr"`, and for text.
2. **A direction flag per component** (`reverse` on flex rows, `rtl` on scroll fades): every component owns a page fact.
3. **`direction` as an inherited text property**, given to Taffy for each node, with what Taffy leaves out done in `gui`.

## Decision

Option 3.

- `style::Direction { Ltr, Rtl }` is `TextStyle::direction`, inherited into `ResolvedText::direction`; `dir_rtl()` and
  `dir_ltr()` set it. `Style::to_taffy(direction)` passes it to Taffy.
- `TextAlign::Start` (the initial value) and `End` resolve by direction (`ResolvedText::physical_align`); shaping is given
  that alignment, so an Arabic line in a left-to-right box starts on the left as CSS puts it.
- `Style::margin_start` and `margin_end` (`ms`, `me`, `ms_auto`, `me_auto`) land on the left or right in `to_taffy`.
- `layout::location` places an in-flow child of a right-to-left block container against the content box's right edge
  (its right margin kept), which Taffy's block layout does not.
- A scroll container's offset runs from the inline start: right to left, its range is how far the children reach left
  of the padding box, the children move right as it grows, and the wheel's horizontal delta is reversed. `scroll-fade-x`
  under right to left turns its gradient `to left` (`effects::mask`), so the start fade sits on the right.
- `shimmer` sweeps right to left through `Shimmer::reverse` (ADR-0003), which a right-to-left caller passes, as the CSS
  utility does under `[dir=rtl]`.

## Consequences

- `ShimmerRTLDemo` 95, `ScrollFadeRTLDemo` 98, `CodeBlockRTLDemo` 80 light: its layout matches, and Chrome draws the
  medium-weight Arabic title with Geeza Pro's bold face (a fallback face at weight 500 is 40.28px wide there, the bold's
  width; `font-family: "Geeza Pro"` at 500 is the regular's 33.89px), where cosmic-text's fallback takes the regular.
- Every component that writes a physical margin where Vue writes a logical one (`ml`/`mr` for `ms`/`me`) is wrong under
  right to left until it is changed; the code block's are.

## Revisit when

- A paragraph's **base direction** must follow `dir` rather than its first strong character (Latin text with `dir="rtl"`,
  trailing punctuation): cosmic-text 0.19 has no way to set it; the seam is `attrs_of`/`shape` in `text.rs`, which see
  `ResolvedText::direction`.
- **Logical insets and padding** (`start-*`, `end-*`, `ps-*`, `pe-*`; the floating code-block actions' `end-2`) are
  needed: they go beside `margin_start`/`margin_end`.
- Chrome's **fallback weight** must be matched (a medium Arabic run drawn bold): it is cosmic-text's fallback face choice.
- A vertical writing mode or `direction` on grid placement beyond Taffy's.
