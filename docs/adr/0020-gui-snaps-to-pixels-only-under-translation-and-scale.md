# 0020. `gui` snaps to pixels only under translation and scale

Status: accepted, 2026-11-21; built 2026-11-21 (M5a, P10; M5b, P1 and P5, see "As built"); a scroller has no layer (see "As built"), and the render thread snaps the scroll offsets it moves as paint does (M5c, P6, ADR-0015). Supersedes the snapping clause of ADR-0006 ("snaps box edges, clips,
images and shapes to device pixels at paint", `Bounds::snapped`, applied to every box whatever its transform); keeps
its fractional layout, 1/64 px text measures and baseline rule.

## Context

ADR-0006 snaps every box edge, clip, image and shape to the nearest device pixel at paint
(`crates/gui/src/geometry.rs:76`, `Bounds::snapped`). Blink does the same with `PixelSnappedIntRect`, but only for
content painted without a non-trivial transform. Three plans now meet that rule:

- **Continuous zoom and scale.** The zoomable canvas (`docs/research-2026-11-21.md` §11.3), scale animations
  (tw-animate's `zoom-in-95` on every dialog and popover) and pinch zoom move edges through fractional positions every
  frame. Snapping each edge on its own makes edges jump a pixel at different moments, so content shakes and boxes
  change width by a pixel while they grow.
- **Compositor scroll** (ADR-0015). Scroll becomes a property-tree offset applied on the render thread. A fractional
  offset (trackpad deltas are fractional) would place snapped content at a fractional position, where it is blurred
  and LCD text cannot be drawn.
- **LCD text on Linux and Windows** (§11.5). Chrome draws subpixel text only at whole-pixel positions on an opaque
  background. On Linux at 1× it also rounds glyph origins to whole pixels when drawing, because its hinting and LCD
  filter assume a pixel grid.

## Options

1. **Keep snapping everything always.** Shaking under zoom and scale, and blurred text after fractional scroll.
2. **Never snap.** Edges of 1 px borders blur everywhere, and the verdict loses the points ADR-0006 won.
3. **Snap only where the transform keeps the pixel grid: translation plus the display scale.**

## Decision

Option 3.

- **When to snap.** At paint, a chunk's transform state (property tree, ADR-0015) is classified:
  - **Grid-preserving:** a translation times the display scale, with any integer-pixel translation.
  - **Anything else:** scale other than the display scale, rotation, skew, perspective, or a running scale animation.

  Under a grid-preserving transform, box edges, clips, images and shapes snap as ADR-0006 says (`Bounds::snapped`,
  half up). Otherwise nothing snaps. Geometry stays fractional and is anti-aliased by area. A scale animation that
  ends snaps on its final frame.
- **Scroll offsets snap to device pixels** where they are applied: the render thread rounds the composited offset to
  the device pixel grid. The unrounded remainder stays in the synced property, so nothing drifts. Snapped content
  under a scroller therefore stays on the grid, and LCD text keeps working. A scroller gets its own offscreen layer
  only if its background is opaque, or its text would lose LCD.
- **Glyph origins.**
  - Text keeps its fractional horizontal position as ADR-0006 says, with 4 horizontal subpixel positions in the atlas
    key.
  - Exception: on Linux at 1× scale, glyph origins are rounded to whole pixels at draw, as Chrome does there.
  - Baselines stay on a device pixel, as ADR-0006 decides.
- **Text under a non-grid transform** (canvas zoom, scale animation, rotation) is drawn as curves (Slug), not from the
  bitmap atlas, so it neither snaps nor shimmers. Text crossing 128 ppem switches likewise.

## Consequences

- Expected: dialogs and popovers zoom in without edge jitter. The canvas zooms smoothly. Trackpad scrolling keeps
  borders and text crisp. LCD text works under scroll on Linux and Windows.
- Static pages paint exactly as under ADR-0006, so the verdict is unchanged for them. Pages that capture frames
  mid-animation may differ from today by anti-aliasing; Chrome does not snap mid-animation either.
- The painter must know each chunk's transform class. It reads it from the property tree, which is already the
  chunk's state.
- Linux 1× glyph rounding is a deliberate per-platform difference. Golden images are taken per platform.


## As built

Built on 2026-11-21 (M5a, task P10). `crates/gui/src/paint.rs` (when edges snap), `geometry.rs`
(`device_snapped`), `layout/mod.rs` (the scroll offset applied), `interact.rs` and `app.rs` (the scale given to
layout), `gpu/mod.rs` (transformed groups drawn straight), `text/mod.rs` (Linux 1× glyph origins, from M3).

- **When to snap.** There is no property tree yet (P1), so the class is read from the scene's groups: paint snaps
  box edges, clips, images and shapes (`Bounds::snapped`) unless it is inside a group whose transform is not the
  identity (scale, rotation, or a running zoom animation). A scale animation that ends returns to the identity, and
  its last frame snaps.
- **Scroll offsets** are applied in whole device pixels (`geometry::device_snapped`); the scroll state keeps the
  unrounded offset, so nothing drifts. The offset is *floored*, as Blink paints a scroller at `FlooredIntSize` of its
  offset (`PaintLayerScrollableArea`), not rounded: rounding moved MessageScroller's content a pixel away from
  Chrome's and lowered two verdict scores; floor matches it (MessageScrollerPreviousContextDemo 73 → 74).
- **Under a transform** nothing snaps and, in a group drawn straight through its transform (ADR-0017), quads are
  covered by exact area, glyphs drawn as curves (Slug) and icons as paths, so a zoom moves every edge continuously.

| Kiểm (plan, P10) | Result |
| --- | --- |
| Continuous zoom does not jitter | `a_continuous_zoom_does_not_jitter`: a 50.3 px rect scaled 1 to 1.195 in 40 steps is `50.3 × k` wide within 0.02 px at every step and grows at each |
| Snap only under translation and screen scale | `only_untransformed_boxes_snap_to_device_pixels` |
| Scroll offset in device pixels | `a_scroll_offset_moves_content_by_whole_device_pixels`: 10.3 px at 2× moves content 10.0 px |

Not met here:

- **The render thread snapping the composited offset** waits for compositor scroll (P6); today the UI thread applies
  it when it writes the scroll offset's transform entry.

### M5b (P1, P5)

`crates/gui/src/paint.rs`, `scene.rs`, `gpu/frame.rs`.

- **The class is read from the property tree.** A chunk snaps unless an effect above it has a transform of its own
  (`EffectNode::transformed`: a scale or turn, or a running zoom); a translation (scroll offsets, moving
  translations) is a transform entry, device-rounded, and keeps the grid.
- **A scroller has no layer of its own, over any background.** A scroll writes the scroll offset's transform entry;
  the chunks under it are replayed from the GPU as they were encoded (none recorded or encoded again) and the
  scroller's visible rectangle is redrawn into the opaque persistent canvas at a whole device pixel offset. Text under
  it is therefore always on the grid over an opaque background, which is what the rule wanted from a layer, and no
  layer texture or composite is paid. A layer would save only that redraw's fill; ADR-0015's compositor scroll on the
  render thread reconsiders it there.
- **Text in a transformed group that also fades** is drawn straight: when nothing it holds overlaps, the opacity is
  folded into its primitives and its text drawn from curves through the matrix (ADR-0016, per-case layers). A layer
  remains only for what cannot be drawn through a matrix (a shadow, an image, a colour glyph, text-through) or an
  opacity over overlapping primitives.

## Revisit when

- A verdict page differs from Chrome because Chrome snaps under a transform we classify as non-grid, or the reverse:
  align the classification with Blink's (`PaintPropertyTreeBuilder`'s pixel-snapping rules).
- Fractional display scales (1.25×, 1.5×) show seams between snapped neighbours: snap in layout units of the scale
  instead.
- Chrome changes its Linux glyph-origin rounding.
