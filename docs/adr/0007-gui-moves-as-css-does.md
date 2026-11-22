# 0007. `gui` moves as CSS does: transitions, keyframe animations and presence

Status: accepted, 2026-11-20; its time source (the 16 ms frame clock) superseded by ADR-0015. Extends ADR-0003 (effects paint as groups).

## Context

The shadcn-vue components move: a popover fades and zooms in from its trigger, a dialog's scrim fades, a sheet slides in
over 500ms and out over 300ms, an accordion section rolls open to its content's height, a switch's thumb slides, a button
eases its hover colour. All of it is CSS: Tailwind's `transition-*`, `duration-*` and `ease-*`, tw-animate-css's
`animate-in`/`animate-out` with `fade-*`, `zoom-*` and `slide-*`, `animate-accordion-*`, `animate-spin`, `animate-pulse`,
and reka's `Presence`, which keeps a closing part mounted until its exit animation ends. `gui` drew each state as a still:
nothing moved, a closed part vanished at once, and the loops (spinner, skeleton, shimmer) took their phase from the
absolute clock, so a still of a page depended on when it was taken.

Opacity was painted per node, each box and glyph multiplied by its ancestors' opacity. CSS makes an element with
`opacity` below 1 a group: it and its children paint together, then composite at that opacity. Per node, a fading dialog
showed its page through its own text and its buttons through its own surface; with every enter and exit a fade, that
showed in every frame of every animation.

## Options

1. **Each component animates itself** with signals and timers. Every component would own a clock, a state machine and
   a teardown; a closed part could not stay painted after its owner stopped rendering it.
2. **A motion system in `gui`, keyed by node identity**, that components declare in their style as CSS declares it in
   classes. One owner for time, interpolation and presence; a component only says what moves.

## Decision

Option 2.

- **Declared in style.** `Style.motion: Option<Box<MotionStyle>>` (boxed, as few nodes move) holds `transition`
  (`Properties`: colours, opacity, shadow, transform, width, height, margin, padding, inset), `duration`, `easing`,
  `delay`, `animation` and `exit`. `Styled` has Tailwind's names: `transition()`, `transition_all()`,
  `transition_colors()`, `transition_props(..)`, `duration_ms`, `ease_in_out()`, `animate(..)`, `animate_out(..)`.
  `Transform` (translate in `Calc`, scale, rotate, an origin, and a translate toward a layer's anchor) replaces
  `Style.rotate`.
- **`Animation` is `@keyframes` with its timing.** `Animation::enter()`/`exit()` are tw-animate's `animate-in`/`-out`
  (150ms `ease`, themed: the node's `duration-*` and `ease-*` win, as `var(--tw-duration)` lets them), edited by
  `fade`, `zoom`, `slide_x`, `slide_y` and `slide_from_anchor` (the four `data-[side=*]:slide-in-from-*` at once,
  resolved from the side the layer opened on, flips included). `accordion_down`/`up` (height 0 to the content's
  measured height), `spin`, `pulse`, `caret_blink`. Keyframes ease per segment; a missing end takes the node's own value;
  `iterations(n)` with a fraction stops part way, as `animation-iteration-count` does. `after_mount()` is reka's
  collapsible: content open as its item mounts shows at once.
- **Identity is the node path.** `Motion` (owned by the pipeline) keeps per path the transitions' runs, the playing
  animation and the measured natural height. Sampling happens while building, once the style is resolved
  (`Motion::sample`), so layout sees the animated height and paint the animated transform. Transitions follow
  css-transitions-1, including the reversing shortening factor.
- **Presence.** A node that leaves while it has an exit animation is kept: `Motion::finish` clones the topmost subtree
  that left from the previous frame's tree, makes it inert (no handlers, no focus, the pointer passes through) and
  `Motion::splice` puts it back in its slot until its exit ends; it holds its last pose (`forwards`). A node coming back
  takes its slot and the leaving copy is dropped.
- **Paint.** Translate moves the box (the pointer follows, as in CSS). Scale and rotate paint the subtree as a group
  composited through the transform about its origin (`Origin::Anchor` is reka's
  `--reka-*-content-transform-origin`). A node whose opacity is below 1 paints as a group composited at that opacity, as
  CSS does; a fully transparent group paints nothing.
- **Time.** The window runs on the frame clock and asks for frames while anything moves. `RunOptions.reduced_motion`
  (an app's `--reduce-motion`) is `prefers-reduced-motion: reduce`: no transitions or animations, loops still. A
  `Scenario` takes frames of states, every finite motion ended as it starts (`Motion::set_instant`, Web Animations'
  `finish()`), loops playing; `Scenario::play_motion` lets motion take its time on the scenario's clock, for frames part
  way through one, and `still_motion` ends what runs and goes back. `Scenario::rest` brings a page to rest as a browser
  still is taken: pending timers within a limit run, finite motion ends, loops hold their first frame. The static
  verdict judges each page at rest, and the reference waits for the timers its demos set while mounting.
- **shadcn.** Each component declares the motion its shadcn-vue source does, with the same durations, easings and
  distances (`crates/shadcn/src/motion.rs` holds the popper and scrim ones written once). Sonner's toasts move as
  vue-sonner's CSS moves them: rising in over 400ms, sinking out for the 200ms before it unmounts, sliding between front
  and behind in one grid cell.

## Consequences

- A component gets motion by declaring it; nothing else in it changes. Frames part way through an animation match
  Chrome's paused at the same time (dialog, sheet, drawer, popover, select, dropdown menu, menubar, combobox, tooltip,
  navigation menu, accordion, switch, progress compared at 0, 75, 150 and 300ms or the component's own times).
- Opacity below 1 costs a layer per group, cleared and composited only over the group's region.
- Every scripted frame shows a state, as before, unless the script plays motion.

## Revisit when

- **Reading the system's reduced-motion setting.** `unsafe_code` is forbidden and spawning `defaults` is a disallowed
  method, so `prefers-reduced-motion` is an app flag. A safe binding for the platform's accessibility setting (or winit
  exposing it) lets `run` read it once and on change.
- **Exits decided after leaving.** A leaving copy keeps the style it had when it left. navigation-menu's
  `data-[motion=to-start]` and `to-end`, which pick the old panel's exit by where the next one is, cannot be followed;
  the old panel fades and zooms out without sliding.
- **Hit-testing under scale and rotate.** The pointer tests the untransformed box; a scaled popper is hit as its final
  size during its 150ms.
- **Layout animation.** Only declared properties move; a box that moves because its siblings changed jumps. Sonner's
  expand and collapse, and a toast's height, jump where vue-sonner's slide.
- **Group cost.** A page with many translucent nodes composites as many layers; if it shows in frame timings, paint a
  leaf with no overlapping parts directly at its opacity.
- **Paused frames.** A reference frame taken at or past an exit's end shows the part gone (reka unmounts on
  `animationend`), so comparisons stop before it.
