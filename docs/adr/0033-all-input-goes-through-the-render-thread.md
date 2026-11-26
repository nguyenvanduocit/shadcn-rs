# 0033. All input goes through the render thread

Status: accepted under delegation 2026-11-25 (wave C (a), round 9); built; amended in round 10 (the switch follows
"the compositor can scroll", ordered clamped commit, the log records where each input is taken, real-surface
numbers). Supersedes I1, the input half of I2,
the residual race and the replay text of [ADR-0032](0032-scroll-is-ordered-by-what-the-ui-has-seen.md). I3–I7 of
0032 stand, with I4's mount identity made exact below. Amends the input path of
[ADR-0015](0015-gui-runs-on-three-threads-timed-by-presentation.md).

## Context

ADR-0032 ordered scroll by what the UI had seen. It kept two producers of UI-bound input. The platform sent clicks,
keys and moves straight to the UI, and wheels to the render thread. The render thread scrolled and handed some wheels
on to UI handlers. Three mechanisms existed only to reconcile those two producers, and the last three review rounds
found their failures:

- The platform numbered its wheels and sent the render thread a `Cut(w)` before each UI-bound input. The render
  thread answered with `Through(w)`, and the UI held input in `InOrder` until the answer came (Codex r8 2: a click
  overtook the wheel sent before it).
- The outbox was kept in runs between cuts so that each input could take "through `w`", with a grant cut so that a
  main frame could too. Its cost had to be argued linear (Codex r9 I2).
- A recorded log had to carry what the two threads observed (applied, cut, take, built, commit) for a replay to pair
  with the platform's events. Pairing by count broke under the UI's move coalescing (Codex r9 I3, Opus r9 I1).

The research for round 9 (`/tmp/m10/research-scroll.md`) asked whether real implementations avoid the whole class.
Every one that orders input correctly has one producer of UI-bound input that also owns scroll:

- **Chromium.** Where a widget has a compositor thread, every event reaches it first. `InputHandlerProxy` handles
  it or forwards it, in arrival order, to the one `MainThreadEventQueue` (`widget_input_handler_manager.cc:785-794`,
  `:1132-1142`). A widget without one dispatches straight to the main thread (`DispatchDirectlyToWidget`, `:795`),
  and that widget scrolls on the main thread, so it has one producer too. The main queue coalesces a new event into
  a queued one of the same class. While it looks for one, it walks past queued events of another class
  (`main_thread_event_queue.cc:154`, `:158`, `KeepIterating`), so a move can be coalesced into a move queued before
  a click. Our rule is stricter: a held move is flushed before any other input, so nothing is coalesced past an
  event of another kind (`:150-193`, `:692-696`). The main thread learns the compositor's delta at
  `BeginMainFrame` (`PullDeltaForMainThread`, `cc/trees/property_tree.cc:2108`). The delta is keyed by an element id
  minted per layout box and never reused (`paint_layer_scrollable_area.cc:3365-3368`,
  `compositor_element_id.cc:11-14`). A delta for an id the main thread no longer has is dropped
  (`property_tree_layer_list_delegate.cc:52-82`, `property_tree.cc:2204-2218`).
- **Android and Flutter.** Input is consumed on the UI thread from one queue (`Choreographer.CALLBACK_INPUT`;
  `gestures/binding.dart:297-344`), and the UI thread scrolls. Neither scrolls off the UI thread, which ADR-0015's
  latency goal rules out here.
- **rr and ReVirt.** rr (O'Callahan et al., USENIX ATC 2017) and ReVirt (Dunlap et al., OSDI 2002) log each
  nondeterministic input at its position in the one execution order. The party that defines that order writes the
  log, so a replay reads it in sequence.

## Decision

**One producer.**

- **Routing (I1).** The platform sends every message bound for the UI to the render thread, in the platform's
  order: wheels as before, everything else, including an assistive technology's action (`Input::Act`), as
  `ToRender::Input` (`window::Threads`), and what it tells of its window as `ToRender::Told` (`input_log::Told`: a
  size or scale, focus taken, visible or hidden, the decorations, a screen reader on or off, quit). The render thread
  applies wheels at once and takes its own part of a `Told` (a surface size, being seen) before it hands it on
  (`FromRender::Told`, with its pull). No platform message reaches the UI by another path, so a click after a resize
  is handled at the new size, live and replayed alike (round 11, Codex 1). While diverted, the platform sends the
  same stream straight to the UI, and the render thread's own part alone to it.
- **Hand-off.** For each input bound for the UI, the render thread pulls what the compositor scrolled since the last
  pull (`Outbox::pull`). That covers its own forwarded input and the wheels it hands to UI handlers. The pull goes
  down the one UI queue with the input (`FromRender::Input`, `Wheel`, each with a `Pulled`). A grant pulls the same
  way into `MainFrame::taken`. This is `PullDeltaForMainThread`, done for each hand-off.
- **Taking it in.** The UI takes an input's pull in right before it handles the input (`Pipeline::take_pulled`). A
  drag move the UI holds to coalesce takes its pull in as it arrives, after any earlier held move is sent. A held
  move is flushed before any other input. That is stricter than Chromium's queue, which coalesces past an event of
  another class (Context).
- **Order.** A click therefore always comes after every wheel the render thread applied before it, and never after
  one it applied later. This holds by construction: there is no cut, no `Through` and no hold.

**The outbox** (`compositor/outbox.rs`) is cc's synced value per container. It is owned by the render thread, with
no lock. It holds one sum per container moved since the last pull, and one sum per container for each pull not yet
in a committed base, numbered. A commit names the last pull its bases hold (`Commit::taken`). `Scrolls::committed`
drops those, then re-applies the rest and what is pending on top of the new base. Each pull lands on the running
position in hand-off order and is clamped there, then the next lands on that (round 10: +30 then −20 into a range of
10 ends at 0, not at 10, as the live compositor clamped them one by one). This is cc's
`PushScrollUpdatesFromMainThread` re-applying the unsent delta on the main thread's new base. Its cost
is O(entries held). Storage is O(containers × hand-offs in flight), and the UI's queue holds those hand-offs anyway.
An aborted commit keeps its pulls for the next.

**Mount identity (0032 I4, made exact).** A container's identity is its mount: `ScrollNode::instance`, minted by
the UI's painter from a counter and never reused. The painter takes in the containers of each layout pass, not only
of each paint (`Painter::mount_scrollers`). A container removed and mounted again at the same key and owner while
the UI is behind its input is therefore a new mount. A delta carries the mount it moved, and the UI takes in only a
delta whose mount is still in its tables. A delta for a mount that is gone is dropped, as cc drops a delta for an
element id it no longer has. This closes the gap 0032 noted and did not fix.

**Pacing (remedy (a) of the research).**

- **Surface pacing.** The render thread blocks in the surface acquire until a texture is free. On a surface it now
  waits on its queue and the frame pacing together (`RenderThread::wait_vsync`). It acquires only when the vblank at
  which its last present shows is due (`acquire_due`), and hands on every input that comes meanwhile.
- **Probe without a phase.** Where the display tells nothing of its phase, it acquires `ACQUIRE_PROBE` (1.5 ms)
  before that vblank, so the acquire still samples the phase (`Vsync::told_lately`).
- **Ticker pacing.** The ticker waits on the queue instead of sleeping.
- **The bound.** An input waits for at most what the render thread is doing when it comes. That is one draw, or an
  acquire of at most the probe (plus the error of the vsync fit). It never waits a whole frame interval.

**Accepted motion is one closed record** (round 11; closed by construction in round 13). Everything the
compositor holds of input it accepted travels together, in `Accepted`:

- the outbox (what it moved and the UI has not taken or committed);
- the displacement its gesture's resampling holds back, settled into the outbox first (`settle_gesture`, I6's rule);
- the fling under way, as data (`scroll::Flinging`: its container as last committed, its gesture's identity, its
  start and velocity, and how far it has moved the container): a fling is a function of time alone, so the next
  compositor carries it on by the time of its own frames and it ends where it would have with no fault;
- whether it flings trackpad gestures at all.

The gesture's latch is not in it: it goes to the UI with the diverted input (`HandedGesture`, set as the UI's
`scroll_latch`), so a diverted wheel scrolls the latched container, not the one under the pointer. A scrollbar's
fade and a container's recent speed are shown state, made again by the next motion. `Compositor::into_accepted`
builds the record by destructuring the compositor and its `Scrolls` with every field named and no `..`, each
either carried or marked as shown state: a field added to either is a compile error until it is placed. Round 13
found the class this closes. Settling the gesture as input was diverted ended its fling (Codex r13 1, Opus r13 I1:
a fling settled by a fault one tick after it began ended at 53.01 px; with no fault, at 295.25). So settling now
applies the resampling's rest and brings the fling up to that instant without ending it; a hidden or occluded
window does the same (`settle_undrawn`) and never ends a fling. A compositor made afresh holds the fling while it
has no commit (`Scrolls::uncommitted`: no tick moves or ends it) and goes on from the first commit. Three things
still stop a fling handed on, as they stop one that was never handed: a jump to its container (the commit's new
generation), its container gone, and a wheel the UI took while input was diverted (`Route::took_wheel`, read at
`restore`, as a new gesture stops the fling on the compositor). A trackpad step held for resampling when the
render thread panics was lost before (5 of 35 px); a nested list's outer one took a gesture latched to the inner
(60 vs 0).

**Fallback (`Forwarding`).** Every send of the platform's is made under one lock, and the lock says where input goes.
When the render thread panics, it diverts under that lock, in this order:

1. It settles the gesture, then sends the scroll it had not handed on with the gesture's latch
   (`FromRender::Diverted`), and logs `obs diverted`.
2. It hands on, in their order, the input, wheels, mail and `Told` it had taken and not yet served (put back
   behind a latch it was installing, then held for one), then those waiting on its channel, keeping its own part of each
   `Told` for when it serves again. A wheel becomes `Input::WheelAt` (`Input::diverted_wheel`), which the UI
   scrolls itself. It keeps the wheel's point, time, precise flag and gesture phase, so the UI hit-tests, scales
   and phases it as the compositor would have; the UI's latch, handed over in step 1, holds it to its container.
3. It sets the switch, so input goes straight to the UI from then on, in the same `Input::WheelAt` form.

What else waited (a commit) is kept until it serves again. **A device made again does not divert** (round 11). The
compositor survives a lost device whole; only the GPU is made again, and never on the input path (round 12, Codex
2: making one blocks for hundreds of milliseconds, and the hop's p99 was 243 ms through a rebuild).
`GpuRenderer::remake` hands a thread of its own what it needs (`Remake`: the instance, the surface's format, the
options); that thread makes the adapter, device and pipelines (`Remake::make`), and the render thread, which goes
on serving its channel meanwhile, polls for the result every `REMAKE_POLL` (4 ms) and takes it in (`take_made`:
the surface checked and configured on its own thread), as Chromium's viz makes a lost GPU channel again
asynchronously while the browser keeps taking input. What costs milliseconds at the swap stays off the render
thread too: the adapter's self-test (banded curve text against the full scan, read back, 6 ms) runs in `make`
(`GpuRenderer::test_adapter`) rather than in the new device's first frame, and the lost renderer is dropped on a
thread of its own (freeing a device, 2 ms). In release the hop's p99 through a rebuild was 3.1–4.6 ms with the
self-test in the first frame, and 135 µs with the old renderer freed inline. A device that cannot be made is tried again after a back-off
doubling from `REBUILD_RETRY` (100 ms) to `REBUILD_MAX` (5 s); the loss is told once and the recovery once, never a
line per attempt; and no attempt is made while the window is hidden. **No frame, no tick** (round 12, Opus P2):
while no frame is drawn (no device, hidden, occluded), nothing ticks a gesture's resampling or fling, so before
input, a `Told` or a grant is handed on, the gesture is settled whole (`settle_undrawn`) and `obs settled` logged; a
click after a trackpad step while the device is lost reads the list at 30, not 0. **The switch follows "the
compositor can scroll"**
(round 10). A recovered render thread has a fresh compositor with no scroll tree, so it does not restore the switch
when it recovers. It restores it when it applies the UI's next full commit (where `awaiting_full` clears), and logs
`obs restored`. Until then input keeps going to the UI, which scrolls it, so no wheel lands on an empty tree and is
lost. The grant is sent under the same lock as its `obs grant` line, so the log's order is the UI's.

**Recovery never blocks input** (round 13, Opus r13 I2: the latch in flight came back 1.61 s late, behind a
synchronous device rebuild). The whole thread body (divert, recover, serve) runs under one catch, so a panic while
it diverts or recovers counts like any other. Returning the UI's latch needs no device, and divert answers it as it
drains: a `Latch` that arrives while `latch_out` is set goes back as `Returned` there and then. `recover` makes no
device: it marks the GPU lost and `ensure_device` makes one on its helper thread, as for any lost device, while the
thread serves its channel. The test holds the device lost for 2 s with the latch in flight, and the latch comes back
within 16 ms.

**A terminal failure keeps the UI as the owner** (round 13, Codex r13 2). After `MAX_RECOVERIES` panics in
`RECOVERY_WINDOW`, the thread diverts for good, under its own catch, then sends `FromRender::Failed`. The UI clears
any latch on its way back, and the platform reports the failure and marks the app's exit code. Nothing quits: no
send goes to a dead channel, since the switch sends everything straight to the UI, and the window keeps its last
frame and takes input until it is closed (`ToPlatform::RenderFailed`; `ToPlatform::Quit` is gone).

**One owner of a gesture's latch** (round 12, Codex 1). A gesture is latched on one side at a time, and the latch
moves with the route, as a token does. Diverting, the compositor hands its latch to the UI and ends its own
(`Compositor::hand_gesture`). Restoring, the render thread tells the UI under the switch's lock
(`FromRender::Restored`, after everything sent straight) and holds the input that comes meanwhile (`waiting`); the
UI, past all it took straight, hands its latch back and ends its own (`ToRender::Latch`,
`InputState::hand_gesture`), and the compositor latches it (`take_gesture`) before the held input goes on in its
order. One exception keeps one owner: a gesture over a wheel handler on its container, or between it and the
container around it, is routed to the UI by the compositor anyway, so the UI keeps it and the compositor latches it
to the UI (`HandedGesture.1`). A latch in flight when the route is diverted again (the render thread panicked
between `Restored` and the `Latch`) is the token's one hard case, and it is closed in both directions: the
`Diverted` says one is on its way (`returning`), the UI holds every input it then takes (`held_back`) until the
render thread hands the latch back (`FromRender::Returned`), and a restore due meanwhile waits for it
(`restore_due`), so no input is routed on either side while neither holds the latch. Before, the UI routed the
next wheel with no latch, to the list under the pointer. Before that, a gesture begun while diverted went on to
the list under the pointer once restored ([20, 30] for [0, 50]). A
replay checks the rule at every line of every log it replays: no gesture on the compositor while diverted, and none
the compositor scrolls a container in on the UI too. At every transition (`diverted`, `restored`) it also checks
conservation (round 13, Codex r13 6). The gestures latched after the transition are exactly those latched before,
and each has exactly one owner then: the UI, or the compositor scrolling a container in it. A dropped latch fails,
as does one on both sides.

**A frame from before a panic is never reused by its numbers** (round 12, the fault matrix). The compositor made
afresh draws on the timeline the one before drew on, and the frames that one presented come back to the new one's
pool as the timeline lets them go. Their scroll lists were versioned, and their property trees numbered, by the old
one's counters, which the new one runs past again: a frame reused so kept the list from before the panic when its
version matched (frame 102 showed the inner list at 80, then 163 again). The new compositor records the last frame
the old one drew (`born`) and takes no frame so numbered or older into its pool: the class (a buffer outliving the
writer whose counters identify its content) is closed where such buffers enter.

**One exact motion account** (round 13, replacing round 12's 1/256 px grid in f32). A gesture's motion is cut into
pieces where frames fall, summed by the compositor at each frame and by the UI at each pull. In floating point those
sums depend on where the cuts fall (round 12: 163.00002 or 163 by the frames), a grid in f32 drops motion finer than
its step (Codex r13 3: 1,024 steps of 1/1,024 px moved nothing), and it is exact only below 2^16 px (Codex r13 4:
at 36,000,000 px forty 1 px steps moved 0 with a frame between each and 40 without). Every scroll position and
every piece of motion is now an `i64` count of 2^-20 px (`compositor::motion`, `Exact`): the layout's scroll model,
the paint's `ScrollNode` (`pos`, `end`), the compositor's owed motion, outbox, predictor samples, fling progress and
`ScrollDelta`, and the UI's holder. f32 is only the GPU's and the public API's: an offset shown is `px2(pos)` and
nothing reads it back as motion. Where the platform's f32 deltas enter, the router's `Carry` rounds each to whole
units and carries the remainder to the next, so nothing below a unit is lost. A resampled or flung position is
computed in f64 and rounded once, as a position; each cut is the difference of two positions, so the predictor's
applied motion equals the change of position exactly and the pieces sum to where the events put the content,
whatever the frames and pulls (property test `random_motion_under_random_frames_settles_on_the_same_unit`: random
deltas, frames and pulls, 60 trials up to 36,000,000 px, shown = UI = sum). A fling ends at a time set by its own
speed (`Fling::lasts`), not when a frame finds it slow, so it ends at the same unit whatever the frames
(`a_fling_ends_at_the_same_place_whatever_its_frames`, failed 294.71 against 295.25). Jumps (`scroll_to`, a reveal)
and range ends come from layout in f32 and land on the nearest unit (a jump to 0.1 px is at 0.1000003814); motion
from there is exact. Chrome differs here: its scroll offsets are floats (`gfx::PointF`, snapped to physical pixels
for paint), so the same divergence of summed cuts is open there, and an offset past 2^24 px loses whole pixels. Ours
is a deliberate improvement, not a port: positions are exact to 2^-20 px up to 2^43 px.

Order holds in both directions: what was sent before the switch reaches the UI before anything sent after it, since
both reach the UI's one queue in the lock's order. While diverted, the UI is the only party that scrolls, so taking
scroll in before handling input holds trivially. A render thread that never recovers (`Failed`) leaves input going
straight to the UI.

**Input log and replay.** The log is written where each input is taken, in the one order it is taken in, as rr and
ReVirt log each nondeterministic input where it is consumed. The writer (`input_log::Route`) is held under
`Forwarding`'s lock, with the switch:

- On the forwarding path the render thread writes each event as it takes it. On the fallback path the platform
  writes it as it sends it straight to the UI, and `divert` writes what it drains. One writer is used per input,
  never both, since both take the one lock.
- `obs seen <n>|none` comes before an event whose frame on screen differs from the last one written. The UI routes
  an input in the frame that was on screen when it came (`Seen`), so a replay routes a click the way the live run
  did, even before a scroll is drawn.
- `obs drawn <n> [<ms>]`: the render thread drew frame `n` for its display at `<ms>`. A replay draws there, at that
  time, so the frames that `seen` names exist and show what the live ones did (a fling carried on to the display
  time: round 11, Codex 2, 40 vs 78.79 px).
- `obs diverted`, `obs restored`: where input went straight to the UI, and where it came through again. A replay
  settles and hands over the gesture at `diverted`, and scrolls wheels on the UI until `restored`.
- `obs uncommitted`: the log began before the render thread applied any commit. A wheel it takes then is routed in
  the committed scene, which is none, and scrolls nothing (input before the first frame is routed in what was
  committed); a replay, which begins from a committed scenario, routes no wheel until the first `commit`. Found by
  the live-run property test (live 0, replayed 120).
- Times are written to the nanosecond: a gesture's resampling and fling read event times, and microsecond rounding
  replayed a fling 0.0002 px off.
- `resize <w> <h> <scale>`, `focus`, `visible`/`hidden`, `chrome <flags> "<title>"`, `access on|off`, `quit`: what
  the platform told of its window, logged where it is taken, like input. A replay applies each at its place.
- A key is logged losslessly: every named key by its name (all 306 of winit's `NamedKey`), a character, a dead key,
  or an unidentified key with its native code (round 11, Codex 4: `Redo` read back as `Unidentified`).
  A dead key's character is always quoted, and one with none is `dead none`, so `Dead('-')` reads back as itself
  (round 12, Codex 4: it read back as `Dead(None)`).
- `obs grant [<ms>]`: a main frame was granted, for the present at `<ms>`. The UI resamples a held drag at that
  present (`HeldMoves::sample`), and a replay does the same at the logged time. A replay's clock first runs on to
  the line's own time, so the timers due by the grant run before its frame, as live (round 12, Codex 5a: 0 vs 100).
- `obs settled`: no frame was drawn, so the gesture's resampling and fling were applied whole here; a replay does
  the same.
- `obs mail <site> <n>`: the UI took one value another thread sent the mailbox so named (its maker's call site,
  hashed, and how many that site made before it), here among the input (round 12, Codex 7). A mailbox's wake goes
  through the one producer like input: through the render thread, which logs and hands it on in its order
  (`ToRender::Mail`, `FromRender::Mail`), or, while diverted, logged and sent straight under the switch's lock. The
  UI delivers one value per wake where its queue has it, and never in its idle step. A replay delivers that
  mailbox's next value at the line and no mail anywhere else; the value is what the sender sends the replay (the
  log holds where, not what).
- A press on the edge of a window that draws its own frame is the window's, live and replayed (Codex 5b), and a
  window resized to nothing and back replays (Codex 5c). A frame drawn before any commit is an empty window in a
  replay too, which input routed in it finds nothing in.
- `obs commit` and `obs abort`: the commit was taken in or dropped.
- `act <action> <node> <tree> [data]`: an assistive technology's action, taken and logged like any other input.

A replay (`replay::replay_observed`) reads the log in sequence:

- it dispatches a wheel, and handles at once one that is handed on;
- it handles other input with the pull at that point, in the frame `seen` names, holding drag moves with the live
  `HeldMoves`;
- it applies what the platform told at its place, with the pull there;
- at `drawn` it draws for the logged display time and keeps the frame;
- between `diverted` and `restored` it scrolls wheels on the UI, from the gesture handed over;
- at a grant it resamples the held moves at the logged present and runs a main frame for it, whose commit waits for
  `commit`, or is dropped at `abort`.

`Follow`, the observation pairing and `Observation::{Applied, Cut, Take, Built}` are deleted. The log's magic stays
`gui-input-log 2`: event lines that existed are unchanged, and a reader of the old lines reads them as before.

Deleted: `Numbering`, `InOrder`, `ToRender::Cut`, `FromRender::Through`, the outbox's runs, cuts, `applied`,
`through`, `open_due`, its mutex, and `Scenario::send_after`. Round 10 deleted `ToRender::Act`, `ToRender::Record`,
`FromRender::Act`, `AccessEvent::Action`, `Threads::act` and the render thread's own recorder.

## Alternatives

- **B, the brief as written (two producers, `Through` bounded, replay keyed by platform sequence).** It treats the
  symptoms of two writers. The order of a click and a wheel still depends on a wait, and the log on a reconstruction
  of two threads' interleaving.
- **Measure before building.** Rejected by the project's rule: the final design is built and measurement proves it.
- **Scroll on the UI thread (Android, Flutter).** This fails ADR-0015's goal that scrolling never waits for the UI.

## Evidence

Measured on 2026-11-25 with `./target/release/examples/stress --run threads` (release, Apple Silicon, offscreen
window, Ticker pacing at 60 and 120 Hz, two runs each). The offscreen ticker has no surface. The surface acquire
bound is measured on a real surface below.

- *Moves*: 400 pointer moves 2 to 6 ms apart, on a page that draws every frame.
- *Click after wheel*: 100 × (wheel, then MouseDown at once).
- Latency runs from the platform's send to the UI's take. The render hop runs from the send to the render thread
  handing the input on.

| | moves p50 / p99 | click after wheel p50 / p99 | render hop p50 / p99 / max |
|---|---|---|---|
| before (0032), 60 Hz | 0.02–0.03 / 2.48–6.96 ms | 5.26–6.33 / 21.40–27.28 ms | (none) |
| before (0032), 120 Hz | 0.02 / 2.29–3.88 ms | 0.67–0.99 / 11.13–13.60 ms | (none) |
| after, 60 Hz, drawing | 0.02–0.03 / 3.54–5.08 ms | 0.03 / 7.62–10.89 ms | 0.01 / 0.16–0.19 / 0.38 ms |
| after, 120 Hz, drawing | 0.02–0.03 / 2.51–5.22 ms | 0.04–0.68 / 8.88–10.84 ms | 0.01 / 0.40–0.42 / 0.46 ms |
| after, 60 Hz, idle | 0.03 / 1.87–3.67 ms | (none) | 0.01 / 0.02 / 0.04 ms |
| after, 120 Hz, idle | 0.03 / 4.12–5.10 ms | (none) | 0.01–0.02 / 0.03 / 0.09 ms |

**Budget: met.** The render hop adds 0.03 ms at p99 when idle, and at most 0.46 ms while drawing every frame. The
UI's take p99 is dominated by the UI's own frames in both models. The click after a wheel no longer waits for
`Through`: its p50 at 60 Hz falls from about 5.3 ms to 0.03 ms, and its p99 roughly halves.

**On a real surface** (round 10, 2026-11-25). Measured with the `docs` example (release, Apple Silicon, macOS live
window 1280×852 on a 60 Hz display, Surface pacing). `GUI_INPUT_HOPS=<file>` records each hop to the UI. A Swift
driver posts synthetic events through the HID tap:

- *draw*: 1500 moves 2 to 4 ms apart, a pixel wheel every fourth (the page scrolls and draws throughout), and a
  click every fiftieth;
- *moves*: the same moves without wheels;
- *resize*: a 600-step drag of the window's corner, 4 ms apart, with Shift taps and pixel wheels during it.

| Run | hop measured from | n | p50 | p99 | max |
|---|---|---|---|---|---|
| draw | the platform's send | 1116 | 0.004 ms | 0.056 ms | 4.66 ms |
| draw, clicks only | the platform's send | 60 | 0.003 ms | 0.114 ms | 0.114 ms |
| draw | the event's time | 1116 | 0.012 ms | 16.55 ms | 17.1 ms |
| moves | the event's time | n/a | n/a | 0.027 ms | 3.9 ms |

- **The forwarding hop is within budget while drawing:** p99 0.056 ms from send to handed on.
- **GPU calls on the render thread** (timed in place for these runs, then removed): acquire p50 1.14 ms, p99
  1.50 ms, max 2.9 ms (the `ACQUIRE_PROBE` bound); draw p99 0.67 ms; `end_frame` p99 0.05 ms. Only three gaps over
  5 ms between queue reads in the whole run, which bounds the 4.66 ms maximum. No GPU call makes input wait beyond
  the budget at p99, so none was moved off the forwarding path.
- **The 16.5 ms tail when timed from the event's time is AppKit's, not ours.** Moves made from a tapped scroll event
  carry the scroll's OS time. Their delay before the platform sends them runs uniformly from 1 to 17 ms (340 of
  1116), which is the shape of delivery aligned to the display's frames. Moves that winit stamps on arrival show
  none. Timed from the send, the same run's p99 is 0.056 ms.
- **During a live resize the window takes no input.** AppKit's resize tracking loop takes the drag itself and
  holds the rest. Of 300 wheels only 68 arrived, coalesced, all after the drag ended (1.08 s p50, 3.0 s max from
  their OS time). The Shift taps arrived as one `Modifiers` change at the end, 3.0 s late. So no input can wait on a
  surface reconfigure during the drag. The input that arrives afterwards was forwarded in 0.007 to 0.36 ms. Acquire
  stayed under 0.93 ms throughout the resize (187 frames).

**At 20,000 scroll containers** (round 11, Opus I1/I2; release, Apple Silicon, 2026-11-25). The page is 20,000
overflow-y containers, each 4 × 0.01 with an 8 × 1 child.

- *Compose* (ADR-0032 I7). Before, each frame rebuilt every container's shown scroll and found each thumb by a
  scan of all thumbs: 60 ms a frame, and a render hop p99 of 34 ms. Now a frame visits only the containers the
  compositor holds apart from committed, those it visited last frame (one come to rest), and those displaced at
  their committed offset by snapping (`compose::ScrollShown`). A commit compares its table with the last and visits
  the entries that differ; it visits all only when the containers themselves changed (another index). Thumbs are a
  map by transform. Each frame's `Presented::scroll` is a reused buffer rewritten only when the shown list changed,
  so steady composing allocates nothing (`compose_alloc`). A compose at rest visits 0 containers (was 200,030,000 in
  the counted test) and takes at most 2.2 µs of 100. With the UI and render threads running offscreen on that page,
  400 moves hop to the UI in p50 0.010 ms, p99 0.021 ms, max 4.0 ms.
- *after_layout*. Each tracked handle's hooks ran once by comparing it with every handle run before it: O(H²),
  2.3 s a frame at 20,000 handles. The handles run are now a hash set: 20,001 lookups in a first frame (was
  199,990,001), a frame after one list scrolled 34 ms in all (two layout passes, 40,002 lookups).

| What | Test (fails with the change reverted where marked) |
|---|---|
| Order: wheel then click, through the render thread | `compositor::tests::a_click_after_a_wheel_handed_to_the_ui_is_handled_after_it` |
| UI takes each input's pull right before it | `window::tests::what_rides_with_input_is_taken_in_right_before_it` |
| Fallback order (fails with `divert`'s drain removed) | `thread::tests::input_diverted_from_a_render_thread_that_panicked_keeps_its_order` |
| Mount identity, remount while behind (failed before `mount_scrollers`) | `remounted_list_does_not_take_previous_lists_wheel` |
| Acquire waits handing on input (failed with a single wait: an `Output` change mid-wait) | `thread::tests::moved_to_120_hz_while_a_frame_waits_*` |
| Replay in sequence: Codex r8 3, r9 I3, Opus r9 I1 | `replay::a_replay_takes_in_what_the_live_threads_took_in`, `top_release_after_two_coalesced_moves_is_replayed`, `a_click_after_three_coalesced_drag_moves_is_replayed` |
| A wheel between a grant and its commit is kept | `replay::a_wheel_between_a_grant_and_its_commit_is_kept` |
| Linear commit over 256 pulls | `a_commit_of_sparse_runs_is_linear_in_entries` |
| Pulls at a commit land in order, each clamped (round 10, fails summed) | `compositor::tests::pulls_outstanding_at_a_commit_land_in_order_each_clamped` |
| Switch held until the recovered compositor's full commit (fails restoring at recovery) | `thread::tests::input_goes_to_the_ui_until_a_recovered_compositor_can_scroll` |
| 200 wheels around a render panic, run → recover → restore (fails: 588 of 600 px) | `compositor::tests::wheels_around_a_render_panic_are_all_scrolled` |
| Fallback input is logged where taken, sent through `Threads::input` (fails without its record) | `window::tests::input_taken_on_the_fallback_path_is_in_the_log` |
| A click after a resize is replayed at the new size (fails with `told` skipped in replay) | `replay::a_click_after_a_resize_is_replayed_at_the_new_size` |
| A frame is replayed at its display time (fails drawn at the clock: 40 vs 78.79) | `replay::a_frame_is_replayed_at_its_display_time` |
| Every key survives the log (failed: `Redo` became `Unidentified`) | `input_log::tests::a_redo_key_survives_a_log`, `every_key_survives_a_log` |
| A recorded live run (wheels, fling, click, keys, drag, resize, panic) replays to the same frames and handler calls (about 100 frames compared a run) | `replay::a_recorded_live_run_replays_to_the_same_frames_and_handler_calls` |
| Wheels before the first commit replay as live (fails with `uncommitted` ignored: 160 vs 40) | `replay::wheels_before_the_first_commit_replay_as_live` |
| A trackpad step held for resampling survives a render panic (failed 5 vs 35) | `compositor::tests::a_trackpad_step_held_for_resampling_when_the_render_thread_panics_is_scrolled` |
| A gesture's latch survives a render panic (failed: outer list 60 vs 0) | `compositor::tests::wheels_around_a_render_panic_keep_the_gestures_latch` |
| A device made again keeps input on the compositor (failed: diverted) | `thread::tests::a_device_made_again_keeps_input_going_through_the_compositor`, `compositor::tests::wheels_during_a_device_rebuild_keep_the_gestures_latch` |
| Compose at rest visits no container at 20,000 (failed: 200,030,000 visits) | `compositor::tests::composing_twenty_thousand_scrollers_at_rest_visits_none` |
| A frame runs 20,000 tracked handles' hooks in linear time (failed: 199,990,001 comparisons) | `scroll::tests::a_frame_of_twenty_thousand_tracked_lists_runs_each_hook_once_in_linear_time` |
| A click before a scroll is drawn is replayed in the frame on screen (fails without `seen`/`drawn`) | `replay::a_click_before_the_scroll_is_drawn_is_routed_in_the_frame_on_screen` |
| A held drag is resampled at the grant's present (fails without the grant's time) | `replay::a_held_drag_is_resampled_at_the_grants_present_time` |
| One latch owner moving with the route (round 12; failed [20, 30] for [0, 50], and [10, 20] for [20, 10]) | `compositor::tests::a_gesture_begun_on_the_compositor_keeps_its_list_on_the_ui`, `a_gesture_begun_while_diverted_keeps_its_list_after_restore`, `a_gesture_ended_and_begun_while_diverted_is_the_new_ones_after_restore` |
| A device made again off the input path (failed: hop p99 243 ms) | `compositor::tests::a_device_made_again_never_holds_input_up` |
| A trackpad step while no frame is drawn comes before the click after it (failed 0 vs 30) | `compositor::tests::a_click_while_the_device_is_lost_comes_after_the_trackpads_step` |
| A fading scrollbar is composed each frame (failed: opacity 0 throughout) | `compositor::tests::a_fading_scrollbar_is_composed_each_frame_until_it_is_gone` |
| `Dead('-')` and every edge character survive the log | `input_log::tests::every_key_survives_a_log` |
| Timers due before a grant, a press on a resize edge, a window resized to nothing (failed 0 vs 100; 2 clicks vs 1; a panic) | `replay::a_timer_due_before_a_grant_is_in_its_frame`, `a_press_on_a_resize_edge_is_the_windows_in_a_replay_too`, `a_window_resized_to_nothing_and_back_replays` |
| Mail from another thread replays in its place among the input, on both routes (failed: the handlers heard mail and clicks in another order; 2 of 24 logged with the route skipped) | `replay::mail_from_another_thread_replays_in_its_place_among_the_input`, `mailbox::tests::each_value_rings_its_address_once_and_is_delivered_by_it` |
| A frame from before a render panic is never reused by its numbers (failed: 20 shown for 40) | `compositor::tests::a_frame_drawn_before_a_render_panic_is_never_reused_by_its_numbers` |
| Input put back behind a latch being installed is handed on when diverted (failed: only the `MouseUp` came) | `thread::tests::input_diverted_from_a_render_thread_that_panicked_keeps_its_order` |
| A latch in flight when diverted again goes back before input is routed (both fail without `Returned`) | `thread::tests::a_latch_in_flight_when_diverted_again_goes_back_before_input_comes_through`, `window::tests::input_waits_for_the_uis_latch_on_its_way_back` |
| The adapter's self-test and the old device's drop off the render thread (failed: hop p99 3.1–4.6 ms in release) | `compositor::tests::a_device_made_again_never_holds_input_up` |
| A resampled gesture settles exactly, on both sides, at 80 frame phases (failed: 163.00003 for 163) | `compositor::tests::a_resampled_gesture_settles_exactly_where_its_events_put_it_whatever_the_frames` |
| A fling settled by a fault ends where the fault-free one does (failed: 53.01 for 295.25) | `compositor::tests::a_fling_settled_by_a_fault_ends_where_the_fault_free_fling_does` |
| A fling ends at the same unit whatever its frames (failed: 294.71 for 295.25) | `compositor::tests::a_fling_ends_at_the_same_place_whatever_its_frames` |
| A gesture settled with its fingers down still flings at the lift (failed: 40 for 294.58) | `compositor::tests::a_gesture_settled_with_its_fingers_down_still_flings_as_they_lift` |
| Motion finer than a pixel adds up (failed: 0 for 1) | `compositor::tests::steps_finer_than_a_pixel_add_up_exactly` |
| Deep in a long list each step moves it, whatever the frames (failed: 36,000,000 vs 36,000,040) | `compositor::tests::deep_in_a_long_list_every_step_moves_it_whatever_the_frames` |
| Out and back from a jump settles on the jump's unit | `compositor::tests::a_gesture_out_and_back_from_a_jump_settles_where_it_began_whatever_the_frames` |
| Random motion under random frames and pulls settles on one unit on both sides | `compositor::tests::random_motion_under_random_frames_settles_on_the_same_unit` |
| A mailbox made and disposed 1,000 times keeps nothing | `mailbox::tests::mailboxes_made_and_closed_again_and_again_keep_nothing` |
| A live run with a fling cut by a panic replays to its frames (failed: 1167.58 for 1164.98, the settle unstamped) | `replay::a_recorded_live_run_replays_to_the_same_frames_and_handler_calls` |
| A lifted gesture whose momentum ended while diverted still flings (failed 83 for 1,387.06) | `replay::a_gesture_whose_momentum_ended_while_diverted_still_flings` |
| The hop is timed from the send, not the event's stamp (failed: 200.04 ms for an event stamped 200 ms early) | `compositor::tests::an_inputs_hop_is_timed_from_its_send_not_its_stamp` |
| The fault matrix (below) | `replay::a_fault_before_any_input_loses_and_moves_no_wheel_and_replays` |

**The fault matrix** (round 12, the user's ask). A script of 33 events on the app's threads offscreen: notched
wheels and a trackpad gesture with the platform's momentum over an inner list, a notched wheel and a gesture over a
wheel handler, a click, a drag, a move off the inner list onto a strip only the outer list is under and a gesture
there, and a click that reads both lists. Before each event in turn it injects a render panic, a lost device, or a
device held lost 60 ms, and a second fault before the next event while the first recovers (panic then panic, panic
then held loss, held loss then panic, loss then loss): 227 faulty runs and one with no fault, four at a time. Each
faulty run must hear what the one with no fault heard (the lists where they ended included), and its log must
replay to its live frames and handler calls with the latch's one owner checked at every line. It found five
classes, all fixed: frames drawn before any commit replayed with content (the live window was empty); a frame
reused from before a panic by its numbers (4 of 227 runs, all panics); in release, input put back behind a latch
being installed left undrained by a second panic (a wheel lost), and a latch in flight when diverted again (case
[(28, panic), (29, panic)]); and in debug, 2 of 6 runs, a gesture settled 1 ulp off by its frames' timing
(case [(20, panic), (21, held loss)], whose replay showed it from the gesture on, not from the fault: motion on a
grid, above). After, on an Apple M-series laptop, macOS, Metal: 0 found in 2 release and 3 debug runs (227 faulty
runs each); input handed on while recovering hopped to the UI in p99 0.14 ms in release (3182 hops) and 0.26–0.27
ms in debug; `a_device_made_again_never_holds_input_up` hopped in p99 0.016–0.031 ms at rest and 0.014–0.027 ms
while the device was made again, release and debug alike.

Round 13 made the matrix prove what it claims (Codex r13 6). The compositor flings (the log's platform says it does
not fling itself), and each script step's events are stamped 4 ms apart whatever their sending's delays, so a
gesture's speed and fling are the same in every run. Each run measures how long the UI took each input it was handed
while recovering; a run that diverted input and timed none fails, and so does an empty sample over the matrix. The
replay checks one owner at every transition (`diverted`, `restored`): the gestures owned before are owned after, by
exactly one side. A frame seen live that the replay never drew fails. New plans fault while diverting, while the
device is made on its helper thread, between `Returned` and the restore, and past the terminal limit (four panics,
from which the handlers must still hear input). With flings on, the matrix found four more classes, all fixed:
a gesture routed to the UI over a non-scrolling area was counted as an owned latch (the check was wrong: the UI owns
where such a gesture goes); a fling's end depended on which frame found it slow; a gesture settled with its fingers
down (no frame drawn) forgot its events, so its lift flung nothing; and a log's `diverted` and `settled` lines had no
time of their own, so the replay settled a fling to a different instant (the line is now stamped with the time
settled to). A notched wheel that cuts a fling short stops it where the frame shown then put it, which no two live
runs share, so the script lets the fling end before its next step's wheel; each run's replay still checks such a
cut frame by frame. Two harness rules came with this. A step after a lift that no frame showed (no device yet)
waits out the longest fling, 1.6 s, as drawn frames would have shown it ending. Frames drawn before the log began
are its starting state, not frames it must account for. The matrix also found that a device made again waited for
the lost device's present callback, which never comes, so a plain loss under load drew nothing again
(`a_device_made_again_keeps_input_going_through_the_compositor` now checks it).

**A gesture's motion goes with its latch** (round 13, the matrix with flings on). The UI scrolled a diverted
gesture's events but had no fling of its own, so a fault before or during a gesture's lift ended it where its
events put it (inner list 163 against 1,467.06, outer 64 against 1,022.75). The fling now belongs to the gesture,
not to the side that routes it. `HandedGesture` carries the gesture's motion (`GestureMotion`: its events of the
last fling window and when its fingers lifted) both ways with the latch. The compositor hands its gesture's motion
to the UI while the fingers are down (`Scrolls::motion_of`). The UI records each precise step it routes for the
latched gesture as the compositor does, the lift included, and hands that back at restore. The compositor adopts
it (`Scrolls::adopt`): with the fingers still down it keeps the events, so a lift on the compositor flings as fast
as they went; lifted on the UI, it flings from the lift's time with the same `Fling`, carried on by time from
then, so it ends where it would have with no hand-over
(`a_gesture_lifted_while_the_ui_routes_it_flings_as_one_lifted_here`, failed 30 against 216.88). A wheel of
another gesture while diverted still stops the fling (`Route::took_wheel`, applied before the latch comes back).
Past the terminal limit nothing is restored, so the UI's last gesture does not fling.

The fling outlives the latch. Under load the matrix found one case, [(6, panic as it diverts), (6, panic)], in which
the render thread diverted 140 ms after the fault. The rest of the gesture, its lift and the platform's momentum
through `momentum-ended`, all reached the UI before the restore. The momentum's end ended the UI's latch, and the
lifted motion went back only with a latch, so nothing went back and the gesture flung nothing (inner list 163
against 1,467.06). A `GestureMotion` now carries the container it moved (`to`). A lifted motion whose latch has
ended goes back on its own, unless the UI latched another gesture since, which stopped its fling. The compositor
adopts it on that container by identity (`Compositor::take_gesture`). A lifted motion returned to the UI with no
latch is kept so that it can go back again
(`replay::a_gesture_whose_momentum_ended_while_diverted_still_flings`, failed 83 against 1,387.06).

**How the hop is measured, and the evidence for the bar** (round 13). The bar is the render hop's p99 under 1 ms
while recovering. The hop runs from the platform's send to the render thread handing the input on, as defined
above. Each `ToRender::Input` and `ToRender::Wheel` carries `sent`, taken as `Threads` sends it (the switch's lock
included), and `FromRender::Input` hands that on. The hop is `sent.elapsed()` where input is handed on, offscreen
and under `GUI_INPUT_HOPS`. It used to be timed from the event's own time (`at`). The two agree for input that winit
stamps on arrival. They differ for an OS's stamp or a script's clock, which come before the send. The matrix
stamps its events on a 4 ms script clock, so a gesture's speed is the same in every run, and its sending thread
sleeps to a millisecond before each stamp, then spins. On a loaded machine that thread woke late, and the wait
before its send was counted as the render thread's hop
(`compositor::tests::an_inputs_hop_is_timed_from_its_send_not_its_stamp`, failed: a hop of 200.04 ms for an event
stamped 200 ms before its send). The matrix also had a hole: it kept hops and takes only up to its faulted step's
settle. A fault that the render thread took late was handed on after that settle, and the next step dropped those
samples, so the slowest tail was never measured. Hops and takes now count from the first fault to the end of the
run.

Measured on 2026-11-26 (release, Apple M-series, 10 cores, macOS, Metal). Other processes used about 370% CPU
throughout (python 225%, qemu 146%, rustc), and the load average was 17–51. Each matrix ran two scheduling
canaries, each sleeping 1 ms in a loop: one at a test thread's default priority, and one raised as the platform
thread is (`ThreadType::Platform`, `QOS_CLASS_USER_INTERACTIVE`). The sending thread runs at platform priority, as
a real app's platform thread does.

| Run | hop measured from | hops | hop p99 / max | UI take p99 | send late after stamp p99 / max | canary oversleep p99 (default / platform) | found |
|---|---|---|---|---|---|---|---|
| 1 worker, 60 plans | the stamp | 74 | 6.81 ms / n/a | 8.53 ms | not measured | 5.00 / 5.00 ms | 0 |
| 1 worker, 60 plans | the send | 73 | 0.240 ms / 0.240 ms | 1.99 ms | 7.16 / 15.2 ms | 3.83 / 3.85 ms | 0 |
| 4 workers, 328 plans | the send | 829 | 0.152 ms / 1.11 ms | 6.22 ms | 3.96 / 9.48 ms | 3.18 / 3.18 ms | 0 |
| 4 workers, 398 plans (fine steps, mailboxes, every sample kept) | the send | 4179 | 0.254 ms / 10.1 ms | 5.24 ms | 4.29 / 15.5 ms | 2.86 / 2.86 ms | 0 |

A standalone canary (15 s, same machine and load) separates the threads. A thread at the render thread's fixed
priority 47 overslept a 1 ms sleep by p99 0.27 ms and max 0.30 ms. Woken by a channel send, it woke in p99 12.5 µs
whether the sender was at default or platform priority. A default-priority receiver woke in p99 4.04 ms. A sleeping
thread overslept by p99 4.7 ms at default priority and by p99 4.8 ms at platform priority. Platform priority is
user-interactive QoS, a timeshare class, and it does not escape this load. So:

- The render thread itself is not starved: its wake from the channel stays in microseconds.
- The 6.8–10 ms "hop" p99 of earlier rounds was the sending thread waking late before its send. That thread is the
  test's stand-in for the platform thread, and the canaries show the same lateness at both priorities. Timed from
  the send, the hop meets the bar in every run.
- The UI's take is timed from the event's stamp, so it includes that sender lateness and the UI thread's own wake
  at default priority. It stays under its 16 ms bar.

The single maximums (1.1 ms with 4 workers, 10.1 ms with every sample kept) are below p99 and within the canaries'
own maximums (21–42 ms). **Revisit** if a matrix's hop p99, timed from the send, passes 1 ms while the
fixed-priority canary stays under 1 ms: that would be a wait of ours.

**Each decision is load-bearing in the matrix** (round 13, revert-proof). Each of decisions 1–5 was reverted on
its own, the release matrix was run against it, and the decision was restored:

| Decision reverted | How | Matrix result |
|---|---|---|
| 1. The fling in the closed record | `into_accepted` drops the fling; `settle_gesture` ends it | 143 of 328 cases fail (handlers: a fling lost) |
| 2. Recovery never blocks | `recover` makes the device inline; `Returned` waits for the device | 57 of 328 fail (33 handlers, 24 replay frames) |
| 3. Terminal fallback | the UI loop returns on `Failed` | 3 of 328 fail (the 3 terminal plans: handlers heard nothing, or the replay differs) |
| 4. The exact motion account | `Carry` drops its remainder | 0 of 328 fail before the fine steps; fails with them (fine list 2.3999977 for 2.3999996) |
| 5. Mailboxes reclaimed on dispose | `on_release` only closes the mailbox | 0 of 328 fail before the rebuilt page's mailboxes; fails with them (2 disposed builds' mailboxes kept) |

Decisions 4 and 5 were invisible to the matrix, so it was strengthened. Its other oracles compare runs with runs,
and a run with its replay. Every route goes through the router's one carry, so a dropped remainder changes all
runs alike. The script now ends with six steps of 0.4 px (an `f32` off the unit grid) over a fine list, and the
run with no fault must show that list at the steps' exact sum (`fine_end`, 2,516,582 units). Every faulty run must
hear what that run heard. The page now builds again on each click, each build in a scope of its own whose mailbox
handler holds a counted token. When the scope is disposed, its handler must be let go at once
(`Faulted::mail_kept`, 0).

## Revisit conditions

- **A slow draw.** A draw that takes most of a frame on real hardware would make input wait for it. The remedy is to
  hand input on from a thread of its own that shares the outbox's pull, under a lock held only for the pull.
- **A display whose acquire does not return at its vblank.** The probe bound would not hold there. Measure on that
  surface with `GUI_INPUT_HOPS` before changing it. macOS was measured in round 10; Windows, Wayland and X11 have not
  been measured.
- **AppKit's delivery of tapped scrolls.** Up to a frame passes between a scroll's OS time and its delivery. That
  is before this ADR's hop, but it is input latency we own the stack for. Disabling AppKit's event coalescing, or
  reading the HID stream directly, is the open question.
