# 0028. A frame can fork in release

Status: accepted, 2026-11-24; not yet built (M13, F1–F4). The direction was chosen by the user on 2026-11-23
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
3. **An id that must be unique is a thread ordinal and a per-thread Fork counter.** No process-wide counter is
   advanced from the UI thread, so a rolled-back fork leaves every counter where it was. F1 moves the two series
   counters and each of the other eight that the UI thread advances.
4. **A handle is an id; its state is registered.** A type that is `Clone` and shares mutable state behind `Rc` is an
   alias the log cannot see, and a cold layout writes through it. `ListState`, `ScrollHandle` and `Columns` become
   `Copy` ids (slot and generation). Their fields live in Fork pieces with a log per field, and `ExtentTree` with a
   log per extent written, so a fork of a list of 10 million rows costs the rows it touched.
5. **A queue of the outside world is never drained in a fork.** `mailbox::deliver` does not run in one. What other
   threads sent stays in its channel for the next live turn.

Three pieces whose policy is decided here:

| Piece | Policy | Cost guarantee |
| --- | --- | --- |
| Style interner (`style.rs`, `CHAINS`) | Shared. A `LinkKey` holds its parent's address, and links are compared by address (`TextDecorations` is). Sharing the table is what keeps those addresses equal between a fork and the live frame. A fork may intern; what it interned stays. Sweeps are deferred while a fork is open. `held()` is a diagnostic and is excluded from the "no counter moved" proof | Nothing per frame; a fork adds nothing |
| Thread ordinals | A process-wide counter advanced only when a thread starts. A fork starts no thread, so no fork advances it | One increment per thread |
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

**What the lint cannot see** gets tests instead (F1):

- **Alias.** Lay a scrolled list out inside a fork and roll back: anchor, window, extents, pins and thumb equal what
  they were, and a copy of the handle made before the fork reads the same. The same for `ScrollHandle` and `Columns`.
- **Capture.** A `Sender` cloned before the fork is used inside it; after the rollback its mailbox has received
  nothing.
- **Counters.** Fork, mount a chart and a list, roll back: no counter moved, on the UI thread or a pool thread.
- **Detached mount.** A branch the live tree lacks is mounted in a check, and its body calls `spawn` and opens a
  mailbox: after the rollback no job has started and no mailbox is registered.
- **The lint itself.** A fixture with a new `static AtomicU64` fails the check.

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
