# 0013. A component runs `setup` once and renders tracked

Status: accepted, 2026-11-21; core built 2026-11-21, dirtiness per node and phase built 2026-11-21 (C4; see "As built") (C6, the shadcn components' rewrite, in progress). Supersedes the whole-app rebuild on any signal write (`take_changed`,
ADR-0008's effect bridge into a rebuild); keeps ADR-0007's keyed motion and ADR-0009's semantics.

## Context

`gui` has no component instance: nothing with an identity and a lifetime of its own. Every write rebuilds the whole
app, and what makes that safe also makes it slow and fragile. `docs/research-2026-11-21.md` §4.1 lists 15 patterns
in `crates/shadcn` that follow from it:

- **State must be made "outside the render closure"** (22 files say so: `select.rs:153`, `command.rs:34`,
  `slider.rs:19`). If it is made inside by mistake, it silently resets every frame.
- **Effects are kept alive in hand-written fields** (`number_field.rs:22`, `form.rs:91`, `time_input.rs:38`).
- **`watch` is rebuilt by hand** from an `Effect` and a `Cell` (`number_field.rs:191-197`).
- **Two sources of truth.** `FormField::reset` writes `value` and the field still shows the old number
  (`form.rs:276-278`).
- **Items register with their parent while rendering.** `CommandState::begin` clears `entries`, then items push
  (`command.rs:70-81`); the same holds in `select.rs:428-504` and `dropdown_menu.rs:8-9`. A subtree that is not
  rendered never registers, so skipping a subtree breaks the list.
- **Render writes non-reactive state** (`select.rs:437-441`; 22 `Rc<Cell|RefCell>` fields).
- **110 `let x = y.clone();` lines exist** only because `Signal` is `Rc`.
- **Timers have no owner** (`carousel.rs:110-119`): autoplay runs until `stop()`.
- **Context is one global value per `TypeId`** (`context.rs:9-27`), so a dark card cannot sit in a light page.
- **Values captured at render** (`checkbox_state(state, …)`, `let t = theme()`) are correct only because everything
  rebuilds.

Measured cost: stress `grid` (2 500 buttons) takes 94 ms per click, and hovering costs a whole frame. Stress `table`
(60 023 nodes) builds, lays out and paints in 388 ms a frame.

## Options

1. **Keep the full rebuild and memoize subtrees by hand** (Xilem's `memoize`, iced's `lazy`). Build gets cheaper,
   but every pattern above stays, and identity stays positional.
2. **A virtual DOM with component instances, as Vue 3.5's renderer.**
   - A render effect per instance (`references/vuejs/packages/runtime-core/src/renderer.ts:1347-1670`).
   - Children are skipped when their props are equal (`shouldUpdateComponent`, `componentRenderUtils.ts:374-440`).
   - A keyed diff with LIS (`renderer.ts:1853-2095`).
   - A scheduler ordered by `uid` (`scheduler.ts`).
   - `provide` shared copy-on-write down the tree (`apiInject.ts`).
3. **Fine-grained only, as Vue Vapor, Leptos, Solid.** `setup` runs once; each dynamic binding is its own small
   effect writing one node; there is no re-render.
4. **Both: option 2's instances with option 3's bindings.** An instance renders tracked and skips equal props. Any
   leaf property can instead bind a closure straight to its node and phase, so most updates never re-render.

## Decision

Option 4.

**An instance.**

```rust
pub fn component<P: Props>(key: impl Into<ElementKey>, props: P,
    setup: impl FnOnce(Ctx<P>) -> Box<dyn FnMut() -> Element> + 'static) -> Element;
pub trait Props: 'static { fn changed(&self, old: &Self) -> bool; } // from PartialEq; Callback and slots compare equal
```

- **Identity** is position plus key plus component type (the `TypeId` of its setup). Two branches of an `if` at one
  position are two instances; Flutter and Dioxus do the same.
- **`setup` runs once per instance** inside the instance's `Scope` (ADR-0012). The scope owns the instance's
  signals, computeds, watches, timers (`after`, `every`), collections and lifecycle hooks (`on_mounted`,
  `on_updated`, `on_unmounted`, `on_cleanup`). Unmounting disposes it.
- **Render is a tracked subscriber of the instance.** It reruns only when something it read changed. It is pure: it
  creates no state and writes no signal (a debug assert enforces both).

**Skipping and props.**

- A parent's re-render skips a child whose `changed` is false.
- `cx.props()` is reactive, so a changed prop reruns only what read it.
- `Shared<T>` compares by `Rc::ptr_eq`, so a large collection costs nothing to compare.
- `Callback<A>` is a latest-value cell. It always compares equal and is never stale. It is never called during
  render (debug assert).
- Slots are closures that run inside the child's own tracked render.

**Two-way binding and context.**

- `Model<T>` is built by `Model::bind(signal)` or `Model::from(default)`, with `.on_change(cb)`. Inside, `use_model`
  writes up when bound and locally when not, as Vue's `useModel` (`helpers/useModel.ts:20-80`).
- `provide(v)` and `inject::<T>()` follow the scope tree, sharing the parent's table copy-on-write. `deferred` layers
  (portals) keep their logical parent.
- Collections register in `setup` (`items.register(entry)`) and unregister on unmount. They are ordered by tree
  position, as reka's collections are. Huge lists read a `Collection<T>` view instead (ADR-0012).

**Bindings by phase.**

- Every style and content setter takes a value or a closure `impl Fn() -> T`. A static value and a reactive one are
  the same property with one owner.
- A closure binds to the retained node and to the phase its property belongs to, as Vue Vapor, Floem and Leptos
  bind. A signal read there dirties only that phase of that node:

| Phase | What it covers | A write there |
| --- | --- | --- |
| Render | an instance's render | reruns that instance; equal-props children skip |
| Layout | size-affecting style, text content | `NeedsLayout` on the node, stopping at its relayout boundary |
| Paint | colour, border, shadow, content of fixed size | re-records the node's paint chunk |
| Property | opacity, transform, scroll offset | writes one property-tree entry |
| A11y | role, name, state | resends that node (ADR-0021) |
| Chunk | GPU data and view of a data chunk | reruns that chunk's upload and compute |

- A write in a phase that was already read earlier in the same frame is a reported error in debug builds. Compose
  documents the multi-frame loop this prevents.

**The scheduler.**

- One synchronous flush per frame. Input-driven updates are never split.
- The queue is keyed by priority (`Input`, `Animation`, `Idle`) and ordered by tree depth, parent first. A job a
  parent already ran is skipped. `Idle` work checks its deadline between instances and never commits halfway.
- **Frame order:**
  1. Receive: timers, `mailbox::deliver`, devtools, input handlers; these only write signals.
  2. Pre-flush watches.
  3. Render queue by depth.
  4. Motion for animating nodes only.
  5. Incremental layout.
  6. Placement.
  7. Post-flush: `on_mounted`, `on_updated`, `on_layout` (replaces `after_layout`, `app.rs:484-495`). New jobs loop
     back to step 3 under a cap that replaces `LAYOUT_PASSES`.
  8. Paint of dirty chunks.
  9. Accessibility of changed nodes.
- A frame is requested only when one of these is non-empty: the render queue, the dirty sets, the animating nodes, or
  the readers of the clock.

**Builders, no macro.** The builder API stays. Setup and render cross a `dyn` boundary so each component compiles
once.

## As built (2026-11-21)

- `crates/gui/src/component.rs`: `component(props, setup)` → `Element::Component`, resolved each frame by
  `Root::resolve`. Identity is the path of keys (or positions) from the parent's output root plus the setup's
  `TypeId`. An instance holds its `Scope`, a render node (`Kind::Render` in the reactive runtime: a write marks it, the
  frame renders it), its props as a `Signal<P>`, its raw and expanded output and its children. A clean instance with
  nothing dirty below gives back its last output; one with work below re-expands its raw output only.
- `Callback`, `Shared`, `Model` (`bind`, `local`, `on_change`), `provide`/`use_context` per instance with a
  copy-on-write table (the global map stays for app-wide values), `on_mounted`, `on_updated`, `on_unmounted`.
  Equal props are stored without a wake (`Signal::store_quiet`), so a child calls the latest callbacks.
- Scheduler: `Priority::{Input, Animation, Idle}`, `with_priority`, `idle`. The walk is top-down, so a parent renders
  before its child and a child re-rendered by its parent's new props renders once. Idle renders run while the frame's
  budget (4 ms, `set_idle_budget`) lasts, checked between instances; a deferred one keeps its old output and the
  frame stays due.
- `take_changed` is gone. A frame is due when an instance is marked, an idle render waits, or a signal read by layout
  or paint (two trackers in `Pipeline`) changed. Post-flush (scroll hooks, `on_mounted`, `on_updated`) loops under the
  pass cap. Timers made under an owner are cancelled with it. A `ScrollHandle`'s layout and offset are reactive.
- Bindings: `.bind(|s| …)` takes any setter, `text_bound(|| …)` binds text content; both run at layout under the
  node's own tracker, so a write they read re-resolves that node alone, with no render (C4, below).
- Devtools: each `FrameReport` lists the instances rendered and why (`grid_button ← signal#18`, `props`, `mount`),
  the bindings run; `instances` lists every instance, its renders and what it read.
- `GUI_COLD_CHECK=1` checks every frame against one with every instance rendered again.
- `v-model` props (C6a, C6b): `Bind<T>` holds the caller's `Model` (`v-model`), a value the caller keeps
  (`:model-value`, a write only calls `on_change`) or a default the component keeps (`default-value`), plus
  `on_change`; `use_model(props, pick)` in setup makes the `Copy` handle that reads and writes it, as Vue's
  `useModel` and reka's `useVModel` do. A leaf given a plain value holds no state and renders into its caller with
  no instance. `Slot<T>` is what a parent passes to show; it never compares equal (Vue's non-stable slots).
- An element tree can be rendered more than once: a placeholder keeps its props and setup (both `Clone`), so a part
  that hides its slot and shows it again (a closed dialog's content, an inactive tab) mounts the instances inside it
  again. A placeholder a render reuses passes nothing new to its live instance.
- A parent styles a child component's root through `Element::restyle(f)` (Vue's attribute fallthrough): on a node it
  applies at once; on a component's place it is kept and applied to the root of every output the instance gives,
  cached per (output, `f`) so a clean instance still gives back the same node. `button_group` joins and a responsive
  field's `w-auto` reach instances this way.
- One `Slot`, gui's: a part's slot is `gui::Slot<Div>`. One `v-model` mechanism, gui's `Bind` and `use_model`.
- The form and overlay components are instances: compound parts are `shadcn::Part`s (a caller-styled box given to an
  instance as its slot) under a root that `provide`s a `Copy` context. Overlays give focus back from a watch on their
  open state (`dialog::return_focus`), so a parent writing the model restores it too. The `XxxState` structs, the
  `&state` parameters and every `_effect` field of these components are gone.
- The data, navigation and content components (C6c, C6d) are instances the same way. `tabs`, `accordion`,
  `collapsible` (and `trace_collapsible`), `sidebar_provider`, `pagination`, `stepper`, `resizable_panel_group`,
  `carousel`, `calendar`, `range_calendar` and `questionnaire` keep their value in a `Bind`, set with reka's names
  (`.model`, `.value`, `.default_value`, `.on_change`; `default_open`/`open`, `default_page`/`page`,
  `default_layout`). Their triggers make a `FocusHandle` in setup and join the root's `gui::Members`, so arrow keys
  walk what is on screen; a closed panel gives nothing. Timers (carousel autoplay, the copy button's reset) are made
  under a scope of the instance and go with it. A calendar day and a chart's crosshair and tooltip are instances of
  their own, so a pick or a hover renders only them. `calendar().initial_focus()` is reka's `initialFocus`.
  `code_block_copy_button(value)` keeps its "copied" state; `message_scroller_provider` keeps the thread's scroll
  engine, read by its parts through `use_message_scroller` (Vue's `useMessageScroller`). Parts with no state
  (`breadcrumb`, `table`, `scroll_area`, `message`, `bubble`, `attachment` and the presentational modules) stay
  `Div` builders: they hold nothing an instance would own. Behaviour is made in setup and look applied in render,
  so M10's S6 can move the first into `primitives`.

- Focus return (C6b follow-up): a modal (`dialog`, `alert_dialog`, `sheet`, `drawer`) gives focus back as reka's
  `getReturnFocusElement` does: to the trigger whose press opened it (`dialog_trigger` holds a `FocusHandle` made in
  setup, registered with the root's `FocusReturn` on mount), else to what held focus as it opened, else to the
  mounted trigger; a trigger gone since is skipped, and with nothing to return to nothing keeps focus, so the docs'
  ⌘K palette (no trigger, opened with nothing focused) no longer leaves focus on its unmounted search field.
- `command().search(model)` / `.default_search(..)` / `.on_search_change(..)` is the command's search as a
  `v-model` (`Bind<String>`, bound to the command's own field in setup). A listbox filter outside the command (a
  tags input) binds the same model through its own `.text(model)`; `tags_input().listbox_filter()` and its reach
  into the command's crate-private context are gone.
- Render is pure in the stress scenes and docs pages too: stress `churn` makes each card an instance whose
  `Signal` is made in setup (its render used to make them, so a cold render reverted a clicked checkbox); the docs
  trace chat keeps its rows' open states in one `Signal<HashMap>` made with the chat, read in render and written by
  `on_change`, instead of making a row's signal the first time render asked for it.
- `Icon::drawing` reads each icon's SVG the first time that icon is drawn, not the whole set at the first icon (about
  4 000 allocations in one frame): stress `toasts` passes its action budget run alone (1 763 for the burst; it was
  5 909) as in the full run.

| Measure | Before | After |
| --- | --- | --- |
| Instances a stress `grid` click renders | the whole app | 2 (the button, the count label) |
| Frame with nothing changed | full rebuild | 0 renders (`tests/components.rs`) |
| Docs bench, calendar steady p95 | 15.6 ms (build 1.57 ms) | 8.05 ms (build 0.36 ms) |
| Incremental vs cold, 85 pages × monkey 80, docs shell × 12 seeds × 300, docs trace, 10 stress scenes | — | 0 differences |
| Verdict | 98/100, 5 FAIL | 98/100, the same 5 FAIL |

### Per node and phase (C4, 2026-11-21)

`crates/gui/src/layout/mod.rs` (the retained tree's marks and the in-place frame), `reactive/runtime.rs` (phases and
the late-write check), `paint.rs` (`record_changes`), `access.rs` (`Accessibility::patch`), `scroll.rs`, `motion.rs`;
tests in `layout/phases.rs`.

- **Marks per node and phase.** Each retained node holds a tracker per phase it reads signals in: resolve (its
  patch, its style, text and draw bindings) and place (what `frame_node` reads: a scroll handle's offset, a
  layout-bound value). A write marks only that node and phase (`need::RESOLVE`, `need::PLACE`) in a queue; the
  pass sets each mark's "below" summary up the parent chain, so patch and place walk only the paths that lead to
  work. Hover, press, focus and scroll changes in the UI state are marks too, found through a path and a focus
  index; a hover marks only nodes that refine their style by it. Unchanged element subtrees (`Element::ptr_eq`) are
  skipped. A child hidden by a binding (`display: none`) moves its reads to its parent, which builds it again.
- **The frame is written in place.** `place` rewrites a node's `FrameNode` only when it was marked, its box moved
  or its parent's placement changed, and logs it in `Frame::changes` (an epoch and a log; each reader keeps a
  `ReadAt`). Nodes that came, went or moved rebuild the frame from the layer they are in and start a new epoch.
- **Paint reads the log.** The painter keeps the scene, each node's property state, subtree end, chunk and thumb
  slot, and records only the logged nodes, the subtrees whose property state moved and the groups whose region
  holds them, writing property entries under an undo log; anything structural (a chunk appearing or going, a
  blinker or thumb added) falls back to the whole record. The commit is built from what it wrote
  (`Commit::changed`). A frame with an empty log visits no node.
- **Accessibility reads the log** (ADR-0021's as-built, C4 part).
- **Late writes.** Debug builds stamp each tracked read with the frame and the phase (render, layout, paint,
  accessibility) and report a write in a later phase of the same frame as a `LateWrite` naming the signal, where it
  was made and both phases (stderr, `FrameReport::late`, `reactive::take_late_writes`), as Chrome's lifecycle
  asserts. A layout pass another pass follows (`LAYOUT_PASSES`, as `ResizeObserver` loops) may write what render and
  layout read.
- **Cold check.** `GUI_COLD_CHECK=1` now compares the frame nodes, the scene (what the window shows, property ids
  read as values) and, when a screen reader is served, the kept accessibility tree with ones built cold; it renders
  every instance again without counting it in the frame's report.

| Measure (release; `stress --run`, docs `--bench`) | Before (M9 / M5b) | After |
| --- | --- | --- |
| Steady frame allocations on the UI thread, stress `grid` (2 500 buttons) | 25 000 | 67 |
| The same, `churn` / `random` | 10 800 / 9 100 | 57 / 61 |
| Steady frame of `grid`: nodes resolved, walked, placed, painted | every node | 0, 0, 0, 0 |
| A hover between two buttons of `grid` | the whole tree placed and painted | 4 resolved, 3 walked, 4 placed, 4 painted, 0 layout misses; 347 allocations |
| docs `--bench`, steady p95 median / worst page | 0.10 ms / 3.95–4.52 ms (`code-block`) | 0.02 ms / 0.37 ms (`code-block`) |
| docs `--bench`, `calendar` steady p95 | 8.05 ms | 0.16 ms |
| `GUI_COLD_CHECK=1`: docs monkey (seeds 1, 3, 7 × 300), `stress --a11y` (every scene, a11y included), `stress --run --monkey=100` | frame nodes only | frame, scene and a11y: 0 differences (`churn` included since its cards are instances) |

The steady frame's time on `grid` (about 1.5 ms of "paint" in `stress`) was the headless GPU renderer's frame plan,
which hashed every chunk's signature each frame. The renderer now plans from what changed (ADR-0016, "The render side
plans from what changed"): a steady `grid` frame costs 0.00 ms p50 and signs no chunk.

**Content moved onto a layer keeps its identity (2026-11-21).** As Vue's `Teleport`, a `deferred` layer adds no step
to the path its content's instances are known by, and a keyed child is matched by its key through the layer
(`component::child_step`). Content that moves between its place in the flow and an overlay keeps its instances, their
state and their scopes (`content_moved_onto_a_layer_and_back_keeps_its_instances`). shadcn's `select` renders the same
list open and closed (hidden while closed, on a layer while open), so its items are set up once
(`opening_and_closing_keeps_the_items_instances`: open, close, open: 0 setups, 0 unmounts after the first); a
`popover_content(..).force_mount()` (reka's `forceMount`) stays mounted while closed and keeps its state
(`content_kept_mounted_keeps_its_state_across_open_and_close`).

## Consequences

Expected; this is not built yet.

- **Every pattern of §4.1 goes away by construction.** No `_effect` fields, no `#[derive(Clone)]` state, no `&state`
  threaded to every part. `FormField` resets through `Model::bind`. Dialog restores focus from a `watch` on its
  `open` model, so a parent writing the model also restores it (today `dialog.rs:65` skips that).
- **Each `crates/shadcn` component is rewritten in the same change,** together with docs pages and stress scenes.
  `take_changed` is removed. The docs app's per-page `Render` cache (`docs/mod.rs:150,228`) and demo state map
  (`docs/component.rs:45`) become instances that unmount.
- **A click in stress `grid` should rerun one button instance and one paint chunk,** not the app. Benches before and
  after are the proof.
- **Incremental equals cold.** Every scenario step must give the same tree, layout, scene and a11y as a cold rebuild.
  This test runs from here on.
- **Devtools gains "why did this render"**: which write dirtied which instance, node and phase.

## Revisit when

- A component needs its state to survive moving to another parent: a keyed move, as Flutter's `GlobalKey` and
  Compose's `movableContentOf` do.
- Equal-props skipping misses in profiles because a prop type compares deeply: give it a cheaper `changed`.
- `Idle` work commits halfway or input waits behind it.
- Bindings by closure prove too coarse or too fine for a component class.
