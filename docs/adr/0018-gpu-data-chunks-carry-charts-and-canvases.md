# 0018. GPU data chunks carry charts and canvases

Status: accepted, 2026-11-21; built 2026-11-21 (M6, D1–D7, see "As built"; readback for user chunks built 2026-11-22; the chart goals measured met, the 4K
chart's CPU at the edge of its budget). Supersedes the research decision to drop GPU preprocessing and indirect
draws (`docs/research-2026-11-21.md` §6.3) for data-heavy content only; UI primitives keep the main pass of ADR-0016.
Replaces how `crates/shadcn/src/chart.rs` draws (shapes rasterised by tiny-skia, ADR-0011).

## Context

`AGENTS.md` sets the target: charts of millions of points and canvases with hundreds of thousands of items, panned
and zoomed at display rate (120 Hz). Through the normal path, every point becomes an instance encoded on the CPU and
uploaded each frame; one pan of a million-point series is over budget before anything is drawn.

GPU preprocessing (compute shaders that transform, cull and reduce data on the GPU) and indirect draws (draw counts
written by the GPU, never read back) were dropped for the UI because a UI frame has 10³–10⁴ instances and layout
already knows what is visible. Data-heavy content is the opposite case.

What exists (`research_notes/Hiệu năng cực hạn/gpu_charts.md`):

- **M4** (Jugel et al., [PVLDB 7(10), 2014](https://www.vldb.org/pvldb/vol7/p797-jugel.pdf)): keeping the first,
  last, minimum and maximum point of each pixel column draws a 1 px line with no wrong pixels. LTTB approximates and
  is sequential; MinMaxLTTB ([arXiv 2305.00332](https://arxiv.org/abs/2305.00332)) still approximates.
- **uPlot** decimates on the CPU per frame; **deck.gl** and regl draw instanced lines and emulate `f64`; **Datashader**
  aggregates to a pixel grid.
- **Bevy** culls with compute and builds indirect arguments (`references/bevy/crates/bevy_pbr/src/render/mesh_preprocess.wesl:132-160`,
  `build_indirect_params.wesl:61-131`), taking output slots with `atomicAdd` (`:372-376`), which loses order; its
  prefix sum (`allocate_uniforms.wesl:1-26`) is the pattern for an order-preserving compaction.
- **wgpu 30**: compute and `draw_indirect` on Vulkan, Metal (not Apple1/2) and DX12
  (`references/wgpu/wgpu-types/src/limits.rs:1170`); `MULTI_DRAW_INDIRECT_COUNT` is a stub on Metal
  (`wgpu-hal/src/metal/command.rs:1723`) and disabled by Bevy on DX12 for a wgpu bug
  (`bevy_render/src/render_phase/mod.rs:1202-1206`). Indirect validation is on by default in release builds; it makes
  bad arguments a no-op at the cost of one compute dispatch and one argument copy per batch, and turning it off is
  undefined behaviour on bad arguments and breaks `vertex_index` on DX12 (`wgpu-types/src/instance.rs:202-225`).
- **Line rendering**: instanced segments with a capsule distance (Inigo Quilez), joins as half planes; overlapping
  translucent segments darken unless drawn as one coverage (WebGL fundamentals, "don't blend a polygon that crosses
  itself").
- **Precision**: deck.gl's fp64 and Cesium's relative-to-eye split a double into high and low parts; `f64` in shaders
  exists only on Vulkan.
- **rerun** `re_renderer` keeps point and line data in GPU buffers built by builders
  (`references/rerun/crates/ui_tools/re_renderer/src/point_cloud_builder.rs`, `line_drawable_builder.rs`).

## Options

1. **Charts as ordinary chunks.** Every point an instance; per-frame cost proportional to the data.
2. **CPU decimation each frame, then ordinary chunks** (uPlot). Proportional to the data on every pan and zoom.
3. **A GPU data chunk**: data resident on the GPU, reduced by compute to what the screen can show, drawn in the main
   pass in painting order.

## Decision

Option 3.

- **The chunk.** In the scene a data chunk is one item: `ChunkId`, version, and small fixed `Params` (`Pod`,
  `PartialEq`). Its data lives in a `ChunkStore` of GPU buffers split into pages of at most 128 MiB, allocated from a
  page pool. In the main pass a chunk takes one place in painting order and carries `(transform_id, clip_id,
  effect_id)` like every chunk (ADR-0016), so rounded clips, transforms and damage apply unchanged.
- **Invalidation.** A new level `NeedsChunk` re-runs only that chunk's upload, compute and coverage: data appended or
  edited (`NeedsChunkData`, only when the new range meets the view) or the view changed (`NeedsChunkView`). It touches
  no layout and no other node's paint. The chunk's damage is its clipped bounds.
- **Series and decimation.**
  - x is stored as an `i64` split into two `u32`; the shader subtracts the view origin with carry before converting to
    `f32`, so precision is relative to the visible range, nanosecond timestamps included. y is `f32` relative to the
    series origin, with an optional high/low split per series.
  - Each series keeps a min/max pyramid on the GPU (64 children per node) and a matching one on the CPU. For line, area,
    bar and candlestick, a compute pass produces M4 per physical pixel column in O(64·log₆₄ n) per column, so per-frame
    cost follows the plot's width, not the point count. Reductions run in a fixed order inside a workgroup, so results
    are deterministic.
  - Streaming appends write only the tail with `write_buffer`; the pyramid is updated on the touched path. A very large
    first load is split across several submits so no single submission risks a GPU timeout.
  - NaN and gaps break a line; they never join across.
- **Marks.**
  - Lines are instanced capsule segments with analytic anti-aliasing (no MSAA), miter and bevel joins, dashes by arc
    length. Lines and areas draw into a per-chunk coverage texture with `Max` blending, then composite once, so joins and
    overlaps never darken, as Chrome fills one path.
  - Bars, candles, heatmaps (a texture) and opaque scatter draw directly in the main pass.
  - Scatter is culled to the viewport by compute and compacted with a stable prefix sum, never an atomic append, so
    overlapping translucent points draw in data order and an incremental frame equals a cold rebuild.
- **Indirect draws only where the GPU decides the count**: culled scatter and user chunks. Line, area, bar and candle
  instance counts are known to the CPU (two per column, or the points in view) and draw directly. wgpu's indirect
  validation stays on. `multi_draw_indirect_count` is not used.
- **Hover and picking on the CPU, immediately.** The CPU keeps the immutable source (`Arc`) that also feeds tooltips and
  AccessKit: binary search on x plus the CPU pyramid for series, a uniform spatial grid for scatter. No GPU id buffer is
  read back for charts; that would lag one or two frames. User chunks may read back asynchronously through a readback
  belt, delivered to the UI thread through `mailbox` as a status signal.
- **Custom chunks.**

  ```rust
  pub trait ChunkKind: 'static {
      type Params: Pod + PartialEq;
      fn pipelines(ctx: &mut PipelineBuilder) -> Self;                       // at renderer creation
      fn update(&mut self, ctx: &mut ChunkUpload, chunk: ChunkId, ops: &[DataOp]);
      fn compute(&self, pass: &mut ChunkCompute, chunk: ChunkId, params: &Self::Params, frame: &FrameInfo);
      fn draw(&self, pass: &mut ChunkDraw, chunk: ChunkId, params: &Self::Params);
      fn hit(&self, _chunk: ChunkId, _at: Point) -> Option<Hit> { None }
  }
  ```

  Kinds are registered with `renderer.register::<K>()` at startup. `ChunkCompute` and `ChunkDraw` allow only
  `dispatch`, `dispatch_indirect`, `draw` and `draw_indirect` on buffers issued by `ChunkStore`; a kind cannot open a
  pass or change the target, so it cannot break painting order or damage. `gui`'s own marks are `ChunkKind`s too: one
  code path.
- **Chart API.** `series()` returns a scope-owned handle with `append` and `set`; `plot().view(range).mark(line(s))`,
  `area`, `bar`, `candles`, `scatter`, `heatmap(grid)`, `rule`; `YAuto::VisibleMinMax` reads the visible range's
  min/max from the CPU pyramid in O(log n), so y follows a streaming pan. `crates/shadcn/src/chart.rs` is rewritten on
  `plot()`: container, tooltip, legend and series configuration stay ordinary UI; the `Shape` path is not kept.
- **Requirements.** Compute and indirect execution are required at startup (ADR-0016); there is no fallback renderer.

## Consequences

Expected, to be verified:

- Pan and zoom of a series cost about the same at 10⁶ and 10⁸ points; the frame is bounded by plot width.
- A chart's line looks as Chrome draws an SVG path of the same data: no darkened joins, no aliasing.
- Every compute pass has a CPU reference (M4 on the CPU pyramid, scatter bins and compaction); tests read back and
  compare bit for bit, and every scenario step (append, pan, zoom) checks the chunk against one rebuilt cold.
- Devtools report per-chunk compute time through timestamp queries (pass `timestamp_writes` on Metal, where
  `INSIDE_PASSES` is often missing).
- Indirect validation adds one small dispatch per indirect batch; charts use indirect only for scatter.
- Compute runs slowly on lavapipe; CI scenarios keep chart data sizes small, and large-data benches run on GPU
  machines.
- Coverage textures cost memory per visible line or area chunk; they come from the layer pool's byte budget.

## As built

Built on 2026-11-21 (M6, tasks D1–D7). `crates/gui/src/chunk/`: `mod.rs` (`ChunkKind`, `DataItem`, `Source`, `Picture`,
`Drawing`), `runtime.rs` (kinds, per-chunk slots, compute, coverage and main-pass draws, timestamps), `store.rs`
(buffers, textures and series pages, at most 128 MiB each), `series.rs` (pages and the 64-ary pyramid, the CPU's M4,
hover search), `kinds/` (lines and areas, bars, candles, scatter, heatmap, and their WGSL), `plot.rs` (`series()`,
`grid()`, `plot()`, the marks, hover), `tests.rs` and `golden/` (SVGs, the PNGs Chrome drew of them, `shoot.sh`).
`crates/shadcn/src/chart.rs` is rewritten on `plot()`.

- **The chunk.** `scene::Item::Data(Arc<DataItem>)`: a kind, a key, bounds, a region (its reach), `Words` params (64
  `u32`; bytemuck's derive is unusable under `forbid(unsafe_code)`, so params are word arrays each kind lays out) and its
  sources (`Source::Series`, `Source::Grid`, `Source::Bytes`). It is a `Seg::Item` of its paint chunk, drawn through no
  matrix (under a scale or turn its effect takes a layer), clipped by its chunk's device clip and faded by its
  effect's alpha. `NeedsChunk` is the element's own binding: `drawing(make)` is a tracked binding of layout
  (`DrawBinding`), so an append or a view change re-runs only that closure and records only that node's chunk
  (`plots::appending_records_the_plots_chunk_alone`: no render, one chunk recorded, under 4 KiB uploaded).
- **Kinds.** `ChunkKind` has `pipelines`, `update` (with `DataOp::Appended`, `Dropped`, `Replaced`), `compute` and
  `draw`; `gui`'s six (canvas tiles, canvas boxes, lines, bars, candles, scatter, heatmap) and `runtime::register::<K>()`
  for a user's. A kind only dispatches and draws on store buffers; it opens no pass.
- **Series.** Pages of up to 2²² points; x is `i64` split into `u32` halves, and `x64.wgsl` takes the exact signed
  64-bit difference from the view's origin before converting (an earlier borrow-based subtraction lost precision and
  culled 46 scatter points too many). Pyramid nodes keep min, max and their indices; the GPU and the CPU build them the
  same way. A page that grows is copied to its larger buffer on the GPU, so only new points are ever uploaded; a large
  first load is flushed every few MiB as its own submit. `Series::keeping(n)` is the ring buffer at page granularity:
  whole pages drop from the start, nothing is moved or renumbered (it keeps between `n` and `n` plus a page).
- **M4** per physical pixel column from the pyramid (`kinds/m4*.wgsl`), compacted with an ordered prefix sum; the
  column before and after the view are kept so the line enters and leaves the plot. Pages are bound side by side in
  gangs (as many as the device's storage bindings allow, up to 16; `page_w` picks one), one dispatch a gang, one
  thread a column a page meets: dispatches in a pass run one after another, so a dispatch a page (25 for 100 M
  points) left the GPU waiting on each (1.9 ms; ganged 0.45 ms). A workgroup a column (64-ary search) was tried and
  was slower: it read 64 probes a step where a binary search reads one. Bit-equal with the CPU on 4.5 M
  points with gaps, runs and jumps across four views (`m4_on_the_gpu_equals_the_cpu_bit_for_bit`).
- **Marks.** A line draws straight into the main pass (`kinds/lines.wgsl`): a compute pass lays out each segment
  (`line_segments.wgsl`: its ends, joins and box), then one quad a pixel column, as tall as the segments that meet
  it, whose fragments take the greatest exact box-filtered coverage of those segments, so joins and overlaps never
  darken (miter, round and bevel joins, linear or natural curves). Dashes (`Mark::dash(array, offset)`, up to 8
  lengths) take arc length from a segmented prefix sum that restarts at each break, as Skia restarts the pattern at
  each contour, and box-filter the pattern along the pixel (`line_dash` golden). Areas, to a floor or to another
  series, go into the chunk's coverage texture and are composited once under the line. The `rule` mark
  (`rule_x(&[i64])`, `rule_y(&[f64])`, colour, width, dash) is a stroked path in the plot's chunk, not a data chunk:
  a few lines need no pages (`rules_cross_the_plot_where_their_values_are_dashed_or_solid`); `y_auto` reaches its
  values; bars (rounded far corners), candles,
  heatmaps (OKLab mixing in the shader), scatter culled and compacted in data order with an indirect draw
  (`dots_are_culled_and_kept_in_order`). NaN breaks lines and areas (`a_gap_breaks_a_line`).
- **Goldens.** Each mark against the same data as an SVG drawn by Chrome: mean channel difference under 1 and under 1 %
  of pixels more than 32 apart. Exact equality is not the bar: Skia's coverage on curves and joins is less exact than
  ours (analytic capsules, exact area), and a pixel-perfect match would mean copying its error.
- **Hover.** `plot().on_hover(radius, f)` runs in the pointer handler (`on_pointer_move`) on the CPU, in the frame the
  pointer moved: binary search and the pyramid for lines, bars and candles; for scatter a 2D search through the pyramid
  (`nearest_within`), pruning nodes by their x span and min/max y. The nearest of the marks' candidates by screen
  distance wins (`hovering_finds_the_point_in_the_frame_the_pointer_moved`).
- **Chart API.** `series()`, `keeping(n)`, `precise()` (the high/low y split), `append`, `push`, `set`;
  `plot().x(..)`/`x_with(..)`, `y(..)`/`y_with(..)`/`y_auto(YAuto { zero, pad })` (the visible range's min and max from
  the CPU pyramid in O(log n)), `base`, `mark(line(s) | area(s) | bars(s) | candles(o, h, l, c) | scatter(s) |
  heatmap(g))`, `on_hover`. `chart_xy` in shadcn is a component: its setup makes one series per config entry and a
  `watch` reloads them; grid, ticks, tooltip and legend stay ordinary UI. The docs' chart demos score as before
  (ChartDemo 67, the four bar demos 100).
- **Timing** (`chunk/timing.rs`). Every pass writes a tagged pair of timestamps (path compute, chunk compute,
  coverage, main); two query sets take turns, and a set is resolved only after `on_submitted_work_done` says its
  frame finished: Metal samples a render pass's end when its fragments finish, which a resolve in the same command
  buffer overtook, so the first numbers (below in the M6 plan's first measurement) mixed stale ends in. `Drawn`
  reports the frame's span (`gpu`) and each kind's sum (`gpu_passes`); stress prints p50/p95/least and the per-pass
  p50.
- **Uploads.** Chunk buffer writes and the chunks' uniforms go through a `StagingBelt` into an upload encoder
  submitted ahead of the frame's commands (`Store::stage`, `take_uploads`); `queue.write_buffer` made a staging
  buffer a write, the largest CPU cost of a pan frame.
- **Damage.** A damaged part skips what a chunk painting all of it opaque hides (the app's root background hides the
  canvas background and everything under it), and that rectangle is written by `fs_fill`, unblended, not by the
  primitive shader: a 4K pan frame's main pass went from 2.4–3.0 ms to 0.8–1.2 ms.
- **Cold check** (`gpu/cold.rs`, `Runtime::cold_pairs`). With `GUI_COLD_CHECK` or `Scenario::check_cold`, every data
  chunk drawn in a frame is drawn again alone from the kept state and by a runtime that never saw it, and the two
  must be equal to the bit (`chunks_drawn_after_changes_equal_ones_built_cold`: append, pan, zoom in, zoom out past
  the data, dashed line on an area on another series, a dashed rule).

Differences from the decision:

- **No `hit` method on `ChunkKind`**: hover is the plot's, on the CPU, from the same snapshots the kinds draw; a user
  kind hit-tests in its own element's pointer handler. A trait method would have had to run on the render thread,
  away from the data's owner.
- **No uniform grid for scatter**: the pyramid already bounds y per x range, so a 2D search through it needs no second
  structure to build and keep in step with appends.
- **Readback for user chunks** (built 2026-11-22, `chunk/readback.rs`). A kind's compute calls
  `ChunkCompute::read_back(buffer, offset, size, tag)`; for an item made `reading_back(readback().to())` the bytes are
  copied after its pass into a mappable buffer of the belt (reused, at most eight kept), mapped after the submit, and
  sent when mapped, a frame or two later, to the UI thread through a `mailbox`, where they set the `Readback`'s status
  signal (`latest()`). Nothing waits for the GPU; while reads are out the renderer reports work pending, so frames go
  on until they arrive (`a_user_chunk_reads_back_what_its_compute_wrote`: nothing read for an item that takes no
  readbacks, the 64 words a compute wrote arrive with their tag, an unchanged chunk reads nothing again). A read
  follows the compute that wrote it: a kind reads what its data or view changing made.

Measured on 2026-11-21 (Apple M4, 10-core GPU, Metal, release, `stress --run chart100m stream`, three runs) on a
shared machine, load average 16–18 (other agents building); GPU times from the corrected timestamps above:

| Goal | Measured | Met |
| --- | --- | --- |
| Line chart, 100 M points, pan/zoom at 4K, GPU ≤ 2 ms | p50 1.17–1.44 ms, p95 1.76–1.80 ms, least 0.09 ms; p50 chunk compute 0.39–0.46, main passes 0.75–0.95 | yes |
| … CPU ≤ 0.5 ms | p50 0.48–0.52 ms, p95 0.88–1.12 ms (drag and wheel frames) | at the median, at the edge |
| Stream 1 M points/s at 120 Hz, only the new data uploaded | 208 KiB a 16 ms frame into a full 10 M window, the oldest dropped; CPU p50 0.45–0.46 ms, p95 0.61–0.74 ms; GPU p50 0.33–0.44 ms, p95 0.39–0.47 ms | yes |
| Hover in the same frame | yes (test) | yes |
| M4 bit-equal with the CPU | yes (test) | yes |
| Gaps and NaN break lines | yes (test) | yes |
| Incremental chunks equal cold ones | yes (test, `GUI_COLD_CHECK`) | yes |
| Chart verdict not lower | ChartDemo 67 (better), bar demos 100; whole gallery 99, no demo lower | yes |

Before this round (same scene, corrected timing, load 23–29): chart100m GPU p50 3.4–4.4 ms (compute 1.0, coverage
0.5, main 2.5–3.3), CPU p50 1.0–1.5 ms; stream GPU p50 0.85 ms, CPU p50 1.0 ms. What remains of the 4K pan frame's
GPU time is the fragment coverage of the line's strips (about 0.6 ms: each fragment takes several segments' exact
coverage) and the opaque fill of the 4K region; its CPU time is mostly wgpu's command encoding and Metal's.

## Revisit when

- A display scale change or a fractional plot origin makes M4 columns misalign with device pixels and the verdict
  shows wrong pixels: snap the plot's data rect to device pixels or re-bucket per scale.
- A series' y range loses precision visibly in `f32`: make the high/low split the default for that mark.
- A driver on a supported platform miscompiles the reduction or the prefix sum (CPU reference mismatch): switch that
  adapter to the full-scan variant or a fallback shader.
