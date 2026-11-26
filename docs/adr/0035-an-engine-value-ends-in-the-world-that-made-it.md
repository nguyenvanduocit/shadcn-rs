# 0035. An engine value ends in the world that made it

Status: accepted, 2026-11-27 (architect, after F1b review round 4; proposed 2026-11-26 after F1 review round 19);
built (F1b, see "As built"). Supersedes, in
[ADR-0034](0034-a-check-switches-worlds-without-running-app-code.md): S4's "Only a drop reaches the live world from
a check" (the `LiveDrop` token, the `live-drops` table and its test, `no_owner_drop_takes_the_explicit_close`), the
obligations lint's `Unique`, `Private` and `Bookkeeping` classes, S6's per-thread serial, the borrow-guard API of
S7's world cells and logged keys, and the scope statement of "What a check isolates". Everything else in ADR-0034
stands: S1 to S3, S5, S8 to S11, S4's owner registration (`engine::own`, no obligation ends on its last `Rc` alone),
S7's sealed keys and value kinds. Research: `/tmp/m10/research-f1-isolation.md`.

## Context

F1 spent 19 review rounds on ADR-0034. The channels it closed by a type rule (user hooks run by the switch; values a
logged store lends) stayed closed. One channel reopened in each of the last five rounds: **a value made live is
dropped while a check runs, and its `Drop`, or a field's further down, does engine work in the check's world**,
which the switch then rolls back. The live resources leak (r19 I1: a live `LayoutTree` dropped in a check disposes
its nodes' six flag signals in the check's arena; after the switch they are alive, and `Interaction::hovered()`
stays `true`). ADR-0034 routes this per type: a drop that ends engine state builds a `LiveDrop` token and hands its
job to `engine::dropped`. Whether a type does so depends on its author remembering, and the lint can only list the
types that already do. `Root` (r15), standalone roots (r16), `Pipeline::close` (r17), `Pipeline::drop` and
`Trackers` (r18), `NodeInteraction` (r19) were each the next type a reviewer exercised.

Round 19 also opened three channels no earlier round reached, each by a capability the mechanism lent without
bounding it:

- a borrow guard of a logged piece (`LogCell::borrow`) or a world cell (`WorldRefCell::borrow{,_mut}`) that is
  forgotten or held across the switch, so the drain and rollback fail or a live write lands from inside a check;
- a chart snapshot that lends an `Arc<Page>` whose `RwLock<PageData>` is public, so a check writes live points;
- world identity from a per-thread serial, so a world cell sent to another thread matches a check there.

## Root cause

**The world is ambient, and Rust ownership events are not state.** Which store a mutation lands in is chosen by a
thread-local mode, not by the data. A `Drop` runs wherever the last owner lets go, on the one heap both worlds share,
and no versioned store can move it. Every remaining finding is a place where the ambient world differs from the world
the data belongs to, and the code used the ambient one. `WorldCell` already records its world, but uses it only to
refuse a write, never to route one.

## Criterion

**An engine value's cleanup runs in the world that made it, decided by the value, not by its type's author. Nothing
the engine lends can outlive a switch or write outside a store.**

## Options

Full assessment in the research note (§4).

- **A. Keep enumerating, close the four gaps.** Closes the three new channels; for drops it moves the enumeration
  from types to call sites (a text lint for "disposes or writes"). Rejected as the drop rule; its three local
  closures are kept (D3 to D5).
- **B. One MVCC store for all engine state.** A version walk on every hot-path read. It rewrites what K4a, K4c and L2
  build on. It does not address drops, which are not writes. Compose, the MVCC reference, has no destructors and
  still leaks escaped objects. Rejected.
- **C. Detect divergence and turn checks off.** A full live-state hash is O(state) per check. An incremental one
  costs every engine write, and nothing yet pays for it. Kept as the contract for impure apps (T3) and as the
  production backstop F4 already has (a mismatch poisons sampling). Rejected as the primary mechanism.
- **D. A cloned engine or another thread.** O(state) per check or persistent structures everywhere. On the same
  thread the app heap still holds live handles. On another thread the sources must be `Send`. Kept as M14's offline
  replay oracle. Rejected for the sampled check.
- **E. Route by the value's world** (this record). Precedents: Leptos's sandboxed owners record their arena and
  install it in their own `Drop` (`references/leptos/reactive_graph/src/owner.rs:582-600`). Its failure report, fields
  dropping after the restore (`:586`), is why the whole value moves here, not a cleanup beside it. Compose delivers
  `onForgotten` and `onAbandoned` through the owning composition, never through whoever let go. GHC's `STM` type
  cannot reach `IO`. Clojure's `io!` refuses at the effect site and defers agent sends to commit.

## Decision

Option E, with A's three local closures.

### D1. `Owned<T: Ending>`: the one engine `Drop` that does engine work

```rust
// Shape, not final names.
pub(crate) trait Ending: 'static { fn end(self); }        // engine work, run in the value's own world
pub struct Owned<T: Ending> { world: WorldId, value: Option<T> }
impl<T: Ending> Drop for Owned<T> { /* the only routing */ }
```

- `Owned::new` records the running world.
- Its `Drop` takes the value out and branches on the world:
  - **the running world:** `end` runs in place;
  - **live while a check runs:** the whole value is boxed and handed to the live release queue (S5), so `end`
    and every field's drop run after the switch back, in live, in FIFO order with other live releases;
  - **an expired check's world** (a check-made value that escaped into app heap): its memory is dropped and `end`
    never runs, because that world's state was discarded by its rollback.
- No other code chooses a world for a cleanup.
- `end` acts on the running world, which by construction is its own. It may read, decide and write, so a
  composite teardown keeps its order and atomicity: a `Pipeline`'s close, a node's flag disposal together with its
  `BOUND` row.
- Nested `Owned` values are routed independently. An outer one routed live makes the inner ones' worlds match
  when they drop.

### D2. A plain `impl Drop` does no engine work

The obligations lint keeps one list of `impl Drop` in `gui`, `primitives` and `shadcn` with three classes:

- **Guard:** a stack-scoped RAII value of engine code. It is never a field of a public type, never `Clone`, never
  behind `Rc`.
- **Plumbing:** memory, OS or other-thread work only, with the reason naming it.
- **Test.**

`Unique`, `Private`, `Bookkeeping` and `Owner` drops become `Ending` impls held in `Owned`. Today these are
`Pipeline`, `Trackers`, `LayoutTree`, `Extra`, `NodeInteraction`, `Tracked`, `Tracker`, `BoundRun`, `StyleBinding`,
`Hub`, `Wait`, `Leaving`, `Offscreen`, `Root`'s `Life`, `FocusRoot`, list views' `Inner` and `Views`,
`SignalHub`, `CloserDropped` and `FieldView`. `Planner` stays Plumbing: it stops the GPU worker threads. The
`LiveDrop` token, the `live-drops` table and its ADR test are deleted. The failure mode of a forgotten wrapper is now
a lint failure (an unlisted `Drop`), or a type whose drop does nothing, never a cleanup in the wrong world. K4a's new
instance, `each` and `show` cleanups are written as `Ending` from the start.

### D3. Sinks refuse foreign-world mutation

Second layer, for a `Drop` the lint misclasses:

- a disposal of a live reactive node of any kind that enters through `Runtime::dispose` (`Scope::dispose`, a
  signal's or effect's dispose, a window's close) is refused and reported as a fault of the check, unless it runs
  inside `reactive::retiring(|| ..)`;
- a write to a live signal is already refused (`cold_refuses`);
- a `WorldCell` write from another world is already refused.

The discriminator is the cause of a disposal, not the node's kind (D008, revised by the architect). A check does
dispose live nodes on its correct path: re-rendering a live instance retires that instance's oldest render scope
(`component::render`), and re-running a live effect or computed disposes what its last run made (`before_rerun`).
Both are the arena's own transaction. `log_node` keeps the live node whole in the undo log, its value `Rc`, its
closures and its cleanups, and gives the check a copy with shared handles and no cleanups. A cascade from a disposed
scope to its signals, effects and child scopes is logged node by node. So the check drops only `Rc` clones, runs no
live cleanup, and the rollback restores every node with every `Owned<Ending>` value or live signal it held. No Rust
value of live state ends.

A value's end is different. A `Drop` that disposes a live node takes away the only Rust owner of the node, so the
rollback restores the node with no owner left, and the node leaks. This is the r19 I1 class whatever the node's kind:
a misclassified `Drop` that disposes a live root scope leaks it as surely as one that disposes flag signals. So the
exemption is a scope the engine opens around its re-run retirement sites only. `retiring` is closure-only (D4) and
keeps a thread-local depth in the builds that compile checks. `component::render` wraps its
`each_to_completion(old, Scope::dispose)` in it. `before_rerun` and the owned cascade call `dispose_index` directly
and never meet the refusal. A further engine re-run site that the cold suite finds is added to `retiring` and named
here. A node kind is never exempted. An explicit `Pipeline::close` of a live window inside a check is a value's end
on a correct path's terms, so it faults. The `GUI_COLD_CHECK=1` suite is the acceptance, together with a seeded
mutant: a plain `Drop` that calls `Scope::dispose` on a live root scope inside a check faults.

**Revised in F1b round 2 (Codex r1-3): the exemption names its scope, not a window of time.** A `retiring` depth held
across `Root::poison`'s unmount also covered the settle that drops the cleanups a check made, so a captured app value
whose `Drop` disposed an unrelated live scope passed. Now the engine's re-run site calls `Scope::retire`, which
permits exactly that scope's node (`Runtime::retire`: a one-shot permit taken by the first `Runtime::dispose`, of that
id only). Its cascade goes through `dispose_index` and never meets the refusal; everything its settle drops, and any
disposal of another node, meets it as any other. The sites: `component::render`'s retirement of an instance's old
render scopes, and `Root::poison`'s teardown of a failed render's tree (its scopes, through
`unmount_by(id, Scope::retire)`, and those left to the resolve).

### D4. No guard leaves a closure

- `LogCell`, `WorldRefCell` and the logged keys lend only through `with(|&T| ..)` and `with_mut(|..| ..)`. The
  public `borrow`, `borrow_mut` and `try_*` that return a `Ref` or `RefMut` are removed, so no guard exists to forget
  or to hold across a switch.
- A check entered while any such closure is active on the thread is refused (`Skipped(Borrowed)`). Production entry
  is from the frame loop with no app frame on the stack, so only the test hook can meet the refusal. The count is
  kept in builds that compile checks.
- **Revised in F1b round 2 (Codex r1-1, r1-2, r1-4): engine-only admission.** Counting the world cells' lends
  alone left `Signal::update`, `WorldRefCell::get_mut` and `Page::with` open, and the next lending API would reopen
  the class. So the count is one "user code running" depth (`engine::lend`, `LENDS`), raised by every engine path
  that runs user code: a lend of world state (signals' `with`, `with_untracked`, `update`; `Computed::with`;
  `Collection::update`; world cells; logged keys) and every closure the engine calls (`turn::reach`, behind
  `call_out`: handlers, hooks, timers, effects, cleanups; `reactive::rendering`: renders and setups). Check
  admission refuses while it is above 0 (`Skipped(InUserCode)`), whatever reference the caller holds. A check thus
  opens only at the engine's own points. No public `gui` function returns a reference into a world cell, a logged
  piece or a page (`WorldRefCell::get_mut` is engine-only, `Page::with` is gone: app code reads points by copy,
  `SeriesSnapshot::get`). The lint `obligations::every_lend_to_user_code_is_counted` fails on a public function that
  runs a caller's closure on a reference uncounted, or returns such a reference, unless listed with why (an engine
  piece's lend, refused as a check begins; a plain value the caller owns).
- **Revised in F1b round 3 (Codex r2, D011): what admission guarantees, and how.** Every round-2 bypass began with
  user code opening a check through the raw test hook with a body that borrowed what a lend had handed it. Opening
  a check from user code cannot be ruled out by construction while `Pipeline` and `Scenario` are public (a handler or
  a `Drop` can build one and drive a frame), so the guarantee is not "no user frame is on the stack". It is two
  rules, both needed:
  1. **By type: every check body is engine code or `'static`.** The engine's own bodies (a frame's cold build) run
     renders, setups and effects, which are `'static`. The one hook another crate may call,
     `gui::testing::in_check` (debug builds only, `doc(hidden)`, for per-thread-state tests in `primitives` and
     `shadcn`), takes `f: impl FnOnce() -> R + 'static` with `R: 'static`, as does its `Result` form
     `gui::testing::check`. Any hook that takes a borrowing body would be `#[cfg(test)] pub(crate)`; as built, none
     is left. A `'static` body holds no `&T`, `&mut T` or guard,
     so no lent borrow is carried into a check, however user code was reached (closure, trait method, `Drop`,
     scoped thread). The round-2 repros are compile-fail doctests on the public hook.
  2. **At run time: admission refuses under a counted lend or an engine piece in use.** No guard of world state
     exists (D4 above) and every lend counts in `LENDS`, so a check driven from inside a lend (a second pipeline's
     frame in a handler) is refused (`Skipped(InUserCode)`), and one driven while engine code holds a piece is
     refused naming the piece. This is a rule, not only a backstop: it is the only thing that refuses a nested
     engine frame. It is compiled wherever checks are, so F2's release checks switch it on with `is_cold`.
     **Revised in F1b round 4 (Opus r3 P1, P2): one gate, not a list of call sites.** A computed's getter ran
     uncounted (the runtime's `update`), and layout bindings and paint closures run through the same tracked-run
     path, so counting at each call site had missed them. The count is now held by the guards the reactive runtime
     switches its running subscriber or owner with, which every engine call into reactive user code passes through:
     `RunGuard` (a computed's getter, an effect's body, a tracked render, binding or paint, `track_render`),
     `OwnerGuard` (`untracked`, `detached`, `Scope::run`, `reactive::run_owned`) and `trigger`'s `RestoreSub`. Each holds an
     `engine::UserCode`, the one counter type; `engine::lend` holds one too, for lends of world state and the
     comparisons `set` runs. `rendering` and `turn::reach` keep their own counts, for the work that does not switch
     the subscriber (setups, handlers, timers). The lint `obligations::every_switch_into_user_code_is_counted`
     fails on any guard whose `Drop` restores the running subscriber or owner and that holds no `UserCode`; the lend
     lint (`every_lend_to_user_code_is_counted`) still covers public closure lends. A check opened inside a check's
     body is refused too (`Skipped(InCheck)`, `frame::NESTED`): before, it panicked the outer check with "a cold
     frame inside a cold frame".
  `LENDS` stays per thread. A world is per thread, a body is `'static`, and world ids are process-wide (D5), so a
  check on another thread reaches neither this thread's world nor, through its body or the engine, a borrow lent on
  it. (A `Send` payload borrow can still reach that thread inside foreign work its body runs: D6, "A check owns what
  its body runs", D016, classes that T3.) Refusing every check off
  one UI thread was rejected: each test thread runs its own engine. Code that runs directly in a test's `act` or
  `read` is the test's top level, not a lend: it holds no borrow of world state unless it is inside a counted lend.

### D5. Lent engine data is immutable; world ids are process-wide

- **Chart pages.** `Page::data` becomes private. A snapshot lends its prefix of each page read-only, through a
  closure or a slice view. Pages are already append-only, and the prefix a snapshot holds is never written, so the
  type now states what the structure guarantees. The append path is unchanged. (As built, round 2: no page is lent
  to app code at all. A closure lend held the series' lock across user code, which deadlocks a `push` of the same
  series, and lent the points appended after the snapshot too. App code reads a snapshot's points by copy,
  `SeriesSnapshot::get`, up to its length.)
- **The rule for every lending API:** what a snapshot or a read lends app code is `&` of data with no interior
  mutability, or an `Arc` of immutable data, as far as the engine chooses the type. The inside of an app-chosen
  value type (a `Collection` row, a signal's value) is app heap (D6, round 3). The field lint's Plumbing reasons that rest on "writes need
  `&mut`" are re-checked against public reachability.
- **World ids.** A check's world id comes from one process-wide `AtomicU64`. Live stays 0. A world cell sent to
  another thread therefore never matches a check there. `world_cell::now()` is `engine::world()` in every build: one
  source, so F2's release checks cannot leave the cells' refusal at a constant 0 (Opus r19 P1).

### D6. What a check guarantees, in three tiers

ADR-0034's "What a check isolates" is restated as a threat model. A review finding states its tier.

- **T1, any code, by construction:**
  - memory safety;
  - the switch runs no app code;
  - an engine value's cleanup runs in the world that made it, or not at all if that world expired;
  - no borrow of the switching thread's world, held by the check's body or the engine, spans that switch;
  - nothing the engine lends is writable outside a store;
  - a world id is unique in the process.
- **T2, pure app code** (render and setup change app heap only through signals): no later live observation
  changes. The V2 differential is the oracle.
- **T3, impure app code** (render writes, moves or drops app-heap values other than through signals): T1 holds.
  The rest is the app's bug, as in React's and Compose's render contracts. The cold check surfaces the divergence,
  and in release (F4) a mismatch writes a record and poisons sampling. A T3 path is an important finding only if it
  breaks T1.
- **App-chosen interior mutability is app heap (F1b round 3, D010).** A store lends `&T` of an app-chosen `T`
  (a `Collection` snapshot's rows, which share an `Arc` tree with live, a signal's or computed's value, a world
  cell's app payload). A write through a `Cell`, `RefCell` or similar inside `T` is a write of app heap, not of a
  store: T3, as ADR-0034's purity contract already classes any render that writes app heap other than through
  signals. T1's "nothing the engine lends is writable outside a store" covers the data the engine chooses (its
  stores' slots and versions, `WorldRefCell` and `LogCell` state, chart pages), never the inside of `T`. T2 is
  unchanged: a pure render writes no row. A lazily filled cache of a pure function of the row (a `OnceCell` memo,
  ADR-0034's Immutable class) changes no observation and stays within T2. Rejected: a sealed no-interior-mutability
  bound on rows (not expressible for "no `Cell`" in stable Rust, and it would refuse every row holding a handle) and
  copy-on-check (O(rows) a check on million-row collections, against M12's list budget, for a case only impure code
  reaches). Revisit if a T2 path writes a row, or the oracle finds a divergence from one.
- **A check owns what its body runs (F1b round 3, D016).** A switch is per thread: it swaps the running thread's
  world and nothing else. Work another thread submitted that a cooperative scheduler runs on a check's stack (a
  `rayon::yield_local` or a steal, a `block_on` that polls another executor's tasks) runs in that check's ambient
  world: `forking()` is true for it, `world()` is the check's, a world cell it makes is the check's, a mailbox send it
  makes goes with the fork. Running foreign work is the body's impurity: T3, with T1 holding. Codex's F1b r3
  reproducer (`scoped_worker`: thread A lends a payload `&mut` through `with_mut`, a Rayon job carries it to worker B,
  and B's `'static` check body runs the job through `yield_local`) keeps T1:
  - the write is A's live code writing the payload A's own live lend handed it (a world cell's payload is app heap,
    D010), on a borrow of A's world that no switch on B swaps, exclusive and valid throughout by Rust's rules;
  - by counterfactual the check changed nothing: `rayon::scope` runs the job with or without the check, so live
    ends at 9 either way. T1 promises the live state a check leaves equals the live state without it, not that
    code running during a check writes no live memory.
  A job that branches on `forking()` or sends on a mailbox from B's stack does diverge; that is the T3 class above,
  surfaced by the cold check as any impure body. No engine check meets it: production checks open only on the UI
  thread at engine points, and the engine's one Rayon use (`layout::engine::pool`) is an `install` into gui's private
  pool from outside it, which blocks and never runs foreign jobs on the caller. Rejected: a process-wide lend count
  (closes this repro only, as a job carrying an `Arc` or a `Sender` needs no lend, and makes parallel test threads
  refuse each other's checks nondeterministically); a `!Send` lend wrapper (a reborrow `&mut *x` of a `Send` payload
  escapes it); an engine scheduler rule (gui cannot govern an app's executors). Revisit if an engine check ever opens
  on a pool worker or an executor thread, or a `Send` engine handle writes live engine-chosen state without
  consulting the world it was made in.
- **T1's carve-out, a runaway:** a check whose disposal runs away (ADR-0034, S2) sets aside, never drops, what it
  still holds. A live value among it (a live `Ends` a check moved into its own state) is then never ended: memory and
  T1 hold, the live state is as the check found it, but that value's end is lost. The fault says so, and sampling is
  off on the thread from then on. Ending such values live is a long-term item (filed against F1b, M11).

## Verification

- The five r19 reproducers become regression tests, each failing on the tree before (see "Acceptance").
- V2 gains a rich pipeline: interactive nodes hovered and focused, a list with a worker and a waiting view, a
  mailbox and a chart series. It is dropped inside a check in one engine and live in the other, and compares signal
  counts, `BOUND`, `hovered()`, worker closure, mail delivery and every id cursor.
- Seeded mutants, each caught:
  - `Owned` routes by the running world instead of its own;
  - the expired-world branch runs `end`;
  - the borrow count is never checked;
  - world ids are per-thread again;
  - `Page::data` is public again (compile-fail);
  - an `Ending` turned back into a plain `Drop` (the lint).
- Measured with performance-dx-lead before acceptance:
  - teardown of a 10,000-node interactive tree, live and inside a check, against the tree before;
  - steady-frame allocations unchanged in the stress scenes;
  - `bench.sh` like for like.
  `Owned` adds one world id per value and one thread-local read and compare per drop. Teardown only, never a
  steady frame.

## Acceptance (the F1b task)

1. `a_live_tree_dropped_in_a_check_ends_its_flags_as_a_live_drop` and
   `a_live_window_dropped_in_a_check_leaves_no_hover_live` (Opus r19 I1): leftover signals equal the live drop,
   `hovered()` is `false`.
2. Codex r19-1: a compile-fail doctest that `LogCell` lends no guard. A runtime test that a check entered inside
   `with_mut` through the testing hook is `Skipped(Borrowed)` (as built, `Skipped(InUserCode)`, D4's round-2
   revision) and the live `ForkMap` row still reads 7.
3. Codex r19-2: a compile-fail doctest that `Page::data` is unreachable outside `gui`. A test that a snapshot's
   prefix reads the same after a live append, and a check's read of it leaves live `2.0`.
4. Codex r19-3: a compile-fail doctest that `WorldRefCell` returns no `Ref` or `RefMut`. A test that a check
   entered inside a live `with_mut` is refused and `window/decoration.rs`'s action list gets no live `Close`.
5. Codex r19-4: a test that a world cell made in one thread's check and sent to another thread's check is
   refused, and that two threads' first check ids differ.
6. The NIT: a check-made `Wait` that escaped into app heap and is dropped live neither panics nor touches live.
7. The lint: no `Unique`, `Private`, `Bookkeeping` or `Owner` class is left, and the fixture catches an
   engine-effecting plain `Drop`.
8. `GUI_COLD_CHECK=1 cargo test --workspace --locked` passes with D3's refusal on, as do clippy, fmt and the
   existing F1 suites (V1 to V5, `fork_r17`, `fork_r18`, `fork_owns_only_its_own`).

## Consequences

- ADR-0034's per-type routing, its table and the review loop it fed are gone. A new engine type with a cleanup
  states `Ending`, and the world it ends in is not its author's decision.
- The world-cell and logged-key APIs become closure-only. About 108 call sites in `gui`, `primitives` and `shadcn`
  move mechanically.
- Reviews of the check judge findings by tier. A T3 path that keeps T1 is filed as a note, not an important
  finding.

## As built (F1b, 2026-11-26)

- **D1.** `Owned` is `engine::Ends<T: Ending>` (`crates/gui/src/engine/ends.rs`); `route` is the one routing. Held in
  it: layout's tracking nodes (`binding::Node`, for `Tracked`, `Tracker`, `BoundRun`, `StyleBinding`, a hidden node
  and `LayoutTree`'s probe), `NodeInteraction`, `Hub`'s node, `Waiting` (a `Wait`), `HubEntry` (`SignalHub`),
  `TrackersScope`, `Closing` (`Pipeline`'s close, its first field), `WindowEnd` (`FocusRoot`), `RootEnd` (`Root`),
  `Scopes` (`Leaving`), and the list views' `Gone` and `Handle`. `Handle` holds the views' `Rc<Inner>`: V2's rich
  pipeline found a live handle a check let go queued without the views, so their wake died in the check and a
  waiting view never asked the list again; moved whole, it ends as live code's drop does. `LiveDrop`,
  `engine::dropped`, `made_in_fork` routing and the live-drops table test are deleted. `Offscreen` and `Planner`
  stay Plumbing; `FieldView` and `CloserDropped` are test-only (`Test`).
- **D2.** `crates/gui/tests/obligations.rs`: classes Guard, Plumbing, Test; a Plumbing body naming engine work fails
  (`ENGINE_WORK`); only `engine/release.rs` and `engine/ends.rs` call `when_live`; the fixture catches an
  engine-effecting plain `Drop` listed Plumbing and an unlisted one.
- **D3.** `Runtime::dispose` (`crates/gui/src/reactive/runtime.rs`) refuses, in a cold frame, the disposal of any
  live node but the one `Runtime::retire` permits, reported as `Phase::Foreign`. `Scope::retire` is called at two
  engine sites in `crates/gui/src/component.rs`: `render`'s retirement of an instance's old render scopes, and
  `Root::poison`'s teardown of a failed render's tree (the root keeps its owner and mounts again at a later resolve;
  the cold suite found it: `fork_r17::a_render_panicking_in_a_check_leaves_the_live_root_whole`). An app value its
  settle drops cannot dispose a live node (`fork_f1b2::retirement`). Explicit `Pipeline::close`,
  `Root::close` and `Scope::dispose` of live nodes in a check fault (V2 `CloseWindowChecked`, `CloseRootChecked`;
  `fork_r17` and `fork_r19::a_plain_drop_disposing_a_live_root_scope_in_a_check_faults`).
- **D4.** `WorldRefCell` and `LogCell` lend through `with`, `with_mut`, `try_with`, `try_with_mut` only (compile-fail
  doctests in `engine/world_cell.rs` and `engine/log_key.rs`); every path that runs user code counts in `LENDS`
  (`engine::lend`, round 2's revision above), and `fork` returns `Dropped(BORROWED)` while it is above 0
  (`Skipped(…InUserCode)` through `testing::check`; `fork_f1b2`). A piece in use otherwise is refused as before,
  naming it. Round 3 (D011): both public hooks, `testing::in_check` and its `Result` form `testing::check`
  (`crates/gui/src/testing.rs`), take a `'static` body and `'static` result (no hook with a borrowing body is
  left, so none needed `#[cfg(test)] pub(crate)`, and the integration suites stay where they are, their bodies
  moved over cloned handles). The round-2 reproducers that borrowed are `compile_fail` doctests on the hooks
  (`Debug`, scoped thread, `Collection` row); `crates/gui/tests/fork_f1b3.rs` keeps each in its reachable
  `'static` form. `obligations::a_check_opens_only_at_an_engine_point_or_on_a_static_body` fails on a new public
  function that opens a check, or a hook whose body is not `'static`. `Signal::set`, the signal's deferred
  `changed` and a computed's update compare inside `engine::lend`, and the old value's drop runs after the slot is
  let go; `WorldRefCell` and `Page` format without the payload. `LENDS` and `fork` are both compiled in debug builds
  only today, so the run-time rule is compiled wherever checks are.
- **D4, round 4 (the one gate).** `engine::UserCode` (`crates/gui/src/engine.rs`) is the one counter of `LENDS`;
  `engine::lend` holds one. In `crates/gui/src/reactive/runtime.rs`, `RunGuard`, `OwnerGuard` and `trigger`'s
  `RestoreSub` each hold one, so a computed's getter, an effect, a tracked render, binding or paint, `untracked`,
  `detached`, `Scope::run` and `reactive::run_owned` are counted at the switch, not per call site; `rendering` and
  `turn::reach` count through `engine::lend`. `frame::fork` refuses, in order: a poisoned thread, `BORROWED`
  (`Skipped(InUserCode)`), then `NESTED` (`Skipped(InCheck)`) when a check already runs on the thread. Lints in
  `crates/gui/tests/obligations.rs`: `every_switch_into_user_code_is_counted` (any `gui` guard whose `Drop`
  restores the running subscriber or owner without a `UserCode`; its fixture
  `the_gate_lint_catches_a_switch_without_a_count`), `every_lend_to_user_code_is_counted`,
  `a_check_opens_only_at_an_engine_point_or_on_a_static_body` and `every_drop_is_classified_and_none_does_engine_work`.
  Tests: `crates/gui/tests/fork_f1b3.rs` (`a_check_inside_a_live_getter`, `a_check_inside_a_check`, `equality_lend`,
  `debug_lend`, `scoped_worker` kept as D016's T3-with-T1 case), `fork_f1b2.rs` (`retirement`, `signal_lend`,
  `action_lend`, `a_check_inside_user_code_is_refused`), `fork_r19.rs` (the five r19 reproducers and the D3 mutant),
  V2's rich pipeline, `engine::ends::tests`, and `tests/teardown_cost.rs`.
- **D5.** `Page::data` is private and no page is lent to app code (compile-fail doctests on `Page`); world ids come
  from the process-wide `WORLDS`, `engine::world()` in every build (`const LIVE` in release).
- **For F2:** the D3 refusal, `Runtime::retire`'s permit, `LENDS` and `WORLDS` are compiled in debug builds only
  today, as `is_cold` is. F2's release checks must switch them on together with `is_cold`, or a release check
  meets none of them. **F2's gate (performance-dx-lead, D019):** with `LENDS`, `WORLDS` and the D3 refusal on in
  release, `bench.sh` like for like adds 0 steady allocations and keeps instructions within B0's tolerance (8% +
  3,000); stress steady allocations are unchanged in every scene; `tests/teardown_cost.rs` stays at ≤70,001
  allocations live and ≤90,041 in a check, with 0 leaked per node. Today `UserCode` is zero-sized and its count is
  compiled in debug builds only, so the release profile pays nothing.
- **Mutants**, each run alone and reverted (isolated copy of the tree):

  | Mutant | Caught by |
  |---|---|
  | `Ends` routes by the running world | `fork_r19`: `a_live_tree_…`, `a_live_window_…` |
  | the expired branch runs `end` | `engine::ends::tests::a_value_ends_in_the_world_that_made_it`, `value::source::tests::a_checks_wait_dropped_live_neither_panics_nor_touches_live` |
  | the borrow count never checked | `fork_r19`: `a_check_inside_a_logged_lend_is_refused`, `check_cannot_queue_a_live_window_close` |
  | world ids per thread | `fork_r19`: `two_threads_first_checks_differ`, `distinct_threads_checks_have_distinct_worlds` |
  | `Page::data` public | the compile-fail doctest on `chunk::series::Page` |
  | `NodeInteraction`'s `Ending` back to a plain `Drop` | `obligations::every_drop_is_classified_and_none_does_engine_work` |
  | D3 exempts `Scope` nodes | `fork_r19::a_plain_drop_disposing_a_live_root_scope_in_a_check_faults` |
  | a views `Handle` without its views | V2 `a_rich_window_dropped_in_a_check_ends_as_a_live_drop` |
  | `Signal::update` runs its closure uncounted (round 2) | `fork_f1b2::signal_lend`, `obligations::every_lend_to_user_code_is_counted` |
  | `turn::reach` runs user code uncounted (round 2) | `fork_f1b2::a_check_inside_user_code_is_refused` |
  | the retirement permit held across the settle (round 1's window, round 2) | `fork_f1b2::retirement` |
  | `Signal::set` compares outside the lend (round 3) | `fork_f1b3::equality_lend` |
  | `WorldRefCell`'s `Debug` formats its payload (round 3) | `fork_f1b3::debug_lend` |
  | `Page`'s `Debug` formats its data under the lock (round 3) | `fork_f1b3::page_formatter_lend` |
  | `testing::check`'s body not `'static` (round 3) | the `compile_fail` doctest on `testing::check`, `obligations::a_check_opens_only_at_an_engine_point_or_on_a_static_body` |
  | admission never reads `LENDS` (round 3) | `fork_f1b2`: `action_lend`, `signal_lend`, `a_check_inside_user_code_is_refused` |
  | a computed's getter `RunGuard` uncounted (round 4) | `fork_f1b3::a_check_inside_a_live_getter` |
  | `OwnerGuard` uncounted (round 4) | `fork_f1b3::a_check_inside_a_live_getter` (`untracked`, `Scope::run`) |
  | `fork` admits a check inside a check (round 4) | `fork_f1b3::a_check_inside_a_check` ("a cold frame inside a cold frame") |

- **Measured** on one machine (Apple Silicon, macOS; shared with other agents' builds, so times are noisy and
  allocations are the firm figures), the tree before = checkpoint e7c9e49 built alone, after = this tree:

  | Workload | Before | After |
  |---|---|---|
  | teardown, 10,000 interactive leaves dropped live (`tests/teardown_cost.rs`, release + debug assertions, best of 7) | 29.0 ms, 80,002 allocations | 26.2 ms, 70,001 allocations |
  | the same dropped inside a check, ended live after | 55.1 ms, 150,100 allocations, 420,006 signals leaked | 30.2 ms, 90,041 allocations, none leaked |
  | stress (`--run --cycles=0 --monkey=0`), steady allocations per frame, every scene | p50 8–15 | identical in every scene; costliest actions within ±51 allocations |
  | `bench.sh --frames=10 button card dialog table`, allocations a frame | 18, 9, 12, 10 | 18, 9, 12, 10 (same bytes and instructions) |

  Round 3 re-run of the teardown row (same command, load average about 37): live 19.8 ms, 70,001 allocations;
  in a check then live 21.6 ms, 90,041 allocations. The allocations are unchanged from round 1, and 6 engine
  signals stay alive after the run either way, with none per node. Round 3 adds a lend around `PartialEq` on
  `Signal::set` and a computed's update: one thread-local increment and decrement per compare, in debug builds
  only.

  Stress fails `toasts` (2,102 allocations against a 2,100 budget) and `dashboard` (a script hit test) on both
  trees; bench is SLOWER than the committed B0 baseline on both trees alike. Neither is this change's.

## Revisit when

- V2 or a review finds a T2 leak through a plain `Drop` the lint classed Plumbing: add the sink refusal it lacked.
- F4's production records show T3 divergences that users hit: an incremental live-state fingerprint (option C)
  behind a measured budget.
- We must survive arbitrary double panics or destructive panic hooks in app code: process isolation (ADR-0034).
- A real caller needs a guard across a call that may enter a check: a scoped token type, not a returned guard.
