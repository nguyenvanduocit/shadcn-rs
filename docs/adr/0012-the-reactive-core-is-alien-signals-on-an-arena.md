# 0012. The reactive core is alien-signals on an arena

Status: accepted, 2026-11-21; built 2026-11-21 (M1, see "As built"). Supersedes ADR-0008's "an effect depends on what its last run read"
(a source records its readers, a reader forgets them before it runs again and when dropped); keeps the rest of
ADR-0008.

## Context

`gui`'s reactive core (`reactive.rs`, `computed.rs`) was written to make a whole-app rebuild cheap to trigger, not to
redo only what changed. Six tests written to Vue's guarantees fail on it:

| # | Case | `gui` today | Vue |
| --- | --- | --- | --- |
| 1 | In `batch`, `s.set(2)` then `c.get()` with `c = s * 10` | `10`, stale | `20` |
| 2 | An effect reads `s` and `c = s * 10` (a diamond) | runs twice, sees `(2, 10)` | once, sees `(2, 20)` |
| 3 | `parity = s % 2`, `s` goes 1 → 3 | the effect after it runs again | skipped |
| 4 | `s.set(1)` when `s` is 1 | the effect runs again | nothing |
| 5 | An effect writes a signal it reads | recurses 51 deep; without a stop, overflows | runs once |
| 6 | e1 writes `b`, e2 reads `b` | `e1 start, e2, e1 end`: nested flush | e1 ends, then e2 |

The causes:

- **Bugs 1 and 2.** A computed is marked dirty only when its bridge runs in `flush()` (`computed.rs:95-113`).
  Subscribers sit in a `HashSet`, so an effect may run before the bridge and read the old value, in an order that
  changes from run to run.
- **Bug 3.** `Computed<T>` demands `T: PartialEq` and never compares.
- **Bug 4.** `Signal::set` never compares, and every write rebuilds the app.
- **Bugs 5 and 6.** `end_batch` drops the depth to 0 before it flushes (`reactive.rs:184-190`), and there is no
  running flag, so a write inside an effect starts a nested flush.

Panics also leave the runtime broken. A panic inside `batch` never restores the depth (`reactive.rs:547-551`), so
effects never run again on that thread. A panic inside an effect never restores `ACTIVE` (`reactive.rs:216-219`), so
later reads are tracked by the wrong reader. `Signal::labeled` runs a `retain` over every probe
(`reactive.rs:243-248`), which makes N labelled signals O(N²); there are 172 of them.

Handles are `Rc`. An effect lives only while someone holds it, so components keep effects alive in hand-written
fields (`NumberFieldState::_apply_on_blur`, `number_field.rs:22`; `let _keep = &open_on_search;`, `tags_input.rs:48`).
Forgetting the field silently drops the behaviour.

## Options

1. **Patch the current core.** Add comparison, a running flag, ordered subscribers and panic guards. The model stays:
   a computed refreshes through a bridge effect, and handles are reference-counted.
2. **Vue 3.5's core** (`references/vuejs/packages/reactivity/src`). Each dep keeps a version, and there is a global
   version. Links sit in two doubly-linked lists and are reused in read order (`dep.ts:32-165`). A write pushes
   dirty marks; a read pulls through `isDirty`/`refreshComputed` (`effect.ts:353-430`). The check recurses along the
   dependency chain, so a long chain overflows the stack; Leptos has the same flaw
   (`references/leptos`, `computed/inner.rs:106-124`).
3. **alien-signals, as Vue 3.6 adopted it** (`references/vue-minor/packages/reactivity/src/system.ts`, 262 lines;
   `references/alien-signals`). Six flags (`Mutable`, `Watching`, `RecursedCheck`, `Recursed`, `Dirty`, `Pending`)
   and a cycle stamp per link replace versions. Propagation and the dirty check are iterative, with an explicit
   stack. Links are reused with no allocation when a run reads in the same order. A computed drops its
   subscriptions when nothing watches it. When effects are queued, an owner goes before what it owns.
4. **The `alien-signals` crate** (`references/alien-signals-rs`, 0.1.4). It follows `system.ts` line by line and
   ports the spec tests, but:
   - its graph is one global `static ARENA` declared `unsafe impl Sync` (`src/node.rs:67-74`), so the repo itself
     sets `RUST_TEST_THREADS = "1"`;
   - its arena never frees nodes or links;
   - its handles are raw `NonNull` with no generation;
   - it has 63 `unsafe` blocks;
   - it has no scope tree, `watch`, `on_cleanup` or panic safety.

## Decision

Option 3, written in safe Rust. The crate (option 4) is read for its algorithm and its tests, and is not a dependency.

**The data.**

```rust
struct NodeId { ix: u32, gen: u32 }                       // Copy; gen detects a disposed node
struct Node { gen, kind /* Signal | Computed | Effect | Render | Collection */, flags,
              deps, deps_tail, subs, subs_tail, owner, created_at: &'static Location, label }
struct Link { dep, sub, cycle, prev_dep, next_dep, prev_sub, next_sub }
struct Runtime { nodes: Vec<Node>, links: Vec<Link>, free_nodes, free_links,
                 values: Vec<Option<Box<dyn Any>>>, active, cycle, batch_depth, flushing, queue }
thread_local! { static RT: RefCell<Runtime> }
```

The runtime is a `thread_local` arena with `u32` indices, free lists for nodes and links, and a generation per slot.
There is no `unsafe`. Every access goes through `.get()`, as the `indexing_slicing` lint requires. **Borrow rule:**
graph operations (track, propagate, unlink, flag checks) never call user code. User code runs only after the
runtime's borrow is released: effect bodies, a computed's function, cleanups, equality.

**The algorithm.** It follows `system.ts`.

- `track` walks `deps_tail` in read order. It reuses the next link when that link points at the same dep, and
  otherwise inserts one. At the end of a run, links after the tail are removed.
- `set` stops on an equal value. Otherwise it writes and propagates iteratively: computeds take
  `Pending`/`Dirty` and pass it on, and each effect is queued once.
- `flush` runs only when the outermost batch ends and no flush is running. The queue runs owner first. Each effect
  runs an iterative `check_dirty`, pulling computeds, before it runs. Reruns are capped.
- A computed counts as changed only when its new value differs from the old one.

**Ownership.** `Signal`, `Computed`, `Effect` and `Collection` handles are `Copy`.

- What owns them is a `Scope` tree. Disposing a scope disposes everything created in it, children first, and runs
  its cleanups.
- A component's scope is its instance (ADR-0013).
- Using a disposed handle is a reported error naming the `#[track_caller]` location where the handle was created,
  as Dioxus' generational-box does.
- RAII on handles is gone. A `Copy` type has no `Drop`.

**Invariants, each with a regression test.**

1. Glitch-free: an effect never sees old and new values mixed, and a diamond runs once.
2. Reads inside `batch` are fresh.
3. Equal values stop propagation, at signals and at computeds.
4. No self-recursion, no nested flush, a recursion limit reported rather than overflowing.
5. Deterministic order: an owner runs before what it owns, and siblings run in subscription order.
6. Panic safety: guards restore `batch_depth`, `active` and `flushing`; the flush runs the rest of the queue, then
   rethrows the first panic.

**The API.**

```rust
Signal::new(v) Signal::labeled(label, v)
.get() .with(|&T| ..) .get_untracked() .with_untracked(..) .set(v) .replace(v) .update(|&mut T| ..) .trigger()
Computed::new(f)  Effect::new(f)  on_cleanup(f)
watch(source, |new, old| ..) watch_immediate(..)  batch(f)  untracked(f)
Scope::new() scope.run(f) scope.dispose()
```

- `with` borrows without cloning.
- `update` triggers only after its `borrow_mut` is released, so no effect runs while a value is borrowed.
- A label lives on the node, so labelling is O(1).
- Async stays outside the graph: a status signal fed through `mailbox`.

**A collection is one source, not one signal per item.** `Collection<T>` holds two things:

- an immutable, structurally shared snapshot;
- a change log of `Splice`, `Update`, `Move` and `Reset` entries tagged with versions.

A reader subscribes to the collection once and reads the changes since its last version. Views (sort, filter) are
permutations built on a worker and delivered through `mailbox` tagged with the source version. A list of a million
rows then costs one node in the graph (`research_notes/Hiệu năng cực hạn/million_lists.md`).

**Tests.**

- The alien-signals spec suite is ported (`references/alien-signals-rs/tests/*_spec.rs`).
- So are the 29 cases of `docs/research-2026-11-21.md` §3.6, among them:
  - a 100 000-deep computed chain with no stack overflow;
  - 10 000 labelled signals created in linear time;
  - a panic inside `batch`, an effect and a computed;
  - use after dispose;
  - an owner running before its child.
- A seeded property test builds random graphs and random writes and compares every value with an oracle that
  recomputes everything.

## Consequences

Built as expected, with the differences in "As built" below.

- The six bugs and the three panic and labelling bugs are fixed by construction, and each keeps its test.
- An equal write no longer triggers anything:
  - `set` needs `T: PartialEq`. A write to a type without it (`MessageScrollerState`, `Turn`, `Msg`,
    `FormField`) uses `replace`, which always notifies; the choice is made where the write is, not when the signal
    is made.
  - Code that writes only to force a refresh moves to `trigger()`.
  - Floats compare by IEEE rules, so NaN always triggers.
- **Call sites change in the same change** (no backward compatibility):
  - The 92 `.update(` calls take `&mut T`; about 30 `|mut all| { …; all }` bodies lose their clone.
  - The 9 `Effect::new` calls stay effects (each reacts to focus or several sources, which `watch` does not
    model better); the fields that kept them alive are gone.
  - `take_changed` stays until ADR-0013's scheduler makes render a subscriber (M2).
  - devtools `graph`, `probes` and `write_probe` read the arena.
- **Hand-kept effect fields disappear.** An effect lives as long as the scope it was created in.
- `reactive::graph()` and `Footprint`'s `Graph` count arena slots and free-list lengths, so a leak shows as a
  growing slot count.
- **Known wrinkle:** an effect that writes its own dependency does not rerun at once (as in Vue). This is documented
  on the API.

## As built

`crates/gui/src/reactive/` (`runtime.rs` the graph and algorithm, `signal.rs`, `computed.rs`, `effect.rs`,
`scope.rs`, `collection.rs`, `trace.rs` for the devtools, `tests.rs`), `crates/gui/src/sum_tree.rs`.

- **A signal keeps its value from before its first write** since it was last checked, as alien-signals keeps
  `currentValue` and `pendingValue`: a value written and written back within one batch wakes nothing. The seeded
  comparison with the oracle found this (seed 4, step 12, numbered from 1 then; `SEED=3` now); it has its own test.
- **The flush goes on past a panicking effect** and raises the first panic again at its end. While a panic
  unwinds, no guard runs user code (a second panic would abort); queued effects and cleanups run at the next flush.
- **`with` and `update` batch**: what `f` writes runs its effects after the value is released, never while it is
  borrowed (§3.6 case 24).
- **`trigger(f)`** (alien-signals' `trigger`) runs `f` through a temporary reader and wakes what depends on what it
  read, in one batch; `Signal::trigger` does it for one signal.
- **`detached(f)`** makes state with no owner, for globals made lazily from wherever they are first touched
  (`focus.rs`'s focus signals).
- **Bridge until M2.** The app's component is still a closure rebuilt on `take_changed`. Each build pass runs in
  its own root scope, and the scopes of the last two passes are kept (the tree on screen and the one a leaving node
  is found in), so state made during a render lives as long as it did when handles were `Rc`. State a render makes
  lazily and keeps (the sidebar's tooltips, a markdown reply's copy buttons, a page made the first time it shows)
  is made in a scope owned by the state that keeps it.
- **A mailbox made in a scope closes with it**, and `deliver` runs every handler in one batch.
- **`Collection<T>`** is one signal holding a [`SumTree`](../../crates/gui/src/sum_tree.rs) (a persistent B+ tree,
  `Arc` nodes so a snapshot can go to a worker, O(1) clone, O(log n) edit) and a log of the last 4 096 changes.
  `apply_changes` replays the log onto a reader's copy. The tree carries summaries, so the height tree of M4's
  lists seeks offset to row with it. `imbl` was not taken: its MPL-2.0 licence is not on `deny.toml`'s list.

Measured (`cargo test -p gui`; release for the last two):

| Goal | Result |
| --- | --- |
| Six invariants, six bugs, §3.6's 29 cases | covered by tests (case 8 by the oracle, 12 by link reuse), all pass |
| alien-signals specs (`effect`, `effectScope`, `trigger`; `build` tests JS packaging, `conformance` an npm suite) | ported, pass |
| Seeded graphs against a recompute-everything oracle | 10 000 graphs × 60 steps pass (`ITERATIONS=10000`; a local run does 1000, CI 10 000; `SEED=n ITERATIONS=1` repeats one; the effects' read sets change and some are disposed mid-sequence) |
| Steady updates allocate nothing | 0 allocations over 19 980 updates of a graph with 66 effects (`tests/reactive_alloc.rs`, counting allocator) |
| A million-deep computed chain | updates and unwinds without recursion, 0.27 s (`--ignored`) |
| 10 000 labelled signals | linear (40 000 take under 10× of 10 000) |
| Use after dispose | reported with the creation site |
| `docs --bench`, 85 pages; stress, 10 scenes; monkey sweep of 85 pages | all run; no page or scene fails on the reactive core |

## Revisit when

- The `propagate` bench (shared with `references/alien-signals-rs`) shows the arena slower than the crate's.
- Vue or alien-signals change the algorithm after 3.6 is stable, for example in ordering or in how computeds
  unsubscribe.
- Profiling shows `Box<dyn Any>` values costing more than typed per-kind storage would.
- A use of the graph needs reads from another thread. The answer then is a snapshot through `mailbox`, not a `Sync`
  graph.
