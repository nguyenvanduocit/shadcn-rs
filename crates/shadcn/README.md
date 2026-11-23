# shadcn

shadcn/ui for `gui`, a native GUI in Rust (`crates/gui`): each component of shadcn-vue's
`new-york-v4` registry, ported to Rust with the same look and behaviour. The
source of truth is the Vue registry: `apps/v4/registry/new-york-v4/ui/<name>/`
in [unovue/shadcn-vue](https://github.com/unovue/shadcn-vue) (`verdict/fetch.sh`
keeps a pinned sparse clone in `verdict/.shadcn-vue/`); its demos, which the gallery pages follow, are
in `apps/v4/components/demo/<Name>Demo.vue` and `apps/v4/components/_internal/sink/`.

## How a component is ported

- **One file per component**, `src/<name>.rs`, re-exported from `lib.rs` with a
  glob. Prefix every public item with the component (`DialogProps`,
  `dialog_content`) so the globs never collide. Shared bits have one owner:
  colours in `theme.rs`, the focus ring in `focus.rs`, `Orientation` in
  `separator.rs`.
- **Each Vue part is a function returning a `gui` element** (usually `Div`),
  carrying the part's classes, so a caller adds more styles after it as Vue's
  `class` prop does: `card().max_w_px(384.0)`. Parts take what they render
  (a title string, an item's value) as arguments; children go in with `.child`.
- **Classes translate one for one** through `gui::Styled`: `px-4` → `.px(4.0)`,
  `h-9` → `.h(9.0)`, `w-[200px]` → `.w_px(200.0)`, `rounded-md` →
  `.rounded_md()`, `bg-primary/90` → `.bg(t.primary.opacity(0.9))`,
  `dark:` → `.when(t.dark, …)`. A state is a closure reading the node's
  `gui::Interaction` (ADR-0022): `hover:bg-accent` →
  `.background(move || i.hovered().then_some(t.accent))`, or the
  `shadcn::focus::StateLook` helpers (`background_while`, `focus_ring_on`,
  `dim_disabled`).
  Read the token colours from `let t = theme();`.
- **A stateful component is an instance holding its own state** (ADR-0013):
  `gui::component(props, setup)`, `setup` run once, its render tracked.
  There are no `XxxState` structs for callers to make and pass around.
  - **`v-model` is a `gui::Bind<T>` prop**, read in setup through
    `gui::use_model`. Each component has the same four builder methods, named
    after its Vue prop:
    - `.model(signal)` (`v-model`, `v-model:open` is `.open(signal)`): the
      caller keeps the value and both sides write it;
    - `.checked(v)` / `.value(v)` (`:model-value`): the caller shows a value
      it keeps, and a write only calls `on_change`;
    - `.default_*(v)` (`default-value`, `default-open`): the component keeps
      the value itself, from `v`;
    - `.on_change(f)` / `.on_open_change(f)` (`@update:…`): hears every write.
  - A leaf whose value the caller keeps as a plain value holds no state and
    renders straight into its caller, with no instance (`checkbox().checked(…)`
    in a table of thousands of rows).
  - Events are closures, held as `gui::Callback`s: they always compare equal
    and are never stale.
- **Compound parts are `crate::part::Part`s.** A part is a box its caller
  styles and fills as any `Div` (`dialog_content().max_w_px(425.0).child(…)`),
  given to an instance as its slot. A root `provide`s a `Copy` context in
  setup (its `UseModel`, signals, `gui::Members` for its items); each part
  `use_context`s it in setup and adds to its box what the state says: an open
  trigger's `aria-expanded`, a closed panel's absence. A write renders only
  the parts that read it.
  - `as-child` parts take the caller's element: `dialog_trigger(button(…))`.
  - Items join their root's list through `.member(&ctx.items, …)` in render,
    filled by layout in tree order. Nothing registers while rendering.
- **Classes a caller adds after a leaf's own state styles** go through
  `.class(|d| …)`, only where a demo needs one (look moves to `Design`
  values with ADR-0022), run inside the leaf's render: what it reads there
  renders the leaf again (`data-[state=checked]:bg-blue-600` is
  `.class(move |d| d.when(on.get(), |d| d.bg(blue)))`).
- **Watches, timers and effects belong to the instance's scope**: they are
  made in setup and disposed with it. No `_effect` fields.
- **Floating layers are the engine's** (ADR-0024): an overlay's region is
  `.layer(Layer::new().modal(…).dismiss(DismissPolicy::…).focus_scope(…)
  .on_dismiss(…))`. The engine sends Escape to the top layer only (never
  while an input method composes), asks with a typed `DismissReason`
  (Escape, press outside, focus outside, scroll, `LayerHandle::dismiss`),
  lets the owner veto, and keeps a modal's hit testing, wheel and reader
  tree to the layer. Its `FocusScope` focuses into the content as it opens
  and gives focus back as it closes unless focus moved since. So a caller
  writing the model closes it as correctly as Escape does; no overlay
  remembers focus or handles Escape itself.
- **App-wide services are reached, not threaded**: a toast is raised with
  `toast(…)` / `toast::success(…)` from anywhere once a `toaster()` is
  placed, as vue-sonner's are.
- **Behaviour comes from `gui`, not hand-rolled**: `.focusable()` (Tab order,
  Enter/Space press a focused `on_click`), `.on_key_down` (arrows),
  `.layer(…)` (dismiss, modal, focus scope), `.inert(…)`, `gui::deferred(Placement::…, …)` for anything
  that floats (popover, menu, dialog, toast), `.on_drag` (slider, resizable),
  `.overflow_y_scroll()` (scroll area), `gui::clock::{now, after}` (spinner,
  skeleton pulse, tooltip delay, toast timeout), `gui::shapes` (chart),
  `gui::image` (avatar), `gui::text_input` with a `TextField` (inputs).
  When `gui` lacks something a component needs, say so in your report; do not
  build around it.
- **Icons** are `gui::icon(Icon::…)` (lucide, 16px by default as shadcn's
  `size-4`); the embedded set is `crates/gui/assets/icons/`.

## Seeing it

Every component has a gallery page, `examples/docs/pages/<name>.rs`:
`render()` builds the page's state and returns its render closure,
`script()` drives it (hover, click, type, press keys) and takes frames.

```sh
cargo run -p shadcn --example docs -- --shots /tmp/shots <name>          # light
cargo run -p shadcn --example docs -- --shots /tmp/shots-dark <name> --dark
```

Each step writes `NN-step.png` (2× scale), `NN-step.overlay.png` (boxes and
what changed) and `NN-step.txt` (frame report and layout tree). Read the PNGs
and compare with the shadcn docs page for the component.

Some pages show no single component but what shadcn-vue's docs build from
several: the agent pieces (chat, thread, reasoning, plan, tool, subagent,
terminal, stack trace, test results, prompt input, markdown), the form
libraries' demos (`form_formisch`, `form_tanstack`, `form_vee`,
`form_cards`, `form_stepper`), file tree, scroll fade and typography. A
page's lead in the docs is its component module's `//!` doc, or the page's
own when it has no module, so every page starts with one.

## The docs

```sh
cargo run -p shadcn --example docs                          # the docs, in a window
cargo run -p shadcn --example docs -- --docs-trace /tmp/docs  # walked headless, a PNG per step
```

The window is ui.shadcn.com/docs for this crate: Getting Started (this
README, the theme's tokens light and dark, typography, and each demo's
verdict score) and every component A–Z. A component's page is its gallery
page: the lead is its module's `//!` doc, and each `demo(…)` becomes a
Preview | Code block, the code cut from the page's own source, with its
latest verdict. ⌘K (Ctrl+K) searches; the sun and moon switch the theme.

## Measuring it

Headless, from the command line, on a release build:

```sh
crates/shadcn/bench.sh [page…] [--budget=16 --frames=30] [--dark]
```

`bench.sh` builds the docs example in release and runs `docs --bench`, which
renders each page headless at the shots' size (960×640 at 2×) and prints a
line per page: the steady frames' p50 and p95 (three rounds of `--frames`,
and the fastest round's median), build, layout and paint, the script's
frames, allocations per frame, what the page holds (nodes, cached images,
heap), and two pointer walks over its targets. A page `FAIL`s when its
steady p95 passes the budget, or when the second walk leaves more than
256 KB held past the first (a leak: the page shows what it showed).

Each set of pages keeps its runs in `/tmp/shadcn-bench[-dark]/<pages>/`:
`bench.txt`, `bench.json` (one object per page) and `bench.prev.json`, the
run before, which the new run is compared with (`--baseline`). A page whose
fastest round moved more than 40% and 1 ms is named `SLOWER` or `FASTER`; a
slower one is measured twice more and the fastest kept, since a whole run can
land on an efficiency core, and a page still slower fails the run. So run
`bench.sh <pages>` before a change and again after it.

Each round also counts retired instructions per frame where the OS allows it
(`gui::instructions`, ADR-0023: the thread on Linux through `perf_event_open`,
the process on macOS through `proc_pid_rusage`). When both runs have them, the
comparison is by instructions (past 5%), which a core's speed does not move;
the first line of the run says whether they are counted.

Everything is painted on the GPU (ADR-0011): windows, and headless frames as
well. A measuring scenario (`bench.sh`) waits for each frame to be drawn and
reads nothing back, so it measures what a window costs. `docs --shots` and
`--verdict` read every frame back, so the verdict judges the GPU's pixels.
There is no CPU renderer: a machine without a GPU paints through a software
Vulkan driver (Mesa's lavapipe, as CI does). The HUD's
and `perf`'s `vsync` is the time a GPU window waits for the display to take
the next frame. It is not counted in `paint`.

Two more headless views of the same numbers:

- `docs --shots <dir> <page>` (and any app's `Scenario`) write each
  frame's report (`NN-step.txt`, `trace.txt`): its causes, signals, times,
  layout changes and what it holds (`holds`).
- Where the time goes: `docs --bench <page> --frames=1500 &` then macOS
  `sample <pid> 8 -file /tmp/page.sample` ("Sort by top of stack").

In a window, `cargo run --release -p shadcn --example docs -- --hud` (or F10
in any `gui` window) shows a four-line HUD: the frame time, its p95, frames
per second and build/layout/paint; the last update (its cause and signal
writes); CPU and memory (resident, heap, cached images); nodes, components
and the reactive graph, and under them a bar per frame for the last 120
(green within 16.7 ms, orange within two frames, red past). F12's overlay
flashes where paint recorded again (green) and what it only re-composited
(teal). ADR-0008 and ADR-0023 record what is measured and why.

## Debugging it: the devtools

Everything a frame holds is read, measured and driven from the command line
(`gui::devtools`, ADR-0010), so a person or an agent debugs an app as text.
Headless, on a page or the whole docs:

```sh
D="cargo run -q --release -p shadcn --example docs --"
$D --inspect dialog -e 'click "Open Dialog"' -e a11y -e 'press esc' -e perf
$D --inspect docs < script.txt              # one command per line; `#` comments
$D --inspect button -e help                 # every command
```

Attached to a running app (`docs --serve`, or any `gui` app with
`DebugTools.attach` or `GUI_DEVTOOLS=1`), over a local socket:

```sh
$D --serve &                                # prints the socket it listens on
$D --attach components                      # the newest app listening
$D --attach -e 'click Button' -e 'screenshot /tmp/now.png' -e memory
```

| | Commands |
|---|---|
| Inspect | `tree [SEL] [--depth=N]`, `components`, `node SEL`, `find TEXT`, `at X Y`, `a11y`, `focus`, `signals [FILTER]`, `signal ID\|LABEL`, `set ID\|LABEL VALUE`, `graph` |
| Measure | `report [N]`, `perf`, `memory` (heap per process and thread, memory by owner), `counters [N]` (per phase), `paints`, `hangs`, `diff`, `bench [N]` and `monkey [N]` (headless) |
| Act | `click`, `rclick`, `hover`, `leave`, `reveal`, `drag`, `wheel DY [SEL]`, `type TEXT`, `press KEYS…` (`enter`, `shift+tab`, `cmd+k`), `activate`, `focus SEL`, `setvalue SEL TEXT` |
| Frames | `frame`, `screenshot PATH [--overlay]`, and headless `wait MS`, `rest`, `size W H` |

`monkey [N] [--seed=S]` (headless) sends N seeded random clicks, keys, text,
scrolls and waits, and after each one checks for a panic, a failed layout, a slow
frame and what the heap kept. `click`, `hover` and `drag` scroll their node into
view first, as `reveal SEL` does.

### Stress scenes

`examples/stress` pushes `gui` further than any gallery page does: a 5 000-row
table (60 000 nodes), a 1 600-entry tree and 120 nested boxes, 2 500 buttons,
5 000 signals with a chain of 2 000 computed values and 500 effects, a thread
sending 5 000 messages through a mailbox, 30 toasts at once, 60 KB of text in
five scripts, keyed shuffles, 300 cards mounted and unmounted, and an admin
dashboard, virtual lists of one and ten million rows, a million-row table
sorted and filtered off the UI thread, and a seeded random tree of 3 000
nodes (`gui::testing::RandomTree`). Each scene runs its script, 30 steady
frames and its cycle (the heap must give back what a round takes, measured
after two warm-up rounds), then the monkey. Each scene has an allocation
budget on the UI thread (ADR-0023): a steady frame's p95 and the costliest
script step; over it, the scene fails. CI runs
`--run --cycles=0 --monkey=0`:

```sh
cargo run --release -p shadcn --example stress -- --run [scene…] [--cycles=4 --monkey=150]
cargo run --release -p shadcn --example stress -- --inspect table -e 'click Amount' -e perf
cargo run --release -p shadcn --example stress -- --window dashboard   # HUD + devtools socket
```

A selector (`SEL`) is a path (`@0/2/k7`), a name (`#button`, `#button:2`), a
point (`120,48`) or text (exact, else contained). Each action draws the next
frame and prints its report: what handled it, the signals written and read,
the times, and every node added, removed, moved or repainted. Values of
signals made with `Signal::labeled` are read; numbers, `bool`, `char` and
`String` are written. A failed command prints `error: …` and makes the run
exit with 1.

## Judging it against shadcn-vue

Wrap each shadcn-vue demo a page reproduces in `demo("<File>.vue", …)` (the
file in `apps/v4/components/demo/`), one for one (`docs --demos [page…]`
lists them), then:

```sh
crates/shadcn/verdict/run.sh button          # or several pages; --dark for dark
```

`verdict/` is a small Vite app that mounts shadcn-vue's own demo (pinned
checkout from `verdict/fetch.sh`, its `themes.ts` neutral tokens, the GUI's
Inter and Geist Mono) at the same 768px width; `shoot.mjs` shoots it with
agent-browser at 2×, and `docs --verdict` crops both to the ink, lays ours
over theirs and scores them. The browser runs at the docs' instant and time
zone (`docs --clock`: noon UTC on 2026-11-20, the day the calendars show), so
a demo that shows today scores the same on any date. `fetch.sh` also caches, as PNG in the gitignored
`verdict/.photos/`, the photos the demos load (GitHub avatars, Unsplash); a
page shows one with `photo(name, filters)`, and without it the component's
own no-image state.

Each demo gets a **score from 0 to 100**: the share of its ink that matches,
less 5 points per logical pixel its size drifts past the tolerance. A faithful
port scores ≥ 90 (`PASS`, no need to look); 75–90 is `WARN` (read its
`side.png`); below 75 is `FAIL`, a port defect, and `run.sh` exits non-zero.
A demo listed in `verdict/better.tsv` with why ours is better grades `BETTER`.
The run ends with the mean score, the FAIL count and the tolerance it judged
with, and writes `/tmp/shadcn-verdict/<style>/<pages>[-dark][@<theme>]/verdict.json` (page, demo,
score, grade, sizes) for tools. `side.png`, per demo under
`<page>/<Demo>/` there, holds theirs, ours, ours at half opacity over
theirs, and theirs faded with differing pixels magenta, top to bottom.
`run.sh --style <name> --theme <spec>` judges a style (vega by default) in a theme
(`baseColor=zinc,theme=blue,radius=large`: shadcn's picker choices); the reference
renders the base components under `.style-<name>`, as shadcn-vue's previews do, and
`/tmp/shadcn-verdict/index.html` sets every style's report side by side
(`run.sh --help`).

The tolerance forgives how the two rasterisers differ, never the port; each
part is a flag of `run.sh`:

| Flag | Default | Forgives |
|---|---|---|
| `--color=N` | 48 | channel difference between two matching pixels |
| `--shift=N` | 1 | physical pixels a pixel may move (sub-pixel glyph placement) |
| `--size=N` | 1 | logical pixels the ink box may be off per axis |
| `--no-coverage` | (forgiven) | turns off forgiving ink of one colour at different coverage: Chrome's text gamma darkens glyph edges tiny-skia leaves light |

Missing or extra ink, a wrong colour, a shifted edge or a different radius
still differ. The reference app pauses looping animations (`animate-spin`) at
their start, as the gallery renders at t = 0.

Text itself is measured apart, line by line, on a sample of sizes from 11 to
160 px (`verdict/text-sample.tsv`, drawn by Chrome from `text-sample.html`
and by `gui`): `crates/shadcn/verdict/text.sh [outdir]` prints ink weight,
stem width, colour fringing, OKLab difference and glyph shift at 1× and 2×,
and writes the shots and `text-sample.tsv` (ADR-0019 has the thresholds).

- `NOREF` means the browser could not render that demo (`shoot.mjs` prints
  why, e.g. a demo importing Nuxt's `#components`).
