# 0026. A component runs once

Status: accepted, 2026-11-24; not yet built (M11, K4–K8). Supersedes ADR-0013's instance, render and scheduler; keeps
its bindings by phase, its `Callback`, `Model`, context and panic rule. ADR-0013's status line changes at K8, when the
old path is deleted, not before. Amends ADR-0010, ADR-0015, ADR-0022, ADR-0023 and ADR-0025 (see "Amendments").
Written together with ADR-0028, which shares its definition of a cold frame.

Who decided what:

- **2026-11-23, the user:** components run once, as Solid, Vue Vapor and Floem do, in place of render and diff
  (`docs/plan-2026-11-21.md`, "Sắp xếp lại ngày 2026-11-23", item 1).
- **2026-11-24, the user:** "làm toàn bộ đi, chỉ dừng khi xong hết" (do all of it, stop only when everything is
  done). This delegates approval of the details below. They were decided by the agents doing the work and accepted
  under that delegation. Three review rounds by two model families failed; this text holds the decisions that
  answered them, and the last set was not reviewed again. The findings of round 3 were mostly about ADR-0028's
  trial, now ADR-0030 (proposed).

## Context

ADR-0013 got half of this right. `setup` runs once per instance and bindings write one node in one phase. The other
half is a virtual DOM at the level of structure (`docs/review-2026-11-23.md` §6.2, §12.1):

- **A dirty instance renders its whole output again.** `resolve` in `crates/gui/src/component.rs` then clones the
  child map, builds a second one, and `expand` walks the raw output with two SipHash steps per child (`path_step`).
  The cost follows the number of children, also when none changed. An instance with work below it walks `expand`
  again without rendering.
- **Layout patches from the root.** Each pass, `LayoutTree::renew` (`layout/patch.rs`) matches the element tree
  against the retained nodes by `NodePath` and skips subtrees whose element is the same pointer (`patch.rs:185`,
  `:203`, `:385`, `:547`).
- **Any node that comes, goes or moves rewrites the frame.** `LayoutTree::structure` turns the write into `Write::All`
  (`layout/write.rs`), and accessibility takes its full path (review §6.1). A virtual list that scrolls over a row
  boundary does this every time.
- **Render exists mostly to change values.** In `crates/shadcn/src` there are 154 `component(` calls, 113 renders of
  the form `props.with(|p| p.slot.0.clone())`, and 146 `.when(`. The trigger of `tabs.rs` renders only for
  `.when(active, …)` and `theme()`.
- **Machinery exists only because render runs again:** `KEPT_RENDERS` (a render scope kept one render longer, wave C
  §3.6), `Slot` that never compares equal, `Ctx::event` and `Signal::store_quiet` (so a handle calls the latest
  closure), `met` and `marked` (so a reused placeholder passes nothing), the `Priority` lanes and `DEFERRED`.

K1 and K2 are built and make the next step possible: a binding definition is one allocation shared by its mounts,
the compared value lives per mount (`value.rs`: `Bound`, `BoundWrite`, `Mounted`), and the common sources are an enum
(`Run`).

**Where the peers who run once break.** These were read in `references/`; the decision answers each one.

| Peer | What breaks | Where |
| --- | --- | --- |
| Floem | Paint can run between two halves of one structural change: `dyn_view` sets the new children at once and removes the old ones in a later message; the update loop is budgeted and a frame is still painted when it stops early | `src/views/dyn_view.rs:26-51`, `src/window/handle.rs:943-1000`, issue #1052 |
| Floem | Duplicate keys collapse in an index set while items are still indexed by position, so a row is built from the wrong item | `src/views/dyn_stack.rs:95-113`, issue #354 |
| Floem, Leptos | A kept key never gets new item data: the item is given to the view function only when the row is added | `dyn_stack.rs:102-104`; `leptos/tachys/src/view/keyed.rs:721-747` |
| Floem | A virtual row that scrolls out is destroyed with its id and state; selection is kept by index; removing a focused view leaves its id in the focus path | `src/views/virtual_stack.rs:427-526`, `src/window/state.rs:330-386`, issue #867 |
| Floem | A second change during an exit animation removes the wrong child and never disposes the middle one | `src/views/dyn_container.rs:144-189` |
| Vue Vapor | Duplicate keys are checked only in dev builds. In production the later row overwrites the earlier one in the key map, and the earlier row's nodes and scope are never released | `runtime-vapor/src/apiCreateFor.ts:229-244`, `:266-281` |
| Vue Vapor | Re-entering a key whose old copy is still leaving forces the leave to finish and mounts a new instance | `runtime-vapor/src/fragment.ts:554-572`, `components/Transition.ts:356-358` |
| Leptos | An event runs after its owner was disposed; a scoped task is not cancelled with its owner | issues #4457, #4847, #4693; `reactive_graph/src/lib.rs:157-158` |
| Dioxus | Unique keys are checked only under `debug_assertions` | `packages/vdom/src/diff/iterator.rs:240-258` |

There is no `solid` directory in `references/`; Solid is cited from the review only.

## Options

1. **Keep ADR-0013 and make `resolve` cheap** (a fast hasher, an in-place diff; review §6.2). Render and diff stay,
   and so does everything listed above that exists for them.
2. **Run once, on a statically typed view tree** (Xilem, Leptos `tachys`). Leptos had to add `AnyView` and
   `--cfg erase_components` for build time (review §12.1). With no diff left, static types buy no speed here.
3. **Run once, with templates from a macro** (Dioxus `rsx!`, Vapor's compiler). The project has no proc-macro and
   wants none; in Rust the static part is already known from how the function is written.
4. **Run once, on the builders we have.** A component's body runs once per mount and builds its nodes. Values change
   through bindings. Structure changes only at named control nodes.

## Decision

Option 4.

### A component runs once per mount

```rust
// What exists after K8. Signatures are the design; names may still change in review.
pub fn component<E: IntoElement>(body: impl Fn() -> E + 'static) -> Element;
pub fn show(when: impl Fn() -> bool + 'static, view: impl Fn() -> Element + 'static) -> Show;      // .or_else(view)
pub fn each<T: Clone + PartialEq + 'static, K: Clone + Eq + Hash + Debug + 'static>(
    items: impl Fn() -> Vec<T> + 'static, key: impl Fn(&T) -> K + 'static,
    view: impl Fn(Row<T, K>) -> Element + 'static) -> Each;
pub fn dynamic_view<K: Clone + PartialEq + Debug + 'static>(
    which: impl Fn() -> K + 'static, view: impl Fn(&K) -> Element + 'static) -> Dynamic;
```

The `Clone` bounds come from ADR-0028's K4a list: a signal holds a value that can be copied, and item data and keys
are kept in signals.

- **An `Element` is a description and holds no owner.** It is an immutable, shareable value. `component(body)` is a
  description that keeps `body`; a function that owns nothing and provides nothing returns a plain `Div` and needs no
  `component` (as `button.rs` already is).
- **The scope is made at mount.** Mounting a `component` makes a child `Scope` of the owner at that place, runs
  `body` in it once, untracked, names it after the function (`short_name`, as today) and mounts what the body
  returned. The scope owns what the body made: signals, effects, `while_mounted` registrations, and the binding of a
  `FocusHandle` or `NodeRef` to its node. Unmounting disposes all of it. The body never runs again for that mount.
- **So a description that is not mounted does nothing.** An element passed by value into a receiver's `show` is not
  mounted while hidden: a hidden tab trigger is not in `Members`, holds no key (`Keyed::hold` is exact for value
  slots and closure slots alike) and runs no effect.
- **Mounting one description twice gives two independent mounts**, each with its own scope and its own `Mounted` cell
  per binding (K1). A `FocusHandle` or `NodeRef` attached to a description that is mounted twice at the same time in
  the live tree is reported in debug builds; in release the last mount wins.
- **Handle bindings are per identity domain** ("Node identity" below). A cold mount or a mount in a fork binds a
  `FocusHandle` and a `NodeRef` in its own domain. It is never "mounted twice" against the live one.
- **Only four things change after the mount:** a binding on a property, on content or on an accessibility property;
  `show`; `each`; `dynamic_view`. (`dyn` is a Rust keyword, hence the name.)
- **A body writes only what it made**, as a setup does today (`reactive::rendering`'s check). What it must give to a
  shared owner goes through `while_mounted` (`holder.rs`), unchanged.
- **A panic** out of a body, a source, an effect or a cleanup during the structure commit poisons its root, as
  ADR-0013's rule of 2026-11-23 says. The rule is carried over unchanged.

**A read in a body is not reactive, and the engine says so.** Every read happens in one of six execution contexts:

| Context | A tracked read (`get`, `with`) |
| --- | --- |
| Body (a `component` body or a view closure) | **Reported** in debug builds, at the read, with its location |
| Binding (a computed counts as one) | Subscribes the binding, as today |
| Effect (a `watch` source included) | Subscribes the effect, as today |
| Source of a control node | Subscribes the control node |
| `untracked(|| …)`, and everything called inside it | Exempt: the caller stated the intent. `get_untracked` is the same |
| Handler | Exempt: it runs untracked |

- A body that runs inside an effect (an effect that mounts) is in body context and is reported.
- Owners run parent first. A loop of reschedules has a cap, and reaching the cap is a report.
- This is the one mistake the model makes easy (Solid has it too), so it is caught where it is made, not by the cold
  check.

### Inputs: props, callbacks, slots, context

- **A reactive prop is a signal or a closure.** A parameter that may change is `impl IntoValue<T, M>` (`value.rs`,
  the same trait every setter takes). A parameter fixed for the mount's life is a plain `T`. There is no props struct
  to compare, so `Props`, `Shared`'s pointer equality and `Slot`'s "never equal" have nothing left to do.
- **A callback is the latest one because it is the only one.** `Callback` is given once and captures signals when it
  needs current values. It still runs untracked as the owner that made it. `Ctx::event`, `Ctx::event_opt` and
  `Signal::store_quiet` are deleted.
- **`Callback::call` returns `Option<R>`.** A callback whose owner is disposed does not run and gives `None` (today
  `run_owned` runs it with no owner). A caller that asks for a veto treats `None` as no veto.
- **`Derive` stays** for a function called inside a binding and tracked as that binding (a calendar's
  `is_date_disabled`). It loses `PartialEq`, which only props comparison used.
- **A slot is an element or `impl Fn() -> Element`, never `FnOnce`.** Either way its scope is made where the receiver
  mounts it, so its owner and its context are those of the place. The closure form is what K6's typed parts use
  (`tabs(model, |cx| cx.list(|l| l.trigger(…)))`); it may be called again for another mount.
- **Context follows the owner chain of the mount.** `provide` writes the table of the current scope; `use_context`
  reads up the chain, each scope sharing its parent's table until it provides (copy on write, as today). A second
  `provide` of one type in one scope is a debug report. There is one table, on the scope: `Instance.context` moves
  into the scope at K4a, for old instances too, and `theme()` reads the same chain.
  Content of a `Deferred` layer is mounted by its opener, so its owner chain is the logical parent; `child_step`'s
  special case for layers goes.
- **A parent styles a child's root.** `restyle` on a `component` applies to the root its body returns, at mount; the
  cached `Fallthrough` per output is deleted. On a boxless root (`show`, `each`, `dynamic_view`) it applies to the
  root of each child the control node mounts. A parent that styles by place passes the place: a constant for a fixed
  child, `row.index()` under `each`.
- **A typed part's behaviour is registered by its mount** (tabs' `Item::active`, handlers, roving membership). A part
  that moves or changes key takes its registrations with it; one that unmounts takes them back.
- **Lifecycle.** `on_mounted` runs after the first layout of the mount; `on_unmounted` is `on_cleanup`; `after_layout`
  keeps its owner contract (wave C §3.1, §3.2 are acceptance conditions of K7). `on_updated` has no heir, because
  nothing updates: a part watches what changed (`watch`) and asks for `after_layout` when it needs the result laid
  out. All 7 uses are in `gui`.
- **What component code sees is final at K4a**, so S8 is written once (ADR-0028, "What lands at K4a"): `spawn`,
  `Port`, `Sender::send`, `clipboard::write` with its context and answer port, `Callback::call -> Option<R>`,
  `Signal::new` with `T: Clone`, time and randomness only through `gui::clock` and `gui::random`, the lints (signal
  types, captured `Rc<Cell>`, the system clock) and the clippy bans. F2 and F3 change no component code, and
  ADR-0030 is to need none. The lint for process-wide state is F1's.
- **Layout parameters of rows under a boxless `each`** (`fill`, `hug` of a typed parent) are settled by L1 in
  ADR-0027. The slice exercises that API before any S8 family port starts.

### The model value

`Bind<T>` keeps its three forms, and the form is fixed when the component is called:

| Form | Made by | Who keeps the value | A write |
| --- | --- | --- | --- |
| Controlled | `Bind::model(model)` | the caller's `Model` | writes the model, then `on_change` |
| Uncontrolled | `Bind::default_value(v)` | a signal the mount makes, starting at `v` | writes it, then `on_change` |
| Callback-based | `Bind::value(move || …)` | the caller, as anything the closure reads | only calls `on_change` |

- `Bind::value` takes a closure, not a plain value. A plain value could only change by rendering the parent again.
- `use_model(bind) -> UseModel<T>`. The props signal, the `pick` function and the type parameter `P` go (also from
  `primitives::Tabs<P, T>`).
- One equality rule stays: a write equal to the current value is a no-op in every form.
- Two rules of ADR-0025 item 1 end, because a bind can no longer be replaced after the call: "a later caller value
  pushes into an uncontrolled instance" and "a model bound later is written too".

### Control nodes

- **A control node has no box.** Its children are a contiguous range of its parent's child list, so rows of an `each`
  are direct flex or grid children. Floem's containers are views with a box of their own, which changes gaps and the
  item count of the parent.
- **Each branch and each row has its own scope**, a child of the control node's owner. The view closure runs in it.
- **The source closure is tracked by the control node's own tracker.** A write marks the node for the structure
  commit and nothing else.
- **A view closure is `Fn` and pure in the sense a body is:** it makes state under its scope and writes nothing it did
  not make. The engine may call it again for another mount, in a fork, or in a cold frame.
- **`show`** mounts `view` while `when` is true; `.or_else` gives the other branch. **`dynamic_view`** mounts
  `view(&k)` and replaces it when `which()` gives a `k` that is not equal. Both follow "Leaving" below.

### Lists

- **`each` diffs by key**, skipping the common prefix and suffix, then moving only rows outside the longest
  increasing subsequence of the kept rows (Vapor's `createFor`). Floem and Leptos move every row whose index shifted.
- **Keys are compared by equality** (`K: Eq + Hash`), never by a digest, as `holder.rs`'s `Keyed` already requires.
  `K: Debug` prints the key in `@PATH` and in reports.
- **A kept key is the same row:** same nodes, same `NodeId`s, same scope and state, same focus. A move changes its
  place in the parent's order and nothing inside it.
- **A kept key gets its new data.** `Row<T, K>` gives `item()` (a read handle on a per-row signal), `index()` and
  `key() -> &K`. On each commit the row's item is written when it differs (`T: PartialEq`), and its index when it
  moved. The index signal is made the first time `index()` is called, so a row that never asks pays nothing for a
  move. `index()` counts present rows only, never leaving ones. This is the heir of `on_updated` for lists.
- **Duplicate keys are found in every build and nothing is lost.** The diff builds the key map anyway. The n-th row
  with an equal key gets the identity (key, n), every item is shown, and the commit reports an error naming the
  `each`'s call site (`#[track_caller]`) in `FrameReport` and devtools. A `Scenario` fails a frame that has reports
  unless the test expects them. Vapor and Dioxus check only in dev builds; Floem and Leptos lose or misplace rows.
- **Focus and selection belong to a key.** Focus moves with the row. Selection is data in the owner, keyed by key,
  never state of a row or an index (Floem keeps `selected_idx`). This closes wave C §3.10: a row's `FocusHandle` is
  made in the row's body and lives exactly as long as its mount.
- **`Members` publishes order after layout, from tree order.** A commit that inserts, removes or moves a member marks
  its list; the next layout pass publishes it in tree order and `on_laid_out` runs once (`members.rs:186`, `:209`).
  Mount order must never show. Test: after a keyed move, the list equals the source order.

### The windowed list (K4c)

Today the virtual list chooses its window when it renders (`list/virtual_list.rs:17-19`, `:1246`). It becomes an
`each` whose source is the window:

- **Source:** the rows in view and prefetched, plus the pinned rows, in data order. A row is pinned while it holds
  focus, is the active row (ADR-0021), or is the target of a scroll-into-view not yet shown. The source reads the
  list state and the data.
- **Two things, two names.** The *scroll anchor* is what the list state holds today: the row at the viewport's top,
  its key and how far the viewport is into it. It says where the reader is. The *placement origin* is new: the point
  the window's rows are placed from. It says where coordinates are measured from.
- **The origin is sticky:** a row key and an offset inside that row, as `f64`. Source extents stay `f64`
  (`ExtentTree`, 1/64 px units). A placed coordinate is an `f32` distance from the origin. The origin does not move
  when the window shifts or the reader scrolls.
- **A re-base moves the origin to the scroll anchor's row.** It happens when the distance between the two passes
  2¹⁶ px, or when the origin's row is disposed. A re-base visits the window's rows once: O(window), counted, and
  allowed. Below 2¹⁶ px an `f32` has a step of at most 1/256 px, so every 1/64 px position is exact; 1/64 px stays
  exact up to 2¹⁸ px, which leaves room for the sum of two offsets.
- **A row taller than 2¹⁶ px** is placed by an `f64` offset inside the row, so no `f32` spans it.
- **A measurement or a changed estimate moves rows on one side only, by one shared offset.** The rule is the same in
  both directions. Take a row between the origin and the viewport whose size changes by `delta`:
  - *the row is after the origin* (the viewport is further down): the rows after it, on the viewport's side, move by
    `+delta`;
  - *the row is before the origin* (the viewport is further up): the rows before it, on the viewport's side, move by
    `−delta`;
  - the rows between the changed row and the origin, the origin's row, and everything on the origin's far side do
    not move.

  The rows that move are one contiguous range of the window, and the range's offset is written once, on the list.
  **Visited:** the changed row, and the one range offset. The rows that move are not visited, and neither are the
  rows that stay. A change in the origin's own row moves the rows after it by `+delta` when the change is below the
  origin's in-row offset and the rows before it by `−delta` when above; the in-row offset is adjusted in the same
  step. A change beyond the built rows on either side moves nothing that is built. A window shift alone moves no
  remaining row.
- **A pinned row far away has its own origin** and never causes a re-base.
- **Counted, at 10 thousand and at 10 million source rows with the same window:** rows mounted and disposed, child
  slots changed, parents updated, re-bases, and unchanged siblings visited outside a re-base, which must be 0.
- **No row is pooled across keys.** A row that leaves the window is disposed; one that returns is a new mount. What
  is reused is storage (slots on a free list). State that leaks from one key to the next is the failure a pool
  invites.
- **Duplicate keys outside the window** cannot be seen by the diff. An `each` source reports its keys; a windowed
  source checks the whole key set in debug builds and reports a duplicate there.
- Reindexing after a splice writes `index` of rows that asked for it. The end spacer, the scroll anchor and the
  placement origin are the list state's. Paint order, hit testing and AccessKit actions follow the retained child order, not storage order
  (K5).
- K4c is done after M15's list fixes and after F1, which share the file.

### Leaving (K4d)

- **Every control node keeps a list of leaving entries**, for any number of branches and for rows. With
  `dynamic_view` going A→B→C while exits play, A and B both leave. Clearing all toasts lets every toast leave.
  (`Motion::leaving` is a `Vec` today; Floem's single slot is what loses a child.)
- **An exit is owned by the control node that removed the branch.** A toast's exit is owned by the toaster's `each`,
  not by the toast.
- **A leaving `each` row stays in the retained child order at its old position** until its exit ends. It is excluded
  from `index()`, from `Members` (roving and typeahead), from focus and from hit testing.
- **A leaving branch is frozen.** Its bindings and effects stop: their trackers are disposed and the last values are
  kept on the nodes. Only the exit animation runs; it is driven by motion and reads no app state. This keeps
  `primitives/src/presence.rs:8-9` ("nothing in it runs while it plays out"). So
  `show(|| sel.get().is_some(), || text(move || sel.get().unwrap().name))` cannot panic during the exit.
- **Freezing is ordered parent first.** In one flush, a control node's structure step runs before the effects and
  bindings of its descendants. So a branch about to leave is frozen before any effect inside it runs, and a child
  effect never sees `sel == None` while its parent still shows it.
- **Its scope is not disposed yet.** Teardown (scope dispose and cleanups) runs when the exit ends, or at once when
  there is no exit.
- **When its key comes back before the exit ends, the same instance is revived.** It is not mounted anew: its scope
  and state are the ones it had, its bindings and effects subscribe again and run once against current state, and it
  enters from the progress its exit had reached. It reopens its layers and its focus scopes; auto-focus does not run
  again, because it is not a new mount. A `show` or `dynamic_view` branch revives the same way when its condition or
  key returns. Vapor forces the leave to finish and mounts a new instance.
- **Order when a branch starts to leave** is "Teardown" below, steps 1 to 3.

### Node identity (K4e)

- **A retained node has a `NodeId`: internally slot and generation, 32 bits each.** It is issued at mount, kept
  through a keyed move, and never names another node. It replaces `NodePath` (`geometry.rs:160`, 111 uses in `gui`) as the key in
  focus, motion, interaction, scroll state, accessibility and the compositor's `NodeKey`.
- **Three identity domains,** told apart in the id:
  - *live*: nodes of the window's retained tree;
  - *cold*: nodes a cold frame mounts into its detached tree;
  - *synthetic*: ids that name no retained node (AccessKit's own nodes such as text runs, scroll acknowledgements).
- **A synthetic child has its own slot.** A text run, or any other node the engine publishes that is not a retained
  node, gets a slot and a generation in the synthetic domain, allocated when it is first published and retired like
  a node's, with its owner. There is no sub-index, so the number of runs under one node has no bound. The hash-based
  run ids go.
- **The published id** (AccessKit, compositor scroll acknowledgements) is 64 bits: domain 2 | slot 30 | generation
  32. Nothing is truncated: it is the internal id with the domain in the slot's two top bits. Two published ids
  cannot collide, by construction. One that arrives with a stale generation is dropped, never applied to the node
  now in that slot. Each domain holds at most 2³⁰ slots. This replaces the path key in ADR-0015's `Commit.scroll`
  and `NodeKey`.
- **A path is derived, not stored.** `inspect` and devtools build `@PATH` from parent links on demand, printing keys
  with `K: Debug`. Nothing hashes a path per frame. "Is below" is a walk of parent links.
- **`patch` and `patch_children` become `mount`.** Nothing matches an element against a node after the mount, so the
  pointer-equality shortcuts go with them. Structure changes only in the commit of a control node.
- **Generations: one policy for every engine id** (wave C §3.12). A slot whose generation would overflow is retired
  and never reused; an id never wraps. Every generation is 32 bits, node and synthetic slots included, so one slot is
  lost per 2³² reuses of it; retired slots are counted in devtools `memory`.

### The frame

1. Receive: input handlers, timers, mailboxes. These write signals.
2. **Flush and structure commit: one flush, ordered by owner, parent first.** A marked control node evaluates its
   source, diffs, and mounts, moves, freezes or removes, before the effects of its descendants run. Effects run in
   the same order. View closures and bodies run here; their effects run after each body and may mark more control
   nodes. The flush loops until nothing is marked, under the cap that `LAYOUT_PASSES` gives today.
3. Layout: bindings resolve, nodes are placed. `inert` and `invisible` are bindings like any other: they are resolved
   here and written to the frame node (`Look.invisible`, `Look.dormant`, the node's inert flag).
4. **Settle**, from the committed and laid-out frame: the layer stack is derived; auto-focus on open, focus-outside
   and `drop_lost` run. Settle, hit testing and focus read `inert` and `invisible` from the frame, never from a
   binding. A leaving branch is inert by its leaving entry, not by its frozen bindings. A settle that moves focus, or
   opens or closes a layer, marks work, and the frame goes back to step 2 under the same cap.
5. Hooks after layout. New marks go back to step 2 under the same cap.
6. Paint. 7. Accessibility.

**At the cap** the frame paints what it has. The stranded work is an error report in debug builds and schedules the
next frame at once. The cold check counts a stranded frame as a difference.

**Invariants** (review §18.5 row 6). One is a type property; the others are runtime checks, each a debug assert plus
a K4a test:

| Invariant | Held by |
| --- | --- |
| A commit's result is used once: `Structure::finish(self) -> Committed` | Type |
| Only the commit inserts, removes or moves; layout, paint and accessibility read a finished commit | Runtime check. Until K8 the old `LayoutTree::renew` is exempt, because it patches structure during layout. At K8 the structural mutators become private to the commit, and the check becomes visibility |
| One control node's commit is never split: paint sees it whole or not at all (Floem #1052). Work still marked at the cap waits for the next frame, whole, as "At the cap" says | Runtime check |
| `each` reports duplicate keys | Runtime, every build |
| Focus and selection belong to a key | K4a tests |
| Teardown has one order | Runtime check and tests |
| An async job is cancelled with its scope | Tests |

**Teardown.** For a subtree removed in a commit:

1. Captures are cancelled; each handler runs while the subtree is alive.
2. Its focus scopes and layers close, innermost first, each recording whether the focused node was inside it. Then
   ADR-0024's focus return runs once, to what the scopes recorded. Scopes close before focus moves, so the return
   never lands inside what is leaving.
3. With an exit: the subtree is frozen and enters its control node's leaving list. Steps 4 to 6 wait for the exit to
   end.
4. Scopes are disposed, deepest first, the cleanups of one scope last made first (ADR-0012).
5. Hooks queued for a disposed owner are dropped, its tasks cancelled, its mailboxes closed.
6. Its `NodeId`s are freed.

After step 1 no handler of the subtree runs.

**An async job belongs to a scope and is cancelled with it.** Work leaves the UI thread only through `spawn`
(ADR-0028). The job gets a cancel token; when its owner is disposed the token is set and its result is dropped. The
two places that start a thread per request today move to it: `primitives/src/command.rs:668` and
`gui/src/list/view.rs:65`.

### What a cold frame runs

The cold check stays an independent referee. The four points below are stated identically in ADR-0028; the two
records change together.

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

Details that belong to this record:

- **Why not the bodies.** A body has no incremental path: it ran once. Its signals hold state the user changed, so
  running it again from nothing would give another program, not the same one built cold.
- **The cold tree** is the root's description mounted again into a detached layout tree, with ids in the cold
  domain. At a control node it takes the expected keys; for a key the live tree has, it mounts the description that
  row was mounted from (kept per row; each retained node keeps its element, `Extra.element`), without running its
  body; for one the live tree lacks, it runs the body or view closure inside the fork.
- **The correspondence** maps each cold node to the live node it stands for. Its key has three parts:
  1. the correspondent of the node's physical parent;
  2. the full nested path of control ranges between that parent and the node, outermost first, each step being
     (range id, row or branch key, occurrence index of that key in its range);
  3. the node's static child position inside the innermost body or view that mounted it (or under the parent, when
     the path is empty).

  The key gives the live `NodeId`. Control nodes have no box, so several can nest under one physical parent: a
  multi-root row that holds a boxless `show` or `each` puts nodes of different depths side by side. The full path
  tells them apart; one (range id, key) pair could not. The range id tells two adjacent ranges apart, the occurrence
  index tells duplicate keys apart, and the static position tells the roots of one row apart. The correspondence is
  computed while the cold tree mounts, and it is how the cold tree reads live per-node state.
- **The structural referee** computes, from the sources alone, what each control node should hold. It shares no code
  with the diff, the LIS or the commit; it only lists. Its observation points are: the physical retained child order
  of each parent (not the control node's own key list, which the commit also wrote); the owners removed; the exit
  ledger; and the correspondence, which must be total and injective. It also checks row signals against the source.
- **`legacy(…)` nodes are opaque to the referee** (below): compared by frame, scene and accessibility only. In a
  cold or fork build the legacy subtree is built fresh by the old path inside the detached tree, with `NodePath`s
  relative to the legacy node.
- **Proof required by K4a:** suppress one insert, and separately one keyed move, in the commit; the cold check fails
  both times.
- **Relation to Fork.** A cold frame is ADR-0028's `Check`: it mounts a detached tree, writes no live signal and
  never writes the retained live tree, and it is always rolled back. So the commit keeps no undo log of the retained
  tree, and K4a builds none. A fork that runs handlers is ADR-0030 (proposed); if it needs such a log, it adds it.

### What becomes of the old mechanisms

| Mechanism | Decision |
| --- | --- |
| `holder.rs` (`while_mounted`, `Holder`, `Registry`, `Keyed`) | Stays: a mount still registers with a shared owner. `Keyed::hold` is released when its mount's scope ends, which is now exact, so `KEPT_RENDERS` and the three loosened test bounds go. `Keyed::get` is deleted (wave C §3.5): only `hold`, which needs an owner, and `peek`. The `is_cold_render()` branches go. §3.9 (`Registry::walk` must stop at the tail it started with) stays an M15 fix |
| Render lanes (`Priority`, `with_priority`, `idle`, `set_idle_budget`, `DEFERRED`) | Deleted, no heir. There is no render to defer, and a commit is never split (Floem's budgeted loop is the failure). `idle` and `set_idle_budget` are called only in `gui/src/component/tests.rs`. Large lists use the windowed list; long work goes off the thread through `spawn` |
| `NodePath` | Replaced by `NodeId` (above). It lives inside the old patch until K8 |
| Generations | One policy, above |
| `Derive` | Stays, without `PartialEq` |
| `Motion::splice` and the ghost element | Replaced by the leaving lists at K8 |
| `on_updated`, `Ctx`, `Slot`, `Part`/`PartProps`, `Element::Component`, `Why`, `Rendered`, `Stats.renders` | Deleted at K8, per K8's list |
| Tabs' and carousel's "did it change" clone (wave C §3.11) | K7 decides. The row signals above make it unnecessary for lists |

### Phase tokens

Tokens do not change the signature of a binding or a handler, so nothing here has to precede S8.

- A binding's phase is already fixed by the setter that takes it. The closure reads signals, which belong to no
  phase. Adding a token parameter would rewrite every binding closure for no check the setter does not already give.
- The engine's tokens are `Structure` and `Committed` above. Typing layout against paint (`&LayoutCx`, `&PaintCx`,
  review §11.3 item 5) is internal to `gui` and can follow at any time.
- The late-write check stays the guard for user code that reads laid-out results.

### Devtools and inspect (K4f)

Built before the render counter is removed (`devtools.rs` uses `renders` 10 times).

- **Events:** mount and unmount (component name, control node, key), move (`each`, key, from, to), leave, revive,
  duplicate key, and binding run (node, phase, property, and why: the signal written, hover, a group, or the mount).
- **`FrameReport`:** `rendered` is replaced by structure counts (mounts, unmounts, moves, child slots changed, parents
  updated, siblings visited) beside the binding count it has.
- **Commands:** `components` lists mount scopes with what each owns; `counters` shows mounts, unmounts, moves and
  bindings by phase in place of renders; "why did this render" becomes "why did this binding run" and "why did this
  mount". Check: command snapshots.

### Migration

Not a shim (plan, "Nguyên tắc chuyển đổi").

- K4 adds the API beside the old path. One subtree has one mount algorithm, one scope tree and one place that writes
  structure.
- **One boundary node: `legacy(element)`.** It mounts an old-path subtree (an `Element::Component` or a `Part`)
  inside a run-once tree. It is a single node, re-rendered and patched by the old path, and opaque to the referee.
  It exists because the docs app puts every demo inside `tabs` (`docs/component.rs:240`): tabs can move first while
  `code_block`, `input` and the rest are still old.
- **Context crosses the boundary** because there is one table on the scope chain from K4a: a legacy component's
  `use_context` and `theme()` read what a run-once ancestor provided.
- **The other direction needs nothing new:** a run-once tree sits in an old tree as the output of an old `component`
  whose setup builds the description once and whose render returns it.
- Nothing else crosses: no adapter passes a slot or a part between the models.
- Families move bottom-up by primitive dependency (K7, S8), each through `gui`, `primitives`, `shadcn`, its demos and
  its tests. **K8's gate is zero uses of `legacy` and zero callers of the old path in the workspace**; K8 then
  deletes both.
- **The slice first:** K4 is accepted only when tabs, select and one virtual list page have moved completely, with
  `GUI_COLD_CHECK=1` green on the slice, its docs pages (legacy demos inside run-once tabs) included. A design error
  in slots, exits or typed parts must show there, not in group 3.

**Acceptance scenarios** for the slice and for every family:

- changing a controlled tab, panels kept mounted (`keep_mounted`): 0 mounts and 0 unmounts; the bindings of the two
  triggers and panels run;
- the same with the default (panels not kept): the opened panel mounts and the old one is disposed, each counted
  separately, and nothing else mounts;
- reordering by key: 0 mounts and 0 unmounts, the moved rows keep focus, motion state and accessibility ids;
- closing an overlay and opening it again while it exits: the same instance revived, its state kept, one layer entry.

### Amendments

- **ADR-0010.** `components` lists mount scopes, not instances. `counters` and the frame report show structure counts
  and bindings, not renders. Selectors `@PATH` are derived from parent links. New events as in K4f.
- **ADR-0015.** Scroll acknowledgements and `NodeKey` carry the published id (domain, slot, generation),
  not a path key; a stale generation is dropped.
- **ADR-0022.** "No component renders" (line 284) becomes "there is no render": the claim is measured by mounts,
  structure changes, bindings run, nodes laid out and chunks recorded. "A choice that changes the tree would be read
  in render" (line 288) and the first "Revisit when" item (line 349) become "is a `dynamic_view` over that choice".
- **ADR-0023.** The cold check is defined by "What a cold frame runs" above, with the structural referee. `counters`
  loses renders. Sampling in release builds is ADR-0028's.
- **ADR-0025.** Item 1: `use_model(bind)`, three forms fixed at the call, the two "later" rules ended. A part is a
  function that decorates a `Div` once: `tabs.trigger(d, value, availability, focus)` keeps its shape, what it read at
  render (`is_selected`, availability, `aria_selected`) becomes a closure (K4b makes every `Div` property take one;
  today only `tab_stop` does, `element.rs:1021`), and its registrations are made by its mount. `tab_stop`'s text "the
  items' render does not read it" becomes "nothing reads it until Tab looks". Presence: a leaving part is frozen and
  is revived as the same instance.
- **AGENTS.md, "Components and scheduling" (lines 89–105)** is replaced by: a component's body runs once per mount
  and builds retained nodes; an element is a description and its scope is made at mount; values change through
  bindings, structure through `show`, `each` and `dynamic_view`; a reactive prop is a signal or a closure; a slot is
  mounted where it is placed; context follows the owner chain; the frame is receive, one flush with the structure
  commit (parent first), layout, settle, hooks, paint, accessibility; one control node's commit is never split.
- **Also touched, in their own "As built" notes when built:** ADR-0007 (motion keyed by `NodeId`; leaving lists in
  place of the spliced ghost), ADR-0014 (the window is a source, not chosen at render), ADR-0021 (accessibility ids
  are `NodeId`s).

## Consequences

- **Hover, theme, style and light or dark run no component code**, by construction. Only the bindings that read the
  value run.
- **A value change in steady state clones no map, makes no `HashMap` and hashes no path.**
- **`resolve`, `expand`, both child maps and the per-frame patch from the root are gone**; so are about 20 names on
  K8's list.
- **Call sites change shape.** A branch on reactive state must be `show`, not `if`. About 75 components are
  rewritten, once, together with S8 and typed layout.
- **A `component` keeps one boxed body per description.** That is the price of a scope made at mount, and what lets a
  hidden part do nothing.
- **A cold frame no longer proves that a body is a function of current state.** The read diagnostic covers the common
  mistake at its source. A body that reads state untracked on purpose is trusted.
- **Most frame invariants are checked at run time, not by types.** Only "a commit is finished once" is a type
  property before K8.
- **Each control node costs** a tracker, a scope per branch or row, a key map per `each`, and a leaving list.
- **The cold check costs** one description kept per row. Each retained node keeps its element already.
- **Compared with B0:** per-demo verdict not below `crates/shadcn/verdict/baseline/vega.tsv`, docs bench and stress
  not worse, `GUI_COLD_CHECK=1` green on every test and monkey run.

## Alternatives rejected

- **A body that runs where it is called, with its scope made at the call** (Solid, Vapor, Floem). No boxed body per
  component. Rejected: an element would then own live state before it is mounted, so a part passed by value and
  hidden by its receiver would still be in `Members` and run effects, and one description could not be mounted
  twice.
- **A cold frame that runs every body again and takes its state back by creation order** (the n-th signal a body
  makes is the n-th the live scope owns, as React's hooks are matched). Rejected: every identity a body mints would
  need the same matching, and a body that legitimately reads a mount-time value would fail the check. The fault it
  finds is caught more directly by the read diagnostic.
- **A leaving branch that stays live** (bindings and effects still running during the exit). Rejected: a binding
  that reads state whose absence is the reason for leaving panics or shows garbage, and ADR-0025 promises that
  nothing in it runs.
- **A leaving branch that is disposed at once and cannot come back** (Vapor; today's ghost element). Rejected: it
  cannot be the same instance again, which ADR-0025 also promises. Frozen with its scope kept gives both.
- **One leaving entry per control node** (Floem's `dyn_container`). Rejected: A→B→C and clear-all need several.
- **Rows placed from the scroll anchor, or from the list's start,** for the windowed list. From the anchor, every
  scroll step would move every row. From the start, a measurement above the viewport would move every visible row
  and `f32` would run out. A sticky origin near the window has neither fault.
- **A control node with a box of its own** (Floem). Rejected: it changes the parent's layout, and typed layout
  (ADR-0027) counts children.
- **Row pools for the virtual list.** Rejected: state leaks between keys; the cost of a mount is what K1 and K2
  reduced.
- **Keeping the first of two duplicate keys and dropping the rest** (what `expand` prints today). Rejected: an item
  silently missing is the failure this record lists.
- **Converting whole families with no boundary node.** Rejected: the docs shell wraps every demo in `tabs`, so the
  first family could not move alone.
- **Phase tokens in binding signatures.** Rejected above.
- **Keeping `idle`.** Rejected: its only meaning was "render later"; a commit split across frames is a tree paint can
  see half done.

## Revisit when

- A body's static tree is found stale in practice despite the read diagnostic: then reconsider matching state by
  creation order for the cold frame.
- A list needs to keep a row's state after it scrolls out and is not pinned: add a way to pin by key, not a pool.
- A source is a `Collection<T>` large enough that diffing its window shows in profiles: feed its change log to the
  commit, with the same referee.
- The boxed body per `component` shows in mount profiles.
- A component must keep its state while moving to another parent (ADR-0013's open item, still open).
