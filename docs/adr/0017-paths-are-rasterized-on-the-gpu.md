# 0017. Paths are rasterized on the GPU

Status: accepted, 2026-11-21. Open coverage defects and proposed revision to the area oracle in [ADR-0031](0031-path-coverage-is-the-area-of-the-filled-and-clipped-region.md); paths, icons and Slug built 2026-11-21 (M5a, P8 and P9, see "As built"); the infinite canvas built 2026-11-21 (M6, D8 and D9; its raster scheduled as Chromium's tile manager, rotation, text and simplification per level built 2026-11-21; wrapped text, the live layer with tiles invalidated per change, and dropped objects' per-pixel averages built 2026-11-22, see "As built: the canvas"); tiny-skia gone from `Cargo.lock` 2026-11-22, gui drawing its own Wayland window decorations (see "As built: window decorations"). Supersedes, in ADR-0011, "shapes are still rasterised by tiny-skia, once
per shape and sub-pixel offset" and "glyphs (swash), icons (resvg), shapes (tiny-skia)" as CPU rasters, its "revisit
when" on tessellating paths with lyon, and `Renderer::capture` returning a `tiny_skia::Pixmap`. Keeps closed-form
quads for rects, rounded rects, bands and shadows.

## Context

Paths are drawn on the CPU (`docs/research-2026-11-21.md` §11.3; `research_notes/Hiệu năng cực hạn/gpu_paths_canvas.md`):

- `Item::Shapes` holds `Shape`s with only move, line, quad, cubic and close, one fill colour and one stroke (colour,
  width). There is no fill rule, no gradient on a path, no join, cap or dash choice.
- tiny-skia fills them (`crates/gui/src/gpu/raster.rs:10-38`, always non-zero, round caps and joins), cached by hash
  (`shape_raster_key`) and uploaded as one texture each, which breaks batches.
- Lucide icons go through resvg and tiny-skia (`crates/gui/src/icon.rs:177`).
- Charts took 59 ms per frame on the CPU before ADR-0011; an animated path is rasterised again on every change.

The targets of `AGENTS.md` make this the wrong place: charts of millions of points (ADR-0018) and zoomable canvases,
where every zoom step changes every path's device shape. Text has the same problem at large or changing scales:
a bitmap atlas holds one size, so a canvas zoom or a scale animation rasterises again or blurs.

What exists:

- **vello compute** (`references/vello/research`): exact-area coverage on 16×16 tiles
  (`research/vello_shaders/shader/shared/config.wgsl:46-47`, `fine.wgsl:1011-1061`); about 20 dispatches that redo
  the whole scene each frame, and fixed bump buffers that drop a frame on overflow.
- **Sparse strips** (`vello_cpu`, `vello_gpu`; Skia Graphite `kCPUSparseStripsMSAA8`): coverage computed on the CPU
  (`vello_common/src/strip.rs:584-595`), so a canvas zoom re-rasterises every path on the CPU.
- **GPUI** (2026): paths binned on the CPU and covered in the fragment shader with Slug's two-ray weighting
  (`references/zed/crates/gpui/src/path_winding.rs:1-26`, `gpui_wgpu/src/shaders.wgsl:1134-1310`). Close to our
  target, but binned on the CPU and approximate at thin features and sharp corners.
- **Stencil-then-cover with MSAA** (Skia Graphite tessellation, Impeller): 5 or 9 grey levels; Skia itself sends
  small paths to a CPU atlas "for higher quality" (`references/skia/src/gpu/graphite/Device.cpp:2279-2287`).
- **Loop–Blinn** and **lyon tessellation with a fringe**: gradient-estimated distance that breaks near triangle edges
  (GPU Gems 3, ch. 25), CPU re-tessellation on zoom, trouble with self-intersection.
- **Pathfinder** (`references/pathfinder/renderer/src/tiles.rs:19-20`) and **forma** (exact area, sort per frame):
  right idea, unmaintained.
- **Stroke expansion on the GPU**: Levien and Uguray, "GPU-friendly Stroke Expansion", HPG 2024
  ([arXiv 2405.00127](https://arxiv.org/abs/2405.00127)): fully parallel, Euler-spiral offsets, 3.09 ms for 475 855
  segments of *waves* on an M1 Max.
- **Slug** (Lengyel, [JCGT 6(2) 2017](https://jcgt.org/published/0006/02/02/)): glyph coverage from curves in the
  fragment shader, bands per glyph, the `0x2E74` root-eligibility table; patent dedicated to the public domain on
  2026-03-17 ([Terathon](https://terathon.com/blog/decade-slug.html)). About 40× an atlas sample per pixel at 32
  ppem. HarfBuzz's hb-gpu ships it in WGSL over a storage buffer (`references/harfbuzz/src/hb-gpu-fragment.wgsl`);
  Makepad found banded shaders miscompiled on some drivers and scans all curves instead
  (`references/makepad/platform/script/resources/glyph_coverage.splash:283-289`).
- **Canvas precision**: `f32` has 24 bits; a point at 10⁶ world units has a 0.06-unit step, 60 px at 1000× zoom.
  Relative-to-center and relative-to-eye rendering (Ohlarik, "Precisions, Precisions", 2008; Cesium
  `czm_translateRelativeToEye`) keep GPU values small.
- **Chromium tiling** (`cc/layers/picture_layer_impl.cc`): while pinching, raster scale moves at most 2× toward the
  ideal (`kMaxScaleRatioDuringPinch`), snaps to an existing tiling within 1.2× (`kSnapToExistingTilingRatio`), and
  re-rasters at the exact scale when the pinch ends; low-res tilings were deleted as dead code on 2025-06-23 (commit
  `868290b`), so a missing tile shows as checkerboard.

## Options

1. **Keep tiny-skia, cache harder.** Every zoom step and every animated path is a CPU raster and an upload.
2. **vello compute as a second renderer.** Exact quality, but its own painting order, a whole-scene redo each frame and
   overflowing buffers.
3. **Stencil-then-cover or tessellation with MSAA.** Below Chrome's analytic anti-aliasing on thin strokes and text.
4. **Our own tiled exact-area rasterizer inside the main pass**, with Slug for transformed text.

## Decision

Option 4.

- **Data.** `PathData` (immutable, `Arc`, keyed by id and version): verbs, `f32` points in the node's space, bounds,
  each curve's Wang constant, fill rule non-zero or even-odd. `PathStyle`: fill, or stroke with width, join, miter
  limit, cap and dash; paint is solid, linear, radial, conic or image pattern, in the existing OKLab gradient table.
  `Item::Path { path, style, clip_id, transform_id }` replaces `Item::Shapes`; `Shape` goes.
- **Closed forms stay.** Rect, rounded rect, band, shadow, and new ellipse, capsule and arc are recognised by the path
  builder and drawn as signed-distance quads, never as paths.
- **Upper bounds on the CPU.** For each path to bin, the CPU computes from the Wang constants and the matrix's largest
  scale an upper bound on segments (`n = ceil(sqrt(s·W / (8·tol)))` per curve; strokes add joins and caps) and on
  tiles, and reserves that range in the GPU buffers. No buffer overflows, nothing is read back, no indirect draw is
  needed, and path tiles draw in painting order between other items. Unused slots are empty instances the vertex shader
  discards. Flattening tolerance is 1/8 device px.
- **Compute passes before the main pass**, only for paths whose linear matrix, fractional translation, visible rect,
  path or style changed:
  1. `flatten`: to device segments (Wang, or Levien's quadratic approximation), segments left of the visible rect still
     counted for winding (vello's culled windings, `vello_common/src/tile.rs:45-151`);
  2. `stroke`: Levien–Uguray expansion to a closed outline filled non-zero, dashes cut by arc length on the GPU;
  3. `bin`: DDA over 16×16 tiles, segment counts and winding deltas at tile tops;
  4. `backdrop`: prefix sum of winding per tile row;
  5. `compact`: tiles with segments become edge instances, runs of filled empty tiles become one solid instance;
     segment lists are sorted by min x; a tile with more than 64 segments splits into four 8×8 tiles.
- **Fragment coverage** in the main pass: start from the tile's backdrop, add each segment's exact area to the right
  of it within the pixel (vello `fine.wgsl:1019-1049`), stop early once sorted segments pass the pixel, apply the fill
  rule (`fine.wgsl:1051-1060`), multiply by paint, rounded clip and the effect node's opacity. Strokes thinner than 1
  device px are expanded to 1 px and modulated by their width, as Skia draws hairlines. Gradients are dithered with
  blue noise before quantising.
- **Reuse.** A binned path is kept while its path, style, linear matrix and fractional translation are unchanged and its
  visible rect stays inside the binned rect: whole-pixel scroll and pan move it by an offset only. Zoom or rotation
  re-bins it on the GPU; the CPU writes one job.
- **Icons.** Lucide SVGs are parsed once into `PathData` (resvg and tiny-skia leave the draw path). A static icon at an
  axis-aligned transform is rasterised once by this rasterizer into the R8 mask atlas at its device size and tinted in
  the shader, so a list of a thousand icons costs a thousand atlas quads. A scaled, rotated or zooming icon draws as a
  path. The threshold rule is the same as text's.
- **Text on curves (Slug).** The bitmap atlas (ADR-0019) holds static, axis-aligned UI text up to 128 device ppem.
  Slug draws text above 128 ppem, rotated or skewed text, text whose scale is animating (it returns to the atlas when
  the animation ends at scale 1), and all text inside a canvas camera. It is a port of hb-gpu's WGSL with two prebuilt
  variants, banded and full curve scan; at startup one test glyph is rendered by both and read back once, and an
  adapter where they differ uses the full scan. Glyph curves live in a storage buffer keyed by font, glyph and
  quantised variation coordinates, evicted by age. Dilation follows the transform's Jacobian. Coverage goes through the
  same gamma and contrast as the atlas. Below about 3 device ppem on a canvas, lines draw as grey bars.
- **Infinite canvas.**
  - A `Camera` node in the transform tree (ADR-0016) holds centre, zoom and rotation in `f64`; scroll is a camera with
    translation only. Pan and zoom write camera entries, nothing else.
  - The world is divided into anchor cells (for example 4096 units). Each item stores its cell as `i64` and a local
    `f32` position. Each frame the CPU computes, in `f64`, one camera-relative `f32` matrix per visible cell; the GPU
    sees only those and small local coordinates. An item whose size × zoom exceeds 2¹⁸ is re-anchored near the camera
    from its `f64` source, using a BVH over its curves.
  - Culling, hit testing and marquee use a dynamic AABB tree with fattened boxes (Box2D's `b2DynamicTree`); hit testing
    runs in `f64` on the CPU. Z order is fractional keys, so the draw list is never fully re-sorted. Paths with many
    points keep a curve BVH so only the visible part is flattened.
  - By default every visible item is drawn directly each frame, sharp at the current zoom. When the estimated cost of
    the visible set exceeds the frame budget (with hysteresis), the canvas switches to a tile cache: 512 device px
    square tiles in a quadtree, prefetched by a margin plus pan velocity (Chromium's skewport idea), scale steps of at
    most 2× while pinching, snapping to an existing tiling within 1.2×, re-raster at the exact zoom when the gesture
    ends, a byte budget with LRU eviction. Tiles are rasterised by the same renderer. An item being dragged, edited or
    animated leaves the tiles and draws live, with the tiles split into below and above it while it does.
  - **Deliberate difference from Chromium:** a missing tile is filled by a coarser tile scaled up, with one whole-scene
    low-level tile always kept; it never shows as blank. A web page scrolls along one axis inside a 3000 px interest
    area and rarely lacks tiles; a canvas zooming out quickly lacks the whole screen.
  - Level of detail by projected size: under 0.5 px dropped into a per-tile average, 0.5–4 px a box of the item's
    average colour, paths simplified per level (Douglas–Peucker or Visvalingam).
  - Pixel snapping (ADR-0006) applies only where the accumulated transform is a translation plus the display scale;
    inside a zooming camera coordinates stay fractional (ADR-0020).
- **Removed.** tiny-skia, resvg on the draw path, `gpu/raster.rs`, `ShapeRaster` and the shape texture cache.
  `Renderer::capture` returns `gui`'s own image type (RGBA8, premultiplied, size, scale). `crates/shadcn/src/chart.rs`
  and `tooltip.rs` move to `PathData`.

## Consequences

Expected, to be verified:

- Paths cost GPU work proportional to their device size and segment count; a zoom step re-bins on the GPU with no CPU
  raster and no upload beyond one job per path.
- Quality equal to Chrome's analytic anti-aliasing or better (curves flattened at 1/8 px); shared edges between two
  paths show the same faint seam Chrome shows (conflation), not a defect against the reference.
- Every compute pass has a plain-Rust CPU version; tests compare each pass bit for bit and compare coverage with an
  exact `f64` polygon-pixel area oracle over hard paths (self-intersecting, nested even-odd, sub-pixel slivers, cusps,
  sharp miters, hairlines, huge paths clipped by the viewport).
- Verdict covers charts, Lucide icons at 16, 20 and 24 px and tooltip arrows; differences that are better are recorded
  in `verdict/better.tsv`.
- Text crossing 128 ppem, or ending a scale animation, switches between atlas and Slug; the same gamma on both keeps the
  weight jump small, and the verdict text page measures it.
- New dependencies: none for paths. hb-gpu's shader is MIT-style and its notice goes into `THIRD_PARTY.md`; vello's
  area formula is Apache-2.0/MIT.


## As built

Built on 2026-11-21 (M5a, tasks P8 and P9): paths, icons and curve text. The infinite canvas was built with the data
chunks of M6: see "As built: the canvas".

`crates/gui/src/shape.rs` (`Path`, `PathBuilder`, `PathStyle`, `Stroke`, `Dash`, `FillRule`), `svg.rs` (the icons'
SVG read into paths), `gpu/path.rs` (jobs, CPU bounds, reuse), `gpu/path_compute.wgsl` (the passes),
`gpu/path_render.wgsl` (tile coverage), `gpu/path/reference.rs` (the CPU reference and the `f64` oracle, test only),
`gpu/slug.rs` and `gpu/slug.wgsl` (curve text), `text/fonts.rs` (`Fonts::curves`), `gpu/tests.rs`.

- **Data.** `Shape` is a `Path` (an `Arc` of verbs, points, bounds, fill rule and a key) plus a `PathStyle` (fill,
  or stroke with width, cap, join, miter limit and dash; paint solid or linear gradient through the existing stop
  table). `Item::Path` replaces `Item::Shapes`; `crates/shadcn/src/chart.rs` and `tooltip.rs` build `Path`s.
- **Upper bounds on the CPU.** `gpu::path` bounds a job's lines from each curve's Wang constant and the matrix's
  largest scale (tolerance 1/8 device px, at most 4096 segments a curve), plus joins, caps and dots for strokes;
  its tile entries from each line bound's tile walk; its instances from its tile rectangle. It reserves those ranges
  in long-lived arenas, writes one job record and one work item per curve. Nothing is read back and nothing can
  overflow; unused slots are empty instances.
- **Seven compute passes** for dirty jobs only: `transform` (control points to device), `flatten` (fills to lines;
  strokes to their two offset sides, round joints inside a curve where it turns sharply), `joins` (joins, caps and
  dots), `count` (an exact integer tile walk: lines per tile and the winding each row-top crossing adds), `scan`
  (backdrop prefix per tile row, list room), `write` (lines into tile lists with their left-edge crossing) and
  `compact` (lists sorted by least x; one instance per tile with edges, one per run of solid tiles). Lines left of the
  job's rectangle are projected onto its left side, so their winding counts.
- **Fragment coverage** starts from the tile's backdrop, adds each line's left-edge crossing and its exact signed
  area right of it within the pixel, stops at the first line right of the pixel, applies non-zero or even-odd, and
  multiplies by paint, the window clip, a clip in the path's own space and alpha. Strokes thinner than a device pixel
  are drawn 1 px wide at alpha equal to their width, as Skia draws hairlines.
- **Reuse.** A job is binned in coordinates relative to the floor of its translation and kept while its path, style,
  linear part and fractional translation are the same and what is visible stays inside its binned rectangle (grown
  by 256 device px): a whole-pixel scroll or pan writes a new offset only.
- **Icons.** The Lucide SVGs are read by `svg.rs` (roxmltree for the XML, svgtypes for path data) into paths once.
  An icon drawn straight is rasterized once by the path rasterizer into an R8 page of the mono atlas (`fs_mask`) at
  its device size and tinted when drawn; an icon in a scaled or turned group is drawn as paths.
- **Removed.** tiny-skia and resvg as dependencies of `gui`, `gpu/raster.rs`, `ShapeRaster` and the shape texture
  cache. `Renderer::capture` returns `gui::Pixels` (RGBA8, premultiplied, size and scale).
- **Slug.** A port of hb-gpu's encoder (`slug::encode`) and fragment shader (`slug.wgsl`), banded and full-scan
  variants both built at start. A glyph's quadratics (cubics split to quadratics, unhinted, with the atlas's skew
  and emboldening, at 1024 curve px per em) are encoded once into a blob in a storage arena, keyed by glyph key
  (font, glyph, size, variations) and darkening, and dropped after 60 frames unused. Each glyph is one 80-byte
  instance (extents, a 2×3 map from curve px to the window, blob, gamma row, colour, two clips). The quad grows by a
  device pixel through the map's least singular value (the Jacobian's dilation). Coverage goes through the same gamma
  table as the atlas. A glyph goes to Slug above 128 device ppem or in a transformed group; colour glyphs stay in the
  atlas. The banded/full self-test renders one glyph both ways at first use (not at start: a window without large or
  turned text pays nothing) and uses the full scan if they differ.
- **Transformed groups** that only transform (opacity 1, no mask or filter) and hold only rects, rounded rects, bands,
  gradients, icons, paths and non-colour glyphs are drawn straight through their transform, with no layer: quads as
  paths, glyphs as curves, icons as paths. Other transformed groups still render into a layer and are composited
  through the transform.

| Kiểm (plan) | Result |
| --- | --- |
| P8: GPU coverage against the exact `f64` area | `paths_are_covered_within_one_level_of_the_exact_area`: ten hard paths (a self-intersecting star non-zero and even-odd, a sub-pixel sliver, a cubic with a cusp, an even-odd ring, a zigzag with sharp miters, round caps, square caps with bevels, a hairline, a circle far larger than the window) at a fractional origin; the CPU passes within 10⁻³ of the oracle, the GPU within 1/255 at every pixel |
| P8: whole-pixel pan reuses tiles | `a_whole_pixel_pan_reuses_the_tiles`: no job binned again, the same pixels moved; a fractional move bins once |
| P8: icons once, then as paths | `an_icon_is_drawn_from_the_atlas_and_as_paths_when_turned` |
| P8: tiny-skia, resvg, `ShapeRaster` gone from `Cargo.lock` | resvg, usvg and `ShapeRaster` gone on 2026-11-21; tiny-skia on 2026-11-22, with winit's `wayland-csd-adwaita` off (see "As built: window decorations"): `cargo tree -i tiny-skia --target all` finds nothing |
| P8: verdict `chart`, `spinner`, icons not lower | every chart, spinner and icon demo scores as before (ChartDemo is BETTER 67 as before; spinners 98 to 100) |
| P9: golden large and turned text | `large_text_is_drawn_from_its_curves` (Inter `g` at 200 ppem), `turned_text_is_drawn_from_its_curves` (56 ppem scaled 1.3 and turned 30°): every pixel more than 1.5 px from an edge is within 2/255 of what the glyph's own winding says (inside or out) |
| P9: banded equals full scan | `banded_curve_text_matches_the_full_scan_here`, and the self-test at first use |

Verdict over 389 demos: 99/100 (mean 98.55 → 98.56), the same 5 FAIL, no demo lower.

`docs --bench`, 85 pages, release, Apple Silicon, the build before M5a and after it run back to back twice (the
machine was shared; p95 FAILs are single spikes that move between pages from run to run):

| Median over pages | Before (run 1, run 2) | After (run 1, run 2) |
| --- | --- | --- |
| steady p50 | 3.22, 2.89 ms | 3.12, 2.90 ms |
| steady p95 | 5.20, 4.04 ms | 4.89, 4.08 ms |
| paint | 3.01, 2.75 ms | 2.90, 2.73 ms |
| allocations per frame | 472 | 458 |
| GPU images held | 5.3 MB | 6.6 MB (path and curve arenas, the mask page) |
| heap | 15.2, 15.4 MB | 16.3, 16.5 MB |
| pages over 16 ms p95 | 6, 0 | 4, 0 |

The plan's "no page over 4 ms p95" is not met, as before M5a: frames are still recorded and drawn whole; it is the
goal of retained chunks and damage (P2, P4). `stress --run`: all 12 scenes PASS.

Not met, and why:

- **Dashes are cut on the CPU**, in path space, by arc length, into a new `Path` kept by the path's key and the
  dash (dropped two frames after its last use); not on the GPU. A dash animation re-cuts on the CPU each frame; the
  binning still runs on the GPU.
- **Paint** is solid or linear only: radial, conic and image patterns, and blue-noise dithering of gradients, are not
  built (no element draws them on a path yet).
- **Ellipse, capsule and arc** are not recognised as closed forms; they are paths. Inside a directly drawn transformed
  group, rects and rounded rects are paths too (exact area, not the signed distance).
- **The 8×8 split** of tiles with more than 64 lines is not built; a dense tile loops over all its lines.
- **Stroke expansion** offsets each flattened segment's sides with round joints inside curves, not Levien–Uguray's
  Euler-spiral offsets; the error stays within the 1/8 px tolerance but costs more segments on wide curved strokes.
- **CPU references** are per algorithm, not per pass bit for bit: the tests compare the CPU passes' coverage with the
  oracle and the GPU's pixels with both; intermediate GPU buffers are not read back.
- **Grey bars below 3 ppem** belong to the canvas (M6).

## As built: the canvas

Built on 2026-11-21 (M6, tasks D8 and D9). `crates/gui/src/canvas/`: `mod.rs` (`Camera`, `WorldPath`, `World`,
`WorldSnapshot`, `canvas()`), `aabb.rs` (the dynamic AABB tree), `zkey.rs` (fractional z keys), `tile.rs` and
`boxes.rs` (the data chunk kinds tiles and small objects draw by), `plan.rs` (what a view of a world draws, as steps),
`planner.rs` (the raster workers), `tests.rs`; `gpu/world.rs` (drawing a canvas on the render thread: straight, from
tiles; the tile manager). `scene::Item::World(Arc<WorldView>)` carries a canvas as one
frame shows it.

- **Camera and world.** `Camera { center: (f64, f64), zoom: f64, rotation: f64, settled }` is a signal the element
  owns; `canvas(world, camera)` pans it on drag, zooms it about the pointer on the wheel, and marks it settled 150 ms
  after the last move. `rotated(angle, at, size)` turns it about a point, which stays under the pointer
  (`the_camera_keeps_the_point_under_the_pointer`); the world is drawn turned, tiles included
  (`a_turned_camera_turns_the_world`).
  `world()` is a scope-owned handle (`add`, `extend`, `set_path`, `raise_above`, `remove`, `hit`); a frame reads an
  immutable `WorldSnapshot` (objects, draw ranks, the tree, the extent), made once per version.
- **Precision.** Each object keeps its `f64` source and an anchor, its box's corner; its `f32` path holds only its own
  extent about the anchor, and one matrix a frame is computed in `f64` relative to the camera. An object larger than
  2¹⁸ px on screen is drawn from its source cut to around the view in `f64` (curves flattened in `f64`, pieces of curve
  whose control points lie outside the area replaced by their chord; fills clipped Sutherland–Hodgman, which keeps every
  winding number inside; strokes cut Liang–Barsky), about a point near the camera, made again when the camera moves a
  quarter of the view or zooms. At zoom 10⁹, 10³ units from the origin, a square and the edge of an object 10⁶ units
  wide move by 0.25 px for each quarter-pixel pan, within 0.01 px, and stand within 0.02 px of where they belong
  (`a_billionfold_zoom_moves_by_the_subpixel_it_was_panned`).
- **Culling, order, hit testing.** Box2D's `b2DynamicTree` in `f64` (fattened leaves, perimeter-cost insertion, AVL
  rotations; checked against a scan through moves and removals); keys between any two bytes strings, appended keys
  counting up as length-prefixed integers (four bytes after a million appends).
- **Level of detail.** Under 4 px an object is a box of its colour, and a run of boxes is one instanced draw
  (`canvas::boxes`, exact area coverage) that a path breaks only where it touches one of the run's boxes (a 64 × 64
  occupancy grid), so the order shows the same wherever objects overlap. Under 0.5 px it is not drawn on its own: its
  area in each device pixel, weighted by its alpha, gathers with the others' into the run (Datashader's aggregation), a
  box a pixel of their average colour and summed coverage, in pixel order so a plan repeats
  (`objects_too_small_to_draw_keep_their_tone`: squares of 0.4 px cover 16 % of every pixel, drawn straight and from
  tiles).
- **Simplification per level.** A path of 64 or more points keeps, per level of detail (`floor(log2 zoom)`), a copy
  simplified by Douglas–Peucker to an eighth of a pixel at that level's zoom; the last four levels are kept
  (`long_lines_are_simplified_per_level_of_detail`: a 4 001-point line keeps its corner and under ten points far out,
  every point close in, and draws as the whole line does).
- **Text.** `World::add_text(WorldText)` places a run of text: shaped once, at 1 024 px an em, through the page's text
  system (Parley, its fallback and bidi), into glyph keys and offsets. At 3 px an em and more each glyph draws from its
  Slug curves under the canvas's matrix, so it stays sharp at any zoom and turn; below, the run is a grey bar of its box
  (`text_in_a_canvas_is_drawn_from_its_curves_and_as_bars_far_out`). `WorldText::width(w)` wraps it to `w` world
  units: the text system breaks its lines as it breaks a UI paragraph to its box (the style's alignment, a word longer
  than the width overflowing it), the lines' boxes kept from its selection geometry; far out each line is its own bar
  (`text_in_a_canvas_wraps_to_its_width`: every line within the width, the same glyphs as on one line, every glyph from
  its curves straight and turned, a bar a line).
- **The live layer** (Figma's and tldraw's interaction layer). `World::lift(id)` takes an object out of what tiles
  draw; it is drawn each frame where it is (`translate`, `set_path` on a lifted object change only the live layer,
  never the tiles' version), in its place in the order; `land(id)` puts it back. The canvas draws live, straight and
  cut to them, a few disjoint device rectangles (at most 16, else the box around them): the lifted objects' ink and
  every change the tiles shown do not hold yet. Everything there is drawn, lifted objects merged in by their z keys,
  so an object dragged under a translucent one shows beneath it; the tiles are drawn around the rectangles, so nothing
  draws twice. `canvas_editor(world, camera, movable)` lifts the object a press lands on (if `movable` takes it, as a
  design tool skips locked ones), moves it with the pointer, lands it on release; elsewhere a drag pans.
- **Tiles invalidated per change** (Chromium's invalidation). Tiles are keyed by world and tiling, not version. Each
  change records its version and world box (the last 1 024 kept; a tile older than that is stale everywhere). A tile
  no change reached since its version is kept as it is; one a change reached is rastered again into a second texture,
  the old one showing until it is done. While a change is little to draw live (a quarter of the canvas, 2 048 objects)
  it is drawn live and finished tiles take their place one by one; a larger change shows the tiles of before until
  every stale tile shown is rastered anew, then all at once (`a_large_change_shows_all_at_once`: every frame is all
  before or all after). Lifting rasters again only the tiles under where the object was, landing only those where it
  lands; a drag rasters nothing (`a_dragged_object_draws_live_and_rasters_only_the_tiles_it_touched`: among 3 000
  objects, at most the four tiles it touched and the whole-world tile, every frame within a channel of drawing
  straight around it, the first frame after the lift included).
- **Snapshots in time independent of the world's size.** The objects a snapshot shares are pages of 1 024, copied on
  write; ranks rise with each object added on top and stay through moves (made again only when an object is raised);
  the AABB tree is a shared copy, objects added or moved since found by a scan, copied again past 512 of them. A drag
  among 100 000 objects makes each frame's snapshot in `O(lifted)`, and landing in `O(pages)`
  (`snapshots_find_what_a_scan_finds_through_edits`: 3 000 adds, moves, raises, removals and lifts, checked against a
  scan).
- **Tiles.** Past 4 096 visible objects the canvas draws from tiles: 256 device px, keyed by world, version, raster
  scale, turn, phase and place, rasterized by the path renderer into store textures and drawn as data chunk items. A
  tiling's grid is turned with the camera and offset by a sub-pixel phase so that, settled, tile corners fall on whole
  device pixels and a tile's pixels are those drawing straight gives (Chromium's raster translation): turned tiles
  match drawing straight within a channel (`turned_tiles_match_drawing_straight_once_settled`). The raster scale steps
  at most 2× toward the zoom while it moves, reuses a tiling within 1.2×, and is the exact zoom once settled. A
  whole-world tiling (a power-of-two scale fitting the extent in a tile) is always kept; a missing tile is filled by the
  coarser tilings drawn under the fine ones, so nothing shows blank (`a_fast_zoom_through_tiles_never_shows_one_blank`:
  13 frames zooming 1.4× each over 40 000 squares, every 128 px block inked, coarse fills seen, no frame past the
  raster budget, and once settled the tiles match drawing straight within a channel on average). Tiles not drawn for
  240 frames, or past 512 MiB the least recently drawn, are let go.
- **Raster scheduling, as Chromium's tile manager.** Each frame the render thread lists the tiles it wants in priority
  order: the whole-world tiling, then visible tiles nearest the centre, then a one-tile ring skewed toward where the
  camera pans (eight frames ahead). Planning a tile (culling, level of detail, cutting huge objects, the steps it draws)
  runs on raster worker threads (`gui-raster-N`, half the cores, one to four); each frame's list replaces what still
  waits, so a tile no longer wanted is never planned (`raster_workers_plan_the_most_urgent_first`; the test holds the
  workers while it asks, since on a loaded machine they could plan every tile of the first list before the second
  arrived: the test's timing assumption was wrong, not the ordering). Plans come back by
  channel and are rastered on the render thread within a budget of 2 048 cost units a frame (a small path is 8); a tile
  larger than a frame's budget is split across frames into the same texture (`tiles_rastered_across_frames_equal_tiles_
  rastered_at_once`: bit-identical). A new tiling shows only once all its visible tiles are ready; until then the old
  tiling stays on screen (Chromium's pending tree activation), so no frame waits for raster. While tiles are pending the
  compositor keeps drawing frames by itself. Headless runs (tests, `Scenario`) wait for the workers each frame, so a run
  repeats exactly. The one case that rasters synchronously is the first frame of a world, when nothing at all is ready.
- **Uploads.** Path data is written through a staging belt in coalesced spans (dirty draw and job slots only), and the
  per-job buffer clears are merged into ranges: before, each `write_buffer` made a staging buffer, and that cost most of
  a raster frame.

Differences from the decision:

- **Per-object anchors, not anchor cells.** An object's own corner is the best anchor there is: its `f32` path holds
  only its own extent, at no cost per cell; cells would have bounded precision by the cell's size instead.
- **No camera node in the transform tree.** The camera, rotation included, is the canvas element's signal and the canvas
  one item; a pan or turn writes the camera and redraws the canvas's item (with tiles: tile quads only).
- **Huge objects are cut in `f64`, not through a curve BVH.** A BVH would make the cut sublinear in the path's curves;
  the cut is linear, made once per quarter view.
- **The tiling is chosen by the visible count (4 096 objects), not by an estimated cost with hysteresis**, and it stays
  on while settled (tiles at the exact zoom), as Chromium's tiles do.
- **Settled tiles are rastered at the exact zoom**, not snapped to a tiling within 1.2× (the scale reuse is for a moving
  camera only): a settled view is as sharp as drawing straight, as Chromium's ideal-scale tiling is.
- **Planning moves to workers; GPU raster stays on the render thread**, split across frames by the budget. Chromium's
  GPU raster also runs on its GPU thread, not its workers; wgpu's queue has one submitter here.
- **Dropped objects average per pixel, not per tile.** A tile's one average would smear 256 px into one tone; per
  pixel keeps where they are at the same cost bound (a box a pixel, at most a tile's pixels).
- **Lifted objects are drawn live with everything in their rectangles, not between tiles split below and above
  them.** Split tiles would raster every visible tile twice when a drag starts (all below and all above) and keep two
  layers per lifted object; drawing the few objects that meet a lifted one's rectangle costs only those, is exact in
  order and in translucency, and needs no layer.
- **Not built**: a quad tree of tilings (the tiles are a map: lookups are by key, and the coarser fill is the
  whole-world tiling, so a tree would only order what a map already finds).

Measured on 2026-11-21 (`stress --run canvas`: 100 000 paths, 1920 × 1080; the machine shared, load average 35–38 for
both, before and after run back to back; best and median of the runs):

| | Before (raster in one frame) | After (scheduled raster) |
| --- | --- | --- |
| Slowest action | 408–441 ms | 34–82 ms |
| Drawing frames CPU p50 / p95 / max | 3.3 / 148–202 ms / — | 2.4 / 5.5–8.3 / 17–54 ms |
| Drawing frames GPU p50 / p95 / max | 9.9–10.3 / 315–328 / 351–506 ms | 0.17 / 1.0–1.5 / 2.4–3.5 ms |
| First frame | 678–996 ms | 66–120 ms |
| Path memory held | 668 MB | 155 MB |
| Heap kept per open and close cycle | 1.7 MB (FAIL) | 0 B |

| Goal | Measured | Met |
| --- | --- | --- |
| Canvas of 100 000 paths, zoom at 120 Hz | raster frames cost at most the budget; CPU p50 2.4 ms, p95 5.5–8.3 ms under load 37, and p50 2.05, p95 3.24, max 10.9 ms under load 17 (GPU p95 0.86 ms); the max is the loaded machine (frames of equal work range 2–17 ms) | p95 yes; max not shown on this machine |
| Zoom 10⁹ without jitter | within 0.02 px, steps within 0.01 px (test) | yes |
| Fast zoom shows no blank tile | yes (test) | yes |

Measured on 2026-11-22 (`stress --run canvas`, its `drag` section: a card dragged 60 frames among 100 000 paths at
1920 × 1080, after raster finished; load average 34, the machine shared): CPU p50 2.4 ms, p95 5.6 ms, max 7.8 ms; GPU
p50 0.39 ms, p95 0.53 ms, max 1.6 ms; 5 tiles rastered in all (the four the card touched and the whole-world tile at the
lift, spread over frames within the budget; the landing's likewise), none in the frames that only move it. Before, each
move was a new version: every visible tile rastered again; and in this round's first build, before snapshots were
made in time independent of the world's size, the landing frame took 7.4 ms rebuilding the 100 000-object snapshot.

| Goal | Measured | Met |
| --- | --- | --- |
| Drag one object among 100 000 paths at 120 Hz, CPU and GPU within 8.3 ms | CPU p95 5.6, max 7.8 ms; GPU max 1.6 ms, under load 34 | yes |
| Tiles rastered: only those touched | 5 (4 touched + the whole-world tile), 0 while moving | yes |

## As built: window decorations

Built on 2026-11-22. tiny-skia had stayed in `Cargo.lock` only through winit's Wayland client-side decorations
(`sctk-adwaita` draws GNOME's title bar on the CPU with it). winit is now built without `wayland-csd-adwaita`
(features `rwh_06`, `x11`, `wayland`, `wayland-dlopen`), and where the compositor leaves the decorations to the
client, `gui` draws its own with its GPU renderer: `crates/gui/src/window/decoration.rs`, wired in `window.rs` and
`app.rs`. tiny-skia, tiny-skia-path, sctk-adwaita and their font stack (ab_glyph, ttf-parser) left `Cargo.lock`;
`cargo tree -i tiny-skia --target all` and `grep tiny-skia Cargo.lock` find nothing.

- **Server side first, as GTK 4 and Chromium.** winit asks for server-side decorations (xdg-decoration) at creation
  and has the first configure before `create_window` returns. Refused, or without the protocol (GNOME), winit puts
  sctk's plain fallback frame around the surface, which makes the outer size larger than the inner: that is how the
  platform thread learns the compositor left the decorations to the client. It hides winit's frame
  (`set_decorations(false)`, which in client mode requests nothing) and tells the UI thread (`Chrome`) before the
  first frame. If the compositor takes them over later, winit reports the window decorated again and the bar goes.
  `GUI_DECORATIONS=client` draws them on any platform (to see them off Wayland), `server` never.
- **The bar is built from the elements an app is built of**: a 35 px bar (sctk-adwaita's header) with the title
  centred in bold 14 px, then minimize, maximize (restore when maximized; Lucide `minus`, `square` added, `copy`, `x`)
  and close as 24 px round buttons 13 px apart, Adwaita's colours as sctk-adwaita has them, light or dark from the
  window's theme, dimmed while the window is unfocused; a 1 px border while it floats, none maximized, no bar
  fullscreen. The app is laid out below it; it stays first in the tree, so its place does not change as the bar comes
  and goes. Buttons are buttons to a screen reader (named, pressable through AccessKit), not in the Tab order, as GTK's.
- **Input as GTK's header bar reads it** (`Caption`): a press that moves past 8 px (`gtk-dnd-drag-threshold`) asks the
  compositor to move the window (`drag_window`, `xdg_toplevel.move`), once a press; two presses within 400 ms and
  5 px maximize or restore it; a secondary click opens the compositor's window menu (`show_window_menu`). A press on a
  button is the button's. sctk-adwaita starts the move on the press itself; waiting for the threshold keeps a click
  a click.
- **Resize edges** are hit-tested on the UI thread before the app sees a press (`edge_at`): 5 px inside the window,
  16 px along an edge from a corner for the corner, as Chromium's `kResizeInsideBoundsSize` and
  `kResizeAreaCornerSize`; the edge's cursor shows over it; a press asks the compositor to resize from that edge
  (`drag_resize_window`). None while maximized, fullscreen or not resizable.
- **Checked headlessly**: `window::decoration::tests` (edges and corners to directions and cursors, none when
  maximized, fullscreen, fixed or server-decorated; double click, drag threshold and menu; a `Scenario` that lays the
  bar over an app and clicks, double-clicks, drags and right-clicks it into the actions the platform is asked for;
  without client decorations the app is the root). `cargo clippy -p gui --target x86_64-unknown-linux-gnu` builds the
  Wayland path. Not run on a real Wayland session.

Differences from GTK and sctk-adwaita:

- **No shadow margin, no rounded top corners.** winit 0.30 does not let the client set the window geometry apart
  from the surface (`xdg_surface.set_window_geometry`), so there is no invisible margin outside the window to resize
  from, as GTK has; the bands are inside the window, over the app's outermost 5 px, as Chromium's are without a
  shadow. Rounded corners would need the same margin and a transparent surface.
- **No tiled state.** winit reports maximized and fullscreen but not tiled edges, so a tiled window keeps its border
  and edges.
- **The button layout is fixed** (minimize, maximize, close at the end): GTK reads `gtk-decoration-layout` and the
  double-click action from its settings, which need D-Bus.
- **Light or dark from winit's theme only**: sctk-adwaita reads the portal's `color-scheme` over D-Bus; winit without
  it reports a theme only when the app sets one, so the bar is light unless the app asks for dark.
- **A compositor that refuses server-side decorations and has no subcompositor** gets neither winit's frame nor ours
  (winit then reports the window decorated); none known.

## Revisit when

- A GPU shows banded Slug or long tile loops miscompiled or slower than the full scan on a supported driver: change the
  default variant.
- Many paths sharing edges (maps, dense diagrams) show conflation seams that look worse than Chrome: add MSAA16 per
  tile only for tiles holding several paths with shared edges.
- The upper bounds reserve far more slots than paths use on real scenes and the memory shows in `Footprint`: tighten
  the bounds.
