# AGENTS.md

## How we decide

This project is in its early stage and is meant to last. Build the best design
now, at its final shape. Do not ship a stepping stone and wait for it to prove
itself.

- **Best design now.** When research shows the right long-term design, build
  it in full. "Defer until measured" and "do the cheap version first" are not
  reasons to settle for less. Implementation is cheap; a wrong foundation is
  not.
- **No backward compatibility.** Any API, module or crate may be refactored,
  renamed or rewritten. No deprecation shims, no compatibility layers. Change
  every call site (`crates/gui`, `crates/shadcn`, examples) in the same change.
- **Resources are not a constraint.** Effort, code size and "good enough
  today" are never reasons to reject or defer a design. A rejection states
  its merit reason: a worse result, added latency, unsafety, incorrectness.
  Aim at what other toolkits consider out of reach: lists of millions of
  items, charts of millions of points, zoomable canvases, all at display rate.
- **Be the first to solve it.** Everything that exists carries old limits:
  web compatibility, a small team, or owning only one layer of the stack (an
  OS compositor that cannot see what an app draws, a layout engine that
  cannot see who read what, a renderer that cannot see the next frame). We
  own the whole stack, from signal to pixel, and have AI and abundant
  resources. Treat an open problem in the field (input-to-photon latency,
  deterministic replay, provable incrementality, typography beyond the
  browser, colour and HDR, energy, apps that AI agents can operate) as ours
  to solve first, not as a known limit to accept. Start from what only we
  know or own.
- **Measurement verifies, it does not gate.** Bench, stress and devtools
  (ADR-0008, ADR-0010) prove a design delivers and catch regressions. They are
  not a precondition for building the right thing.
- **Chrome is the reference, not the ceiling.** Output is judged against
  shadcn-vue in Chrome (`crates/shadcn/verdict`) to catch what we got wrong.
  Where we are better or more correct than Chrome (closer to the spec, better
  typography, crisper rendering), we keep ours and differ on purpose: the
  difference and why it is better are recorded (`verdict/better.tsv`, or an
  ADR for a rule). A difference that is not better is a bug.
- **Safety holds.** `forbid(unsafe_code)` and the denied lints in `Cargo.toml`
  stay. A dependency that relies on unsafe global state (for example the
  `alien-signals` crate's static arena) is read for its algorithm, not used.
- **No hot reload.** It is not a goal; nothing is designed around it.
- **Decisions are recorded.** A changed standing decision gets a new ADR that
  supersedes the old one (`docs/adr/README.md`).
- **Read the sources for their problems.** Reference implementations live in
  `references/`; their issue trackers and forks show where they broke. Read
  the matching ones before designing, to learn the failure modes, the
  evidence and the numbers. Their fix is one data point shaped by their
  limits, not the recommendation: design our own answer from what only we
  know, match their guarantees or exceed them, and write down why we differ.

## Architecture status

The sections below describe the target. This table says what exists today
(checked against the code on 2026-11-24). **Built** = in the code and tested;
**partial** = some of it is; **not yet** = does not exist. Do not rely on a
part marked partial or not yet without checking the code first, and update the
row when a milestone lands.

| Part | Status | Evidence, or what delivers it |
|---|---|---|
| Reactive core (ADR-0012) | built | `gui/src/reactive/`, `sum_tree.rs` |
| Render-and-diff components, scheduler (ADR-0013) | built, being replaced | `gui/src/component.rs`; K8 (M11) deletes it |
| Run-once components: `show`, `each`, `dynamic_view`, `NodeId` (ADR-0026) | not yet | M11 K4a-K4f; nothing of it in `gui/src` |
| Typed contexts and parts | not yet | M11 K6; `use_context` and `shadcn/src/part.rs` are the old form |
| Bindings (values or closures on setters) | partial | `gui/src/layout/binding.rs`; K1, K2 rework, K4b covers the a11y setters |
| Retained layout, `CacheTree`, parallel islands, virtual lists (ADR-0014) | built | `gui/src/layout/engine/mod.rs`, `gui/src/list/` |
| Typed one-pass layout (ADR-0027) | not yet | M12 L2-L3; the ADR is written and accepted (L1), nothing of it is in `gui/src`; Chrome fixtures in `gui/tests/chrome_one_pass/` |
| Paint, property trees, retained chunks, GPU atlases, GPU paths and data chunks (ADR-0016-0018) | built | `gui/src/paint.rs`, `scene.rs`, `gpu/`, `chunk/`, `canvas/` |
| Three threads, presentation time (ADR-0015) | built | `gui/src/compositor/thread.rs`, `clock.rs` |
| Text: Parley, skrifa, Slug (ADR-0019) | built | `gui/Cargo.toml`, `gui/src/text/`, `gpu/slug.rs` |
| Accessibility deltas (ADR-0021) | built | `gui/src/access.rs`; screen reader checks open |
| Layer stack, focus scopes (ADR-0024) | built | `gui/src/layer.rs`, `focus.rs` |
| `primitives` behaviour (ADR-0025) | partial | `crates/primitives`; S6b groups 3–6 and 8 have open findings or no review, rewritten per family at M11 K7 |
| `gui` style engine: `Interaction`, `Shape`, OKLCH, `ContentStyle` (ADR-0022 S1-S5) | built | `gui/src/interact.rs`, `shape.rs`, `color.rs`, `style.rs` |
| `Theme`, `Design`, `VEGA` | partial | `shadcn/src/theme/`, `design/vega.rs`; verdict vega is 64/100 until S9 |
| Seven more style consts | not yet | M10 S11 |
| `shadcn` components read `Design` through defaults; no `Color` constructor in components (lint) | not yet | M10 S8, per component family after M11 K4a-K4f, K6 and M12 L1-L3, alongside that family's K7; K8 follows the last family; `shadcn/clippy.toml` has no such ban, components still hold their values |
| Fork frame (ADR-0028): `ForkMap`, its undo log, the cold check | partial | `ForkMap` exists in every build (`gui/src/engine.rs`); its undo log and the cold check (`GUI_COLD_CHECK`, read in `app.rs`, `gpu/mod.rs`, `compositor/mod.rs`) are debug builds only |
| Fork in release, typed outbox, sampled check | not yet | M13 F2-F4 |
| Verification: allocation budgets, seeded tests, bench, devtools (ADR-0010, 0023) | built | `gui/tests/`, `gui/src/testing.rs`, `.github/workflows/ci.yml` |
| OS display boundary crate, MCP/intent surface | not yet | decided 2026-11-23; no crate or code, see the plan |

## Target architecture

The unit of "only redo what changed" is a node and a phase: a signal read while
building, laying out, painting or compositing a node invalidates only that
phase of that node.

### Reactive core (`crates/gui/src/reactive/`, `sum_tree.rs`)

Algorithm of alien-signals as Vue 3.6 adopted it (`references/vue-minor/packages/reactivity/src/system.ts`,
`references/alien-signals`, Rust reading aid `references/alien-signals-rs`).

- A `thread_local` arena of nodes and links with generational ids; links
  reused when a run reads in the same order; no `unsafe`.
- Flag-based push at write time, iterative (no recursion) propagate and
  dirty-check; computeds are lazy and drop their subscriptions when nothing
  watches them.
- Handles (`Signal`, `Computed`, `Effect`) are `Copy`. Ownership belongs to a
  `Scope` tree: disposing a scope disposes everything created in it. Using a
  disposed handle is a reported error naming where it was created (as Dioxus'
  generational-box). This supersedes ADR-0008's "forgets it when dropped".
- Invariants, each with a regression test (the alien-signals spec suite is
  ported too):
  1. Glitch-free: an effect never sees old and new values mixed; a diamond
     runs once.
  2. Fresh reads inside `batch`.
  3. Equality cutoff at signals and at computeds.
  4. No self-recursion, no nested flush, a recursion limit.
  5. Deterministic order: an owner runs before what it owns; siblings in
     subscription order.
  6. Panic safety: a panicking closure leaves the runtime consistent.
- API: `get`, `with` (borrow, no clone), `get_untracked`, `untracked`, `set`,
  `update(&mut T)`, `trigger`, `watch(old, new)`, `on_cleanup`, `batch`.
  Async stays outside the graph (a status signal fed through `mailbox`).

### Components and scheduling

Today's component model is render-and-diff (ADR-0013): `setup` once per
instance, a tracked render, a parent re-render that skips children with equal
props, and a scheduler by depth with `Input`/`Animation`/`Idle` priority
(`crates/gui/src/component.rs`). It is being replaced by run-once (ADR-0026,
M11): a component runs once and builds retained nodes, values change through
bindings, and structure changes only at `show`, `each` and `dynamic_view`.
Read `docs/adr/0026-a-component-runs-once.md` for the design; do not write new
code against render, `Part` or `on_updated` unless the task is in the old path.

Today, in the old model: identity is position + key + component type, and
the scheduler's queue is keyed by `Input`/`Animation`/`Idle` priority. Both go
with run-once: a keyed row keeps its scope and `NodeId`s when it moves, and
structure commits without priority lanes.

What holds in both models:

- A component's `Scope` owns its signals, effects, watches and timers.
- Fine-grained bindings: a setter takes a value or a closure
  (`impl Fn() -> T`), bound straight to the retained node and its phase (Vue
  Vapor, Floem, Leptos). Not every setter does yet (for example
  `Div::placeholder` takes `Into<String>`); K4b covers the rest.
- `Model<T>` for two-way binding; `provide`/`use_context` scoped to the tree.
- Input-driven updates stay synchronous.

### Layout (`crates/gui/src/layout/`, `crates/gui/src/list/`)

- A retained node tree that implements Taffy's low-level traits directly
  (Blitz-style), with our style read through Taffy's style traits (no copy),
  `calc()`, and a style diff before anything is dirtied.
- Relayout boundaries decided at layout time by Flutter's rule (size not used
  by the parent, sized by the parent, tight constraints), plus scroll
  containers and layers; per-node memo of Taffy's repeated measure probes.
- Our own `CacheTree`: a cache entry records which inputs the node read
  (parent size, percentages, available space) and matches on those only, so a
  sibling's size change does not relayout nodes that never read it.
- Each relayout boundary is an island with its own node arena; dirty islands
  lay out in parallel waves (shallow first) in safe Rust, and results are
  bit-identical across thread counts and job orders.
- Virtualized lists and tables for millions of rows: heights in a sum tree
  (offset to index in O(log n)), scroll position anchored as row index plus
  offset so nothing depends on `f32` beyond 2^24 px, no signal per item, keyed
  diff over the visible window only, sort and filter off the UI thread,
  selection as index ranges, accessibility through row count and row index.
- A debug mode lays out cold and compares with the cached result.

### Paint, composite, GPU (`crates/gui/src/scene.rs`, `paint.rs`, `gpu/`)

- Property trees (transform, clip, effect, scroll) as Chromium has them:
  scrolling and opacity or transform animation write one entry and
  re-composite, never re-layout or re-record. An offscreen layer is decided
  per case, not per group.
- Recorded paint chunks retained per node and replayed when clean; compact
  typed GPU instances; dirty-range uploads; cached bind groups; every pipeline
  built up front.
- Multi-page glyph and icon atlases (icons as masks, tinted in the shader).
- Vector paths (fills and strokes) are rasterized on the GPU with exact area
  coverage in tiles, not on the CPU. Static icons are rasterized by it once
  into the mask atlas; scaled or rotated ones draw as paths.
- The renderer requests the adapter's real limits and requires compute;
  there is no OpenGL backend.
- GPU data chunks: a chunk may own GPU-resident data, run compute passes
  (transform, culling, level of detail) and draw indirectly, while keeping
  painter order, clips, property-tree state and damage like any other chunk.
  Charts of millions of points and zoomable canvases are built on them.
- A persistent canvas redrawn only in damaged regions, then copied to the
  surface; `desired_maximum_frame_latency: 1`.
- Three threads. The platform (main) thread runs winit, AccessKit and IME
  and only forwards stamped events. The UI thread runs reactive, components,
  layout, paint recording and hit testing. The render thread owns the GPU,
  the retained chunks, the property trees and the atlases, and runs scroll,
  fling and compositor-only animations by itself. The UI thread commits only
  what changed; a late commit still gets a compositor-only frame, so latency
  stays one frame. Every input event carries what was on screen when it
  arrived, and hit testing uses that.
- Animations are sampled at the predicted present time from the platform's
  vsync source; pointer and scroll input are resampled at frame start.

### Text and accessibility

- Parley + fontique (inline boxes, per-script fallback, ICU line breaking and
  bidi, variable axes so `opsz` replaces ADR-0005's static Inter faces);
  shaping cached apart from line breaking. ADR-0005/0006's Blink rules are kept
  on top of it. A new ADR supersedes ADR-0001's cosmic-text basis.
- Glyphs are rasterized by our own code (skrifa) on every OS, not by
  CoreText or DirectWrite, so output is the same on every machine; per-OS
  parameters are copied from Chrome. Hinted coverage bitmaps for UI sizes,
  subpixel (LCD) coverage where the platform and an opaque background allow
  it as Chrome does, gamma and contrast decided by side-by-side comparison
  with Chrome; curve text on the GPU (Slug) for zoomed, scaled or very large
  text.
- AccessKit updates carry only changed nodes; text fields expose `TextRun`s.

### Styles (`crates/primitives`, `crates/shadcn`)

ADR-0022: a shadcn style is a typed value that components read, as Compose's
defaults read `MaterialTheme`.

- Three crates: `gui` ← `primitives` ← `shadcn`. `primitives` holds behaviour
  as reka-ui does (state, `v-model` props, parts and their context, focus)
  and no look. Each `shadcn` component draws itself once and reads its
  values through public defaults (`ButtonDefaults::height(d, size)`).
- `Theme` holds the tokens: scheme, palettes, radius, fonts, type scale,
  shadows, menu (`accent`, `inverted`, `translucent`), icons. `Design` is a
  style: about 250 named values (size, spacing, corners, text, focus,
  elevation, field, density, motion, colours, parts), no `Option`, one
  `const` per style: `VEGA` (the default), `NOVA`, `MAIA`, `LYRA`, `MIRA`,
  `LUMA`, `RHEA`, `SERA`. No const is built from another. new-york-v4 is
  retired.
- Port values, not CSS: setters take a value or a closure (`border(1.0)`,
  not `border_0`). No class names, hooks, `@apply` bundles, cascade,
  inheritance, or tooling that parses upstream CSS. A porter reads a
  style's values from upstream once; the verdict judges the pixels.
- Colours by state are data: `Ink`, `Tone { light, dark }`, `StateTones`
  (hover × expanded, as Chrome shows them), `ButtonColors`. The order
  between states (invalid before focus) is the component's code, written
  once. A component cannot create a `Color` (clippy lint).
- Each property has one owner, at three scopes: the app
  (`install(theme, design)`), a subtree (`with_theme`, `with_design`), an
  instance (typed params such as `variant`, `size`, `corner`, `colors`,
  `padding`, or a raw setter, which makes the instance the owner). No
  layers, no precedence between states.
- `Interaction` per node, one signal per flag, replaces hover and focus
  refinements. Text, icons and `currentColor` read the `ContentStyle` their
  container provides; only `Direction` still flows to every node.
- One signal per `Theme` and `Design` group, with equality cutoff (Flutter's
  `InheritedModel` aspects); bindings go straight to their phase. Hover, a
  theme switch or a style switch renders nothing.
- Verdict per style against shadcn-vue in Chrome; a deliberate difference
  goes in `better.tsv`.

### Verification

- Every scenario step: incremental result equals a cold rebuild.
- Seeded property tests of the reactive graph against a recompute-everything
  oracle.
- Allocation budgets per frame (counted per thread) in CI; bench and stress
  before and after every render-path change; verdict never lower.

## Where the plan lives

- Research and the reasons behind every decision above: `docs/research-2026-11-21.md`;
  for styles, `docs/research-2026-11-21-style.md`.
- The decisions themselves: ADR-0012 to ADR-0023 (`docs/adr/README.md`).
- Milestones, measurable goals and tasks: `docs/plan-2026-11-21.md`. A milestone
  is done when its goals are measured; its ADRs' status then changes from
  "not yet built" to the date.
