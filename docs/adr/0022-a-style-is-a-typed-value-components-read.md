# 0022. A style is a typed value that components read

Status: accepted, 2026-11-21; not yet built (M10). Supersedes, in part, the inherited text properties of ADR-0001
(what a span honours), ADR-0003 (`Ink::Current` against the inherited `ResolvedText`) and ADR-0005 (`tabular_nums`):
text, icons and `currentColor` read `ContentStyle` instead.

It replaces two things:
- the porting rules of `crates/shadcn/README.md`: "Classes translate one for one through `gui::Styled`" and
  "Behaviour comes from `gui`, not hand-rolled";
- `gui`'s Tailwind-named setters (`styled.rs`).

It keeps ADR-0004's direction, ADR-0007's motion, ADR-0013's bindings by phase, and ADR-0014's style diff.

Research, data and sources: `docs/research-2026-11-21-style.md`. An earlier draft of this ADR, written the same day,
made each style about 400 look functions, one per upstream hook. The user rejected it; Options says why.

## Context

**Today, behaviour, structure and look sit in one function per component.**
- `button.rs:35-116` builds the structure. It then picks colours in six variant arms, three of which branch on
  `t.dark`; destructive hard-codes `Color::WHITE`. It sizes eight size arms, four with `.has_icon(..)`, and repeats
  `focus_ring` in every arm.
- `accordion.rs:141` asks the caller to give the last item `.border_0()`.
- There is one look: new-york-v4's.
- `Theme` holds colours and `dark` only, and is read in render (`theme.rs:128`). A theme switch therefore re-renders
  every instance that read it.

**shadcn has leaked into `gui`.**
- Constants: `UNIT = 4.0` (Tailwind's spacing) and `RADIUS = 10.0` (shadcn's `--radius`), `styled.rs:15,19`.
- `Radius { times, plus }`, which is `calc(var(--radius) * k)`, and `radius_var`.
- `FontFamily::{Sans, Mono}`, fixed to Inter and Geist Mono.
- `has_icon`, which emulates `:has(> svg)`.
- A fixed set of pseudo-class refinements in `States`. They are resolved during layout by testing
  `state.hovered.contains(path)`, not per node and phase.
- 246 setters named after Tailwind classes (`rounded_md`, `text_sm`, `border_0`, and `shadow_xs`, which allocates a
  `vec!`).

**Upstream splits four tiers** (shadcn-vue `d1bf1b3`, the verdict's pin):
- **reka-ui** holds behaviour.
- **The base** (`registry/bases/reka/ui`, 68 components) holds the DOM. It carries 435 `cn-*` hooks.
- **Eight styles** (`registry/styles/style-<name>.css`, about 400 `.cn-x { @apply … }` rules each) hold the look.
- **Themes** hold the CSS variables: 7 base colours, 17 accents, `--radius`, fonts, menu accent and colour, icon
  library.

**What the eight styles differ in.** Research §3.8 has the data, from reading the style files once.
- They share one DOM. Of the 395 hooks all eight define, 99 are identical (412 hooks in all).
- Most values follow roles, not hooks:
  - **Height.** Each style has one height ladder (xs, sm, default, lg; four distinct ladders across the eight). It fits
    19 of the 29 sized hooks, and icon-only sizes are squares of it.
  - **Padding** is a second ladder, independent of height.
  - **Corners.** Eleven roles fit 91% of the radius cells. That holds once CSS's rule for overlapping radii is applied:
    luma's 3xl fields and 4xl buttons both render as stadiums.
  - **Text.** Twelve roles fit 95% of the text cells.
  - **Focus.** Each style has one ring (width 1 to 3, alpha 0.3 or 0.5), and sera underlines fields instead. This fits
    18 of the 34 focus-bearing hooks; 9 of the others are rings that the base hard-codes.
  - **Elevation.** A ring tone and a shadow step per surface role.
  - **Fields.** Four treatments: outline, tinted outline, filled, underline. `FieldKind` has three; the tint is
    `Field.fill`.
  - **Density.** Two container densities, mapped step by step rather than by one factor; menus and tables are separate
    axes.
- The rest is component-specific:
  - each surface's own corner (card, alert, empty state, code block…), and those of the chips (badge, kbd);
  - the geometry of switch, slider, checkbox, progress and kbd;
  - about 30 rules that switch structure rather than values: a boxed accordion, a floating drawer, footer bars, sera's
    accent-bar alerts and text-only badges, lyra's flush menus, the close button's offset and fill, joined or separate
    OTP slots.
- Motion is identical, except for which properties fields transition, rhea's 200 ms, and rhea's drawer overlay, which
  does not fade.

**CSS forced shadcn's engineering.** A stylesheet reaches a DOM only through class names, attributes and selectors.
So the base puts a hook on every part, and a style is an `@apply` bundle per hook. Precedence is cascade layers plus
twMerge, and inheritance carries colour and type. 8,390 of the 18,901 utilities use a variant to learn a fact the
base already knows. None of this is shadcn's value.

**The principle** (the user, 2026-11-21).
- Port the value, not the engineering that CSS, HTML or JS forced on shadcn. That rules out:
  - class names and hooks;
  - apply-style bundles;
  - cascade;
  - CSS inheritance;
  - tooling that parses or tracks upstream CSS.
- Setters take values (`border(w)`, not `border_0()`). Named steps are tokens. Facts arrive as typed state.
- Design as Compose and Flutter do, and judge by value and impact.
- Chrome and shadcn-vue judge the pixels, not the mechanism.

**How Compose and Flutter do it** (research §4–5).
- **A component is a function with typed parameters, and its defaults read the theme.** Compose has
  `Button(shape, colors, contentPadding)` with `ButtonDefaults`. Flutter has `widget ?? theme ?? defaults`, per
  property. Compose reads `CompositionLocal`s only "in the default expressions… so users can override them".
- **Appearance variants are typed values:**
  - `VisualDensity`;
  - `OutlineInputBorder` and `UnderlineInputBorder`, with `filled`;
  - `StadiumBorder` and `RoundedSuperellipseBorder`;
  - `Shape.createOutline(size, …)`;
  - `MotionScheme.standard()` and `expressive()`.
- **One implementation carries two design languages as values.** `MaterialExpressiveTheme` runs the same components
  with other motion, shapes and colours.
- **Colours by state are data with a resolver.** Examples are `ButtonColors.containerColor(enabled)` and
  `TextFieldColors`, resolved by `(enabled, isError, focused)`. The guidelines call this "not introducing the overhead
  and complexities of the 'styles' pattern".
- **Content colour and text style are explicit context.** Compose has `ProvideContentColorTextStyle`; Flutter has
  `DefaultTextStyle` and `IconTheme`. A container provides them, and `Text` and `Icon` read them.
- **State is typed** (`InteractionSource`, `WidgetState`). Indication and ink repaint without recomposing or rebuilding.
- **The guidelines:** "Prefer multiple components over style classes… Avoid grab-bag style parameters and classes,
  akin to `ComponentStyle`."
- **Both compile geometry into each component** (M3's component tokens, Flutter's `gen_defaults`), because each library
  carries one design language.
- **Both pay broadly for a theme change.** M3 keeps the whole theme in one static local, and `Theme.of` depends on all
  of `ThemeData`.
- **Elsewhere** (research §6):
  - Slint is retreating from one implementation per style.
  - Qt's StyleKit moves to one file per control plus style values.
  - String keys fail silently.
  - Routing each changed property to its phase is the common pattern.

## Options

1. **A CSS engine in `gui`**: selectors, cascade, specificity, class strings at run time. It adds a second
   invalidation mechanism, shows errors only at run time, and goes against the principle.
2. **Read upstream CSS with tooling**: generate Rust from it, or track it (pins per style, rule diffs, computed-style
   parity). The code keeps CSS's shape, and every upstream change becomes "port hook X". The user ruled it out.
3. **A look function per upstream hook, over a typed state per part, folded in layers.** This was this ADR's earlier
   draft. Each piece reproduces a CSS workaround:

   | Piece | CSS workaround |
   | --- | --- |
   | functions per hook | the stylesheet's layout |
   | a look writing a property bundle | `@apply` |
   | ordered layers | cascade |
   | inherited properties | CSS inheritance |
   | part names | `data-slot` |
   | computed-style parity | CSS's property model |

   It would also be about 3,200 functions (20–30k lines), plus a `State` struct for each of about 350 parts.
4. **A template per style** (Qt Quick Controls 2, Slint, WinUI): eight copies of structure and behaviour, where
   upstream guarantees one DOM. Slint is backing away from it.
5. **A style object per component**: Flutter's `ButtonStyle` with every field nullable, or Compose's experimental Styles
   API.
   - This is the grab-bag the Compose guidelines warn against.
   - Null means "fall through", so the code does not say who wins.
   - State layers are hidden specificity.
6. **Components written once over two typed values.**
   - A style is a `Design` and a theme is a `Theme`.
   - Each component reads both through its defaults.
   - Overrides are typed parameters, or values provided to a subtree.

## Decision

Option 6.

**Three crates**, in one dependency direction: `gui` ← `primitives` ← `shadcn` ← app.

| Crate | Holds | Must not hold |
| --- | --- | --- |
| `gui` | engine; setters taking values or closures; per-node `Interaction`; `Shape`; decorations; `Placement`; OKLCH; `ContentStyle` | tokens, named steps, any notion of style beyond a node's properties |
| `primitives` (new) | reka-style behaviour: state machines, keys, roving focus, typeahead, focus scope, dismiss, presence, collections, `Model<T>`, a11y roles and states, typed facts (`open`, `checked`, `side`, `highlighted`) | visual values |
| `shadcn` | components (structure, rendering, defaults); `Theme` and its presets; `Design` and the eight styles; icon sets as cargo features | colour literals in components |

The earlier draft needed a fourth crate to stop the base from writing look. That boundary no longer exists:
components are where rendering is written, once, and every visual value comes from data. A lint keeps colour literals
out of components (see Verification).

**`gui` takes values and closures, and knows no design system.**
- **Removed:** `UNIT`, `RADIUS`, `Radius { times }`, `radius_var`, `FontFamily::{Sans, Mono}`, every step-named setter
  (`rounded_md`, `text_sm`, `border_0`, `shadow_xs`…), `has_icon`, and the refinements of `States` (`.hover`,
  `.active`, `.focus`…).
- **Every setter takes a typed value or a closure** (ADR-0013): `border(1.0)`, `padding_x(10.0)`,
  `.background(move || …)`. A property has one owner.
- **New:**
  - **`Interaction` per node:** hovered, pressed, focused, focus-visible, focus-within, one signal each. Hit testing and
    focus write them; a binding that reads `focus_visible` does not rerun on hover.
  - **`Shape`:** `Rect`; `Rounded(Radii)`, a radius per corner, shrunk where radii overlap as CSS Backgrounds 3 §5.5
    does; `Stadium`; `Superellipse`.
    - One shape serves background, border, ring, shadow, clip and hit testing.
    - It is drawn as a path on the GPU (ADR-0017).
  - **Decorations:** borders per side, a focus ring outside the shape, an overlay border with blend, an inset layer
    (the floating drawer's panel), an edge bar (sera's alert), and `backdrop(filters)` clipped to the shape.
  - `hit_slop`; `Placement { side, align, available, anchor, origin }` from anchored layout; `Slide::FromAnchor`.
  - **OKLCH colour:** components, relative colours, and mixing in OKLCH as CSS Color 4 computes them.
  - **`ContentStyle`:** the colour, text style and icon size that a container provides to its subtree.
    - Only `text`, `icon`, text fields and `currentColor` (ADR-0003) read it, and only for what they are not given.
      A span within a paragraph falls back to its paragraph (ADR-0001); `tabular_nums` is a field of the provided
      text style (ADR-0005).
    - Nothing else flows down the tree, except ADR-0004's `Direction`, which layout reads at every node.
  - **Renamed so each name has one meaning:** today's `gui::Shape` (a path and its paint) becomes `gui::Figure`, the
    window-wide `gui::Interaction` becomes `gui::InputState`, and ADR-0003's `gui::Ink` becomes `gui::StopColor`.
    A position among siblings is a `Place`, since `gui::Position` is CSS `position`.
- **Writing a property allocates nothing.**

**A theme is tokens.** `Theme { scheme, colors: Palettes, radius, fonts, type_scale, shadows, menu, icons }`, built
with `Theme::new(BaseColor).accent(Accent)` (7 base colours × 17 accents).
- Named steps are tokens: colour roles, radius steps sm to 4xl, type steps, shadow steps, fonts. Counts are not
  tokens.
- `menu.accent: Bold` is a palette transform.
- `menu.inverted` makes menu content provide the dark scheme to its subtree, in both app schemes, as upstream's `.dark`
  does. `menu.translucent` is independent of it (upstream has four `menuColor` values); menu content and items read it.
- Icons are requested by role (`IconRole::ChevronDown`). Each library is a cargo feature.

**A style is a `Design`.** It is plain data that every component reads:

```rust
pub struct Design {
    pub name: &'static str,
    pub size: BySize<f32>,    // control heights xs, sm, default, lg; icon-only sizes are squares of them
    pub spacing: Spacing,     // control padding by size and beside an icon, gaps, glyph sizes
    pub corners: Corners,     // roles several components share: control by size and in a group, field,
                              // multi-line field, item, popup, modal, indicator
    pub text: TextRoles,      // about a dozen roles: size, line height, weight, case, tracking, font
    pub focus: Focus,         // ring width and alpha; underline fields show focus on their bottom border
    pub elevation: Elevation, // ring tone and shadow step of card, popup, sub-menu, modal, sheet; control shadow
    pub field: Field,         // kind: Outline | Filled | Underline; fill tones; padding
    pub density: Density,     // containers (card, dialog, alert, popover, sheet, empty, field group), menus, tables
    pub motion: Motion,       // duration; what fields transition
    pub colors: ColorUse,     // where styles pick other tones: outline fills, hovers, backdrop, separators, selection
    pub parts: Parts,         // what roles do not explain, one struct per component: corners, geometry, structure
}
pub const VEGA: Design = Design { /* … */ };   // and NOVA, MAIA, LYRA, MIRA, LUMA, RHEA, SERA
```

- **Every field is a typed value with a meaning:**
  - `Corner::{None, Step(RadiusStep), Plus(RadiusStep, f32), Capped(RadiusStep, f32), Px(f32), Full}`.
  - `Tone { light: Ink, dark: Ink }`. An `Ink` is a palette role, optionally with alpha, mixed with another role, the
    enclosing content colour, a fixed colour, or clear. `StateTones` holds a tone for each hover and expanded state.
  - `FieldKind`.
  - Enums for structure: `AccordionFrame::{Divided, Boxed { … }}`, `DrawerFrame::{Edge, Floating { … }}`,
    `AlertKind::{Plain, AccentBar}`, `BadgeKind::{Box, Text}`, `MenuFrame::{Padded, Flush}`,
    `OtpSlots::{Joined, Separate}`, `Footer::{Plain, Bar, Line}`.
- **No field is an `Option`.** Each const states every value, so adding a field breaks all eight until each has one.
- **No const is built from another** (`..NOVA` in `LYRA`): a fix to nova must not silently change lyra, and each
  upstream style sheet is complete too. A value shared by meaning gets a name (`Corners::SQUARE` for lyra and sera).
  An app's own `Design` may start from `..VEGA`; following vega is then the app's choice.
- **Values identical in all eight styles are not in `Design`.** The component writes them once (popup motion, the
  invalid ring's colours, avatar sizes).
- **An app's own style** is one more `Design` value.
- **Size.** Ported from research §3.8, a `Design` holds about 250 values, and each const is a few hundred lines.

**A component is written once.**
- Its structure follows the Reka base, and its behaviour comes from `primitives`.
- Each visual property is a binding. It reads `Design`, `Theme` and state through the component's public defaults
  (`ButtonDefaults::height(d, size)`, `ButtonDefaults::colors(d, variant)`), as Compose's `ButtonDefaults` reads the
  theme.
- Colours by state are data. `ButtonColors` holds fill and text as `StateTones`, the tone Chrome shows for each
  combination of hover and expanded, plus the border, the focused border and the ring. Which of invalid and focus wins
  is the component's code, written once.
- A structure choice is matched in the bindings it affects. The eight styles share one tree, so none of them changes
  the tree.

```rust
let (d, t, i) = (design(), theme(), Interaction::new());                      // Copy handles
let colors = move || colors.unwrap_or_else(|| ButtonDefaults::colors(d, variant));
primitives::pressable(i, on_click, disabled)
    .height(move || ButtonDefaults::height(d, size))                                            // layout
    .shape(move || t.shape(corner.unwrap_or_else(|| ButtonDefaults::corner(d, size, group)))) // paint
    .background(move || t.ink(colors().container.get(i, expanded)))                             // paint
    .provide_content(ContentStyle { color: Prop::bind(move || t.ink(colors().content.get(i, expanded))), /* … */ })
```

**Overrides: each property has one owner.**

| Scope | How |
| --- | --- |
| App | `install(theme, VEGA)` |
| Subtree | `with_theme(\|t\| …)`, `with_design(\|d\| …)`: a computed of the enclosing value, so an app-wide switch still reaches it |
| Instance | typed parameters (`variant`, `size`, `corner`, `colors`, `padding`); a raw setter (`.shape(..)`, `.background(..)`) makes the instance the owner of that property |

- A component binds its default only for the properties an instance did not set.
  - This is Flutter's `widget ?? theme ?? defaults`.
  - It is decided once, when the node is built, not by folding layers at run time.
- Typed parameters keep the behaviour by state: `colors` with its own `StateTones` keeps the style's border, text
  colour and focus. A raw `.background(BRAND)` does not, and the code says so.
- There are no layers, and there is no precedence between states.

**Reactivity.**
- **`Theme` and `Design` are provided as one signal per group.**
  - Theme groups: scheme, palettes, radius, fonts, type scale, shadows, menu, icons. Design groups: each field above,
    and each field of `parts`.
  - Writing a whole value writes each group. An unchanged group stops at equality cutoff.
  - This is Flutter's `InheritedModel` aspects, applied to the theme. Neither M3 nor `Theme.of` does this.
- **Each binding dirties its property's phase** (ADR-0013):
  - heights, padding and gaps → layout;
  - type → text shaping;
  - colours, shapes, rings, shadows and decorations → paint;
  - opacity and transform → property (ADR-0013's phase: the render thread's property trees; no chunk is re-recorded).
- **No component renders** on hover, focus, a theme switch or a style switch. Opening mounts an overlay's content
  once.
  - Light ↔ dark writes `scheme`; only the bindings that resolve a `Tone` (`t.ink`) rerun.
  - A radius edit reruns only shape bindings.
- **A choice that changes the tree** would be read in render, and would re-render only its component. None of the
  eight styles has one.
- **Comparable values.** Every value in `Design` and `Theme` is `PartialEq` and holds no function pointers.

**Vega is the default style, and new-york-v4 is retired as the reference.**
- shadcn/ui calls new-york-v4 "the legacy source registry".
- Its DOM differs from the base: one rotated chevron in the accordion, and a styled `DialogClose`.

**Verification.**
- **Verdict per style.**
  - The verdict renders the 389 demos with the unexpanded Reka base inside `.style-<name>`, with the eight style sheets
    imported into `layer(base)`.
  - It no longer maps demos to new-york-v4 (`verdict/vite.config.ts:19-24`).
  - Chrome reads that CSS; our code reads none.
  - Each style is scored, and no score may drop.
  - Where we differ on purpose (a fixed upstream bug, or nested corners made concentric if that is chosen),
    `better.tsv` says why.
- **Lint.** `crates/shadcn/clippy.toml` disallows `Color`'s constructors. Only the theme's palette module allows them,
  with a stated reason.
- **Tests.**
  - Hover reruns only the paint bindings of one node that read `hovered`, and renders nothing.
  - Light ↔ dark renders and lays out nothing. A style switch renders nothing.
  - A theme radius edit reruns only shape bindings and dirties only paint.
  - A subtree override follows an app-wide style switch.
  - Incremental equals cold under every style.
  - Binding reruns allocate 0 bytes.
- **Not used:** computed-style parity, upstream pins per style, a CSS-diff script.

## Consequences

Expected; this is not built yet.

- **Every `crates/shadcn` component is rewritten once, in its final shape:** a primitive, the component, and its
  defaults.
  - C6 (M2) makes each stateful component an instance (ADR-0013): its state, `v-model` props, parts and their
    context. S6 lifts that behaviour into `primitives`; S8 rewrites the look once, split into C6a–C6d's groups and
    agents.
  - The README's porting rules are rewritten.
- **Every `gui` call site changes with the setter rewrite:** `crates/shadcn`, the docs app, stress.
- **Behaviour moves out** of `crates/shadcn` and `gui::time_input`, into `primitives`. Each primitive is read against
  its reka-ui source first.
- **A style is data.** Eight consts of a few hundred lines replace the earlier draft's 3,200 look functions. A porter
  reads a style's values from upstream once, as with any reference, and the verdict judges the pixels.
- **The verdict's reference changes** from new-york-v4 components to base plus style sheet. Scores restart per style
  and are never lower afterwards.
- **Style and theme switch at run time,** with no re-render.
  - The docs app gets a switcher.
  - Devtools get `set style`, `set theme` and `set design <path> <value>`. Per property, they show its owner and the
    `Design` or `Theme` fields its binding read.
- **Upstream bugs found while reading are fixed, not ported,** and recorded in `better.tsv`:
  - lyra's `pr-28` on menubar checkbox items;
  - rhea's invalid `hsl()` around an oklch token;
  - mira's select and combobox items without room for the check;
  - boxed accordion triggers with no focus indication.
- **On acceptance (done 2026-11-21):**
  - AGENTS.md's "Target architecture" gains a styles section.
  - `docs/plan-2026-11-21.md` gains M10, tasks S1–S14 of the research document's §10.
  - The status lines of ADR-0001, ADR-0003 and ADR-0005 name the part that ADR-0022 supersedes.

## Revisit when

- **A style needs a different tree.** Then that component reads the choice in render, or the style gets its own
  component (Compose's "multiple components").
- **A new style, or a new upstream version, needs values `Design` cannot hold.** Add fields; there is no compatibility
  to keep.
- **Group signals show up in profiles**, for example live token editing over a large mounted tree. Then split the hot
  group.
- **A second design system built on `primitives`** needs a property `gui` lacks.
- **Upstream adds a base** (Base UI) whose structure differs from Reka's.
