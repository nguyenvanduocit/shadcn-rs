# 0005. `gui` sets text as Blink does

Status: accepted, 2026-11-20; its baselines' font and rounding superseded by ADR-0006; its static Inter optical-size faces superseded by ADR-0019 (the Blink rules stay); its inherited `tabular_nums` superseded by ADR-0022 (a field of the text style in `ContentStyle`). Extends ADR-0001 (a
paragraph holds inline runs).

## Context

The shadcn-vue demos are judged against Chrome's rendering (`crates/shadcn/verdict`). Four differences in how `gui`
shaped and placed text cost whole demos, each measured in Chrome (`agent-browser eval` on the verdict's pages):

- **Optical size.** Chrome sets Inter's `opsz` axis to the font size (`font-optical-sizing: auto`, axis 14 to 32):
  "The Joke Tax" at 24px semibold is 141.06px wide with `auto` and 147px with `none`. cosmic-text 0.19 varies only `wght`,
  so every heading came out wider (`TypographyH3` and `TypographyDemo` failed, 0 to 44).
- **Baselines.** Blink rounds the primary font's ascent and descent to whole pixels and floors the half-leading above
  them (`CalculateLeadingSpace`): Geist Mono at 14px on a 22.75px line has its 18px content box 2px below the line's top,
  not 2.375px. cosmic-text centres unrounded metrics, so baselines sat up to 0.4px off and glyphs were rasterised on
  fractional rows.
- **`text-wrap: pretty`.** Chromium re-breaks the last four lines of a paragraph so the last does not hold one word
  (the questionnaire's titles and descriptions).
- **`tabular-nums`** (OpenType `tnum`) on counters (`Question 1 of 3`, trace durations, the slider's values).

## Options

1. Tolerate the differences, or excuse them one by one in `verdict/better.tsv`: they are not better, only different.
2. Correct each demo's sizes by hand: every caller owns a font fact.
3. **Make the text system set text as Blink does**, once, for every caller.

## Decision

Option 3, in `crates/gui/src/text.rs`:

- Static instances of Inter at optical sizes 16, 18, 20, 24, 30 and 32 (`assets/inter-opsz/`, cut from the variable font
  by `instance.py` with fontTools, named `Inter OpszN`) are loaded beside Inter; a sans run takes the instance nearest its
  size, and Inter itself (opsz 14) up to 15px (`sans(size)`).
- Each shaped paragraph keeps its lines' baselines (`css_baselines`): ascent and descent of the line's font (the one most
  of its glyphs use) rounded, the half-leading floored. Drawing puts each baseline on a whole device pixel.
- `WrapStyle::{Auto, Balance, Pretty}` replaces the `balance` flag (`text_pretty()`, `text_balance()`); pretty re-breaks
  a paragraph's last four lines (all of a shorter one; `PRETTY_MAX_LINES = 4`, Blink's `kMaxLinesForOptimal`) as Blink's
  `ScoreLineBreaker` does, the lines before them greedy's:
  - **When:** greedy's last line is one segment (no break opportunity inside) shorter than a third of the box
    (`ShouldOptimize`), the window has no word cut by an emergency break, it holds at least three opportunities, and every
    line is shown (a clamp that hides the last line leaves the lines as they wrap).
  - **Window DP** (`best_breaks`, Blink's `ComputeScores`): each non-last line scores the square of its room, the last line
    none; a line wider than the box (plus Blink's 1/64 px epsilon) scores 1e12; every line adds a line penalty of 4 × box
    width × font size; breaking before the last segment adds 10 000 (`kOrphansPenalty`), four times that when it starts
    the last line; Blink's `best_hope` pruning and its `<=` tie-break are kept. A result with another line count than
    greedy's falls back to greedy, as in Blink.
  - **Opportunities** are Parley's own: the window is broken again with no room at all, so the breaker takes each one;
    the pieces it cuts a word into (emergency breaks) are summed back into the word.
  - **Realised as widths** (`Fit`: the window's first line and up to six per-line widths, copyable): each chosen line's
    max advance is its text's width plus 1/64 px, so Parley's greedy breaker lands exactly there; a line that does not
    falls back to greedy.
  - **Work:** one greedy pass that keeps the breaker's state at the start of each of the last four lines
    (`BreakerState`, `BreakLines::revert_to`), then the window alone (its probe and its re-break); the greedy result is
    the final layout when pretty does not apply.
  - **Balance** keeps its binary search on one width for all of a paragraph of at most six lines, realised through the
    same per-line widths; balanced and narrowed lines alike are now aligned in the whole box
    (`BreakLines::set_prior_line_width`), as Blink places them, not in the narrowed width shifted.
  - **Still different from Blink:** no hyphenation (so no hyphen penalties and no trigger on two hyphenated lines),
    no `text-align: justify` (`TextAlign` has no justify; Blink's justified scoring is not ported), no line-clamp of the
    new kind (Blink suspends scoring there; ours only declines when the last line is hidden), no first-line indent, and
    the zoom factor is 1.
  - Parley is pinned exactly (`=0.13.0`): `set_prior_line_width` is doc-hidden.
- `tabular_nums` is an inherited text property that adds the `tnum` feature to the run's attributes.

## Consequences

- Typography: H1 96, H2 to Table 100 (`run.sh typography`); the code blocks' line-numbered demos and `CodeBlockSizeDemo`
  (88 to 100) line up glyph for glyph.
- The binary carries six more Inter faces, about 3.8 MB (637 KB each).
- A baseline is a whole number of pixels below its line's top, as in Chrome, so text one line tall may sit up to half a
  pixel from where cosmic-text would centre it.

## Revisit when

- cosmic-text varies any axis (it sets `wght` from `Attrs` only): set `opsz` to the size and drop the static faces.
- A font other than Inter has an optical axis the design uses.
- A demo's lines start at a fractional page offset in Chrome (`CodeBlockHeightDemo`'s box sits at y = 31.999 there):
  half-pixel baselines then round the other way in Chrome; the seam is the one rounding in `TextSystem::draw`.
