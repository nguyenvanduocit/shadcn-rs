# 0010. `gui` is debugged from the command line

Status: accepted, 2026-11-20. Extends ADR-0008 (what a frame measures) and ADR-0009 (the accessibility tree).

## Context

`gui` measured every frame (ADR-0008), but reading what it measured meant a scripted `Scenario` writing files, the
bench, or the F10 HUD and F12 overlay painted in the window. Nothing answered a question about one running app: which
node is under this point, what does this button do, what value does that signal hold now, what did this click change.
A browser answers those in its devtools (Vue DevTools: a component tree, state shown and edited, a timeline). Those are
panels for a person with a mouse. The work here is done more and more by agents, which read and write text and
cannot use a panel.

## Options

1. **An inspector panel in the window** (a component tree, a state editor, a timeline), as Vue DevTools has. A person
   can use it; an agent cannot. It is also a large UI to keep working inside the framework it inspects.
2. **A remote protocol** (Chrome DevTools Protocol, JSON over WebSocket) for an existing front end. Much to build
   for one client, and an agent still needs a client.
3. **Commands on the command line**: one plain-text vocabulary to read, measure and act on a frame. It runs headless
   on a `Scenario`, or attached to a live window over a local socket. The window keeps only a small HUD.

## Decision

Option 3. The user asked for it: "only the CLI, as complete as possible, so an agent can work; the UI only simple
(updates, memory, CPU, components), not heavy or complex".

- **One vocabulary, two hosts.** `gui::devtools::execute` runs a split command line against a `Target` (a
  `Pipeline`). A headless `Session` (over a `Scenario`, its clock scripted) and a live window's `Attached` answer
  it the same way. A command returns `Done`: text, input that waits for the next frame's report, a frame, a
  screenshot, or a headless-only verb (`wait`, `rest`, `size`, `bench`).
- **Inspect**: `tree`, `components` (the named nodes, nested), `node` (box, look, text, behaviour, role, the named
  nodes it is inside, its children), `find`, `at`, `a11y` (the AccessKit tree of ADR-0009, unnamed containers folded,
  each node with its path and actions), `focus`, `signals`/`signal`/`set`, `graph`.
- **Measure**: `report` (the logged `FrameReport`s, the last 120), `perf` (total, build, layout and paint p50, p95 and
  max, allocations and effect runs per frame, frames over 16 ms), `memory` (the process's resident memory and CPU time,
  the heap, cached images, the graph, the tree), `diff`, `bench`, and `monkey` (seeded random input, each step checked
  for a panic, a failed layout and a slow frame, and the heap compared at rest before and after).
- **Act**: `click`, `rclick`, `hover`, `leave`, `drag`, `wheel`, `type`, `press` (chords such as `shift+tab` and
  `cmd+k`; `mod+` is the platform's shortcut key), and AccessKit's `activate`, `focus` and `setvalue`, which need
  no pointer. Every action draws the next frame and prints its report. A pointer action says which node the point
  reaches, and which node covers it when that is a different one.
- **Selectors are what an agent can see or reuse**: `@PATH` (a node's `NodePath`, printed by every command), `#NAME[:N]`,
  `X,Y`, or text (exact, a link span, then contained, any case, matched on text, input value, placeholder and accessible label).
- **Signals are read by label.** `Signal::labeled` now needs `T: Debug` and registers a weak probe
  (`reactive::probes`). The devtools list each labeled signal's id, label, type, `{:?}` value and readers, and
  `write_probe` writes numbers, `bool`, `char` and `String` through `Signal::set`, so subscribers run as for a click.
  shadcn's `SelectModel` and toasts, and the docs' page state, gained `Debug` for it. `Signal` itself is `Debug`.
- **Attaching is opt-in and local.** `DebugTools.attach` or `GUI_DEVTOOLS=1` (or a socket path) makes the window listen
  on a Unix socket, `$TMPDIR/gui-devtools-<pid>.sock`. A listener thread hands each line to the UI thread through a
  `mailbox` (which wakes the window), and the answer comes back within 10 s. An action's answer waits for the frame
  it causes; a screenshot is that frame's pixels, before the HUD is painted. `devtools::request` is the client, and
  `devtools::listening` finds the live sockets, newest first, removing stale ones. Attaching also turns on the reactive
  trace, so reports name the signals written and read.
- **The HUD stays small**: four lines. The frame, its p95, frames per second and build/layout/paint; the last update
  (its causes shortened, `click sidebar-menu-button`, and its signal writes); CPU percent over half a second, resident
  memory, the heap and cached images; nodes, components (named nodes), signals, computed values and effects. It paints
  again once a second while shown, so CPU and memory move when nothing else does. `process` reads CPU time and resident
  memory through `cpu-time` and `memory-stats` (MIT/Apache-2.0, safe APIs; the workspace forbids `unsafe`).
- **The docs example exposes it**: `--inspect <page|docs> [--size=WxH --scale=N] [-e CMD]…` (else stdin, `#`
  comments), `--serve [--hud]`, `--attach [--socket=PATH] [CMD…]`. A failed command prints `error: …`, and the run
  exits with 1.

## Consequences

- An agent checks a change by running it: `--inspect dialog -e 'click "Open Dialog"' -e a11y -e perf` shows what
  opened, what a screen reader hears, and what the frame cost. The first run already found one: the dialog's "Save
  changes" button has no click handler (nor has shadcn-vue's demo), so no role is implied, and a screen reader hears a
  label rather than a button.
- `Signal::labeled` needs `Debug`, so a labeled signal of a type without it no longer compiles. An unlabeled signal is
  counted but not shown.
- A live window that attaches keeps 120 frame reports (a layout diff each), and records the reactive trace every frame.
  A window that does not attach keeps only frame times, as before.
- The HUD paints once a second while shown. Hidden, it costs nothing.
- Attaching needs Unix sockets. On Windows the window says so on stderr, and the headless session still works.

## Found by the stress scenes

`examples/stress` and a `monkey 80` sweep over all 85 gallery pages found no panic and no failed layout. They found:

- **Group layers were never given back.** `Painter` kept every window-sized layer it had ever needed at once. After two
  nested groups painted (a fading popover inside a transformed box), the app held 2 × 9.4 MB at scale 2 for the rest
  of its life, which showed as +9.4 MB on most pages after the monkey. The pool now keeps as many layers as the last
  two frames had in use at once, the rule ADR-0008 set for the caches.
- **Text drew every line of a paragraph.** A long paragraph in a scroll container had every glyph placed, and dropped
  pixel by pixel by the clip mask. `text::Target.visible` (the clip's box, set only where the mask applies) now skips
  the lines wholly outside it, with a line's height of margin for ink outside the line box. The `text` scene's paint
  went from 25 to 14.5 ms. Every frame of 13 text-heavy pages and of `--docs-trace` is byte-identical.
- **A small group holds a window-sized layer.** A 16px close button at opacity 0.7, a disabled menu item at 0.5, or a
  12px chevron turned 180° each paints as a group into a layer the size of the window (9.4 MB at 960×640, scale 2),
  for as long as it is shown. `node` and `tree` now name why a node is a group (`group(opacity 0.70)`). Layers sized to
  the group's region, or opacity and right-angle turns drawn without a layer, are the fix; not done here.
- **Whole-tree rebuilds bound large screens.** 60 000 nodes (the 5 000-row table) take about 370 ms a frame, 345 ms of
  it layout, and a hover costs as much. 5 000 nodes (the grid) take about 90 ms. Nothing is incremental, and lists are
  not virtualised. This is the next performance work, measured by these scenes.
- **The devtools needed fixes.** A selector `#17` fell to a name that did not exist; it now falls back to the text.
  `setvalue "Customer name"` reached the label before the input; a screen-reader action now goes to the first node
  (or its nearest ancestor) that takes it. A node scrolled out of view could not be clicked; pointer actions now
  scroll it into view first (`reveal`). `monkey` says when the page ends showing other nodes than it began with (a menu left open), since
  the heap then holds what that shows. The devtools' own frame log made the leak checks report leaks: a report
  holds its layout diff and every signal written. `Session::forget_log` clears it before the heap is read.

## Revisit when

- A command's text is not enough for its consumer: add `--json` to the commands whose output is parsed, not to all.
- Someone wants component-level timing (build time per component). Components are functions that return elements,
  so this needs a scope marker at each component's root.
- Agents start apps on Windows (a named pipe or a localhost TCP port with a token), or attach from another machine,
  which this ADR does not allow: the socket is only for the local user.
- Labeled signals are too few to follow a bug, and every signal should be inspectable (a `Debug` bound on `Signal::new`).
