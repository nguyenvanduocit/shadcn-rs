# 0029. One crate holds the OS display boundary

Status: proposed, 2026-11-24 (D1). Built with it: the crate `crates/gui-display` with a macOS backend and an X11
backend, wired into `gui`'s clock and platform thread; see "As built" for what is tested and "Open" for what is not
built. Amends ADR-0015's time source (the "upstream contributions" it waited for are no longer the plan) and the
workspace rule that no crate holds `unsafe` (AGENTS.md, "Safety holds"), by the user's decision of 2026-11-23.

## Context

`gui` times its frames by presentation (ADR-0015) and has no way to ask the OS about its displays. What it does today:

- **Vsync is inferred.** The moment a blocked `get_current_texture` returns is taken for a vblank, and a phase is
  fitted to the last 16 of them (`clock::Vsync`). On Wayland, frame callbacks are samples too. A timer wake that
  slipped into the fit as a vblank was a confirmed bug (plan M15).
- **The refresh rate is winit's** `refresh_rate_millihertz`, read again when a window event hints the window may be
  on another monitor (`Moved`, `Resized`, `ScaleFactorChanged`, `Occluded(false)`). A mode change with no window
  event is not seen. winit has no event for a monitor plugged in, pulled out or changed.
- **Colour space and HDR headroom** are chosen once (`SurfaceColorSpace::Auto`), not per display (plan M15).
- **Variable refresh** ranges are not known at all.

ADR-0015 planned to get these through upstream work (winit #2412, wgpu #2869 and #9856), "because calling platform
display links ourselves needs `unsafe`". None of it has landed, and the plan's later work needs the data now: frame
scheduling from content (D2), a self-checking phase estimator (D3), a rhythm per display for several windows (D4).

On 2026-11-23 the user decided that one small crate may hold the OS `unsafe` this needs, and nothing else may.

### What broke for others

Read for their failure modes, not their fixes (AGENTS.md). Checked against the source where a path is given; the
rest is from their public trackers and is marked so.

1. **A display link cannot be torn down safely** (GPUI, `references/zed/crates/gpui_macos/src/display_link.rs`).
   `CVDisplayLinkStop` returns before the link's thread has left, and nothing tells when it has. Releasing the link
   after a stop crashed in `CVHWTime::reset`; a callback still on its way read a freed context pointer. GPUI now
   keeps one immortal link per display and passes the display id, not a pointer, as the context.
2. **A lock cycle through Core Video's own locks** (same file). The callback takes the toolkit's lock, maybe under
   Core Video's; starting or stopping a link under the toolkit's lock can close the cycle.
3. **A display link that never stops** keeps a thread waking at every vblank for a window at rest. Chromium stops
   its link after a run of callbacks nobody used (from its tracker, not checked here).
4. **Notifications arrive mid-reconfiguration.** Core Graphics reports a display change in several steps, and
   display ids can change under a display that stays (a laptop switching GPUs). Toolkits that update their model
   event by event see states that never existed as a whole; Chromium debounces (from its tracker, not checked here).
5. **A fallback time passed off as the display's.** GPUI's callback returns its own receipt time when the stamp is
   invalid (same file, `frame_request_instant`). The consumer cannot tell a vblank from a wake. Ours had the same
   class of bug with timer wakes.
6. **`CVDisplayLink` is a model, not an interrupt.** It is driven by a timer fitted to the display, and on a
   variable-refresh panel it reports the nominal cadence. Its output time is a forecast.
7. **Timing delivered on the main thread** (`CADisplayLink`, a winit frame event) is late whenever that thread is
   busy, which is the stall ADR-0015 moved rendering off the main thread to avoid.
8. **Core Graphics reports a refresh rate of 0** for some built-in panels.
9. **winit has no display events**, and identifies a monitor only by a native id that is not stable.

## Options weighed

**Where the `unsafe` lives.**

- *Wait for winit and wgpu* (ADR-0015's plan). Rejected: neither has shipped a frame event or presentation feedback,
  neither plans colour, headroom or variable-refresh range per display, and a winit frame event would be delivered
  on the main thread (failure 7).
- *Lift `forbid(unsafe_code)` in `gui`* for one module. Rejected: a `forbid` cannot be lifted at a site, so the crate
  would drop to `deny` everywhere and every file of `gui` could hold an `#[expect(unsafe_code)]`. The boundary would
  be a convention.
- *A separate crate* (chosen). The compiler keeps `gui`, `primitives` and `shadcn` at `forbid`; the boundary is a
  dependency edge.

**Push or pull.**

- *Events pushed as the OS sends them.* Rejected: failure 4. Each event describes a moment the consumer did not
  see whole, and a callback on an OS thread runs the consumer's code there.
- *Poll and difference* (chosen). The OS's notification sets a flag and wakes the owner. The owner's `poll` reads
  every output afresh and returns the difference from what it knew. An event is then always a difference between
  two whole states, and a later notification gives a later difference; nothing is debounced by time.

**The vsync source on macOS.**

- *`CADisplayLink` from `NSScreen`/`NSView`* (macOS 14, not deprecated). It follows a ProMotion panel's real
  cadence and takes a preferred rate range. Rejected for now on merit: it is delivered on a run loop, so on the main
  thread (failure 7), and it needs an Objective-C target class defined in Rust, a larger `unsafe` surface than a C
  callback.
- *`CAMetalDisplayLink`* (macOS 14; GPUI's choice). It hands out the drawable with its target present time, which is
  the best data there is. Rejected for now: the drawable must come from its callback, and wgpu owns the layer's
  drawables. It becomes possible when `gui` owns the present path.
- *`CVDisplayLink`* (chosen). A C API with its own thread, independent of the main thread. Deprecated since
  macOS 15, and a model of the display (failure 6): its times are labelled predicted, never presented.

**Labelling time.**

- *An `Instant` and a doc comment.* Rejected: failure 5 is this.
- *One enum, `Stamp`* (chosen): `Predicted`, `Presented`, `Synthesized`. A consumer decides per variant;
  `Stamp::of_the_display` returns none for a synthesized time, so fitting a phase to a timer does not type-check
  by accident.

## Decision

### The crate

`crates/gui-display` is the OS display boundary, and the only crate of the workspace that may hold `unsafe`.

- It cannot take the workspace's lints (they `forbid`), so it repeats every one of them and changes one:
  `unsafe_code = "deny"`. It adds `unsafe_op_in_unsafe_fn`, `clippy::undocumented_unsafe_blocks` and
  `clippy::multiple_unsafe_ops_per_block` at deny.
- **A site is one function** that holds one `unsafe` block with one unsafe operation, a `// SAFETY:` comment, and
  `#[expect(unsafe_code, reason = "…")]` on the function. Nothing else in the crate can compile `unsafe`.
- `tests/sites.rs` pins the count of blocks and of `unsafe` declarations per file, and the count of
  `unsafe_code` lifts, which must equal them: each an outer `#[expect]` on one function or extern block, never on a
  module, an impl or the crate (`#![…]` is refused in every source file, and the manifest must keep
  `unsafe_code = "deny"`). It fails for a source file it does not list, and checks each block is lifted on its own.
  Adding a site means editing that test.
- **Every change to the crate is reviewed by Codex** (the second model family) before it lands, as well as by the
  usual review. A change that edits `tests/sites.rs` says so in its report.
- `gui`, `gui-tls`, `primitives` and `shadcn` keep `forbid(unsafe_code)`.
- The crate knows nothing of `gui`: no wgpu, no winit. It takes a `raw-window-handle` display handle to learn which
  windowing system it is on.

### The API

```rust
let mut displays = Displays::connect(&window, wake)?;   // window: &Arc<W>; on the thread that will own it
for event in displays.poll() { … }                      // Added, Removed, Changed { output, was }
let output: &Output = displays.by_native(native_id)?;   // or .output(id), .outputs()
let ticks: Ticks = displays.ticks(output.id)?;          // Send + Sync: goes to the render thread
ticks.latest(); ticks.wait(after, timeout); ticks.run(wanted); ticks.generation();
```

- **`connect` keeps the window** (an `Arc` of the toolkit's window, which gives both the display and the window
  handle). On X11 the backend uses the toolkit's own connection, which stays open while the window lives
  (raw-window-handle's contract), so `Displays` holds the window and drops it after the backend.

- **`Output`** is plain data: an `OutputId` (never used again in the session), an `OutputKey` (the same for one
  monitor across sessions: macOS's display UUID, X11's EDID identity plus connector), the OS handle windowing
  toolkits report, name, `Geometry`, `Refresh { interval, range }`, `Colour { gamut, headroom }`.
- **`Refresh::range`** is the variable-refresh range: the shortest and longest time a frame stays on screen, and the
  granularity where the OS tells it.
- **`Headroom`** is `current`, `potential` and `reference`, as multiples of SDR white (macOS's three EDR values).
- **`Gamut`** names the standard gamut the primaries match (sRGB, Display P3, Adobe RGB, Rec. 2020) or `Unknown`.
  An ICC profile's colorants are D50-adapted and an EDID's are the panel's own, so each is compared with the
  standards in its own terms (`gamut.rs`); the API exposes no raw primaries whose meaning would differ by OS.
- **Every event names its output.** `Changed` carries the output as it was, so the consumer compares the parts it
  cares about. Removals come before additions.
- **Identity.** An output is the one known before when its key is the same; among several with one key, the one
  with the same OS handle first. A handle that changes under a display that stays is a `Changed` (failure 4), and
  ends the vsync subscriptions made for the old handle.
- **`Stamp`**: `Predicted` (the OS's forecast of a vblank), `Presented` (presentation feedback, measured),
  `Synthesized` (a timer at the nominal rate: a rate with no phase). A stamp the OS marks invalid produces no tick
  at all (failure 5).
- **`Tick`**: output, sequence number, when the source's callback ran, the target `Stamp`, the interval the
  source reported, and the **generation** of the output's mode it was taken in. A poll that reads another interval
  for an output moves its subscriptions to the next generation (`Ticks::generation`), and a timer source ticks at
  the new interval from then on. A consumer that resets its timing for the new mode takes only ticks of it: one
  stored just before the change is of the old grid. **The generation is stamped when the time is read**, not when
  it is stored: a source reads the interval and generation together (`Feed::mode`) before it reads or makes its
  time (the timer before it sleeps, the display link's callback before it reads its `CVTimeStamp`), and a time read
  in a generation older than the feed's when it is stored is dropped. So the first tick after a change is of the
  new mode, never an old-mode time labelled new.
- **A read that fails changes nothing.** A backend's probe is whole or an error; on an error the poll keeps the
  outputs it last read whole, publishes nothing, and reads again at the next poll. A failed request never reads as
  an output pulled out.
- **A source that fails to start is not kept.** `Displays::ticks` returns the error with nothing registered or
  counted, and a later call tries afresh. A source that will not start again after an idle stop ends its
  subscription (`Ticks::ended`); the consumer falls back to what it did with no source.

### Threads

- A `Displays` belongs to the thread that made it (on macOS the main thread; off it, `connect` returns
  `NotMainThread`) and is neither `Send` nor `Sync`.
- **No code of the caller's runs under a lock of the crate's.** A vsync callback converts the OS's time, stores one
  `Tick` under a short lock, and wakes whoever waits. The one piece of the caller's code that can run on the OS's
  thread is `wake`, when an idle source asks to be stopped: the callback collects the wakes it owes while it holds
  the feed table's lock and sends them after releasing it (`ticks::Asked`, a must-use value and the only way a wake
  leaves a tick). So a wake that blocks on a lock the owner holds while it stops another reader cannot close a cycle
  with ours. `wake` must not block all the same (it runs before the vsync thread's next vblank), and should only
  have the owner's thread call `poll`.
- **Sources are started and stopped by the owner alone,** and never under a lock the callback takes (failure 2). A
  reader says whether it wants ticks (`Ticks::run`). Unwanted for `IDLE` (500 ms), the source asks the owner to stop
  it, once; wanted again, the owner is woken to start it (failure 3). A reader that sleeps between bursts of input
  pays no stop and start for each. A start can fail (`Source::start` returns the OS's error); a stop the OS refuses
  leaves what the OS says runs, and the backend tries again at every poll (`Backend::tidy`).

### macOS

- **The oldest macOS read is macOS 11** (Big Sur, Rust's own floor for Apple silicon); `connect` returns
  `Unsupported` before it (`available!(macos = 11.0)`). Every selector and function called exists in 11, but the
  refresh range (`minimumRefreshInterval`, `maximumRefreshInterval`, `displayUpdateGranularity`) and
  `maximumFramesPerSecond`, which are macOS 12 and read behind `available!(macos = 12.0)`; before 12 a display has no
  range. `tests/sites.rs` holds the table of every Apple call of the backend with the macOS that brought it, fails
  for a selector or `CG`/`CV`/`CF`/`NS` function it does not list, for a call newer than the minimum not behind its
  check in its function, and for `NSScreen.CGDirectDisplayID` (macOS 26) at all. The display id is read from the
  screen's `deviceDescription["NSScreenNumber"]`, as on every version (and as winit does).
- Outputs are `NSScreen.screens`. **What changes an output is observed through two notifications**, audited
  against what an output holds:
  - `NSApplicationDidChangeScreenParametersNotification`: screens added, removed, moved, resized or remoded, and,
    per `NSScreen.h`, a change of `maximumExtendedDynamicRangeColorComponentValue` (the current EDR headroom).
  - `NSScreenColorSpaceDidChangeNotification` (macOS 10.6): a screen's colour profile, so its gamut.
  - Not separately notified, and read again only when one of the two fires: the potential and reference EDR maxima
    (fixed per panel and setting, as far as the headers say) and the refresh range (fixed per mode). That they never
    change alone is the headers' reading, not verified.
  `macos::tests::every_notification_that_changes_an_output_is_observed` pins the list. A screen's gamut is parsed
  from its ICC profile once per (display, profile hash) and reused while the profile is the same.
- The interval is the Core Graphics mode's rate; when that is 0 (failure 8), the screen's
  `maximumFramesPerSecond`, then its `minimumRefreshInterval`.
- **One `CVDisplayLink` per display for the whole process**, made at the first subscription to that display and
  **never released** (failure 1), shared by every reader of it from any `Displays`, running while one wants ticks.
  The table of links is the main thread's (a `thread_local` used only with a `MainThreadMarker`: there is one main
  thread per process, and a `Displays` is only made and used there), so reconnecting, or a second `Displays`, reuses
  the link. `gui_display::native_links_made()` counts the links made; the live test asserts it is one after two
  readers at once and three reconnections. A link's context is a number looked up in a table, not a pointer, so a
  callback after a stop finds nothing and no lifetime depends on when Core Video stops calling. A link is made
  immortal (counted, and kept out of `Drop`) only once its callback is set: when
  `CVDisplayLinkSetOutputCallback` fails, the link is released and the subscription returns the error
  (`a_link_whose_callback_could_not_be_set_is_released`, through a fault hook on the setter).
- The link's output time becomes an `Instant` by reading both clocks at once (they are one clock in two units).

**Its `unsafe` sites: ten blocks and one declaration, in `src/macos.rs`** (1 to 9 and 12; X11 has two more, below).

| # | Function | Operation | Why it is sound |
|---|---|---|---|
| 1 | `screens_changed` | read AppKit's extern static notification name | set when the framework loads, never written |
| 2 | `observe` | `addObserverForName:object:queue:usingBlock:` | the block captures only `Send + Sync` values; no object, no queue |
| 3 | `unobserve` | `removeObserver:` | the token is the one this centre returned |
| 4 | `uuid_raw` | `CGDisplayCreateUUIDFromDisplayID` | takes any id by value, returns null for an unknown one |
| 5 | `adopt` | `CFRetained::from_raw` on the UUID | a Create function's +1 object, released once |
| 6 | `create` | `CVDisplayLinkCreateWithCGDisplay` | the out pointer is a live local for the call |
| 7 | `adopt_link` | `CFRetained::from_raw` on the link | written by a successful create, +1 |
| 8 | `set_callback` | `CVDisplayLinkSetOutputCallback` | the callback has the C signature and catches unwinds; the context is a number Core Video never reads |
| 9 | `stamp` | read the callback's `CVTimeStamp` | valid and unwritten for the callback's length, plain data |
| 12 | `colour_space_changed` | read AppKit's extern static `NSScreenColorSpaceDidChangeNotification` | as site 1 |
| – | declaration | `unsafe extern "C"` for the one ColorSync function no binding crate in the tree has | its C signature from `ColorSyncDevice.h` |

Everything else goes through objc2's generated bindings, which check each message's signature.

### X11

- **The toolkit's server and screen.** Requests go through the toolkit's own connection: an Xlib handle's
  `Display` gives its XCB connection (`XGetXCBConnection`, loaded by `x11-dl`), an XCB handle gives it directly,
  and it is adopted into `x11rb` without ever being closed (two sites below). A handle with no connection in it
  (raw-window-handle's "default display") gets one of the crate's own, and keeps the screen the handle names: the
  connection and the screen are chosen apart, so the default connection's preferred screen never replaces it. The
  screen is the root of the window when a window handle is given and its root is one of the server's, else the
  screen the display handle names.
- **RandR 1.3 is required** (`GetScreenResourcesCurrent`, which reads the server's state without probing the
  hardware); below it `connect` returns `Unsupported`.
- An output is a connected RandR output with a CRTC, each one of them, mirrored outputs sharing a CRTC included; the
  handle is the CRTC, as winit reports it.
- **A request that fails, or a configuration that changes under the read (`InvalidConfigTime`), fails the probe**,
  so the displays keep their last whole read and read again at the next poll.
- The interval is the mode's pixel clock over its whole frame. Name, identity, primaries and the vertical range come
  from the EDID property (`edid.rs`). The variable-refresh range is the EDID's range limits when the driver sets
  `vrr_capable`. X11 has no HDR, so the headroom is none.
- **Notifications** cannot be read on the toolkit's connection (its event queue is the toolkit's). They are read on
  a connection of the crate's own to `$DISPLAY` (safe `x11rb`), kept only when its server is the toolkit's (same
  vendor, release and screens); a thread waits on it, sets the flag and wakes the owner, and is told to leave
  through a client message when the displays drop. It reads nothing else. Where `$DISPLAY` is another server there
  is no such thread, and the backend says it does not notify (`Backend::notifies`): **every poll then reads the
  outputs**, so a change is seen at the next poll, which a window event (`gui` follows its display on each) or any
  other poll brings.
- **No vsync source.** `Displays::ticks` returns `Unsupported`; `gui` keeps fitting acquires there.

**Its `unsafe` sites: two blocks, in `src/x11.rs`.**

| # | Function | Operation | Why it is sound |
|---|---|---|---|
| 10 | `xcb_of` | call `XGetXCBConnection` through x11-dl's loaded pointer | the `Display *` is the toolkit's, open while the window lives, which `Displays` keeps; the function reads it and returns the connection Xlib made at open |
| 11 | `adopt` | `XCBConnection::from_raw_xcb_connection(ptr, false)` | a live `xcb_connection_t *`, open while the window lives, which `Displays` drops after this backend; never disconnected here; XCB is thread-safe and no event is read from it |

### In `gui`

- The platform thread owns the `Displays` (`window.rs`). Its wake is a `UserEvent::Display` (an event-loop proxy
  send, which does not block); `user_event` hands it to `window::on_display`, which follows the window's display
  again. So a mode change or a pulled monitor with no window event is seen.
- `window::follow` polls the displays first, then decides what to tell the render thread, and only what it was not
  told: `ToRender::Output { interval, ticks }` on another output or handle, `ToRender::Interval` on the same one.
  The interval is the OS mode's; winit's rate is used only where the displays are not read. Polling first means a
  monitor winit reports before the displays' own wake is handled is told once, as its output, not as winit's rate
  and then as the output (two resets).
- The render thread keeps everything it times against one output in one mode in one value (`thread::Timing`): the
  vsync and its phase, the output's ticks and the mode generation they must carry, the vblank reserved by the last
  frame that presented, the one a frame drew blank for, and the one a main frame was granted for while it slept.
  `ToRender::Output` makes it afresh; `ToRender::Interval` with a new rate makes it afresh keeping the ticks. No
  deadline on the old grid is aimed at after a move or a mode change, and a tick of the old mode is not taken.
- **What a frame aimed is of a timing epoch.** `Timing` counts the times it was made afresh; a frame captures the
  epoch when it computes its present and deadline. Messages handled while it waits (the drain before its grant,
  the wait for its commit) can make the timing afresh under it; then the frame aims again on the new timing before
  it grants or draws, and `Timing` itself drops a grant, a reserved or a blank vblank of another epoch than its
  own. One rule, in `Timing`, not at each call site.
- The render thread wants ticks while it draws and not while it sleeps (`thread.rs`). Each frame takes the latest
  tick of its mode into `clock::Vsync::ticked`: a predicted or presented time is the phase; a synthesized one sets
  nothing. While ticks come, blocked acquires are not fitted; a second after the last tick they are again.
- `Vsync::next_after` was wrong for a phase ahead of the time asked about, which a forecast always is. Fixed.

## Consequences

- The workspace has a fifth crate, and one place where `unsafe` compiles. `Cargo.lock` gains `gui-display` and
  `objc2-core-video`; the other objc2 crates and `x11rb` were in the tree.
- On macOS the phase is the display's own, not inferred. Measured on a 60 Hz external display (stress `stream` in a
  window, 393 blocked acquires): 390 returned 1.1 to 1.7 ms after the link's vblank (mean 1.23 ms), so a frame is
  still aimed at the same vblank as before, at a time 1.2 ms earlier, with one tick per frame. That is one display
  on one machine, not a claim about others.
- A window at rest wakes no thread at a vblank: its link stops 500 ms after the render thread sleeps.
- A display link is leaked per display handle, once per process, by design.
- On X11 `gui-display` depends on `x11-dl` (already winit's) and on `x11rb`'s `allow-unsafe-code` and `dl-libxcb`
  features (already enabled by winit); the crate's sites are twelve blocks and one declaration.
- **`CVDisplayLink` is deprecated.** The crate builds with `#[expect(deprecated)]` at each use. If Apple removes it,
  the backend changes and the API does not.
- The colour and headroom of each output are now known and change with events, but `gui` does not use them yet:
  the renderer draws sRGB into an `Auto` surface. Reconfiguring the surface per display belongs to the colour
  pipeline (plan M15, "không gian màu và headroom"), which can now be written against `Event::Changed`.

## As built

- `cargo test -p gui-display`: 40 unit tests on macOS (scripted displays for topology, identity, events, tick
  labelling and mode generations, a time read before a mode change dropped, idle stop, a start that fails, a probe
  that fails, synthesized ticks whose first tick after each change is of the new mode, a backend that does not
  notify read at every poll, a scripted change whose closure runs under no lock; ICC and EDID parsing; macOS's time
  conversion, threading, a wake that blocks on the owner's lock while the owner stops another reader, which
  deadlocked before, a link whose callback could not be set released, the gamut cache, the notification list);
  `x11.rs`'s 6 unit tests (screen selection, a handle with no connection keeping its screen, RandR version,
  same-server check, mirrored outputs) compile here and run only on Linux; `tests/sites.rs` (site,
  lift and Apple-availability inventory); and `tests/live.rs`, which runs on the main thread against the real OS on
  macOS: outputs are sane, a display link ticks predicted times one refresh apart, stops when unwanted, starts
  again, two readers share a link, and three reconnections leave one link made for the display. Elsewhere it prints
  that it skipped.
- `gui`: `clock::tests` (a tick is the phase, a synthesized one never is, acquires stay out while ticks come, the
  interval within what the monitor reports), `window::tests` (`an_os_display_event_reaches_the_render_threads_vsync`,
  `the_displays_wake_alone_tells_the_render_thread_of_a_change`,
  `a_monitor_winit_reports_before_the_displays_poll_resets_the_vsync_once`), and `compositor::thread::tests`
  (`an_outputs_ticks_are_the_phase_of_its_frames_and_unwanted_while_the_thread_sleeps`,
  `moved_from_60_to_120_hz_the_next_frame_aims_at_a_120_hz_vblank`,
  `moved_to_120_hz_while_a_frame_waits_nothing_aimed_at_60_hz_is_granted_or_kept`,
  `moved_to_120_hz_while_a_frame_waits_for_its_commit_it_is_drawn_for_a_120_hz_vblank`,
  `after_a_mode_change_a_tick_of_the_old_mode_is_not_the_phase`).
- Clippy with `-D warnings` passes for the crate on macOS, `x86_64-unknown-linux-gnu` and
  `x86_64-pc-windows-msvc`.

## Open

Said plainly; none of these is built.

- **No `Presented` time exists yet.** The variant is there and `gui` handles it; nothing produces it. On macOS it
  needs the drawable's presented handler, so `gui` owning the present path or wgpu exposing it. On Wayland it is
  `wp_presentation`.
- **Wayland has no backend.** `connect` returns `Unsupported`; `gui` keeps winit's rate and its frame-callback
  fitting. Reading outputs needs winit's `wl_display` (a foreign-display site in this crate), and presentation
  feedback needs its surface.
- **Windows has no backend** (DXGI output descriptions, `DwmGetCompositionTimingInfo`).
- **The X11 backend has never run.** It compiles and passes clippy for Linux from macOS; its unit tests (parsing,
  screen selection, a default-connection handle keeping its screen, RandR version, same-server check, mirrored
  outputs) need no server but run only on Linux, so not on the machine this was written on. Adopting the toolkit's
  connection, the window's root, the probe against a real RandR, a failing request, the notification thread, and
  reading at every poll with no notification connection were never exercised on X11 (the last is tested only
  through a scripted backend that does not notify). Whether `$DISPLAY`'s server is
  the toolkit's is judged by the setup (vendor, release, screens), a heuristic: two servers alike in all three
  would be taken for one, and the notifications then come from the wrong one (the outputs are still read from the
  right one). X11 gives no vsync ticks (the Present extension reports a window's presents and needs the toolkit's
  window).
- **macOS before 11 and before 12 were not run.** The refusal below 11 and the 12-only reads behind `available!` are
  pinned by `tests/sites.rs` against a table of versions written from Apple's documentation, not run on those
  systems.
- **A stop Core Video refuses** is handled (the link's running state is what `CVDisplayLinkIsRunning` says, and the
  stop is retried at each poll) but was never provoked; the scripted backend fails starts and probes, not stops.
- **ProMotion is untested.** The only display at hand was a fixed 60 Hz one. The link reports a nominal cadence on
  a variable panel (failure 6); the range is read from `NSScreen` but nothing was measured against it.
- **Hot-plug was tested only through scripted displays.** No monitor was plugged or pulled during this work, and
  the AppKit notification path was not exercised by a real change.
- **`gui` does not consume colour, headroom or the variable-refresh range.**
- **One window.** Several windows on displays of different rates is D4.
- A display handle the OS reuses for another display reuses its leaked link, on GPUI's observation that a link reads
  its display's timing as it runs. Not verified here.

## Revisit when

- `gui` owns the present path, or wgpu ships presentation feedback: produce `Presented`, and weigh
  `CAMetalDisplayLink` again.
- Apple removes `CVDisplayLink`, or D2's pacing needs to ask a ProMotion panel for a rate: move to `CADisplayLink`
  with its run loop on a thread of the crate's own.
- D3's estimator measures the link's phase error against presented times above a quarter interval on some display.
- Another site is needed (Wayland's foreign display): the rule and the review stay; the table grows.
