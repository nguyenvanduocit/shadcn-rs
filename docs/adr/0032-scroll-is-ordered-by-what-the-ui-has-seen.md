# 0032. Scroll is ordered by what the UI has seen

Status: accepted under delegation 2026-11-25 (wave C (a), round 7); I1, the input half of I2, the residual race
and the replay text superseded by [ADR-0033](0033-all-input-goes-through-the-render-thread.md) (round 9), which also
makes I4's mount identity exact. I3–I7 stand. Supersedes the generation rule of
[ADR-0015](0015-gui-runs-on-three-threads-timed-by-presentation.md) ("an offset set by the program carries a
generation, and older deltas are dropped") and the scroll stamps, the hybrid logical clock and its `LOGICAL`
watermark that rounds 4–6 built on it ([ADR-0028](0028-a-frame-can-fork-in-release.md) amended to match).

## Context

The render thread scrolls a container at once (ADR-0015); the UI owns its offset and may assign it (`scroll_to`,
a virtual list's jump, a caret brought into view). The two meet at a commit. The question is which of the two
wins when they cross: a reader turns the wheel while the UI, behind, handles a click on "Top".

Rounds 4–7 ordered the two by time. A program write was stamped, every compositor delta was stamped with the OS's
event time folded into a hybrid logical clock (Kulkarni et al. 2014), and a commit dropped the deltas stamped before
its write and kept those after (last writer wins). Each round fixed the findings of the one before, and each review
found more of the same class (round 7: Codex 9 and Opus 2 failures, almost all stamps). A two-model committee
traced them to two arguments that no patch answers.

1. **No shared clock.** A wheel's time is the OS's input clock, read on the platform thread. A program write's is the
   frame clock, read on the UI thread. A script, a replay or a test drives a third. A clock set back, an `f32` of
   seconds at long uptime, a device whose events come late, and the fold of one clock into another each reorder
   events the reader saw in one order. An HLC orders stamps consistently with causality it is *told* of, and
   nothing tells it that a wheel turned before a click: the two events reach different threads.
2. **Bounded memory against per-stamp order.** Splitting the compositor's untaken scroll at an arbitrary stamp needs
   every stamp kept. A UI stalled for a long time then holds memory proportional to the wheel events since the stall.
   Bounding it (round 6's `OWED_CAP`, merged runs) loses the stamps a later split needs (Opus N2: after a 3 s stall,
   a merge crossed a jump).

The cause is the model. Time is not the order the reader acted in, and nothing in the system records that order
except the platform thread, which sees every event in the order the OS delivered it.

## Options

- **A. Physical order (stamps).** What rounds 4–7 built. Rejected by the two arguments above.
- **B. Causal order as Chromium's compositor does it.** A container's offset is a synced property
  (`cc/trees/property_tree.cc`): the main thread's value is a base. The compositor's delta rides on top. At each
  main frame the main thread pulls the delta (`PullDeltaForMainThread`, ~2108) and adds it to its own offset
  (`property_tree_layer_tree_delegate.cc:38`, `UpdateScrollOffsetFromImpl`, ~44). A commit brings the new base back:
  the delta it sent is dropped, and the rest stays on top (`PushScrollUpdatesFromMainThread`, ~2196). An aborted
  commit's sent delta is handled by `ApplySentScrollDeltasFromAbortedCommit` (~2282) according to
  `main_frame_applied_deltas`. When the main thread already applied the delta, it is treated as applied and committed.
  Otherwise it goes back to the unsent delta (`client_layer_tree_host_impl.cc:84`, `BeginMainFrameAborted`). I4's abort
  is the first case: the UI took the delta into its own offset, so the next commit's base carries it. A scroller may ask to clobber
  the compositor's value with its own (`commit_state.h:177`, `scrollers_clobbering_active_value`). Deltas are
  collected for the main frame in `input_handler.cc` (~1347, `CollectScrollDeltas`). The UI's writes are then
  ordered against compositor scroll by *what the UI had taken in when it wrote*, and nothing compares time. The gap
  is input the UI handles between main frames: a click handled late runs before the main thread has pulled the
  wheels turned before it, so "wheel, then Top" can end below Top.
- **C. B, cut at the platform thread.** As B, and the UI takes in, before it handles any input, what the compositor
  moved of every wheel the platform sent *before* that input, and nothing of the wheels sent after.
- **D. Compositor-arrival order** (the render thread decides by what reached it first). Rejected. A UI write reaches
  the render thread only in a commit, so the render thread must hold every wheel until the UI answers or drop what it
  already showed. The first adds a UI round trip to every wheel. The second visibly erases scrolling the reader saw.

## Decision

C. Each invariant has one owner.

- **I1, order** (superseded by ADR-0033): every input goes through the render thread, the one producer of the UI's
  queue, so a click comes after every wheel the render thread applied before it. The platform's wheel numbers, cuts,
  `Through` and `InOrder` this item once had are deleted.
- **I2, take before handling** (`app.rs`, `Pipeline::take_pulled`; `compositor/outbox.rs`): before the UI handles an
  input it takes in what the compositor scrolled before the render thread handed that input on. That is the pull
  riding with the input (ADR-0033). A main frame takes in the pull made at its grant (`MainFrame::taken`), as
  Chromium's `PullDeltaForMainThread` snapshots the delta as it sends `BeginMainFrame`. Takes are additive, made
  once, and never block. Program work queued before the input is not run inside it.
- **I3, owed** (`compositor/scroll.rs`): displayed = base + owed, within the range. A move is clamped from where the
  container shows, so a push outward at an edge adds nothing now and nothing later; its wheel step is still told
  once. Untaken scroll is one sum per container, and pulled scroll not yet committed is one sum per container for
  each pull (ADR-0033). Storage is bounded by the containers moved and the inputs queued, never by the event count.
- **I4, commit** (`Scrolls::committed`): a commit carries its base and the number of the last take it holds
  (`Commit::taken`). The compositor drops the takes it holds and recomputes the rest on top of the new base, each run
  clamped in order. An aborted commit (a delta dropped while a full one is awaited after a panic) keeps its takes for
  the next. A jump (a new `ScrollNode::generation`, a count compared only with itself) clobbers the compositor's own
  motion only: its fling, the fling's untaken sums, and the fling in a take the UI makes after it jumped. Real input
  is never dropped. That is narrower than cc's clobber, which overrides the whole delta. State is keyed by the
  container's instance, so a remount inherits nothing. Real and synthetic motion stay separate sums through every
  transform. A range that shrinks cuts each kind by the same share (`keep`), so a surviving fling stays synthetic and
  a later jump still drops it. The jump is told from each outbox entry's own generation, not from the compositor's
  state. A compositor rebuilt after a render-thread panic (`Scrolls::with` on the outbox it hands over) therefore
  tells it the same way. Sums are keyed by identity (one hash probe a container), so a commit costs O(entries held),
  never O(N²). A delta names the mount it moved (`ScrollNode::instance`, minted by the UI's painter for each layout
  pass and never reused), and the UI takes in only a delta whose mount is still laid out. A remount at the same key
  and owner is a new mount and gets nothing of the old one's (ADR-0033).
- **I5, UI writes** (`scroll.rs`, `edit.rs`, `list/virtual_list.rs`): program-ordered assignments. A jump is an
  assignment; the UI's wheel and a rebase are additive. A virtual list's `jump_to` puts the anchor and the pending
  request in one write. The reader's scroll taken since then lands on the request rather than over it, so the
  offset and the rows go out from the same anchor in the same commit. The anchor owns the reader's scroll. Every
  list write goes through one function (`ListState::poke`), which first folds what the handle took in since it was
  last placed (`ScrollHandle::take_added`) into the anchor (`absorb`). A splice, a reorder, a measured height, a
  followed tail or the end space set between a take and the list's layout then moves content under the place the
  reader scrolled to, not under the stale one, and a rebase stays additive.
- **I6, gesture end** (`Scrolls::stop_gesture`): ending or replacing a gesture applies all of its real displacement
  that the resampling had not applied yet. `Changed(+30), Ended, MayBegin` with no tick between keeps the 30 px. What
  an edge stopped is not the gesture's to settle. When a resampled step meets an edge, the gesture records what the
  container accepted, and the stopped part of its events' motion leaves the predictor's totals
  (`ScrollPredictor::absorb`). Settling then never pulls the content back off the edge (80 to 100 rests at 100, not
  95), and a turn back moves it from the edge at once. One exception is intended: a gesture whose container a modal
  layer covers stops where it is (`tick`), and its unapplied rest is dropped. A container below a modal takes no
  scroll, just as a wheel over it takes none.
- **I7, dirty sets**: grant, sample, tick and commit work is O(containers moved or held for, plus the hit path).
  Containers are looked up by identity in the tables' index, and nothing walks every scroller. The cost test counts
  each identity visited.

Time stays where one source owns it: the OS's event times resample a gesture and measure its speed within that
gesture, and the frame clock drives flings and scrollbars. No time crosses sources.

### Consequences

- Wheel, then a click on "Top" during a 500 ms UI stall, ends at Top: the wheel was taken before the click. A wheel
  after the click moves on from Top. Both are covered by `a_million_row_list_is_ordered_by_what_the_ui_saw` and
  `scroll::tests::a_turn_before_a_key_is_taken_before_it_however_late`.
- A gesture or the platform's momentum that goes on after a jump moves on from the jump, as do a notch and a
  smooth-wheel sample the UI had not taken when it jumped. Rounds 4–6 dropped them; the reader sees the motion they
  made.
- A wheel sent before a key is applied before the key is handed on, so there is no residual race (ADR-0033). A
  recorded log is written by the render thread in the one order it took everything, and a replay reads it in
  sequence (ADR-0033). A log synthesized by hand replays on the display clock as before.

### Revisit conditions

- Usability evidence that a jump should swallow a gesture begun before it. The remedy would be a per-gesture clobber:
  the render thread drops the real motion of a gesture whose start it saw before the jump's commit.

## Evidence

The tests are in `crates/gui/src/compositor/tests.rs`, `compositor/outbox.rs`, `scroll.rs`, `interact.rs`,
`list/virtual_list.rs`, `window.rs` and `gui-display/src/macos.rs`. Each real fix's test fails with that fix
reverted:

| Fix | Test |
|---|---|
| I2 take | the million-row slice; `outbox::a_pull_has_what_was_moved_since_the_last` (ADR-0033) |
| I4 abort | the million-row slice; `a_commit_that_never_came_leaves_its_takes_to_the_next` |
| I4 clobber | `a_scroll_to_a_far_row_at_an_equal_local_offset_stops_a_fling` |
| I5 list | `a_jump_during_fast_wheels_lands_them_on_it_with_its_rows` |
| I6 | `a_gesture_ended_and_touched_again_before_a_tick_keeps_its_motion` |
| I7 | `a_gesture_held_on_the_last_of_twenty_thousand_scrollers_costs_lookups_alone`, 20,005 visits with a walk, at most 8 without |
