# 0015. `gui` runs on three threads, timed by presentation

Status: accepted, 2026-11-21; built 2026-11-21 (M5c, P6 and P7, see "As built"); input log replay built 2026-11-22
on logs synthesized from documented event streams (no log from real hardware is checked in yet); the upstream work is
not done. Supersedes ADR-0007's time source (the window "runs on the frame clock
and asks for frames while anything moves" with a 16 ms timer) and ADR-0011's single-threaded renderer in a window
(`GpuRenderer` drawing on the event-loop thread, `desired_maximum_frame_latency: 2`); keeps ADR-0007's motion model,
reduced motion and scenario clock, and ADR-0011's scene/renderer split.

## Context

Everything in a `gui` window runs on winit's main thread (`crates/gui/src/window.rs`):

- events;
- reactive updates, build, layout and paint;
- GPU submission and AccessKit.

Animation is scheduled with `ControlFlow::WaitUntil(now + 16 ms)` (`window.rs:25`, `FRAME_INTERVAL`), not by vsync.
`clock::begin_frame` fixes a frame's time at "when building starts" (`clock.rs:60`), and reading `clock::now()` asks
for a whole pipeline frame (`clock.rs:42`). The surface is configured with `desired_maximum_frame_latency: 2`
(`gpu/mod.rs:233`).

What this costs:

- **Stalls.** A 30 ms layout (resize on a large table, ADR-0014) stalls scrolling, trackpad momentum and every running
  animation, because they run on the same thread as everything else. On macOS every event, momentum included, arrives
  only on the main thread. So even a separate render thread would wait for a long layout on the main thread.
- **Uneven motion.** A 16 ms timer is wrong at 120 Hz and drifts against vsync, and "time when building started" is off
  by the build and present time.
- **Latency.** Latency 2 lets a frame queue behind another (`references/wgpu/wgpu-types/src/surface.rs:856-866`:
  Vulkan images = latency + 1, Metal `maximumDrawableCount` = latency + 1, DX12 `SetMaximumFrameLatency`).

Production engines solve this by splitting threads:

- **Chromium cc** commits from main to compositor (`cc/trees/proxy_main.cc:346-352`, `:555-569`). It keeps scroll
  offsets as synced properties (`cc/base/synced_property.h:11-21`). When main misses its deadline, it draws without
  main (`cc/scheduler/scheduler_state_machine.cc:1017-1024`, `:1336-1364`).
- **Android** runs a RenderThread.
- **Flutter** ticks animations before build (`references/flutter/packages/flutter/lib/src/scheduler/binding.dart:162-200`).
- **Ladybird** forks render state for its render thread (`references/ladybird/Libraries/LibWeb/Rust/src/render_state/clock.rs:7-26`).

What the libraries give today:

- winit 0.30 and 0.31-beta.4 have no vsync event. Issue #2412 is open; PR #4776, a macOS display link, was closed.
- wgpu 30 has no presentation feedback or present-at-time API (#2869, #9856).
- `CAMetalDisplayLink` cannot be combined with how wgpu acquires drawables.

## Options

1. **Keep one thread**, and fix only timing. Long UI work still stalls scroll and animation.
2. **Two threads**: winit and UI on main, render on its own. Scroll and animations survive a long GPU submit but not a
   long layout, because input arrives on the same thread as layout.
3. **Pipelined rendering** (Bevy: frame N renders while N+1 builds). It adds a frame of latency to everything.
4. **Three threads, timed by the predicted present time.**

## Decision

Option 4.

**Threads.**

- **Platform thread** (main): winit, the AccessKit adapter, IME and the cursor. It stamps each event on arrival and
  forwards it. It never runs component code, layout or GPU work.
- **UI thread:** reactive graph, components, layout (ADR-0014), paint recording, accessibility tree and hit testing.
- **Render thread:**
  - owns the wgpu device and queue, retained chunks, property trees and atlases, and rasterizes glyphs;
  - runs wheel scroll, fling, and transform and opacity animations by itself, at its own frame time;
  - scrolls only within content already built (a virtual list's materialized rows).

**One frame.**

1. The render thread acquires the next drawable and ticks compositor animations at `present_at`. It sends
   `BeginMainFrame(MainFrameToken)`. The token carries `present_at`, a `deadline`, the frame interval and the scroll
   deltas applied since the last commit. It is not `Clone`: one token, one commit.
2. The UI thread runs the frame and answers with one `Commit`. A commit carries only what changed:
   - chunk inserts, replacements and removals, as `Arc<Chunk>` (immutable, `#[repr(C)]` and `bytemuck::Pod`
     instances);
   - the draw order, when it changed;
   - property-tree deltas;
   - scroll acknowledgements;
   - animations started or cancelled;
   - new resources.

   Nothing is copied, and the UI never blocks on the render thread.
3. A commit is applied whole or not at all. If it misses the deadline, the render thread still draws a
   compositor-only frame: scroll and animations advance, the rest stays. This is the compositor-only frame of
   Chromium's high-latency mode. There is no pending tree; our case is cc's single-threaded mode.
4. When input arrives while everything sleeps, the render thread grants a token at once, so input-driven updates are
   not delayed by a vsync wait.

**Scroll** is a synced property, as in cc's `SyncedProperty`:

- the render thread holds `active = base + unacknowledged delta`;
- `BeginMainFrame` carries the delta to the UI, which applies it to its scroll signal;
- `Commit.scroll` acknowledges what was applied, so nothing is applied twice;
- an offset set by the program carries a generation, and older deltas are dropped.

**What the user saw.** The render thread publishes an `Arc<Presented>`: frame, commit, scroll offsets and the
animated values on screen. Every input event carries the `Arc<Presented>` current when it arrived, and hit testing
uses it. A click therefore hits what was on screen even while the compositor scrolls.

**Time.**

- One time drives UI animation (`clock::frame_time`), compositor animation, fling and input resampling: the predicted
  present time.
- Reading `clock::now()` no longer starts a pipeline frame. `FRAME_INTERVAL` and `ControlFlow::WaitUntil` for
  animation are gone. The surface is `AutoVsync` with `desired_maximum_frame_latency: 1`.
- **Today's vsync source**, with no `unsafe`: the moment `get_current_texture` returns after blocking (more than
  ~0.5 ms) is a vblank, and the frame drawn now shows at the next one.
  - A phase estimator fits the last ~16 blocked acquires, dropping outliers beyond ¼ interval.
  - The nominal interval comes from `refresh_rate_millihertz`.
  - Wayland frame callbacks stop drawing while the surface is hidden.
- **Upstream contributions are part of this decision,** not a later option, because calling platform display links
  ourselves needs `unsafe`:
  - winit #2412: a frame event with the target present time, from `NSView.displayLink` on macOS, frame callbacks with
    `wp_presentation` on Wayland, and the composition clock on Windows;
  - wgpu #2869/#9856: present ids and presented timestamps;
  - wgpu: `presentsWithTransaction` for smooth live resize on macOS.

  When they land, only the clock module changes.

**Input resampling.**

- Pointer drags (slider, drag and drop, text selection, column resize) are resampled at `present_at − 5 ms`:
  - interpolated between the two samples around it, or extrapolated at most `min(8 ms, ½ the last sample gap)`;
  - samples closer than 2 ms are not predicted.

  These are Android's constants (`InputConsumer.cpp:148-161`) and Chromium's `linear_resampling`
  (`ui/base/prediction/linear_resampling.cc:18-46`). Clicks and key presses are never resampled.
- Scroll is predicted linearly at `present_at` on the render thread, as Chromium's `ScrollPredictor`. Prediction stops
  35 ms after the last event (`kScrollPredictorMaxResampleTime`). A prediction that did not happen is returned the
  next frame.

**Invariants**, each with a test:

1. One token, one commit, applied in sequence order.
2. A commit is applied all or nothing.
3. After every commit the render thread's state equals a cold build from the UI's `Arc`s.
4. Scroll is never lost or applied twice.
5. Hit testing with the carried `Presented` matches the pixel at the point.
6. The render thread reads no signal and the UI touches no wgpu (`!Send` handles; the device lives only in the render
   thread's struct).
7. A render panic or device loss rebuilds the render thread from the UI's `Arc`s.
8. No thread spins: all three sleep on a channel or the event loop.

## Consequences

- Expected:
  - scroll, fling and compositor animations stay at display rate while the UI thread is busy;
  - latency is one frame for everything, and less for scroll and compositor animations;
  - motion is even at 60, 120 and 144 Hz.
- `window.rs`, `app.rs`, `clock.rs` and `gpu/` are restructured. `Scenario` drives the render thread's clock through
  the scenario clock, so headless stills and played motion keep ADR-0007's behaviour.
- Scroll no longer goes through layout, and positions no longer bake in the offset (ADR-0014's anchors and the
  property tree carry it). Hover after a compositor scroll is recomputed by the UI from the `BeginMainFrame` that
  reports the scroll.
- Tests replay recorded event logs (macOS trackpad, notched wheel, libinput) against fake 60/120/144 Hz clocks. They
  measure step jitter and error against the true position at `present_at`, with regression thresholds in CI.
- Risk: live resize on macOS may wobble until `presentsWithTransaction` is exposed by wgpu. winit events carry no
  timestamps, so the platform thread must stay light to stamp them accurately.

## Revisit when

- winit or wgpu ship a frame event or presentation feedback: replace the acquire-based estimator in the clock module.
- The estimator's phase error, measured against presentation feedback, exceeds a quarter of an interval on some
  platform.
- A device class shows the render thread's handoff costing more latency than it saves (input-to-photon measured with
  presentation timestamps).
- Kalman or least-squares prediction beats linear prediction on recorded scroll logs: switch the predictor.

## As built

Built on 2026-11-21 (M5c: plan tasks P6 and P7). Files: `crates/gui/src/window.rs` (platform thread, UI loop,
drag resampler, `Offscreen`), `compositor/mod.rs` (`Commit`, `Compositor`, `Presented`, `Headless`, the `Compose`
trait), `compositor/scroll.rs` (synced offsets, `ScrollPredictor`, `Fling`), `compositor/thread.rs` (the render
thread), `compositor/tests.rs`, `clock.rs` (`Vsync`, frame clock), `motion.rs` (`Curves`), `paint.rs` (the
compositing tables), `interact.rs` (`shift_of`, `hit_shifted`, `apply_scroll`), `app.rs` (`Pipeline::draw_with`,
`gui::offscreen`), `text/fonts.rs` (`Glyphs`), and the stress check `crates/shadcn/examples/stress/threads.rs`.

**Threads.**

- Each thread is scheduled after Chromium's `PlatformThread::SetCurrentThreadType` (`priority.rs`, through
  `gdt-cpus`' safe API):
  - the platform thread as Chromium's browser UI thread (`kDisplayCritical`): `QOS_CLASS_USER_INTERACTIVE` on macOS,
    `THREAD_PRIORITY_HIGHEST` on Windows, nice −10 on Linux (through rtkit when the process may not lower it);
  - the UI thread as a renderer's main thread (`kDefault`): `QOS_CLASS_USER_INITIATED`, normal, nice 0;
  - the render thread at the top fixed priority of the user band: `SCHED_RR` 47 on macOS,
    `THREAD_PRIORITY_TIME_CRITICAL` on Windows, nice −20 on Linux. This differs from Chromium on purpose. Chromium's
    compositor gets the user-interactive class, which is timeshare and decays while the thread runs. Measured
    interleaved on the same loaded machine (stress `threads`, load 40–50), that class still woke the render thread up
    to 4 ms late, missed 0–15 ticks in 180, and showed 16–20 of 20 clicks within the goal. The fixed priority missed
    none in 7 runs, with 18–20 of 20 clicks within the goal. The render thread sleeps on the vblank or its channel
    and never spins (invariant 8), so it cannot starve the rest.
  - macOS's time-constraint policy, which Chromium keeps for audio, is not used: a thread that overruns its declared
    computation is demoted below every other.
- The UI thread and the layout workers run on a 64 MB stack (`layout::STACK`, address space committed as it is
  reached), room for the deepest tree `gui` takes, 512 levels (`layout::MAX_TREE_DEPTH`, Chromium's parser limit), in
  a debug build three times over (`a_tree_as_deep_as_gui_takes_runs_on_the_app_threads`).
- `gui::run(title, setup, options)` takes a `setup: FnOnce() -> C + Send` that runs on the UI thread and returns the
  root render `C: FnMut() -> Element`. The app's state (signals, `provide`) is made on the thread that owns it; the
  reactive runtime stays thread-local and `!Send`. The docs and the stress `--window` provide their theme inside
  `setup`.
- The **platform thread** (main) runs winit, the AccessKit adapter, the input method and the cursor. Each event is
  stamped with `Instant::now()` and the current `Arc<Presented>` (read from a `Mutex<Arc<_>>` the render thread
  replaces after each frame). The wheel goes to the render thread; everything else goes to the UI thread.
  AccessKit's requests are forwarded to the UI thread, and the UI thread sends back each frame's `TreeUpdate`, which
  the platform applies with `update_if_active`. The cursor and the input method's area travel the same way.
- The **UI thread** loop (`window::ui_loop`) sleeps on its channel or until the next timer. It asks for a frame
  (`NeedsFrame`) and runs one when granted a `MainFrame`. Mailboxes wake it through `mailbox::wake_with`.
- The **render thread** (`compositor::thread::RenderThread`) owns the `GpuRenderer`, the committed scene, the
  property trees, the atlases and a `Glyphs` (the raster half of the text system, moved out of `TextSystem`;
  glyphs are rasterized here). Each frame it:
  1. waits for a vblank (a blocked `get_current_texture`, or a ticker with no window);
  2. predicts the present time;
  3. grants the UI a `MainFrame` if it asked;
  4. waits for the commit until the deadline;
  5. draws.

  The frame is drawn at the deadline, or as soon as a commit comes. While it waits, a main frame asked for meanwhile
  is granted at once, for this frame's present time (cc's `BeginMainFrame` sent inside the impl frame): a click that
  comes after the vblank still shows at the next one. Interleaved with drawing at once when nothing is owed (cc's
  immediate deadline), this showed 18 of 20 clicks within the goal against 12–16, and no more missed ticks. When input wakes everything up,
  the token is granted at once, not at the next vblank. The deadline is
  `present − margin`, where the margin is the worst draw cost of the last 32 frames (waking late included) plus
  1 ms, between 3 ms and half an interval. When nothing moves the render thread sleeps on its channel. On Wayland it
  draws again only after the frame callback (`pre_present_notify`, `request_redraw`, `RedrawRequested`).

**Commit.**

- `Commit::between(last, new)` carries:
  - the chunks recorded again (`Arc<Display>`, shared, not copied);
  - the order, only when it changed;
  - the changed property entries by id;
  - the scroll, hit and moving tables (`Arc<[_]>`, sent only when they changed);
  - the glyph tables it needs (`GlyphUpdate`: the font data `Arc`s and the axes of the faces new since the last
    commit, and the text settings when they changed).
- Chunks are `Arc<Display>`, not `#[repr(C)]` instances: the typed records are written on the render thread from
  the display (ADR-0016's arena), so the UI never touches wgpu.
- The compositor applies a commit whole, and it then equals the UI's scene. A compositor frame composed at the
  commit's own time equals the UI's property trees (`a_commit_applied_is_the_ui_scene_and_composes_to_it`).
- The HUD and the debug overlay travel with the commit, and as an `Overlay` message after it.

**Compositor-only frames.**

- `Compositor::compose(at)` writes, over the committed property trees:
  - each scroll container's synced offset, snapped to device pixels as paint snaps them (ADR-0020);
  - its thumb's position;
  - each moving node's opacity and transform at `at`.
- The moving values are sampled by the same code layout uses: `motion::Curves`, the node's transition runs and
  keyframed animation, with `Own::keyframed` and `progress`. A node is moved by the compositor when its motion
  touches only opacity and transform, and it lays nothing out while it runs. Its end wakes the UI through
  `Motion::wake_at`.
- A transform's offset is the whole way to the window, so a shift reaches every descendant transform (`totals`),
  clip, effect region and mask.
- When the UI misses the deadline, scroll, fling and motion still move.
- Compose is incremental (2026-11-21, `compositor/compose.rs`): the compositor keeps the shown scene and recomposes
  only the subtrees of transforms displaced now or last frame and the entries commits wrote, and hands the renderer
  what it changed (`scene::Changed`, ADR-0016). A commit that reshapes the trees (order, size, scale, a property's
  parent, transform or clip) composes whole. `GUI_COLD_CHECK=1` asserts the result equals a whole compose.

**Recovery (invariant 7).**

- A lost device (the device's lost callback: a driver reset, a GPU removed) is made again on the same instance and
  surface (`GpuRenderer::rebuild`). The compositor keeps the scene as committed (the UI's `Arc`s), so the next frame
  encodes every chunk and rasterizes every glyph and icon again, and draws the canvas whole.
- A panic in the render thread is caught (`catch_unwind` around its loop). Its device and compositor are dropped and
  made afresh, and the UI is told (`FromRender::Lost`). The UI's next commit carries the whole scene and every glyph
  table (`Commit::full`). Deltas sent before the UI knew are dropped. Nothing is drawn until the full commit comes, so
  the window keeps its last frame. This is Chromium's recovery from a lost GPU process: a new `LayerTreeFrameSink`,
  then a full commit.
- A panic that comes back three times in ten seconds belongs to the scene, not to the GPU: the app closes, saying
  why. Fifty devices asked for in vain, 100 ms apart, end it the same way.
- Tested by fault injection (`Offscreen::lose_device`, `Offscreen::panic_render`):
  `a_lost_device_or_a_render_panic_is_recovered_from_the_uis_arcs`. After the loss the same pixels are shown, and a
  click after it is drawn. After the panic the full commit draws what was on screen, and a click after it toggles
  back to the first frame's pixels.

**Scroll.**

- The render thread scrolls a container by the wheel at the pointer. The target is found as the UI finds it: the
  `Hit` table in paint order, then the scroller chain, then the first one that can move that way.
- Offsets are synced properties: `committed offset + sent deltas + pending`.
  - `MainFrame.scroll` hands the pending deltas to the UI, which adds them to its offsets (`ScrollHandle::synced`
    does not bump the generation).
  - `Commit.answers` drops what the UI took in.
  - A generation bumped by the program (`scroll_to`, the UI's own wheel, `reveal`) drops what the compositor had
    scrolled before.
- Trackpad gestures (`PixelDelta`) are resampled at each frame's present time by `ScrollPredictor`:
  - interpolated between events;
  - extrapolated at most `min(8 ms, half the gap)`;
  - events closer than 2 ms are not extrapolated from;
  - prediction stops 35 ms after the last event.
- Where the platform does not fling (not macOS, iOS or Windows), `Fling` carries the gesture on after it ends:
  exponential decay, τ = 325 ms, stopping under 50 px/s.
- Notched wheels apply at once.
- **Reach.** Each `ScrollDelta` carries how far the compositor may scroll its container before the commit after this
  frame's shows: the container's speed (a fling's own, or what it moved over the last 100 ms) times a horizon of
  `2 × the UI's worst answer time over its last 32 main frames (the one it owes now included) + 3 intervals`. A
  virtual list builds that much more beyond its viewport in the direction it moves. This is Chromium's interest rect
  and Flutter's `cacheExtent`, sized by speed and latency. The compositor scrolls only the rows committed, so a fling
  or a busy UI never shows rows that are not built.
  - A fling or a resampled gesture asks the UI for a frame as it moves, so the UI is told of the scroll.
  - Tested by `a_compositor_scroll_within_its_reach_never_passes_the_rows_built`: a 1 000 000-row list, a UI that
    answers at once under a 12 000 px/s wheel, and one six frames late under 3 000 px/s. Every compositor-only frame
    after the first commit moves the whole step. Without the reach, the late case reaches the end of the rows built
    at cycle 2.

**What was on screen.**

- `Presented` holds, for each frame:
  - the frame and commit numbers and the present time;
  - each scroll container's offset and applied shift;
  - each moving node's shift.
- `interact::shift_of(frame, presented)` turns it into a per-node shift. Hit testing (`hit_shifted`) and drag
  positions use it, so a click while the compositor scrolls hits the row that was there
  (`a_click_while_the_compositor_scrolls_hits_what_was_on_screen`).
- The devtools read positions off the UI's frame and send input unshifted.

**Time.**

- `clock::now()` is gone; reading the time asks for no frame.
- `clock::begin_frame(present_at)` fixes the frame at the `MainFrame`'s predicted present time, and motion samples
  at it.
- `clock::Vsync`:
  - blocked acquires (longer than 0.5 ms) are vblanks;
  - the phase is fitted to the last 16, dropping those more than a quarter interval from their median;
  - the interval comes from `refresh_rate_millihertz`.
- Drags are resampled on the UI thread (`window::PointerResampler`):
  - one move a frame, at `present − 5 ms`;
  - interpolated, or extrapolated at most `min(8 ms, half the gap)`;
  - no extrapolation from samples closer than 2 ms.

  A release sends its own position. Clicks and keys are never resampled.

**Headless.**

- `Scenario` and the devtools `Session` drive a `Headless` compositor on the same thread. Each commit is applied and
  drawn at once, at the commit's own time on the scenario clock, and read back.
- `Scenario::wheel` scrolls through the compositor and draws a compositor-only frame, as a window's wheel does. The UI
  takes the scroll in with its next frame.
- Why one thread: a scripted run must give the same frames every time, and two threads would make which commit a
  compositor frame shows depend on scheduling. The stages and their order are the threaded ones (commit, apply,
  compose at a time, draw); only the scheduling differs.
- The threaded path is measured by `gui::offscreen` (`window::Offscreen`): the UI and render threads with no window,
  the render thread on a 120 Hz ticker, and the caller playing the platform thread.

**Kiểm and measurements.**

Every row below was measured on a shared, loaded machine (10 cores, load average 40–80 throughout).

| Kiểm | Result |
| --- | --- |
| A click while scrolling hits what was on screen | `compositor::tests::a_click_while_the_compositor_scrolls_hits_what_was_on_screen` |
| Scroll is never lost or applied twice | `a_scroll_is_owed_until_a_commit_takes_it_in_and_never_twice`, `the_program_setting_an_offset_drops_what_the_compositor_scrolled`, `a_scroll_stops_at_the_edges` |
| A compositor frame at the commit's time is the UI's scene | `a_commit_applied_is_the_ui_scene_and_composes_to_it` (a fade in motion, a scroll) |
| A late UI still gets moving frames | `a_frame_the_ui_owes_still_moves_what_the_compositor_runs` |
| An animation evenly spaced under a fake clock | `an_animation_is_evenly_spaced_at_predicted_present_times`: a 120 Hz display with 0–0.3 ms jitter, present times from `Vsync`, a linear 500 ms fade; every step 1/60 of the fade ± 10% |
| Vsync phase fit | `vsync_is_fitted_to_blocked_acquires`: within 0.5 ms of the grid, an off-grid newest sample ignored |
| Prediction, fling, drag resampling | `a_scroll_prediction_interpolates_reaches_8_ms_ahead_and_stops_after_35`, `a_fling_slows_and_ends`, `window::tests` (3); on input logs at 60/120/144 Hz, `compositor::replay` (7 tests, see "Input logs") |
| UI busy 50 ms, scroll and animation at 120 Hz (I7) | stress `threads`: the UI is busy 50 ms of every 100 ms while the wheel scrolls a 300-row list every 4 ms and a box spins. Before, run alone at load 60–80: 102–115 Hz, 11–27 ticks missed. After, with the render thread at fixed priority, at load 40–50: 180–182 frames in 180–182 ticks (120 Hz), 0 missed, worst gap 8.3 ms, in 7 of 7 runs; 86–102 frames drawn without the UI. |
| Input to present: one frame | stress `threads`, 20 clicks with the UI idle and a spinner keeping the render thread busy. Before: median 14.8 ms, 16/20 within two intervals + 1 ms (17.7 ms), worst 82.5 ms. After, at load 40–50: median 9.4–10.7 ms, worst 12.7–23.6 ms, 18–20 of 20 within 17.7 ms. A click is matched to the first frame whose main frame was granted after it. |
| `retained` (ADR-0016) after the change | A wheel step is drawn by the compositor. The UI's frame after it records nothing, lays out nothing and submits nothing (the canvas already shows it). |

Verification:

- Verdict at the end of M5c's second pass: 98/100 with 6 FAIL. `AspectRatioDemo`, `InlineCitationDemo` and
  `MessageScrollerPreviousContextDemo` were failing before this pass. `CalendarDemo`, `CalendarPersianDemo` and
  `CalendarCustomCellSize` fail on the date alone: the docs fix "today" to 2026-11-20
  (`examples/docs/pages/calendar.rs`) and Chrome draws the real day, so they match only when run on that date. No
  change in this pass meant to move a pixel.
- `stress --run`: every scene of this milestone PASS. `threads` at load 90: 180 frames in 180 ticks, 0 missed;
  input to present median 11.5 ms, worst 22.2 ms, 16/20 within 17.7 ms. `canvas` (M6, in progress) FAILs on a
  wgpu validation error in its chunk buffers.
- `cargo test -p gui --lib`: 306 passed. The tests for this pass: `a_lost_device_or_a_render_panic_is_recovered_from_the_uis_arcs`,
  `a_compositor_scroll_within_its_reach_never_passes_the_rows_built`, `a_tree_as_deep_as_gui_takes_runs_on_the_app_threads`,
  `the_render_and_platform_threads_are_raised`.
- `GUI_COLD_CHECK=1` on the docs monkey: 300 steps, no difference.
- The docs window starts and renders on three threads. Driven through `--serve --hud` and `--attach`: a click, a
  screenshot read back from the render thread, a wheel.
- docs `--bench`, `code-block`. A profile showed SipHash over each node's whole path (`paint::key`, `NodeKey::of`,
  both new in M5c) taking 431 of 1135 paint samples. `NodePath` now carries a 64-bit key, computed in O(1) from its
  parent's when the path is made, so hashing or comparing a path touches one word. Instructions per frame went from
  16.7M to 11.7–12.0M (stable at any load). At load 45–58: p50 2.5–3.6 → 0.9–1.7 ms, layout 1.4–1.9 → 0.6–1.0 ms,
  paint 0.9–1.2 → 0.3–0.6 ms.
- docs `--bench`, all 85 pages at load 60–93: steady p95 median 0.63 ms. p95 is not a measure at that load:
  `typography` (p50 0.06 ms) gave p95 19.1, 5.8 and 0.09 ms in three runs. The slow frames come in runs with every
  stage inflated, which is preemption.

**Input logs, recorded and replayed (2026-11-22).** Files: `compositor/input_log.rs` (the log, `Router`,
`from_winit`, `Recorder`), `compositor/replay.rs` (the replay and its tests), `crates/gui/tests/input_logs/`.

- **What is logged** is what the platform thread got from winit, before `gui` makes anything of it: pointer moves
  (logical pixels), buttons, a wheel's raw delta (lines, or logical pixels for a trackpad, in winit's sign) with its
  touch phase, modifiers, keys (logical key and text), and the input method's preedit and commit. Each line has its
  arrival time in milliseconds (µs resolution). The header names the platform, the window's logical size and scale,
  and where the log came from. The format is text, one event a line, described in `input_log.rs`.
- **One routing for both.** The window's platform thread now turns each winit event into an `input_log::Event`
  (`from_winit`) and routes it with `Router::route` (pointer position, modifiers, the wheel's sign and line size,
  `ended` for winit's ended and cancelled). The replay runs the same two functions, so a log replays exactly what
  the window would have forwarded. The UI thread's drag gating (hold moves while the button is down, flush on
  release) is mirrored in the replay around the shared `PointerResampler`, because `ui_loop` was being reworked by
  the window decorations work at the same time.
- **Recording is one command:** `GUI_RECORD_INPUT=crates/gui/tests/input_logs/<name>.log cargo run --release -p
  shadcn --example docs`, then scroll, flick, drag and click, and close the window. Any `gui::run` app records the
  same way. The platform thread only sends each stamped event down a channel. A thread of its own formats and writes
  the line and flushes it, so stamping stays as cheap as before. A log dropped into `tests/input_logs/` is replayed
  by `cargo test -p gui --lib compositor::replay` with every check below.
- **The replay** (`replay::replay`) feeds a log at its own times through `Router::route`, the compositor's
  `gesture` and `wheel`, and the pipeline's `input_shown` (hit testing against the `Presented` of the last frame
  drawn before the event). It runs on a fake display at 60, 120 and 144 Hz. Each vblank follows the render thread's
  schedule: a main frame when the UI asked, else a compositor-only frame while something is dirty or moves, else
  nothing. A frame is composed at its present time, one interval after the vblank. The UI answers at once, or (the
  busy-UI run) only every fourth vblank it is owed one. Everything runs on the scenario clock in one thread, and
  two replays give the same frames (`a_replay_is_the_same_every_time`). Unlike the real thread, a token is granted
  only at vblanks, never at once on input.
- **The probe scene:** a list of 2 000 rows 48 px tall, opened in its middle so no log reaches an edge, each row
  clickable, and a pad 100 px wide beside it that takes drags.
- **Checks run on every log at every rate** (`every_input_log_replays_without_losing_scroll_at_60_120_and_144_hz`):
  - **Nothing lost or doubled.** When the log is over, the UI owes nothing, holds exactly the offset the compositor
    shows, and that offset is the start plus every routed step (within 0.05 px). Where `gui` flings, the excess has
    the gesture's sign.
  - **Prediction stops 35 ms after the last event.** In every gap after a wheel event longer than 35 ms + 3
    intervals, every frame presented from 35 ms on shows where the events put the list, and the compositor draws no
    frame of its own from 35 ms + one interval on.
  - **Error against the true position.** The truth is the events' running total, interpolated between events. While
    a gesture is resampled, each frame's error against the truth at its present time stays within `speed × (interval
    + the event gap) + 1 px`. That is what showing the events up to the vblank costs. A lost or doubled step is far
    more. Worst errors measured: macOS flick 50 / 25 / 32 px at 60 / 120 / 144 Hz (up to about 3 000 px/s), Wayland
    34 / 21 / 18 px, slow scroll 8 / 5.5 / 5.5 px. The step jitter is printed for each run.
  - **Clicks hit what was on screen.** A click on the list fires on the row the carried `Presented` showed under the
    pointer, and it does not fire when the press and the release were on different rows.
  - **Drags are resampled.** Each frame hands the drag at most one move, at the oracle's position: the raw samples
    received by the vblank, interpolated at `present − 5 ms`, or extrapolated at most `min(8 ms, half the gap)`, and
    not from samples under 2 ms apart. The release first hands the last raw position.
  - **Flings of `gui`'s own** (platforms that send no momentum) slow down every frame, never turn, and end by
    `τ · ln(v₀ / 50 px/s)` + 0.1 s, after which nothing is drawn.
- **Checks run on particular logs:** macOS momentum is scrolled as it comes, with no fling of ours, and nothing is
  drawn 35 ms after its last event (`macos_momentum_is_scrolled_as_it_comes_and_ends_with_its_last_event`). A
  libinput gesture lifted while moving flings 438 px. One lifted 60 ms after the fingers stopped flings 281 px. One
  lifted after 200 ms does not fling (`a_libinput_gesture_is_flung_and_fingers_that_stopped_fling_slower`). Notches
  show whole in the next frame (`a_notched_wheel_applies_at_once`). A 1 000 Hz mouse's 300 samples reach the drag
  as one move a frame (`a_drag_from_a_1000_hz_mouse_is_one_move_a_frame`).
- **A mutation check:** with `Scrolls::committed` keeping deltas the UI had already taken in, the replay fails at
  once ("the UI took in 48040 but the compositor shows 48080").
- **The logs checked in are synthesized, not recorded.** No trackpad was at hand when this was built, so each log reproduces a
  documented event stream, and its header says which:
  - `macos-trackpad-flick.log`: AppKit's `scrollWheel:` phases (MayBegin, Began, Changed, Ended, then
    `momentumPhase` Began, Changed, Ended with phase None), as winit 0.30 maps them (`platform_impl/macos/view.rs`,
    `scroll_wheel`: the momentum phase first; MayBegin and Began become started, Ended and Cancelled ended). Precise
    `scrollingDeltaY` in points. Finger events every 8–9 ms. Momentum every 16.7 ms, decaying by 0.998 a millisecond
    (`UIScrollView.DecelerationRate.normal`).
  - `macos-trackpad-slow.log`: a slow scroll, a 200 ms rest (no events), the other way, a lift with no momentum,
    then a touch that never scrolls (MayBegin, Cancelled).
  - `macos-mouse-wheel.log`: a notched wheel on macOS (no precise deltas, accelerated lines, phase None).
  - `windows-wheel.log`: WM_MOUSEWHEEL, 120 units a notch. Then a precision touchpad's deltas under 120, and its
    inertia as more of them, with no end message, all `LineDelta(delta / 120)` with phase moved
    (`platform_impl/windows/event_loop.rs`).
  - `wayland-touchpad-fling.log`: libinput finger axis frames every 7–12 ms, and an axis stop at the lift, which
    winit makes started, moved and ended (`platform_impl/linux/wayland/seat/pointer/mod.rs`). Then `axis_discrete`
    wheel steps.
  - `drag-and-click.log`: 125 Hz and 1 000 Hz mice, and clicks right after wheel steps.

  Chromium's input test data is not in `references/`. The rates and deltas are chosen within these documented rules,
  not measured. `the_synthesized_logs_are_as_generated` keeps the files equal to their generator.
  `cargo test -p gui --lib write_synthesized_input_logs -- --ignored` writes them again.
- **Found by the replay, and changed: the fling's speed.** A libinput gesture's fling took the speed of its last
  three events, whenever the fingers lifted. Fingers that stopped and then lifted within 35 ms flung at full speed.
  After 35 ms the gesture was dropped and nothing flung. The fling now takes GTK's rule
  (`gtkeventcontrollerscroll.c`, `scroll_history_finish`, `SCROLL_CAPTURE_THRESHOLD_MS`). The speed is what moved
  from the first event within 150 ms of the lift, divided by the time from that event to the lift, the lift's own
  time included. The gesture is kept that long where `gui` flings. Whether `gui` flings is now set per compositor
  (`Compositor::set_flings`, by default the platform built for). A replay sets it from the log's platform, so a
  Wayland log flings on a macOS host.

**Not met, and why.**

- **No log from real hardware is checked in.** The replay and the recorder are built, and the checks run on every
  log. The logs themselves reproduce documented streams, not measured ones. Recording one on a MacBook trackpad, a
  Windows precision touchpad and a libinput touchpad is the one command above.
- **Upstream work** (winit #2412 frame event, wgpu presentation feedback and `presentsWithTransaction`) is not
  started. Live resize on macOS may wobble.
- **The first frames of a fling.** The reach is sized by speed already measured, so the first commit after a fling
  starts holds only the prefetch margin (one viewport). A fling that outruns it before the UI's next commit stops at
  the rows built for that frame.
- **Platform flings.** On macOS and Windows a fling is the platform's momentum events, scrolled as they come
  (precise ones resampled). There is no fling of our own.
- **No page over 4 ms p95 in docs `--bench`** is unmeasured. The machine was at load 60–93, where p95 measures
  preemption (above). What remains in `code-block`'s frame, by instruction count, is the bench's forced patch of the
  whole tree, which predates M5c.
- **The machine was never idle.** Every number above was measured at load 30–93 on 10 cores, so the goals are
  unmeasured on an idle machine.
