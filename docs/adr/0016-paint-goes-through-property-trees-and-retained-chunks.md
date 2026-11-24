# 0016. Paint goes through property trees and retained chunks

Status: accepted, 2026-11-21; device, pipelines, bind groups and atlases built 2026-11-21 (M5a, P3); property trees, retained chunks, damage on a persistent canvas and per-case layers built 2026-11-21 (M5b, P1, P2, P4, P5, see "As built"); the commit to a render thread built 2026-11-21 (M5c, P6, ADR-0015). Supersedes, in ADR-0011, "the GPU draws every primitive with one
pipeline", "instance data is written as bytes, without `bytemuck`", "a full page first lets go of what the last two
frames did not draw, then doubles", "groups render into layer textures first" (every frame, for every group), and the
renderer's `downlevel_defaults` limits with GL as a backend; in ADR-0003, "a group costs a clear, a copy and a
composite of its region per frame". Keeps the scene/renderer split, signed-distance quads, closed-form shadows, OKLab
gradients, sRGB-encoded blending, headless read-back and lavapipe in CI.

## Context

ADR-0011 moved paint to the GPU, but every frame is still rebuilt from nothing (`docs/research-2026-11-21.md` §6.1):

- `app.rs::draw_with` records a whole `Scene`; `GpuRenderer` encodes every item into a 256-byte `Prim` (16 `vec4`),
  writes all of them with `write_buffer`, and creates its bind groups anew (`gpu/mod.rs:509`, `:558`).
- Every group renders into a pooled layer texture every frame; the pool keeps eight textures whatever their age.
- The whole surface is cleared and redrawn on any change: a caret blink costs as much as a page change.
- Each glyph is a full 256-byte instance with its own copy of a 32-byte rounded clip.
- A full atlas page drops every placement and doubles, so every glyph is rasterised again.
- The device asks for `Limits::downlevel_defaults()` (`gpu/mod.rs:267`): 4 storage buffers per stage, 128 MiB per
  storage binding (`references/wgpu/wgpu-types/src/limits.rs:597-614`), too little for compute paths (ADR-0017) and
  data chunks (ADR-0018).
- The surface asks for `desired_maximum_frame_latency: 2` (`gpu/mod.rs:233`), one frame more than needed.

The target (`AGENTS.md`) is "a node and a phase": a signal read while painting or compositing a node invalidates only
that phase of that node. Scrolling, opacity and transform animation must not re-record or re-lay out anything.

How others retain paint:

- **GPUI** (`references/zed/crates/gpui/src`) replays a cached view's primitive range from the previous scene
  (`Scene::replay`, `scene.rs:227`; `view.cached`, `view.rs:46`), but copies the range into a new scene and uploads
  and draws everything again. It has no damage and no rounded clips.
- **Bevy** keeps extracted UI data per entity and re-extracts only changed nodes (`bevy_ui_render/src/lib.rs:417-790`)
  but rebuilds its vertex buffer every frame (`lib.rs:2319`); its sparse buffer uploads dirty elements and rewrites
  the whole buffer above 15% dirty (`render_resource/sparse_buffer_vec.rs:370-404`).
- **iced** re-prepares a `text::Cache` only when its version or transform changes (`wgpu/src/text.rs:30-245`) and
  groups damage rectangles while wasted area stays under 20 000 px² (`graphics/src/damage.rs:48-86`), but only its CPU
  backend uses damage.
- **Makepad** keeps draw lists on the GPU and skips a widget's draw code when its list is clean
  (`draw_list_2d.rs:93-150`); animations write instance fields in place (`area.rs:594-640`); scroll is a uniform.
- **vello** re-rasterises the whole scene on the GPU every frame in about 20 compute dispatches, and drops or corrupts
  a frame when a fixed bump buffer overflows (`research/.../config.rs:401-408`).
- **Chromium** keeps four property trees (transform, clip, effect, scroll); a paint chunk is a run of display items with
  the same property-tree state, and a layer is split off only when its state is expected to change on the compositor
  ([RenderingNG data structures](https://developer.chrome.com/docs/chromium/renderingng-data-structures)).
- **Flutter** has a third invalidation level, `markNeedsCompositedLayerUpdate`, that updates a layer's properties
  without repainting (`references/flutter/packages/flutter/lib/src/rendering/object.dart:3405-3425`).
- **Android** `RenderNode` changes alpha and translation without re-recording, and draws alpha without an offscreen
  buffer when content does not overlap (`setHasOverlappingRendering(false)`).

## Options

1. **Keep rebuilding each frame, faster.** Smaller instances and cached bind groups only. Work stays proportional to
   the page, not to the change.
2. **vello as the renderer.** Its compute renderer is research-grade and re-rasterises everything each frame; its
   sparse-strips renderer still panics on mask layers (`vello_gpu/src/lib.rs:70-75`) and keeps nothing across frames
   but atlases. Both draw rects, rounded rects and shadows through generic paths, which our closed-form quads beat.
3. **Retain at the scene level, GPUI's replay.** Saves recording, not upload or drawing, and needs views marked by
   hand.
4. **Property trees plus retained chunks, as Chromium and Makepad, with damage as iced.** Each node's paint is a chunk
   that lives on the GPU until the node's paint changes; transforms, clips, effects and scroll live in tables that a
   change writes once.

## Decision

Option 4.

- **Property trees.** Four tables owned by the renderer: transform (2×3, scroll offsets are translation nodes, the
  canvas camera of ADR-0017 is a node too), clip (rounded rect, deduplicated), effect (opacity, filters, blend, mask)
  and scroll (scroll chaining, offset relative to the virtual list's visible window). Each paint chunk carries
  `(transform_id, clip_id, effect_id)`; an instance references its chunk's state, never a baked position or clip.
- **Invalidation levels per node**, set by the phase that read the signal (ADR-0013's phase-tagged reads):
  `NeedsLayout` > `NeedsPaint` (re-record that node's chunk) > `NeedsProperty` (write one table entry, re-composite).
  ADR-0018 adds `NeedsChunk` for data chunks. A debug build reports a signal written during layout or paint after it
  was read in an earlier phase of the same frame.
- **Animated values are separate from layout values.** Each node has a transient translate, scale, rotate and opacity
  beside its laid-out box (Android's `setTranslationX` beside `setPosition`). Ending an animation never touches
  layout.
- **A layer is decided per case.** A group gets an offscreen texture only when its effect needs isolation (opacity over
  overlapping chunks, blur, blend, mask) and it is animating or costly to repaint. Otherwise opacity is multiplied into
  its instances; overlap is tested conservatively on chunk bounds. A layer's texture is kept while its subtree is
  clean, so a fading dialog costs one composite quad per frame.
- **Retained chunks.** Each chunk owns a stable range in the instance buffer, allocated from a free list. A clean chunk
  is replayed as it is; a clean repaint boundary skips its subtree walk. Dirty ranges are coalesced and written with
  `Queue::write_buffer_with` (`references/wgpu/wgpu/src/api/queue.rs:218`); above about 15% dirty the buffer is
  rewritten whole, as Bevy does.
- **Compact typed instances.** Glyph and icon instances of 32–48 bytes (position, atlas slot, colour, `clip_id`), quads
  of 64–96 bytes, path tiles (ADR-0017) and chunk draws (ADR-0018) each with their own layout. All are
  `#[repr(C)]` + `bytemuck::Pod` (a safe API; the `unsafe` stays inside the crate).
- **Clips.** An instance wholly inside its clip's inner rectangle (inset by the radii) drops the clip. A rectangular
  clip is clamped in the vertex shader (Makepad's `draw_quad.rs:21-37`); only a rounded clip is tested per fragment.
- **Pipelines and bind groups.** The main pass has several pipelines (UI quads, path tiles, Slug text, chart marks,
  LCD text), every variant built at renderer creation; none is built mid-frame (Impeller's rule). Bind groups are
  cached by (buffer, atlas generation, texture) and rebuilt only when one of them changes; redundant `set_bind_group`
  and `set_pipeline` calls are skipped.
- **Device.** The renderer requests the adapter's real limits for buffer size, storage binding size and storage
  buffers per stage, and requires compute shaders and indirect execution at startup. There is no OpenGL backend: a
  machine with neither Metal, Vulkan nor DX12 uses lavapipe. Data is still split into pages of at most 128 MiB.
- **Atlases.** Multi-page glyph and icon atlases (R8 coverage, RGBA8 colour, RGBA8 LCD per ADR-0019); a full page
  adds a page and evicts by age and frequency, never clears and doubles. Glyphs above 128 device ppem are not cached
  as bitmaps (glifo's cap, `references/vello/glifo/src/atlas/cache.rs:69`).
- **Damage.** The frame's damage is the union of the bounds of dirty chunks and of chunks under a changed property,
  each grown by its effects' reach (`effects::reach`), old ∪ new. Rectangles are merged while wasted area stays under
  iced's threshold, and into one beyond eight. They are redrawn into a persistent canvas texture with scissor and
  `LoadOp::Load`, culling items outside; the canvas is then blitted to the surface with a full-screen triangle (no
  `COPY_DST` on the surface needed). wgpu 30.0.2 has no damage present and no buffer age; when
  `Queue::present_with_damage` is released (`references/wgpu/CHANGELOG.md:179`) the damage is passed to it as a hint.
- **No submit when clean.** A frame with nothing dirty, no animation and no damage submits nothing.
- **Present.** `desired_maximum_frame_latency: 1` with `AutoVsync`; Metal then holds two drawables, the least
  `CAMetalLayer` allows (`references/wgpu/wgpu-types/src/surface.rs:856-866`).
- **Present only what changed (M15).** The canvas is blitted, and the frame presented, only when what the window
  shows changes:
  - the frame has damage;
  - an overlay's content changed (a content generation set by the compositor, not its presence), or an overlay was
    hidden (presented once);
  - the display does not show the canvas: the surface was configured, exposed, or shown again, or a present was
    owed because its texture could not be taken.

  A frame that changes nothing, an animation holding still included, presents nothing. One exception: on Wayland,
  after 8 frames in a row that present nothing, one presents unchanged, to ask for the frame callback that tells a
  hidden window apart (ADR-0015, as built, "Wayland", `WAYLAND_PROBE`). One drawable is held while
  idle: the texture taken at a frame's start, to wait for the vblank, is kept for the next frame that presents.
  It is never handed back, since Vulkan cannot hand one back (ADR-0015, as built).
- **The scene is immutable data.** A commit carries chunk updates, property updates, started and cancelled animations
  and damage; the render thread of ADR-0015 owns the trees, chunks and atlases and applies it.

## Consequences

Expected, to be verified by bench, stress and devtools before and after:

- A frame's CPU work and upload follow what changed: hover re-records one chunk; scroll, opacity and transform
  animation write table entries only; an idle window submits nothing.
- GPU fill follows damage: a caret blink redraws a few hundred pixels plus one blit.
- Text-heavy pages upload 5–8× fewer bytes per glyph; the clip table removes one 32-byte copy per instance.
- Old data cannot be drawn: a replayed chunk's state ids name live table entries, and every scenario step checks the
  retained frame against a cold rebuild, pixel for pixel.
- Groups lose their per-frame offscreen cost unless they truly need isolation; `navigation-menu` and `dialog` no
  longer render their subtree into a layer every frame.
- Machines with only OpenGL are not supported; CI keeps painting on lavapipe.
- `gpu::tests` grow: damage bounds of every effect, chunk replay after property writes, atlas eviction under pressure,
  a cold-equals-retained check per scenario step.


## As built

Built on 2026-11-21 in two parts: M5a (task P3) the device, pipelines, bind groups and atlases; M5b (tasks P1, P2,
P4, P5) property trees, retained chunks, damage on a persistent canvas and per-case layers. The render thread that
owns them and applies commits is M5c (P6, ADR-0015).

`crates/gui/src/gpu/mod.rs` (device, pipelines, bind group cache), `gpu/atlas.rs` (multi-page atlases),
`gpu/arena.rs` (ranges in the long-lived GPU buffers), `gpu/tests.rs`.

- **Device.** Backends are Vulkan, Metal and DX12 (intersected with `WGPU_BACKEND` when set); OpenGL is gone. The
  adapter must have `COMPUTE_SHADERS` and `INDIRECT_EXECUTION` or the renderer is not made; `required_limits` is the
  adapter's own `limits()`. `DUAL_SOURCE_BLENDING` is requested when the adapter has it (ADR-0019, P11).
- **Every pipeline is built when the renderer is made:** quads (the uber-shader), path tiles, the R8 mask pass that
  rasterizes icons into the atlas, Slug banded and full scan, and the seven path compute passes (ADR-0017). Each
  fragment entry has a dual-source and a single-source form; which is built follows the device. Nothing is built
  mid-frame.
- **Bind groups are cached.** Each holds the generations of the buffers and textures it binds (a buffer's generation
  goes up when it is made larger, an atlas's when it adds a page); a frame makes one again only when a generation
  differs. `GpuRenderer::bind_groups_made` counts them.
- **Atlases** (mono R8, colour RGBA8, LCD RGBA8) are texture arrays of 1024² pages, up to 8 each. A full atlas adds
  a page by copying the pages it has into a texture array one layer longer, on the GPU: nothing is rasterized again.
  At 8 pages it empties the page with the least `hits / (1 + age)`, never one drawn this frame, and forgets only that
  page's entries. Pages are never cleared together and never doubled. 1024² rather than 2048²: what the docs pages
  draw fits in one, a second page costs one GPU copy, and the stress `dashboard` scene holds 4.2 MB of glyph pages
  instead of 16.2 MB.

| Kiểm (plan, P3) | Result |
| --- | --- |
| No bind group is made in a steady frame | `a_steady_frame_makes_no_bind_group`: 0 after the first frame |
| A full atlas does not rasterize again | `a_full_atlas_adds_a_page_and_rasterizes_nothing_again`: 30 icons of 10 to 39 px fill several 128² pages, each rasterized once; three more frames rasterize none and give the same pixels |
| Eviction at the most pages | `an_atlas_at_its_most_pages_evicts_what_is_not_drawn` |

Not met in M5a: storage data is not split into 128 MiB pages (see below).

### M5b: property trees, retained chunks, damage, layers

`crates/gui/src/scene.rs` (chunks, `Properties`, `Drawn`), `paint.rs` (chunks kept per node), `gpu/chunks.rs`
(chunks kept on the GPU), `gpu/frame.rs` (the frame plan, layers, damage, passes), `gpu/record.rs` (typed records),
`gpu/words.rs` (the record arena and the property table), `gpu/damage.rs`, `gpu/shaders.wgsl`, `app.rs`
(`FrameReport::paint`, `laid_out`, `drawn`), `inspect.rs`; tests in `gpu/tests.rs` and `crates/gui/tests/retained.rs`.

- **Property trees (P1).** A `Scene` is paint chunks in painting order plus `Properties`: transforms (an offset in
  window pixels, scroll offsets and moving translations included, and an optional matrix for a scale or turn),
  clips (rounded rects, one per clipping node, shared by everything it clips) and effects (opacity, filters, mask,
  the transform and outer clip it applies, the window region it may paint). A chunk is a node's paint (shadows,
  background, border, ring, text, icon, paths, text input) or a scrollbar thumb, recorded in its node's paint space,
  with its `(transform, clip, effect)` state. There is no separate scroll table: a scroll offset is a transform
  entry, and scroll chaining stays in `scroll.rs`.
- **Invalidation levels.** The painter keeps each node's chunk with what it was recorded from (Blink's
  `StyleDifference` split): a node whose box size changed was laid out again (`NeedsLayout`); one whose paint
  changed is recorded again (`NeedsPaint`); one whose offset, opacity, transform or clip alone changed writes its
  property entries and records nothing (`NeedsProperty`). A chunk records what lies within 4000 px of its clip
  (Chromium's cull rect) and again only when its clip leaves that. `PaintStats` counts chunks recorded (and why) and
  moved.
- **Retained chunks on the GPU (P2).** A chunk is encoded once into typed records of `u32` words (`record.rs`: a
  glyph, icon or rectangle is 32 bytes, a rounded rectangle 48, a band 80, a shadow 84, an image 64; the 256-byte
  primitive is gone) in a `Words` arena kept across frames (a free list; a chunk recorded again to the same length
  keeps its span), cast with `bytemuck::cast_slice` (no derive, no `unsafe`). A write compares with the CPU mirror
  and marks only the words that differ; dirty spans are merged and, past 15% of what is in use, written as one.
  Records name a state slot; the property table (slots, transforms, clips, effects, each region rounded up to 64 or
  256 entries so an entry more moves no other) is uploaded where it differs. The vertex shader applies the slot's
  offset, clamps a quad to its clip's rectangle, and drops the clip when the quad lies inside its inner rectangle.
  Paths keep their tiles (ADR-0017) and Slug glyphs are issued per frame for the chunks drawn.
- **Persistent canvas and damage (P4).** The frame is drawn into a canvas texture kept across frames and blitted to
  the surface with a full-screen triangle. Each canvas entry (a chunk, a layer's composite) has a signature of what
  decides its pixels (chunk and its encoding, offset, clip, folded opacity, matrix, the entry before it) and a
  rectangle (its primitives' bounds, effect reach included); an entry that changed, appeared or went damages its
  old and new rectangles. Rectangles merge while the union wastes at most 20 000 px², into one past eight, and are
  redrawn with `LoadOp::Load`, one scissored part each, background first. An unchanged frame submits nothing and
  uploads nothing. The surface asks for `desired_maximum_frame_latency: 1`. A scene larger than the device's texture
  limit (a whole page captured at once) is drawn without a canvas.
- **Per-case layers (P5).** Each frame, per effect: a filter or a mask, or a scale or turn over what cannot be drawn
  through a matrix (a shadow, an image, a colour glyph, text-through), gets a layer; so does an opacity below one
  over primitives that overlap (a sweep over the primitives' rectangles, touching is not overlapping, cached while
  the chunks and their offsets are the same). Otherwise the opacity is folded into the property table's effect entry
  and the primitives draw straight, a transformed one through its matrix. A layer's texture is kept, keyed by effect
  and the signature of its content, while the effect was layered in the last frame, younger than 120 frames and
  within 64 MB, so a fade or move of a layered group composites it again and draws none of it. `Drawn` counts layers
  drawn, kept and folded.
- **Devtools.** `FrameReport` carries `paint` (chunks recorded, why, moved), `laid_out` (layout placements) and
  `drawn` (submitted, chunks encoded, bytes uploaded, damage pixels and rectangles, layers drawn, kept, folded);
  `inspect`'s report prints them, and `held` adds the chunks and the canvas.

| Goal (plan, M5) | Result |
| --- | --- |
| An unchanged frame submits nothing | `an_unchanged_scene_submits_nothing` (GPU) and `an_unchanged_frame_submits_nothing` (app): not submitted, 0 B uploaded, 0 px damage |
| A hover re-records one chunk, uploads ≤ 1 KB, damages the button and its shadow's reach | `a_hover_redraws_its_chunk_and_its_shadow_alone`: 1 chunk recorded, 1 encoded, 36 B uploaded, 1 damage rectangle of 29 044 px (the 240×72 button plus `shadow-md`'s reach) |
| Scroll, opacity and transform animation: 0 layouts, 0 chunks recorded again | `a_scroll_…`, `a_fade_…`, `a_move_records_nothing_again_and_lays_out_nothing`: every frame 0 recorded, 0 laid out, drawn; the scroll encodes no chunk |
| Golden damage; the canvas stays whole | `damage_is_what_changed_and_the_canvas_stays_whole`: a recolour damages 32×12 px, a move 122×22 px in one rectangle, and the canvas then equals a whole redraw pixel for pixel |
| Upload counter | `uploads_are_what_changed`: one of 200 chunks recoloured, ≤ 64 B; a move, no chunk encoded, ≤ 64 B |
| Layer counter, folded alpha | `a_fade_is_folded_unless_what_it_holds_overlaps`: apart, folded with no layer; overlapping, one layer, kept while moved and faded |
| docs `--bench`: no page over 4 ms p95 | Before (the M5a build, run just before): p95 median 3.85 ms, worst 10.90 ms (`code-block`), 35 pages over 4 ms. After, two full runs: median 0.10 / 0.11 ms, worst `code-block` 4.52 / 3.95 ms; `code-block` alone, three runs, 2.42–2.63 ms p95 (p50 1.6–1.8 ms: layout 1.1–1.3 ms, paint 0.5 ms) |

Verdict: 99/100 over 389 demos, the same 5 FAIL; against the same reference shots the M5a build and this one give
the same score and the same differing pixel share on every demo. `stress --run` passes; `GUI_COLD_CHECK=1` passes on
the docs monkey (seeds 1, 3, 7) and on every stress scene. Held memory grows by the canvas (the bench pages' textures
6.7 → 16.1 MB at the shots' 2× size).

Not met:

- **4 ms p95 on every page holds with almost no margin on `code-block`.** One of two full runs had its p95 at
  4.52 ms. What is left of its steady frame is not paint (0.5 ms) but the retained layout pass over 1 298 nodes
  (1.1–1.3 ms, M4's engine), and the tail is single slow frames in a long run. A bench frame lays out and paints the
  whole page by design (it is "a frame while something moves"); skipping a clean tree's layout and paint walk is
  the repaint-boundary skip below.
- **A clean repaint boundary does not skip its subtree walk.** Built later (ADR-0013 C4, below).
- **A debug check that a signal is written in layout or paint after an earlier phase read it**, and a per-step
  scenario check of retained paint against a cold recording: built later (ADR-0013 C4, below). The
  canvas-equals-whole-redraw check is per test, not per scenario step.

### Clean subtrees skipped (ADR-0013 C4, 2026-11-21)

- `Painter` keeps the scene it last recorded and, per frame node, its property state, subtree end, chunk and thumb
  slots, whether its effect's region is its subtree's, and its hit box. `record` reads the frame's change log
  (`Frame::changes`) and records in place only the logged nodes, the subtrees whose property state moved with
  them, the groups whose region holds them and the inputs whose caret blinked; property writes go under an undo
  log, and a structural change (a chunk appearing or going, a thumb or blinker added, a frame rebuilt) falls back
  to the whole record, which compares with the scene as committed. The commit is built from what was written
  (`Commit::changed`), and the compositor's tables are rebuilt only when a visited node's scroll, motion or hit box
  changed. `PaintStats::visited` counts the nodes visited.
- Layout leaves hold only what measures (`Leaf::measuring`): a recolour, an underline or a run's background no
  longer lays the text out again.
- `GUI_COLD_CHECK=1` compares, each frame, the scene with one a fresh painter records from the same frame: per
  chunk, the items the window shows and its transform, clip and effect read as values.

| Measure | Before | After |
| --- | --- | --- |
| Nodes the painter visits, steady frame of stress `grid` (5 007 nodes) | 5 007 | 0 |
| A colour binding on one of 40 rows (`a_paint_binding_records_its_chunk_alone`) | every node visited | 1 visited, 1 recorded, 0 layout misses |
| A hover between two `grid` buttons | 5 007 visited | 4 visited, 4 recorded (two buttons and their labels), 0 layout misses |
| `GUI_COLD_CHECK=1` docs monkey (seeds 1, 3, 7, 300 steps) | frame nodes only | frame, scene: 0 differences |

- **`Queue::write_buffer_with`** is not used: the arena keeps a CPU mirror to diff against, so dirty spans are
  written from it with `write_buffer`.
- **Storage data is not split into 128 MiB pages** (no buffer is near that size before data chunks, ADR-0018).
- **The commit to a render thread** that owns the trees, chunks and atlases is M5c (P6).

### The render side plans from what changed (2026-11-21)

Before this, every frame the compositor composed the whole shown scene and `gpu/frame.rs` re-signed every chunk and
wrote every property table: about 1.5 ms of a steady `grid` frame, with nothing changed.

- The compositor hands the renderer a `scene::Changed` with the scene: the chunks, transforms, clips and effects
  whose shown value moved, or `all`. `Compositor::apply` records what a commit wrote; a structural change (a full
  commit, a new order, size, scale or background, a property whose parent, transform or clip changed) marks `all`.
  `compose` composes only the subtrees of transforms displaced now or last frame (scroll, fling, motion) and the
  entries commits wrote, expanding to descendants when a matrix changes (`compositor/compose.rs`); the links
  (children, clips and effects by transform) are rebuilt only on such a reshape.
- The renderer keeps its plan across frames (`Plan`): per chunk its place (root, layer member, skipped), and indexes
  of chunks by transform, clip and effect, and the chunks that show a canvas with its revision. `replan` re-signs
  only the chunks those indexes reach, recomposites only the layers touched, and plans whole when an effect gains or
  loses its layer or its skip, a route's target changes, or a chunk's id or state changes. A frame where nothing
  changed signs nothing and submits nothing.
- The property tables persist on the render side (`Tables`): each changed entry is patched in place and only the
  patched spans are uploaded (`Table::patch`, `flush`); the tables are laid out again only when their shape changes.
- Atlas pages are pinned by the chunks kept that draw from them (`Page::readers`), and images by a count per chunk,
  instead of being touched by every chunk each frame; a page is evicted only with no reader. Chunks no longer drawn
  are dropped by the sweep of a whole plan; between sweeps only transient overlay chunks go.
- `GUI_COLD_CHECK=1` (or `check_cold`) now also composes whole in the compositor and plans whole in the renderer
  each frame, and asserts both equal the incremental result (properties; fx, routes, root, shown, folds, tables).
- `Drawn` reports `signed` (chunks re-signed) and `planned_whole`; `tests/retained.rs` asserts 0 signed on an
  unchanged frame and a fade, 1 on a hover, 2 on a move, all without a whole plan.

| Measure | Before | After |
| --- | --- | --- |
| stress `grid` steady frame (shared machine, load 17 / 44) | p50 0.85 ms, p95 3.08 ms, paint 0.85 ms, 67 allocations (1.3 MB) | p50 0.00 ms, p95 0.00–0.03 ms, paint 0.00 ms, 17 allocations (800 B) |
| docs `--bench`, steady p95 median / worst page | 0.02 ms / 0.31 ms (`code-block`) | 0.00 ms / 0.14 ms (`spinner`) |
| docs `--bench`, instructions per steady frame | `calendar` 1.08 M, `code-block` 1.41 M | `calendar` 0.04 M, `code-block` 0.07 M; 24 pages faster, none slower |
| `GUI_COLD_CHECK=1` docs monkey (seeds 1, 3, 7 × 300), `cargo test --workspace` | — | 0 differences |

## Revisit when

- A scene's damage regularly covers most of the window (full-screen video, a canvas pan) and the extra blit shows in a
  GPU profile: draw such frames straight to the surface.
- Measured overlap tests send groups to layers that did not need isolation, or miss ones that did (a visible
  difference from Chrome): change the test.
- wgpu ships buffer age or partial present on Metal and DX12 with correct results on our CI drivers: draw damage
  straight into the swapchain.
