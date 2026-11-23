# primitives

Behaviour, no look. This crate is to `shadcn` what reka-ui is to shadcn-vue. It holds state, two-way values, parts
and their context, focus, keyboard and accessibility semantics. It draws nothing. Colours, sizes, corners and type
belong to the component that uses a primitive (`crates/shadcn`, ADR-0022). The rules are recorded in ADR-0025.

Crates run `gui` ← `primitives` ← `shadcn`. The crate keeps `forbid(unsafe_code)` and the workspace lints.

## Blocks

| Module | What it holds | reka source read |
|---|---|---|
| `availability` | `Availability`: enabled, unavailable (focusable, read as disabled, its clicks not run: `gui::Div::unavailable`), disabled | `disabled` props across reka |
| `context` | `use_root`: a part reads its root's state; outside its root it reports an error naming both | `shared/createContext` |
| `collection` | `Collection` (mounted items in tree order), `Items` (any list by index, `from_fn` for data), `step_in_list` (a highlight by data index over a virtual list) | `Collection/` |
| `keymap` | `Keymap`, `Intent`, `step`, `activates`: the one arrow-key rule and the wrap table | `shared/getFocusIntent`, `useArrowNavigation` |
| `roving` | `Roving`: a group that is one Tab stop, the stop bound to the node | `RovingFocus/` |
| `typeahead` | `Typeahead`: a 1000 ms search, `next_match` by index | `shared/useTypeahead` |
| `selection` | `choose_one`, `choose_many`, `range`, `Rows` | `shared/useSelectionBehavior`, `Listbox/` |
| `filter` | `Filter`: collator-based starts-with, ends-with and contains | `shared/useFilter` |
| `form` | `FormField`, `FormData`, `Submit`, checks, `form_submit`/`form_reset`/`form_data` over the caller's fields | `useFormControl`, `BubbleSelect` |
| `label`, `relation` | `Label`, `Linked`: typed `NodeRef` relations | `Label/`, `useId` users |
| `separator`, `visually_hidden` | AT semantics of a rule and of hidden text | `Separator/`, `VisuallyHidden/` |
| `presence`, `popper` | `Presence` (open, kept, gone), `Present` (its four stages, from the engine's `Div::on_stage`), `Popper` (a typed `Placement`, the arrow's place and hiding, the transform origin) | `Presence/`, `Popper/` |
| `direction` | `provide_direction` (one signal for keys and layout, `DirectionScope::root`), `use_direction` (read at each key) | `ConfigProvider`, `useDirection` |
| `time_field` | `Time` (the segments as the value, to its precision), `TimeField`: the segment core's time segments (locale order, hour cycle, steps, rounding, bounds) | `TimeField/`, Chrome's `<input type="time">` |
| `date` | `CalendarDate`, `CalendarSystem` (Gregorian, Persian, Hebrew, Islamic and others), `CalendarLocale` (en-US, vi, fa-IR: order, weeks, names, `DateStrings`), `month_weeks`, `week_number` | `shared/date/`, `@internationalized/date` |
| `calendar` | `CalendarGrid` (anchor, months, keys, one Tab stop), `Calendar` (one day or many), `CalendarOptions`, `Day` | `Calendar/`, `useCalendar` |
| `range_calendar` | `RangeCalendar`, `DateRange`, `RangeOptions` (non-contiguous, maximum days, fixed end): presses, preview, Escape | `RangeCalendar/` |
| `date_field` | `DateEntry` (partly typed segments), `DateField`: the locale's segments, Chrome's typing, a 12- or 24-hour clock | `DateField/`, `shared/date/useDateField` |
| `date_picker` | `DatePicker`, `DateRangePicker` (its two fields made over its one value): a field and a calendar over one value, the popup as an engine layer | `DatePicker/`, `DateRangePicker/` |
| `listbox` | `Listbox` (reka's `ListboxRoot`: parts, Tab entry, leave and return, a filter, a virtual list), `ListCore` (the highlight, Shift anchor, typeahead, veto; focus or active descendant), `Choice` (one or several), `Bound`, `Options` by index, `ListKey` | `Listbox/` |
| `select` | `Select`: options known while closed, the 10 px press, 50 ms scroll buttons, the anchor option, a modal list | `Select/` |
| `combobox`, `autocomplete` | `Combobox` (text and term apart, `ComboboxConfig`, a field or a button as the way in), `Autocomplete` (the value is the text) | `Combobox/` |
| `command` | `Command`: sections filtered by collation over a virtual list, the field's active descendant, the keys on data | `Listbox/`, `ListboxFilter`, shadcn's `Command.vue` |
| `tags_input` | `TagsInput`: one adding rule, the tag keys from the field's start, delete buttons named for their action | `TagsInput/` |
| `tree` | `Tree`: rows of the open part by index, open and close by direction, propagated choice | `Tree/` |

## Conventions

Every primitive follows these rules.

1. **State is a `Copy` struct made once, in setup.** Examples are `Roving`, `Typeahead` and `TimeField`. It holds
   signals, not values, so render closures and event handlers capture it by copy. Do not make one during render.
2. **Values in and out use `gui::use_model`.** A part's two-way value is a `Bind<T>` prop read through `use_model`.
   The owner is decided once. An equal write is a no-op (ADR-0025 §1). A primitive never keeps its own copy of a
   caller's value next to `UseModel`: it reads and writes the `UseModel` itself (`TimeField::new(value, options, direction)`), so a
   caller keeping the value decides what is shown.
3. **A part is a function that takes the look's `Div` and returns it with behaviour added.** Examples are
   `Roving::item(d, item)`, `TimeField::segment(s, d)` and `Separator::apply(d)`. The look draws the node first; the
   primitive adds focus, keys, semantics and membership. A part never sets a colour, a size or a corner.
4. **Context goes through `provide` and `primitives::context::use_root`.** A root provides its state; its parts read
   it with `use_root::<State>("part", "root")` in setup. A part rendered outside its root is a reported error, as
   reka's `createContext` throws: it names the part, the root it needs and where the part was made, once per place in
   the source. The part then renders its look without behaviour. It does not panic: the workspace denies panics in
   library code, and the report shows the wiring bug as plainly as reka's throw (ADR-0025 §13). Nested roots are
   separate: each makes its own `Collection`, and items join the list they are given.
5. **Keys follow one handler per node.** `Div::on_key_down` replaces any earlier handler. A primitive that sets keys
   in a part documents it. A component that needs keys of its own on the same node sets them after the part and
   passes the remaining keys to the primitive (`Roving::key` returns `Nav::Moved`, `Stayed` or `Ignored`).
6. **One notion of "cannot be used".** Take an `Availability`, not a `bool`, and apply it with
   `Availability::apply`: a disabled node takes no focus, keys or clicks; an unavailable one keeps focus and keys,
   is read as disabled, and runs no click (`gui::Div::unavailable`). Navigation, typeahead, selection and forms read
   the same value; a `FormField` takes its control's own (`FormField::availability`, a value or a closure), never a
   second copy. A disabled field is never checked: not on submit, blur or change (a change made while disabled waits),
   and it has no errors, is not invalid and is valid. Enabled again, it is checked as its `OnChange` says: `Check`
   when it was checked before or its value changed meanwhile, `Recheck` when it was checked before, `Clear` clears
   when the value changed, `Keep` does nothing. "No click runs on it or through it" (ADR-0025 §2) is about
   activation, and the engine applies it once, for every handler: an unavailable node blocks activation on itself
   and through it (`on_click`, Enter and Space, a screen reader's Click), while a gesture an ancestor owns
   (`on_mouse_down`, `on_drag`, `on_touch`, `on_long_press`, `on_secondary_click`) passes it by to that ancestor.
   The reason is that `aria-disabled` changes what the element does, not where pointer events go: on the web they
   still bubble past it. A drawer or a toast swipes from a blocked day or an unavailable action, and a context menu
   opens over an unavailable item on a right click and on a long press alike. The unavailable node's own gesture
   handlers do not run, because they would act on it. A press still focuses it, since focus is not an action
   (`focus_on_press`, `gui::Div::press_focuses`). Disabled is inherited by the subtree, as `inert` is and as a
   disabled `<fieldset>` disables its controls (`gui::Div::disabled`): every node in it takes no focus (Tab passes
   it by), no keys and no activation, and is read as disabled. No handler of a node in it that acts runs, and a
   press in it moves no focus. What only follows the pointer still runs on a disabled or unavailable node, as
   Chrome sends `mouseover`, `mouseenter` and `mousemove` to a disabled control: `on_hover`, `on_pointer_move` and
   `on_wheel` (a disabled node's hovered flag stays off). An unavailable node is read
   as disabled with what it holds, as Chrome reads the descendants of an `aria-disabled` element, though those keep
   their focus and their own clicks. A gesture an enabled ancestor owns still reaches that ancestor through it, as Chrome sends
   pointer events on past a disabled form control: a drawer or a carousel drags from a disabled button inside it.
   Activation never passes a disabled node. A drag or a touch whose node becomes disabled or unavailable while it
   is held is cancelled: its handler gets `DragPhase::Cancel` or `TouchPhase::Cancelled` once and nothing after, and lets go
   without acting on a release (a slider writes no more, a drawer slides back, a number field stops repeating).
   The touch target is found once, as the contact goes down, and keeps every later phase (a captured pointer).
   A captured node is followed by its path, not its index: a drag source inside a list whose items come, go or
   move must be keyed (`key`), since an unkeyed rebuild reuses nodes by position, as Vue does, and the drag would
   follow whichever item came to its place. A tap is one gesture, recognised as it lifts and hit-tested once then,
   as Chrome's `GestureTap`: its focus, caret, press, drag start and end and click all go to the node under the
   tap point in the frame of that moment. A mouse click still runs only when released over the node pressed.
   This refines ADR-0025 §2, which names the levels but not inheritance or gestures through a disabled node. A
   segmented field (`DateField`, `TimeField`) that is unavailable or read-only moves focus between its segments
   and changes nothing. A press beside its segments focuses the first, as Chrome does for an `aria-disabled` time
   input.
7. **Lists are by index.** Anything that walks items (step, search, range) takes `&impl Items`. It then works on
   the mounted `Collection` and on a virtual list's data alike, without allocating per item.
8. **Relations are typed.** Link nodes with `NodeRef` (`Linked`, `Label`), never with generated id strings.
9. **Numbers and rules from upstream are cited where they are written.** Each module's docs name the reka source
   it ports and where it differs, and why.

## What the look reads

A primitive exposes tracked getters for what a look depends on: `UseModel::get`/`with`, `TimeField::value` (the
model's value) and `focused`, `Typeahead::search`, `Presence::is_open`, `Present::stage`, and `Item::active`. A
floating part's look reads `gui::Placement` and asks `Popper::arrow_at` and `Popper::transform_origin` for the arrow
and the origin. Hover, press and focus rings come from `gui::Interaction` on the node, not from a primitive.

What only the node needs (whether it is a Tab stop, its layout direction) is bound to the node with a closure, so a
change of it renders nothing: `Roving::item` binds `Div::tab_stop`, `DirectionScope::root` binds `direction`. A part
schedules nothing from render (render is pure): `Roving`'s stop is worked out when Tab looks, from the items as
layout last laid them out, not settled after each render.

## How shadcn uses a primitive

- The component makes the state in its setup and provides it in its context. Example: `radio_group` has
  `Roving::new(Keymap::RADIO_GROUP, use_direction(None))`.
- In render it draws its look, then hands the `Div` to the part:

  ```rust
  g.roving.item(root.aria_checked(checked), Item::new(value, focus).active(checked))
  ```

- A shadcn module re-exports the primitive types its parts take, so callers name one crate
  (`shadcn::form::FormField`).
- Behaviour fixes go in this crate. A look fix goes in `shadcn`. A component file that grows key handling, a
  selection rule or a typeahead of its own is a bug: move it here.
