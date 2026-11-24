# 0011. `gui` paints a scene on the GPU

Status: accepted, 2026-11-21 (the CPU renderer removed the same day). Supersedes how ADR-0003 and ADR-0008 paint; keeps what they decide. Its single-threaded window renderer and frame latency 2 superseded by ADR-0015; one pipeline, bytes without `bytemuck`, atlas doubling, layers every frame and `downlevel_defaults` with GL by ADR-0016; tiny-skia shapes, resvg icons and `tiny_skia::Pixmap` capture by ADR-0017.

## Context

`gui` painted every frame on the CPU: tiny-skia filled each path into a window-sized pixmap and softbuffer copied it to
the window. ADR-0008's bench showed the cost. With every cache in place, 16 of 85 pages were over the 16 ms budget.
The slow cases were those the CPU is bad at: groups compositing a window-sized layer (`dialog`, `sonner`), chart
paths (59 ms), and gradient masks (`skeleton`, `shimmer`). Every machine `gui` runs on has a GPU. The user asked to
paint on it "as GPUI does", keeping everything `gui` already does (the verdict, headless scenarios, the devtools,
groups, gradients, masks, filters).

GPUI (Zed, `references/zed`) splits a frame into a **scene** of primitives (quads, shadows, glyph and image sprites,
paths) and a renderer per platform. Its macOS renderer calls Metal directly, and its Linux, Windows and web renderer
uses wgpu. Both draw the same way. A quad's rounded corners, border and clip come from a signed distance in the
fragment shader, not from a rasterised path. A box shadow is a closed-form blurred rounded box (Raph Levien's).
Glyphs are rasterised once into atlas pages packed by etagere. Primitives of a kind are drawn in one instanced call.

## Options

1. **Metal directly, as GPUI on macOS.** It has the least overhead, but it runs only on macOS, it needs `unsafe`
   and Objective-C bindings throughout, and headless capture has to be written by hand.
2. **wgpu, as GPUI on Linux and Windows.** One safe API over Metal, Vulkan, GL and DX12. A UI frame is a few hundred
   draws, so the overhead is negligible. It renders to a texture and reads it back for headless frames.
3. **vello** (GPU compute vector rendering). It needs less shader code of our own, but it is heavier, and matching
   Chrome's pixels is harder to control.

## Decision

Option 2. Paint is split in two, as GPUI splits it.

- **The painter records a `scene::Scene`** (`paint.rs`, `effects.rs`): rectangles, rounded rectangles, bands (borders,
  rings), shadows, gradients, glyphs, icons, images, shapes, text showing a gradient through it, and groups (opacity,
  mask, filters, transform) holding their own items. Each item carries its clip (a rounded rectangle). Every decision
  of what to paint is made here, once.
- **A `Renderer` turns a scene into pixels.** `gpu::GpuRenderer` draws in a window, or headless with read-back. It
  is the only renderer. The former tiny-skia painter was first moved behind the same trait (on all 647 docs frames
  it gave the same bytes as before but one pixel, off by one), held the GPU to its pixels, then was removed with
  softbuffer.
- **The GPU draws every primitive with one pipeline** (`gpu/shaders.wgsl`). Each instance is a quad over the pixels it
  may touch. The fragment shader picks the primitive's kind and computes coverage: GPUI's rounded-box signed distance,
  exact area for axis-aligned rectangles, GPUI's blurred rounded box for shadows (cut out by the casting box, as CSS
  draws). It mixes gradients in OKLab per pixel and filters as the CPU does. One pipeline keeps the scene's painting
  order with one instanced draw per run of primitives that read the same textures. A frame is a handful of draws.
- **Rasters are made once and kept.** Glyph coverage goes to a one-channel atlas page; colour glyphs and icons go to
  an RGBA page (etagere, as GPUI). A full page first lets go of what the last two frames did not draw, then doubles
  (to 8192). Images and rasterised shapes are textures kept as long as they are drawn. Shapes are still rasterised by
  tiny-skia, once per shape and sub-pixel offset, wherever they are drawn.
- **Groups render into layer textures first.** A layer is sized to the group's region, rounded up to 128 pixels and
  pooled. The group's quad then samples it through the inverse transform, with the mask and filters. Text showing a
  gradient draws its glyphs into a layer and takes its alpha.
- **Blending is in sRGB-encoded values**, as tiny-skia and browsers blend. The surface is `Bgra8Unorm`, never an sRGB
  view.
- **Everything paints on the GPU, headless too.** A `Scenario` (shots, verdict, bench, devtools sessions,
  `snapshot`) draws on the GPU and reads back, in bands no taller than a texture may be. A measuring scenario only
  waits for the GPU. A live screenshot (`--attach screenshot`) reads the frame back from the GPU. With no adapter,
  a window or scenario fails to start (`ScenarioError::Gpu`); a machine without a GPU installs a software Vulkan
  driver (Mesa's lavapipe), which wgpu paints on through the same code. Rasters made once and then cached on the
  GPU are still made by the CPU: glyphs (swash), icons (resvg), shapes (tiny-skia), decoded images. GPUI makes its
  glyphs the same way.
- **The adapter uses least power** (`gpu::adapter_options`, M15). A machine with two GPUs draws the UI on the
  integrated one (`PowerPreference::LowPower`), as a UI's load wants. `WGPU_POWER_PREF` (wgpu's own variable:
  `high`, `low`, `none`) overrides it, for a window and headless alike.
- **`unsafe` stays forbidden.** wgpu's safe API is enough; instance data is written as bytes, without `bytemuck`.

## Consequences

- The verdict judges the GPU's pixels the same as the CPU's: 99/100 over 389 demos, and every demo has the same score
  under both renderers. All 647 docs frames match within anti-aliasing, except where an image had loaded by the time
  the GPU shot was taken and not by the CPU one.
- In a window (the docs' Sidebar page scrolled down and up, 120 frames, `--attach perf`): paint p50 4.05 → 0.47 ms,
  p95 4.23 → 0.86 ms. The whole frame's work (build, layout, paint) drops from 6.03 to 2.54 ms. Allocations per
  frame drop from 47.7 to 7.7 MB, and resident memory from 192 to 140 MB. Layout is now the largest stage.
  `Timings::vsync` holds the wait for the next drawable (13.7 ms at 60 Hz). That wait is the frame's pace, not work,
  so it is outside `paint` and `total`. Softbuffer had no such pace and painted frames the display never showed.
- Headless, `docs --bench` over the 85 pages, each frame waiting for the GPU: 17 pages fail the 16 ms budget on the
  CPU, 4 on the GPU (`chart` p95 30.5 → 6.7 ms, `shimmer` 46.8 → 9.4 ms, `sidebar` 32.6 → 10.3 ms, `dialog`
  21.0 → 5.8 ms). `calendar` and `code-block` still fail on layout (9–10 ms). The GPU's layer pool kept up to eight
  textures and grew after a second walk (`navigation-menu`, +5.9 MB). It now keeps what the last two frames used at
  once, as the CPU's pool does.
- Edges anti-aliased by area (a 1 px line at a fractional y) differ from tiny-skia's 4× supersampling by up to 30/255.
  Chrome judges both the same.
- CI has no GPU and runs the GPU renderer on lavapipe. There is no second renderer to hold the GPU to: the
  verdict against Chrome's pixels and `gpu::tests` (exact pixels of small scenes) are the reference.
- `Footprint`'s rasters are what the renderer holds: atlas pages and textures. Shadows are computed in the shader,
  so the shadow cache and its `Footprint` entry are gone.
- New dependencies: `wgpu` 30, `etagere` 0.3, `pollster` 0.4 (MIT/Apache-2.0). GPUI's signed-distance and shadow
  functions are Apache-2.0 (`THIRD_PARTY.md`).

## Revisit when

- Paths drawn every frame at new sub-pixel offsets churn the shape cache: tessellate them (lyon) and draw them on the
  GPU, as GPUI's path atlas does.
- A large image shrinks with aliasing: give image textures mipmaps.
- A machine shows wgpu's overhead in a profile: a Metal renderer beside it, as GPUI has, under the same scene.
