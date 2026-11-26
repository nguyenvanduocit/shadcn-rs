# 0035. An engine value ends in the world that made it

Status: proposed, 2026-11-26 (architect, after F1 review round 19). Would supersede, in
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

- a disposal of a reactive node of another world is refused and reported as a fault of the check;
- a write to a live signal is already refused (`cold_refuses`);
- a `WorldCell` write from another world is already refused.

S6's fork-region ids already tell a node's world. A pure check never disposes a live node (ADR-0028: it removes no
live scope), so the refusal changes no correct path. The `GUI_COLD_CHECK=1` suite is its acceptance.

### D4. No guard leaves a closure

- `LogCell`, `WorldRefCell` and the logged keys lend only through `with(|&T| ..)` and `with_mut(|..| ..)`. The
  public `borrow`, `borrow_mut` and `try_*` that return a `Ref` or `RefMut` are removed, so no guard exists to forget
  or to hold across a switch.
- A check entered while any such closure is active on the thread is refused (`Skipped(Borrowed)`). Production entry
  is from the frame loop with no app frame on the stack, so only the test hook can meet the refusal. The count is
  kept in builds that compile checks.

### D5. Lent engine data is immutable; world ids are process-wide

- **Chart pages.** `Page::data` becomes private. A snapshot lends its prefix of each page read-only, through a
  closure or a slice view. Pages are already append-only, and the prefix a snapshot holds is never written, so the
  type now states what the structure guarantees. The append path is unchanged.
- **The rule for every lending API:** what a snapshot or a read lends app code is `&` of data with no interior
  mutability, or an `Arc` of immutable data. The field lint's Plumbing reasons that rest on "writes need
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
  - no borrow of world state spans a switch;
  - nothing the engine lends is writable outside a store;
  - a world id is unique in the process.
- **T2, pure app code** (render and setup change app heap only through signals): no later live observation
  changes. The V2 differential is the oracle.
- **T3, impure app code** (render writes, moves or drops app-heap values other than through signals): T1 holds.
  The rest is the app's bug, as in React's and Compose's render contracts. The cold check surfaces the divergence,
  and in release (F4) a mismatch writes a record and poisons sampling. A T3 path is an important finding only if it
  breaks T1.

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
   `with_mut` through the testing hook is `Skipped(Borrowed)` and the live `ForkMap` row still reads 7.
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

## Revisit when

- V2 or a review finds a T2 leak through a plain `Drop` the lint classed Plumbing: add the sink refusal it lacked.
- F4's production records show T3 divergences that users hit: an incremental live-state fingerprint (option C)
  behind a measured budget.
- We must survive arbitrary double panics or destructive panic hooks in app code: process isolation (ADR-0034).
- A real caller needs a guard across a call that may enter a check: a scoped token type, not a returned guard.
