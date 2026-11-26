# 0015. `gui` runs on three threads, timed by presentation

Status: accepted, 2026-11-21; built 2026-11-21 (M5c, P6 and P7, see "As built"); scroll's generation rule (a
program's offset drops older deltas) superseded 2026-11-25 by
[ADR-0032](0032-scroll-is-ordered-by-what-the-ui-has-seen.md); input log replay built 2026-11-22
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
  stamped with `Instant::now()` and what the screen showed then (`Shown`, a query of the presentation timeline the
  render thread appends each present to). The wheel goes to the render thread; everything else goes to the UI thread.
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
  1 ms, between 3 ms and half an interval. When nothing moves the render thread sleeps on its channel.

  **When it waits for a vblank, and when for a deadline (M15).**

  - A frame begins with a texture taken (`GpuRenderer::acquire`). The acquire blocks only while the last present waits
    for its vblank. That block is the vblank wait, and its length is the vsync sample.
  - A frame that changes nothing presents nothing. It keeps (holds) the texture it took, because Vulkan cannot hand
    one back (wgpu's discard leaves the image acquired). The next frame then takes no texture, waits for no vblank,
    and is paced only by its deadline.
  - Each frame is drawn for a vblank (`RenderThread::aim`): the first it can make, never one a frame that presented
    or took a texture was drawn for (`reserved`). A main frame granted while the thread slept is granted for the same
    vblank the frame will be drawn for, by the same rule. It is not granted for T while the frame draws for T + 1.
  - A frame that presented nothing and took no texture reserves nothing (`blank`). The next frame is drawn for that
    vblank again only if something new comes before its deadline (a wheel, a commit, a main frame asked for).
    Otherwise it is drawn for the vblank after. So idle frames are not drawn again and again for one vblank.
  - Only the ask that gave the texture is timed. Configuring an out-of-date surface (or making a lost one again)
    before it is not a vblank. Neither is a frame whose acquire failed, which woke on a timer.

  **Pending presentation.** A frame meant to present whose texture could not be taken leaves the presentation owed:
  - the timeout came at the start and again when drawing (`Window::stale`, `GpuRenderer::owes_present`);
  - the thread keeps it as `retry`, apart from `dirty` (what came and is not drawn yet). The next frame presents the
    canvas, damage or not;
  - a retry is not news (`news_until`): that frame waits for the next vblank, so a window whose acquires keep failing
    does not spin. A commit or wheel that came before a frame's deadline is news, whether the frame's own wait or the
    thread's turn (`serve`) took it in;
  - input stays stamped with the frame the window shows: a frame whose present could not be made (no texture)
    never enters the presentation timeline.

  An out-of-date surface is configured again. A lost one is made again from its window, then configured. A surface
  that says the window cannot be shown (wgpu's `Occluded`: minimized, covered) is not a timeout. Nothing more is tried
  until the window is shown, exposed or resized, or a second passes (`OCCLUDED_PROBE`). That try is no frame: nothing
  is logged or counted for it. Resizes that come together configure the surface once, at the last size.

  A main frame granted while the thread slept is for the vblank `aim` gives then. The frame drawn next keeps that
  vblank (`granted_for`) however long its acquire takes, so the UI's motion and the compositor's sample one time,
  phase fitted or not.

  **Wayland.**
  - A frame callback is asked for with each present (`pre_present_notify` before it, `request_redraw` after). A present
    predicted and not made (its texture could not be taken) waits for no callback.
  - The thread draws again only once it comes (`RedrawRequested`, `ToRender::FrameCallback(at)`).
  - Each callback is a vsync sample. A gap between two callbacks samples the refresh only when the window presented
    right after the first one, within the same refresh (`chained`). A present after the window idled (every third
    refresh for a key repeat) or a probe present samples nothing, so the interval never locks onto a present cadence.
  - With at least 3 gaps, the refresh is the shortest gap that another agrees with (within 10%), averaged with it: one
    callback late then one early is jitter, not a faster refresh. It sets the interval when it is off by more than 3%.
  - When winit reports the monitor's rate, the fit is bounded by it: never faster, never more than 1.5 times slower.
    Otherwise the interval starts at 60 Hz, and the callbacks alone set it.
  - winit commits no surface without a present, so a window whose frames present nothing would ask for no callback,
    and a hidden window would never be known as one. After 8 frames in a row that present nothing, one presents
    anyway (`WAYLAND_PROBE`). The thread then sleeps until its callback: on a hidden window, until it is shown.
  - The probe's cost: an idle window that still animates to no effect (a held animation) presents one unchanged frame
    every 9 vblanks, about 7 a second at 60 Hz: one blit of the canvas and one compositor repaint each. A window with
    nothing to draw sleeps and probes nothing.

  **Not testable headless.** Real Wayland frame callbacks, and a compositor withholding them from a hidden window,
  are tested only through a fake `FrameCallbacks`. Not tested at all:
  - real swapchain extents under Vulkan on X11 and Win32 (the virtual surface's strict mode stands in);
  - Metal drawables and their one-second acquire timeout;
  - the refresh rate read again after a `Moved` onto another monitor.

  **Input snapshot.** Input is stamped with the frame on screen as it came (`Shown`, from the presentation
  timeline, below). The UI routes a click by the frame the user saw, not the one being drawn.

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
  - the frame's hit-test data, all of one frame: its hit table, its property trees as composed, its scale and
    the layers the renderer drew it with (`scene::LayerStage`: each layer's map into its parent target, its outer
    layer and its output clip). The tables are `Arc`s shared while unchanged. The property trees are the frame's
    own copy, kept in a recycled `Presented`: it takes in only the entries the composes since its buffer's frame
    wrote (each compose records them by id, as cc carries changed-node lists rather than whole trees), so the cost
    is what changed (`compose_alloc::a_frames_property_trees_copy_what_changed_not_the_trees`: a radius change over
    20,000 clips copies a few entries, a compositor scroll a few and allocates nothing). The stages are set when the
    frame is drawn, before it is handed to input; a page taller than a texture publishes the layers its banded
    capture drew.
- The snapshot follows the frame on the display, through one presentation timeline (`compositor::OnScreen`). The
  compositor owns it and the platform thread reads it. It is append-only:
  - The render thread enters a frame at the vblank it was drawn for whether or not pixels changed: a present made,
    or a draw with nothing to blit (its pixels are those on screen). A commit that changed only what takes the
    pointer is on screen from that vblank all the same, and a click after it lands on the new box
    (`thread::tests::a_commit_that_changes_only_what_takes_the_pointer_shows_from_the_next_vblank`). Tiles still
    rastering do not hold it back: the hit table is the frame's, not its pixels', as cc builds a frame's
    `HitTestRegionList` from the active tree whether or not its tiles are ready (`layer_tree_host_impl.cc:3892-3895`)
    and a checkerboarded frame (`frame_data.h:49-50`) is hit-tested like any other. A present
    that could not be made (no texture) is owed and enters nothing, so it never pushes out the frame on screen
    (`thread::tests::five_failed_presents_leave_the_frame_shown_stamped_and_kept`,
    `failed_presents_never_push_out_the_frame_on_screen`). Without a window, each frame drawn is the image.
  - A frame entered is never changed, and its number is given as it is published: a snapshot with other contents
    is another frame. A capture drawn with other layers (a page taller than a texture, read back in bands) is a
    copy with a new number and an empty record of what it wrote; the frame drawn before keeps its own
    (`clip::tests::on_a_page_taller_than_a_texture_input_reads_the_layers_its_capture_drew`,
    `clip::tests::a_capture_aside_leaves_the_frame_presented_as_it_was`). Entries go forward in number and vblank
    (a debug assertion).
  - What showed at a time `t` is a pure query, `OnScreen::at(t)`: the last frame entered whose vblank had come by
    `t`, not the frame whose `present` returned (`a_frame_presented_shows_from_its_vblank_not_from_its_present`).
    A display keeps its image until the next is latched, as SurfaceFlinger keeps a layer's buffer until a new one is
    ready (`Layer.cpp:1470-1483`). Asking about a later time never changes the answer for an earlier one: a wheel
    stamped before B's vblank and handled after it lands in A.
  - Retention is by time. Every frame shown within `RETAIN` (250 ms) of the newest frame's vblank is kept, plus the
    one showing at its start, each a whole `Presented`, and never more than `KEPT` (64: a quarter second at 250 Hz).
    Event Timing reports interactions slower than 104 ms by default and INP calls 200 ms or less good, so input
    later than a quarter second is a stall, not the norm. A count of frames would mean 17 ms at 240 Hz; a time
    holds at every rate (`a_wheel_after_a_50_ms_stall_at_240_hz_still_routes`). A time older than every frame kept
    answers `Shown::Gone` (`a_wheel_is_routed_in_any_frame_kept_as_it_showed_and_one_no_longer_kept_nowhere`).
  - Input from a frame gone is never a hit. A press, a wheel, a button or a touch that starts is dropped, not taken
    for a press on nothing, which would dismiss what opened since
    (`app::tests::a_press_from_a_frame_no_longer_kept_does_not_dismiss_what_opened_since`). A move only has hover
    worked out again in the frame on screen. Input that is not over anything (keys, a release ending a captured
    press) goes on. Limit: a UI stalled longer than a quarter second while the compositor keeps presenting drops
    the positional input it held.
  - A render thread made again after a panic goes on with the same timeline, numbering its frames after the last
    presented, so no stamp names two frames.
  - There is no per-present feedback from the GPU API (wgpu exposes none): the vblank a present was aimed at stands
    for it, so a present the GPU finishes late is taken as shown up to one refresh early.
- The stamp is a type, not a number. Input carries `Seen::BeforeFirst | Frame(id)`, which holds nothing, and every
  queue and message carries only that. It is resolved on the timeline as the input is handled, into
  `Shown::BeforeFirst | Frame(Arc<Presented>) | Gone(id)`. `FromRender::Presented` carries nothing and is sent once
  until the UI looks (`OnScreen::tell`/`look`). Frame number 0 means nothing.
- Input is routed against that snapshot, not the frame laid out since: Chromium's viz keeps a hit-test region
  list per `CompositorFrame` for the same reason. One rule (`clip::hit_in`) answers for a click, the devtools pick
  and the wheel's first target: a point hits a box where the device pixel shows at least half of it, read through
  each resampling layer as the composite samples it (`presented_routing::input_before_b_is_presented_lands_as_as_pixels_show_by_click_devtools_and_wheel`,
  `clip::tests::input_before_a_frame_is_presented_lands_by_the_frame_shown`). The texels of each level are measured
  once, so the cost is the texels each level holds under the point, not four to the power of the depth
  (`clip::tests::twelve_resampling_layers_measure_each_texel_once`).
- An event point is mapped into its node through the frame it was routed in, never through the live tree:
  - one owner (`InputState::placing`) undoes that frame's composed matrices to the box the frame showed the node
    as, for a click, a span, a text offset and a drag alike
    (`interact::tests::a_click_routed_in_a_frame_maps_into_the_span_that_frame_showed`);
  - Blink records a hit's local point in the same walk that picks the node (`hit_test_result.cc:210-219`) and maps
    between spaces through one mapper over the property trees (`geometry_mapper.h:43`).
- Text is hit in the frame that showed it, as its box is. Chrome, Flutter and Android hit text in the layout as
  it is when the event is handled (`local_frame_view.cc:2402-2409` brings the lifecycle up to date before a hit),
  and we diverge from them here on purpose. Hitting the box in the frame shown and its text as laid out since gives
  an answer that is in neither frame.
  - Each frame carries `Presented::texts`: for every box that shows text input hits into, a `ShownText`, either a
    paragraph's span boxes or a field's text, type, content box, focus and scroll, as paint laid it out.
  - Paint keeps each record in an `Arc` while it shows the same. A frame shares the table with the frame before
    until a text changes, and a change of text alone (its box the same) rebuilds the tables.
  - A span or caret hit for an event routed in frame F reads F's record through `interact::shown::Shown`. A node that
    F showed no text for takes no span and no caret. Under a frame no longer kept, positional input is handled as
    any other.
  - The records hold no shaped glyphs. A field's record shares its text with the edit (`Arc<str>`) and copies its
    `ResolvedText`; a paragraph's holds 20 bytes per span line box. The table is 16 bytes per entry, built only
    when the compositor's tables are. Steady frames allocate nothing (`text_alloc`, `compose_alloc`).
  - Tests: `shown_frames::a_click_queued_under_a_picks_the_span_a_showed`,
    `…_puts_the_caret_where_a_showed_the_text`, `…_picks_the_span_it_showed_when_only_the_spans_text_changed`,
    `…_puts_the_caret_in_the_text_that_frame_showed`.
- One rule decides what a hit reads (review r10). What was shown comes from the frame the event was routed in, or
  the frame on screen for hover: geometry, the text and how its offsets map to the value, the cursor, and which span
  is which. Only behaviour, the handlers, is looked up in the frame laid out, by stable identity, never by place. This
  extends Chrome's model, where a hit and the values it reads come from one up-to-date layout; ours is the frame shown
  instead.
  - Offsets: a field's record carries `edit::Offsets` (plain, masked, or a composition standing in for the selection),
    a pure function of what was shown. `TextEdit::unmasked`/`uncomposed` are that function too. A press queued under
    a masked frame and handled after the value is revealed, or after a composition was cancelled, puts the caret where
    the frame showed it (`shown_frames::a_press_queued_under_a_masked_frame_…`, `…_password_frame_…`,
    `…_clear_frame_…_after_masking`, `…_composing_frame_…`; without the record: 18 vs 6, 9 vs 3, 2 vs 6, 2 vs 0).
  - Cursor: each `Hit` carries the cursor its node showed (its own or its nearest ancestor's), each `ShownSpan` its
    span's. `InputState::cursor` reads the frame shown; a cursor changed alone shows when its frame does, as its
    pixels do (`shown_frames::a_cursor_changed_alone_shows_with_its_frame`). Paint rebuilds the tables when a visited
    node's resolved cursor changes, and the hover key (`compositor::Under`) includes the cursor, so such a frame wakes
    the UI under a resting pointer (`compositor::tests::a_cursor_changed_alone_under_a_resting_pointer_wakes_the_ui`).
  - Span identity: a span is `SpanId::Keyed` when the app gives it a key (`Span::key`), else its place (`SpanId::At`),
    as an unkeyed child is. The handler is the frame laid out's span of that identity; a span gone since takes the
    click and does nothing, as a node gone since (`shown_frames::a_click_queued_under_a_keyed_span_reaches_it_…`).
    Unkeyed spans keep positional identity: a span put before one since takes its place, as Compose and React treat
    unkeyed children.
  - A dispatch in another frame than the one on screen is scoped by a guard (`app::Routing`) that puts the frame on
    screen back as the dispatch ends, by return or by a handler's panic unwinding through it
    (`shown_frames::a_dispatch_that_unwinds_leaves_the_frame_on_screen`).
  - The pointer `next_change` weighs is the platform's latest; the UI's own may be a resampled move a frame behind it
    during a drag (`HeldMoves` samples 5 ms before present). The two differ by at most the motion of one frame, and
    while the pointer moves every frame brings a move that works hover out again, so the bound is one frame.
- One reader of what was shown (review r11). The rule above was written down in rounds 9 and 10, and each review
  still found a site that read the frame laid out where the frame shown was meant: in round 9 the hover key and
  the span, in round 10 the offsets, the cursor and the span identity, in round 11 a click's bounds, a removed
  node's cursor, the input method's caret and a text area's scroll copied at paint. A rule kept beside each
  reader is missed by the next reader, so the rule is now a type.
  - `interact::shown::Shown` is the only way input reads geometry and presentation. It covers boxes, event bounds,
    local points, the screen rectangle of a laid-out box, text records and offsets, scroll, the cursor and span
    identity. It reads them from one frame: the frame the event was routed in, or the frame on screen for hover,
    the cursor and the input method. With no frame shown (the devtools' frame as laid out), it reads the frame
    laid out as paint draws it.
  - What maps a point (`Anchor`, `Placing`) and the hit-table index (`HitKeys`) are private to that module. The
    frame laid out answers only identity and behaviour: which node a target is, and its handlers, field and spans,
    found by identity.
  - Event payloads come from `Shown::event`, one lookup giving the point and the box. The box has the size the
    frame showed, at the node's laid-out origin, so the point and its box belong to one frame
    (`shown_frames::a_click_routed_in_a_frame_gets_the_bounds_that_frame_showed`). Drag grips take their bounds
    from it too.
  - The cursor is `Shown::cursor_at`, a pure function of the frame on screen (`compositor::under`, the same rule as
    the hover key). It needs no node laid out now, so a node removed since keeps its frame's cursor until a frame
    without it shows (`…_removed_since_shows_its_frames_cursor_…`). A drag's cursor is `Shown::cursor_of` for its
    node.
  - A field's record (`ShownText::Field`) holds its content transform (`scrolls`). `ShownText::scrolled` reads
    the scroll from the property trees of the frame asked, as the content transform's offset from the field box's
    transform, both from that frame. Paint writes the content transform at the box's offset plus the scroll
    applied, and whatever moves the box (an ancestor's scroll, the compositor's) moves both alike, so the
    difference is the field's own scroll alone. A text area the compositor scrolled since its commit is hit as it
    showed (`…_text_area_the_compositor_scrolled_lands_where_it_showed`: line 1 instead of 4 before). So is one
    inside a container the compositor scrolled (`…_text_area_in_a_scrolled_container_keeps_its_text_…`, review
    r12: byte 15 instead of 1 when the scroll was measured from a base fixed at commit). The record also
    holds the caret, the selection and the composition range it showed. The input method's window follows the
    caret on screen and moves when a frame that moved it shows (`the_input_methods_area_follows_the_caret_on_screen`).
    Caret placement, drag-select and autoscroll read the same `Shown::field`. A field the frame showed no text of
    is placed nothing.
  - A span hit reads the frame's spans without the node's behaviour. Only the handler is looked up, by `SpanId`.
  - Hover and the key it was last worked out by change together. When a move or a leave unwinds, `app::Routing`
    drops the key as it puts the frame on screen back, so the next check works hover out again
    (`…_a_move_that_unwinds_leaves_hover_and_its_key_together`).
  - The input method's area is a value of the frame on screen, as the cursor is. The window's UI loop tells the
    platform both from one place (`window::ui_loop`'s `follow`), after a frame is built, after each input and
    after each idle wake, when either differs from what it last told. A frame only the compositor made that comes
    on screen as the UI idles moves the candidate window too
    (`window::tests::the_ui_wakes_at_the_vblank_a_frame_entered_shows_from`, review r12: told nothing before).
  - Working the area out is O(1) while the field shows unchanged (review r13: `follow` runs on every wake, and
    laying a 20,000-line field out on each cost 3.81 ms per call in release). What the field's text alone
    decides is kept by the record it was worked out of (`interact::ImeText`): where the text sits from the box's
    corner, unscrolled, and the caret with its composition, or the selection on the caret's line, in the text.
    A record is shared, unchanged, by every frame that shows the field the same, and a new one is made when its
    text, caret, selection, composition, box or type changes, so the same record means the same answer. It is
    held weakly, so its address is never another record's. The box and the scroll are read from the frame on
    screen each time, as `input_text` places the text linearly in both. The frame laid out (no record) is not
    kept. Measured by `shown_frames::an_unchanged_input_method_area_is_not_worked_out_again`, 20 equal asks
    after the first, Apple M4: without the record kept, 1.02 s at 20,000 lines against 2.0 ms at 20 lines
    (debug); with it, 66 µs and 129 µs (debug), 7.4 µs and 4.6 µs (release), about 0.4 µs per ask.
- One rule for hover (review r12): commit, then tell. Every hover entry point (a move, a leave, a converge or
  rehover, a drag following its node) writes everything hover keeps as one step before any user code runs: the
  nodes' hovered flags, the hovered paths, the marks the next change is found by, and the key `checked` (written
  by `converge` and `input_shown` before they dispatch). Only then does it run the handlers that change tells,
  from a list gathered with the write (`InputState::commit_hover`, `tell_hover`). A handler that panics leaves
  hover whole as written, and it is not called again for the same change. The next change tells the next
  handlers, so a hover entered under a panicking handler is still left, with its handler told
  (`…_a_hover_handler_that_panics_leaves_hover_state_whole`,
  `…_a_converge_whose_hover_handler_panics_hovers_on_the_next`). A drag following its node already writes the
  point it last saw (`Pointer::seen`) before it calls the drag handler. `app::Routing` keeps dropping the key on
  unwind only as a defence. Rounds 10, 11 and 12 each found this class at another entry point, because each
  ordered its own writes and callbacks. The F1 reactive work reached the same principle independently: no user
  code runs between state steps that must agree.
- The hover transaction (review r13). Round 12's rule missed two kinds of user code. Writing a flag is a signal
  write, and a signal write runs the effects reading it at once, so an effect ran between two writes of the
  commit and saw half a chain (`shown_frames::an_effect_on_hover_sees_the_whole_chain_and_cannot_tear_it`:
  `(true, false)` after leaving, before the fix). And a handler that panicked stopped the handlers after it, so
  an enter gathered in the same change was never told, and no later change tells it
  (`…_a_panicking_leave_does_not_skip_the_next_enter`: B never entered). The order is now:
  1. Commit every write hover keeps (flags, paths, marks, `was`) in one `reactive::batch`. Effects run once, as
     the batch ends, and see the whole chain. One that panics finds hover whole.
  2. Tell every gathered handler. Every one is told, though one panics, and the first panic goes on once the
     dispatch is done. Chrome does the same: it reports a listener's exception and dispatches to the next
     element and the next phase.

  The pressed flags (`write_pressed`) and the focus flags commit in one batch with their paths, marks and
  chain the same way.
- One dispatch is one transaction for the user code it reaches (review r13 addendum: close the class, not the
  site). The two PoCs above were the last of a series in which each round found another ordering point in
  another dispatcher, so the rule is held in one place, `engine::dispatch`, rather than at each site. Round 13
  still placed it at the public entries (`input_shown`, `converge`) and caught at `in_owner`; review r14 found
  each of those one level too high (the devtools' `Showing::send` reached `input_on` with no dispatch; a
  layer's dismiss answer, a scroll handle's reveal and the other hooks reach user code through
  `engine::call_out` without `in_owner`). The final rule (review r14), after research into how the references isolate
  listeners, nest dispatch and keep focus (the DOM's listener loop as Ladybird implements it,
  `DOM/EventDispatcher.cpp:72,141`, and its microtask checkpoint, `HTML/Scripting/Environments.cpp:229`;
  Flutter's `PointerRouter` and `GestureBinding`, `pointer_router.dart:93-118`, `binding.dart:506`, and
  `FocusManager.applyFocusChangesIfNeeded`, `focus_manager.dart:2083-2101`; Compose's
  `PointerInputEventProcessor` and focus owner per `Owner`):
  1. **One turn owns every entry from outside** (review r15: a rule each site opted into left every site that
     did not outside it). `engine::turn(window, work)` (`engine/turn.rs`) is the only way work from outside
     reaches the UI thread: it enters the window (the one guard, rule 4) and opens the dispatch. The pipeline
     reaches it through `Pipeline::turn`, which first takes in what the compositor scrolled that was handed on
     with the input or frame (`take_pulled` only queues it), so the pull and the work it rides with are one
     turn and a dismiss answer that panics as the scroll is taken in drops none of the input. The turns:
     input (`input_shown`, `input_on`: the platform's, a scenario's, the devtools'), a frame (`draw_with`),
     hover converging (`converge`), mail (`mail_waiting` for the platform and a replay, the frame's delivery
     in `Scenario::render_for`), timers (`idle`, and `Scenario::at`, `finish_raster` and `compositor_wheel`
     advancing the clock), a caller's act (`act`), the window's activation (`set_window_active`), what the
     platform told (`Scenario::told`), accessibility (`access_update`, `access_tree`, `access_probe`), the
     IME's target (`ime_target`) and a devtools command (`devtools::execute`). Making a window makes nothing
     current (`FocusRoot::new`): only its turns do. A turn inside another (input sent to a second window from
     a handler) shares the outer dispatch. `engine::dispatch` and the window guard are private to the module.
  2. **One gate catches.** Inside a dispatch, `engine::call_out`, the gate every caller's closure is reached
     through (ADR-0028), catches its panic, hands it to the dispatch and answers `None`: no answer. A handler
     answers as no handler would (`R::default()`: a key not taken goes on up), a dismiss answer is a request
     not taken (the press outside goes on to its target, focus moves, it clicks, as a DOM listener that throws
     has not called `preventDefault`), a hook asks for no pass. `in_owner` and `Callback::call` tell this
     `None` from a fork's by `engine::forking`. After the catch, what the closure's own batch left queued as it
     unwound runs (`reactive::flush_unbatched`), as the web runs its microtask checkpoint after a listener
     however it ended; the dispatch's end runs it too.
  3. **One arbiter, total over unwinds, first in time.** The turn keeps the first panic handed to it
     (`raise`), whatever reaches it: the gate's catch, the turn's own catch of its work and of its closing
     flush, a render's (`attempt`), and every place that runs several pieces of user code though one panics
     (the flush's effects, the hooks after a layout, cleanups, an unmount's children and scopes, effects run
     first after a batch, a members list's watchers, mail values, timers), which runs each under
     `engine::Gather` and hands each panic to the arbiter as it catches it, not after its loop: a later effect
     may dispatch to another window and panic there, and the first in time must stay first. A later panic's
     payload is dropped outside the arbiter's borrow, under a catch of its own (`drop_quietly`): user data
     whose drop reaches the gate again neither aborts nor replaces the first. No other place catches or raises
     an unwind of the UI thread: `tests/dispatch_kinds.rs` (`catching_lives_in_the_engine`) fails one outside
     the engine module unless a reviewed list names it with its reason (the render, list worker and canvas
     planner threads, which no turn runs on; the seeded test harness; a replay fixture; F1's check verdicts).
     The DOM and Flutter report and never rethrow, so they do not meet this; we rethrow, so a test or a
     caller sees it. Outside a turn (a unit test driving the runtime directly), each place raises its own
     first once all have run.
  4. **A window's ambient state is scoped, and work keeps the window it was scheduled in.** The current window
     (its focus, its ring, and its own ambient signals, `focus::WindowSignals`: where the pointer is, whether it
     is active, its size, whether it can be seen) lives in a module of `focus.rs` of its own (`cx`), written only
     by the guard (`enter_window` → `Entered`, which needs the engine's `Entering`, made only by a turn and by
     `Here::within`) and by a window closing; `the_window_current_is_written_by_the_guard_alone` checks it. A
     turn's guard puts back the window current before when it ran inside another's code; at the top, its
     window stays current. Work scheduled in a window (a timer, an effect, a mailbox's handler) captures the
     window current where it is scheduled (`engine::here`, kept next to its reactive owner) and runs in it
     (`Here::within`, which always puts back the one before), whichever window's turn runs it. Modality and
     reduced motion are the user's, one per app. Focus is per document, binding or owner in all three
     references; ours is per window too, and so is what was thread-wide.
  5. **Commit is the only writer of engine signals.** Focus and its ring, the modality, a node's flags, a
     window's pointer, activation, size and visibility, and reduced motion are `engine::Owned` signals: anyone
     reads them; `put` takes the `Commit` that only `engine::commit` makes, so a write outside a commit step
     does not compile. A step that writes several (focus with its ring and the modality, `focus::set_focus`;
     focus with the selection of the field it lands in, `engine_focus`; the focus chain; press; hover) is one
     `reactive::batch`: what reads them sees all or none, Flutter's commit-then-notify. In a debug build an
     effect that runs while a commit step runs panics (`flush` asserts `!engine::in_commit()`).
  6. **A failing component does not blank the window** (review r15, superseding the round's first answer,
     which tore the root down and showed nothing for the rest of the turn). The user-visible goal decides
     it: a panic of user code inside a render or a layout costs the one piece that failed, never the rest
     of the window. The references: Chrome keeps showing the page when a script throws, and goes on with
     the event loop; Flutter's `ComponentElement.performRebuild`
     (`references/flutter/packages/flutter/lib/src/widgets/framework.dart`, the `catch` around `build()`)
     puts an `ErrorWidget` in place of the one element whose build threw, keeps its state, and builds it
     again when it is next marked; React unmounts the whole root on an uncaught render error, the failure
     this rule exists to avoid. The pieces, each held by what failed alone, in a turn:
     - **A render** (any instance's, the app's root among them; a list's rows and a `Derive` run inside
       one) is pure: one that panicked changed nothing of its instance but what it made in its own render
       scope, disposed at once. The instance keeps its last output (none before its first render) and its
       state, and renders again when what it read before the panic changes (`component::render`,
       `Fault::Render`). We keep the last output where Flutter draws an error box: the last output is still
       a true view of the instance's state as it last rendered, and its handlers still answer, so the
       window looks and acts as it did (Chrome's page after a throw).
     - **Props compared** (user `PartialEq`, run as a parent passes them) that panic leave the child its
       props and its output; the next props passed reach it (`Fault::Props`). It is never mounted again for
       this: a mount compares nothing and would keep the props the comparison refused, for the next to
       fail on (the coordinator's r15 report, `components.rs`).
     - **A setup** that panics leaves an instance half made: that instance alone is torn down, its slot
       empty, its siblings and parent drawn; the slot mounts anew when its parent's output is expanded
       again (`Fault::Setup`).
     - **A layout's bindings** (`Value::Bound`, a bound text, a drawing's `make`) that panic hold what they
       last gave (a style, a text, a drawing; nothing before their first value) and run again when what they
       read before the panic changes (`layout::binding`); a bound Tab stop answers as no closure would, a
       stop (`TabStop::get`).
     Each panic goes to the arbiter as it is caught; the turn goes on, and its input is routed into the
     window as it is. What a panic left held is part of its root's state (rule 12) and the cold check
     reuses it as opaque, never switching off. Outside a turn (a unit test driving a root directly) and in a fork, a panic goes on to the caller
     and the root is torn down (`Root::resolve`), as before. A panic of the engine's own inside a resolve or
     a layout (no user code left outside these) tears the root down for the rest of the turn
     (`engine::ThisTurn`), the frame stale, mounted again at the next turn's refresh.
     Tests (`tests/dispatch_turn.rs`): a root render that panics keeps the window as it was; one component
     keeps its last output while the rest draws; props whose comparison panics leave the child as it was; a
     child whose setup panics leaves its slot empty and the rest drawn; a bound text keeps what it gave; a
     binding panicking as input lays out drops no input; a bound Tab stop that panics drops no Tab.
  7. **The kinds are structural.** The gate takes the kind of user code it reaches (`engine::Site`: every
     handler registration, a span's click, a dismiss answer, the scroll and list hooks, a members watcher, a
     callback, a timer, a mailbox's handler, a hook after layout, an effect, a cleanup), declared with its
     list (`Site::ALL`) by one macro. Effects and cleanups, which the reactive runtime runs of its own, pass
     the same gate's turn half (`engine::reach`: the runtime, not the gate, says whether a fork runs one), so
     every path that runs one (an effect's first run in a turn's work or after the setup that made it, a
     rerun by a user's write or in an engine commit's flush; a cleanup before a rerun, as an instance
     unmounts, as an effect is disposed) goes on past its panic. The fault harness matches the kinds without
     a wildcard (`case`), so a kind added does not compile there until it has a case,
     `sites_are_all_in_the_harness` fails until the script reaches it, and every runtime path is a case
     of its own (`RUNTIME_PATHS`).

  Review r16 found rules 1, 4 and 6 still held by convention at the places that mattered: the live UI loop
  wrote the window's size and visibility with no turn (`window.rs`, while the replay's `Scenario::told` had
  one, so live and replay differed), the app's setup ran before its window existed, a window's close ran its
  unmount hooks after it had stopped being current, a timer kept its closed window alive and re-entered it,
  hooks after layout and cleanups carried no window at all, one held fault anywhere switched every window's
  cold check off, and `Derive` was missing from the hand-declared kind list. Each was a list or a convention
  kept at a call site; rules 8 to 12 make each a type or a single constructor, so a new site cannot miss it.
  The alternative, adding the missing sites to the lists, is what rounds 14 to 16 did; each round's review
  found the next site.

  8. **A turn is a capability** (`engine::Turn`, a zero-sized value only `engine/turn.rs` makes, lent by
     reference to a turn's work). `engine::commit` takes `&Turn`, so every write of an engine-owned signal
     (rule 5) is inside a turn or does not compile. The platform's writes go through the window's own turn:
     `WindowHandler::resized` and `visible` (`Pipeline::told_size`, `told_visible`), `chrome` (activity and
     decorations in one turn), `focused`, `renderer_lost`, `take_gesture` and `hand_gesture` each open the
     pipeline's turn, and the replay (`Scenario::told`) writes through the same `put_*` functions with the
     turn it opened, so live and replay are one path. The public setters (`environment::set_window_size`,
     `set_window_visible`, `set_reduced_motion`, `focus::set_focus`) are for code that runs inside a turn
     (a handler's); called from outside any turn, they open a turn of the thread's own window
     (`engine::in_turn`, rule 15), never of the window a turn last ran in.
     Nothing runs bare: `reach` with no turn under way opens one, and so does the reactive runtime's flush
     of effects a write outside any turn queued (`reactive::flush`). The app's setup runs in its window's
     first turn, after the window exists (`Pipeline::made`, which `live` uses), so its ambient reads and
     what it schedules are that window's (Opus r16 I2: it ran against a stand-in window that the first
     turn replaced and disposed, leaving its effects on dead signals). Code with no window current runs in
     the thread's own window (`focus` `NOWHERE`), made once and never closed or replaced. It is made only
     when its state is read: work kept or run with no window current names none (`Here(None)`), and a turn
     opened with none current enters none, so the reactive core used alone allocates no window's signals
     and its graph counts and the recompute oracle see only the user's nodes.
  9. **A window owns what is scheduled in it, and its close is one of its turns, in a fixed order.** Every
     piece of user code the engine keeps to run later is made by one constructor, `engine::Stored::new(kind,
     f)`: timers, effects (`Effect::new`), cleanups and unmount hooks (`on_cleanup`, `on_unmounted`),
     hooks after layout (`after_layout`, `on_mounted`, `on_updated`, the stage hooks) and mailbox handlers.
     It registers the kind (rule 11), keeps the window current at that moment weakly (work never keeps a
     closed window alive, Opus r16 I4), and runs the work in that window, inside a turn (`Stored::run`, or
     `call`, which never runs in a fork). Once the window has closed, the work does not run. `Here` is
     private to `turn.rs`; the engine's own bookkeeping cleanups (a timer cancelled, a mailbox retired, a
     memo freed) use `reactive::on_release` and run as their owner ends. `Pipeline::close` (also called by
     `Pipeline::drop`, which logs and never panics) is the window state machine of rule 13: one turn of
     the window in which the root unmounts and the close settles to a fixed point, every window-owned drop
     inside it.
  10. **The arbiter orders panics by when their unwind began.** A panic caught while the thread is already
      unwinding (`std::thread::panicking()`) came from a destructor that the earlier unwind ran, so it began
      later. It always loses: it is dropped quietly and never resumed, even if the earlier panic has not yet
      reached the arbiter (`raise`, `caught`). Proof: Rust runs destructors only during an unwind that has
      already begun. A panic raised in one of them while `panicking()` is true therefore began after the
      unwinding panic, and every panic caught when `panicking()` is false began before any panic that is
      still unwinding at that point. So ordering by arrival, with secondary panics always losing, is
      ordering by start time. Rust aborts when a panic leaves a destructor during an unwind; we keep the
      first panic, as Rust does, and drop the second without aborting. Tests: Codex r16 2, and the same
      inside a nested turn (`tests/window_owns_work.rs`).
  11. **A contained getter runs once per pass, and failure has a defined value.** A binding's replay writes
      only what its mount gave in this pass. A held write that never gave a value is absent: the property
      keeps what the earlier writes left. Replay never falls back to running the closure again (Codex r16
      1: a first-use width panicked twice and left the window empty). The kinds come from the type:
      `engine::stored_kinds()` reads the registry that `Stored::new` and `Derive::new` fill, so
      `every_kind_the_engine_stores_is_a_case` fails for any kind the script stores without a case of its
      own. `Site` gains `Derive`.
  12. **Holds belong to their root, and a hold never switches the oracle off.** What an instance holds
      (`component::Holds`: its render, its props, the slots whose setup panicked) is a field of the
      instance, so it belongs to its root and is forked with it. One window's hold means nothing to another
      window (Codex r16 7, Opus I5). What a node's bindings hold (`layout::binding::HeldNode`: a style, a
      span's style, a text, a drawing) is read from the live layout tree by path. The cold rebuild treats
      every held piece as opaque: it reuses the live held output, props, empty slot or value and never
      runs that user code again (`component::opaque`, `LayoutTree::held`). Everything else in the tree is
      checked, so a sibling's missed dependency is still caught while another sibling is held, and a held
      render with no dependencies leaves the check on. A cold frame changes no hold. `Skipped` no longer
      happens because of a hold. Any remaining skip (a fork that reached a site that cannot answer inside
      one) is counted, and more than `SKIP_FLOOR` (32) in a row fails the check, so a checker that only
      ever skips checks nothing.

  Alternatives considered, per rule:
  - Rule 8. Keep wrapping each entry point in a turn by convention (rounds 14 to 16): each review found an
    entry with none (the devtools, the live loop's `Told` arms, chrome, setup). Or let `reach` keep a bare
    mode outside a turn: user code would again run with no arbiter. The capability makes a missed entry a
    compile error, and `in_turn` gives what has no turn one, so no bare mode remains. Rejected too: a debug
    panic for work kept with no window current. The reactive core is used alone (its tests and the
    recompute oracle make effects with no window); the thread's own window (`Here(None)`) is a defined
    owner, never "whatever window is current when it fires".
  - Rule 9. Attach a window to each queue (timers had it, round 15): hooks, cleanups and unmount hooks did
    not. `Stored::new` is the one constructor, and `tests/kept_closures.rs`
    `every_closure_kept_is_stored_or_said_why` (rule 18) fails on a kept closure that neither is `Stored` nor
    says why it runs only inside its window's turn. Holding the window by `Rc` (round 15) kept a closed
    window alive and let its work re-enter it. Finding dead work when it fires, rather than cancelling it
    at close, would keep what it captured alive until then (`a_timer_of_a_window_closed_but_still_held_never_runs`).
  - Rule 10. Order by arrival at the arbiter (round 15): a destructor's panic during the first unwind
    arrives first and won. Aborting as Rust does on a double panic would take the app down for a panic in
    user cleanup.
  - Rule 11. Fall back to running the closure again when its mount gave nothing (round 15): the getter
    ran twice in a pass, uncontained the second time, and took the root down.
  - Rule 12. Skip the cold check while anything holds (round 15): one held fault switched every window's
    check off for good. Per-root skipping would still leave the held root unchecked; the opaque rebuild
    checks everything but the held pieces.

  Review r17 found that a window's life was still a set of steps and saved pointers rather than one state
  machine. Close ran in phases, so work a cleanup scheduled after its phase (a timer, mail, a hook after
  layout) outlived the close (Codex 1, 2; Opus I2). Turn guards saved the `Rc` of the window they entered
  from and put it back, so a window closed inside a nested turn became current again (Codex 3, Opus I1).
  Code outside any turn ran in "the last" window. A nested close's panic was lost to the outer arbiter.
  Rule 12's whole-node style hold hid a stale height under a held width. Rules 13 to 17 replace each of
  these with one mechanism.

  13. **A window is a state machine: Open → Closing → Closed, and close is one turn run to a fixed point.**

      ```
      Open ──Pipeline::close──▶ Closing ──its close turn settled──▶ Closed (no record)
       │                          │  round: dispose owned effects (their cleanups run),
       │                          │  cancel timers, hooks after layout, mailboxes, end
       │                          │  WindowOwned; what a round's user code schedules in
       │                          │  the window joins the next round
       │                          └─ past focus::CLOSE_ROUNDS (64) rounds: a fault of the close
       └─ work registered: kept in its record
      ```

      A window's record (`focus::WindowWork`, `[Fork(log)]` in `WINDOW_WORK`) holds its life (`Life::Open`
      or `Closing`), the effects it owns and its `WindowOwned` obligations. Closed is the absence of the
      record. `Pipeline::close` is one `engine::turn_own` of the window: `begin_close` (Open → Closing,
      `false` if it was already closing or closed, so a second close or a close from inside its own close
      does nothing), the root unmounts, then `settle_close` loops `close_round` until a round finds
      nothing. Each round runs under one `Gather`, so every piece of user code a round runs (and every drop
      of what it scheduled) runs inside the close turn, its panics ordered by the turn's arbiter. Then
      `finish_close` removes the record, strikes the window off the stack (rule 14) and ends its signals.
      A close that does not settle within `CLOSE_ROUNDS` is reported as a fault of the close ("did not settle
      in 64 rounds"), never a hang. In a debug build, removing a record after a settled close that still
      holds live effects or obligations panics.
      Registration follows the state. Work registered in an Open or Closing window joins its record, and a
      Closing window's next round takes it. Work registered with no record (Closed) is dropped at once by
      its registrar: `own_effect` disposes the effect, `owned_by_window` refuses the obligation. A Drop
      that reads window state while a close drops it runs inside the close turn and sees the closing
      window: the `Says` probe reads `window_visible` from such a Drop and gets the closing window's
      hidden state, not another window's (`cancellation_drops_timer_capture_in_its_window`,
      `work_scheduled_as_a_window_closes_is_dropped_by_the_close_in_it`). Work handed to an already-closed
      window is dropped where its caller runs, never in the closed window: the thread window at a test's
      top level (`closed_pipeline_rejects_new_work`).
      **A Closed window is never entered again.** `engine::turn` and `turn_own` on a closed window drop
      the work unrun and answer `None` or `Err(None)`. `Stored::run` and `Here::run` find no window (the
      weak handle fails to upgrade) and run nothing. The public entries `Pipeline::act`, `read` and every
      `turn_with` user drop the work and debug-panic ("a turn of a closed window"), because using a closed
      pipeline is the caller's bug. In release they do nothing.
      Tests (`tests/window_owns_work.rs`):
      - `cancellation_drops_timer_capture_in_its_window`;
      - `close_disposes_effect_made_by_cleanup`;
      - `work_scheduled_as_a_window_closes_is_dropped_by_the_close_in_it`;
      - `a_close_that_never_settles_is_a_fault_not_a_hang`;
      - `closed_pipeline_rejects_new_work`.
  14. **One engine-owned window stack; guards hold no window.** `focus::cx::STACK` is a `[Fork]`
      `Cell<Stack>` of plain entries: `Thread`, `Window(salt)`, `Gone`. Each turn pushes its entry through
      `enter_window` or `enter_thread`. The guard (`Entered`) holds only its index, and dropping it on
      return or unwind cuts the stack back below it. It restores nothing it saved, so it can never bring
      back a window. The top entry, skipping `Gone`, is the current window. A salt finds its window through
      `WINDOW_OF`, a `[Shared]` weak index written only by `FocusRoot::new` and its `Drop`. A close strikes
      every entry of its window to `Gone` (`cx::forget`), so a pop that would expose a closed window
      exposes the next open entry below it. Implicit turns push too: `in_turn`, `Here::run`, and
      `Stored::run` entering their own window inside another's turn. The stack is bounded (64 entries);
      deeper nesting is runaway recursion and an engine fault. Because the stack is plain data in a
      `[Fork]` cell, a check's pushes and strikes are undone by the switch.
      Tests: `nested_turn_restores_thread_window` (Codex 3),
      `a_window_closed_inside_a_scheduled_entry_is_put_back_current` (Opus I1) and
      `a_window_closed_but_held_inside_another_turn_is_never_current_again` (the strike: a closed window
      whose handle still upgrades is never the top again).
  15. **Outside work runs in the thread window, never in the last one.** Code that runs with no turn under
      way (a test's top level, a background callback reaching the UI thread outside a dispatch) runs in
      the thread's own window, `NOWHERE` (salt 0, never closed). That covers its reads
      (`focus::focused`, `environment::window_visible`, and the rest) and its setters
      (`environment::set_window_visible` and the others open a turn of the thread window). Window setters
      called with no turn are not a debug panic; they write the thread window. The reactive core and its
      tests use setters with no window, and a debug panic there would make the thread window unusable. A
      test that reads a window's state names the window: `Pipeline::read` and `Scenario::read` run a read
      in one turn of it. `focus::window_id()` returns the current window's id, `WindowId(0)` outside any.
      Test: `an_outside_write_lands_in_the_thread_window` (two windows plus an outside write; neither
      window sees it).
  16. **Close reports its own panic, nested or not.** `Pipeline::close` returns `Err(message)` for the first
      panic of its own turn in every case. At the top level, the panic is dropped after its message is
      taken. When nested inside another turn, the close turn has its own arbiter slot (`dispatch_own`
      swaps `TELLING`), so the close's panic is not lost to, or merged into, the outer turn. After the
      close the panic is also raised to the outer arbiter, so the outer turn reports it once, as its first
      if it was first.
      Test: `close_reports_its_own_panic_at_the_top_and_nested`.
  17. **Holds are per setter.** A node's held style is no longer one whole-style flag. `HeldNode.style`
      holds `(run index, mount snapshot)` pairs, one per binding run whose closure panicked
      (`StyleBinding::held`, `Mounted::snapshot`). The replay writes each held run's snapshot in its place
      and evaluates every other run fresh, on the live path and in the cold rebuild alike. A held width no
      longer hides a stale height on the same node.
      Test: `a_held_width_hides_no_stale_height_on_the_same_node`. It reproduced the r17 plausible finding
      before the fix ("differs from a cold one" was not raised).
  18. **The kept-closure lint is by type.** `tests/kept_closures.rs` parses the gui, primitives and shadcn
      sources with `syn` and flags:
      - fields of structs, enum variants and statics whose type holds `dyn Fn`, `dyn FnMut` or `dyn FnOnce`
        outside a `Stored<…>`;
      - type parameters of a type definition bounded by `Fn*`, in the generics or a where clause;
      - type aliases to either;
      - statics declared inside a macro (`engine_local!`, `thread_local!`) whose tokens hold `dyn Fn*`.

      Test modules and `tests.rs` files are skipped. Each hit names its item (`crate:file: Type.field`),
      not its line, and must appear in `ALLOWED` with a reason (handler, pure read, render, layout hook,
      borrowed, or engine plumbing, each said). An entry that matches no hit fails.
      `the_lint_finds_each_kind_and_passes_stored` checks every kind and the `Stored` exemption.
      The tooltip closers (`primitives` `CLOSERS`) run in their own window. The open tooltip is kept per
      window (`OPEN` keyed by `window_id`), so opening a tooltip closes only the one open in its own
      window, in a turn of that window.
      Test: `a_tooltip_opened_in_one_window_closes_none_of_another`.

  Alternatives considered (r17):
  - **Saved-pointer guards** (rounds 15 and 16): each guard saved the window it entered from and put it
    back. That was correct until a close happened while the guard lived. The guard then restored a closed
    window, and every fix added another "is it closed?" check at a restore site. An engine-owned stack
    whose entries a close strikes has no restore site to miss.
  - **A single-pass close** (round 16, phases in a fixed order): any work a later phase's user code
    schedules in an earlier phase's queue outlives the close. A fixed point covers what any cleanup
    schedules, however deep, with a bound so a runaway is a fault rather than a hang.
  - **The last window as the default**: convenient for single-window tests, but outside code then writes
    whichever window ran last, which depends on history. The thread window is a fixed, named owner.
  - **A debug panic for window setters outside a turn** was rejected for the reason in rule 15.
  - **Text matching for the lint** (round 16) saw only the line's text. It missed generic bounds, aliases
    and other crates, and went stale on reformatting. Parsing the types fixes all four.
  - **Whole-node style hold** (rule 12 as first built): simpler, but hid any other setter's staleness.

  Revisit when: a kind of user code needs to outlive its window (a cross-window service), which would need
  an owner other than a window; or a binding's held value must be checked against a fresh run (it cannot
  be, while the closure keeps panicking).

  A panic of the engine's own unwinds at once, and what user code raised is dropped with it. The text
  settings a window reloads as it regains focus decide only how glyphs rasterize, never where text or its
  caret lies, so the IME cache's key needs none of them (`interact::ImeText`).

  Review r14's reproducers are regression tests (`tests/dispatch_rules.rs`): the devtools' hover tells the
  move after a panicking enter; a panicking dismiss answer leaves the press to go on; input sent to another
  window leaves this window's focus; Tab publishes focus and its ring at once; the first panic in time goes
  on. Each fails without its part of the rule.

  The fault-injection test `tests/dispatch_faults.rs` checks the class. Over a script of 23 inputs and frames
  (moves between siblings and along a nested chain, a click, a drag and its release, a secondary press, focus
  by a press and by Tab, a key, frames that move a node under a resting pointer so that `converge` rehovers,
  leaving and coming back), every call of user code the script makes panics in its own run. That is 116
  cases: `on_hover`, `on_pointer_move`, `on_mouse_down`, `on_click`, `on_release`, `on_secondary_click`,
  `on_drag` start, move and end, `on_key_down`, and effects reading each node's hovered, pressed, focused and
  focus-within flags (focus in and out). Each run is compared with the run where nothing panics:
  - exactly one input unwinds;
  - every callback, the one that panicked included, is told exactly what it is told without the panic, in
    the same order;
  - every node's five flags and the cursor end the same.

  Without the dispatch, 71 of the 116 cases differ (a panicking effect skipped the hover handlers after it, a
  panicking hover handler skipped the pointer-move phase, a panicking press effect skipped `on_mouse_down` and
  the drag start). With it, none differ.
- The compiler keeps dispatch off the frame laid out (review r13 addendum, which brought this forward from the
  open issues). A laid-out node's geometry fields (`bounds`, `content_box`, `space`, `space_content`, `offset`,
  `clip`, `scroll`) and the frame's `viewport` are `pub(in crate::layout)`: layout writes them, and every other
  reader goes through an accessor that takes a reader's token (`layout::Sight`, a sealed trait). Each reading
  module (paint, accessibility, clip, effects, the overlay, the devtools, the inspector) makes its own token,
  which only it can make. The input side's token is made in `interact::shown` alone: `interact.rs` reads what
  the screen showed through `Shown`, and the scroll model, keyboard editing and scroll-into-view read the frame
  laid out through the three `laid_` readers there (`laid_scroll`, `laid_content_box`, `laid_bounds`), each
  named for the model it serves. A dispatch that reads `n.bounds` does not compile (shown by a probe in
  `pointer_left`: `E0616`). Code outside the engine (tests, tools, the docs' scripts) holds the public
  `inspect::OUTSIDE`. Rust cannot make that one public item unreachable from inside the crate, so the source
  check below fails `interact.rs` naming any token.
  - The source check `tests/shown_reads.rs` is the second line. It fails `interact.rs` naming a token
    (`OUTSIDE`, `Outside`, `Sight`, `SIGHT`), and any function of `interact.rs` that reads a laid-out node's
    geometry outside `Shown`, unless an allowlist entry gives the reason. Geometry here means `bounds`,
    `content_box`, `scroll`, `viewport`, `offset`, `clip`, `space`, `space_content` and `look.transform`, read on
    any expression: a binding of any name, an index, a call or a chain (review r12; its fixtures include
    `frame.nodes[i].bounds`). A pattern of a frame type binding one, `FrameNode { bounds, .. }` on one line or
    several, is a read too, and so is a call of the clip and hit helpers (`hit_in`, `clip_box`, `hit_box`) or
    of a `laid_` reader, called or handed on by name (review r13). A field of another type spelled the same is renamed or listed with what it is, so the check
    fails closed. A listed read exempts only its own span of the line, not the rest of it. The allowed
    readers are the scroll model (the offset and range the UI holds, which wheels, autoscroll and the compositor's
    deltas step), keyboard editing, scroll-into-view and the devtools' pick before any frame. Like ADR-0028's lint,
    it reads text, not types.
- A hit costs `O(log N + k)`. Each transform space has an R-tree of its boxes, bulk-loaded by Sort-Tile-Recursive
  (Guttman 1984; Leutenegger, Lopez and Edgington 1997), built once per hit table. A second R-tree indexes the spaces
  by where they lie on screen. Its bounds are refitted once per frame, as dynamic BVHs refit (Wald, Boulos and
  Shirley 2007), so a hit composes chains only for spaces under the point. Measured on 20,000 boxes
  (`clip::tests::a_hit_and_a_miss_cost_the_logarithm_of_the_boxes_and_the_spaces`), bound `8·log2 N + 16k + 32`:
  - 400×1 rows: 57 candidates for a hit, 6 for a miss;
  - 1200×24 rows: 57 and 6;
  - 20,000 transformed controls: 108 and 57.

  The refit is `O(spaces)` at the first hit of each frame. It is exact against a walk of the table
  (`clip::tests::the_index_finds_what_a_walk_of_the_table_finds`).

  A refit keeps the shape of the last pack, so a frame is packed again (`O(S log S)`, in buffers of its own while
  older frames keep theirs) when that shape no longer fits it (`SpaceTree::fits`). This happens in two cases:
  - A space came onto or left the canvas: its bounds became finite, or stopped being. Otherwise the space would be
    neither in the tree nor among those mapped for every point, and a warm index would miss what a fresh one hits.
    This was round 9's Codex 4, pinned by `shown_frames::a_space_whose_bounds_become_finite_is_hit_as_a_fresh_index_hits_it`.
  - The refit's expected cost of a point in the window grew past twice the packed tree's, both weighed over the
    window the frame was drawn for (`Presented::viewport`; `clip::expected_cost`). The cost is
    `1 + Σ_nodes count(node) · area(node ∩ window) / area(window)`: the entries a query tests for a point drawn evenly
    from the window, which is what a hit counts as candidates. It is MacDonald and Booth's (1990) surface-area
    heuristic for points, with its assumption about where queries fall made explicit and ours: a pointer is only in
    the window. A changed window weighs the packed tree again under the new one.

    Round 9 weighed nodes over the current root's bounds instead. One space far off (x = 1e8, or -1e6) then made the
    root so large that nodes scattered across the window weighed nothing, and the refit was kept. That was round 10's
    Codex 3 and Opus I1, now pinned by `clip::tests::one_distant_space_does_not_hide_spaces_that_scattered` and
    `clip::tests::a_shuffle_with_one_far_space_is_packed_again`. Without the window weighting, a miss tests 21,335 and
    14,647 candidates, against at most 152 with it. In release, 1,000 misses took 610 ms and 413 ms with the refit kept,
    against 0.41 ms and 0.39 ms repacked (fresh index: 0.70 ms and 0.66 ms; counters on).

    The refit-or-rebuild structure is Lauterbach et al.'s RT-DEFORM (2006). The cost and the threshold of 2 are our
    own, checked by the counted tests:
    - Box2D's `b2DynamicTree_GetAreaRatio` (box3d `dynamic_tree.rs:808-829`) is a diagnostic that nothing in its
      maintenance reads.
    - box3d's hybrid rebuilds on a fixed cadence of 8 steps (`broad_phase.rs:153-172`). A cadence would repack rigid
      scrolls for nothing and still leave bad frames.

    Box2D's rotations and Kopta et al.'s (2012) would bound the cost per frame further, but they need a pointer tree
    instead of packed STR levels. A rigid scroll never repacks unless it carries dense content into a window that was
    sparse when the tree was packed: that costs one repack, and the window cost is then measured afresh.

  `clip::tests::spaces_that_scattered_since_the_tree_was_packed_are_packed_again` covers the second case. It packs
  20,000 spaces in a row 50 px apart, then moves them into two piles 200 px apart. A miss between the piles then
  tests 21,335 candidates refit-only, and at most 152 after the repack.

  Timings come from a release run of `shown_frames::the_space_tree_over_twenty_thousand_spaces_costs_what_adr_0015_records`
  on 20,000 spaces, on an M-series Mac shared with other builds, so the numbers are noisy by about 2×:

  | Stage | Time | Miss after it |
  |---|---|---|
  | First hit of the table (index built, frame placed) | 3.9–9.5 ms (round 10: 7.4 ms) | 0.13–0.32 µs |
  | Fresh frame scrolling all spaces (refit) | 0.3–0.8 ms (round 10: 0.51 ms) | 0.13–0.32 µs |
  | Frame scattering them (packed again) | 0.9–1.9 ms (round 10: 1.9 ms) | 0.11–0.34 µs |

  With the cost check removed, the scattered frame's miss takes 73 µs. Round 9's Codex 5 measured 17.6 ms against
  0.109 ms per 1,000 misses.
- There is no other hit rule. Before any frame is presented (`Seen::BeforeFirst`), every kind of input runs the
  same function on one snapshot of the commit, built by one constructor (`Presented::of`: its hit table, scroll
  containers, property trees and scale, no layer stage, since none was drawn) and keyed by the commit on either
  thread: a click on the UI thread and a wheel on the render thread. An ordinary list scrolls on its first wheel
  (`compositor::tests::a_list_scrolls_on_a_wheel_made_before_the_first_present`). The canvas's pixels decide, and the first frame presented through a layer switches to that
  layer's composite. A click and a wheel at the same pixel pick the same target
  (`clip::tests::input_before_the_first_present_lands_by_the_canvas_and_after_it_by_the_layer`). `interact::hit`
  paints the frame it is given from nothing and goes through the same function.
- Hover is a derived value: a function of the pointer's latest position and the frame shown now, worked out again
  when either input changes, like a memo keyed by its inputs. `Pipeline::checked` is exactly the key of the last
  real evaluation, `(position, frame)`, set only by a move or a leave routed and by `Pipeline::converge` itself:
  - a modifier or a key routed in the frame on screen works out no hover and leaves the key alone
    (`app::tests::a_modifier_stamped_with_the_frame_on_screen_leaves_hover_to_converge`);
  - a move routed late in an older frame keys hover to that frame, so idle works it out again in the newest
    (`app::tests::a_move_from_an_older_frame_leaves_hover_on_the_frame_on_screen`);
  - a move from a frame gone is routed nowhere, but its position is kept, and hover is worked out there
    (`app::tests::a_move_from_a_frame_gone_is_hovered_where_it_went`).

  One frame per question (review r9, Codex 3): an input is dispatched in the frame it came under
  (`InputState::route_in`, for that dispatch alone), while hover and the cursor read the frame they were last
  worked out in. A modifier, a key or a press routed in an older frame never rolls the cursor back to it
  (`shown_frames::a_late_modifier_leaves_the_cursor_of_the_frame_on_screen`). Flutter's `MouseTracker` keeps the
  same line: a signal event, or one at the same position, does not touch hover state
  (`mouse_tracker.dart:212-228`).

  When to work it out again is one pure function, `compositor::next_change(pointer, frame on screen, frames
  waiting)`: the vblank of the first frame waiting whose hit under the pointer differs from the frame on screen.
  It is evaluated when it is read (`OnScreen::sample`, `Compose::shown_and_next`), from the pointer and the frames
  as they are then. Nothing is predicted when a frame enters and kept (review r9, Codex 1 and Opus I1: a frame that
  entered while the pointer was elsewhere, before the pointer moved onto its box, was never looked at again;
  `compositor::tests::a_frame_entered_before_the_pointer_moved_onto_its_box_is_rechecked_at_its_vblank`,
  `shown_frames::a_frame_entered_before_the_pointer_moved_onto_its_box_is_hovered_at_its_vblank`). It is the
  model of Flutter's `updateAllDevices` after each frame (`rendering/binding.dart:565-583`, `mouse_tracker.dart:358-384`)
  and of Blink's fake mouse move after layout: a re-hit at the last pointer position. It differs in one way: it runs
  only when something can have changed, not at every frame, because compositor frames are not UI frames (see
  `/tmp/m10/research-clip.md`). The pointer is noted on the timeline before the input that moved it goes on, so
  the UI handling a move samples with it. The UI's idle timer wakes it at that vblank: a timer, not a message, so
  it holds however late the UI hears (`window::tests::the_ui_wakes_at_the_vblank_a_frame_entered_shows_from`). The
  window loop reads the cursor after every idle, as Blink's fake move updates the cursor with hover, so a hover
  worked out at a vblank, with no input and no frame drawn, shows its cursor (the same test).

  What is compared under the pointer is everything hover and the cursor are decided by there (`compositor::Under`):
  the box, the span of its text the frame showed under the point, and the modal flag. A scroll that brings another
  span of one paragraph under a resting pointer changes its cursor and hover
  (`compositor::tests::another_span_of_one_paragraph_under_a_resting_pointer_wakes_the_ui`).

  Energy: the UI is woken only when the key can change. As the render thread enters a frame, it compares the hit
  under the pointer where it is now with the frame before (`O(log N + k)`), and tells the UI only on a change; with
  no pointer in the window, never. This is a wake, not a prediction: what the UI then waits for comes from
  `next_change` when it samples. Both frames are hit outside the timeline's lock, so the platform stamping input
  never waits for an index being made: entering a frame with a new table of 20,000 boxes takes 19–53 ms in a debug
  build (two indexes made). A stamp made meanwhile waited at most 0.2–1.7 ms, which is scheduling noise. With the
  hit under the lock, a stamp waited the whole entry, 35–53 ms
  (`compositor::tests::a_frame_entering_hits_its_boxes_outside_the_timelines_lock`). A UI stalled 100 frames under a
  resting pointer, with frames that change what is under it (10 of them), has one message waiting, holding no frame
  (`compositor::tests::a_stalled_ui_keeps_a_bounded_number_of_frames_alive`). Measured over a compositor scroll of
  120 frames at 120 Hz
  (`compositor::tests::a_compositor_scroll_wakes_the_ui_only_when_what_is_under_the_pointer_changes`):
  - pointer outside the window: 0 wakes (0/s);
  - pointer at rest over a box the scroll leaves: 0 wakes;
  - pointer over the rows, 10 px each, scrolled 1 px a frame: 12 wakes, one per row under it, about 12/s at that
    speed, against 120/s for one a frame.

  Moves and leaves wake the UI as input does. Comparing the box, its span and the modal flag stands for the whole
  hover chain: a frame that changes a hovered node's ancestors and not the node under the pointer is not seen until the
  next input. It is not checked when
  layout commits, because the frame before still shows then
  (`app::tests::a_box_moved_under_a_resting_pointer_is_hovered_once_its_frame_presents`). Blink marks hover dirty
  after layout (`local_frame_view.cc:2146-2150`, "Fire a fake a mouse move event to update hover state") and
  recomputes it at the next `BeginMainFrame` (`web_frame_widget_impl.cc:3310-3315`). It hit-tests the main
  thread's new layout there, before that layout's frame is drawn. A frame the compositor scrolls alone does not
  move hover until a main frame comes. Here the check is keyed to the frame that shows, scrolled frames included.
- Memory: each `Presented` holds its own property trees. At 20,000 clips that is 20,003 clip entries of 44 bytes
  plus a few transforms and effects, about 0.88 MB a copy (measured in `compose_alloc`). The strong holders are
  the timeline, the pool (`POOL` = 3), the frame composed last, and at most two on the UI (`InputState`'s frame and
  one being routed). Queues hold ids only. The frames alive are at most `min(ceil(RETAIN / interval) + 1, KEPT) +
  POOL + 1 + 2`: 22 at 60 Hz (19 MB at 20,000 clips), 38 at 120 Hz (33 MB), 68 at 240 Hz (60 MB), however long the
  UI stalls (`compositor::tests::a_stalled_ui_keeps_a_bounded_number_of_frames_alive`). That test counts each
  snapshot alive by a token it holds, not by weak references, which would keep frames from being used again. The
  records of what each compose wrote are capped at `KEPT + POOL + 2`. A frame older than them, or one behind by
  more writes than the trees have entries, is copied whole when it comes back
  (`compose::tests::catching_up_through_more_writes_than_entries_copies_the_trees_whole`). Each thread's hit indexes
  add at most 4, about 0.45 MB each at 20,000 boxes; one whose table is gone is dropped at that thread's next hit.

  Measured at display rate (`compose_alloc::the_first_quarter_second_at_240_hz_makes_at_most_the_bound_of_copies`,
  a list scrolled over trees of 20,009 entries at 240 Hz, debug build):
  - While the timeline fills (66 frames), each frame is a new snapshot: 655 allocations, 55.6 MB and 1.26 M entries
    copied in all.
  - After that, frames come back from the timeline: 0 allocations, 162 entries copied over 54 frames.

  This
  bound is not sharing: the cost grows with the refresh rate and the trees. Open: share unchanged chunks of the
  trees between frames (a persistent vector), so that a frame kept costs what it changed. Revisit before trees
  pass about 100,000 entries at 240 Hz. Gradient stops are shared (`Arc<[_]>`), so copying a masked effect
  allocates nothing.
- The snapshot names a node by path and by the component instance that made it. Dispatch delivers to the live node
  only when both match; a node another instance made at that path since is another node and takes nothing
  (`interact::tests::a_click_on_a_box_another_instance_replaced_since_it_was_shown_reaches_neither`), as Blink
  never reuses a hit target across a change of the document (`HitTestCache`).
- A click while the compositor scrolls therefore hits the row that was there
  (`a_click_while_the_compositor_scrolls_hits_what_was_on_screen`).
- A wheel stamped with an older frame still kept for routing reads that frame's own `Presented`: its tables,
  property trees (effects included) and layers together
  (`presented_routing::a_wheel_stamped_under_an_older_frame_reads_that_frames_layers`).
- The devtools pick and send input through the same snapshot (`devtools::Showing`); there is no route against the
  frame as laid out. A command that scrolls its node into view runs again once a frame showing the node there is
  presented (`Done::Again`, for click, drag, reveal and hover alike;
  `presented_routing::the_devtools_send_input_through_the_frame_shown`). A live window that presents none within 2 s
  answers with an error. The deadline is a timer on the UI clock, so it fires with no other activity
  (`devtools::tests::a_rerun_a_stopped_window_never_presents_for_is_answered_with_an_error`). A command naming a
  node acts only where the frame on screen shows that node, a node inside it, or the box that takes the pointer for
  it. Where the screen shows nothing there, or a box around the node, the command fails and names what the screen
  shows (`devtools::tests::a_named_click_where_the_screen_shows_nothing_or_what_is_around_it_is_refused`).

**Time.**

- `clock::now()` is gone; reading the time asks for no frame.
- `clock::begin_frame(present_at)` fixes the frame at the `MainFrame`'s predicted present time, and motion samples
  at it.
- `clock::Vsync`:
  - blocked acquires (longer than 0.5 ms) are vblanks;
  - the phase is fitted to the last 16, dropping those more than a quarter interval from their median;
  - the interval comes from `refresh_rate_millihertz`, read again after `Moved`, `Resized`, `ScaleFactorChanged`
    and `Occluded(false)`, and sent to the render thread only when it changed (sending it resets the fit). On
    Wayland it also comes from frame-callback gaps.
- A window resized has its surface configured to the new size at once (`ToRender::Resize`): a swapchain not the
  window's size is out of date on Vulkan. Until the UI commits a frame laid out at the new size and scale, the last
  frame is shown at its own size in the corner, and the rest is cleared to the root's background (the root's own
  colour when it paints the whole window opaque, `paint::window_background`, else white). An overlay is drawn at the
  frame's size there too, in the same corner. It is never
  stretched, and no scene is drawn at one scale into another.
- A focused caret blinks for 10 s from its last change (`paint::CARET_BLINK_TIMEOUT`, GTK's
  `gtk-cursor-blink-timeout`), then stays shown, and nothing wakes for it. It shows and blinks only in the active
  window (Blink's `FocusedOrActiveStateChanged`). The window gaining focus again, even between two frames, restarts
  it. `input-otp`'s caret (a compositor animation) ends after the same 10 s and shows only in the active window.
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
