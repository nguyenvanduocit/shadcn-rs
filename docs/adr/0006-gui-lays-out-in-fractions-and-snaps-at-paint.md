# 0006. `gui` lays out in fractional pixels and snaps at paint

Status: accepted, 2026-11-20; its Taffy tree rebuilt each frame superseded by ADR-0014, its snapping of every box by ADR-0020, its cosmic-text patch by ADR-0019. Supersedes the baselines of ADR-0005 (their font and where they round).

## Context

The shadcn-vue demos are judged against Chrome (`crates/shadcn/verdict`). Blink lays boxes out in `LayoutUnit`s (1/64 px)
and keeps them fractional; it snaps a box's edges to device pixels only when it paints it (`PixelSnappedIntRect`), and
draws text at its fractional place with the baseline on a device pixel. `gui` let Taffy round every box to whole pixels,
which cost whole demos:

- **Text that fits wrapped.** A line measured at 281.03px got a 281px box and wrapped when painted (`SourcesDemo`, 0). The
  text system shaped every measured width 1px wider (`FIT_SLACK`) to hide it, then shaped again at the box's width when the
  text wrapped anyway.
- **Rows drift.** Each box rounded on its own, so places summed differently from Chrome's (`EmptyBackgroundDemo`'s button
  at y = 186 against 186.75, 87; `FieldGroupDemo` 86; `CodeBlockHeightDemo` 89; the field demos 90 to 93).

Three more layout faults were found beside it:

- **Min-content split words.** A text leaf measured for its min-content width was wrapped at width 0 with glyph breaks, so
  a flex item could shrink to one glyph a line (`InputGroupWithButtonGroup`'s "https://" and ".com", 89;
  `MessageScrollerPreviousContextDemo`'s "64px", 72). CSS min-content is the widest piece no line breaks; breaking inside a
  word (`overflow-wrap: break-word`) does not narrow it.
- **A percentage-wide image held its parent open.** An image measured its natural size whatever its style, so a `w-full`
  photo gave its card a 640px min-content width (`ItemHeaderDemo`, 0); css-sizing-3 §5.2.2 makes a percentage-sized
  replaced element's min-content contribution zero. With one side set, the other also kept its natural length instead of
  the natural ratio.
- **A clamped paragraph's last line began with a space.** cosmic-text 0.19.0 starts an ellipsized last line at the blank
  before its first word; upstream fixed it after the release (`c5d3205`, "fix ellipsize on last line").

With boxes fractional, a line's baseline lands on half device pixels, and the font ADR-0005 measured it from (the one most
glyphs on the line use) showed: a line of Persian digits took Geeza Pro's ascent, 1 device pixel off Chrome's
(`CalendarPersianDemo` 95 to 90). Blink takes a line's metrics from the primary font, the first of `font-family`.

## Options

1. Keep whole pixels and widen the slack where a demo fails: every new measurement finds a new rounding.
2. Lay out in whole device pixels: still not Blink's places, and layout would depend on the window's scale.
3. **Lay out fractionally, as Blink does, and snap where Blink snaps: box edges and clips at paint, baselines at draw.**

## Decision

Option 3.

- `layout.rs`: the Taffy tree has rounding off; frame boxes are fractional.
- `geometry.rs`: `Bounds::snapped(scale)` puts each edge on the nearest device pixel (half up), as `PixelSnappedIntRect`;
  `paint.rs` paints backgrounds, borders, rings, shadows, clip masks, images and shapes on the snapped box, and
  `effects::paint_background` clips to it. Text and its runs stay at their fractional place; icons keep their own snapping.
- `text.rs`: `FIT_SLACK` and the second shaping pass are gone: a measured width is rounded up to the layout unit
  (`widest_line`, 1/64 px), so text laid out at it keeps its lines. `Breaks` names where lines break: `None` (`nowrap`),
  `Words` (the min-content measure, `TextSystem::min_content`) and `WordsOrGlyphs` (laying out in a box, as
  `overflow-wrap: break-word`).
- `layout.rs` measures a text leaf at `MinContent` by `min_content`; an image (`ImageMeasure`) by its natural size, a set
  side keeping the natural ratio, and zero at `MinContent` when its width or max-width is a percentage.
- Baselines: the primary font is the style's own (`primary_metrics`), its ascent and descent rounded to whole pixels and
  the half-leading floored as before; the sum goes to the nearest device pixel, a half rounding up as Skia does
  (`baseline`, at draw, where the line's window place is known).
- cosmic-text is taken from upstream at `c5d3205` through `[patch.crates-io]` (and `deny.toml`'s `allow-git`), which also
  drops the second `fontdb` (0.23) from the lock.

## Consequences

- Full verdict (389 demos, `run.sh`, same tolerance): light mean 97.24 to 98.80, FAIL 5 to 2 (`InlineCitationDemo`, and
  `ProgressDemo`, whose timer did not fire in that run); dark mean 98.12 to 99.43, FAIL 4 to 1. `SourcesDemo` 0 to 100,
  `ItemHeaderDemo` 0 to 99, `InputGroupWithButtonGroup` 89 to 100, `EmptyBackgroundDemo` 87 to 100, `FieldGroupDemo` 86
  to 96, `CodeBlockHeightDemo` 89 to 99, `CalendarPersianDemo` 95 to 100. Four demos lost 2 points on half-pixel
  baselines (`FieldSwitchDemo`, `CodeBlockLanguagesDemo`, `CalendarSystems`, `CalendarYearAndMonthSelector`; see
  *Revisit when*).
- Inspect dumps print whole pixels of fractional boxes (`Bounds`' `Display`), so a dump can show a box 1px off its paint.
- The lock takes cosmic-text from git until a release carries the fix.

## Revisit when

- cosmic-text releases past 0.19.0 with `c5d3205`: drop the patch and the `allow-git` entry.
- A demo needs `overflow-wrap: normal` (a long word overflowing its box rather than breaking): add it as an inherited text
  property choosing `Breaks::Words` for layout; `WordsOrGlyphs` is the only layout mode today.
- The verdict's reference sits at a fractional offset: Chrome reports the specimen at y = 31.99993 (its `translateZ(0)`
  layer), so a baseline exactly on a half device pixel rounds down there and up here. Half the field and code-block lines
  fall on such halves; neither rounding wins across them (measured both ways), so the seam is the one rounding in
  `baseline`, and the harness, not `gui`, is what to change.
- An image's `max-width` percentage must keep the natural ratio while it clamps the width (Taffy clamps after measuring).
