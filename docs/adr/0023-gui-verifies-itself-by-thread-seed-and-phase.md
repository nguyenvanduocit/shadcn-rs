# 0023. `gui` verifies itself by thread, seed and phase

Status: accepted, built 2026-11-21 (M9); CI steps written, not yet run on a runner. Supersedes ADR-0008's "the counts
are the process's" (the heap is counted per thread too) and its time-only bench comparison; extends ADR-0010's
command vocabulary.

## Context

ADR-0008 counted the heap for the whole process and compared benches by time; ADR-0010 gave the devtools a vocabulary
to read a frame. The architecture plan (`docs/plan-2026-11-21.md`, M9) needs more of both, because what it builds
next runs on several threads and is judged by budgets:

- Three threads (ADR-0015), a layout pool (ADR-0014) and threads that feed the UI (mailboxes) all allocate. A process
  count of "allocations this frame" mixes the network thread's work into the UI thread's budget, and a budget cannot
  be set on it.
- Time moves 35% between runs of unchanged code on an Apple silicon laptop (ADR-0008), so the bench can only see a
  change past 40%.
- Nothing ran work in a random order on purpose: a mailbox delivering between two sends, a timer between two
  messages, were tested only in the order a test happened to write.
- The devtools said what a frame cost, not why: which phase did the work, where paint recorded again, which frames
  hung, who holds the memory.

## Options

1. **External tools per question** (Instruments, `perf stat`, `dhat`, Chrome-style tracing). Deep, but none of it is
   recorded with a frame or judged in CI.
2. **Our own counting `GlobalAlloc`.** Exactly what we want, but implementing `GlobalAlloc` needs `unsafe`, which the
   workspace forbids.
3. **Counters from a dependency, budgets and seeds in our code.** A counting allocator crate that keeps per-thread
   counters, instructions from what the OS gives a process without privileges, and everything else (budgets,
   seeded scheduling, random trees, phase counters, the hang log, paint flashing, the memory report) in `gui`.

## Decision

Option 3.

- **The heap, per process and per thread (I1).** `gui::heap::COUNTING` is `smelt-perf`'s counting wrapper over the
  system allocator (MIT, no dependencies): process atomics (allocations, bytes, bytes held, high-water mark) and
  per-thread `thread_local` cells (allocations and bytes), read by `heap::now()`, `heap::thread()` and
  `heap::measure(f)`. A binary installs it with `static HEAP: gui::heap::Counting = gui::heap::COUNTING;`. Every
  `FrameReport` carries `footprint.thread`, what the thread that built the frame allocated while building it.
  Considered and rejected: `stats_alloc` (process only), `alloc_counter` (thread counts only, no bytes, no process
  totals), `alloc-metrics` (net counts only), `alloc_tracker` (counters private behind sessions that allocate to
  read), `tracking-allocator` (MPL-2.0, outside the licence list), `allocation-counter` (installs a global allocator
  from a library), jemalloc's per-thread stats (a C build, no Windows MSVC, bytes only). `dhat` remains for
  diagnosis: a binary installs `dhat::Alloc` instead, and these counters then read `None`.
- **Budgets per scene (I1).** Each stress scene has a `Budget`: the 95th percentile of a steady frame's allocations on
  the UI thread, and the most any script step's frames allocate together. `stress --run` fails a scene over either;
  CI runs `stress --run --cycles=0 --monkey=0` on a release build, and `tests/reactive_alloc.rs` holds a steady
  reactive update at zero allocations on its thread.
- **Phase counters, paint flashing, the hang log, the memory report (I2).** `FrameReport.layout` is layout's own
  `Stats` over the frame's passes (calls, cache misses, measure calls, containers placed, parallel jobs), replacing
  the single `laid_out` count; `FrameReport.painted` lists where paint recorded a chunk again or only moved, faded or
  clipped one it kept (`Painter::painted`). New commands: `counters [N]` (renders and bindings, layout, text shaped,
  chunks recorded and moved, GPU encoded, bytes uploaded and damage, allocations on the thread and in the process,
  per frame and their spread), `paints` (the regions and their area), `hangs` (every frame over 50 ms, Chrome's long
  task, since the log started: when, its stages, the slowest, why, what each phase did; the last 64 kept past the
  120-frame log), and `memory` gains the thread's allocations, the heap's peak and a report by owner: frame, layout
  tree, paint chunks, reactive arena, text, the devtools' own log, each counting its containers at capacity, the heap
  left unattributed, then the GPU's atlases, images, paths, layers and chunk buffers exactly. The F12 overlay flashes
  what paint recorded again (green) and what it only re-composited (teal); the F10 HUD draws a bar per frame of the
  last 120 straight into its scene (green within 16.7 ms, orange within two frames, red past), with a line at the
  budget. The commands' text is held by tests.
- **Seeds (I3).** `gui::testing`, after GPUI's `seeds.rs` and test dispatcher: `seeds(iterations, explicit)` and
  `run_seeded` (`SEED`, `ITERATIONS`; a failure prints the seed to run alone); `Rng` (xorshift64 seeded through
  splitmix64, so seed 0 walks too; the devtools' `monkey` uses it); `Scheduler`, tasks run one at a time in an order
  the seed picks, on a fake clock that is `gui`'s scripted clock, so `gui`'s timers fire among the tasks at their
  times (`spawn`, `spawn_after`, `random_delay`, `tick`, `run_until_parked` with a livelock limit, `advance_clock`,
  `advance_to_next`, `run_to_end`, and a log the same seed repeats); `RandomTree`, a seeded keyed element tree in
  GPUI's four shapes (wide, deep, mixed, narrow; nesting capped at 48) with seeded edits (add, remove a subtree,
  retext, restyle). Tests: producers sending through a mailbox with random delays and deliveries at random points,
  glitch-free and whole for every seed; random trees edited step by step, every frame equal to a cold rebuild. The
  stress scene `random` benches a 3 000-node tree edited 20 nodes at a time.
- **Layout from Chrome, text snapshots (I4).** Fixtures laid out by Chrome through the verdict's browser, their boxes
  committed, compared in `cargo test`; snapshots of the layout dump and of what a screen reader hears (see the
  consequences for the tool and what they cover).
- **Instructions beside time (I5).** `gui::instructions::Instructions` counts retired instructions where an
  unprivileged process can, with no `unsafe` of ours: on Linux the calling thread's user-space instructions through
  `perf_event_open` (`perf-event`), on macOS the whole process's from `proc_pid_rusage`'s `ri_instructions`
  (`libproc`). `docs --bench` reports instructions per frame of the best round, writes them to `bench.json`, and
  compares by them when both runs have them (past 5% and 200 000 instructions), by time otherwise. CI benches the same
  build twice and fails if the second run reads slower.
- **The GPU CI paints on (I6).** `tests/adapter.rs` prints the adapter and checks compute (and indirect execution) and,
  under `GUI_REQUIRE_DUAL_SOURCE=1` as CI sets it, dual-source blending, which LCD text needs. Lavapipe advertises
  `dualSrcBlend`, so no Windows runner is added; should the check fail on a runner, the LCD goldens move to one.
- **Stress scenes (I7).** New: `list1m` (a million rows), `random`. Already there: `list` (ten million rows),
  `bigtable` (sort and filter a million rows off the UI thread). Not yet possible: chart of 100 million points, a
  stream of a million points a second and a canvas of 100 000 paths wait for ADR-0018's data chunks and ADR-0017's
  canvas (M6); "UI busy 50 ms while scrolling" is the render thread's own scene (`threads`, ADR-0015).

## Consequences

- A frame's allocation budget is the UI thread's alone; the layout pool's and the GPU driver's allocations show as
  the process's minus the thread's.
- The counting allocator adds, per allocation, a few relaxed atomics and, once a reading has started it, two
  thread-local adds.
- On macOS instructions are the process's: a GPU driver thread or another test in the same process adds to them,
  so their noise is larger than Linux's per-thread count. A VM without exposed counters (some CI runners) has none,
  and the bench compares by time there, as before.
- The memory report's heap owners are shallow: what a node's strings and vectors own beyond the node is
  "unattributed". Deep sizes need a size trait on every owner's types.
- `FrameReport.laid_out` is gone; `report.layout.placements` is what it counted.
- Measured on macOS (2026-11-21): steady frames allocate from 72 (`toasts`) to 195 210 (`table`, 5 000 rows all
  laid out) on the UI thread, and the costliest step from 455 (`reactive`) to 982 971 (`table`, a backspace in its
  filter). Each budget is about 1.25 times its measurement; a change that allocates less lowers it. Steady frames that
  allocate at all (`grid` 25 153, `random` 9 111, `churn` 10 844) are work for the retained paths, not noise.
- Instructions on macOS vary under 1% between bench runs of the same build (button 1.36M to 1.37M per frame, table
  2.90M to 2.91M) while the best round's time varies up to 30% and p95 up to 2x.
- Layout from Chrome (I4): 47 fixtures (`tests/chrome_layout/`, flex, grid, block, percentages, `calc()`, min/max,
  aspect ratio, absolute, overflow, text, baselines), generated by `verdict/layout.mjs` in Chrome with `gui`'s Inter
  and defaults, every box compared within 1/64 px. 45 match; `text_single_line` differs on purpose (an `i` is
  3.8125 px as the font file says, Chrome on macOS reads 3.8109); one is a bug listed in the test until fixed:
  `aspect_ratio_content` (Taffy's block and flexbox resolve a child's aspect ratio before laying it out, so the
  content cannot make a ratio box larger, and a min or max of the ratio-dependent axis is carried back to the
  other). The first two bugs it found are fixed (ADR-0014, "As built"): `aspect_ratio` (a box with both sizes
  definite took its height from its ratio in Taffy's leaf layout) and `flex_baseline` (`gui`'s text leaf gave no
  baseline, so Taffy aligned by height). Both lists are strict: an entry that matches Chrome again fails the test. Snapshots (insta, Apache-2.0, through `cargo deny` unchanged) hold six fixtures'
  layout dumps and a settings form's layout and a11y dumps before and after a click.
- A seeded tree 48 levels deep overflows a 2 MB debug thread stack while building and laying out; the seeded test
  runs on an 8 MB thread. The UI thread of ADR-0015 needs a stack sized for the deepest tree it accepts.

## Revisit when

- The render thread (ADR-0015) lands: its own thread counts belong in the report beside the UI thread's, and its GPU
  damage rectangles beside paint's flashes.
- The heap left unattributed grows past what the owners report: give owners a deep size.
- A CI runner exposes no counters: run the bench under a counting emulator (cachegrind) on the CPU-side phases.
