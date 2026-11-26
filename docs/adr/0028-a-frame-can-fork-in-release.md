# 0028. A frame can fork in release

Status: accepted, 2026-11-24; partly superseded 2026-11-25 by [ADR-0034](0034-a-check-switches-worlds-without-running-app-code.md)
(the switch back to live runs no app code; teardown faults fail-stop; payloads, the release queue, identity per world,
sealed storage and the closed check report there replace this record's rounds 11 to 13 where they conflict); F1 registry implemented; block-boundary rollback and Select alignment under verification ([M13](../plan-2026-11-21.md#m13-frame-rẽ-nhánh-trong-bản-release)), F2–F4 not yet (M13). The direction was chosen by the user on 2026-11-23
(`docs/research-2026-11-23-frontiers.md` §13, item 1: "ta cần điều tốt nhất", the full option in its final shape). The
details below were decided under the user's delegation of 2026-11-24 ("làm toàn bộ đi, chỉ dừng khi xong hết") by the
agents doing the work. Three review rounds by two model families failed. Nearly every finding of the third was about
the `Trial`, a fork that runs handlers; it is split off as ADR-0030 (proposed, M17), and this record keeps what is
settled: the fork, the check, the outbox and the registry. Replaces the debug-only cold frame of ADR-0013 and
`crates/gui/src/engine.rs`. Amends ADR-0012 and ADR-0023 and, with ADR-0026, defines the cold frame under run-once.
Written together with ADR-0026.

## Context

**What exists.** `engine.rs` can fork the UI thread's state and put it back:

- Every piece of per-thread state is an `engine_local!` with a policy: Fork (put back at the end), Fresh (the frame
  sees an empty one) or Shared (with a reason). `crates/gui/clippy.toml` bans raw `thread_local!`. Today 61 pieces are
  declared, test fixtures included: 32 Fork, 17 Fresh, 12 Shared.
- The reactive arena and the instance tree fork by an undo log, so a fork costs what the frame writes, not what the
  thread holds (`reactive/runtime.rs`: `ColdLog`; `component.rs`: `TreeLog`).
- `ColdFrame` restores every piece when it drops, on return or on unwind, and drops what the frame made while the
  frame still runs.
- A cold frame runs no effect and no cleanup, and a write to a live signal is refused and recorded (`cold_refuses`).
- The check builds its frame apart from the live one (`app.rs:708-735`): motion runs on a copy, the painter paints
  its own scene, and the result is compared and thrown away.

**What it cannot do yet.**

- **It is compiled only in debug builds.** `component.rs`, `engine.rs` and `reactive/runtime.rs` hold 22, 44 and 61
  `cfg` lines on `debug_assertions`. The release build, the one users run, only ever takes the incremental path, and
  nothing checks it there (review §6.5).
- **The registry does not list all state** (frontiers §2, verified by Codex):
  - Handles that share mutable state behind `Rc`: `ListState` (`list/virtual_list.rs:186`, `:268`, about 20 `Cell`
    and `RefCell` fields), `ScrollHandle` (`scroll.rs:60`), `Columns` (`list/columns.rs:53`). A cold layout writes
    them.
  - State held by the pipeline, not by an `engine_local!`: `Pipeline.input_state`, motion with its leaving list, the
    painter's scene.
  - Process-wide counters. A fork can advance `chunk/series.rs:717-718` (`NEXT_SERIES`, `NEXT_PAGE`). Eight more
    were found by reading the sources and are not yet classified: `scene.rs:78`, `:164`, `edit.rs:60`,
    `chunk/mod.rs:89`, `:162`, `chunk/readback.rs:79`, `canvas/mod.rs:1212`, `layout/engine/mod.rs:448`.
  - Process-wide tables: the style interner (`style.rs:984`, `CHAINS`), `layout/engine/calc.rs:27`,
    `chunk/runtime.rs:356`, `text/fonts.rs:206`, `icon.rs:249`, and `primitives/src/filter.rs:55`.
  - The layout pool's threads have `engine_local!` state of their own, which a fork on the UI thread does not enter.
- **Effects leave directly.** `Sender::send` writes its channel (`mailbox.rs:50`), so a sender captured before a fork
  reaches the world from inside it. `clipboard::copy` calls the system and answers at once; cut deletes only when it
  answered `true` (`interact.rs:3614`). `primitives/src/command.rs:668` and `gui/src/list/view.rs:65` start a thread
  per request.
- **It runs no handler.** It can check a frame but cannot show what an input would do. That is ADR-0030's problem,
  not this record's.

**Why it matters now.** Four research threads need the same base (frontiers §2): previewing an input before it is
real, stepping back through recent frames, showing an agent what an action would do, and checking in release that
incremental equals cold. ADR-0026 also changes what "cold" means, because there is no render to run again.

**Precedent.** Chromium ships `PaintUnderInvalidationChecking` as a runtime feature, not a build flag (frontiers
§6.3). No toolkit in `references/` can roll back a frame of application code.

## Options

1. **Keep the fork in debug builds.** Release stays unchecked, and none of the four uses can be built.
2. **A second mechanism for release** (snapshot and restore). Two mechanisms must be kept equal, which is the problem
   the cold check exists to remove.
3. **One mechanism in every build, whose forks may be committed.** A committed fork must be exactly a live frame:
   cleanups at commit, cancels, the order of queues, a panic half way through a commit, answers to effects. Two
   review rounds failed on these.
4. **One mechanism in every build, whose forks are always discarded.** A fork observes and is rolled back.

## Decision

Option 4, with one user of the fork decided here: the `Check`. A fork that runs handlers (a trial) is ADR-0030.

### A fork is always discarded

A fork is one transaction over every piece of engine state on the UI thread. It ends by rollback, always. There is
no commit.

```rust
// Shape, not final names.
pub struct Check { /* not Clone, not Send; Drop rolls back */ }
impl Check { pub(crate) fn begin(of: &LiveFrame) -> Option<Self>; }
```

- **A `Check` is the cold frame.** It proves that incremental equals cold, in debug builds on every frame and in
  release by sampling.
- **It works on a detached tree, as today.** The check mounts its own tree inside the fork and never writes the
  retained live tree. So no log of the retained tree is needed, and none is built.
- **It refuses a write to a live signal** and reports it, as a fault of the code checked (`cold_refuses`, kept).
- **It runs no handler, no effect, no hook, no timer and no cleanup.**
- Only one fork is open on a thread at a time, and forks do not nest (the assert in `ColdFrame::begin` stays).
- **`Check::begin` gives `None`** in two cases: a fork is already open; or the build has `panic = "abort"`. In such a
  build a panic cannot be unwound into a rollback, so the app gets no sampled check and `GUI_COLD_CHECK` does
  nothing.

**What "no handler" removes from this record.** Because a check runs no application code that is allowed to change
state, none of these is needed for it:

- undoing a signal mutation in place (`set`, `update`): a check never performs one;
- a log of structural changes to the retained tree: the check's tree is detached;
- pending work and timers carried into the fork: a check runs none, so the queues start empty (Fresh, as built);
- cleanups inside a fork, and scopes "removed and revived": a check removes no live scope;
- a clipboard overlay, answers to effects, and recorded inputs: a check neither reads nor acts.

Each of these is a problem only for a trial, and is listed in ADR-0030.

### What a check frame runs

A check runs with the frame time and the input state captured from the live frame it checks. The four points below
are stated identically in ADR-0026; the two records change together.

- **Runs again:** the source of every live control node (`when`, `items` and `key`, `which`) that is not inside a
  frozen subtree, trusting no memo; every binding of every phase of every node that is not frozen, with empty
  comparison cells; the body or view closure of a branch or row that the source expects and the live tree lacks;
  layout, paint and accessibility from empty caches.
- **Never runs:** the body of a live mount, effects, handlers, cleanups, hooks, timers.
- **Carries from the live frame:** per-node engine state (scroll offsets, motion, hover, focus) through the
  correspondence, and the leaving ledger. Leaving entries are engine state, like scroll and motion: the cold build
  replays the ledger into its tree, each entry at its recorded position with its frozen content, as `Motion::splice`
  does today.
- **Compares:** the referee's expectations with the live tree (the physical retained child order, removed owners,
  the leaving set against the ledger by key and position, the correspondence); then frame nodes, scene and
  accessibility tree.

### The transaction

- **Rollback** puts every piece back, last entered first, and runs no application code. What the fork made is
  dropped while the fork is still open, in rounds, so that what those destructors do is the fork's too
  (`ColdFrame::end`, kept as built, with its runaway check).
- **What the fork made** is the detached tree with its scopes, signals and bindings, and the subscriptions those
  bindings took on live signals. The reactive arena's log (`ColdLog`) undoes the last; the rest is dropped. No
  cleanup of a dropped scope runs.
- **The log exists only while a fork is open.** The gate has two parts, both against the same workload without the
  machinery: allocations and undo records added on an ordinary frame equal 0; and instructions per frame are not
  above B0 beyond its noise. The second part catches what the first cannot: the indirection of `ListState` as an id,
  and the fork flag read in `Sender::send`.
- **F2 turns it on in release.** The `cfg` gates on `debug_assertions` in the registry, the arena log and
  `EngineKey::registered` go. `TreeLog` and `cold_copy` belong to the instance tree of ADR-0013; K8 deletes them
  with it, and the detached mount of ADR-0026 stands in their place. F2 also takes wave C §2.7 and §2.12. It is done
  after K8, so that nothing about to be deleted is ported.

**What the boundary covers.** A fork enters all of this, and a test enumerates it:

- every `engine_local!` piece on the UI thread, by its policy;
- the reactive arena, by its log;
- `Pipeline.input_state`, read as captured; motion and the leaving lists, on a copy; the painter's scene, its own;
- focus and the layer stack, read through the correspondence and not written;
- mailbox open and close;
- `ListState`, `ScrollHandle` and `Columns`, registered as ids (rule 4 below).

**Pool threads are outside it, and a fork never reaches them.** A forked frame lays out inline on the UI thread; the
layout pool is disabled while a fork is open. So the pool threads' `engine_local!` state is not touched by a fork,
which is why their counters hold. The cost is a single-threaded layout for a forked frame (28–34 ms cold at 60
thousand nodes, ADR-0014).

### The registry

| Policy | For | In a fork | At rollback |
| --- | --- | --- | --- |
| **Fork** | State and counts the fork may read and write | Visible; the first write of each part is logged | Put back |
| **Fresh** | Queues, reports, caches and scratch | Starts empty | The fork's is dropped; the live one returns |
| **Shared** | Handles and inert tables, each with its reason | The same one | — |

**The 17 Fresh pieces stay Fresh for a check.** A check runs no pending work, so it must not see the live queues,
and what it queues is never run:

| Piece | What it is | In a check |
| --- | --- | --- |
| `MARKED`, `DEFERRED` (`component.rs:1001`, `:1003`) | Marks waiting for a flush | Empty at the start; marks a detached mount makes are dropped |
| `POST` (`component.rs:1011`) | Hooks after layout | Empty; never run |
| `LEAVING` (`component.rs:1015`) | Scopes leaving in the current render | Empty; goes with the old path at K8 |
| `FRAME_DUE`, `STRANDED` (`component.rs:1017`, `:1020`) | "Another frame is wanted", "work was stranded at the cap" | Clear at the start; a check schedules no frame and its flags are dropped |
| `RENDERED` (`component.rs:1022`) | Report of what rendered | Empty; goes with the old path at K8 |
| `BINDINGS` (`component.rs:1103`) | Count of bindings run | Starts at 0; the check's count is its own |
| `BOXES`, `BELLS` (`mailbox.rs:64`, `:65`) | Mailboxes registered and their wake bells | Empty; a mailbox made in a check is never drained and never rings |
| `ANSWERS` (`layer.rs:284`) | Dismiss handlers waiting to answer | Empty; a check runs no handler |
| `IDLE` (`layout/leaf.rs:417`) | Idle text systems | Empty: the check lays text out from fresh caches |
| `SCRATCH` (`layout/engine/grid.rs:262`, `row.rs:108`) | Layout scratch | Empty |
| `LABELS` (`canvas/mod.rs:845`) | Canvas label cache | Empty |
| `PROBES`, `SWEEP_AT` (`reactive/trace.rs:113`, `:116`) | Trace probes and their sweep mark | Empty: a check's reads are not traced into the live trace |

Rules:

1. **A Fork piece costs what the fork writes.** A plain `[Fork]` is a `Copy` cell or one handle. Anything larger keeps
   its own log (`[Fork(log)]`, `Logged`). This is the existing rule of `engine.rs`.
2. **A Shared process-wide table must not change a result.** A hit and a miss give equal results, and nothing
   observable depends on how many entries it has or in what order they came.
3. **An id that must be unique is a leased block and a per-thread Fork counter.** No process-wide counter is
   advanced per id from the UI thread, so a rolled-back fork leaves every counter where it was. F1 moves the two series
   counters and each of the other eight that the UI thread advances.
   *Built (F1), changed in review round 1:* `engine::unique_id` is the one place. A thread leases a block of 2^16
   ids (`engine::IdLeases`, `ID_BLOCKS_LEASED`) when its block runs out; its place in the block is the kind's
   `[Fork]` counter, which holds the last id it gave, so an id costs one use of engine state, as the counter before
   it did. Block numbers come in two spaces, each a monotonic atomic: the live one, and one only forks lease from.
   A fork never takes from the live space, so with the same allocator state and the same schedule the live ids
   after a check, at first use and across a block's end alike, are the ids without it
   (`engine::tests::a_check_leaves_the_live_id_sequence_as_it_was`, an allocator of its own per run); neither atomic
   goes back, so no id is handed out twice, and a fork's ids are never live ones. Thread ordinals, which an earlier
   F1 cut used, ran out after 2^20 threads that never came back; blocks are spent by ids handed out, not by threads,
   and there are 2^46 of each space below bit 63 (`ChunkId` marks pushed chunks with bit 63). Block 0 is never
   leased, so no id is 0.
   *Changed in review round 2:* a fork that began in the middle of a live block took the rest of that block, and the
   rollback put the counter back, so the live run handed the same ids out again. Identity is now never minted from
   state a rollback restores, as a database keeps its sequences out of the undo log (PostgreSQL's `nextval` is never
   rolled back; ARIES's log sequence numbers only grow): inside a fork every id comes from a fork block, the first
   one leased at the fork's first id however far the live block got, and the live counter is left where it was
   (`engine::tests::ids_lease_blocks_and_never_run_out`, `ids_of_two_threads_never_meet`). A value a check keeps
   past its rollback (`cold_scene`, the cold frame's output) holds fork ids, so it is compared with the live one by
   content, never by id (`a_cold_frame_leaves_the_content_ids_as_they_were`). Other identity follows the same rule:
   a list's loan of its heights is named by such an id (`virtual_list::LOANS`), not a count the list keeps (below).
   *Changed in review round 3:* identity a caller can hold is minted from the non-rollback source, every kind of it:
   `TimerId`, `Registration`, `FocusHandle` (and the window salt its path ids hash), `NodeRef`, `LayerHandle`,
   `CaptureGroup`, a toast's and a toaster's ids, a tooltip's, a drawer's and its openings, a hover grace's, a menubar
   menu's name, a frame's epoch and a `Views`' holder id all go through `engine::unique_id`
   (`clock::tests::a_timer_id_a_fork_kept_cancels_no_live_timer`). A `[Fork]` `Cell<u64>` advanced by hand fails the
   lint unless listed as no identity, with why (`COUNTS`: the counts the tests read of index visits, text compares,
   text copies and the scroll containers the compositor's passes visit;
   `scroll::tests::a_fork_leaves_the_live_generations_as_they_were`). *Changed 2026-11-25 (ADR-0032):* the frame
   clock's hybrid-logical-clock watermark `LOGICAL` this list held is gone with the scroll stamps it ordered; scroll
   is ordered by what the UI has taken, not by time;
   nothing fork-made that
   outlives the rollback is compared but by content (`cold_scene`).
   *Changed in review round 4:* round 3 still kept each kind's cursor in a `[Fork]` cell of its module, which
   `unique_id` trusted. Two failures followed from that one place. Counters seeded at 1 looked already leased, so
   every thread's first id came from block 0 and two threads' first timers shared an id. A Shared piece made inside a
   check (its init runs suspended, live) took a live id, and the rollback put the cursor back under it. The model
   applied in full now is the database sequence's: PostgreSQL's `nextval` is never rolled back, and ARIES's log
   sequence numbers only grow. An identity source is therefore not transactional state at all. `unique_id` takes a
   static descriptor (`engine::IdKind`, a name only, declared beside what its ids name) and owns the cursors itself:
   one live and one fork cursor per kind, thread and allocator, in a `[Shared]` map no rollback touches. A cursor
   starts at none, so a thread's first id leases a block of its own. Modules hold no id cells. The lint fails on any
   `[Fork]` `Cell<u64>` not listed in `COUNTS`, handed to `unique_id` or not. Tests:
   `engine::tests::the_first_ids_of_two_threads_differ` (timer, focus handle and node ref) and
   `a_shared_init_in_a_check_keeps_an_id_no_live_handle_gets_again`. Each fails when the cursor is seeded at 1 or kept
   as forked state.

   The reactive arena follows `ForkSlab`'s slot-generation rule now. Round 3 marked a cold-made node with a top
   generation bit that carried no identity across rollbacks. Two checks making a node in the same slot handed out the
   same generation, so a signal kept from the first was alive in the second. A slot a rollback freed also kept the bit
   into live use. Each slot now keeps its next generation in `Runtime::next_generation`, which is never logged or
   truncated. It only grows, a rollback included, so whatever a frame handed out names nothing after it, in a live
   run or another frame, and no bit tells live from fork. A slot whose generations ran out is retired. Tests:
   `a_signal_kept_from_one_check_is_dead_in_the_next` and `a_kept_fork_handle_names_nothing_over_two_checks`, the
   second with a Shared node made above the frame's nodes. Both fail with a fixed fork generation.
   *Changed in review round 5: one slot rule for every arena, and one answer to "is this the fork's".* Round 4 gave the
   reactive arena one monotone counter per slot. A check then advanced the live counters too, so live code made
   after a check got other ids than without it (`app::tests::the_first_cold_check_on_a_thread_takes_no_signal_id_from_the_live_frames`
   failed). The keyed memos (`holder::Keyed`) and the component instances kept rules of their own, and `ForkSlab`
   treated an entry made while a fork was suspended (a Shared piece's init) as the fork's, so the rollback emptied
   it (Codex: a Shared init that runs `Columns::new` in a check left columns that were dead after it). The rule is
   now one type, `engine::Generations`, used by the reactive arena, every `ForkSlab` (lists, scroll handles,
   columns, and the keyed memos, now the `[Fork(log)]` slab `holder::MEMOS`) and the component tree. Each slot keeps
   two next generations, one for what live code makes and one, under the top bit (`FORK_GENERATION`), for what forks
   make. These are the id blocks' two spaces, applied per slot. Neither is ever logged, put back or cut off with the
   slots a rollback takes away, by the same sequence model as the id cursors. Live code therefore hands out exactly
   the ids it would have without the check, and a handle a fork handed out names nothing after it, live or in a
   later fork. A slot whose generations ran out in either space is retired. Which space an allocation takes has one
   answer, `engine::forking()` at the moment it is made. An entry made while the fork is suspended is live: the slab
   records it (`kept`), keeps it through the rollback and logs its later writes. Tests:
   `engine::tests::a_forks_slab_ids_never_alias_a_later_entry` (now also: the live ids equal a run without the fork),
   `a_slab_entry_made_while_a_fork_is_suspended_is_live_and_kept`,
   `columns::tests::columns_a_shared_init_makes_in_a_check_stay_alive_after_it`,
   `holder::tests::a_keyed_a_check_made_names_nothing_after_it`, `a_keyed_made_live_in_a_check_survives_it`, and
   the app test above.
   *Changed in review round 6: a suspended fork is as if there were none.* Round 5 still kept one free list and one
   slot sequence for both. An entry a fork made took a live slot number. A slot a suspended removal freed went into
   a free list the rollback put back, and an entry made while the fork was suspended was told apart by a list
   (`kept`). Live code after a check could therefore get other slot numbers than without it. A suspended write to a
   node the fork had already logged was also undone by the rollback (Codex 4: `owner.dispose()` after a rollback
   left a memo's entries). The fix follows the sequence model again: what the live world numbers must not depend on
   whether a transaction ran beside it, as PostgreSQL's `nextval` and ARIES's log sequence numbers are never rolled
   back. A fork now makes its entries in a **fork-only region** of each arena, one whose numbers carry a top bit
   (`engine::FORK_SLOT` for `ForkSlab`, `FORK_NODE` for the reactive arena, `FORK_INSTANCE` for the component tree),
   with its own slots and generations. The live slots, free list and generations are never touched by a fork. A
   fork's first write to a live entry copies it into a shadow the fork alone reads (copy on write). The rollback
   drops the region and the shadows, and nothing in the live arena needs putting back. A suspended fork
   (`engine::live`) is not forking: what runs then allocates, writes and frees in the live arena with the live free
   list, and nothing of it is logged. When the fork had already logged a node, the reactive arena swaps the live
   state back in for the suspended code and sets the fork's state aside until it resumes, so the suspended write
   lands on the live node and stands after the rollback. Tests: `engine::tests::a_suspended_insert_takes_the_id_a_run_with_no_check_gives`,
   `a_suspended_signal_takes_the_key_a_run_with_no_check_gives` (each compares slot numbers with a run on another
   thread with no check), `a_suspended_write_or_remove_of_a_live_entry_stands` (a freed slot is reused by the next
   live insert), and `holder::tests::a_keyed_made_live_in_a_check_survives_it` (`owner.dispose()` after the rollback
   frees the memo, with and without the fork having written the owner). Each fails when suspension is treated as
   forking or the swap is left out.

   *Changed in review round 6: a memo's fork writes cost what they change.* A keyed memo's map was saved whole on a
   fork's first write, O(keys). It is now a persistent layering (`holder::Layered`): a shared base map behind `Rc`
   and a small layer of the writes since (up to 64), folded into a new base when full. Saving it is two `Rc`
   clones, and a fork's writes go into its own copy of the layer, as Driscoll et al. (1989) copy only the path a
   write walks. Test: `holder::tests::a_forks_insert_into_a_memo_of_a_million_keys_copies_none_of_them` (under
   64 KiB allocated; 69 MB with the base copied).

   *Changed in review round 8: a fork never runs live code; suspension is gone.* Rounds 5 to 7 let a Shared piece
   first touched inside a fork be made live there, by suspending the fork around its init (`engine::live`). Round 7
   made the suspension one engine-owned depth with a `suspend` and `resume` per piece. Each round still found
   suspension bugs. In round 7 they were: a panic in the middle of a swap lost the undo (Codex 1); suspended code
   resolved handles that only the fork had (Codex 2); a resume hid a slot that a Shared init had reused (Codex 3);
   a torn view after resume, link reuse, a re-entrant swap (`BorrowMutError`, or a `try_borrow` silently skipped)
   and registration during resume (Opus nits 1 to 3); and the swap's cost on every call (Opus I2). They have one
   cause. Live code running while a transaction is open must either see the transaction's writes or be shown the
   committed state by swapping every piece. The first breaks isolation, and the second is the error-prone machinery
   these rounds kept patching. Under snapshot isolation the transaction keeps reading its own snapshot, and
   committed state is not written while it is open (Berenson et al. 1995). A fork is such a transaction that never
   commits, so nothing of it may be live work.
   - **First touch gives a stand-in.** A Shared piece first touched while a fork runs is not made. The fork gets a
     *stand-in* of its own (`EngineKey::stand_in`), made by the same `fn() -> T` init inside the fork, so what the
     init makes lands in the fork-only regions. It is the same stand-in each time the fork touches it again, and it
     is dropped with the fork's drains. *Round 9 replaced what round 8 did next* (doom the fork, make the piece
     live after the rollback, run the fork again); see "Changed in review round 9" below.
   - **`InFork::Refuse` dooms the fork** (`Doom::Dropped`, `engine::drop_fork`). The fork runs on with the inert
     answer and its result is dropped. A check that sampled it skips the sample, because running it again reaches
     the same site.
   - **Deleted:** `engine::live`, `Suspension` and its depth, `Entry::suspend` and `resume`, `Logged::suspend` and
     `resume`, `swap_entered`, and every hand-written swap: `ForkMap::swap_logged`, the timers', the component
     tree's (`TreeLog::swap`), the reactive runtime's (`swap_logged`, `Aside::swap`), `interact::Handles`' and the
     Fresh and Fork slot swaps. Registration while suspended and the tests of states that can no longer happen are
     gone too. The custom policy is three functions again.
   - **Registry pieces are not Shared.** `WHEN_LIVE` is the registry's own (as `REGISTRY` is): it carries what the
     rollback leaves to do, so a stand-in for it would drop the releases.

   The explicit callers are reworked so that no per-use live call remains:
   - `TooltipProvider::window` is a Shared piece with a fixed init. Its options live in a `[Fork]` cell (round 9).
   - `value::source::PROBE`'s init is plain.
   - The holder test uses a Shared memo.

   Kept from round 7: the fork-only link region (`FORK_LINK`), which keeps the live links, their free list and
   their numbering as without the fork.

   The round 5 and 6 text above about entries "made while the fork is suspended" describes that mechanism and no
   longer holds. Its sequence-model rule, that live code hands out the ids it would have without the fork, now
   holds because no live code runs inside one.

   *Changed in review round 9: no rerun; what a fork makes is the fork's.* **This rewrites text added in review
   rounds 6 to 8.** That text was never part of the accepted ADR (accepted at 3de98e2, before those rounds), so it
   is rewritten here rather than superseded by a new ADR. Round 8 doomed a fork that touched a Shared piece first
   (`Doom::Rerun`), made each such piece live after the rollback (a `due` list, each touched under its own catch),
   and ran the fork again, capped at the thread's slot count. Review 9 found six bugs in it, each with a PoC that
   ran and failed (now regression tests in `crates/gui/tests/fork_doom.rs`):
   - a destructor in the drain doomed the fork after its verdict was read, so the result was dropped live
     (Codex 1);
   - a `[Fork]` piece first made in the fork kept a value copied from a stand-in, which the rollback put back
     (Codex 2);
   - a panic while entering left a fork open forever (Codex 3);
   - pieces were made live in completion order, which reversed the id order of a run with no check (Codex 4);
   - a stand-in panic that the fork caught left it undoomed (Codex 5);
   - a nested init that failed live ran twice (Codex 6).

   Four of them (1, 4, 5 and 6) are ordering bugs in the second write path to live state that "made live at
   rollback" is. Before fixing them, F1 ran a research phase on how other systems handle what a discardable run
   initialises (`/tmp/m10/research-f1.md`, with citations):
   - **What a discarded run makes is that run's, deleted with it.** React does not preserve state for renders
     suspended before they mounted (react.dev, Suspense caveats). Vue unmounts a restarted branch and discards its
     effects (`references/vuejs/packages/runtime-core/src/components/Suspense.ts:370-384, 1095-1103`). Leptos
     cleans up what an owner made (`references/leptos/reactive_graph/src/owner.rs:285-299`). PostgreSQL deletes a
     relation made in an aborted transaction (`storage.c`). Harris et al. 2005 (§3.5) keep only the allocation of
     what an aborted transaction made, never its writes.
   - **Lazy singletons are never rolled back, and their init is pure.** A GHC top-level thunk has the same value
     whichever transaction forces it first. Effectful globals are made outside any transaction (`newTVarIO`).
     React initialises at module level. The one Dioxus thread-local singleton has a context-free init
     (`references/dioxus/packages/vdom/src/nodes.rs:190-200`).
   - **No system reruns a run because something was initialised in it.** The one rerun in the sources is React's
     store-tearing check (`ReactFiberWorkLoop.js:1170-1195`), for inconsistent reads of outside state. A stand-in
     is consistent with its fork.
   - **Compose has our Codex 2 unsolved.** An object made in a discarded snapshot that escapes it keeps that
     snapshot's record
     (`references/androidx/compose/runtime/runtime/src/commonMain/kotlin/androidx/compose/runtime/snapshots/Snapshot.kt:1056-1077`,
     with `:2362-2379`).
   - **Ids are not transactional**: PostgreSQL `nextval`, Compose's `nextSnapshotId`, React's client `useId`.

   The decision follows from these:
   - **No doom, no rerun.** A stand-in lives and dies with its fork, and the fork's result stands. Live code makes
     the piece at its own first touch, in its own order, so ids, keys and order equal a run with no check by
     construction.
   - **What is deleted:**
     - `Doom::Rerun`, the `due` list and making pieces live at rollback;
     - the rerun loops in `in_cold_frame` and `Pipeline::check_cold`, and their cap;
     - "a failing init is raised once".
   - **A Shared init reads no state a fork changes**, neither `forking()` nor any registered piece other than the
     reactive arena it allocates in. Its value is then the same in a fork and live (a pure init), so a stand-in is
     what live code would make.
     - The read panics in debug, naming the piece (`engine::no_forked_read`).
     - The `process_state` lint `mutable_shared` fails on a Shared piece that holds a cell. A Shared piece is
       immutable once made, and written state is `[Fork]`.
     - Five pieces changed policy: `BY_CONTENT` is now `[Fork]`; `number_format::RESOLVED`, `context::REPORTED`,
       `context::MISSING` and `progress::REPORTED` are now `[Fresh]`, and their reports are made live only.
     - Four are listed with reasons in `SHARED_CELLS`: `ID_CURSORS`, `HUBS`, and the test counters `HANDED` and
       `VISITS`.
   - **The undo of a piece first made in a fork is "never made"** (the undo of a created row is its deletion).
     - This applies when the piece's init read engine state (`gui_tls::noting`). It is then marked `UNMADE` and
       made again by its init where it is next touched.
     - A constant init's value is what a live init makes, so it stands, and the arena's generations (a sequence)
       are not reset.
   - **The verdict is read once the drain is done** (`ColdFrame::settle`). A dropped fork's result is dropped
     inside the fork (`ColdFrame::keep`). A cold build that panics raises its panic even when the sample is
     dropped.
   - **`ColdFrame::begin` makes its guard before entering any piece.** A panic while entering rolls back what was
     entered.
   - **Inits that reach their own piece.** An init that reaches its own piece, live or making a stand-in, panics
     naming it (`gui_tls::initing`), rather than overflowing the stack.

   Tests:
   - `fork_doom::{late_doom, cached_stand_in, failed_enter, nested_order}`: Codex 1 to 4.
   - `caught_stand_in_panic` and `nested_failure`: Codex 5 and 6, rewritten as tests that the forked read is
     refused.
   - `engine::tests::a_shared_piece_a_check_touches_first_is_a_stand_in_and_made_at_the_first_live_touch`: slot keys
     and id steps equal a run with no check.
   - `engine::tests::nested_first_touches_in_a_fork_are_stand_ins_and_made_live_later`.
   - `engine::tests::a_shared_init_reading_forked_state_panics_naming_the_piece`.
   - `engine::tests::a_shared_init_reaching_itself_panics_cleanly`.
   - `engine::tests::a_fork_dispatching_a_consumed_flag_is_dropped`.
   - `engine::tests::a_shared_init_in_a_check_keeps_an_id_no_live_handle_gets_again`.
   - `app::tests::a_dropped_cold_check_keeps_its_build_panic`.
   - `columns::tests::columns_a_shared_init_makes_in_a_check_stay_alive_after_it`.
   - `holder::tests::a_shared_keyed_a_check_asks_for_first_is_a_stand_in_and_made_live_later`.
   - `tooltip::tests::{first_live_caller_sets_window_options, the_windows_provider_made_first_in_a_cold_frame_is_a_stand_in, a_provider_first_made_inside_a_shared_init_in_a_check_is_a_stand_in}`
     (Codex 7).
   - `process_state::{the_lint_catches_cells_in_shared_pieces, the_lint_catches_counters_in_logged_state}`.

   Consequences:
   - **F2 and M17 (ADR-0030).** These no longer need rerunnable forks. Round 8's consequence, that a release fork
     and a trial be rerunnable from captured input, is withdrawn.
   - **F4.** Sampled checks drop a `Dropped` sample rather than compare it.
   - **A handle out of a kept fork.** A Shared piece's handle that a fork returns is its stand-in's and names
     nothing live, like any handle a fork makes.
   - **Repeated stand-ins.** A piece that no live code has touched yet gets a new stand-in in each fork that
     touches it, bounded by the pieces a fork touches.
   - **A pure init's value.** It is the same at a fork's first touch and at live's, because neither reads forked
     state. Opus's round-9 concern that rollback-time init could differ from first-live-touch init no longer
     arises, as nothing is made at rollback.
   - **Release.** The fork path stays debug-only until F2 (after K8). Its release tests are gated with the same
     `cfg` as the API, so `cargo test -p gui --release --lib` builds and runs.

   *Changed in review round 10: the fork alone owns the life of what it makes.* This again rewrites round-9 text
   that was not in the accepted ADR. Two round-9 mechanisms are replaced: "marked `UNMADE` and made again", and
   `ColdFrame::keep`/`finish`. Review 10 ran five PoCs that failed, now regression tests in
   `crates/gui/tests/fork_doom.rs` (`codex_r10::*`, `opus_r10::*`):
   - **Codex 1 (`drop_side`).** A piece first made in the fork kept its fork-made value in its undo record, and the
     rollback put it back. The next live touch remade the piece and dropped that value live, so its destructor
     wrote live state (19 vs 0).
   - **Codex 2 (`retry_remake`).** A remade piece was marked made before its init ran. When the live init
     panicked, the next read returned the discarded fork value (9 vs 2).
   - **Codex 3 (`shared_signal_read`).** The read guard exempted the whole reactive arena, so a Shared init read a
     signal the fork had written, and the stand-in's value (7) stood for a live one (0).
   - **Codex 4 (`failed_drain_result`).** A panic in the drain did not stop the result being kept. The rollback
     then unwound, dropping the result live (19 vs 0).
   - **Opus I1 (`one_stand_in_per_fork_through_its_drain`).** The drain dropped the stand-ins first. A destructor
     later in the same drain reached the piece again and made a second stand-in (inits 2, a mismatch 1).

   They share one cause: no single place owned a value's life in a fork. The undo record, the slot's state byte,
   the drain's order and the panic handling each decided part of it. The fix makes `ColdFrame` the one owner, by
   four rules:
   - **R1. Everything a fork makes dies inside the fork.** Before any piece is put back, the drain drops, while the
     fork is still open:
     1. the result, unless it is kept;
     2. the values of the pieces the fork made from engine state, last made first;
     3. what the fork wrote in each entered piece, last entered first;
     4. once a round touches no engine state, the stand-ins, last made first, each one reachable until its own
        drop.

     It repeats until a round makes nothing new. A stand-in reached again in the same fork after it was dropped
     panics: there is one stand-in per piece per fork. A piece the fork made by an init that read engine state is
     entered by nothing, so no undo record holds its value. The drain moves the value out and drops it, and the
     piece is left truly unset: in debug builds `Held` keeps its value in a `RefCell<Option<T>>`, `None` while
     unset. This is ARIES's rule: the undo of an insert is a delete, and the compensation log record carries no
     after-image of the rolled-back row (Mohan et al. 1992: CLRs are redo-only). A constant init's value stands
     as before. It is entered, and the arena's generations are not reset.
   - **R2. Any panic while the fork is open means the result is not kept.** This covers a panic in the body and
     one in the drain. The result is dropped inside the fork, before the rollback; then the first panic goes on,
     the body's before a drop's. This is `ColdFrame::close(body: thread::Result<R>) -> Option<R>`, the one way to
     end a fork with a result. `in_cold_frame` and `Pipeline::check_cold` both use it. A dropped sample still
     raises its build's panic, which closes Opus's `check_cold` nit.
   - **R3. A slot holds a value only once its init returns.** This is `std::cell::OnceCell`'s rule: an init that
     panics leaves the cell uninitialised, and the next `get_or_init` runs it again. An unset piece is made in
     `EngineKey::registered` by its own `init` (the per-macro `Entry::remake` is deleted). It is marked made only
     after the init returns. A panic leaves it unset, so the next touch runs the init again. `LazyCell` poisons
     instead; poisoning would turn one failed live init into a failed piece for the life of the thread, which a
     retry avoids.
   - **R4. A Shared init reads no engine state, reactive values included.** While a Shared init runs, live or in a
     fork, a read or write of a signal's or a computed's value panics in debug, naming the piece
     (`engine::no_reactive_read`, called by `Signal::slot` and `Computed::fresh`). Making a handle stays allowed:
     the arena's own bookkeeping goes through `EngineKey::with_arena` (`reactive::runtime::with`, the labels'
     `register_probe`), and so does the engine entering a piece that the init first touched in a fork. The
     round-9 slot exemption (`reactive::is_arena`, `trace::is_labels`) is deleted. `TooltipProvider` and the theme
     defaults in `primitives` and `shadcn` pass, as they only make handles.

   Research analogy, qualified (Codex nit): Vue's `discardEffects` drops a discarded branch's pending effects, but
   it requeues those flagged `REQUEUE_ON_SUSPENSE_DISCARD`
   (`references/vuejs/packages/runtime-core/src/components/Suspense.ts:1095-1103`). So "a discarded run's effects
   are dropped" holds for Vue only for the effects that are not flagged. A fork has no such flag: rule 5
   (`call_out`, `when_live`) is its one way to reach live state.

   Also in this round:
   - **The Shared-cell lint follows types.** `mutable_shared` resolves a piece's type through the aliases and
     structs it names, in any file read (`Types::holds_any`), for `Cell`, `RefCell`, `OnceCell`, `Mutex`, `RwLock`
     and atomics (Codex plausible, Opus P1). `value::source::HANDLE_WAITERS: Rc<Waiters>` holds a `RefCell` inside
     `Waiters`. It is listed in `SHARED_CELLS` with why: a cold build registers nowhere (`source::watch` returns
     first when `cold()`), and a fork never holds a live mount whose `Wait` it could drop (a cold build's
     `BoundRun`s are its own tree's; the live tree is lent by `&`).
   - **The counter lint closes the cheap gaps** (Opus nit). It now catches:
     - steps that are not literals (`x.f = x.f + step`);
     - counting down (`-=`, `wrapping_sub`);
     - `Cell<Option<u64>>`;
     - `NonZero*`.

     It flags only a field that is stored somewhere, so arithmetic on a read never stored back is let be. The
     new checks found `clock::FRAME_TIME` and `hover::ON_TRIGGER`; both are listed in `COUNTS` as no identity.
     Not followed: an alias of an integer type (`type Id = u64`), and a step taken in a helper
     (`x.f = next(x.f)`). Both are read by review.
   - **A `ForkSlab`'s generations after an unset.** An unset `ForkSlab` piece is made again from its init, which
     restarts its generations. Every `ForkSlab` init today is constant (`RefCell::default()`), and a constant
     init's piece is never unset, so this cannot happen now. A future slab whose init reads engine state must keep
     its generation counter outside the piece.
   - **Release.** `holder::tests::a_registry_holds_the_mounted_parts_entries_alone` gates only its cold-frame
     block, so its live assertions run in release. `reactive::rendering` scopes its `Marked` guard in a block,
     rather than calling `drop` on a type that has a `Drop` only in debug (a release clippy error since a3478f99).

   Tests:
   - `fork_doom::codex_r10::{drop_side, retry_remake, shared_signal_read, failed_drain_result}`.
   - `fork_doom::opus_r10::one_stand_in_per_fork_through_its_drain`.
   - `process_state::{the_lint_catches_cells_in_shared_pieces, the_lint_catches_counters_in_logged_state}`, with
     the new fixtures.

   *Changed in review round 11: one teardown, scoped by a closure, R4 where state is read.* Review 11 ran seven
   PoCs that failed. They are now regression tests in `crates/gui/tests/fork_doom.rs` (`codex_r11::*`,
   `opus_r11::*`):
   - **Codex 1 (`drain_error`).** The Fresh drain replaced an undo record with `()` and then ran the piece's init,
     which may panic. The panic skipped the restore, so the next restore found a record of the wrong type, and the
     drain's panic hid the body's. Observed `("drain failed", 0)`; expected `("body failed", 42)`.
   - **Codex 2 (`borrowed_at_close`, `borrowed_stand_in`).** `Root::cold()` returned a guard that could be dropped
     inside a use of a piece. A borrow of a piece the fork made blocked its unset, so the fork's value survived
     (9 vs 1). A stand-in's `Rc` held by a `with` outlived the fork, and its destructor wrote live state (88 vs 1).
   - **Codex 3 (`result_before_stand_in`).** After a queued destructor panicked, the stand-ins were torn down
     before the result was dropped, so the result's destructor could not reach its stand-in.
   - **Codex 4 (`liveness`).** R4 guarded only a signal's value. A Shared init read a handle's liveness, which the
     fork had changed (`Ok(false)` vs live `true`).
   - **Opus 1 (`result_not_kept_dropped_after_stand_ins`).** The same ordering as Codex 3, seen from a `[Fresh]`
     queue that panics as it drops.
   - **Opus 2 (`a_shared_init_reading_cold_render_panics`).** A Shared init read `component::is_cold_render`,
     whose answer differs in a fork and live.

   The cause they share: user code (a destructor, an init) ran between steps of the teardown that were not
   atomic. Undo records were rewritten in place, the result's fate was decided after the stand-ins went, and a
   guard let the fork end at any point in the caller's code. Round 10's rules R1 to R3 stand. Their mechanism is
   replaced by one teardown sequence, `ColdFrame::teardown`, which `close` and `Drop` both call, once:
   - **T0. Undo records are untouched until T4.** A record is an `Rc<Box<dyn Any>>`. The drain reads it in place
     through a clone and never replaces it.
   - **T1. Drain.** Every value the fork made or wrote is moved out with no user code running: an unset piece's
     value taken from its `Held` (state `DRAINED`), a written piece's value swapped for a copy of its record. Then
     they are dropped, last made first, each under its own `catch_unwind`. This repeats until a round moves
     nothing, bounded by `DRAIN_ROUNDS`, past which the rest is leaked and reported. No init runs in the drain. A
     `[Fresh]` piece the drain emptied is made again from its init on its next touch, which is outside the drain:
     a destructor's touch in a later round, or live code after T4.
   - **T2. Decide the result.** It is kept only if the verdict is `Kept` and nothing has panicked. Otherwise it is
     dropped now, under catch, while the stand-ins are alive, and then T1 runs again.
   - **T3. Stand-ins.** Each is dropped, last made first, under its own catch, with T1 after each. A stand-in
     reached after its drop still panics: there is one per piece per fork.
   - **T4. Restore.** Each record is moved back by `Rc::try_unwrap` and the entry's `exit`, a move that runs no
     user code. The pieces the fork made are marked unset, and the fork is closed.
   - **T5. Resume the first panic**, the body's before any other.

   `ForkMap` drains through the same T1, by its entry's `Logged::drain`.

   Sources:
   - **ARIES** (Mohan et al. 1992). Undo is applied from the log, and each step writes a compensation log record,
     so a rollback is never interleaved with arbitrary code and can resume after a failure. T0 and T4 are that
     rule: a record is read during the drain but changed only by the final restore, which cannot fail part way.
   - **Rust's standard collections.** `VecDeque`'s `Drop` (its `Dropper` guard, `alloc/src/collections/vec_deque/
     mod.rs`), `vec::Drain`'s and `btree::IntoIter`'s `DropGuard`s, and hashbrown's table drop each keep dropping
     the remaining elements when one element's destructor panics. T1 and T3 do the same with one `catch_unwind`
     per drop. A fork cannot abort on a second panic as std does, so each panic is caught, and the first one is
     kept for T5.
   - **Closure scoping**, like `std::thread::scope` (`std/src/thread/scoped.rs`). Nothing borrowed from the scope
     outlives it, because the scope is a closure, not a guard. `Root::cold(|frame| ..)` and
     `engine::in_cold_frame(f)` are the public entry points. `Root::cold()`'s guard is gone. The frame is lent to
     the closure by reference, and a `compile_fail` doctest on `Root::cold` shows it cannot leave. A `with` begun
     inside the fork ends inside it, so no use of a piece or stand-in spans the fork's end. A debug assert stays:
     the drain panics if it finds a piece still borrowed. Round 10's "value stays (logged)" path is deleted.
     `ColdFrame` keeps a `Drop` for the crate's own tests and for unwinding. `Pipeline::check_cold` runs its build
     inside the closure.

   **R4 is checked where state is read.**
   - Every read of a reactive node goes through `Runtime::slot`: its liveness, value, links and any other field.
     While a Shared init runs, a read there panics, naming the piece.
   - `reactive::is_cold`, and so `component::is_cold_render`, panics the same way, as does `forking()`.
   - The engine's own bookkeeping takes an unchecked path, `engine::allocating`, as does the engine entering a
     piece:
     - making a handle (`Signal`, `Computed`, `Effect`, `Scope`, a render node);
     - labelling it (`register_probe`);
     - undoing what a failed Shared init made;
     - `Keyed::new`'s choice to register a cleanup.
   - Scheduling (`first_run_or_defer`, `settle`) reads the unchecked `engine::is_cold`.
   - `unique_id` reads it unchecked as well. Its answer differs in a fork and live, which is fine: a stand-in's
     id is a fork id and dies with the stand-in, and the live piece takes a live id when live code first makes
     it.

   Tests:
   - `fork_doom::codex_r11::{drain_error, borrowed_at_close, borrowed_stand_in, result_before_stand_in, liveness}`.
     The two `borrowed_*` tests now run the same scenarios through the closure, which cannot express the bug.
   - `fork_doom::opus_r11::{result_not_kept_dropped_after_stand_ins, a_shared_init_reading_cold_render_panics}`.
   - The `compile_fail` doctest on `Root::cold`.

   *Changed in review round 12: entering and leaving a fork run no user code.* Review 12 ran six PoCs that failed.
   They are now regression tests in `crates/gui/tests/fork_doom.rs` (`codex_r12::*`, `opus_r12::*`):
   - **Codex 1 (`outer_borrow`).** A fork opened inside `EngineKey::with` of a `[Fresh]` piece. The teardown could
     neither take the fork's value out nor move the live one back, and the live value was lost (9 vs 42).
   - **Codex 2 (`late_failure`).** The last stand-in's drop queued a value whose drop panicked. T3 checked for a
     panic before draining that queue, so the result was returned with the panic and dropped live (88 vs 1).
   - **Codex 3 (`early_init`).** Entry ran each `[Fresh]` piece's init, one piece at a time. An init wrote a
     `[Fork]` piece not yet snapshotted, so an empty fork changed live state (2 vs 1).
   - **Codex 4 (`map_keys`).** `ForkMap` cloned keys into its log. The drain took values only, and the rollback
     dropped the key clones after the fork had closed, where `call_out` runs live (2 calls vs 0).
   - **Codex 5 (`aggregate_read`) and Opus 1 (`aggregates_read_by_a_shared_init_in_a_fork_panic`).**
     `reactive::graph()`, `cleanups()` and `held_bytes()` read the runtime without the node check, so a Shared init
     saw fork-only counts.
   - **Codex, plausible.** T3 read the verdict only before the first stand-in, so a destructor that doomed the
     fork during T3 could have its result kept.
   - **Codex nit.** The module doc of `engine.rs` gave the old order: restore before dropping. It is fixed.

   The principle they all violate is the one every reference system holds: entering and leaving a transaction run
   no user code and move only data. User code (inits, destructors, callbacks) runs only inside the body.
   - **Compose's snapshots.** Taking a snapshot records an id and an invalid set. A state object gets a record in
     it only on its first write in it (`writableRecord`, `newWritableRecord` in
     `references/androidx/compose/runtime/runtime/src/commonMain/kotlin/androidx/compose/runtime/snapshots/Snapshot.kt`).
     Disposing one marks its id. Neither runs code of the state's owner.
   - **Database transactions.** `BEGIN` takes a snapshot of ids, and abort applies the undo log (ARIES, round 11).
     Neither runs a trigger.
   - **STM** (Harris et al. 2005). A transaction's log is private until commit. An abort discards it, and its
     effects are not seen outside.

   Round 11's T0–T5 become three rules:
   - **E1. Entry is pure data and all-or-nothing.** `ColdFrame::begin` first asks every registered piece whether a
     use of it is under way (`Entry::free`, a `try_borrow_mut` of its `Held`). If one is, the fork is refused with
     a panic that names the piece, and nothing has changed. Then each piece is entered by moves:
     - a `[Fresh]` piece's live value is moved out whole (`EngineKey::drained`) and the piece is left `DRAINED`;
       its init runs where the body first touches it, inside the body (Compose's record made on first write);
     - a `[Fork]` piece's value is copied (a `Copy` value or one handle);
     - a log begins.

     `fresh_enter` and `Slot::into_value` are deleted. A `[Fresh]` piece's exit moves the whole value back
     (`EngineKey::restore_whole`).
   - **E2. Exit has one drain loop**, inside the fork, with the checks inside it. Each round:
     1. If a panic was caught or the verdict, read again now, is not `Kept`, the result is dropped under a catch.
     2. Everything the fork owns that is outstanding is moved out with no user code and dropped, each under its
        own catch. This includes the values it made and wrote, and a `ForkMap`'s entries, key clones included:
        its drain now moves out the keys and values it wrote, the keys it added and the keys it noted. Only the
        live entries to put back stay, and the rollback moves them into the map.
     3. If that round's drops used engine state, or a result is left to drop, the loop goes on.
     4. Otherwise one stand-in is dropped, last made first, and the loop goes on. When no stand-in is left, the
        loop ends.

     After `DRAIN_ROUNDS` busy rounds in a row, what is left is leaked as before. After the loop no step runs
     user code: the records are moved back, the fork is closed, and the first panic goes on, the body's before any
     other.
   - **E3. R4 has one gate, `reactive::runtime::with` (and `try_with`).** Every access to the runtime struct goes
     through it, and it refuses a Shared init: a node, its liveness or value, a count, the graph, a trace. Round
     11's checks in `Runtime::slot`, `Signal::slot` and `Computed::fresh` are deleted. The exempt path is still
     `engine::allocating`, the engine's own bookkeeping, which gives the init nothing but a handle. It covers
     creating a handle, labelling it, registering a handle's cleanup with its owner (`owned_by_scope`, `Keyed::new`),
     the owner and reader scoping of `detached` and `untracked` (`OwnerGuard`), and the steps `shared_init` takes
     around the init. `Keyed::new` reads the unchecked `engine::is_cold`, which now has a release half (`false`).
     `reactive::is_cold` and `forking()` keep their own checks: whether a fork runs is not runtime state.

   Tests:
   - `fork_doom::codex_r12::{outer_borrow, late_failure, early_init, map_keys, aggregate_read}`.
   - `fork_doom::opus_r12::aggregates_read_by_a_shared_init_in_a_fork_panic`.

   *Changed in review round 13: the exit rule made exact.* Codex ran four PoCs that failed. They are now
   regression tests in `crates/gui/tests/fork_doom.rs` (`codex_r13::*`), with one more for Opus's nit
   (`opus_r13::*`):
   - **Codex 1 (`hash_during_restore`).** `ForkMap`'s rollback put live entries back by `insert`, which ran the key's
     user `Hash` after the fork had closed. A `call_out` in it wrote live (88 vs 1).
   - **Codex 2 (`failed_release`).** The fork kept its result, and the restore then ran the `when_live` releases.
     When one panicked, `close` unwound and dropped the result live (88 vs 1).
   - **Codex 3 (`suppressed_payload`).** The body's panic won, and a destructor's panic payload, which lost, was
     dropped after the restore. Its `Drop` wrote live (88 vs 1).
   - **Codex 4 (`empty_name`).** The R4 exemption was the string `""`, which any caller can pass to `shared_init`. A
     nested `shared_init("", ..)` read forked state (9 in the fork, 1 live).
   - **Opus nit (`a_shared_init_making_an_effect_is_refused_live_and_in_a_fork`).** A Shared init that made an
     effect panicked live but passed in a fork, where the effect's first run is skipped.

   Round 12's principle was "entering and leaving a fork run no user code". It missed values: user-defined values
   that were hashed, or dropped, after the restore, and a result held while live work ran. The exact rule is:
   - After the drain loop, no user-defined value is dropped, hashed, compared or cloned.
   - Nothing that can panic runs while the fork holds its result.
   - Every user-defined value the fork owns is either dropped inside the loop or handed to the caller.

   Changes:
   1. **`ForkMap` forks by an overlay.** While a fork runs, every write goes to an overlay of its own, a
      `HashMap<K, Option<V>>`, and every read looks there first. The live map is never written. The drain moves the
      overlay out whole (no hashing) and the loop drops it. The rollback moves nothing back, as nothing live
      changed. The undo-log insert path is deleted.
      - This is `ForkSlab`'s shadow copies, and Compose's state records made on a snapshot's first write.
      - Hashing a key in the body is user code inside the body, which is allowed.
      - Chosen over the brief's preferred option, a whole-map clone at first touch, because the cost stays
        O(keys written): `engine::tests::a_fork_map_costs_its_writes_not_its_size` writes 4 keys of a
        100,000-key map in 4 fork steps, where a clone would copy 100,000.
      - A persistent map would also be O(written), but it would add a dependency and change every read.
   2. **`when_live` releases are live work.**
      - A fork no longer runs them. They stay in the live queue, and the live loop runs them at its next settle
        (`reactive::runtime::settle`, before the cleanups and like them), or a caller runs them now with
        `engine::run_releases`.
      - So the fork hands its result over before any release runs, and a release's failure is a live failure, not
        the fork's.
      - The tests that ran releases at the fork's end now run the live queue after it:
        - `focus::tests::a_live_window_a_fork_drops_leaves_no_entry_and_no_signal`;
        - `list::view::tests::{live_views_a_fork_drops_are_released_after_it_and_wake_the_waiting, a_panicking_live_action_still_lets_the_views_release_run}`;
        - `failed_release`.
   3. **Panic payloads are the fork's.**
      - The body's panic seeds the winner, and the teardown's first panic is kept only if the body did not panic.
      - Every losing payload is dropped inside the loop, under a catch (`ColdFrame::lose`, `drop_payload`). A payload
        whose drop panics is replaced by that panic's payload and dropped the same way, at most `DRAIN_ROUNDS`
        times, and then leaked.
      - Only a string leaves the fork. A winner of another type (`panic_any` of a value with a `Drop`) is replaced,
        inside the loop, by a `String` that says so, and the original is dropped there
        (`ColdFrame::plain`; test `non_string_winner`).
      - The loop does not end while the winner is not a string.
   4. **The R4 exemption is a type, not a string.** `MAKING` holds a private `Mark`, either `Piece(name)` or
      `Engine`. Only `engine::allocating` and the engine's own entering push `Engine`, and no name a caller passes
      can stand for it.
   5. **An effect in a Shared init is refused alike.** `Effect::new` refuses a Shared init, live and in a fork.
      Its run is code that reads state, and a fork never runs it, so allowing it would differ between the two.

   Not changed: after the loop, the restore still drops what the exits hand back. By construction that is empty
   containers and the arena's node shells, whose values the drain took. The engine cannot check that for a type it
   does not know.

   Tests:
   - `fork_doom::codex_r13::{hash_during_restore, suppressed_payload, non_string_winner, failed_release, empty_name}`.
   - `fork_doom::opus_r13::a_shared_init_making_an_effect_is_refused_live_and_in_a_fork`.
   - `engine::tests::a_fork_map_costs_its_writes_not_its_size`.
   - `engine::tests::faults::every_fault_at_every_user_hook_leaves_the_live_state_whole`, the fault-injection
     harness below.

   **Fault injection closes the class, not the site.** Rounds 9 to 13 each found one more moment where user code
   ran at a bad time, so the class is now tested by enumeration rather than by review.
   - **The hooks.** The harness's user types count every hook they have:
     - a value's `Drop`, `Hash`, `Eq` and `Clone`, as a `ForkMap` key and value, in a `[Fresh]` piece and as the
       result;
     - a `[Fresh]` init;
     - a `[Shared]` init, and its stand-in's `Drop`;
     - a `when_live` release;
     - a panic payload's `Drop`.
   - **The faults.** The hook at index *k* injects one fault: a panic (`panic_any` of a payload with a `Drop`), a
     write of a `[Fork]` piece, a touch of a `[Shared]` piece, or a read of a signal.
   - **The scenarios.** 256 of them: kept or doomed, × body panic or not, × `ForkMap` untouched, insert, overwrite or
     remove, × touching any subset of `[Fresh]`, `[Fork]` and `[Shared]` pieces, × a release or not. Each runs on a
     thread of its own, once with no fault and then with each fault at each hook index in turn: 7,936 cases.
   - **The invariants after each run:**
     - the live state equals its value before the fork;
     - no hook ran inside the fork call after the fork closed;
     - a result is kept only if the fork was kept and no panic happened in it;
     - every panic in the fork surfaces as a string payload, the body's when the body panicked;
     - a release's panic surfaces from the live queue;
     - no fork is left open, and the next fork works.
   - **What it found on the final code:** nothing. Every run that panics with no panic fault is one the engine
     refuses loudly by design: a Shared init that reads or writes forked state, one that reaches its own piece, or a
     stand-in reached after its drop. Live state stayed whole in all of them.
   - **Mutation checks**, each fix reverted in turn:
     - releases run at restore: 128 cases fail;
     - a non-string winner let out: 1,504 cases fail;
     - a stand-in's panic not dropping the result: 32 cases fail.

   *Changed in review round 7: a reactive handle is `!Send`.* A `Signal`'s identity is a slot and generation in its
   thread's arena. Moved to another thread, a dead fork handle named that thread's node (Codex 1). No caller in the
   workspace moves one: values cross threads through mailboxes, as plain data. So the type says it, as `SlabId`
   does: `NodeId` carries a `PhantomData<*const ()>`, which makes every handle that holds it neither `Send` nor
   `Sync` (`Signal`, `Computed`, `Effect`, `Scope`), and no access pays a runtime thread check. Compile-fail
   doctests on `Signal`. One identity does cross threads: the instance that made a node, which the render thread
   carries back with input (`HitTarget`). That is now its own plain type, `reactive::OwnerId`: the slot, the
   generation and the arena's thread serial (`engine::arena`, an id from `unique_id`, never handed out twice in a
   process). It turns back into a node only on that thread (`OwnerId::node`), so on another it names nothing. The
   serial is read from the thread's own slots. Round 7 took `thread::current()` each time, which clones a handle
   per call (Opus nit 6, round 8). Tests: `engine::tests::an_owner_id_names_nothing_on_another_thread`, and
   compile-fail doctests on `Signal`, `Scope`, `Computed` and `Effect`.

   An `IdKind`'s cursor is keyed by where the kind is declared (`#[track_caller]` on `IdKind::new`: file, line and
   column), not by its name, so two kinds of one name never share a cursor (Opus nit 3). The counter lint covers a
   `[Fork]` cell of any primitive integer (`Cell<u32>`, `Cell<usize>`, ...), not only `Cell<u64>` (Opus nit 6).
4. **A handle is an id; its state is registered.** A type that is `Clone` and shares mutable state behind `Rc` is an
   alias the log cannot see, and a cold layout writes through it. `ListState`, `ScrollHandle` and `Columns` become
   `Copy` ids (slot and generation). Their fields live in Fork pieces with a log per field, and `ExtentTree` with a
   log per extent written, so a fork of a list of 10 million rows costs the rows it touched.
   *Built (F1), with one change:* the log is per handle, not per field. Each kind lives in one `[Fork(log)]`
   `engine::ForkSlab`; a fork's first write to a handle saves that handle's state once, and the rollback puts it
   back, frees what the fork inserted and restores the free list. The cost goal holds because `ExtentTree` is a
   persistent tree (`SumTree`, `Arc` nodes): saving it is O(1), and the fork's writes copy only the paths they walk.
   A per-field log would put the same guarantee in a dozen places. Review round 1 closed the rest of the cost: the
   free list is undone by a log of each push and pop the fork made, not saved whole, and a list's rows measured
   since a loan are a shared vector each copy reads up to its own length, so a list's state is saved in O(1) however
   many there are (`engine::tests::a_fork_of_a_slab_with_a_million_free_slots_costs_what_it_touched`,
   `virtual_list::tests::a_fork_copies_a_list_with_many_rows_measured_since_a_loan_in_o_1`). Each slot keeps the
   generation its next entry takes, which only grows, a rollback included, so an id a fork handed out never names
   an entry made after it; the slots the fork added stay as free slots, and a slot whose generations ran out is
   retired. Handles are `!Send` and `!Sync` (their ids name one thread's slab), checked by compile-fail doctests on
   `engine::SlabId`. A handle is owned by the scope it is made in
   (`engine::owned_by_scope`), as a signal is; made outside any, it lives until `dispose`. A copy kept past its
   owner cannot reach a reused slot (the generation differs). Used after it through its owner-facing API (a handle
   made in a short-lived scope and passed out), it fails loudly in a debug build, as a disposed signal's read does
   (`engine::check_alive`), and is inert in release (reads give the empty answer, writes do nothing). The engine's
   own paths read it quietly: a binding or a timer may run once after the owner ended, in a frame being torn down. No state of one handle is read or written inside another's borrow: user hooks, signals and other
   handles are called outside the slab. A binding an element holds reads through the handle, never a signal copied
   out of it: an element built while its owner ran may be laid out once after the owner ended (a layer's content
   mounted late), and it then reads the inert answer.
   *Changed in review round 2*, each with its test: a slot whose last generation a fork took stays retired through
   the rollback, as the free-list undo never brings it back as free (`a_slot_whose_last_generation_a_fork_took_stays_retired`).
   A loan of a list's heights is owned by its holder: the `Heights` and the `Views` request that took it carry its
   id, `end_loan` ends only that one, and `reorder_with` checks the loan again where it puts the heights in place,
   after the caller's mapping ran (compare-and-swap), so a loan a fork kept, a fork's `Views` dropped, or a mapping
   that lent anew never takes or ends a newer live loan. Rows the mapping measures are mapped too. A delivery to a
   list disposed meanwhile ends its request for good, quietly. The rows measured since a loan never grow an
   allocation another copy shares: a copy that pushes with no room left writes a tail of its own, so a fork's notes
   cost O(themselves) and the live allocation is as it was after the rollback
   (`virtual_list::tests::a_fork_never_grows_the_live_notes`).
   *Changed in review round 3:*
   - `reorder_with` is one optimistic transaction (Kung and Robinson, 1981: read, validate, write): every call of the
     caller's mapping runs first (the rows measured, then the anchor and the focused row), each take of notes only
     while the loan is still the list's, so a loan never drains a newer one's; one write then checks the loan, the
     shape and that no row was measured unseen, and puts heights, anchor and focus in place and ends the loan
     together, or ends the refused loan and writes nothing else. Rows the mapping measures get one more pass; a
     mapping that still measures is refused, the list's heights as they are (`a_newer_loans_notes_are_never_taken_by_an_older_one`,
     `a_splice_inside_either_mapping_refuses_the_reorder`, `a_mapping_that_never_stops_measuring_is_refused`).
   - One `Views` holds a list's heights at a time (`ListState::claim_heights`): another is refused, said as its
     failure, so two never void each other's loans in turn; views a fork made are inert after it; a failure
     delivered after the list's disposal drops the request's recipe too.
   - A `[Fork]` entry is saved and put back by value, so a pointer in it is put back, not what it points to: the
     focus to hand on and the focusable nodes moved from `FocusRoot`'s cells to a logged map by window
     (`focus::WINDOW_FOCUS`, `a_hand_on_a_fork_asks_leaves_the_live_one`), and the lint fails on a `[Fork]` entry whose
     type reaches a cell through `Rc`, `Arc` or `Weak`.
   *Changed in review round 4:*
   - **A loan belongs to a holder.** Round 3 checked the holder's claim and the loan separately, so a `Views` that
     dropped ended a newer loan another holder had made, and that loan's notes with it. A loan now records its holder
     (`Lent { loan, shape, holder }`), and `lend_heights` takes one: `lend_heights_for(holder)`, while a direct caller
     gets a fresh holder. `release_heights` ends only its own holder's loan, and ending a loan never clears another's
     notes. A request refused because another holder has the heights is no longer dropped as a failure. It is kept
     (the latest per holder) and asked again when the heights are let go (`ListState::wait_for_heights`). The waiter
     is called outside the list and never in a fork. So a list remounted with new views before the old views go still
     ends sorted. Tests: `view::tests::a_loan_another_holder_took_survives_a_views_drop`,
     `views_remounted_before_the_old_ones_go_end_sorted` and `two_views_on_one_list_take_turns`.
   - **The commit validates the whole read set.** Kung and Robinson's validation phase checks everything the
     transaction read, not only part of it. Round 3 checked the loan, the shape and the measured rows, but restored an
     anchor the mapping had moved, and plain `reorder` validated nothing. One function, `commit_reorder`, now commits
     both reorders. It checks what the heights were read from: the loan, its shape and that no row was measured
     unseen, or, for plain `reorder`, that the list's own persistent tree is unwritten since it was read (root
     identity, O(1)). It also checks what the anchor and the focused row were worked out from (`Seen`: the anchor,
     the scroll asked for and its revision, the focused row, the window, the end flags). Then it writes all of it or
     nothing. A mapping that scrolls or splices the list refuses the reorder and its write stands. Plain `reorder`
     returns whether it committed. It drops the heights it replaces on the calling thread: that is O(m), less than its
     own O(m log m), while `reorder_with` does both off the UI thread. Tests:
     `a_mapping_that_scrolls_refuses_either_reorder_and_the_scroll_stands` and
     `a_splice_inside_plain_reorders_mapping_is_not_lost`.
   - **`Held`'s policy is checked.** It is a value (`HeldFork::Signals`, or `Outside(why)`), and `Bound::set` in a
     fork refuses to write an `Outside` value (loud in a debug build;
     `listbox::tests::a_fork_writing_a_value_only_the_outside_writes_is_loud`).
   - A window dropped while its focus map is borrowed leaves no entry: its removal waits in a logged set for the
     map's next use (`focus::tests::a_window_dropped_while_its_focus_is_read_leaves_no_entry`). The waiting set
     is bounded: one entry per window dropped in a single borrow, emptied by the next focus read or write.
   *Changed in review round 5:*
   - **The read set is recorded first.** Kung and Robinson record the read set as the read phase starts. Round 4
     took `Seen` after the caller's mapping had already run on the measured rows (`reorder_with`) or on the heights
     (`reorder`), so a mapping whose first call scrolled the list was validated against the state it had made
     itself. Both reorders now take `Seen`, the focused row by key and the heights' identity before any call of
     `to` (`ReadSet`). Test: `a_first_call_of_to_that_scrolls_refuses_either_reorder`.
   - **No outside-world effect in a drop during a fork.** Dropping the last clone of live `Views` in a fork closed
     the real worker, while the release of the claim happened only in forked state that the rollback put back. The
     waiting views were then never woken (Codex). Live views a fork drops are now released whole once the fork has
     rolled back (`engine::when_live`, a `[Shared]` queue that `ColdFrame::end` runs after every piece is put back).
     The worker is closed, the loan ended and the claim released live, and the waiters are woken. The outbox cannot
     carry this, as it is always dropped. Views the fork made hold nothing live and end at once. Test:
     `view::tests::live_views_a_fork_drops_are_released_after_it_and_wake_the_waiting`.
   *Changed in review round 6:*
   - **The read set holds the window's focus.** `to` may move focus, and the commit restores the focused row, so
     the focus is read state. `ReadSet` now records the window's focused node before the first call of `to`, and
     `commit_reorder` refuses when it changed. Test: `a_first_call_of_to_that_moves_focus_refuses_either_reorder`.
   - **Every live drop in a fork waits for the rollback.** Round 5 deferred only `Views`. A drop that disposes a
     live node, unmounts a live instance or removes a live entry in a fork undid nothing the rollback could see once
     the fork-only regions held the fork's own. Each such `Drop` now goes through `engine::dropped(made_in_fork, f)`:
     what the fork made ends at once, what was live waits in `when_live`. Routed: `FocusRoot`, `Views`,
     `component::Root`, the layout bindings (`Tracked`, `Tracker`, `BoundRun`, `StyleBinding`), `layout::Extra`,
     `LayoutTree`'s probe and `value::source::Hub` (through `RenderNode::release`). Exempt, each by reason: state no
     fork rolls back (`Link`, `Wait`, `SignalHub`, `Planner`), scoped guards that end what their own scope began
     (`Making`, `ColdFrame`, `PopCurrent`, `Leaving`, `LeaveGuard`, `OwnerGuard`, `RunGuard`, `Marked`, `FlushGuard`,
     `BatchGuard`, `RestoreSub`, `Reading`) and test-only types. Test:
     `focus::tests::a_live_window_a_fork_drops_leaves_no_entry_and_no_signal`.
   - **A batch runner loses nothing to a panic.** `run_when_live` ran its actions in one loop, so a panic in one
     lost the rest (the `Views` release among them). Each action now runs under `catch_unwind`, and the first panic
     is raised by `ColdFrame::end` after all have run. `mailbox::deliver` does the same per mailbox and keeps every
     box. Effects, cleanups and post hooks already caught per item, and timers are taken one at a time. Tests:
     `view::tests::a_panicking_live_action_still_lets_the_views_release_run`,
     `mailbox::tests::a_panicking_handler_loses_no_other_mailbox`.
   *Changed in review round 7: what a panic leaves borrowed is given back.* Two paths took state out and gave it
   back only on the normal return.
   - **The mailboxes.** `deliver` ran every handler in one batch and caught each handler. The effects that batch
     ran as it ended were outside every catch, and ran before the boxes were put back, so one effect's panic dropped
     every receiver (Codex 5). Each mailbox's handler now runs in a batch of its own, and the handler and its
     batch's end are under one catch. A guard puts the boxes back however `deliver` ends.
   - **The reorder loan.** `reorder_with` consumes the `Heights` it is given. A panicking `to` unwound past it, and
     the loan stayed live with nothing able to take it back, noting every row measured from then on (Codex 6). A
     guard (`EndLoanOnUnwind`) now ends the loan on unwind, as a refusal does. The other loan paths already end
     theirs on drop: `Views` through its inner drop, and `lend_heights` calls no caller.

   Tests: `mailbox::tests::an_effect_panicking_as_a_handlers_batch_ends_loses_no_mailbox`,
   `virtual_list::tests::a_panicking_to_ends_the_loan_it_was_given`.
5. **A queue of the outside world is never drained in a fork.** `mailbox::deliver` does not run in one. What other
   threads sent stays in its channel for the next live turn.
   *Changed in review round 4:* a caller's callback is the outside world too. What it writes is whatever it captured,
   which no rollback puts back. So no entry that runs callbacks now runs one in a fork:

   | Entry | In a fork |
   | --- | --- |
   | `clock::run_due_timers`, `clock::advance_to` (`take_due`) | Run no timer, live or the fork's. The fork's timers go with it (`clock::tests::a_check_runs_no_timer_callback`) |
   | `component::run_post` (`after_layout`, `on_mounted`, `on_updated` hooks) | Runs none. `POST` is Fresh, so the fork's own hooks go with it (`component::tests::a_check_runs_no_after_layout_hook`) |
   | `mailbox::deliver` | Returns at once (round 1) |
   | `reactive::flush` (effects), `flush_deferred` (effects a render made) | Run none, and what was queued is dropped (round 1) |
   | Cleanups (`on_cleanup`, a disposed node's) | Taken out with the frame's nodes and dropped, never run (`drain_cold`) |
   | `ListState::release_heights`'s waiters | Not woken; the rollback puts the claim back |

   *Changed in review round 5: one gate.* Round 4 gated entries one by one and missed some. `Model::set` and the
   component bind's `UseModel::set` called `on_change` in a fork, and the table above called the scroll handle's
   layout hook engine-owned, although `ScrollHandle::on_layout` installs the caller's closure (Codex 2 and nit, Opus
   nit 7). A caller's closure is now reached only through `engine::call_out(f) -> Option<R>`, which runs `f` outside a
   fork and returns `None` inside one. `Callback::call` goes through it, so every `on_change`, and every handler or
   veto passed as a `Callback`, is gated in one place. A fork that needs the answer of a non-unit callback has none,
   since it never calls it. It gets `R::default()`, and a debug build fails loudly. The default answer is the one
   given when no caller is asked (`listbox::Pick::Allow`). Derivations the engine runs during a render (`Derive`,
   `active_from`) are pure and still run.

   | Entry | In a fork |
   | --- | --- |
   | `Callback::call` (every `on_change`, `Model::set`, `UseModel::set`, handlers and vetoes passed as callbacks) | Calls nothing; `R::default()`, loud in debug unless `R` is `()` (`component::tests::a_write_in_a_check_calls_no_on_change`, `a_bound_write_in_a_check_calls_no_on_change`) |
   | `ScrollHandle::on_layout`, `ScrollHandle::on_reveal`, `ListState::on_layout` (caller hooks) | Not called; no pass asked again, nothing revealed (`engine::tests::a_check_calls_no_scroll_layout_hook`) |
   | `ScrollHandle::on_layout_built`, `on_built`, `on_wheel` (`pub(crate)`, installed by `gui`'s own list and containers) | Run: they are the engine's own and write only forked state (the list's or the handle's, logged in their slabs) |
   | Live `Views` whose last clone the fork drops | Released once the fork has rolled back (`engine::when_live`), never in it |

   *Changed in review round 6: the gate is at dispatch.* Round 5 gated `Callback` but not the closures stored raw.
   An element's event handlers, `Layer::on_dismiss`, `Div::on_measured` and `Members::on_laid_out` were called
   directly, so a fork that dispatched an event or asked a layer to close ran caller code. Each is now called through
   `call_out` where it is dispatched: `element::in_owner` (every handler an element keeps), `Layer::ask`, the
   measured loop in `layout::write` and `Members::notify`. So that the next stored closure cannot miss it, a lint
   (`process_state::every_kept_closure_is_reviewed`) fails on any `dyn Fn`, `FnMut` or `FnOnce` a `gui` type or
   static keeps unless `CLOSURES` lists it with why it is safe (gated where called, pure, or not on a fork's thread).
   Tests: `engine::tests::a_fork_calls_no_event_or_dismiss_handler` and `the_closure_lint_sees_what_is_kept`.

   | Entry | In a fork |
   | --- | --- |
   | Element handlers (`on_click`, keys, wheel, press, hover) | Not called (`in_owner`) |
   | `Layer::on_dismiss` | Not asked; the request is not taken |
   | `Div::on_measured`, `Members::on_laid_out` | Not called |

   *Changed in review round 7: each dispatch site states its fork answer.* Round 6 answered every gated handler
   with `R::default()`. For a consumed flag, that answer is "not consumed", so the fork ran the engine's default
   action where live may have taken the handler's (Opus nit 2). An answer is now stated where each handler is made
   (`element::InFork`), never by a blanket default.
   - `Same(r)`: the answer under which the engine takes live's path whatever the handler would do. It is used for
     every handler with no answer (`()`).
   - `Refuse { site, inert }`: used where the answer decides what the engine does next (`on_key_down`,
     `on_window_key`, `on_wheel`, `focus_on_press`, `press_focuses`). Only the caller knows that answer, so no fork
     answer can keep the fork on live's path. *Changed in round 8:* a fork that dispatches there is doomed and
     dropped (`Doom::Dropped`, which names the site). It runs on with the answer under which the engine does nothing
     more, and its result is thrown away. Round 7 failed loudly instead, by panicking through sites that catch.

   `Layer::ask` answers "not taken" and `Callback::call` answers `R::default()`, loud unless `R` is `()`. Both
   already stated their answers.
   Test: `engine::tests::a_fork_dispatching_a_consumed_flag_is_dropped`.

   *Changed in review round 8: the closure lint's gaps.* It now also reads:
   - higher-ranked bounds (`dyn for<'a> Fn`), as in the alias `message_scroller::Meta`;
   - bounds on an `impl` only (`impl<F: Fn() -> T> Read for Closure<F>`, in the header or a `where` over lines),
     listed as the type's `<F>`;
   - declarations over several lines (a field's type, an alias before its `;`), joined first;
   - any visibility, including `pub(in path)`.

   The counter lint also reads the integer fields of what a fork logs by value. That covers a `Logged` struct and
   the entries of a `[Fork(log)]` slab or map. A field advanced by hand there is put back by a rollback, so a fork
   and the live run after it would hand out the same value.
   - This is how `interact::Handles::last` handed a fork's `Interaction` number out again live (Opus I1). It now
     comes from `unique_id`.
   - The four fields that are no identity are listed in `COUNTS` with why.

   Tests:
   - `process_state::the_closure_lint_sees_what_is_kept`;
   - `the_lint_catches_counters_in_logged_state`;
   - `engine::tests::a_forks_interaction_is_never_one_live_code_makes`.

   *Changed in review round 7: the closure lint reads all of the workspace's libraries, by owner and field.* Round
   6 read only `gui`, matched only `Rc`, `Box`, `Arc` or `Weak` directly around `dyn Fn` on one line, and keyed its
   list by bare names, so a type of a listed name elsewhere passed (Opus nit 3, Codex nit). The lint now reads
   `gui`, `primitives` and `shadcn` by braces, so a field is told from an argument by what declares it. It matches
   a closure however it is held: `RefCell<dyn FnMut>`, `dyn Send + Fn`, `dyn 'static + FnOnce`, a generic field
   bounded by a closure (`F: Fn`, in the header or a `where`), a tuple field, a variant, an alias or a static. A
   borrow (`&dyn Fn`) keeps nothing and is not listed. Its list is keyed by file, owner and field
   (`TreeData.label`), and an entry whose closure is gone fails. The two named in review are justified there.
   `OpenTooltip.close` is the tooltip's own write of its model, whose caller callback is a `Callback` and so gated.
   `TreeData`'s closures are pure reads the layout makes.

Three pieces whose policy is decided here:

| Piece | Policy | Cost guarantee |
| --- | --- | --- |
| Style interner (`style.rs`, `CHAINS`) | Shared. A `LinkKey` holds its parent's address, and links are compared by address (`TextDecorations` is). Sharing the table is what keeps those addresses equal between a fork and the live frame. A fork may intern; what it interned stays. Sweeps are deferred while a fork is open. `held()` is a diagnostic and is excluded from the "no counter moved" proof | Nothing per frame; a fork adds nothing |
| Id blocks (`engine.rs`, `ID_BLOCKS_LEASED`) | Two process-wide monotonic counters of the blocks of 2^16 ids leased (rule 3): the live space and the forks'. A fork leases only from its own, and the live cursors (`unique_id`'s own, never rolled back) are untouched by it, so the ids handed out live are as without the fork | One atomic increment per 2^16 ids a thread hands out |
| First-use registration (`EngineKey::registered`) | Kept: a piece first touched inside a fork is entered at once, as today | One predictable branch per access, counted by the instruction gate |

**The lint for process-wide state is an allowlist, in CI, and lands at F1.**

- A `static` or `thread_local!` in `gui`, `gui-tls`, `primitives` or `shadcn` must either be registered or have a
  type from an allowlist of immutable types. Anything else fails. `clippy.toml` keeps `std::thread_local` disallowed
  in `gui`, and the same line is added for `primitives` and `shadcn`.
- `OnceLock`, `LazyLock` and `Once` that hold data never changed after it is made are allowed and listed by name: the
  icon drawings, the font data, the clock's start, the heap's init, the GPU instance, the layout pool, and the root
  filter of `primitives/src/filter.rs:55`.
- A `gui` handle type with a field of `Rc<…Cell…>` or `Rc<…RefCell…>` must be registered with a fork policy. A test
  enumerates these types and fails on one that is not.
- Dependencies are out of scope: their statics are not ours to list. What they hold that matters (font caches, the
  GPU device) is reached through pieces that are registered.
- The check reads the sources; it is not a type-level proof.

*Built (F1)* as `crates/gui/tests/process_state.rs`, run by `cargo test -p gui`. It reads `src` and `examples` of the
four crates (an integration test or bench is a binary of its own, whose harness lock no app runs). Each listed static
carries its reason, and an entry whose static is gone fails, so the list cannot let a new one of the same name
through. Besides the immutable data above, the list holds the Shared tables (the style interner's `CHAINS` and
`DEAD`, `calc.rs`'s `TABLE`, the fonts' `SHARED`, the chunk kinds' `REGISTERED`), the id blocks, `gui-tls`'s
slots, the registry's own pieces, the statics two `macro_rules!` make (`engine_local!`'s own, and the icon
libraries' `SvgIcon`s) and one test knob (`stack.rs`'s `MEMO_MIN`).

Review round 1 closed the lint's blind spots, each with a fixture test: a static whose type reaches a cell through a
reference (`&Counter`) is caught, as a reference is only as immutable as what it refers to; a static a
`macro_rules!` makes (`static $name`) is caught; and the handle check reads `primitives` and `shadcn` too, and
follows aliases, `Weak`, enums and the structs of other files that a file's imports or paths name (a closure behind
`Rc` is a callback and is not followed). Review round 2 narrowed that last rule and closed one more: a trait object
behind `Rc` or `Weak` is a handle unless its trait is on a short list of callbacks (`Fn`, `FnMut`, `FnOnce`, each
with its reason: what a closure captured is its maker's, checked where that is declared), and a static named `_X`
is read as any other (`the_lint_catches_trait_objects_and_underscored_statics`). Round 3 keyed each handle's entry by
the file that declares it and its name, so a type of a listed name elsewhere is caught; made `primitives`' `Held`
(which `Bound` shares behind `Rc`) require each implementor to state its fork policy (`Held::fork_policy`; the
library's are signal-backed, and a caller's own model, such as one that refuses writes, says how it is written); and added the `[Fork]` pointer and
counter checks above (`the_lint_catches_pointers_counters_and_handles_by_file`). The handles it finds besides the
three made ids, each listed with its policy in the test:

| Handle | Policy |
| --- | --- |
| `NodeFlags` (`interact.rs`) | Shared: what it shares is the handle bound to the node, which a cold bind never writes (`value::source::cold`) |
| `Views` (`list/view.rs`) | Shared: written by handlers, its mailbox's deliveries and `spawn`, which in a fork returns at once: no worker starts (the worker starts at the first request outside a fork), nothing is lent, no request is counted |
| `Group` (`value/source.rs`) | Shared: a cache of a value read from signals, equal on a hit and a miss (rule 2); a cold build reads it anew |
| `Members` (`members.rs`) | Shared: filled by a layout's join of the members it laid out and told at its end of pass, both of which a detached layout (a fork's) skips (`join_lists`); read by handlers |
| `Frame` (`layout/frame.rs`) | Fork by value: one layout's output; a fork lays out its own. The cells its behaviours reach are `NodeFlags`' |
| `Noted` (`list/virtual_list.rs`) | Fork by length: the rows measured since a loan, shared by copies that each keep their own length; a copy that pushes into a shared allocation with no room writes a tail of its own, never growing the shared one |
| `Row` (`primitives/menu.rs`), `MemoryStorage` (`primitives/splitter.rs`), `InFamily` (`primitives/splitter.rs`), `SelectEvent`, `MessageScroller` (`shadcn`) | Shared: written by layout callbacks a detached layout never runs, handlers, effects, timers, registrations a cold render does not make, or an app's `save` |
| `Highlighting` (`shadcn/code_block`) | Shared: a cache, equal on a hit and a miss (rule 2) |
| `Select` (`primitives/select.rs`) | Shared as the row above. Its alignment is no state: the render hands it to `viewport(d, aligned)`, whose layout hook (the scroll handle's, forked with it) reads it, so a fork rendering the other alignment writes nothing live (`select_lists::a_fork_rendering_the_other_alignment_leaves_the_live_select_as_it_was`) |

The select's scroll is a handle made in its setup and owned by its scope, started afresh as the list opens
(`ScrollHandle::reset`); its layout hook holds the select's parts weakly, so nothing outlives the select
(`select_lists::an_opened_select_mounted_and_unmounted_leaves_nothing_behind`).

**Sending in a fork (F1).** Until K4a gives `Sender::send` its `Result`, a send on the UI thread inside a fork drops
the value and returns `true` (counted as sent). Mailboxes' open state is a `[Fork(log)]` map, so one a fork opens is
gone with it, and `deliver` returns at once inside a fork (rule 5). Fork gating is per thread: a fork is the
calling thread's (`engine::forking`), so the style interner's sweep waits, and a send is dropped, only on the thread
whose fork is open. A send from another thread (a worker's) while the UI thread forks goes to the channel and waits
there for the next live `deliver`. Another thread (a layout pool thread interning a style) may sweep the interner
while the UI thread forks: a sweep takes out only entries whose link nobody holds, which no lookup can return, so
the fork's results are as without it (rule 2); what moves is the table's size and its count of links let go, both
diagnostics outside the "no counter moved" proof.

**What the lint cannot see** gets tests instead (F1):

- **Alias.** Lay a scrolled list out inside a fork and roll back: anchor, window, extents, pins and thumb equal what
  they were, and a copy of the handle made before the fork reads the same. The same for `ScrollHandle` and `Columns`.
- **Capture.** A `Sender` cloned before the fork is used inside it; after the rollback its mailbox has received
  nothing.
- **Counters.** Fork, mount a chart and a list, roll back: no counter moved, on the UI thread or a pool thread.
- **Detached mount.** A branch the live tree lacks is mounted in a check, and its body calls `spawn` and opens a
  mailbox: after the rollback no job has started and no mailbox is registered.
- **The lint itself.** A fixture with a new `static AtomicU64` fails the check.

Built at F1 as `virtual_list::tests::what_a_fork_writes_through_a_list_is_put_back`,
`scroll::tests::a_scroll_a_fork_made_is_put_back`, `columns::tests::widths_a_fork_observed_are_put_back` (alias),
`mailbox::tests::what_a_fork_sends_through_a_sender_it_captured_goes_with_it` (capture),
`engine::tests::a_cold_frame_leaves_the_content_ids_as_they_were` and `ids_of_two_threads_never_meet` (counters),
and `process_state::the_lint_catches_a_new_static` and `the_lint_catches_a_new_shared_handle`. The detached-mount
test waits for `spawn` (K4a); its mailbox half is `a_mailbox_a_cold_frame_made_leaves_no_drain`.

### The outbox

Everything that leaves the UI thread's state goes through a port. A port is the only way to get the effect.

```rust
/// A typed way out. `Copy`. Owned by the scope that makes it.
pub struct Port<M: 'static> { /* id */ }
/// `deliver` runs on the UI thread with no fork open.
pub fn port<M: 'static>(deliver: impl FnMut(M) + 'static) -> Port<M>;
impl<M: 'static> Port<M> { pub fn post(self, message: M); }

/// Work off the UI thread, owned by the current scope and cancelled with it. `then` runs on the UI thread.
pub fn spawn<T: Send + 'static>(job: impl FnOnce(&Cancel) -> T + Send + 'static, then: impl FnOnce(T) + 'static) -> Task;

impl<T: Send> Sender<T> { pub fn send(&self, value: T) -> Result<(), Closed>; }

pub mod clipboard {
    /// `context` comes back with the answer. `Wrote` is `Ok` or the system's error.
    pub fn write<C: 'static>(text: String, context: C, answer: Port<(Wrote, C)>);
    pub fn paste() -> Option<String>;   // an input read
}
```

**One rule:**

- **Outside any fork, an effect is delivered at once.** `post` calls `deliver`; `send` writes the channel. A live
  turn queues nothing and allocates nothing for the outbox. The one exception is re-entry: a `post` made while a
  `deliver` of the same port runs is queued and delivered when that `deliver` returns, never entered recursively.
- **Inside a fork, every effect posts to the outbox**, `Sender::send` and cancels included.
- **The outbox is always dropped.** In a check a post is also a fault of the code checked, because bindings and
  bodies have no side effects by contract; debug builds report it.

What follows from it:

- **`Sender::send` returns `Result<(), Closed>`.** A post inside a fork counts as `Ok`. On the UI thread with a fork
  open, `send` posts; it finds out by reading the thread's fork flag, which no other thread has. On another thread
  it writes the channel: that thread is the outside world.
- **Ports, mailboxes and `spawn` may be made inside a fork**, by the body of a branch the check mounts. They are
  registered in the fork, never started, and dropped with it. A job therefore cannot have been started by a fork.
- **An effect with an outcome is confirm-then-act.** `clipboard::write` gives its answer through a port; nothing
  depends on a returned `bool`. Live, the answer arrives in the same turn.
  - **Cut:** the context is the field's `NodeId`, the text revision and the range that was copied. The handler
    deletes that range only if the field still has that revision. Otherwise nothing is deleted and the copy stands.
  - `code_block`'s "copied" feedback shows on the answer.
- **Reading the outside is an input, not an effect.** `clipboard::paste` stays a synchronous read. AccessKit actions
  are inputs too: they arrive as `Input::Act`, as today.
- **Messages should be plain data.** A destructor that acts on the world is outside what `gui` can undo.
- **App I/O** is a port the app makes with `port`: a save, a request, a log line.

**Ports `gui` owns** (F3; about 14 call sites in `primitives` and `shadcn`):

| Port | Replaces |
| --- | --- |
| Mail | `Sender::send` on the UI thread inside a fork |
| `spawn`, and its cancel | `std::thread::spawn` |
| Clipboard write | `clipboard::copy` |
| Window | Title, cursor, IME area, wake |
| Commit | The engine's own commit of a frame to the render thread, and the accessibility tree update |

### What lands at K4a, so that S8 is written once

The signatures and lints that component code sees are final at K4a, before any family is ported. Some of them a
check does not need. They are kept because ADR-0030 will need them, so that a trial, when it is designed, changes no
component; and each is also right on its own.

| At K4a | Right on its own because | ADR-0030 needs it for |
| --- | --- | --- |
| `Port`, `port`, `spawn`, `Sender::send -> Result` | A job is owned and cancelled with its scope; a check can hold an effect back | Posts as the observation |
| `clipboard::write(text, context, answer)` | Cut cannot delete on a stale answer | Answers that can be held or simulated |
| `Callback::call -> Option<R>`: `None` when the owner is disposed; a caller asking for a veto treats `None` as no veto | A call into a disposed owner has a defined result (ADR-0026) | — |
| `Signal::new` requires `T: Clone` | A signal holds a value, not a handle; devtools and stepping back can keep a copy | Undo of a mutation in place |
| Lint: no `Signal<T>` in `gui`, `primitives` or `shadcn` whose `T` is or holds `Cell`, `RefCell`, `Rc<…Cell…>` or `Mutex` | A change inside such a value notifies nobody | State a log can see |
| Lint: no closure in `primitives` or `shadcn` captures `Rc<Cell>` or `Rc<RefCell>` | The same: state that drives the view is in a signal | State a log can see |
| Time and randomness only through `gui::clock` and a new `gui::random`; lint on `SystemTime::now`, `Instant::now` and `rand` in `primitives` and `shadcn` | Tests and Scenarios already drive `gui::clock`; a component that reads the system clock cannot be tested by them | Recorded inputs |
| Clippy bans in `primitives` and `shadcn`: `std::thread::spawn`, `std::fs`, `std::process::Command`, direct clipboard calls | An effect a check cannot hold back would leave from a sampled check | The same |

- **Existing sites:** `drawer.rs:729`, `:731`, `holder.rs:661` and toast's `swallow` (signal lint) and
  `date/mod.rs:52`, `:366`, `:378` (clock lint) are fixed in M15. The 9 captured `Rc<Cell>` and `Rc<RefCell>` in
  components move into signals in S8, family by family, as part of each family's one port; the lint is on for a
  family from its port. The 17 `State` types in `primitives` and the `gui` types kept in signals derive `Clone`.
- **At K4a the ports deliver at once and nothing forks.** F2 and F3 change no component code: F2 turns the fork on
  in release, F3 routes the same calls through the outbox while a fork is open.

### Panics

- **In a check:** the check is discarded and the panic is reported; the live root is untouched. In debug builds and
  tests the report fails the test. In a sampled check in release it is written to the mismatch record: the cold path
  panicked where the live one did not, which is a difference.
- **During rollback:** a destructor that panics aborts the fork: every piece is still put back, what the fork made
  and has not dropped is leaked, and the root is poisoned (torn down and mounted again, ADR-0013's rule), because a
  value half dropped may have left what it owned inconsistent. A destructor that keeps making work fails the same
  way after 16 rounds (existing `DRAIN_ROUNDS`).

### The boundary for app callbacks

`gui` cannot sandbox a Rust closure. The boundary is stated, not hidden:

- **A check runs no handler.** The app code it runs is sources, bindings and the bodies of missing branches, which
  have no side effects by contract.
- **Guaranteed:** what such code reaches through `gui`'s API is isolated. A write to a live signal is refused; a
  post is dropped; ids, lists and mailboxes are put back. So the sampled check is safe in every app without a
  declaration.
- **Not guaranteed:** what a binding or a body does on its own: a file written with `std::fs`, a `std::sync::mpsc`
  sender of the app's, an `Arc<Mutex<…>>` shared with a thread, a `Cell` it captured. This breaks the contract of a
  binding with or without a fork. The clippy bans and lints cover our crates; an app answers for its own.

### Sampling in release (F4)

- A release build runs a `Check` when the app has been idle (no input, no animation) for a set time, 60 s by
  default, at most once per idle period. `GUI_COLD_CHECK=1` still checks every frame.
- **It can be abandoned at a step boundary** when input arrives: between sources, layout, paint and accessibility,
  and inside each of the last three between relayout islands (ADR-0014). Abandoning puts the live state back at
  once. The values the fork made are parked and dropped at the next idle, inside a fork opened for that, so their
  destructors still reach no live state.
- **Gate, measured:** first-input latency after idle at p99, with sampling on, is at most the same with sampling off
  plus 2 ms. On a machine class where the gate fails, sampling defaults to off.
- A mismatch does not panic in release. It writes a record (the difference and the recent inputs) to a diagnostics
  port. The app decides where that goes; by default nowhere outside the process.
- Before F4, the comparison of chunk data must stop being O(data) (wave C §2.9): it compares content hashes kept on
  the snapshots.
- F4 also merges the three incremental and cold pairs into one path each where that is possible (`record_changes` and
  `record_all`, `compose_changed` and `whole`, `replan` and `plan`; review §6.5), and brings every Scenario and stress
  step under the check.
- Gate: release monkey runs with 0 mismatches, and a sampled frame lets no effect out.

### Amendments

- **ADR-0023.** The cold check is not debug-only. Its definition is "What a check frame runs". It gains the sampled
  mode and the mismatch record.
- **ADR-0026.** Shares the definition above. The K4a table above is part of K4a.
- **ADR-0012** (line 126) and **AGENTS.md** (line 85). "Async stays outside the graph: a status signal fed through
  `mailbox`" becomes: through `spawn` and a mailbox, both ports. `Signal::new` asks `T: Clone`. Components read time
  and randomness through `gui::clock` and `gui::random`. AGENTS.md's testing line "incremental result equals a cold
  rebuild" (line 214) gains "in release too, sampled".
- **`engine.rs`'s module text** keeps its three policies; it gains rule 4 (a handle is an id) and rule 5.

## Consequences

- **Release is checked.** The path users run is compared with a cold frame, without a debug build.
- **Nothing a fork did is ever real.** There is no commit to get wrong: no cleanup order at commit, no half-committed
  panic, no merge of queues.
- **The fork is the base for the other three uses** (input preview, stepping back, agent preview), but none of them
  is decided here. They need a fork that runs handlers, which is ADR-0030 and is open.
- **Effect APIs change shape.** An effect with an outcome answers through a port. About 14 call sites change, once,
  at K4a's signatures.
- **`Signal::new` gains a `Clone` bound**; 17 `State` types and the `gui` types in signals derive it.
- **Components lose the system clock and `rand`**; three sites in `date` move to `gui::clock`.
- **`ListState`, `ScrollHandle` and `Columns` change** from shared objects to ids; their callers follow.
- **A forked frame is laid out on one thread.**
- **Release binaries carry the registry and the arena log.** The two-part gate keeps their steady-state cost at B0.
- **An app built with `panic = "abort"` has no sampled check and no `GUI_COLD_CHECK`.**
- **Stated limit:** the lints read sources; they are not type-level proofs.

## Alternatives rejected

- **A fork that commits** (option 3; rounds 1 and 2 of this record). Every rule it needed was a place where the
  fork could differ from a live frame.
- **Deciding the trial here.** Three rounds did not settle its contract (its start point, time, cleanups, what is
  compared, writes in place, hidden history). An accepted record with an unsound part would have had S8 written
  against it. The trial is stated as an open problem in ADR-0030; its component-visible API lands now.
- **A check that changes the live tree under a log and puts it back.** The detached tree needs no log of the
  retained tree and cannot leave the live tree half restored.
- **Snapshot and restore instead of an undo log.** A fork would cost what the thread holds (60 thousand nodes, a list
  of 10 million extents), on every check.
- **Queue every effect on live turns too.** Every live effect would wait for a turn's end and the live path would
  pay for a mechanism it does not use.
- **A second signal constructor for types that cannot be cloned.** A second kind of signal that every later use of a
  copy (devtools, stepping back, a trial) would have to refuse; deriving `Clone` is cheaper than carrying the case.
- **The style interner as a per-thread cache.** Links are compared by address, so two tables would make equal styles
  unequal between a fork and the live frame.
- **Keep the layout pool on inside a fork.** The pool threads' state would need its own fork, across threads.
- **Process-wide atomic counters with the gaps accepted.** Ids would differ between a run that forked and one that
  did not, which breaks replay and the "no counter moved" test.
- **A denylist of mutable types for the lint.** A new wrapper type would pass unseen. An allowlist fails closed.
- **Nested forks.** No use needs one yet, and every piece's log would need a stack.

## Revisit when

- ADR-0030 is accepted: it may change Fresh queues to forked ones, add logs, and add a second user of the fork. It
  must not change the check.
- Stepping back through W live frames is built (frontiers §4): the logs of live turns must then be kept.
- The source-reading lint misses a real leak: move to a compiler lint.
