# 0019. Text is shaped by Parley and rasterized by `gui`

Status: accepted, 2026-11-21; built 2026-11-21 (M3, see "As built"). Supersedes ADR-0001's basis on cosmic-text (a paragraph is still shaped
as one) and ADR-0005's static Inter faces at optical sizes 16 to 32 (its Blink rules for baselines, `text-wrap:
pretty` and `tabular-nums` stay). ADR-0006's rounding of glyph origins on Linux at 1x is ADR-0020's.

## Context

Text is shaped by cosmic-text 0.19 and rasterized by swash (`crates/gui/src/text.rs`). Reading it against Parley,
Chrome and Skia (`docs/research-2026-11-21.md` sections 7 and 11.5; `research_notes/Khám phá kiến trúc GUI
Rust/text_accessibility.md`; `research_notes/Hiệu năng cực hạn/text_quality.md`) found:

- **A width change reshapes the paragraph.** `ShapeKey` holds `width: Option<u32>`, so a window resize, or layout
  measuring the same text at two widths, misses the cache and shapes again. Shaping and line breaking are not
  separated.
- **The text is not hinted.** Inter has `prep` and `gasp` but no `fpgm`. skrifa's default `AutoFallback` then picks
  the TrueType interpreter (`skrifa-0.49.0/src/outline/hint.rs:47-58`), which has no instructions to run.
- **Optical size is faked.** cosmic-text varies only `wght`, so ADR-0005 ships six static Inter instances (about 3.8
  MB) and picks the nearest. Any other size, or a variable axis of another font, is out of reach.
- **No inline boxes.** An icon or a chip inside a line of prose cannot wrap with the words.
- **Fallback is fontdb's list.** It is not the platform's per-script, per-locale fallback.
- **No text pieces for screen readers.** VoiceOver and Narrator read a text field character by character and word by
  word only through `TextRun` children (character lengths, word starts, positions). cosmic-text gives nothing to
  build them from.
- **Glyphs are blended with no gamma or contrast correction and never in subpixel (LCD) coverage.** Chrome on
  Windows and Linux draws LCD text when the user has it on, and corrects every glyph's alpha with Skia's
  `SkMaskGamma`. Ours is lighter and softer there, and by AGENTS.md a difference that is not better is a bug.

Parley 0.13.0 (2026-11-06, six releases in 2026) shapes with harfrust 0.15 and breaks lines, words and bidi with
ICU4X. It has:

- CSS line boxes, `WhiteSpaceCollapse` and `vertical-align` for inline boxes;
- `InlineBox` (U+FFFC, `parley/src/inline_box.rs:15`);
- `FontVariation` (`parley_engine/src/shape/shaper.rs:42`);
- carets and selections by extended grapheme cluster, and a correct caret where the direction changes;
- `PlainEditor` (`set_compose`, `clear_compose`, `ime_cursor_area`);
- since 0.12, the cursor and layout queries that let a caller build accessibility for text outside Parley.

fontique finds system fonts on CoreText, DirectWrite, fontconfig and Android, and falls back per script from the
platform's own data. Parley shapes a paragraph once and breaks it at a width separately (`break_all_lines`).
Blitz (0.13), Bevy (0.11) and Masonry use it. cosmic-text's last release is 0.19.0 (2026-04-22).

## Options

1. **Keep cosmic-text** and fix the cache by re-wrapping its buffer at a new width. That fixes the reshaping, but it
   leaves no inline boxes, no CSS line boxes, `wght` as the only axis, fontdb fallback, and no accessibility
   pieces.
2. **Parley + fontique for shaping and layout, our own rasterizer on skrifa** with Chrome's parameters per OS.
3. **Parley for shaping, the OS's rasterizer** (CoreText on macOS, DirectWrite on Windows), as Chrome does. It would
   match Chrome closest on each machine. But the output then depends on the OS version, so goldens and the verdict
   differ between machines. It also needs FFI bindings, with `unsafe` inside the dependency.
4. **Zed's DirectWrite-style contrast and gamma formula** (`crates/gpui_wgpu/src/shaders.wgsl:31-77`, ported from
   Windows Terminal) instead of Skia's.

## Decision

Option 2. The user chose our own rasterizer over the OS's on 2026-11-21. Zed's formula (option 4) is kept as the
candidate to compare against Skia's, and wins only if the side-by-side comparison with Chrome says it looks better.

- **Parley and fontique shape and lay out every paragraph.** ADR-0001's model stays: one text node is one paragraph,
  its spans become Parley style runs, and it is shaped as one. Inline boxes carry icons and chips in a line. Blink's
  rules from ADR-0005 (rounded baselines with the half-leading floored, `WrapStyle::Pretty`, `tnum`) are applied on
  Parley's lines.
- **Shaping is cached apart from line breaking.** The shape cache keys on a paragraph's content and resolved styles,
  never on width. Breaking at a new width reuses the shaped paragraph. At or above its max-content width, the
  unbroken result is reused. A long text is split at hard line breaks into paragraphs, cached by content hash, so a
  keystroke in a 10,000-line textarea reshapes one paragraph. Shapes are kept by ADR-0008's frame cache. That counts
  as in use whatever a virtual list holds in its pool or prefetch (ADR-0014).
- **Variable axes are set at run time.** `opsz` is the CSS px size (`font-optical-sizing: auto`, as Chrome sets it);
  `wght` and any other axis come from the style. Each distinct set of axis values gets a small interned id in the
  glyph key. The six static Inter faces and `assets/inter-opsz/` go.
- **`gui` rasterizes glyphs on every OS with skrifa.** Each OS gets a parameter set copied from Chrome:
  - **Gamma and contrast.** Skia's `SkMaskGamma` preblend, built as Skia builds it, is held in a 256×8 lookup texture
    the shader reads. Text colour stays out of the glyph key. The values come from Chromium's `skia/BUILD.gn`:
    - Linux: gamma 1.2, contrast 0.2.
    - Windows: sRGB, contrast 1.0, or the user's ClearType registry values.
    - macOS: sRGB, contrast 0, with stem darkening when `AppleFontSmoothing` is on, as CoreText thickens strokes.
  - **Hinting** is skrifa's autohinter, forced on (`Engine::Auto`, not `AutoFallback`), light and vertical only, with
    advances unchanged, below a 2× scale. From 2× up glyphs are unhinted.
  - **Subpixel positions:** four horizontal bins, one vertical.
  - **Subpixel (LCD) coverage on Windows and Linux,** when the user has it on. Coverage is rasterized at three times
    the width and run through FreeType's default five-tap filter. It is blended with wgpu's dual-source blending
    (`Features::DUAL_SOURCE_BLENDING`: DX12, Metal 1.2+, Vulkan `dualSrcBlend`), one blend state for every
    primitive. LCD applies only under Chrome's conditions:
    - drawn straight onto the opaque window;
    - at a whole device pixel;
    - at most 48 device px (`SK_MAX_SIZE_FOR_LCDTEXT`);
    - not in a group layer and not under an animating transform.

    Otherwise, on macOS, and without the feature, coverage is grayscale.
  - **User settings** are read through crates with safe APIs: `winreg` on Windows; XSETTINGS (`x11rb`) or the desktop
    portal (`zbus`) on Linux; `AppleFontSmoothing` on macOS. They are read again when the window regains focus or a
    setting changes.
- **Colour glyphs.** COLR fonts are painted through skrifa (sweep gradients by `vello_cpu`). sbix and CBDT bitmaps
  are scaled down with a good filter on the CPU. Both go into RGBA atlas pages, with no gamma table and no LCD.
- **Bitmap or curve.** Glyphs go to the multi-page atlas up to 128 device ppem, when the transform to the screen is a
  translation (or a static scale folded into the size) and no axis is animating. Larger text, rotated or skewed
  text, text whose scale or axes animate, and all text on a zoomable canvas are drawn as curves on the GPU (Slug,
  ADR-0017). Slug reads the same gamma table so the weight matches across the switch.
- **Editing and accessibility.** Inputs and textareas edit through `PlainEditor` (IME composition, word movement,
  `ime_cursor_area` sent each frame the caret moves). Their text is exposed as `Role::TextRun` children with
  character lengths, word starts, positions and the selection (ADR-0021).

## Consequences

- A resize re-breaks lines without reshaping. Text measured at two widths by layout shapes once.
- Glyphs gain real hinting at 1×. That changes stems on every page at 1×, so the verdict is re-run and the pages are
  re-scored.
- Text on Windows and Linux is drawn as Chrome draws it there: LCD when the user has it, Skia's gamma and contrast.
  macOS approximates CoreText's thickening with Pathfinder's constants, and never matches it pixel for pixel. Windows
  never matches exactly either, since Chrome draws through DirectWrite with its own hinting.
- Output is the same on every machine with the same settings, so goldens and the verdict stay comparable everywhere.
- A text sample page is rendered by Chrome and `gui` on macOS, Windows and Linux, with a fixed fontconfig file on
  Linux. Its sizes sit on both thresholds: 11 to 160 px, including 48/49 and 128/129. It is compared for:
  - ink weight and stem width;
  - colour fringing;
  - OKLab colour difference;
  - glyph positions.

  Unit tests check the gamma table against Skia's maths and that hinting changes the output. Lavapipe goldens cover
  gray, LCD-RGB and LCD-BGR; if lavapipe has no dual-source blending, the LCD goldens run on a Windows runner (DX12
  WARP).
- LCD atlas pages cost four bytes per pixel. There are two shader modules, with and without dual-source blending,
  both built at startup.
- A scroll area keeps LCD text only if it is drawn straight or its layer has an opaque background. Scroll offsets
  snap to device pixels.
- New dependencies: `parley` and `fontique` 0.13, `skrifa` 0.49, `winreg`, `x11rb`, `zbus` (MIT/Apache-2.0).
  cosmic-text, swash, the `[patch.crates-io]` entry and `deny.toml`'s `allow-git` for cosmic-text from ADR-0006,
  and the static Inter faces are removed. Parley breaks its API every minor release; each upgrade is done whole.
- fontique does not expose fontconfig's per-font render settings. Until a contribution upstream lands, Linux uses
  the desktop-wide settings.

## As built

`crates/gui/src/text/` (`mod.rs` shaping, the cache, breaking, measuring, carets and drawing; `fonts.rs` the font
collection, glyph keys and rasterizing; `raster.rs` the outline filler, emboldening and the LCD filter; `gamma.rs`
`SkMaskGamma`; `settings.rs` the user's settings; `color_glyph.rs` COLR, sbix and CBDT; `tests.rs`),
`gpu/mod.rs` and `gpu/shaders.wgsl` (gamma table, LCD pages, dual-source blending), `interact.rs` (editing and IME).
cosmic-text, swash, the `[patch.crates-io]` entry, `deny.toml`'s `allow-git` and `assets/inter-opsz/` are gone.
New dependencies: `parley`/`fontique` 0.13, `skrifa` 0.49, `vello_cpu` 0.4 (COLR gradients), `foldhash` 0.2 (the
glyph and shape caches), `plist` (macOS), `winreg` (Windows), `x11rb` and `zbus` (Linux); all pass `cargo deny`.

- **A paragraph is split at hard line breaks into pieces**, each shaped once into a Parley `Layout` and cached by a
  hash of its text and resolved styles (verified by equality), never by width. Breaking at a width calls
  `break_all_lines` on the cached layout; each piece also keeps the line widths and heights of its last four
  breakings, so layout's probes at alternating widths measure without breaking again, and its min-content width.
  `FrameReport::shaped` counts the pieces shaped in a frame. Unused pieces go after two frames (ADR-0008).
- **`PlainEditor` is not used.** It holds one `Layout` for the whole text and rebuilds it on every edit, which breaks
  the goal of reshaping one paragraph per keystroke in a long text. Editing uses the same primitives on the cached
  pieces instead: Parley's `Selection` and `Cursor` move by grapheme (`next_visual`/`previous_visual`), by word
  (`next_visual_word`, `previous_logical_word`), to line ends and across lines (`move_lines`), and a step past a
  piece's end lands in the next (`TextSystem::step`, `text::Step`). Backspace deletes one character of a combining
  sequence and a whole emoji cluster, as browsers do. ⌥ (Ctrl elsewhere) moves and deletes by word; ⌘ with an arrow
  goes to the line's or the text's ends on macOS. The IME area is `PlainEditor::ime_cursor_area`'s: the preedit (or
  the caret and its line's selection) widened by three average characters, clipped to the input. The time input
  edits segments, not text, and is unchanged.
- **Run padding** is letter spacing on the character before the run and on its last character, so no break
  opportunity comes between a padding and its text: Parley 0.13 makes an in-flow inline box an atomic inline on a
  U+FFFC with a break opportunity on each side. Only a padded run that starts its piece has a U+FFFC put before it
  (`ShapeKey::lead`). `Paragraph::boxes` (`InlineSlot`) places inline boxes on U+FFFCs the caller puts in the text,
  Parley's contract; no element uses them yet.
- **Font stacks** are ours then the generic family as Chrome resolves it: `"gui Inter", sans-serif` and
  `"gui Geist Mono", Menlo, monospace` on macOS (Chrome's default fixed font is Menlo; fontique's `monospace` is
  Courier, which lacks ✓ and ✗ and left them as `.notdef` boxes in the code blocks). Then fontique's per-script
  fallback.
- **A run's line height is its own** (a smaller inline code run inside 1.625-leading prose keeps its own box),
  as CSS computes it; using the paragraph's made lines a pixel taller.
- **macOS is not hinted.** Chrome on macOS draws through CoreText, which ignores hinting; hinted 1× text measured
  further from it (OKLab mean 0.157 against 0.130 at 13 px, 0.121 against 0.053 at 48 px). Hinting (forced light
  autohinter, below 2×) applies on Windows and Linux, where Chrome hints. The unit test checks that the hinter
  changes 1× output on every OS.
- **macOS stem darkening is fitted to Chrome**, not Pathfinder's constants: Pathfinder's (0.0121/ppem, at most
  0.3 px, none above 72 ppem, for macOS 10.13) left text 5 to 17% lighter than Chrome draws it now. Measured with
  the text sample, CoreText's thickening per side is `min(0.1 + 0.0085 × ppem, 0.45)` px horizontally and 1.25×
  that vertically, up to 256 device ppem, from which Chrome draws glyphs as paths with none.
- **LCD** follows Chrome's conditions as far as the scene knows them: only for glyphs drawn straight onto the
  opaque window (not in a layer), not colour, at most 48 device ppem. Positions are already whole device pixels plus
  a quarter-pixel bin. `GUI_TEXT_AA=gray|lcd-rgb|lcd-bgr|lcd-vrgb|lcd-vbgr|none` overrides the OS settings.
- **Not built here:** curves beyond 128 ppem (Slug, ADR-0017, task P9; built in M5a, below);
  `TextRun` children for screen readers (ADR-0021, M7); lavapipe goldens for gray/LCD-RGB/LCD-BGR and the text
  sample on Windows and Linux (only macOS could be run; on macOS, LCD was checked on Metal through the override);
  per-font fontconfig render settings (fontique has none, as written above).

The text sample (T8): `crates/shadcn/verdict/text-sample.tsv` lists 19 lines from 11 to 160 px (48/49, 128/129
included); `text-sample.html` lays them out in Chrome, `docs --text-sample` in `gui`, and `docs --text-compare`
measures each line at 1× and 2×: ink (sum of coverage, ours/Chrome), stem width (mean run of ≥ 50 % coverage),
colour fringe (mean chroma of inked pixels), OKLab difference (mean and p95 over pixels either side inks) and glyph
positions (ink edges and the shift that best aligns the column profiles, to a quarter of a device pixel).
`crates/shadcn/verdict/text.sh` runs the three. Thresholds, checked on macOS 26 with Chrome 145:

| Measure | Threshold | macOS 1× | macOS 2× |
| --- | --- | --- | --- |
| Ink, ours / Chrome | 0.95 to 1.05 | 0.976 to 1.026, except 13 px 0.942 | 0.983 to 1.026 |
| Stem width, ours − Chrome | ≤ 0.4 px | −0.37 to −0.02 | −0.35 to 0 |
| Colour fringe (gray text) | 0 | 0 (Chrome 0) | 0 |
| OKLab mean | ≤ 0.10 | 0.016 to 0.086, except 13 px 0.130 | 0.009 to 0.063 |
| Glyph shift | ≤ 0.25 px | 0 on every line | 0 on every line |
| Ink edges | ≤ 1 device px | within 1 | within 1 |

13 px at 1× is the one line outside two thresholds (ink 0.942, OKLab 0.130): open, to look at next.

With `GUI_TEXT_AA=lcd-rgb` or `lcd-bgr`, lines up to 48 px fringe (mean chroma 0.27 to 0.29) and from 49 px they do
not (0.000); gray text never does.

Measured (release, Apple Silicon; the machine was shared, so frame times vary about ±15 % run to run):

| Goal | Before (cosmic-text) | After |
| --- | --- | --- |
| A resize reshapes nothing | width in the shape key | 0 pieces shaped at four other widths (test) |
| Keystroke in a 10 000-line textarea | whole text reshaped | 1 piece of 10 001 (test) |
| Hinting changes 1× output | Inter not hinted (no `fpgm`) | autohinted bitmap differs (test) |
| `SkMaskGamma` table | none | byte-equal to Skia's C++ for Linux, Windows and macOS parameters (7 tests) |
| "The Joke Tax", 24 px semibold, `tracking-tight` | — | 141.0656 px (Chrome 141.06) |
| Verdict, 389 demos | 98/100, mean 98.48, 5 FAIL | 99/100, mean 98.55, 5 FAIL; none down more than 2, InputOTPFormDemo +5 |
| stress `text` steady p50 / layout | 7.1–7.7 ms / 0.45 ms | 6.6–7.0 ms / 0.44 ms; first frame 303 → 59 ms |
| stress `table` first frame / steady p50 | 819 ms / 374 ms | 484 ms / 318 ms |
| `docs --bench`, 85 pages | median p95 4.20 ms; FAIL code-block 17.4 ms | median p95 4.45 ms; FAIL code-block 17.6 ms and calendar 16.5 ms (both also fail on the old build in reruns: 17.6–21.9 ms) |
| Incremental equals cold | — | `GUI_COLD_CHECK=1` monkey 300 steps, seeds 1, 7, 42, and `--docs-trace`: no mismatch, no panic |


M5a (2026-11-21, tasks P9 and P11):

- **Slug above 128 ppem** and in transformed groups, as ADR-0017's "As built" says; colour glyphs stay in the atlas.
- **Dual-source blending for every primitive** (P11). With `DUAL_SOURCE_BLENDING`, every pipeline that draws onto
  the window or a layer (quads, path tiles, Slug) blends `src + (1 − src1) × dst`, its second output the per-channel
  coverage times alpha (LCD text) or alpha alone (everything else), so all primitives share one blend state and draw
  in one order. Without the feature the same shaders are built single-source with premultiplied alpha blending and LCD
  text falls back to grey. `every_primitive_draws_the_same_without_dual_source_blending` draws the same busy scene
  both ways: every channel within 1/255. `GpuOptions::dual_source` turns it off for tests.

## Revisit when

- The comparison with Chrome on a platform stays off after tuning: weigh that platform's rasterizer again (option 3)
  against stable output.
- Zed's contrast formula looks better than Skia's next to Chrome: switch, and record why in `verdict/better.tsv`.
- Text animating across 128 px visibly changes weight: move the threshold or blend the two over a range.
- Parley reshapes only the edited part of a paragraph (its safe-to-concat flags, 0.13): use it for long texts.
- Dual-source blending is slow on a GPU in a bench: draw LCD text in a pass of its own there.
