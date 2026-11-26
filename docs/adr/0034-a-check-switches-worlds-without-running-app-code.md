# 0034. A check switches worlds without running app code

Status: accepted under delegation 2026-11-25 (F1, after review round 14 and two independent red-teams, Codex xhigh and
Opus, which converged on this design); being built (M13, F1). Partly superseded by
[ADR-0035](0035-an-engine-value-ends-in-the-world-that-made-it.md) (accepted 2026-11-27): S4's live-drop routing, the
`Unique`, `Private` and `Bookkeeping` drop classes, S6's per-thread serial, S7's borrow guards and the scope of "What
a check isolates". Supersedes the parts of
[ADR-0028](0028-a-frame-can-fork-in-release.md) that promise to recover from a failing destructor during a fork's
teardown ("Panics, during rollback"), the drain-loop arbitration of panic payloads (its rounds 11 to 13), the public
fork entry (`in_cold_frame`), the public release queue (`when_live`, `run_releases`) and the generic `ForkMap<K, V>`
and `ForkSlab<T>` bounds. Everything else in ADR-0028 stands: one fork mechanism, never committed, its one user the
check; the registry's three policies; the outbox; sampling in release.

## Context

Fourteen review rounds of F1 failed in the same way. Each found a user type whose engine-invoked hook (`Drop`,
`Hash`, `Eq`, `Clone`, an initialiser, a panic payload's `Drop`, a release job) panicked, wrote engine state, read
it or re-entered the engine at some moment of the fork's life (entry, drain, restore, release, arbitration). Each
round closed the moments it was shown; the next found another. `engine.rs` grew to 4,079 lines. A fault-injection
harness of 7,936 cases found nothing, and the next review found seven more defects, because the harness enumerated
the hooks we had already thought of.

The root cause, agreed by both red-teams: **switching back to the live world runs open-ended application code**
(drains, destructors, releases, payload drops), **and the engine promises to recover when that code fails.** Every
policy (Fresh, Fork, log, Shared, stand-in) and every moment (enter, drain, restore, release) must then reason about
what arbitrary code can do, and another round can always find a new path to the same transition. Sealing the generic
API alone does not close it: the check's body runs the app's components, which hold user values (signals over
`Rc<RefCell<T>>`, closures, cleanups, `Keyed` memos).

Three guarantees are entangled today and are separated here (Codex red-team, §1):

- **Representation integrity.** After success, cancellation, doom or a body panic, the next engine access sees a
  complete live world; no user code observes a half-restored registry.
- **Engine isolation.** The check observes the sampled frame's sources; its writes, allocations, registrations,
  effects and destruction change no later live observation: ids, focus, waiter wake-ups, resource ownership.
- **Application purity.** What a body does on its own (a captured `Rc<Cell<_>>`) is outside the guarantee, as
  ADR-0028's "boundary for app callbacks" already says.

The review-14 reproducers, each now closed by a decision below rather than at its site:

| # | Reproducer | Broken guarantee | Closed by |
|---|---|---|---|
| r14-1 | a slab whose init reads another piece allocates in live generation space; its fork id reads a later live entry | isolation (identity) | S6, S8 |
| r14-2 | one panicking overlay value skips the other seven destructors | integrity | S9, S2 |
| r14-3 | a losing release payload's drop replaces the winner and discards later jobs | isolation (obligations) | S3, S5 |
| r14-4 | thread exit drops queued releases uncalled | isolation (obligations) | S5 |
| r14-5 | a re-entrant drain runs releases out of order | isolation (obligations) | S5 |
| r14-6 | an equal-but-different key shows the fork's key where live keeps the old | isolation | S7 |
| r14-7 | 1,000 mutable accesses to one key clone the live value 1,000 times | cost | S7 |
| opus r14 | after a runaway, the result's payload is dropped live after the restore | integrity | S1, S2 |

## Criterion

**The switch between worlds is a structural operation that cannot call application code, and speculative garbage
has an explicit disposal policy. Live obligations never depend on speculative destruction.**

## Options

- **A separate thread for the check.** Impossible as stated: `Rc` values and closures are `!Send`, and recreating a
  blank app loses mounted state and changes the oracle. It would need a transferable snapshot of sources.
- **OS `fork()`.** Forbidden by the unsafe rule, unsound with threads, and the GPU device is not inherited.
- **A second engine instance on the same thread.** Still needs live inputs by copy-on-write and does not remove the
  disposal problem.
- **Compose-style MVCC** (record chains selected by snapshot id). Worth it with several long-lived readers or a
  concurrent writer; with one non-nesting, discard-only reader a base and one overlay give the same visibility rule
  without record chains or version reclamation.
- **Process isolation.** The strongest: survives arbitrary double panics. Needs replayable inputs and equivalent app
  code; fits an offline oracle sooner than a transparent sampled check.
- **Keep patching moments (rounds 9 to 14).** Rejected: the surface is unbounded.

## Decision

Keep the one discard-only check on the UI thread, and enforce the criterion.

### S1. Structural switch

Restore is moves only. No exit hook drops, hashes, compares, clones or calls a user value: an `Entry`'s exit puts
the live record back and gives nothing back. Speculative values are disposed of *inside* the fork's world, before
the switch, where touching fork state is legitimate. A post-switch tripwire (`gui_tls::watch` from the start of the
restore to the fork's return) asserts, in every debug check, that no engine state was reached after the switch.

### S2. Disposal goes on past a fault; only a runaway sets values aside

Revised after review r16 (it superseded "fail-stop": the first fault quarantined the rest).

- Every disposal box is dropped under its own catch, in the fork's world, under the watch, **whatever faults
  before it**, and the disposal runs on to its fixed point (the drain rounds, the stand-ins, the result once it is
  not kept). This is `core::ptr::drop_in_place`'s rule: a struct's fields and a slice's elements are all dropped
  though one's drop panics, the first panic propagating once the rest are done (the Reference, "Destructors"; std's
  `Vec` drop guard). The abort std accepts when a second panic meets the first in one unwind cannot happen here:
  each box has its own catch, so two panics never meet (S9).
- The first fault is the report (`Faulted { phase, piece, message, later }`); later ones follow it in order. The
  check is still `Faulted`, and sampling stays poisoned for that engine: no later fork opens on the thread
  (`Skipped(Poisoned)`). ADR-0028 already asked for poison after a rollback destructor failed.
- Values are set aside (`mem::forget`) only on a **runaway**: a disposal still making work after `DRAIN_ROUNDS`
  rounds. There, dropping more would not end. What is left, the stand-ins and the result are moved out and
  forgotten, and the switch finishes by moves.
- A fault-free run never leaks; a faulted one without a runaway leaks nothing either. After a runaway, live values
  reachable from what was set aside (the engine's own copies of live handles included) may stay in memory for the
  rest of the thread, and the destructors of theirs that are not obligations may never run. Only obligations are
  guaranteed to end (S4): with their live owner, never with a count a set-aside value pins (review r17, decision 6).
- Only the body's own panic is recovered, meaning reported (as a string) with the rollback done.

Why "stop running user code" was narrowed to the runaway. Fail-stop treated a panicking `Drop` as evidence that
the fork's world was corrupt, so it ran nothing more. But a check's disposal is the fork's own: the values are the
check's, dropped in its world under the watch, and S1 keeps the switch to live moves alone whatever they do. A panic
in one box says nothing about the next. What fail-stop cost was real: the forgotten boxes held copies of live
handles the engine itself made (a logged map's overlay copy of a live `Rc`, a scroll handle's shadow sharing its
live callbacks, a `[Fork]` cell pointed at a live value), and a closure holding a live `Rc<Root>`, so one unrelated
fault pinned live values past their live owner (Opus r16 I1, Codex r16 2). Dropping every box closes all of these at
once, the opaque ones included, rather than proving each kind of box shareable-safe. The runaway is the one case
where going on cannot end; there S4 makes the forgetting harmless: obligations end with their live owner.

Forgetting is memory-safe and runs no code (Rustonomicon, leak amplification). It does not by itself release a live
obligation; S4 makes that independent of it.

### S3. Payloads normalised at the check and release boundaries only

At catch time a `String` payload is moved, a `&'static str` copied, and any other payload replaced by a fixed
diagnostic and forgotten: its `Drop` is user code. Live dispatch keeps typed payloads (ADR-0015's dispatch rule).

### S4. Live obligations have a live owner

Mandatory engine transitions (close a list worker and notify it, end a height loan, wake waiters, release a focus
root's entry, unmount a root, dispose a scope) are triggered by the live lifecycle: a live handle's drop or its
owner's teardown. They are never triggered by a speculative value's `Drop`, nor by a reference count reaching zero
through a speculative clone. A handle cloned while a fork runs is speculative: its drop releases nothing, so
forgetting it (S2) leaves no obligation pending. Each release is split into its mandatory transition (exactly once,
idempotent) and any optional callback delivery.

**Live obligations end with their live owner, never with a reference count** (review 15). A speculative clone of the
handle itself can be told apart, but a check may also clone an `Rc` that *contains* a live handle (a rebuild closure
cloned into a handler on every render), and park it in a closure or a `Box<dyn Any>`. Opaque values cannot be
inspected, so S2's quarantine forgets the live handle inside with them, and its count never reaches zero. Owner-bound
release is what makes that quarantine harmless: an obligation a shareable handle's drop performs is also ended by the
live owner it was made under (`engine::owned_by_scope`, `on_cleanup`, or the window that owns it), exactly once,
idempotently and never in a check; a later drop of a clone finds it done. A lint (`crates/gui/tests/obligations.rs`)
lists every `impl Drop` of `gui`, `primitives` and `shadcn` with its class (guard, unique, engine-private, bookkeeping,
owner, test-only) and fails on one not listed, on a guard or unique type that becomes `Clone` or is named behind a
shared pointer, and on an owner whose registration is gone. The ones that end a live obligation:

| Type | File | Obligation | Live owner |
|---|---|---|---|
| `Views`, its `Inner` | `list/view.rs` | close the worker, end the height loan, let the heights claim go, wake the waiting views | the scope running, else the window whose code runs (`engine::own` in `Views::new`); with neither, a debug panic. Its last live handle too |
| `FocusRoot` | `focus.rs` | dispose the window's focus signals, remove its entry | the pipeline (`Pipeline::close`: `begin_close`, `settle_close` in one turn, `finish_close`; `Pipeline::drop` hands the same close, `close_window`, to `engine::dropped`, so a live pipeline a check drops closes after the switch back, review r18, decision 4) |
| `Root` (its `Life`) | `component.rs` | unmount the instance tree | made by `Root::new`: the scope running, else the window whose code runs (`engine::own`); with neither (the thread's own window never closes), a debug panic. A pipeline's (`Root::of_pipeline`): the pipeline's close turn. In every case its last handle's drop too, whichever is first |
| mailbox (no `Drop`) | `mailbox.rs` | let go of the handler and its bell | the scope running (`on_cleanup`), else the window it was made in (`close_window`), asked through `engine::owner`; with neither, a debug panic |
| timer (no `Drop`) | `clock.rs` | cancel | the scope it was made in (`on_cleanup`); ids are `Copy` |

**An obligation never ends on its last `Rc` alone** (review r16). Even with S2 revised, a runaway sets values
aside, and a handle among them pins its count. So every obligation behind a shareable handle has an owner that ends
it regardless: the lint has a forbidden class, `LastRc`, which no entry may take, and it fails a Private or
Bookkeeping drop whose body releases live work through the release queue (`engine::dropped`, `when_live`), the
textual sign of an obligation ending on a count. `Root` was listed Unique ("a field of the pipeline"), but the
public `Root::new` made standalone roots an app can hold behind `Rc`; it is an Owner now.

**One registration path** (review r17, decision 5). Every obligation-bearing constructor (`Root::new`,
`Views::new`, `mailbox`) asks `engine::own`/`engine::owner`: the scope running, else the window whose code runs,
else a debug panic ("made with no scope running and no window's code"). The answer is acted on, never dropped.
The obligations lint fails on a direct `owned_by_window` call elsewhere.

> **Superseded by [ADR-0035](0035-an-engine-value-ends-in-the-world-that-made-it.md) (built in F1b, 2026-11-26):**
> this paragraph, the `LiveDrop` token, `engine::dropped`, the table below and its test, and the next paragraph's
> `made_in_fork` routing. An engine value's end is held in `engine::Ends`, which routes by the world that made it;
> an explicit close or dispose of a live node inside a check now faults (ADR-0035, D3). The table is kept as the
> record of what was converted; no test reads it.

**Only a drop reaches the live world from a check** (review r17, decision 3). An explicit lifecycle operation inside
a check (`Pipeline::close`, `Root::close`, `Scope::dispose`, a window's close, a mailbox's retire) acts on the
check's world: the root's state is the check's copy in `ROOTS`, the window's record its overlay of `WINDOW_WORK`,
the unmount the check's instance tree. Codex r17 3 was a check's `Pipeline::close` queueing a live release that
dropped the live work table. Queueing to the live release queue (`engine::dropped`) takes a `LiveDrop` token.
`LiveDrop::in_drop()` builds it, and the obligations lint (`only_drops_queue_live_drains_split_and_owners_register_on_one_path`)
checks that it is built only inside a `Drop` impl, and that `when_live` is called only by the queue's own file. A
live value a check lets go has no other moment to end, so its drop is the one path. The drops that build the token
are exactly these (`the_adr_names_every_drop_that_reaches_the_live_queue` fails on a drop that builds one and is
not a row, and on a row whose drop builds none; review r18, decision 5):

| Type | File | What its drop hands to the live queue |
|---|---|---|
| `Root` | `gui/src/component.rs` | the root's end (`end_root`): unmount, then its `ROOTS` entry |
| `FocusRoot` | `gui/src/focus.rs` | the window's focus entry let go (`cx::unmade`, `end_window`) |
| `Pipeline` | `gui/src/app.rs` | the window's close (`close_window`), as `Pipeline::close` runs it |
| `Trackers` | `gui/src/app.rs` | the pipeline's trackers' scope disposed (V2 found it disposed in the check's world) |
| `Inner` | `gui/src/list/view.rs` | the views' release, once their last handle goes |
| `Views` | `gui/src/list/view.rs` | a live handle let go (`handle_dropped`) |
| `Wait` | `gui/src/value/source.rs` | its registration left on the source's list |
| `Hub` | `gui/src/value/source.rs` | its tracking node, through `release(LiveDrop)` |
| `LayoutTree` | `gui/src/layout/mod.rs` | its probe node, through `release(LiveDrop)` |
| `Extra` | `gui/src/layout/mod.rs` | its tracking nodes, through `release(LiveDrop)` |
| `Tracked` | `gui/src/layout/binding.rs` | its tracking node, through `release(LiveDrop)` |
| `Tracker` | `gui/src/layout/binding.rs` | its tracking node, through `release(LiveDrop)` |
| `BoundRun` | `gui/src/layout/binding.rs` | its tracking node, through `release(LiveDrop)` |
| `StyleBinding` | `gui/src/layout/binding.rs` | its tracking node, through `release(LiveDrop)` |

**A live owner's drop never takes the explicit path** (review r18, decision 4). Codex r18 1 was `Pipeline::drop`
calling `Pipeline::close` in a check: the close acted on the check's world, the live window's mail was never
retired, and after the switch and shutdown its sender still delivered. A drop's job is the explicit operation run
in the owner's own world: the drop builds the token and hands that operation to `engine::dropped`, which runs it now
for an owner the running world made (`made_in_fork`) and after the switch back for a live one. The obligations lint
(`no_owner_drop_takes_the_explicit_close`) reads the drops of `Pipeline`, `FocusRoot`, `Root`, `Inner` and `Views`
and fails on a call of their explicit operation outside the job handed to `engine::dropped` (a closure bound by
`let` and handed in counts as the job; one also called directly fails). The
per-method guards this replaced are gone: `Root::new`'s `forking()` return and its scope registration's `is_cold`
test, `Life::end_with_window`'s `is_cold` test, `Views::new`'s scope registration's `is_cold` test,
`Views::spawn`'s early `forking()` return (the recipe is now the check's state; only the worker request stays live),
and `owned_by_window`'s refusal in a fork.

The layout tree's trackers, interaction flags and source registrations (`Tracked`, `Tracker`, `BoundRun`,
`StyleBinding`, `Extra`, `NodeInteraction`, `Wait`, `Hub`), the pipeline's `Trackers` and the GPU world's `Planner` are
unique values of the pipeline: no check value reaches them. A check can still drop the pipeline that holds them (an
app's last `Rc` of a window let go in a render), so each of them that ends engine state hands it to `engine::dropped`
from its drop, as the table above lists: V2 (review r18, decision 6) found `Trackers` disposing its scope in the
check's world, leaving the live scope and its two render nodes for good
(`a_live_pipeline_dropped_in_a_check_ends_as_a_live_drop`). The `Planner` stops its worker threads, the outside
world's, the same in either world. `SignalHub` is shared only by live mounts' registrations.

### S5. One release queue

- Engine-private (`pub(crate)`): `when_live` and `run_releases` leave the public API.
- A job asked for while a fork runs, or while the queue is draining, is enqueued; outside both it is enqueued and the
  queue drained at once, so job starts are FIFO.
- The queue's borrow is released before each job runs; each job is popped on its own.
- One active drainer: a nested drain returns at once, and the outer one runs what was added.
- Each job runs under its own catch; a failure (its payload normalised, S3) loses no other job; the first is raised
  once the queue is empty.
- `engine::shutdown()` drains it live, called by the owner (`Pipeline`'s drop, so App, Scenario, Window and
  Offscreen) while services are reachable. Never a TLS destructor (`LocalKey` destruction order is unspecified).
- Covers every caller of `dropped`: `Root::drop`, `FocusRoot::drop`, scope disposal, list views.

### S6. Identity is per world

- Allocation is routed by the active world before any slot is touched: a `ForkSlab` allocates in its fork region
  whenever a fork runs, whether or not the fork entered the slab.
- Fork ids come from a fork-only domain: a fork-region id carries the fork's serial, a per-thread counter no
  rollback moves.
- Lookups reject ids from an expired world: a fork-region id resolves only inside the fork that made it.
- Live id sequences are identical with and without sampling: a test runs the same inputs in two engines, one
  sampling, and compares the id streams.

### S7. Sealed storage

- `ForkKey`: sealed, `Copy + Eq + Hash`, implemented in `gui` for the integers, `TypeId` and canonical engine ids.
  Keys are copied by dereference.
- Values come in three kinds: `Copy` (bit-copied); an engine-sealed `ForkClone` with first-write-only
  materialisation and a stated cost; replace-only entries (lookup, replace, tombstone; no owned previous value, no
  `entry_mut`, no `Default`).
- `entry_mut` checks the overlay first. A value an overlay write displaces goes to the fork's disposal list, never
  dropped during the caller's borrow.
- The back doors close: the macro's custom-policy arm, public `Entry` fields and `EngineKey` constructors become
  crate-private; `primitives` and `shadcn` get a narrow copy-cell and handle-map API.
- Per piece: `MemoBox`/`Keyed` keep the layered memo's sharing with an explicit memo overlay, user key operations
  only in the body; the open tooltip's close callback becomes an identity backed by a live owner; `WindowFocus`
  splits the frequent `hand_on` mutation from the published focusable set; list, scroll and columns state get
  field-specific first-write (share immutable roots, overlay mutable observations, resolve callbacks by identity).
- A logged container is never a value the caller can move (Codex r15-2: `take()` on a `[Fork(log)]` map moved out
  the live rows with the overlay; the default left in place took the check's writes into its base). A `[Fork(log)]`
  key is a `LogKey`, not an `EngineKey`: no `take`, `set`, `replace` or cell. Reads get `&L`; writes get a `LogMut`
  with no `DerefMut`, whose only methods are the overlay API (`ForkMap` replace, forget, insert, remove,
  `entry_mut`; `ForkSlab` insert, forget, remove, and `write` through `&`), so `mem::take`, `replace` and `swap`
  find no `&mut L`, in `gui` too. Compile-fail fixtures on `engine::log_key` hold it. `ForkMap` and `ForkSlab` are
  the only `Logged` types: the interaction handles' composite log became two logged maps and two `[Fork]` scalars.
  Backstop: a container replaced anyway has a rollback with no log it began, which quarantines and faults the
  check (S2). Rejected: `Pin` (needs unsafe projection), no public constructor (the declaring crate's init must
  build the value; two keys' containers could still be swapped), and the overlay held in the registry (the live
  rows would still move out).
- **A logged value is sealed** (review r16, Codex 1). A read lends `&V` of the live value itself, with no copy, so
  a `V` whose `&` can change it (a `Cell<Option<Root>>` a check `take`s the live root out of) writes live state no
  log records, and a logged collection nested in another writes to a log the outer one never drains (a nested
  `ForkSlab`'s write was read by the next check). `ForkMap<K, V>` and `ForkSlab<T>` require `V: ForkValue`, sealed
  by a private module, of three kinds (review r17, decision 2):
  - **Data, `ForkData`**: `Copy` plus a seal, a closed set `gui` implements: the scalars, `TypeId`, engine ids
    and handles (`Signal<T>`, `Scope`, `FocusHandle`, `SlabId`, …), and `Option`, arrays and tuples (up to four)
    of data, and `gui`'s own structs of data (`Id`, `RootState`), declared through the crate-private
    `fork_data_struct!`, which bounds every field's type by `ForkData`. A reference, a `Cell`, a `RefCell`, an
    `Rc` and a raw pointer are never `ForkData`, so data is what a `&` cannot change. `Copied<T: Copy>` is gone:
    `Copy` admitted `&'static Cell<_>` (Opus r17 I3).
    **Sealed completely** (review r18, decision 1). The r17 seal was public (doc-hidden), so any crate could
    implement it by hand, under an alias too, and admit a `&'static Cell<u32>` a check then wrote live through
    (Codex r18 3, Opus r18 verify I3). `ForkData`'s supertrait is now a public trait in a `pub(crate)` module, as
    `ForkValue`'s is: no other crate can name it. The derive (`gui-derive`) is deleted. Rust cannot tell a
    derive's output from a hand impl, and `bytemuck`'s answer (an `unsafe` trait plus a derive) conflicts with
    `forbid(unsafe_code)`. The derive's only callers outside `gui` were the toaster (`primitives`) and sonner's
    `Store` (`shadcn`). `Store` is `Copy` engine handles alone and now goes in as `AppHandle<Store>`, which it is
    never changed through. Another crate's value goes in as a tuple of data or behind an `AppHandle`. Revisit
    when a downstream crate needs a logged data struct of its own that neither fits.
  - **App objects, `AppHandle<T: ?Sized>`**: an `Rc` of an object the app owns (a context value, a tooltip's close
    callback), which the engine keeps by handle. A check isolates the collection's structure (which handles sit
    under which keys); the object behind a handle is app heap, outside the check's isolation, as an `Rc` a
    component's closure captures is (see "What a check isolates"). `dyn Any` and `Rc<T>` are no longer values of
    their own: an object reached through them could be anything, a `RefCell` included (Codex r17 2, Opus r17 I2).
  - **gui's audited states**, each taken apart through `Parts` as it drains (S9).

  `ForkClone: ForkValue`. The seal is a type, never a caller's statement: both seals sit in private modules, so
  the r17 text lint (`process_state::fork_data_is_derived`) is gone. Compile-fail fixtures on `ForkValue`:
  `Cell`, `RefCell`, `ForkMap`, `ForkSlab`, `Rc<RefCell<_>>`, `Cell<Option<Root>>`, `Rc<dyn Any>`, `&'static
  Cell<u32>`, a seal impl of `ForkValue`'s, and from outside `gui` (review r18): a hand impl of `ForkData` and its
  seal for a `&'static Cell<u32>` wrapper, the same under `use … as` aliases, a hand impl through a type alias,
  and another crate's `Copy` struct as a logged value. Positive controls: `u8`, a tuple row,
  `AppHandle<dyn Fn()>`, `AppHandle<dyn Any>`, `Option<(u8, u64)>`. In the workspace, sonner's toasters, the
  tooltip's closers and the context map are `AppHandle`s. Rejected: an auto trait `Freeze` bound (unstable), and
  copying on every read (a read would cost a clone and still lend the pointee).

#### What a check isolates

A check isolates **engine-owned state**: the registry's pieces, the reactive arena's signals and effects, the
instance tree, ids, and the engine handles. An engine handle (`Root`, `FocusRoot`, `Views`, `ListState`,
`ScrollHandle`, a mailbox `Sender`, `Scope`) holds an id and data fixed when it is made. Its mutable state is a
registry piece's, keyed by that id (`ROOTS` for a root, `VIEWS` for list views, `WINDOW_WORK` for a window), so a
check's writes to it go to the check's overlay by construction and the switch puts the live state back (review
r17, decision 1).

**No state is per world by assertion** (review r18, decision 2). The r17 lint let a field hold a cell with the
reason "routed per world", and two such reasons were false. A tree a check made with `LayoutTree::new` was not
detached, published into a live `Members` list, and emptied it (Codex r18 2). A check reading a live registry
counted its visits and reads on the live `Entries` through `&` (Codex r18 nit, Opus r18 plausible 1). State that
differs by world now lives in one of two places:
- in a registry piece;
- in a cell of one world, `engine::WorldCell` or `engine::WorldRefCell`. Its type records the world that made it
  and refuses, with a panic naming the cell, a write from any other world before the write lands. Reads are free.
  A copy belongs to the world of whoever copies it.

The former per-world fields are now world cells:
- the layout tree's lists, measure hooks and placed fields;
- `Members`' six fields;
- `ByScroller::changed`;
- a registry's `Entries` and `Slot::gone`;
- `NodeFlags::bound`;
- the source waiters and hubs.

Those who write them act on the answer:
- A tree made in a check is detached, whichever constructor made it.
- A registry read in a check on a live registry counts nothing, and its removal goes through `update`, which a
  check is refused on a live signal.
- A mount's registration a check lets go leaves the live list through the release queue.
- `Noted` writes in place only into an allocation of its own world.

The field lint (`tests/cell_fields.rs`, review r18, decision 3) parses `gui`, `primitives` and `shadcn` with
`syn`. It follows each crate's type aliases to any depth, reads enum variants (named and tuple) and the items a
macro's body declares, and fails on any field that holds a cell, an atomic, a lock or a lazy cell at any depth,
unless the field is listed with one of three reasons:
- **Immutable**: set once (a `OnceCell`), or a memo of a pure function of the value that holds it, equal on a hit
  and a miss.
- **Plumbing**: a thread no check runs on or reaches, which the reason names, or the pipeline's own state.
- **Mechanism**: the world switch's own cells, only in its files.

Its shared parser (`tests/common/sources.rs`) is the one the kept-closure lint's sources come from.

A check does **not** isolate app-owned heap: what closures capture, an `Rc` an app shares, the object behind an
`AppHandle`. No engine can snapshot arbitrary app memory. The contract is that render and setup change app heap
only through signals. This is React's render-purity contract (components and hooks must be pure; StrictMode calls
render twice in development to surface the impure ones: react.dev, "Keeping Components Pure" and `<StrictMode>`).
It is also Compose's: snapshot isolation covers only snapshot state objects, and a composable must not write other
state as it composes (developer.android.com, "Side-effects in Compose" and the `Snapshot` API). A render that
breaks it is an app bug the cold check helps surface (its frames diverge from the cold rebuild). It is not a hole
in the engine's isolation. So a check that takes a live root out of app heap really removes it from the live
world, and its unmount is consistent with that.
- **The audited states, field by field** (what `&` reaches, and why it cannot change live state no log records):

| State | Field | Rule through `&` |
|---|---|---|
| scroll `State` | `version`, `moved` | arena signal handles (`Copy`): a write goes to the reactive arena, which forks |
| | `pos`, `end`, `wheeled`, `wheels`, `added`, `generation`, `reach` | `Copy` |
| | `laid: Option<Rc<ScrollLayout>>` | replaced whole; written in place only through `&mut` when unique (`Rc::get_mut`) |
| | `hook`, `take`, `reveal`, `on_wheel` | `Rc<dyn Fn>`: called, never replaced through `&`; their captures are app state |
| list `State` | `extents` | a persistent tree: a write makes a new root through `&mut` |
| | `built`, `waiting` | `Rc` written copy-on-write through `&mut` (`Rc::get_mut` / a new `Rc`) |
| | `since_lent` | `Noted`, its own tail written through `&mut` |
| | `hook` | `Rc` callback, called only |
| | `active_from` | a `Holder` id; `taken` a `Weak`, upgraded to read |
| | the rest | `Copy` (signals are arena handles) |
| columns `State` | `widths: Rc<[Width]>` | never written after it is made |
| | `seen.base: Rc<Vec<f32>>` | `Rc::make_mut` through `&mut` only |
| | `seen.over`, `version` | owned `Vec` (through `&mut`); an arena signal |
| `WindowFocus` | `hand_on` | `Copy` |
| | `focusables: Rc<Focusables>` | refilled in place through `&mut` when unique; else a new set |
| `WindowWork` | `effects: Vec<Effect>` | effect handles; disposed only by the window's live close |
| | `owned: Vec<Weak<dyn WindowOwned>>` | upgraded and ended by the window's close; in a check, `end_with_window` ends the check's copy (`Root`'s state in `ROOTS`, the views' in `VIEWS`), never the live world (review r17, decision 3) |
| `ViewsState` | `recipe: Option<Rc<dyn Fn>>` | called, never replaced through `&` |
| | `shown: View` | `Arc`s of rows, never written after made |
| | the rest | `Copy` and an owned `String` |
| `MemoBox` | `Rc<dyn ErasedMemo>` | read through `&`; a check's write goes to its own overlay (`fork_clone`), the live maps shared read-only |
| `NodeFlags` | six flag signals | arena handles |
| | `bound: Rc<WorldCell<Option<Interaction>>>` | the one cell reached through `&`: the world's that made the flags, so a check's write to live flags is refused by type (review r18); its writes are guarded by `value::source::cold()` too |
| `Bounds`, `mailbox`/`focus` `()` and `u64` maps | | `Copy` |

### S8. Initialisers

A registered piece's init that reads another piece is a debug panic at first touch (handle allocation is not a
read). The workspace tests inventory real ones; with none, `NEW_READ`, `UNSET`, `made_in_fork` and `Unset` go.

### S9. Per-element disposal

The drains of the reactive arena, the instance tree and the overlays return one box per element. No container of
user values is dropped whole, which removes the double-panic abort (a second panic while a container's drop glue
unwinds) and lets S2 go on past each fault. Every logged value splits as it drains (`ForkValue::split`, review r17,
decision 4): `ForkMap`'s overlay and `ForkSlab`'s check part call it for each value. Data splits to nothing, an
`AppHandle` to its one `Rc`, a `Vec` of values to each element, and an audited state to its fields through
`Parts`. Codex r17 1 was a `Vec` of callbacks dropped in one box, two panics in one unwind, abort. The obligations
lint fails on a drain (`drain`, `*_drain`, `take_fork`) that calls `Box::new` itself, except those listed with why
they hold one element (`fork_drain`: a `[Fork]` cell's one value). It flagged the clock's timer drain (already a box a
timer), which now goes through `engine::whole` per timer, so the rule has no unlisted exception. An audited state's
`Parts` names every field in its pattern (no rest), so a field added states how it drops. A field left to the
rest drops with the call's frame, outside the drain's catch (the obligations lint
`every_audited_state_takes_apart_every_field`).

The split costs a box per element, so it applies only where it buys something: a value whose drop may run app code
(review r16, decision 4). `Parts::inert()` marks what drops only memory: the scalars, `String`, `Box<str>`, `Weak`,
`Cell` of `Copy`, and `Vec`, `VecDeque`, `HashMap`/`HashSet` (with a hasher that has no drop glue), `BTreeMap`,
`Option` and tuples of only those, and a `FreshValue` with no drop glue. An inert container goes whole, one box.
It is a function rather than an associated constant because `Box<T>` is a `FreshValue` by a blanket impl that
coherence will not let a narrower `Box<str>` impl override; the blanket's `inert()` tells `Box<str>` apart by
`TypeId`, which folds to a constant when optimised.

### S10. Closed public boundary

The production operation is "check this frame", returning a closed report: `Ok`, `Mismatch`, `Skipped(reason)`,
`Cancelled`, `Faulted`. Raw closure entry is crate-private, with a `#[doc(hidden)] gui::testing` hook for downstream
tests. Compile-fail fixtures with positive controls cover raw entry, a custom `Entry`, the policy arms, unapproved
keys and bypass constructors.

### S11. Delete what S1 to S9 make unnecessary

Only after their preconditions hold: the `lose`/`plain`/`drop_payload` arbitration, and the stand-in rounds if first
touch no longer relies on stand-in teardown order. Production-path transitions and line counts are reported before
and after (tests excluded).

## Verification

Two independent oracles, so that what we did not imagine is found:

- **V1. Collection model.** Random operation sequences (insert, overwrite, remove, get, write, iteration as a set,
  len, is_empty, contains, tombstones, free-slot reuse, escaped handles) on a `ForkMap`/`ForkSlab` inside a fork and
  on a plain std model give the same observations; live is untouched after the fork.
- **V2. Whole-engine differential.** The same deterministic app inputs in two independently initialised engines, one
  never checking, the other checking or cancelling at generated boundaries. Compared: the sampled frame and the later
  live turns, id streams, effect and release traces, worker closure and wake counts, persistent allocation growth.
  Skipped samples are recorded with a coverage floor, so skipping every hard sample fails.
- **V3. Transition generator.** Short sequences of first registration, body return, doom, cancel, disposal,
  re-entrant enqueue, settle and shutdown, with two faults in one run, a successful destructor that schedules work,
  a fault after a partial release and runaway disposal; the exact winning fault and phase asserted; abort and
  thread exit in subprocess tests.
- **V4. Seeded mutants** the suite must detect: a live write instead of an overlay write, a reused generation, a
  premature world switch, a stale cold cache, a missing notification, a reversed release order.
- **V5. Vertical slice first.** A real virtual-list check interrupted after its first write switches back with no
  destructor; the next input, the next live id, the worker and a waiting view behave exactly as in a run that never
  checked. Then tabs and select through primitives, shadcn and the demos.
- **V6. Gates, optimized builds, real check:** sampling-off overhead; first input after idle p99 at most +2 ms
  (ADR-0028); cancel and reclamation, allocations and retention over repeated samples; a million-key memo, a
  million-row list with a small viewport, a large focusable window with repeated writes to one key, a worker with a
  waiting height claimant, each reported as records touched against data size.

## Built so far (2026-11-26)

- S1 to S10 are in, each with a test that fails without it. S8's inventory found reading inits only in tests, so
  `NEW_READ`, `UNSET`, `made_in_fork` and `Unset` are gone.
- S7. `ForkKey`, `Snapshot`, `Logged`, `Parts` and the new `ForkClone` are sealed. `ForkClone` is a copy that runs no
  user code at a stated cost, made on a check's first write. `ForkMap` gives `insert`, `remove` and `entry_mut` for
  `ForkClone` values (a value the check wrote itself is moved back out, never copied). It gives `replace` and `forget`
  for any value; these hand nothing back, so the check copies nothing (shadcn's toasters and the tooltip's trigger
  boxes use them). `ForkSlab` likewise offers `write`/`remove` for `ForkClone` and `forget` for any value. Per piece:
  - `WindowFocus` keeps the small hand-on beside a shared `Rc` of the published focusables. A check's layout
    publishes its own set, and live refills its set in place.
  - List, scroll and columns states share their large groups (keys and pins, waiting holders, the scroll layout, the
    widths) and their callbacks (`Rc`, `Weak`, ids). Columns overlay their observed widths.
  - `Keyed` memos share the live base and layer read-only and write an explicit overlay. Key operations run in the
    body only, and the drain hands back one box per key (`ForkSlab::by_parts`).
  - The tooltip's `OPEN` holds an id; close callbacks live in a replace-only map their live owner writes.
- S9 covers `[Fresh]` pieces: `Parts` takes their values apart one box per element.
- S10. A check is cancelled by `cancel_check` or by a `CancelFlag` its owner sets from any thread. It unwinds at its
  next cancellation point with its own marker (no panic hook) and reports `Cancelled`. The points are between the
  phases, at every instance render, at every piece of text shaped, and every 64 nodes a whole paint records; they are
  inert once the teardown starts. Release sampling (F2) sets the flag as input arrives.
- S11: the payload arbitration is gone. The stand-in rounds stay, because a Shared first touch still makes a
  stand-in. The doc-hidden public switch, restore and drain methods, `fork_enter`, `fork_drain` and `fork_work` are
  crate-private. Production lines of the fork machinery (`engine.rs`, `engine/frame.rs`, `engine/release.rs` and
  `engine/frame_report.rs`, tests excluded): 2599 (1730 code) before this record, 3485 (2325 code) after. The growth
  is the value kinds, `Parts` and cancellation.
- Verification:
  - V1, V2 (now with checks cancelled inside a phase), V3 (now with cancels) and V5 are tests in `gui`.
  - V4 is a hand-seeded mutant set, because cargo-mutants is not installed. Every mutant is detected.
  - V6, release build with the check compiled in, measured in thread CPU time because the machine was shared (load
    average 55 during the final runs). Input arriving during a page check waits at most p99 0.90 to 1.28 ms: the
    longest uninterruptible slice (p99 382 to 534 us, 57 to 58 points a check) plus the teardown after a cancel.
    Before cancellation points it waited the whole check, p99 2.50 to 2.83 ms. Page sizes: a million-row list, alone
    and with a worker and a waiting claimant. Wall time on the loaded machine: input waited p50 84 to 381 us, p99
    0.83 to 8.7 ms.
  - V6 records touched: at most 2 of 2,000,000 for a million-key map plus a million-entry slab, 1 for a thousand
    writes to one key, 1 for a million-key memo, and at most 36 for a page check.
- A check's drain lets go at once of an `Rc` it shares with another holder (a count-down, which runs no code), so a
  fault's quarantine never pins live maps: a live memo a faulted check wrote folds again.
- After review r15 (Opus I1, Codex 1 to 5), one invariant: the fork machinery owns only what the fork made. Live
  handles, live owners and undo state never pass through a box the fork drops, forgets or replaces.
  - The drain reaches a fixed point before the switch. Each drain hook runs under the switch's watch; a hook that
    reaches another piece the check entered leaves it for the next round. `Parts` never drops anything itself: a map
    or set is emptied entry by entry and handed back whole, hasher included. The switch drops a displaced value only
    when it is the live value itself (`Snapshot::same`); anything else is quarantined and the check faults.
  - A plain `[Fork]` cell the check never wrote gives its drain nothing, so no clone of a live handle is in a batch
    a fault could quarantine.
  - A live owner a check drops (a `Root`, by a normal drop or as the body unwinds; `FocusRoot`; `Views`) queues its
    transition on the live release queue. It never runs it in the speculative world.
  - One destructor per box. Containers take their elements apart recursively (`Vec`, maps, sets, `Option`, tuples):
    a map's key and value are two boxes, and an `Rc` another holder shares is let go of. A caller's type in a
    `[Fresh]` container states `FreshValue`: one opaque box. What it holds cannot be inspected, so a live obligation
    inside it must end with its live owner (S4), not with its drop.
  - A cancellation point does nothing while the thread unwinds (Opus r15 P1): unwinding again aborts.
  - Font faces and variation coordinate sets a check interns first are its own: the text system forgets them as the
    check exits, so live face ids are those of an engine that never checked. The per-frame cold check runs after the
    live paint, so the cold scene reuses the ids live paint gave and a face only the check uses is a real difference.
  - V2 generates teardown faults (pinning an opaque `Rc` of live views or not) followed by the live owner's drop,
    and compares worker closure, wakes, loans and claims, every id kind's live cursor, chunk ids and glyph face ids
    after each step. Mutants caught: views released only on the last handle (V2 and the Opus repro), the drain
    boxing an untouched live handle, the drain's watch starting after its first round, and `Root::drop` unmounting
    in place during a check.
- After review r16 (Codex 1a, 1b, 2; Opus I1 with three repros):
  - S2 revised: `drop_each` drops every box under its own catch and the disposal runs on to its fixed point
    past any fault; the result, not kept once the check faulted, is dropped in the check before its stand-ins
    go. Only a runaway sets aside. `Fault.later` carries every later fault in order, drain hooks' included (a
    round's second failing drain hook was once lost). The six r16 repros in `fork_owns_only_its_own` (an
    engine-copied logged entry, a scroll shadow, a `[Fork]` cell, a standalone root, a root under a runaway, a
    logged handle read by the next check) fail without the fix. `fork_doom`'s `result_before_stand_in` and
    `result_not_kept_dropped_after_stand_ins` now assert the result is dropped once against its own stand-in;
    `holder`'s panicking-key tests assert every key's fault is reported. Mutants caught: "stop at the first
    fault" (V2, V3, both holder tests, r14, five repros), "a faulted result is forgotten" (both `fork_doom`
    tests), "one drain fault a round" (`r14::two_failing_drains_are_two_faults_in_one_report`).
  - S4: `Root` has a `Life` its scope or window ends; `Root::new` with neither is a debug panic. Mutant "no
    registration" caught by `a_root_a_runaway_set_aside_unmounts_with_its_scope`. The obligations lint has the
    forbidden `LastRc` class and flags a Private/Bookkeeping drop that releases through the queue.
  - S7: the value seal is a type (`Copied<T: Copy>`, `Rc<dyn Fn()>`), its trait in a private module; the
    compile-fail fixtures include a hand-written seal impl and an `Rc` of a `RefCell` holder.
  - S9 measured (decision 4), release with debug assertions (checks are compiled only with them), thread CPU
    time, Apple M4 shared with other builds (load 15 to 37), best of 7, three runs: a check's teardown of a
    1M-entry `HashMap<u64, String>` costs 27.5 to 30.5 ns an entry against a std drop's 27.5 to 34.4 (ratio
    0.89 to 1.04; 1.26 once in wall time under load 37), 2 allocations (the disposal list); 1M
    `HashMap<u64, Box<caller value>>` costs 63 to 75 ns an entry against std's 24.5 to 25.6 (2.5 to 2.9x),
    1,000,021 allocations: one box per value, about 38 to 49 ns of overhead a value. Test:
    `engine::disposal_cost` (the split counts are asserted, `Box<str>` included; the timing test is ignored).
- After review r17 (Codex 1 to 5, Opus I1 to I3), repros in `crates/gui/tests/fork_r17.rs`. On the tree before, 7
  failed (one by abort, in a subprocess); the render-panic-in-a-check and speculative-dispose ones passed before
  and stand as regressions:
  - Decision 1: `Root` holds its `Life` (a `SlabId`) and its render. `poisoned`, the root instance, `down` and
    `ended` are `RootState` in `[Fork(log)] ROOTS: ForkSlab<RootState>`. `Views` holds its key, the worker's
    shared part, the list and the holder; the request number, recipe, view shown, failure, loan, live handle count
    and released flag are `ViewsState` in `[Fork(log)] VIEWS`. The worker's sender moved into the worker's
    `Slot`. The field lint `no_gui_field_holds_engine_state_in_a_cell` lists 64 fields, each Immutable, PerWorld or
    Plumbing. Mutant "`Root.poisoned` back in a `Cell`" is caught by the lint,
    `a_check_leaves_a_poisoned_live_root_to_remount_live` and V2 (seed 0, step 4).
  - Decision 2: `#[derive(gui::ForkData)]` (`crates/gui-derive`) and `AppHandle`. `Copied`, `ForkPointee` and the
    `Rc`/`Arc`/`Option`/tuple value impls are gone. The context map, verify's `HELD` and the tooltip's closers are
    `AppHandle`s; sonner's `Store` and the toaster derive `ForkData`. Mutant "`ForkData` for `&'static T`" is caught
    by three compile-fail doctests.
  - Decision 3: `LiveDrop` and the lint above. `FocusRoot::end` ends in the running world; its `Drop` alone
    queues. `Life::end` and `Inner::end` act on the running world. Mutant "`Life::end` queues live" is caught by
    `window_owns_root_after_check_close` and the obligations lint.
  - Decision 4: `ForkValue::split`, the drain lint, `separate_logged_jobs` in a subprocess (both faults
    reported, no abort). Mutant "the map drain boxes a value whole" is caught by it (SIGABRT) and the lint.
  - Decision 5: `engine::own`/`engine::owner`. Tests: `ownerless_views_are_refused`,
    `ownerless_mailboxes_are_refused`, `mailbox::tests::an_ownerless_mailbox_is_refused`. Tests that make
    obligations at a test's top level use `engine::owned` (a detached scope of the test's own).
  - Decision 7: V2 checks ownerless constructors refused in both engines. Every run poisons the app's own root
    live, then runs a check that resolves it (remounting in its world), closes it and disposes of its scope,
    then remounts live, and ends with a speculative close followed by the live close. Each step compares the
    root's and the views' registry state.
  - The list state's split named only some fields and left the rest to the pattern's `..`, so its layout hook
    (an app closure) dropped with `parts`'s frame, outside the drain's catch. Two lists a check gave such hooks
    aborted (`fork_r17::a_check_drains_each_list_hook_on_its_own`, SIGABRT before the fix). Every audited state's
    `Parts` now names every field: list, scroll, columns, views, window work, window focus and memo. The hook and
    the other `Rc`s go through `Parts`, one box each; the engine's own containers go whole; data is let go. The
    lint `every_audited_state_takes_apart_every_field` fails on a `Parts` pattern with a rest, or one that reads
    fields off `self`. Its fixture is that list state, caught. `NodeIndex` is the one state handed on whole, as the
    engine's own index. (`Built`, earlier thought an element tree, holds only row keys.)
- Review r18 (2026-11-26). Root cause: two contracts still rested on a caller's word, the public seal of `ForkData`
  and the field lint's "PerWorld" reason. Reproducers in `crates/gui/tests/fork_r18.rs` (three failed before the
  fixes) and `fork_r17::a_render_panicking_in_a_check_leaves_the_live_root_whole` (no longer vacuous: the check
  does the first render, and its root is poisoned).
  - Decision 1: `ForkData` is sealed by a private supertrait (`engine::fork_data::Sealed`, `pub(crate)`), and the
    derive and `crates/gui-derive` are gone: no caller needed them. The toaster derives nothing; sonner keeps its
    `Store` behind an `AppHandle`. `Id` and `RootState` are declared through `fork_data_struct!`. Mutant "the seal
    made public" is caught by two compile-fail doctests (a hand impl, and one through a `use .. as` alias).
  - Decision 2: `engine::WorldCell` and `WorldRefCell`. No reason is "per world" any more: the fields that held
    one are world cells, or gated on `is_here`. Codex r18 2 (a check's tree publishing into live `Members`) is
    fixed by `LayoutTree::with_schedule` taking `detached = forking()` and the members lists as world cells; the
    visits nit by counting only in the world that made the registry.
  - Decision 3: `crates/gui/tests/cell_fields.rs`, on `syn` (shared helper `tests/common/sources.rs`). It reads
    `gui`, `primitives` and `shadcn`, resolves aliases (by file and through `use` paths), reads tuple and enum
    variants and macro bodies. The fixture `the_field_lint_sees_through_aliases_variants_and_crates` covers each
    former blind spot. The allowlist: Immutable 3 (a `OnceLock`, two memos of pure functions), Plumbing 27 (each
    names its thread), Mechanism 13 (the switch's own files only), Shared 2 (the interned decoration table's sweep
    count). Mutant "a per-world field back in a plain `Cell`" is caught by it.
  - Decisions 4 and 5: `Pipeline::drop` hands `close_window` to `engine::dropped`; `FocusRoot::drop` calls
    `cx::unmade` inside its job. The `live-drops` table above and `the_adr_names_every_drop_that_reaches_the_live_queue`
    agree. `no_owner_drop_takes_the_explicit_close` reads the owners' drops. Mutant "`Pipeline::drop` calls
    `close()`" is caught by it, by the ADR test, by `a_live_pipeline_dropped_in_a_check_retires_its_mail` and by V2.
  - Decision 6: V2 compares each window's work and `hand_on`, the mailboxes (`OPEN`, `MADE`, the boxes and bells),
    the timers and `POST`, and whether a second window takes mail. Every run has a check that closes that window
    explicitly (the live one stays open), then either a check that lets go of it (closed live after the switch
    back; the never-checking engine lets go of it live at that step) or a live whole-window close. V2 found
    `Trackers::drop` disposing its scope in the check's world: fixed, with
    `a_live_pipeline_dropped_in_a_check_ends_as_a_live_drop` (a scope and two render nodes left before the fix).
    `a_checks_first_write_costs_the_entry_not_the_table` measures the first write to one of 10 and one of 10,000
    entries of `ROOTS` and `VIEWS` in a debug build (median of 41): an empty check took 113 µs at 10 entries and
    133 µs at 10,000; the first write to a root added 68 µs at 10 entries and nothing measurable at 10,000; to a
    view, 12 µs and nothing measurable. It fails if the cost at 10,000 exceeds 8 times the cost at 10.

## Order

1. This record and the plan. 2. S6, S5 and S4 with their tests. 3. S1, S2, S3 and S9 with the tripwire. 4. The V5
slice. 5. S7, S8 and S10. 6. V1 to V4 and V6. 7. S11 with line counts. Each step's tests fail without it.

## Consequences

- A teardown fault no longer pretends to recover: the check reports `Faulted`, sampling stops on that engine, and the
  live world is whole by construction rather than by arbitration.
- Typed panic payloads do not cross the check boundary; they do cross live dispatch.
- Tests that drove forks with arbitrary closures move inside `gui` or to the `gui::testing` hook.
- List views, roots, focus roots and scopes hold their obligations by live ownership, not by reference count.

## Revisit when

- We must survive arbitrary double panics or destructive panic hooks in app code: process isolation.
- A real (non-test) caller needs a user-defined key or a typed payload across a check.
- ADR-0030 (trial) needs handlers in a fork: a separate mechanism, which must not widen this surface.
- Several concurrent readers of speculative state appear: MVCC.
