# 0008. `gui` measures every frame and holds what the screen shows

Status: accepted, 2026-11-20; "an effect depends on what its last run read" superseded by ADR-0012, the layout rebuilt each frame by ADR-0014, the process-only heap count and time-only bench comparison by ADR-0023. Extends ADR-0006 (the frame pipeline's caches) and ADR-0007 (what moves).

## Context

`gui` reported per frame what it did (causes, signals written and read, build, layout and paint times, the layout diff)
but only into a headless scenario's files; the window threw the report away. Nothing timed a page over many frames,
nothing held a frame budget, and nothing counted memory. Reading the code found what that let through:

- **Caches with no bound.** Icons and shadows were cached per colour forever; a button fading on hover added one image per
  frame of the fade. Glyphs were cleared wholesale at 20 000.
- **Dependencies only added.** An effect stayed subscribed to every signal any of its runs had read, so it woke for signals
  its last run no longer read; a dropped effect's id stayed in a signal's subscribers until that signal was next written.

Measuring then found where frames go (a release build, 960×640 at scale 2, `docs --bench` over the 85 gallery pages):
paint was most of every slow frame. A shadow was one image the size of its box with the box cut out, stored and blended
whole every frame: 30 input and card shadows on `form_tanstack` held 58 MB and its frame took 31 ms. An image was
sampled from its full-size pixels through a bilinear pattern every frame (`aspect-ratio`: 26 ms for 5 nodes).

## Options

1. **An external profiler and allocator per investigation** (Instruments, `dhat`, `samply`). Deep, but nothing is
   recorded with the frame, nothing is judged, and a regression is found only by whoever looks again.
2. **Measurement in the frame report, judged by a bench.** Every `FrameReport` carries what the app holds; a bench
   runs every page and fails a page over budget or that grows; the window shows the same numbers live.
3. **Hard caps on each cache** (an LRU of N entries). Bounds memory, but N is a guess, and an entry the screen still shows
   can be evicted and remade every frame.

## Decision

Option 2, and caches bounded by what the screen shows, not by a count.

- **What a frame holds is part of its report.** `inspect::Footprint` (nodes, shaped paragraphs, glyph, icon, shadow,
  scaled-image and layer images with their bytes, the reactive `Graph`, the heap) rides on every `FrameReport` beside
  `Timings`. `inspect::Spread` is the one percentile rule (nearest rank) for times and counts alike.
- **The heap is counted by the binary that wants it.** `gui::heap::COUNTING` (stats_alloc over the system allocator,
  since the workspace forbids `unsafe`) is installed with one `#[global_allocator]` line; the docs example installs it. `heap::now()` is `None` when it is not installed. The counts are the process's, every thread's.
- **A cache keeps what the last two frames drew.** `cache::FrameCache` stamps an entry with the frame that used it and
  drops it two frames later; shaped text, glyphs, icons, shadows and scaled images all use it, so what they hold follows
  the screen whatever a transition passes through.
- **An effect depends on what its last run read.** A source (`Signal`, `Computed`) records its readers; a reader
  (`EffectInner`, a computed value's bridge) records its sources, forgets them before it runs again, and forgets them
  when dropped. `reactive::graph()` counts signals, computed values, effects and subscriptions.
- **A shadow keeps its bands.** `ShadowImage` holds the parts of the blurred image around the cut-out box's clear middle
  (the box inset by its largest corner radius and a pixel), so a card's shadow is a ring in memory and in paint; the
  pixels are the same.
- **An image is scaled once per size.** `paint_image` samples the decoded image bilinearly into a pixmap of the box's
  physical size once and draws that pixel-aligned (`Nearest`), the corners still rounded by the path.
- **`docs --bench`** times every page's script and three rounds of `--frames` steady frames, fails a page whose steady 95th percentile
  passes `--budget` (16 ms), and walks the pointer over up to 24 pointer targets twice: after the second walk the page
  must hold what it held after the first (within 256 KB), unless the walk left it showing other nodes.
- **Headless first.** `crates/shadcn/bench.sh` builds release, runs the bench, writes `bench.json` (a line per page)
  and compares with the run before it (`--baseline`): a page whose fastest of three rounds moved past 40% and 1 ms is
  `SLOWER` or `FASTER`, a slower one measured twice more before it fails the run, so a change's cost is proved
  headless, by whoever made it. Unchanged code moved up to 35% between runs on an Apple silicon Mac mini (a whole run landing on efficiency cores), which
  sets the threshold. `docs --shots` and an app's `Scenario` trace carry every frame's footprint in their reports.
- **The window shows it live.** F10 (or `DebugTools.hud`, `--hud`) paints the HUD: the frame and its stages, the last 120
  frames' 95th percentile, frames per second, and the footprint's two lines.

## Consequences

- The numbers (before → after this change, steady p50): `card` 14.8 → 4.7 ms, `command` 22.9 → 7.8 ms, `form_tanstack`
  30.8 → 10.2 ms with its images 58.5 → 10.6 MB, `aspect-ratio` 25.6 → 11.4 ms; over the 85 pages the steady p95's
  median 7.88 → 6.64 ms and the pages over budget 23 → 16. No page grows after a second walk. The verdict is unchanged
  (99/100 over 389 demos, the same two FAILs).
- The bench still fails pages on paint: what paints as a group composites a window-sized layer (`dialog`, `sonner`), chart
  paths (`chart`, 59 ms) and gradient masks (`skeleton`, `shimmer`) are drawn through tiny-skia's high-precision pipeline
  at full window size. Those are the next work, each measured by the bench.
- The counting allocator adds an atomic add per allocation to the binaries that install it.
- A cache entry not drawn for two frames is made again when it returns (a tab shown again re-rasterises its icons).

## Revisit when

- A frame budget belongs on a release gate: the bench runs in CI on a fixed machine, its numbers kept per commit.
- The noise threshold hides a regression an agent needs to see: pin the bench to performance cores (a QoS class
  needs `unsafe`, which the workspace forbids) or count instructions instead of time.
- Paint moves to the GPU, or tiny-skia gains a fast path for pixel-aligned pattern draws: the bands and the scaled-image
  cache may then go.
- The process runs work on other threads (a network client beside the UI): the heap counts are then the process's,
  not the UI's, and a per-thread count is needed.
