# 0025. Primitives hold behaviour, one rule per concern

Status: accepted, 2026-11-22; built 2026-11-22 (M10 S6b, groups 1–2). Builds the `primitives` crate that ADR-0022
names (`gui` ← `primitives` ← `shadcn`). Supersedes nothing. It replaces the per-component keyboard code in
`crates/shadcn` (`radio_group::step`, `toggle_group::move_focus`/`focus_end`, `tabs::roving_target`,
`accordion::roving_target`, `dropdown_menu::{MenuNav, menu_nav_key, menu_step, menu_typeahead, menu_typed_char,
menu_picks}`), the form model in `shadcn::form`, and `gui::time_input`.

## Context

reka-ui (`references/reka-ui/packages/core/src`) writes behaviour once and leaves the look to its callers. Its
blocks are `Presence`, `Popper`, `RovingFocus`, `Collection`, `useTypeahead`, `useArrowNavigation`
(`getFocusIntent`, `wrapArray`), `useSelectionBehavior`, `useFilter`, `useVModel`, `Label`, `Separator`,
`VisuallyHidden`, `useDirection`, and the form helpers (`useFormControl`, the hidden `BubbleSelect`/`BubbleInput`
twins). Read side by side, they do not always agree. Each component decides wrap and PageUp/PageDown for itself.
Select adds Space to a running search, but Menu's content returns on Space before its search while its items skip
Space mid-search, so there Space does nothing during a search. Listbox leaves disabled items out of a Shift range,
but Tree keeps them. `useVModel` emits on every controlled write, even an equal one. The virtualised Listbox finds a
typeahead match by its string, so it always lands on the first of two items with the same label.

Before this change, `crates/shadcn` had five copies of arrow navigation and one of typeahead. Each copy had its own
rules. A radio group and a toggle group were one Tab stop per item instead of one per group.

## Decision

A new crate, `crates/primitives`, holds behaviour and no look. It has `forbid(unsafe_code)` and the workspace lints.
Its README holds the conventions every later primitive follows. Each concern has one rule, written once:

1. **The model (`gui::use_model`).**
   - Who owns a value is decided once, when the instance is made. A `Model` or a value passed by the caller makes the
     instance controlled. A default alone makes it uncontrolled.
   - A later caller value pushes into an uncontrolled instance's copy without calling `on_change`.
   - A `Model` a caller binds later to an uncontrolled instance is pushed in the same way, and the instance's writes
     then go to its copy and to that model, so the model keeps matching what is shown (Vue's passive `v-model`).
   - The one equality rule: a write equal to the current value is a no-op in both modes, by `T: PartialEq`. vueuse's
     controlled `useVModel` emits on every write; ours does not, so a parent never re-renders for nothing.
   - `UseModel::is_controlled` reports the decision.
2. **One notion of "cannot be used" (`Availability`).**
   - The levels are `Enabled`, `Unavailable` (focusable and announced, not actionable: `aria-disabled`) and
     `Disabled` (not focusable, not actionable).
   - `gui` gains `Div::unavailable` for the middle level, which `Availability::apply` sets. The node keeps focus and
     its key handlers, AccessKit reads it as disabled and offers no `Click`, and no click runs on it or, through it,
     on its ancestors: not the pointer's, not Enter or Space, not a screen reader's. On the web `aria-disabled` only
     tells; each widget then guards its own handlers, and a forgotten guard is a live control read as disabled.
     Here the engine guards once.
   - Navigation, typeahead, selection and forms all read the same value. A disabled form field is not submitted. A
     `FormField` takes its control's availability as a value or a closure (`FormField::availability`, as
     `Label::label` takes it), so the field and the control it checks cannot disagree.
3. **One keymap rule (`Keymap`).**
   - Arrow keys move along the primitive's axis. Right and Left swap under RTL.
   - Home and End go to the first and last usable item. PageUp and PageDown mean First and Last everywhere: reka's
     `getFocusIntent` maps them so for every caller that passes them, and one meaning beats a per-component gap.
   - Control, Alt and Command block navigation. Shift blocks it unless the primitive takes Shift ranges.
   - Disabled items are skipped, Home and End included. Unavailable items are reached.
   - Wrap is decided per primitive and recorded in the constants:
     - wraps: `TABS`, `RADIO_GROUP`, `TOGGLE_GROUP`, `TOOLBAR`, `ACCORDION` (reka's `loop` defaults);
     - stops at the ends: `ROVING` (`RovingFocusGroup`'s default), `MENUBAR`, `LIST` (menus, listbox, command), `LINE`.
4. **Roving focus (`Roving`).**
   - A group is one Tab stop. The stop is the focused item, else the active one (the checked radio, the selected
     tab), else the last focused, else the first usable item.
   - This is reka's entry order (`[activeItem, highlightedItem, currentItem, ...items]`). Entering by Shift+Tab
     lands on the same stop as entering by Tab.
   - `gui` gains `Div::tab_stop` (`tabindex=-1` when false): a node out of Tab order counts as a stop while it
     holds focus. It takes a value or a closure. Nothing is laid out, painted or announced from it, so it belongs to
     no phase: a closure runs, untracked, only when Tab looks for the next stop. `Roving::item` binds the stop so,
     and the items' render does not read it: moving focus in a group renders none of its items.
   - The stop is not kept in a signal settled after each render. It is worked out when Tab looks, from the items as
     layout last laid them out, and kept for that look (`gui::Members::pass`, the focused node and the last one key
     it), so one walk answers every item. An item's render then schedules nothing (render is pure), and the stop
     is never stale.
   - An item with keys of its own (Enter in a radio) sets its handler after `Roving::item` and passes the remaining
     keys to `Roving::key`.
5. **Collection by index.**
   - Mounted items are a `gui::Members` list in tree order.
   - Data lists (virtual lists of millions of rows) implement `Items` by index (`from_fn`), so navigation, typeahead
     and Shift ranges run on the data, not on the mounted rows (research §4 item 15).
   - `step_in_list` moves a highlight by data index over a `gui` virtual list and scrolls the row reached into view,
     built or not; the caller keeps that row built with `ListState::set_active` (`aria-activedescendant`). Command
     uses it.
6. **Typeahead (`Typeahead`).**
   - The search resets 1000 ms after the last key, on the frame clock. A repeated character cycles.
   - Matching is by index, so repeated labels stay apart.
   - Space is part of a running search and activates otherwise. This is Select's rule (`SelectContentImpl`), now
     Menu's too: reka's Menu drops Space during a search (`MenuContentImpl` returns before its search, `MenuItem`
     skips activation), so a label with a space cannot be typed there.
   - The buffer is cleared when a menu level closes.
7. **Selection.**
   - Toggle and Replace for one value and for many.
   - Shift ranges leave out disabled and unavailable items (Listbox's rule) and run on `gui::list::Selection`'s
     index ranges.
8. **Filter.**
   - Matching uses an ICU collator at primary strength on NFC text: case and accents aside, as reka's
     `Intl.Collator({ sensitivity: 'base' })`.
   - The locale is the caller's. Matching falls back to lowercase comparison when no collator data exists.
9. **Form.**
   - Moved from `shadcn::form`: `FormField`, checks, submit and reset, typed data (`FormData`) and the `Submit`
     trait. These replace reka's hidden twin inputs.
   - Reset puts every field back to its initial value, whoever owns it (a controlled field emits the change).
   - A form is the fields its caller lists: `form_submit`, `form_reset` and `form_data` take a slice of them, and
     there is no second, registry-based API. Fields are `Copy` handles made in the caller's setup, so the caller
     holds them already; what a submit checks (a whole form, a stepper's step, a dialog's fields) reads at the call
     site, and nothing is allocated or registered per field. A registry that fields join by scope was rejected: it
     makes membership depend on which scopes are alive, and a stepper then needs a form per step.
10. **Typed relations.**
    - `Linked` (trigger to content: controls or described-by, only while open) and `Label` (`labelled_by`, a press
      that focuses or activates the control) use `NodeRef`.
    - A label reads its control's `Availability` when clicked, as `<label>` does nothing for a disabled `<input>`: a
      disabled control is neither focused nor acted on, an unavailable one is focused and not acted on.
    - There are no generated id strings.
11. **Separator, VisuallyHidden, Presence, Popper, direction.**
    - A separator is decorative by default (hidden from AT, as shadcn passes `decorative`). Otherwise it is a
      splitter with its orientation (`gui` gains `aria_orientation`).
    - VisuallyHidden has two forms: hidden from sight, or hidden from AT as well.
    - Presence is `Open`, `Kept` (mounted, invisible, inert) or `Gone`. Exit animations stay the engine's
      (ADR-0024).
    - Presence's four milestones (reka's `enter`, `after-enter`, `leave`, `after-leave`) are `gui::Stage`:
      `Entering`, `Entered`, `Exiting`, `Exited`. The engine reports them from the part's own node
      (`gui::Div::on_stage`). `gui::motion` knows when that node's animations start and end (its children's aside,
      themed durations and reduced motion included), so nothing is guessed from a computed style. The report runs
      after the layout pass that reached each stage, in order, none skipped. `primitives::presence::Present` tells
      a caller the stages and holds the last one. A part shown again before its exit ends tells no `Exited`; it tells
      `Entering` again, as reka does. A kept part never leaves the tree, so `Present` tells its closing (`Exiting`,
      `Exited`, at once: it is hidden at once) and its showing again (`Entering`, `Entered`: no second entry
      animation). reka's quirk is not kept: reka tells no `leave` when the exit animation has the entry's name.
    - Popper is a typed `Placement`. The gap is `side_offset` plus the arrow's height, as reka's `PopperContent`
      adds it. `gui::Placement` gains `bounds`, the part's own box after flipping and shifting.
    - Popper's arrow follows floating-ui's `arrow` middleware (`Popper::arrow_at`). It sits on the edge facing the
      anchor, its centre on the anchor's centre, clamped to `arrow_padding` from the corners (at most half the edge
      less half the arrow less 1 px). Where the clamp moved it, it cannot point at the centre and is hidden, keeping
      its place (`hide_shifted_arrow`, default on).
    - The transform origin follows reka's `transformOrigin` (`Popper::transform_origin`). It is the arrow's tip
      while the arrow can point at the centre. Otherwise it is the aligned point of the facing edge (start 0 %,
      centre 50 %, end 100 %), with start and end swapped under RTL on a top or bottom edge.
    - floating-ui also moves a start- or end-aligned part when the anchor is narrower than the arrow plus its
      padding, so the arrow can still point (`shouldAddOffset`). `gui`'s placement does not, and the arrow hides
      there. No shadcn component aligns an arrowed part to the start or end of an anchor that narrow.
    - Direction is an instance's own value, else the nearest provided one, else LTR. It is read when a key arrives
      (`ReadDirection`), so switching it at run time turns every keymap and renders nothing.
    - One `Signal<Direction>` drives both halves of a subtree's direction. `provide_direction` gives it to the
      keymaps, and the `DirectionScope` it returns binds the subtree root's layout `direction` to the same signal
      (`DirectionScope::root`). Keys and layout cannot disagree.
12. **The time field types as Chrome does.**
    - `gui::time_input` moved to `primitives::time_field`, with its look in `shadcn::input`.
    - `TimeField` reads and writes the part's `UseModel` and keeps no copy of the value. A caller keeping the value
      that does not write a typed digit back keeps the segments as they were (research §2.2 item 12). The typing
      buffer counts only while its segment shows what it spelled, so a value written from outside starts a new
      number.
    - Its value is a `Time`: the three segments as they stand, a partly typed time included. The model holds what
      is shown, so a form reset or a caller's write (the empty time) always puts the segments back, as Chrome's
      `setValue` does for a partly typed field. `Time::value` is the input's `value` and what a form sends:
      `HH:MM:SS` once every segment has one, else empty.
    - Each segment is a spin button to AT, as Chrome's are. It is named for its part of the time, its value runs
      from 0 to the segment's largest, and it reads as shown ("09") or as "Blank" while empty. AccessKit's
      `Increment` and `Decrement` step it as ArrowUp and ArrowDown do (`gui::ValueRange`, `Div::aria_value_range`).
      The names are Chrome's English strings by default; an app gives its own (`SegmentNames`).
    - A digit joins the one before it in its segment, and the segment moves on after two digits or when no third
      could follow. "2" then "5" in the hour gives 23. Backspace clears the whole segment.
    - reka's TimeField gives 5 and deletes one digit at a time. The verdict compares shadcn's time input against
      Chrome's `<input type="time">`, and one rule keeps a time typed the same way everywhere.
13. **A part outside its root is a reported error.**
    - A part reads its root's state with `primitives::context::use_root`. reka's `createContext` throws, naming the
      root the part needs (research §3.2). Here the error is reported, once per place in the source, naming the
      part, the root and where the part was made, and the part renders its look without behaviour.
    - It does not panic: the workspace denies panics in library code, and a page with one misplaced trigger is
      still usable. The report shows the wiring bug as plainly as reka's throw does in a console.

## Consequences

- A radio group, toggle group and tabs list are one Tab stop each (previously one per item). Their keymaps are the
  table above. Accordion triggers stay a Tab stop each and gain PageUp/PageDown. Menus, select and command gain
  PageUp/PageDown and a 1000 ms typeahead buffer in place of one-character search.
- Pixels do not change. The accessibility tree changes in two places: a decorative separator is hidden from AT, and
  the time input's segments are spin buttons. `primitives::label::Label` gives its node `Role::Label`, but no
  shadcn component uses it yet (`shadcn::label` is still a plain node), so shadcn's tree does not change there.
- `shadcn` modules re-export the primitive types their parts take (`shadcn::form::FormField`).
- Menus read `use_direction`: under RTL ArrowLeft opens a submenu and moves to the next menubar menu, ArrowRight
  closes it (reka's `SUB_OPEN_KEYS`/`SUB_CLOSE_KEYS` and Menubar's `prevMenuKey`; research §4 item 13).
- Accordion items (and the accordion) and tabs triggers take `disabled`. A disabled trigger is skipped by the
  arrow keys, Home and End included, and takes no press.
- `shadcn::accordion` and `shadcn::tabs` read their roots with `use_root`; the other components adopt it as their
  groups port them.
- `shadcn::input_time`'s value is a `Time` (`model`, `default_value`, `on_change`); `default_value("10:30:00")`
  still reads its `HH:MM:SS`.
- Not in this change, because each belongs to the later group that ports its component: the overlays adopting
  `Popper`, `Present` and the arrow; `shadcn::label` and `field` adopting `Label`; a single `Orientation` type shared
  by shadcn and `gui::Axis`.

## Revisit when

- A primitive needs a wrap or PageUp/PageDown meaning other than the table above.
- ICU collator data proves too large for the binary-size budget.

## Group 7: dates (M10 S6b)

Built 2026-11-22. `primitives::date` (the day, calendar systems, locales), `calendar`, `range_calendar`,
`date_field`, `date_picker`, and `time_field` on the field's segment core. `shadcn::calendar`,
`shadcn::range_calendar` and `shadcn::input_time` keep only their look; `shadcn::calendar_system` and `calendar_locale` moved to
`primitives::date`. Tests: `crates/primitives/tests/dates.rs` (behaviour, and AT snapshots of the calendar, range calendar, date field, time field beside an invalid read-only date field, date picker and range picker), the time field's in `blocks.rs`, and the unit tests
of `primitives::date`.

1. **One grid, two values.** `CalendarGrid` holds the anchor (reka's placeholder), the shown months, the keys and
   the Tab stop; `Calendar` puts one day (`Option<CalendarDate>`) or many (`Vec<CalendarDate>`, in the order
   picked) on it, `RangeCalendar` a `DateRange`. The options are a closure read once per change (`Computed`), so a
   caller's `min`, `max`, predicates and locale are reactive without a signal each.
2. **Kinds of day.** Blocked (outside `min`/`max`, the caller's `is_blocked`, a disabled calendar, outside the view
   when asked) is skipped by arrows and takes no press; unavailable (`is_unavailable`) is reached by arrows and
   announced disabled but takes no press; a day of another month is shown and pressable unless the view hides it.
   Arrows step a day or a week (mirrored right to left); Home and End go to the week's ends; PageUp/PageDown turn one
   month, with Shift a year, keeping the day or the month's last. A blocked target is probed onward at most 24 days
   (reka's `MAX_FOCUS_RETRIES`): a page key on outward, Home and End back toward the day, and a target held to
   `min` going back (or `max` going forward) inward from the bound, as reka's `focusDate` turns its outward sign;
   an arrow looks at most 366 steps for a usable day (reka never stops). Enter, numpad
   Enter and Space pick; with Ctrl, Alt or Command they go on to the page.
3. **Tab stop.** One per grid: the anchor if usable in its own month, else the first usable day from `min` or the
   month's start. reka skips the whole of `min`'s month when the anchor is before `min` and loses the stop; reka
   also looks up the initial focus by a cell's text, which finds the 30th of the month before; here by date.
4. **Range presses** are reka's: a press starts, the second ends (ends swap if before), a third starts again; the
   same day twice gives a one-day range, and once more clears it. A press across a blocked day without
   `allow_non_contiguous_ranges` is refused once and starts there on the second (reka's preview rule, fixed to use
   the preview it shows). Hover and focus point a day; the preview is the pointed span, or with `maximum_days` the
   window of that many days toward it. `maximum_days` blocks days too far from a lone start, and from a fixed end.
   Escape while a range is being built restores the last whole range, the caller's own writes included (reka keeps
   only its own), and goes on to the page. `aria-selected` and `aria-pressed` agree on every day (reka's disagree
   on unavailable days inside a range).
5. **Other reka bugs fixed.** `disable_outside_view` disables only days outside their month (reka disables the
   grid's every cell); a focused Prev or Next that becomes disabled gives focus to the other; a disabled calendar
   without `min`/`max` does not page; the AT strings are the locale's (`DateStrings`), reka's English only.
6. **Today** is the caller's option (`CalendarOptions::today`, default `CalendarDate::today()` in the local time
   zone through jiff's bundled tzdb). It is not refreshed at midnight: the frame clock is scripted in tests and an
   app that runs past midnight passes a signal.
7. **Fields type by Chrome's rule** (item 12) in every segment: digits join; a value past the largest is held to it;
   one below the smallest leaves the segment empty and waiting; the segment moves on when full or when no further
   digit fits; Backspace and Delete clear the segment; ArrowUp/ArrowDown step and wrap, an empty segment starting
   at its smallest or largest and an empty year at the anchor's. `time_field` uses the same `type_digit` and
   `step_value`. reka reads "33" in a day as 3 then 3 and drifts a segment; here it is 31.
8. **Field fixes.** The day's largest follows the typed year and month (reka the anchor's year, refusing
   29 February typed before its year); a day past its month is held only when a typing ends, on a step, or on blur,
   so the year's first digits do not cut it. The hour runs 1–12 on a 12-hour clock and 0–23 on 24 (reka allows 24
   and reads 0 as empty); the day period typed on an empty hour is kept; there is no era segment (reka's has no
   handler); the form gets the ISO Gregorian day whatever the system (reka sends the system's fields as if
   Gregorian); digits type in the locale's script as well as ASCII. Enter goes on to the page (a form's submit), as
   Chrome's date input does.
9. **Locale.** Segment order, separators, the hour cycle, week numbering and placeholders are the locale's:
   en-US M/D/Y on 12 hours with "mm", "dd", "yyyy"; vi D/M/Y on 24 hours, Monday first, ISO weeks, with "dd",
   "mm", "yyyy" as Chrome's vi input shows them; fa-IR Y/M/D with its own words. The day periods are translated
   (vi SA/CH), as Chrome's time input shows them; reka writes AM/PM always.
10. **Picker.** `DatePicker` joins a `DateField` and a calendar over one `DateEntry`: a pick keeps the time typed
    (an empty time becomes 00:00), clearing the day clears the time, and the popup closes on a pick when
    `close_on_select` says so. The popup is an engine layer (Escape, a press outside, focus outside close it) with a
    focus scope returning focus to the trigger; the calendar takes focus by its `initial_focus`
    (`AutoFocus::Keep`). The trigger is named ("Show date picker", Chrome's name; reka's has none) and ArrowLeft
    on it goes back to the field's last segment. `DateRangePicker` does the same over two fields.
11. **Page functions** are reka's (`nextPage`, `prevPage`): the root's in `CalendarOptions`, a button's own through
    `CalendarGrid::page_button_by`, overriding it. They turn from the day the view was made from (reka's
    `grid[0].value`, kept beside the first month); by a button's own function the anchor is the day it gives, by
    the root's that day on its month's first after a turn of a month or more and on the year's first month after
    a year or more; Next and Prev are blocked by `handleNextDisabled`/`handlePrevDisabled`'s rule. The page keys
    keep reka's own function (a month or a year on, the month's first). No live region: a page turn is announced
    through focus on a day whose name holds its month and year, and the visually hidden level-2 heading.
12. **The AT tree** (the gaps in gui recorded before are closed) carries `aria-invalid` and `aria-readonly` (a minimal `gui` edit: `Announce::invalid`,
    `Announce::read_only`, `Div::aria_invalid`, `Div::aria_readonly`, read by AccessKit's `set_invalid` and
    `set_read_only`): every segment of a date or time field carries the field's invalid and read-only state and
    its group the invalid one (reka's `DateField.test.ts:455-492`), and a calendar's grid its read-only state. An
    IME commit reaches only text inputs, so a segment gets key text only (no gap for typing: a commit's digits
    type in turn). No shadcn component wraps `DatePicker` (shadcn-vue's date picker is a popover and a calendar
    in the docs), so it has no shadcn caller yet. No `better.tsv` entry: the demos the research found wrong in
    Chrome (week numbers, multiple, booked dates) are not pages here.
13. **A new locale or calendar system** makes the view again from the anchor, which stays (reka rebuilds the grid
    from its placeholder, `useCalendar.ts:316-324`); before, it went back to today. The verdict pages switch
    locale on a calendar whose anchor is today, so they show the same month either way.
14. **TimeField is the segment core's time segments** (reka's `TimeField`), not a fixed Chrome emulation: the value
    is a `Time` to its precision (granularity hour, minute — the default — or second, decided by the value as the
    date field's precision is), the hour cycle the locale's or `TimeFieldOptions::hour_cycle`, the segments in the
    locale's order (`CalendarLocale::day_period_first` for locales writing the day period first), the arrows by
    each unit's step and mirrored right to left, `round_to_step` (reka's `stepSnapping`, `snapValueToStep`: a
    minute stepping by 15 rounds 58 to 45) as focus leaves a unit, `min`/`max` making it invalid and never held,
    the strings the locale's `DateStrings`. reka compares the bounds on today's date, so a date-time of another day
    is compared wrongly: here a bound is a time of day compared with the value's time of day, and
    `TimeField::over` edits the time of a dated entry, its day kept. The form gets `HH:MM`, or `HH:MM:SS` to the
    second, as Chrome's time input sends it (reka's `Time` always writes the seconds). Its root is a `group`, as
    reka's. `shadcn::input_time` is it to the second on a 24-hour clock (what the reference's Chrome draws), its
    pixels unchanged.
15. **A range picker owns one value.** `DateRangePicker::new` makes its two fields itself over its one
    `DateRangeEntry` model (`DateField::range`), with one set of options, so the fields, the range and the form
    value cannot disagree; `invalid` reads those options (reka's `DateRangeField` rule: an end unavailable or out
    of bounds, the start after the end, an unavailable day between, whatever the calendar allows). The arrow chain
    start → end → trigger follows the segments' order as the locale has it at the time. A picker's trigger reads
    its field's availability, no longer a second `availability` argument, so each property has one owner.

## Group 6: form controls (M10 S6b)

Built 2026-11-22. `primitives::checkbox`, `switch`, `toggle`, `toggle_group`, `radio_group`, `slider`,
`number_field`, `number_format`, `pin_input`, `input_otp`, `progress` and `stepper`; the shadcn components of the
same names keep only their look. `shadcn::number_field::{NumberFormat, NumberStyle, SignDisplay}` moved to
`primitives::number_format` (with `UnitWidth`), `shadcn::checkbox::CheckedState` to `primitives::checkbox`,
`shadcn::StepState` to `primitives::stepper` (re-exported by shadcn, no alias). `InputOtpPattern` is `OtpPattern`
and `PinInputType` is `PinKind`; a pin's value is `Vec<Option<String>>` (holes are holes); a toggle group's model is
`ToggleGroupModel` (`Option<String>` for one, `Vec<String>` for many), no longer select's `SelectModel`; a slider's
values are `Vec<f64>` (`Slider::min`, `max`, `step` take `f64`). Tests: `crates/primitives/tests/form_controls.rs`
(behaviour, and one AT snapshot per primitive with the states that matter: a disabled thumb, a filled slot, a box
held back by its group's limit, an unbounded field) and each module's unit tests.

1. **One control is a part function, not an instance.** `checkbox::root`, `switch::root` and `toggle::root` take
   the state shown and a setter: they hold no signal, so shadcn keeps its instance-free fast path and its `bool`
   APIs, and a caller with its own model writes it from the setter. A checkbox holds `CheckedState` (checked, not,
   indeterminate), not reka's `trueValue`/`falseValue`: a box of other values is a group's (`CheckboxGroup<T>`).
   The user's toggle never makes a box indeterminate. Enter does nothing on a checkbox and a radio (WAI-ARIA);
   a switch and a toggle are pressed by Enter and Space, as their buttons are.
2. **One or many is the type.** `ToggleGroup<V>` writes `Option<T>` (one; pressing the pressed one empties it) or
   `Vec<T>` (many, in the order pressed) through the `Pressing` trait; reka infers it from the value's shape. A
   group roves (one Tab stop, the arrows wrap, both axes) unless `set_roving(false)`, then each item is a Tab stop
   and the arrows do nothing. An item's own `@click` runs through the primitive (`ToggleGroup::item_then`), after
   the press and only when the press was taken, so the look adds no handler of its own. A checkbox group's `max`
   makes the boxes not checked unavailable (focusable, read as disabled, clicks ignored); its parent box adds the
   usable values up to the limit or removes them all.
3. **A veto is a controlled value.** A radio group, as every model here, writes through its `Bind`: a caller that
   keeps its value refuses the check (it hears it in `on_change`); there is no cancelable event. Its arrows check
   and Home, End and the page keys only focus, as reka's (`RadioGroupItem.vue:62` sets the select flag for the four
   arrows only).
4. **reka bugs fixed, by intent** (research §4 item 13, §10), each with its test:
   - Slider: the minimum distance between thumbs is counted in steps, so 0.3 − 0.2 is one step of 0.1 (reka
     compares raw floats and refuses it). The values are `f64` end to end: snapped to the decimals of min and
     step and never rounded again to `f32`, and the distance may fall short of `steps · step` by the
     subtraction's rounding error only (a few units in the last place of the larger value). A key that changes
     nothing (at an off-grid max) commits nothing. A disabled slider's thumbs are disabled for AT too.
   - NumberField: Home and End write nothing while read-only (reka's do).
   - PinInput: a paste passes over disabled slots (reka writes into them) and splits by grapheme, not by UTF-16
     unit; a slot's place in its name ("pin input i of n") follows layout.
   - Toggle: a disabled or unavailable toggle ignores Space and clicks (reka's handler toggles first).
   - Stepper: `go_to` refuses a disabled step (reka's check reads the wrong index and never holds); the trigger is
     a button.
5. **Slider.** Values are snapped to the step grid counted from min and clamped after snapping, so max is reached
   when off the grid. Thumbs are inset (reka's `thumbAlignment: 'contain'`): the pointer maps to values inside
   half a thumb of each end, as the look draws them; a press off a thumb jumps the nearest by value, a press on one
   grabs it where pressed; a drag commits once, at the end; each key that moves commits. Shift with an arrow and
   the page keys move ten steps. Right to left mirrors the horizontal arrows and the look. Names for AT are
   `SliderNames` ("Minimum", "Maximum", "Value i of n"), reka's English.
6. **NumberField.** `f64`, NaN meaning no value (the default), so the API stays one number and a form's reset
   empties it. Typing that would not be a number in progress is refused key by key (a minus only while negative
   values are allowed; an IME composition is left alone); Enter and leaving the input commit, clamped and on the
   grid unless `allow_out_of_range`. A mouse press steps at once, repeats after 400 ms and every 60 ms
   (`REPEAT_AFTER`, `REPEAT_EVERY`, reka's), and lets go at the end of the range; the click after a press steps no
   more, and AT's press steps once. A touch or pen steps nothing as it goes down: its tap steps once on release,
   held still 400 ms it steps and repeats, moved past 10 px (`TOUCH_TOLERANCE`, the engine's `gesture::SLOP`) or
   cancelled by the platform it stops, and a long hold opens no context menu. The wheel steps the focused input
   once per event (down steps up; `WheelSteps` turns it off or inverts it), not when it moves more across than up
   or down; an idle field passes the wheel on, so a page scrolls under it. With no bound, none is told to AT.
7. **NumberFormat** (`primitives::number_format`) is `Intl.NumberFormat` by CLDR, through ICU4X: separators,
   digits of the locale's numbering system, group sizes and signs from `icu_decimal`; the percent, currency (by
   its ISO code, accounting or not, with the currency's own decimals) and unit patterns from `icu_experimental`'s
   `dimension` data; a unit's name by plural form through `icu_plurals`. Units are those ICU4X keeps names for
   (length, area, volume, mass, duration: `inch`, `kilometer`, `minute`, `liter`, `kilogram`, …); an id it does not
   know writes the bare number. Reading learns the locale's symbols by writing samples, as `NumberParser` does: a
   group separator is dropped, the decimal separator is the point (`de-DE` reads `1.234` as 1234), the literal
   text around the number (a code, `%`, a unit's name in any plural form) is dropped, and digits of every script
   and the Han numerals are read in any locale. The formatters are resolved once per thread and format. `icu`
   2.3 and `icu_experimental` 0.6 match the ICU4X that parley pins.
8. **PinInput and input_otp.** A pin is a row of Tab stops, one value per position; a one-time code (`otp`, on
   in shadcn as shadcn-vue's demos have it, off in the primitive as reka's) holds focus to its first empty slot.
   `input_otp` is one focus stop holding the code as a `String`, as vue-input-otp's one hidden input: characters
   go in order, a paste as long as the code fills from the first slot, a shorter one goes on and the rest is cut,
   the pattern drops what it refuses first. Both say when full (`set_on_complete`). Both take an input method:
   its composition shows in the slot (`PinInput::composing_at`, `Otp::composing`) and its keys are its own; what
   it commits is typed, spread over the slots. A slot's character (a bullet when masked) and the code are their
   value to AT.
9. **Progress.** A value out of `0..=max` or not finite is indeterminate, a max not above 0 is 100 (reka's
   validation), reported once per call site. The percentage is the value text, not the name, so a label still
   names the bar.
10. **Stepper.** Linear by default: the steps up to the current one and the next are reachable, the rest disabled
    and out of the Tab order. A primary press moves at once, on the way down; with Control it neither moves nor
    takes focus, and its click does nothing (reka prevents the `mousedown` default); Enter and Space move; the
    arrows of its axis move focus between reachable triggers without wrap and choose nothing; the other axis'
    arrows are consumed. A polite status line tells "Step i of n" (visually hidden in shadcn), i the trigger's
    place among those laid out.
11. **Names that follow layout.** A pin slot's "i of n" and the stepper's status read their collection as last laid
    out; the part copies it into a signal in `gui::after_layout`, so the first frame already reads right and a
    change of slots or steps renders the names again in the same frame, never in a loop (equality cutoff).
12. **What gui gained for this group** (each the smallest hook, documented on its method):
    - `on_wheel` returns whether it consumed the wheel; declined, routing goes on up to the next handler or scroll
      container. The compositor's hit table marks boxes under a wheel handler (`Hit::wheel`,
      `ScrollNode::{wheel, wheel_above}`), and over one the render thread hands the platform's wheel to the UI
      thread (`FromRender::Wheel`) instead of scrolling, as Chromium routes a wheel over a blocking listener's
      region to the main thread; elsewhere scrolling stays on the compositor. Before this, the platform's wheel
      never reached an `on_wheel` in a window (only the devtools' did).
    - `on_touch`: the raw phases of the touch or pen contact that went down on a node, the engine's gestures
      running as well, `Ended` told after a tap's press and click.
    - `focus_on_press`: whether a primary press with these modifiers focuses the node; refused, focus stays (a
      prevented `mousedown`).
    - `on_ime`: what an input method composes and commits while focus is on a node that is not a text input; the
      IME target is then the focused node's box.
    - `aria_value`: the text a node holds without being a text input.
    - `ValueRange`: an end that is not finite is not told (no `aria-valuemin`).
    No `better.tsv` entry: the looks are unchanged.
13. **Forms.** A control whose value no `Submit` of `form` fits is a `FormValue` itself, its value a type of its
    module that says how it is sent: `Slider` (`SliderValues`, one number per thumb as `name[i]`) and `PinInput`
    (`PinCode`: the positions' values and the slots shown, sending the shown characters joined; a reset puts the
    values back, holes included). The OTP's and the number field's models are a `String` and an `f64`, sent as
    they are; checkbox, switch and toggle send `bool` or `Pressed`. A disabled control sends nothing through
    `FormField::availability` reading the control's own (`Slider::availability`, `PinInput::config`). Tested in
    `form_controls.rs` (pairs, a check, a reset, disabled).
14. **A read-only number field is a read-only input.** `NumberField::input` takes the `TextInput` and makes it
    read-only while the field is read-only or unavailable: nothing is typed, pasted or composed, and AT hears it,
    as reka's `readonly` input; refusing keys alone left typed digits shown until blur.
15. **A pin drops empty entries at the end for both kinds.** reka's `removeTrailingEmptyStrings` compares with `''`
    only, so a number pin keeps a cleared last slot as `undefined` (`PinInput.test.ts:340-354`) and a text pin does
    not; the intent (the value ends at its last character) is one rule here, and a hole before a character stays
    a hole in both.
16. **Names given are not hidden by fallbacks.** AccessKit reads a node's own name before `labelled_by`, so a
    fallback name would hide a label's: `OtpConfig::label` names the OTP by its `Label` and drops "One-time code"
    (shadcn's `input_otp().label(…)`). A stepper trigger is named by its step's title and description, those in the
    tree (`Stepper::labels`, `StepLabels::{title, description}`; shadcn's `stepper_title(item, …)` and
    `stepper_description(item, …)`), as `StepperTrigger.vue:37-56`; one the caller names
    (`Stepper::trigger_labelled`) is described by the description instead. The stepper's separator is
    decorative (`StepperSeparator.vue`), hidden from AT. The looks are unchanged.

## Group 3: overlays (M10 S6b)

Built 2026-11-22. `primitives::dialog`, `alert_dialog`, `drawer`, `popover`, `hover_card`, `tooltip`, `toast`
and `hover` (the grace area and the delay they share). Each overlay is an engine layer (ADR-0024) with its focus
scope, not code of its own for Escape, presses outside or focus. `shadcn::dialog`, `alert_dialog`, `sheet`,
`drawer`, `popover`, `hover_card`, `tooltip` and `sonner` keep only their look; shadcn's `HoverIntent` is gone.
Tests: `crates/primitives/tests/overlays.rs` (behaviour and seven AT snapshots) and the unit tests of `hover`,
`drawer`, `toast` and `dialog`.

1. **What closes what.**

   | Overlay | Escape | Press outside | Focus outside | Scroll | Focus |
   |---|---|---|---|---|---|
   | Dialog, modal | yes | yes, but not a context press | no | no | trapped |
   | Dialog, not modal | yes | yes | yes | no | loops |
   | AlertDialog | yes | no | no | no | trapped |
   | Popover | yes | yes | yes | no | loops; trapped when modal |
   | HoverCard, Tooltip | yes | yes | no | yes | not moved |
   | Drawer | as its modal level says | | | | |

   The caller's veto (`Veto`, reka's prevented `escapeKeyDown`, `pointerDownOutside`, `focusOutside`) is asked
   for each reason except code's own. A context press means a secondary press, or a primary press with Control
   on macOS (`is_context_press`). A modal overlay refuses it, as reka's `DialogContentModal` does.
2. **Focus back.** A closing Dialog, AlertDialog, Popover or Drawer gives focus back by the engine's rule: to what
   held it as the overlay opened, never taking back focus that something else took, and nowhere after a press
   outside. Popover and Drawer name their trigger (`FocusScope::return_to`, added to gui for this), as reka's do.
3. **reka bugs fixed, by intent** (research §4 item 13, §7):
   - AlertDialog: a prevented `open_auto_focus` (`AutoFocus::Keep`) is respected. reka focuses Cancel anyway.
     Otherwise Cancel is focused wherever it is in the content, through `register_cancel`. In a modal, focus left
     behind is pulled in by the trap, to the first Tab stop, unless the caller focused a node of its own.
   - Drawer: closing gives focus back to the trigger (`final_focus`, on by default). reka's vaul port loses it.
   - Toast: Escape on a toast, or an action pressed from the keyboard, focuses the region. reka drops focus to the
     body.
4. **HoverCard.** Opens 700 ms after the pointer or focus reaches the trigger (`OPEN_DELAY`). A second request
   keeps the first deadline. It closes 300 ms after the pointer leaves (`CLOSE_DELAY`). A press held in the card
   keeps it open. Leaving toward the card holds the grace area.
5. **Grace area** (`hover::Grace`, reka's `getPaddedExitPoints` and `getHull`). It is the convex hull of the exit
   point, padded 1 px back along the side left, and the target's corners. While it is held, moving inside the
   hull keeps the overlay open. Leaving the hull closes it. Reaching the trigger or the content drops the area.
   "In transit" lasts at most 300 ms. Being inside a box is judged as hit testing judges it: left and top edges
   in, right and bottom edges out.

   The pointer comes from a new tracked `gui::interact::pointer_position`, read only while an area is held.
   Timers of `hover::Delay` belong to the scope that made them, not to the watcher that asks. Otherwise the
   watcher's next run cancels them.
6. **Tooltip.** Constants: `DELAY` is 700 ms, the provider's skip window (`SKIP_DELAY`) is 300 ms.
   - The skip window: after one closes, the next opens at once. It is shared by a provider's tooltips.
   - One tooltip is open per window, whatever its provider.
   - It opens on the first pointer move over the trigger, not on enter. It does not open while the pointer
     crosses another tooltip's grace area. It opens at once on keyboard focus (`keyboard_focus_only` limits it to
     visible focus); a focus from a press does not open it.
   - Leaving the trigger closes it at once when not `hoverable`.
   - A press on the trigger closes it (`closing_trigger`), and so does a click. Content over another trigger
     closes it.
   - A tooltip outside any provider uses the window's (`TooltipProvider::window`), where reka throws. shadcn's
     window provider has its 0 ms delay.
   - The content node itself is the `tooltip`, and the trigger is described by it. reka also renders a visually
     hidden copy for this, which gui does not need.
7. **Drawer** (reka's vaul port).
   - Modal levels: `On` is modal and trapped, with an overlay. `TrapFocus` is not modal, trapped, with an
     overlay; the source wins over the docs here. `Off` is neither.
   - Every change asks `veto(open, ChangeReason)`. The reasons are `Swipe`, `EscapeKey`, `OutsidePress`,
     `FocusOutside`, `TriggerPress` and `ClosePress`. reka's `click` and `cancel` are never sent, so they are
     dropped.
   - Swipe:
     - it starts after 1 px;
     - it closes past 40 px or faster than 0.3 px/ms (`SWIPE_THRESHOLD`, `SWIPE_VELOCITY`);
     - a reversal of 10 px cancels it;
     - velocity samples are 16 to 80 ms old;
     - pulled away from its edge, it is damped by the square root;
     - progress is the displacement over the size plus 40.
   - While a drag is held, presses and focus outside are refused.
   - Snap points, nesting, the swipe area and the provider: see "Group 3: review fixes" below.
8. **Toast.**
   - Countdowns: 5000 ms by default; shadcn's sonner uses 4000 ms and keeps a loading toast. They pause together
     while the region is hovered (`pause_on_hover`), focused within, pressed, or held by the caller. They resume
     from the time each had left.
   - Showing an id that is already shown updates it in place and restarts its countdown.
   - Each toast is a `status`: `assertive` when `Sensitivity::Foreground`, `polite` when `Background`.
   - The region is a named `region` ("Notifications").
   - Swipe-away: 50 px (`SWIPE_THRESHOLD`); it starts after 2 px with a mouse and 10 px with touch
     (`ToastSwipe`).
   - The region's flags are watched from the first frame (`on_mounted`): a node's flags read while setup renders
     are not subscribed to.
9. **Open state.** Every root takes a `UseModel<P, bool>`, so open is controlled or uncontrolled by the same
   rule as every model (group 1). shadcn's `dialog().modal(…)`, `.on_dismiss(…)`, `drawer().modal(…)`,
   `.on_request(…)`, `popover_close`, `tooltip_provider()` with `delay_duration`/`skip_delay_duration` are new.
   `toaster()` and `toast()` raise toasts on the last mounted toaster.
10. **AT.**
    - Titles are headings of level 2.
    - Content is named by its title and described by its description.
    - A trigger says expanded and controls the content while it is open. A popover is labelled by its trigger.
    - Dialog, sheet and drawer titles and descriptions are `ModalText`, which links them to whichever modal
      holds them.
11. **Not ported**, recorded: the drawer's virtual keyboard inset (`gui` runs on desktops), the hover card kept
    open while text in it is selected (see the review fixes), and Escape while an IME composes, which is left to
    the engine (ADR-0024). What this item listed first is built since: "Group 3: review fixes" below.
12. **Pixels.** Overlay, offsets, durations and animations are unchanged, so there is no `better.tsv` entry. The
    AT tree changes: headings, relations, the sonner region and toast statuses are new.

## Group 8: structure (M10 S6b)

Built 2026-11-22. `primitives::collapsible`, `accordion`, `tabs`, `pagination`, `scroll_area`, `splitter`,
`avatar`, `aspect_ratio` and `carousel`. The matching `shadcn` components keep only their look: `collapsible`,
`accordion`, `tabs`, `pagination`, `scroll_area`, `resizable`, `avatar`, `aspect_ratio`, `carousel` and the
trace's collapsible step. Tests: `crates/primitives/tests/structure.rs` (behaviour, timings, controlled and
uncontrolled, and an AT snapshot of every primitive) and the unit tests of `pagination`, `scroll_area`,
`splitter` and `carousel`.

1. **Disabled or unavailable.** Accordion, Collapsible, Tabs and Pagination take `Availability` and their shadcn
   callers pass `Disabled`, the state the look dims. Carousel's Previous and Next at an end are
   `CarouselOptions::at_end`: reka's are unavailable, Embla's (which shadcn's demos and the verdict use) are
   disabled, and shadcn passes `Disabled`. A splitter handle is never taken out of the Tab order: a disabled one
   is unavailable, announced disabled, and F6 still leaves it. reka gives it no disabled state.
2. **Options are read when used.** The accordion's `collapsible` and orientation, the tabs' activation,
   orientation and wrap, the splitter's orientation and step, the scroll area's policy and delay, and the
   carousel's options are closures over the part's props. A change counts at the next key or press. reka reads
   some of them once, at setup.
3. **Accordion.** One section (`Option<T>`) or many (`Vec<T>`) is a type, not a string or array chosen at run time.
   With many, the one opened goes last, as reka appends it. The arrows, Home and End move only among this
   accordion's own usable triggers. reka's Home and End land on disabled triggers, and its arrows walk every
   trigger in the document, nested accordions included. The header is a heading of level 3 (gui's AT tree now
   has `aria-level`); the content is a region named by its trigger.
4. **Tabs.** With automatic activation a tab is selected when focus reaches it from the keyboard or from code. A
   press selects as the button goes down. A Control press is a Mac's secondary click: it neither selects nor
   focuses (`Div::focus_on_press` with a rule over the modifiers). A tab that is not selected stays out of the
   Tab order (roving). Kept panels are hidden and inert. The Indicator (`Tabs::indicator`, `indicator_part`) is
   reka's `TabsIndicator`: the selected tab's offset from the list's padding box, its length and thickness, as
   layout places them, bound to the part's node so a new selection moves it with no render. The maps of tabs
   kept per value forget the values no tab has any more.
5. **Pagination** is reka's `getRange`: always 2 × siblings + 5 entries when there are that many pages. First and
   Previous are disabled on the first page, and Next and Last on the last. A page button is "Page N" and current
   (`aria-current=page`) on its page. A page written past the count is corrected only when the caller asks
   (`clamp`), from the first value on (reka's `immediate` watch), and the caller hears the corrected page. The
   English names are `PaginationNames` defaults; the page's name is a closure, so a locale can be captured.
6. **ScrollArea.** Only an axis with a bar scrolls; the axes are the caller's, so reka's quirk of turning both
   off when one bar unmounts is not kept. The viewport is a Tab stop, and its keys are Chrome's: 40 px a line,
   87.5 % of the viewport a page, Home and End, and Space and Shift+Space only while the viewport itself holds
   focus. A key that cannot move it is left to the scroller around it, as Chrome chains scrolling. The bars,
   thumbs and corner are parts the look draws, laid over the viewport; the engine's own thumb is off there.
   Policies are reka's `type`: `Always`, `Auto` (overflow checked 10 ms after the last change of size),
   `Hover` (the default; shown while the pointer is in and the content overflows, hidden 600 ms after it
   leaves), `Scroll` (shown while scrolling, hidden 100 ms after the last scroll plus the delay) and `Glimpse`,
   each bar with its own state machine as reka has it. The thumb is the track times viewport / content, never
   under 18 px, only while the content overflows; its length and offset are bound to its node, so a scroll
   moves it with no render. A thumb drag keeps the grabbed point under the pointer; a track press puts the
   thumb's centre under it and drags on; the wheel over a bar scrolls its axis by the vertical delta and is
   taken only between the ends. The corner fills the square where both shown bars meet, not under `Scroll`.
   Right to left the vertical bar is on the left and the horizontal thumb runs from the right. Fixed against
   reka (research §4 item 13): a `Scroll` bar under the pointer stays (reka's never learns of the pointer and
   hides); a `Hover` bar shows at once (reka remounts its overflow check on each entry and shows 10 ms late);
   the wheel's "taken between the ends" holds right to left (reka tests a negative offset and never takes it).
7. **Splitter** ports reka's `adjustLayoutByDelta`, `resizePanel` (a collapsible panel snaps at the midpoint
   between collapsed and min), `validatePanelGroupLayout` and its default layout, in percent with 10 digits of
   precision. Keys: arrows by the step (10 %), Shift by 100 %, Home and End to the ends, and Enter to collapse or
   expand. In right-to-left text only ArrowLeft and ArrowRight flip; reka also flips Home and End. Enter expands
   to the size the panel had before it collapsed; reka goes to its min. F6 and Shift+F6 cycle the usable handles.
   A drag counts from the layout at its start. The handle announces an orientation across the group and a value
   range rounded to whole percent, and controls the panel before it.
   A panel's constraints are in percent or pixels (`Unit`). The room the panels share is the group's content
   box less its handles, measured after the pass that laid both out; reka takes the whole content box, so its
   pixel panels come out short by their share of the handles. As the group resizes, pixel panels keep their
   pixels; a group first measured under 50 px that grows past ten times that is laid out afresh (reka's nested
   group rule). The layout is worked out at the first layout pass after panels join or leave, so panels mounting
   together never throw the caller's layout away one by one; it is committed, so the caller hears the first
   layout (in each panel's unit, `on_layout`) and each panel its first size, collapse and expand, as reka's
   `callPanelCallbacks`. A default size changed later is ignored, as reka does. A group that saves
   (`persist(id, storage)`, the `Storage` trait; `MemoryStorage` lasts as long as the app) writes 100 ms after the
   last change, keyed by its id and its panels' ids or order and constraints, and starts from what it saved.
   The pointer reaches a handle within 5 px of its box, 15 px once a finger or a pen has pressed in the group
   (gui has no query for the pointer's coarseness, so the group learns it from touches; reka asks the device once
   at load); the nearest handle wins. A secondary press ends a drag. At a limit the cursor is the one-way resize
   cursor of the way the pointer can still go; reka reads that way after its right-to-left flip and points the
   wrong way there. While a drag runs the cursor stays the drag's wherever the pointer goes (gui: the dragged
   node's cursor wins) and the panel boxes and all they hold take no pointer (see "Group 8: review fixes").
   Handles of different groups crossing are dragged together (same section). The maps of handles, panels and
   sizes kept per index forget an index that left.
8. **Avatar.** The root holds the status; the image part, made in its setup, works it out from what the caller
   holds (`ImageLoad`) and writes it, telling `on_status` each status from the first. With no image part the
   status is `Idle`, and the fallback shows. A fallback's delay counts from when it mounts and again each time
   the image stops being loaded (see "Group 8: review fixes"); 0 shows it at once, where reka never shows it
   (research §4 item 13). shadcn's `avatar`, `avatar_image` (`.on_status`) and
   `avatar_fallback` (`.delay`) are parts on the primitive.
9. **AspectRatio** sets the box's width to 100 % and its height from Taffy's `aspect-ratio`. The content goes in an
   absolutely placed layer that fills the box, so content never sizes it.
10. **Carousel.** The snap is the model; `carousel::snaps` trims the trailing space as Embla does. The root is a Tab
    stop described as a "carousel". Its keys act only while the root itself holds focus (reka's viewport rule; the
    keys of a control inside are its own). shadcn-vue's own root listens to keys from its descendants, but no
    verdict demo presses a key. The track's place is `Carousel::offset` (the snap's fraction plus the drag's
    pixels, mirrored right to left) with `motion` and `move_duration` (eased over `CarouselOptions::duration`
    to a snap, 250 ms back from an overscroll, at once while held or under reduced motion), bound to the look's
    node, so a drag moves it with no render.
    Pointer, reka's numbers: a drag starts after 3 px along the axis (mostly across is the page's), follows the
    pointer, past an end at 0.3 of the way; on release the velocity of the last 80 ms projects 120 ms ahead and
    the nearest snap is shown (a tie of half a pixel keeps the current), one snap at most unless `skip_snaps`.
    `drag_free` glides, keeping 0.996 of its speed a millisecond, and rests off the snap. A drag past 10 px is no
    click: `gui::interact::claim_press` (reka's `swallowNextClick`). Wheel: 30 of delta is a snap, one per gesture
    (turns under 180 ms apart); at an end a new gesture is the page's. Along the axis the wheel pages, where reka
    lets the native track scroll and then snap: gui's track is translated, not scrolled; across, only with
    `wheel`.
    Rotation: one snap every delay (reka 4000 ms; shadcn's demo 2000 ms), counted again from each change of snap.
    It is playing until the user stops it and paused while the mouse is over it (when asked), a finger holds it, a
    drag runs, the window is hidden, or the carousel is out of the window. Keyboard focus coming in stops it, and
    so does reduced motion, at once and when it changes; the rotation button (`rotation_button`, "Stop slide
    rotation" or "Start slide rotation") plays it again. Indicators: in a group (`join_indicators`,
    `indicators`) they are tabs named "Slide N" in a tab list "Choose slide to display", roving with the arrows
    (both axes, wrapping) and showing the focused one's snap, and the slides become tab panels named by their
    snap's tab; alone, a button "Go to slide N", unavailable on its own snap. A slide is a group "N of M"; one out
    of view is hidden from AT and inert. `on_settle` hears the snap once the track has come to rest on a new one.
    The content is a polite live region, off while rotating.
    Going round, fading, auto-scroll, auto height and focus handed on to the slide coming in are built (see
    "Group 8: review fixes").
11. **Find in page** opens nothing: gui has no find-in-page, so reka's `hidden="until-found"` for kept content is
    not built (kept content is hidden and inert).
12. **gui additions.** `Div::aria_level`, `aria_current`, `aria_role_description` and `aria_live` (the
    `Announce` part of `Semantics`), passed to AccessKit as they are. They are the first heading level, current
    page, role description and live region in the AT tree; group 7's gap 12 about heading levels is closed.
    Then: the resize cursors `ResizeEastWest`, `ResizeNorthSouth` and the one-way `ResizeEast`, `ResizeWest`,
    `ResizeNorth`, `ResizeSouth`; while a press drags a node its cursor wins wherever the pointer is;
    `gui::interact::claim_press`, a drag's way of saying its release is no click; and `gui::environment`, the
    platform's facts as signals: the window seen or not (winit's occlusion), its size, and reduced motion
    (`RunOptions::reduced_motion`).
13. **Pixels** are unchanged on every page of the group: the carousel's track is translated by the same fraction
    and eased the same way, the scroll area draws the bars shadcn draws, and the avatar's parts lay out as before.
    What is better than Chrome is behaviour and the AT tree, recorded above, so there is no `better.tsv` entry.

## Group 4: menus (M10 S6b)

Built 2026-11-22. `primitives::menu` (the engine reka's `Menu` is), `dropdown_menu`, `context_menu`, `menubar`,
`navigation_menu` and `toolbar`. `shadcn::dropdown_menu`, `context_menu`, `menubar` and `navigation_menu` keep only
their look; the crate-private `MenuState` and shadcn's navigation timers are gone. Tests:
`crates/primitives/tests/menus.rs` (behaviour and six AT snapshots), `crates/shadcn/tests/menus_focus.rs` and
`menus_direction.rs`.

1. **The highlight is focus**, as in reka: the item under the mouse takes focus, the mouse leaving it gives focus to
   the content, and the next arrow starts from an end. The look binds `Interaction::focused` (or
   `Menu::is_highlighted`) to the row's node, so a highlight moving renders no row. A sub trigger is lit while
   focused or while its submenu is open (`focus:bg-accent data-open:bg-accent` in all three menus), and a radio
   item's dot follows its row's focus. A pointer event sent again after a frame at the same place over the same
   item is no move, as a browser sends no `pointermove` then: the keys' highlight stays under a still mouse. A
   content that opens under the pointer starts from where the pointer is, and the pointer leaving it unhighlights
   only after it moved inside. Opened from the keyboard, the content takes focus and steps to its first (or last)
   item once its items are laid out, in the same frame; its layer's scope moves no focus (`AutoFocus::Keep`), so it
   never undoes that step. Opened by its caller, the content takes focus as reka's does on mount.
2. **Levels.** Each level (`Menu::new`, `Menu::submenu`) has its own `Collection`, typeahead, keys and highlight.
   Submenus are not layers: they are inside the root's layer by ownership (ADR-0024), so a press in a submenu is
   inside, and Escape at any level reaches the root's layer and closes the whole tree. A submenu closes when
   another item of its parent is highlighted, when focus leaves it for anything but its trigger, and whenever its
   parent opens or closes.
3. **Numbers kept**: a submenu opens 100 ms after the mouse's first move on its trigger (`SUB_OPEN_DELAY`; leaving
   first cancels it; a click opens it at once and never closes it); the grace area is reka's pentagon, its first
   point 5 px behind the exit point (`GRACE_BLEED`), held 300 ms (`GRACE`) while the pointer moves toward the
   submenu's side as placed; typeahead's 1000 ms; a context menu's long press is the engine's 700 ms and 10 px
   slop; the context menu stands 2 px from the point (`SIDE_OFFSET`); a navigation menu opens after 200 ms
   (`OPEN_DELAY`), after 150 ms while one is open or within 300 ms (`SKIP_DELAY`) of one the pointer closed, closes
   150 ms after the pointer leaves (`CLOSE_DELAY`), and ignores a mouse click for 300 ms after a move asked to open
   (`CLICK_AFTER_MOVE`). These timers are owned by no scope (`gui::detached`), each guarded by its level being
   alive and cancelled by it: a scope that happens to run the pointer event (a row rendered again by its
   highlight) disposing would cancel them, and the submenu would never open.
4. **Picking** answers `gui::Dismiss`: `Close` closes the whole menu, `Veto` keeps it (reka's prevented `select`).
   A checkbox item asks for the value its pick turns it to (mixed becomes checked, else it flips), a radio item for
   its value even when chosen; the caller keeps both.
5. **Keys** follow §3 and §6 over reka where they differ: PageUp and PageDown reach the ends from an item too (reka:
   from the content only), and Space during a typeahead search is part of it (reka drops it, so "New Tab" cannot be
   typed). Tab: a modal menu keeps it (its focus scope is trapped and does not cycle); one that is not modal lets
   focus go past it, and focus outside closes it.
6. **Focus back** is the engine's focus scope (ADR-0024), the wrappers' table in research §8: to the trigger
   (`FocusScope::return_to`, reka's `onCloseAutoFocus` focusing `triggerElement`; a context menu has none, so to
   what held focus as it opened), unless something else took it since or a press outside closed it; a modal menu
   gives it back after a primary press outside, not after a secondary one. Escape focuses the trigger with its
   ring. A press on a dropdown trigger while its modal menu is open closes it as a press outside, and no click
   follows (the page below takes no press), so it does not open again: reka reaches the same through its
   `interact-outside` veto and the click.
7. **Context menu.** A secondary press in the area while the menu is open moves it to the new point (reka's test),
   through the layer's veto, without closing and opening. `gui`'s layer memo now compares the requested
   `Positioning`, so a moved point places the layer again (it reused the old place; a one-line fix in
   `gui::layout`). The context-menu key and Shift+F10 open it on its first item against the area. Its content is
   unnamed, as reka's. Recorded limitations: the long press is the engine's fixed 700 ms (reka's `pressOpenDelay`
   needs a per-node delay in `gui::gesture`); a secondary press on another area while a modal context menu is open
   only closes it (the page below a modal layer takes no press).
8. **Menubar.** A primary press opens a trigger's menu as it goes down; a press on the open one closes it; while one
   is open, the pointer entering another usable trigger opens it. The arrows past a content's items open the next
   or previous menu with no item highlighted (reka's), so the docs script presses ArrowDown before Enter. Focus on a
   trigger of the bar is not outside its menus.
9. **NavigationMenu.** One timer for every hover request, a later one replacing the earlier (reka's debounce). The
   panel is a layer that is not modal and has no focus scope; a press on a trigger or link of the menu, and focus on
   one, are not outside it. Escape closes it and focuses its trigger; after Escape or a click closed it, the pointer
   moving on the trigger does not open it again until it leaves and comes back. ArrowDown (ArrowRight, or ArrowLeft
   right to left, in a vertical menu) on the open trigger enters the panel; arrows move among the panel's links, no
   wrap. A link is `aria-current="page"` when active; following it closes the menu unless its handler vetoes or
   Command is held. shadcn's `navigation_menu_link` is now a part (it reads its item), and
   `navigation_menu_link_as` takes a look such as `navigation_menu_trigger_style`.
10. **Toolbar.** `Keymap::TOOLBAR` no longer wraps: reka's `loop` is off (an absent Boolean prop is `false` in Vue);
    a caller wraps with `Wrap::Around`. A separator in a toolbar turns across it (item 12): vertical in a horizontal
    toolbar, as ARIA's separator orientation is the line's own and APG's toolbar draws it. reka gives it the
    toolbar's orientation. No shadcn component uses `Toolbar` yet.
11. **reka bugs fixed, by intent** (research §4 item 13):
    - Menubar: from a submenu's plain item, the key toward the inline end opens the next menu under right to left
      too; reka hard-codes `ArrowRight`, so under RTL that key both closed the submenu and moved on.
    - Menubar: a disabled trigger does not open on hover (reka opens it).
    - NavigationMenu: the arrows between triggers and links follow the reading direction (reka ignores it).
    - NavigationMenu: the "closed by a click" flag is taken before the value changes. reka reads it after, so it is
      set when a click opens the panel, and a click that closes it lets the hover open it again 200 ms later.
    - Group: named by its label only when it has one. reka points `aria-labelledby` at a label that may not exist;
      shadcn's groups have no label part, so they are unnamed groups.
12. **AT.** Content `menu`, vertical, named by its trigger (a context menu's is unnamed). Items `menuitem`,
    `menuitemcheckbox` (true, false, mixed), `menuitemradio`; a disabled item is read, disabled. The dropdown
    trigger, menubar triggers and sub triggers say `aria-haspopup=menu` and `aria-expanded`, and control their
    content while open. Groups and radio groups are `group`; separators horizontal `separator`. The navigation menu
    is a `navigation` landmark with a `list` of `listitem`s; its triggers have `aria-expanded` and no
    `aria-haspopup` (it is no menu); its panel is named by its trigger. A content kept mounted while closed is
    hidden from AT, is no layer, takes no keys and traps nothing.
13. **Not built**, recorded: drag-to-select (pressing elsewhere and releasing on an item picks it) needs a release
    handler in `gui`; hover highlighting by mouse only relies on `gui` sending pointer moves for the mouse alone;
    `DropdownMenuFilter`; NavigationMenu's viewport, `NavigationMenuSub`, focus proxy and taking a closed panel's
    links out of the Tab order (shadcn's demo uses `:viewport="false"` and none of them); a per-instance long-press
    delay.
14. **Engine fixes** (one line each, outside this group's files): `gui::layout`'s layer memo compares the
    requested `Positioning` (item 7); `gui::interact`'s settle syncs the nodes' focus flags whenever focus moved,
    not only when a layer rule moved it, so focus a post-layout hook moves (the step to the first item) shows in
    the same frame, as `after_layout` promises.
15. **Pixels.** Looks, offsets and animations are unchanged but where reka's behaviour shows: the context menu
    stands 2 px right of the point (context-menu frames 02 to 05 and 07); a navigation menu closes 150 ms after
    the pointer leaves and a trigger closed by Escape keeps focus with its ring (navigation-menu 07 to 10, 14); the
    arrows open the next menubar menu with no item highlighted (menubar 14 and 16, item 8). The docs scripts wait
    for the delays reka has (a submenu's 100 ms, a navigation menu's 200 ms, and its ignored click). Every other
    frame of the dropdown-menu, context-menu, menubar, navigation-menu, table, chat, message-scroller and
    prompt-input pages is identical. No `better.tsv` entry: each difference is reka's own behaviour, which Chrome
    shows too.

## Group 5: select and lists (M10 S6b)

Built 2026-11-22. `primitives::listbox` (the core reka's `Listbox` is), `select`, `combobox`, `autocomplete`,
`tags_input`, `tree` and `command` (shadcn's command, on reka's `Listbox` and `ListboxFilter`). `shadcn::select`,
`combobox`, `command` and `tags_input` keep only their look. Tests: `crates/primitives/tests/select_lists.rs`
(behaviour and ten AT snapshots) and the unit tests in each module.

1. **One list core.** `listbox::ListCore<T>` holds the highlight (by value, its index found again in the options
   it is given), the Shift anchor, the typeahead and the pick veto; every list primitive here is built on it. A
   value is a `Choice<T>`: `Option<T>` (one) or `Vec<T>` (several, in the order chosen), read and written through
   `Bound<M>`, a `Copy` handle over the part's `UseModel` (or a caller's `Signal`). Options are read by index
   (`listbox::Options`, on `collection::Items`), so a list of millions walks its data and a mounted list
   (`[Item<T>]`, a `Collection`) is read the same way. `ListConfig` picks the keymap, the selection behaviour,
   whether the highlight is focus (`ListFocus::Items`: select, tree) or an active descendant of a field
   (`ListFocus::Field`: combobox, autocomplete, command), and whether hovering highlights. `Listbox::key` answers
   what a key did (`ListKey`), so each primitive decides what a choice does after (close, refocus, stay open).
   `listbox::Listbox<T, M>` is reka's `ListboxRoot` itself, on the core: parts `root`, `content`, `item`,
   `group`, `group_label`, `filter`, and `virtual_content` / `virtual_option` for a list of millions. Its
   guarantees are reka's: mounting highlights the chosen option (else none) and moves no focus; the content is
   the one Tab stop while nothing is highlighted, the highlighted option after; Tab into the content (not a
   pointer press on it) highlights the option focus last left, else the chosen, else the first; focus leaving
   lets the highlight go and remembers it; the pointer leaving the root lets it go; a caller's write highlights
   its option and moves focus to it (`keep_focus` and a filter field keep focus where it is, reka's
   `ListboxFilter`); the user's own choice does not move focus again.
2. **reka bugs fixed, by intent** (research §9, §4 item 13):
   - End (and Home) with nothing highlighted reaches the last (first) usable option; reka's End did nothing then.
   - Select-all (Control, Alt or Command + A) chooses every option that can be chosen in a virtual list and a
     mounted one alike; reka's virtual branch chose disabled ones too.
   - A Shift range leaves out what cannot be chosen, in a listbox and in a tree (reka's Tree kept disabled rows).
   - Typeahead finds by index over the options given, mounted or not.
   - Control, Alt or Command + Enter is the form's, never a choice.
   - A tags input adds by one rule on every path (Enter, the delimiter, Tab, paste, blur): trimmed, nothing when
     blank; reka adds a blank draft on Enter and blank pieces of a paste. Home with no tag selected selects the
     first tag (reka's selects the last).
   - A combobox puts its text back as it closes, after the value was written; reka waits 1 ms to land after the
     choice. Its field starts showing the value (reka's immediate watch).
3. **Select.** Options are known while closed: the content stays laid out, invisible, inert, 0 × 0 and out of the
   flow (`Select::content`), so the trigger shows the chosen labels from the first frame and typeahead on a closed
   trigger chooses (one value), and the items are the same instances open and closed (reka renders them into a
   `DocumentFragment`). The highlight is focus. A primary press without Control opens at once; released within
   `PRESS_SLOP` (10 px) on both axes it chooses nothing, dragged further and released on an option it chooses it,
   outside the list it closes it; a touch opens on the tap and an option is chosen only by a tap that started on
   it. Scroll buttons step one anchor option's height every `SCROLL_EVERY` (50 ms) while the pointer rests, the
   step owned by the select (not by the handler that started it). Choosing replaces one value
   (`SelectionBehavior::Replace`): Enter, a click or a sole typeahead match on the chosen option keeps it (reka's
   `SelectRoot` never clears on re-pick); several values toggle. `Select::on_pick` lets the caller refuse a
   choice on every path (keys, click, drag-release, typeahead on a closed trigger). The anchor (what opening focuses and an
   item-aligned list lines up) is reka's rule, a disabled chosen option before the first usable one included.
   The trigger is empty by the value and shows its placeholder when no option has the value. Open, the list is a
   modal layer: Tab is consumed, focus leaving does not close it, Escape and a choice refocus the trigger. Items
   may be valued `''`. The look plays its opening animation only while open (a list laid out closed would have
   played it unseen).
4. **Combobox.** The text and the filter term are apart: the term is what was typed since the list opened, emptied
   as it closes; typing opens the list and highlights the first match once it shows; opening highlights the chosen
   option (else the first); closing lets the highlight go, so Enter on a closed combobox never chooses. While an
   input method composes the arrows and Enter are its own. `ComboboxConfig::REKA` holds reka's defaults. Beside
   reka's trigger (out of the Tab order, "Show popup"), `Combobox::button` is a trigger that stays in the Tab order
   and opens on Enter, Space and the vertical arrows: shadcn's combobox puts the field inside the list, so the
   button is the only way in. shadcn's combobox list is a command list: picking empties its search when it holds
   several (shadcn-vue's `CommandItem`), and a `null`-valued item clears the value.
   `Combobox::popup` is the list's layer: it dismisses on Escape, a press outside and focus moving outside (the
   field and its root count as inside: the layer is non-modal), and if it goes away while it holds focus (a
   choice, the button's toggle) focus returns to the trigger. reka's `ComboboxContent` dismisses the same way;
   ours had no layer of its own, so Tab out of shadcn's combobox left the list open. `Combobox::option` is an
   option that chooses on click, the same path as Enter.
5. **Autocomplete** is the combobox in text mode (`Combobox::set_text_value`, `ComboboxConfig::AUTOCOMPLETE`),
   reka's `Combobox` with its value the text, not a second implementation: typing writes the value, opening
   filters by it and highlights the option equal to it, choosing writes the option's value to the text, Enter on
   a highlight chooses it, Escape with the list closed empties the text (`cancel_clears_value`), a caller's
   value is written to the field. Its parts and its click helper are the combobox's
   (`Autocomplete::combobox`).
6. **Command.** The filter is `filter::Filter` (contains, case and accents aside); a group shows while one of its
   items does, the empty state when none does. The highlight is the field's active descendant, the first item when
   none is set, let go at every change of the search: a highlight made under an earlier search no longer comes back
   when that search returns (ours did, in the docs' "cleared, down to profile" frame). The keys run on the data over
   the virtual list (`step_in_list`), Home and End included, taken by the field before its own editing (reka's
   `ListboxFilter`). An option's `aria-selected` is whether the caller holds its value (`Command::new`'s
   `holds`), as reka's `ListboxItem` reads the model; the highlight is the active descendant only. A heading
   is a group label and a separator a separator to AT, outside the options' set (their place counts options
   only). Up to `FILTER_AT_ONCE` (20 000) items filter in the event; above it the rows are built on a worker
   thread, a newer search cancelling an older one (checked every 4096 items), and arrive through `gui::mailbox`
   while the list keeps the rows it has: a million items never stall a key. The rows hold indices into the
   shared sections (`Arc`), not copies. The field is a combobox to AT, expanded, controlling the list (research §4 item 14). A
   disabled item takes no pointer and no pick, and its look dims it (`verdict/better.tsv`: shadcn-vue's
   `data-[disabled=true]` never matches reka's `data-disabled=""`).
7. **Tags input.** The keys (Backspace, Delete, the arrows by reading direction, Home, End) run in the field, before
   its editing, only from its start and while a tag can be used; a duplicate is refused, kept typed, and marks the
   input invalid until the draft changes (state and AT only: shadcn-vue draws no invalid look for it); a limit
   refuses without marking. The selected tag carries shadcn-vue's active ring (`data-[state=active]:ring-2
   ring-offset-2`). A delete button is out of the Tab order and named for its action ("Remove apple",
   `TagsNames`), not by the tag alone. `TagsInput::set_availability` makes it refuse everything (Enter, the
   delimiter, Tab, paste, blur, the keys) when disabled or read-only, as reka's `TagsInputRoot` `disabled`
   does. The delimiter is a character, a string or a pattern (`Delimiter::Pattern(regex::Regex)`, reka's
   `string | RegExp`); `add_on_enter` is reka's `addOnEnter`. shadcn's `tags_input` forwards every reka prop
   (`add_on_enter`, `delimiter`, `max`, `add_on_paste`, `add_on_tab`, `add_on_blur`, `duplicate`, `disabled`).
8. **Tree.** Rows are the open part of the caller's data, by index; Right and Left open, close, and go to the first
   child or the parent, by reading direction; a click chooses and opens or closes; `propagate` chooses
   descendants, and a parent once all its children are, half chosen (`Toggled::Mixed`) when some are. One Tab
   stop, kept on the highlighted row and moved up when its branch closes. The rows are a `Computed`, rebuilt
   only when what is open changes (an iterative walk: a chain of any depth flattens), with a key-to-row map, so a
   key, a click and a choice read no data and find a row in O(1); the half-chosen set walks up a parents map from
   the chosen keys. `Tree::virtual_list` lays a million rows on `gui::list`, the highlight scrolled into view and
   focused after layout (reka's `TreeVirtualizer`).
9. **AT, and what the engine gained.** `gui::Div` gained `aria_multiselectable`, `aria_required`, `aria_has_popup`
   and `aria_auto_complete` (`Announce`, mapped in `access::node_of`). A select's list is multi-select when it
   holds several (reka's never is, §9); its trigger is a required combobox when its field is. Options are
   `option` with `aria-selected` and their place in the set; a combobox's and a command's field names the
   highlighted option as its active descendant. `gui::inspect::find_text` (a scenario's `locate`, `click_text`)
   skips invisible nodes, and `Scenario::click_text` counts the press's handlers as well as the click's: a select
   opens on the press, so its click lands on the list. `gui::Positioning::Over` gained `direction` and
   `min_height` (`placement::over`), and `primitives::form::FormField` gained `named_by` (a field whose form
   data is not its value as is).
10. **Select placement and form.** `Select::arrow` is the popper's arrow (10 × 5, on the edge facing the trigger,
    hidden with the list). Item-aligned placement (`Positioning::Over`) keeps a 10 px window margin, reads the
    reading direction (the trigger's start edge, mirrored in RTL), and keeps at least five rows' height
    (`Select::aligned_min_height`) by moving the list up when the window below is short; when the up button
    first shows, the list places again with the anchor in view (reka's `onScrollButtonChange` reposition). An
    empty select sends `Select::set_fallback`'s value ('' by default) in a form, as reka's native `<select>`
    sends its empty option (`FormField::named_by`). Not built, for a merit reason: reka's growth of an
    item-aligned list as it scrolls. It changes the popup's size on every scroll frame, so a scroll re-lays out
    the popup on the UI thread, where ADR-0016 has a scroll write one property-tree entry and re-composite on
    the render thread; the list is placed with its anchor in view and scrolls inside its height. Not built
    either: reka's `ComboboxVirtualizer` as a separate part (the combobox reads its options by index and a
    command list is virtual already) and a tree's drag and drop (reka has none; its demo wires a separate
    library).
11. **Pixels.** The select, combobox, tags-input, field and form pages shoot identical frames. The command page
    differs in its last two frames only, by the stale-highlight fix (item 6): ArrowDown twice after the search is
    cleared reaches Profile, as the frame's name says, and Enter picks it. The debug overlays differ where a closed
    select's hidden options are now laid out. After the review fixes (Listbox parts, the combobox's layer, the
    command's worker filter), every frame of the select, combobox, tags-input, field and form pages is
    identical, and the command page's first frame draws the focused search field's caret at once (it showed
    one frame later before); the debug overlays differ by that repaint and by the combobox trigger's repaint
    as focus returns to it.

## Group 3: review fixes (M10 S6b)

Built 2026-11-22, after the independent review of group 3. Each guarantee that group 3 listed as not ported
is built, except two that have a merit reason (the end of this section).

1. **Engine (gui) edits**, each the smallest that carries the rule:
   - `DragEvent` tells `kind` (mouse, pen, finger) and `on_control`: the press went down on a control below the
     dragged node. A control is a node with a click handler, a field, or a button, link, checkbox, radio, switch,
     slider, combobox, text input or label.
   - A finger's pan starts by scrolling a container between the hit node and the dragged node, if that container
     can still scroll the pan's way. Otherwise it drags. A mouse drag never scrolls in gui, so this rule is only
     for touch (vaul's scroll-edge rule, as an engine rule).
   - `interact::window_active()`: the window's focus, tracked and fed by the platform chrome. Also
     `Scenario::set_window_active`.
   - `Div::on_window_key`: a key handler that runs on every key press the window gets, after the focus chain,
     unless its node is inert or blocked by a modal. This is how an F8 hotkey is heard wherever focus is.
   - `Layer::tab_stops(false)`: the nodes in this layer are no Tab stops, except one that holds focus now.
     `Layer::branch()`: a layer whose presses count as inside every other layer (Radix's `DismissableLayerBranch`).
   - `Deferred::anchor_part(key)`: a layer anchored to a keyed node in its owner's subtree, not to the owner's box.
   - Motion: a leaving node whose layer stays declared but holds nothing goes back into it. Before this, its exit
     was dropped and `Stage::Exited` was never told, for every overlay that keeps its layer declared.
2. **Drawer.**
   - Snap points are a two-way model (`Drawer::new(open, snap, options)`), with `sequential_snap`.
     `Swipe::with_room` lets the pointer drag the content open, linearly, up to its snap offset; past that a
     square-root rubber band damps it.
   - Settled: `on_settled(true)` is told when the entry has run, and `on_settled(false)` when the exit has. It is
     not told when the other way interrupted it, nor when the root unmounts. With `keep_mounted` it is told as
     the value changes, because no presence stage runs.
   - `FinalFocus { Trigger, Nowhere, Node(FocusHandle) }` chooses what gets focus back.
   - A drawer nested in another tells its parent how many drawers are open in it, the frontmost one's height,
     and its swipe. A nested drawer has no overlay unless `overlay_nested` is set.
   - reka reports a child's swipe to the grandparent and not to its parent. This is fixed by intent: each drawer
     tells its own parent.
   - `DrawerProvider` serves the page behind: whether any drawer is open, the frontmost one's progress and its
     height (reka's `DrawerProvider` and `DrawerIndent`).
   - `swipe_area` opens the drawer by a swipe from the edge. `no_swipe` marks a region that never swipes.
   - A press on a control: a mouse or a pen does not swipe the content. A finger does, as in vaul. In the swipe
     area, no pointer swipes from a control.
   - The window losing focus ends a swipe where it is.
   - shadcn's drawer look reads `snap_offset`, `offset`, `progress` (the overlay fades by `1 - progress`),
     `swiped` and `release_scalar`. While dragging, the duration is 0. After a swipe it is 500 ms times the
     release scalar.
3. **Toast.**
   - `ToasterOptions` holds these:
     - `hotkey`: F8 by default; shadcn's sonner uses Alt+T and labels its region "Notifications alt+T". The
       hotkey focuses the region.
     - `pause_on_window_blur`: on by default.
     - `limit`: toasts past the limit are inert and take no input.
     - `swipe` and `veto_swipe`: `ToastSwipe` is now wired to every toast. The click that ends a swipe is
       swallowed, and the swipe offset is a binding.
   - An action must have alt text (`AltText::new` refuses blank text). The alt text is the action's
     accessible name.
   - The region is a branch layer that never dismisses: a press on a toast does not close an open dialog.
4. **HoverCard.**
   - Its layer has no Tab stops, as reka's `HoverCardContent` does with `tabindex=-1` on each one.
   - `HoverCardOptions::touch`: a tap toggles the card, while the hover and focus that a touch makes are ignored.
     By default a touch does nothing, as in reka.
   - Not ported: keeping the card open while text in it is selected. gui selects text only in fields, and the
     browser's `getSelection`, which reka reads, ignores text in fields. So the rule has nothing to act on.
5. **Popover and dialogs.**
   - Every Dialog, AlertDialog, Drawer and Popover trigger says `haspopup=dialog`.
   - `Popover::anchor(d)` and shadcn's `popover_anchor`: once registered, the content is placed against the
     anchor and not against the trigger (reka's `PopoverAnchor`).
6. **Tooltip.**
   - Leaving the trigger while the tooltip is open but not yet placed closes it at once.
   - A click clears the `pressed` flag, so a press on a trigger that already had focus does not block the next
     focus-open.
7. **Not ported, by merit.**
   - The drawer's virtual keyboard inset: desktops have no on-screen keyboard that covers a field.
   - The hover card held open by a text selection: see item 4.
8. **Pixels.** At rest nothing moves, so there is no `better.tsv` entry. The AT tree adds `haspopup=dialog`
   (overlay snapshots), and toasts past the limit leave it (`overlays__toast_limit_a11y`).

## Group 6: review fixes (M10 S6b)

Decided 2026-11-22, after the group's review. Tests: `crates/primitives/tests/form_controls.rs`.

1. **A parent box tells AT what it controls.** `CheckboxGroup::parent` takes `all` as `(value, NodeRef)` pairs and
   sets `aria-controls` to those boxes (`CheckboxRoot.vue:256-262`; one not mounted is not told, as reka lists the
   mounted items only); `CheckboxGroup::item` takes the box's `NodeRef` and names its node with it, so a box can
   always be pointed at. The group's roving items are `checkbox::Member<T>` (`Parent` or `Box(value)`): the parent
   no longer joins under its first box's value, and two items never share one. AT snapshot `a11y_checkbox`.
2. **Required is told by the part.** `checkbox::root`, `switch::root` and `RadioGroup::root` take `required` and
   set `aria-required` (`CheckboxRoot.vue:311`, `SwitchRoot.vue:108-109`, `RadioGroupRoot.vue:99-105`), so a
   required control cannot be left unannounced; the check stays the form field's rule (`checkbox::required`,
   `switch::required`). shadcn's `checkbox()`, `switch()` and `radio_group()` gained `.required(bool)`. A group's
   boxes are not required one by one: reka's group `required` asks for one value in the list, a rule on the list.
   AT snapshots `a11y_checkbox`, `a11y_switch_toggle`, `a11y_radio_group`.
3. **A press with no click after it leaves no mark.** The number field's buttons remember that the pointer's press
   stepped, so the click after it steps no more; the mark is now dropped when the press ends with no click coming
   (its drag cancelled because the button turned disabled, or released off the button), so assistive technology's
   next press steps. The stepper's trigger keeps no mark at all: its click goes to the step unless Control is held,
   and after the pointer's press the step is already that one, so the model's equality cutoff writes nothing.
   Test `a_press_with_no_click_after_it_leaves_no_mark_on_the_next_at_press`.
4. **A label inside a disabled radio reads disabled.** The foundation's AT tree now marks the descendants of a
   disabled node disabled, as Chromium's `AXObject::Restriction` does for a disabled control's subtree; the radio
   snapshot (`Label "y" disabled`) was reviewed and kept.
5. **Tab entry is tested.** Tab and Shift+Tab enter a radio group at its checked item, else its first usable one,
   and a toggle group at its pressed item, else its first (`tab_enters_a_radio_group_at_the_checked_item_and_a_toggle_group_at_the_pressed_one`).
6. **The slider's off-grid max, as built.** End targets max and snapping rounds it to the grid (min 0, max 10,
   step 3: End gives 9); an arrow from 9 clamps up to 10, as reka. The module doc said End gave 10; it is corrected,
   and a unit test holds it.

No `better.tsv` entry: the looks are unchanged.

## Group 3: second review fixes (M10 S6b)

Built 2026-11-22, after the second independent review of group 3.

1. **Development warnings** (research §7, Dialog, AlertDialog and Drawer "Dễ sót"; reka's `useWarning`).
   - Title and description parts register from their setups (`register_title`, `register_description`, as an
     alert dialog's Cancel does); a part that unmounts gives its count back. shadcn's `ModalText` does it for every
     modal's title and description.
   - Once the content is laid out open, a debug build reports, once per modal and naming where its root was made:
     a content with no title (it is read unnamed), and a dialog or alert dialog with no description unless its
     caller said it has none. `DialogOptions::described` and `AlertDialogOptions::described` (reka's
     `aria-describedby="undefined"`) turn the description off: the content then points at none and nothing is
     reported. A drawer reports a missing title only, as reka's.
   - `missing()` tells the same at any time, in every build (tests, devtools).
   - reka's alert dialog warns with `DialogContent` and `DialogTitle`; here it names its own parts. Fixed by intent.
   - The docs' search palette (a `CommandDialog`) gets upstream's off-screen header, title and description.
2. **Toast.**
   - **Announcement.** Each toast is read by a live node of its own, off screen, held by the region (or by its
     positioner): the toaster's label (`ToasterOptions::label`, "Notification", a `Spoken`, never blank), then the
     toast's `Announcement` (its text in chunks, an action by its alternative text, no Close). A live node's name
     is one string, so each chunk is read as a sentence, ending with a full stop when it has no end mark, where reka
     renders a text node per chunk for the pause. The node is made again on each update (`Shown::updates`, reka's
     `updateKey`), so an update is read again. A toast past the limit has none. The toast node itself is a `group`,
     so its Close is not in what is announced.
   - **No two-frame delay.** reka shows the live text two frames (or 1000 ms) after its live node, for NVDA to see a
     change in a live region that already exists. AccessKit raises the live-region event itself when a node comes
     with a name (UIA `LiveRegionChanged`, the macOS announcement), so the toast is read in the frame it shows.
     Better by merit: no added latency, and no timer.
   - **Positioner** (`Toaster::positioner`, `ToastAnchor`, `POSITIONER`): a toast shown with an anchor is not in the
     region but on a layer of its own beside the node the positioner is declared in, on the top side by default
     (reka's defaults: top, arrow padding 5). It pauses the countdowns as the region does, is a branch of every
     layer, and holds its toasts' announcements. reka's collision padding has no counterpart: layout keeps a layer
     on screen.
   - **Promise** (`Toaster::promise`, shadcn's `toast::promise`): a loading toast (`ToastStatus::Loading`, which
     always stays) that the work's result, sent from any thread through a `gui::mailbox` owned by the toaster,
     turns into a success or an error toast in place, its countdown started and read again. A result for a toast
     dismissed meanwhile shows nothing. Async stays outside the reactive graph.
   - `show` takes `ShowOptions` (sensitivity, `Lifetime`, status, anchor, announcement); `AltText` became `Spoken`.
   - **Tab order** (reka's `ToastViewport`): the newest toast first, then its controls, then the older ones; out at
     either end; from before the region into the newest toast, from after it into the oldest toast's last control.
     The look declares toasts oldest first (the newest paints on top, as reka's DOM has them) and the region visits
     its children last first. Engine edit: `Div::tab_children_reversed` (`gui::focus::tab_order` reorders the stops
     under such a node by child, each child's stops, and its layers', kept in order).
3. **Clock.** The drawer's swipe samples and the toasts' countdowns read `gui::clock::current_time()` (a `Duration`,
   added for this) instead of `f32` seconds, which after three days of uptime are 15.6 ms apart, next to the swipe's
   16 ms least sample.
4. **Tests.** The AT dump shows `hidden`: the dialog, alert dialog and drawer snapshots show the page hidden below
   the modal. New: missing title and description (dialog, drawer), the toast Tab table with F8 and the focus a
   keyboard close gives back, the announcement and its re-reading, the anchored toast, the promise.
5. **Pixels.** The announcer takes no room and paints nothing; no `better.tsv` entry.

## Group 4: review fixes (M10 S6b)

Built 2026-11-22, after the independent review of group 4. This section supersedes Group 4's item 13 ("Not built"):
every part and guarantee it listed is built; and item 7's two recorded limitations (the long press's delay is the
caller's, 4 below; another area opening in the same press, "Group 4: second review fixes" item 2). Item 2's rule for a submenu closing on focus is corrected (1 below).

1. **A submenu closes on focus leaving it, not on focus merely moving.** Group 4 closed a submenu on any focus
   change that was not to its trigger or into it, so the pointer leaving the sub trigger for the parent content's
   empty area (`unhighlight` gives focus to the parent content) closed it, against research §8 ("Highlight biến mất
   → submenu vẫn mở"). Now only focus that was in the submenu and leaves it, for anything but its trigger or its
   parent's filter, closes it: reka's `focusout` on `MenuSubContent`. Test:
   `a_submenu_stays_open_when_the_highlight_merely_goes_away`.
2. **Drag-to-select.** `gui::Div::on_release`: a primary press that went down outside the node, released over it
   (or a child), runs its nearest `on_release`; a press and release on the node itself stays its click. Every menu
   item (and sub trigger) picks on it, as reka's `MenuItem` `pointerup` calls `click()`: a menubar trigger pressed,
   dragged to an item and released; a press on one item released on another; a context menu's long press lifted
   on an item. A touch or pen contact held into a long press neither pans nor drags after it, and its lift is a
   release where it lifts (in `gui::interact`). A dropdown opens on the click's release, so its trigger's press
   never drags into a pick (reka's).
3. **`DropdownMenuFilter`** (`primitives::dropdown_menu::DropdownMenuFilter`, on `Menu::use_filter`,
   `Menu::filter_key`, `Menu::active_node`, `Menu::scroll_with`): a `searchbox` naming the highlighted item as its
   active descendant. While it holds focus the highlight is the level's own (`Level::active`, reka's
   `highlightedElement`), not focus: the pointer and the vertical arrows, Home and End move it (focus stays in the
   field) and bring it into the content's scroll view the nearest way; Enter picks it; Escape empties a field
   with text and keeps the menu, and closes it when the field is empty; while an input method composes every key
   is its own; typing (Space included) types. The pointer coming into the content focuses the filter; a submenu
   closed by its key gives focus back to the filter with its sub trigger highlighted. With `auto_focus` it takes
   focus once laid out (in a submenu only from the keyboard), where reka waits 1 ms. The caller filters the items
   from the field's text, as reka's filter does not filter. The item ids reka's docs ask callers to give are the
   level's own node refs (research §8 "Dễ sót"). shadcn-vue has no filter part, so shadcn adds none.
4. **ContextMenu's `pressOpenDelay`**: `ContextMenu::press_open_delay`, shadcn's `.press_open_delay(…)` on the
   root, on `gui::Div::long_press_after` (the long-press delay is read from the node as the contact goes down;
   `gui::gesture` keeps it per press).
5. **NavigationMenu.**
   - `disable_click` turns off the mouse's click only: a touch or pen tap and Enter or Space still toggle (reka's
     `pointerType === 'mouse'` check). Group 4 blocked every activation, so the panel could not open from the
     keyboard at all.
   - **Viewport** (`NavigationMenu::use_viewport`, `viewport`, `open_item`, `viewport_size`): one surface draws the
     open item's panel; the pointer in it holds the panel open, leaving it closes it after 150 ms. shadcn's
     `navigation_menu()` shows its viewport by default, as shadcn-vue's (`viewport: true`): below the menu at its
     start, `mt-1.5`, `rounded-lg`, border and shadow, `zoom-in-90` / `zoom-out-90` in 100 ms, the panel sliding by
     its motion inside it; `.viewport(false)` is shadcn's demo (`:viewport="false"`), which the docs page passes.
   - **Tab order without a focus proxy.** A viewport is declared in the root, after the list, but its Tab stops
     follow the open trigger: `gui::Deferred::tab_after(key)` puts a layer's Tab stops after the subtree of a keyed
     node inside the declaring node (`NavigationMenuItem::trigger_key`). Tab from the open trigger enters the
     panel, Tab past its end reaches the next item, Shift+Tab from the next item reaches its last link: what
     reka's `VisuallyHidden` proxy and its Tab handlers make. We differ on purpose: the engine knows the Tab
     order, so no invisible focusable node is added (reka's proxy is a stop assistive technology reaches and reads
     as an empty element). The open trigger owns the panel for AT while a viewport draws it (reka's `aria-owns`
     node).
   - **A panel that is not open has no Tab stops** (reka's `removeFromTabOrder`): its links' Tab stop is bound to
     the item being open, read when Tab looks, so a panel kept while it leaves is skipped. reka also takes an open
     panel's links out when focus leaves it for elsewhere in the menu and restores them through the proxy; with
     the order above they are reached from the same two places (the trigger and the next item), so nothing more is
     needed.
   - **`NavigationMenuSub`** (`NavigationMenu::sub`): its own value and orientation; its triggers switch at once on
     hover, the pointer leaving closes nothing, a press outside its panel never closes it, Escape closes its panel
     alone and focuses its trigger; focus anywhere in the navigation (`NavigationMenu::holds`, nested menus
     included) is not outside any panel.
   - **Space once per press.** `gui::window::Input::KeyRepeat`: the platform's repeat of a held key (winit's
     `repeat`; the input log writes it `repeat …`) runs as a key press, but Space does not press the focused
     control again, as a browser presses a button on Space's release. A held Space on a trigger toggles once.
6. **Toolbar toggle group** (`Toolbar::toggle_group`, `Toolbar::toggle_item`, reka's `ToolbarToggleGroup`,
   `ToolbarToggleItem`): a `group` whose items are toggles pressed while the `ToggleGroup` holds their value and
   items of the toolbar's one Tab stop and arrows (reka's group with `rovingFocus` off inside the toolbar).
7. **Tests** (`crates/primitives/tests/menus.rs`): the RTL menubar (the inline-end key from a submenu's plain item
   opens the next menu; the other closes the submenu only), the RTL navigation menu's arrows (triggers and panel),
   menubar wrap, Tab leaving the bar and Shift+Tab back to the trigger focused last, the caller's value; toolbar
   memory of the last item, Shift+Tab, an all-disabled toolbar being no Tab stop, the toggle group; the navigation
   menu's `disable_click`, `disable_hover`, `disable_pointer_leave_close`, Tab into and out of a panel, a held
   Space; drag-to-select (mouse and long press), `press_open_delay`, the filter's keys, its input-method case and
   its active descendant; the viewport's Tab order, size and hover; the nested menu. New AT snapshots:
   `menus_filter_a11y`, `menus_toolbar_toggle_a11y`.
8. **Engine (gui) edits**, each the smallest that carries the rule: `Div::on_release`, `Div::long_press_after`
   (`interact`, `gesture`, `element`); `Deferred::tab_after` (`element`, `component`, `layout::Layer`, `layer`,
   `focus::tab_order`); `Input::KeyRepeat` (`window`, `app`, `interact`, `compositor::input_log`).
9. **Pixels.** Item 15's frames are docs script frames, not verdict pages: the verdict judges each page's demo at
   rest, where nothing of item 15 shows. Scores of the four pages, `crates/shadcn/verdict/run.sh dropdown-menu
   context-menu menubar navigation-menu` (vega, light), against the full vega run before group 4's review:
   context-menu 100 (100 before), dropdown-menu 9 (9), menubar 100 (100), navigation-menu 0 (0), each demo's
   differing share the same to the tenth of a percent (0.0 %, 35.6 %, 0.0 %, 60.3 %). The two low scores are
   sizes from vega's values S9 has not written yet (ours 70 and 548 px wide against 58 and 512), the same before
   and after. No `better.tsv` entry: nothing here differs from Chrome on purpose.

## Group 4: second review fixes (M10 S6b)

Built 2026-11-22, after the second independent review of group 4.

1. **A link picked at any depth closes the whole navigation menu.** A link in a nested panel
   (`NavigationMenuSub`) closed only its own menu; reka sends its dismiss up to the root content (research §8
   NavigationMenu, "chọn Link ở tầng sâu đóng cả cây"). Now a link of the list or of any panel closes the root menu
   and every menu nested in it, and focus held in the panel goes back to the trigger of the root item whose panel
   holds the link. A Command-click neither closes nor moves focus (it opens elsewhere; focus moved before). Test:
   `a_link_picked_in_a_nested_panel_closes_the_whole_menu` (keyboard and mouse).
2. **Another context menu's area opens in the same secondary press** (reka's `ContextMenu.browser.test.ts:28-38`).
   The engine rule, in `gui::interact`: a secondary press that closes the top modal layer reaches what is under it
   (hit-tested down to the next modal layer below), as a platform's context menu hands a right-click outside it to
   what is below. A primary press still reaches nothing below a modal layer (ADR-0024: a click outside a menu or a
   dialog closes it and does nothing else). The area's own secondary press still moves the open menu (its veto).
   Test: `a_secondary_press_on_another_area_closes_one_menu_and_opens_the_other`.
3. **A navigation panel is read inside the landmark** whether or not a viewport draws it: the open trigger owns
   its panel (`aria-owns`), as reka's panel without a viewport is rendered in its `li`. Group 4 owned it only with
   a viewport, so shadcn's demo (`.viewport(false)`) read its links outside the `navigation`. Snapshot
   `menus_navigation_a11y` (the dump now prints `owns`).
4. **Tests added**: the indicator's place (`NavigationMenu::active_trigger`) follows the open trigger and keeps
   its last place while every panel is closed; ArrowUp (and ArrowDown) from the content itself reach the last (the
   first) item though the menu does not wrap (research §8 "Dễ sót"); Tab and Shift+Tab in a modal menu leave focus
   on its item.
5. The toolbar's module doc said its separator takes the toolbar's orientation; it turns across it (Group 4 item
   10), and the doc now says so.
6. **Pixels.** None of this paints: the verdict pages of dropdown-menu, context-menu, menubar and navigation-menu
   score as in "Group 4: review fixes" item 9.

## Group 5: review fixes (M10 S6b)

Decided 2026-11-22, after the group's review. Tests: `crates/primitives/tests/select_lists.rs`.

1. **A virtual listbox keeps the listbox guarantees over its data** (research §4 item 15, §9 "Ảo hóa"). Its data
   is given once in setup (`Listbox::set_virtual(state, source)`, `listbox::Source<T>`, the type the combobox's
   `set_source` takes too) and read by every path that read only the mounted `Collection` before: the mount
   highlight, a caller's value, Tab entry (the option focus last left, else the chosen, else the first), the
   filter's first-match highlight and the keys. A virtual list brings the highlighted option into its own view on
   each of them (the mount included: it scrolls the list, never the page, `ListboxVirtualizer.vue:184-195`), and a
   caller's value moves focus to the list, whose options are its active descendants; that focus is not an entry by
   Tab, so it does not replace the highlight with the one last left. `virtual_content` takes the list's keys itself
   (no `virtual_key` for the caller to wire), and `virtual_option(d, options, index)` reads availability from the
   data; a click highlights the option and chooses it, found again by value if the data moved under the row. Test
   `a_virtual_listbox_keeps_the_listbox_guarantees_over_its_data`.
2. **Hover keeps the highlight read in O(1).** `ListCore::option` and `Combobox::option` take the option's index
   and the pointer stores it with the value; before, hover stored the previous highlight's index, so every later
   read missed and walked the options to find the value (a million value reads per pointer move on a million
   options). A command's hover finds the item's index from its place in its rows with the list's item count kept
   in `FlatRows`, not counted per hover. Test `hovering_a_virtual_option_keeps_the_highlight_read_in_o_1` (fewer
   than 1000 value reads per hover over a million options).
3. **Select-all is the Listbox's only** (`ListConfig::select_all`, on in `ListConfig::LISTBOX`): reka's
   `ListboxRoot` and `ListboxVirtualizer` take Control, Alt or Command + A; its Select and Tree do not, so ours set
   it off. A Select of several had chosen everything on Control + A. Tests `a_select_of_several_has_no_select_all`,
   `a_tree_s_shift_range_leaves_disabled_rows_out_and_a_click_toggles`.
4. **A tree's Shift range moves focus with its end.** The tree's highlight is its focus; a Shift range (and any key
   the core answers with `ChoseMany`) moved the highlight only, leaving focus and the Tab stop on the old row.
   `Tree::key` now focuses the highlighted row after every move or range.
5. **A paste is a paste, and adds the pasted text only.** With `add_on_paste`, the tags input takes the platform's
   paste key (Command or Control + V, the Paste key) before its field inserts: the clipboard's text alone is split at
   the delimiter (no delimiter in it: one piece, as reka's `split` gives it whole), each piece added by the one
   adding rule (trimmed, a blank piece dropped), the draft left as it is (`TagsInputInput.vue:132-150`).
   `TagsInput::paste(text)` is that path, for a caller's own paste. Before, a paste was "more than one character
   inserted", so a paste without a delimiter added nothing, the draft joined the paste, and an input method's
   commit or an autocorrection holding the delimiter was taken for a paste. Ours differs from reka by intent: what
   a paste could not add (a duplicate, the limit) is pasted as text at the caret, joined by the delimiter, instead
   of dropped, so nothing pasted is lost. Tests `a_tags_input_pastes_its_pieces_and_leaves_the_draft`,
   `a_tags_input_adds_on_tab_and_blur_up_to_its_limit`.
6. **Tab reaches the focused node's own key handler first** (`gui::interact`, a foundation edit): a DOM `keydown`
   listener sees Tab before focus moves and may prevent it; ours never ran, so `add_on_tab` could not work. Only
   the focused node's own handler sees it (not its ancestors), and Tab moves focus when it returns `false`.
7. **A select's list gives focus back to its trigger on every dismissal**, a press outside included, unless
   focus is already elsewhere (`SelectContentImpl.vue:305-312`); the engine's modal focus scope returned it too, so
   the test passes either way, and the rule is now the select's own. Tests for a press outside (closes, reaches
   nothing below, focus on the trigger), a press dragged past `PRESS_SLOP` and released outside the list (closes),
   and touch (opens as the tap ends; a moved touch chooses nothing; a tap chooses).
8. **The combobox can be disabled** (`Combobox::set_availability`, reka's `disabled`): it does not open by typing,
   the keys, the button or the trigger; the field is disabled, the button and trigger are read as disabled. The
   trigger controls the list while open (`aria-controls`, `ComboboxTrigger.vue:39-55`). Test
   `a_combobox_trigger_controls_the_open_list_and_is_disabled_with_it`.
9. **Corrections to item 1 and item 5 of the group's section.** Mounting highlights the chosen option, else the
   first usable one (reka's `highlightSelectedItem`), not "else none". Escape on an autocomplete with its list
   closed keeps the text (reka's `Autocomplete.test.ts`: "should keep input text on close"); only the cancel
   button empties it (`ComboboxConfig::AUTOCOMPLETE.cancel_clears_value`).
10. **shadcn's select takes `required`** (`.required(bool)`): the trigger is a required combobox to AT, as the
    primitive's `set_required` makes it. Not built: a `name` prop. No shadcn part carries a form name (checkbox,
    switch, radio group neither); a form sends a control through `primitives::form::FormField::named` /
    `named_by` over the primitive, which is where the select's fallback is read (`Select::form_data`).
11. **Pixels.** Nothing here changes a look. `crates/shadcn/verdict/run.sh select combobox command tags-input`
    (vega, light) scores every demo as the full vega run before these fixes did, to the same differing share
    (SelectDemo 71, SelectMultipleDemo 71, SelectScrollable 73, the five combobox demos 64 to 71, CommandDemo 88
    BETTER, both tags-input demos 100; the FAILs are vega's values S9 writes, the backlog's 64/100). No
    `better.tsv` entry: the differences from reka here are behaviour and AT, not pixels, so they are recorded here
    (select.rs and tags_input.rs point here, not to `better.tsv`).

## Group 3: third review fixes (M10 S6b)

Built 2026-11-22, after the third independent review of group 3.

1. **A non-modal dialog opened by code closes once on the next press of what opened it** (research §7, reka's
   `DialogContentNonModal` `interactOutside` with `getFocusedBeforeOpen`; `DialogReturnFocus.browser.test.ts`:
   "closes on the next click of that button instead of reopening"). The dialog records what held focus each time
   it turns open, before its content takes focus (reka's `recordFocusBeforeOpen`), and names it on its layer.
   Engine edit: `gui::Layer::opener(Option<FocusId>)`; for a layer that is not modal, the node holding that focus
   (and its subtree) is inside it as its owner is (`Stack::inside`), so a press or focus there asks nothing and
   the toggle's click closes it once. Before, the press closed it and the click opened it again. reka keeps the
   previous record when the dialog reopens while its exit still plays with focus in its content; here the record
   is what holds focus then. Pressing that content node is inside anyway, so the only difference is which node
   outside counts as the opener in that one case. Test:
   `a_non_modal_dialog_opened_by_code_closes_once_on_the_next_press_of_what_opened_it`.
2. **A press held in a hover card keeps it open, on every path** (reka's `isPointerDownOnContentRef`, checked in
   `handleClose`). The grace area's exit now checks the press too, as `request_close` already did. Before,
   pressing in a placed card and dragging out past the hull closed it after 300 ms. **Let go:** a press that ends
   with the pointer on neither the card nor its trigger starts the close delay then. reka asks nothing as the
   press ends, so its card stays open until the pointer next comes and leaves. The press was only meant to keep
   the card while held, and a card left open with the pointer far away is the bug the delay exists to avoid.
   Fixed by intent. Test: `a_press_held_in_a_hover_card_keeps_it_open_past_its_grace_area_until_let_go`.
3. **Another hover trigger ends a grace area** (reka's `[data-grace-area-trigger]` in `useGraceArea`). The
   group's first section left this rule out because only one tooltip is open at once. That holds for tooltips
   and not for hover cards: a card whose area covered another card's trigger stayed open while the other
   opened. Now each tooltip and hover-card trigger tells its grace area when the pointer comes onto it or leaves
   (`Grace::trigger`). Any other area held then drops and tells its owner, even inside its hull, and a hover card
   starts its 300 ms close. Test:
   `a_hover_card_starts_its_close_when_the_pointer_reaches_another_cards_trigger_in_its_grace_area`.
4. **A toast action with a blank label is refused and reported** (shadcn's `Toast::action`). `ToastAction::label`
   is a `Spoken`, so the look never meets a blank label. `Toast::action` with a blank or whitespace label adds no
   action, and a debug build names the call site, as reka throws on an action with no `altText`. Before, the
   action became a plain closer and its `on_click` was dropped silently. A button that shows nothing and is read
   as nothing is no action.
5. **The tooltip's arrow and the tooltip's and hover card's transform origins read the side placed after
   flipping** (research "Look cần đọc"). The arrow reads `Tooltip::placement`'s side, and the side asked for
   until layout first places it. The tooltip and hover card zoom about `Origin::Anchor`, as the popover does
   (upstream's `origin-(--reka-tooltip-content-transform-origin)` and
   `origin-(--reka-hover-card-content-transform-origin)`). Before, they zoomed about the centre. An unflipped
   tooltip looks the same at rest; only the opening and closing frames change, toward upstream's.

## Group 8: review fixes (M10 S6b)

Decided 2026-11-22, after the group's review. Tests: `crates/primitives/tests/structure.rs` and the unit tests of
`carousel`.

1. **Carousel goes round** (`CarouselOptions::wrap`, reka's `loop`) when a slide's worth stays out of view on
   either side (`carousel::loop_fits`); a track too short runs back from the last snap to the first, as before.
   Going round, every slide is a snap. Each slide is translated by whole tracks into the lap that puts it within
   half a track of an anchor. The anchor is the middle of the current move, so the slides it passes are in place
   all the way; a move longer than they cover goes at once. This follows Embla's slide looper; reka reorders with
   CSS `order`, which gui's retained tree does not need. Steps go on round; `go` and an indicator take the
   shorter way; a drag or a fling has no end. At rest the position is taken back to the first lap, a change of
   lap that moves nothing on screen. Tests `carousel_wraps_when_asked` and the placement unit tests.
2. **Carousel fades** (`fade`): the slides are translated onto the first, one snap each. The shown one has
   opacity 1 and the rest 0, inert; the track does not move and a step settles at once. A swipe steps when it
   travels `SWIPE_DISTANCE` (40 px) or is faster than `SWIPE_VELOCITY` (0.3 px/ms), timed from the press, as
   `useCarouselViewport.ts:696-704`. With `wheel` any wheel pages. Test
   `carousel_fades_one_slide_over_another_in_place`.
3. **Carousel auto-scrolls.** `Rotation` is now `{ pace, pause_on_hover, stop_on_interaction }`. `Pace::Step {
   delay, stop_on_last }` is the old rotation; `Pace::Scroll { speed, backward }` runs the track on at `speed`
   px/s (reka's 60 by default, `Rotation::scroll`). It advances on UI-thread frame timers (16 ms, a frame's step
   capped at 64 ms), as the drag-free glide does. The snap shown follows the track. It stops at an end of a
   track that does not go round. Paused or stopped, the track rests on the nearest snap. `on_settle` is not
   heard while it runs. `Carousel::next_step()` gives when the next step's count began and how long it is, so a
   look can draw the progress to the next step. Test
   `carousel_auto_scrolls_at_its_speed_and_rests_on_a_snap_when_paused`. Moving the track renders nothing.
   Only the frame where a slide enters or leaves the view renders, because its `inert` and AT hidden state are
   plain node values in gui.
4. **Carousel auto height** (`auto_height`, horizontal): each slide is aligned to the start, so it keeps its
   own height. The viewport's height is bound to `Carousel::height()`, the tallest at-rest slide in view as last
   laid out (`useCarouselViewport.ts:247-259`). Test `carousel_auto_height_follows_the_slides_in_view`.
5. **Carousel hands focus on.** Focus inside a slide going out of view goes to the first Tab stop of the slide
   coming in, else to the root (reka's `rescueFocus`, `useCarouselViewport.ts:208-238`), through the new
   `gui::focus::hand_on`. Test `carousel_hands_focus_on_to_the_slide_coming_in`.
6. **Render counts.** A drag's moves after its start render nothing (`carousel_a_drag_moves_with_no_render`).
   The scroll thumb moves with no render (`scroll_area_a_scroll_moves_the_thumb_with_no_render`): `has_thumb`
   now reads a `sizes` signal written only when the viewport or content size changes, not the scroll metrics.
   The tabs indicator follows a selection with no render (`tabs_indicator_follows_the_selection_with_no_render`).
   Its presence is a `Computed<bool>`, so the part also appears once layout first places the tab. Before, an
   untracked read could leave it absent. A splitter drag renders no panel between its start and end: the drag
   signal changes only then, and the last pointer place is a plain cell
   (`splitter_handles_crossing_drag_together_in_two_dimensions`).
7. **Splitter groups in groups** (reka's document-wide registry, `registry.ts:103-334`). A group laid out in
   another's panel joins its family by context (`provide`/`inject`), not through a global registry. The pointer
   in reach of handles of several groups hovers them all, and a press drags them together, each along its own
   axis. Where two handles cross, the cursor is `Move`, or the diagonal or one-way cursor of the ways still
   open (reka's `style.ts`). A handle is reached through the group the pointer is over, or the group around it,
   never through a node laid over it; reka compares stacking order for that. Test
   `splitter_handles_crossing_drag_together_in_two_dimensions`.
8. **Splitter panels take no pointer during a drag**, their content included: `Div::pass_through_all`,
   inherited by the subtree as CSS's inherited `pointer-events: none`, which reka sets on the body. Hover flags
   inside a panel stay as they were. Test `splitter_panels_take_no_pointer_while_a_drag_runs`. The room a drag
   divides is the group's room less its handles (`self.size`), as the keys use. Before, it was the handle
   event's bounds.
9. **ScrollArea right to left.** The arrows are physical: ArrowLeft shows what lies further left, and
   ArrowRight what lies further right, as Chrome does in an RTL scroller. Before, they ran the wrong way. Test
   `scroll_area_keys_scroll_the_view_physically_right_to_left`.
10. **Tabs** under automatic activation select on any focus, from code too, not only under keyboard modality
    (`TabsTrigger.vue`'s `@focus`). A press focuses and selects in one gesture. It asks a controlled caller once:
    the ask is keyed by the focus and the frame. Tests `tabs_automatic_selects_on_focus_from_code_too` and
    `tabs_controlled_by_a_value_only_asks`.
11. **Avatar fallback delay** (`avatar::Fallback::new(delay, status)`, `shown()`) counts again each time the
    status goes from loaded to not loaded. A new picture that fails waits its delay again before the fallback
    shows, so a quick swap never flashes the fallback. reka counts only from mount; this follows the delay's
    intent. shadcn's `avatar_fallback` passes the root's status.
12. **gui additions.** `gui::focus::hand_on(from, into, or)`: applied while the next frame settles, only if
    `from` still holds focus and no node holds it any more. `Div::pass_through_all(bool)` and the layout's
    inherited `passes`. The cursors `ResizeNorthEast`, `ResizeNorthWest`, `ResizeSouthEast`,
    `ResizeSouthWest` and `Move`, mapped to winit's.
13. **AT snapshots.** `carousel_a11y` and `carousel_indicators_a11y` now read the label inside a disabled
    Previous or Next as disabled. That comes from the foundation's disabled subtree (Group 6 review fixes, item
    4), reviewed and kept.

No `better.tsv` entry. Our images of the group's 14 demos are byte-identical to the run before these fixes; no
demo shadcn ships loops, fades, auto-scrolls or sizes by height.

## Group 4: third review fixes (M10 S6b)

Built 2026-11-22, after the third independent review of group 4.

1. **A control with keys of its own keeps them in a toolbar** (research §8 Toolbar, `Toolbar.test.ts:150-163`). An
   item took the arrows in its own key handler, and `gui::Div::on_key_down` replaces a node's handler, so a
   dropdown trigger made a toolbar item lost either its keys or the toolbar's, depending on the order. Now the
   arrows are the toolbar root's: `Toolbar::item` joins the group through `Roving::member` (the item's Tab stop and
   collection entry, no key handler) and `Toolbar::root` takes, through `Roving::focused_key`, what the focused
   item's own handler left as the key bubbles up. That is reka's order: the group's listener runs after the
   item's, and an item's handler that takes a key stops it. Both orders compose (`toolbar.item(dd.trigger(d, ..),
   ..)` and `dd.trigger(toolbar.item(d, ..), ..)`): ArrowDown opens the menu, in a vertical toolbar too, Enter and
   Space open it, and the arrows the trigger leaves move along the toolbar. No public key step was added to the
   triggers: composing by bubbling needs none, and keeps one handler per node. Test:
   `a_menu_trigger_in_a_toolbar_keeps_its_keys` (horizontal and vertical, both orders). Foundation edit:
   `Roving::member` and `Roving::focused_key` in `primitives::roving`.
2. **A context menu's long press tolerates 10 px of movement**, a deliberate difference from reka, which cancels
   it on any `pointermove` (research §8 ContextMenu, "Dễ sót"). A finger held still still moves a pixel or two, and
   touch screens report that jitter, so reka's long press fails to open on a real finger now and then; the engine's
   long press (`gui::gesture`, 10 px slop, the platforms' touch slop) is the one every long press here uses. A move
   past 10 px still cancels it, as a pan. This corrects Group 4 item 3, which listed the slop among the numbers
   kept from reka. No `better.tsv` entry: Chrome's own long press on touch has a slop as well.
3. **Tests added**: the filter's highlight brought into the content's scroll view the nearest way
   (`a_highlight_moved_from_the_filter_is_scrolled_into_view_the_nearest_way`, on `Menu::scroll_with`); a group
   named by its label only when it has one (`a_group_is_named_by_its_label_only_when_it_has_one`, `menu::group`,
   `menu::label`); a right-to-left toolbar (`a_horizontal_toolbar_swaps_its_arrows_right_to_left`); a press on the
   trigger of a menu that is not modal (`a_press_on_the_trigger_closes_a_menu_that_is_not_modal`); shadcn's
   menubar `default_value` and `wrap` (`crates/shadcn/tests/menus_focus.rs`).
4. **Unused API removed or used.** `OpenFocus::Last` is gone: no trigger in reka opens a menu on its last item
   (a dropdown trigger's ArrowUp does nothing, research §8). shadcn's checkbox and radio items show their mark
   through `menu::indicator_shown` (reka's `MenuItemIndicator`), so that rule is the primitive's.
5. **shadcn's menubar** takes `.default_value(…)` (reka's `defaultValue`) and `.wrap(Wrap)` (reka's `loop`, off),
   which the primitive had and shadcn could not reach.
6. **Pixels.** Nothing here paints differently: the indicator shows on the same condition as before.

## Group 5: second review fixes (M10 S6b)

Decided 2026-11-22, after the group's second review. Tests: `crates/primitives/tests/select_lists.rs`.

1. **A propagating tree chooses alike by key and by click.** Enter and Space go through the same propagation as a
   click (reka's `handleSelect` -> `rootContext.onSelect` either way, `TreeItem.vue:197`): the descendants are
   chosen or let go and the parents fill in. Before, only the click propagated. Test
   `a_propagating_tree_chooses_alike_by_key_and_by_click`.
2. **Each combobox reset holds for a choice on its own.** reka closes, then writes the value, and puts the text
   back on closing 1 ms later, so a choice of one value shows the chosen option's text when either
   `reset_on_close` or `reset_on_select` is on, and keeping the typed text takes turning off both (research §9
   "Dễ sót"). Ours closes at once, so `Combobox::choose` writes the value, then closes, then puts the text back
   when `reset_on_select` is on and the value changed; Enter on one value goes through `choose` too (it wrote the
   value apart before). Test `each_reset_option_puts_the_text_back_after_a_choice_on_its_own` (the four pairs).
3. **A command's option is told its group as its description**, a deliberate difference from reka's
   `ListboxGroup` (`role=group` named by its `ListboxGroupLabel`). The command's rows lie flat on one virtual
   list (item 6 of the group's section: a million items), so no node holds a group's options, and a relation to
   the heading (`aria-labelledby`, `aria-describedby`) is not told while the heading is scrolled out of view. The
   heading's text is the option's `aria-description` (`Command::option`'s `group`, `CommandSection::heading`),
   so a screen reader reads "Profile, Settings" wherever the list is scrolled; the heading row stays a text row
   in reading order. This replaces item 6's "a heading is a group label to AT", which the code never did.
   Foundation edit: `gui::Div::aria_description` (`Announce::description`, mapped in `access::node_of`). The
   `command_a11y` snapshot shows each option's group.
4. **The empty state shows only under a search** (`CommandEmpty.vue:15`: a search, and nothing matches), so a
   command with no items and no search shows nothing. Corrects item 6 of the group's section ("the empty state
   when none does").
5. **Listbox, Combobox and Autocomplete are `form::FormValue`s**, as Select and TagsInput are (reka's hidden
   input): a form field binds the primitive itself, and a failed check focuses the listbox's content or the
   field (the raw value signal had no focus to give).
6. **Dead API removed**: `select::KEYMAP` and `tree::KEYMAP`, which nothing read (both navigate by
   `ListConfig::LISTBOX`'s keymap). The keyboard-table tests check the option assistive technology is told has
   focus, not the list's highlight.
7. **Pixels.** Nothing here changes a look: every frame of the combobox, command, select, tags-input, field and
   form pages (`docs --shots`) is byte-identical to the shots after the first review fixes.

## Group 8: second review fixes (M10 S6b)

Decided 2026-11-22, after the group's second review. Tests: `crates/primitives/tests/structure.rs`.

1. **ScrollArea's Virtualizer is built** (reka's `ScrollAreaVirtualizer`, research §12). `ScrollArea::virtualizer`
   (setup) makes the viewport's rows a `gui::list::ListState`, and `ScrollArea::virtual_viewport` (render) lays them
   out on `gui::list` (ADR-0014), scrolled by the area's own bars, thumb drag, track press, wheel over a bar and
   Chrome's keys. The list's own thumb is off. reka's numbers are kept (`ScrollAreaVirtualizer.vue:53-91`): rows
   are estimated at `ROW_ESTIMATE` (28 px) until measured; `OVERSCAN` (12) rows' worth are built beyond each edge
   (12 × the estimate, in pixels, as `gui::list` prefetches by distance); the focused row is kept built with one
   neighbour on each side, so Tab and Shift+Tab from a row scrolled away reach the next and previous rows. Both
   are `VirtualizerOptions`. A row that takes focus while not wholly in view is brought into view; Chrome does
   this for reka's rows. The bars read the list's offset over its whole extent, and the keys work in `f64`, so a
   million rows scroll exactly. Better than reka: the list gives assistive technology the row count and each
   row's place itself. reka leaves `aria-setsize` and `aria-posinset` to the caller (`scroll-area.md:309-314`).
   Not built, for a merit reason: reka's `horizontal` mode, and its right-to-left scroll to a row (`:61-69`),
   which only that mode reads. `gui::list`'s extent tree and anchor run down one axis. A row of virtual columns
   would need a second list engine, and nothing in shadcn, the docs or the plan lays content across in that
   number. Tests `scroll_area_virtualizer_builds_the_rows_in_view_and_keeps_the_focused_ones` and
   `scroll_area_virtualizer_a11y`. gui gained `ListState::keep_around_focus(rows)` (0 by default, so other lists
   are unchanged).
2. **A splitter handle is named.** "Resize panel N" is the handle after panel N (`splitter::handle_name`), unless
   the caller passes a locale's words (`Splitter::set_name`). This completes research §4 item 13: reka's handle
   has no orientation, no disabled state and no name, and item 7 above fixed the first two. shadcn's
   `resizable_handle` gets the name with no change.
3. **A disabled splitter handle is tested.** `splitter_disabled_handle_keeps_focus_moves_nothing_and_f6_passes_it`:
   it stays in the Tab order and is announced disabled. Its keys (Enter included) and a drag move nothing. F6
   moves off it, and F6 and Shift+F6 from the other handles pass it. `splitter_a11y` now has a disabled handle.
4. **A lone carousel indicator controls the content**, as the steps do (`useCarousel.ts:114-124`). Test
   `carousel_lone_indicators_go_to_their_slide_and_control_the_content`, with its AT snapshot.
5. **Collapsible's module doc** now states item 11's decision (find in page is not built) and no longer names a
   hook that does not exist.

No `better.tsv` entry: the differences are behaviour and the AT tree. No shadcn call site changes. No pixel
changes, because the names and relations are AT-only and no shadcn page uses the virtualizer.

## Group 5: third review fixes (M10 S6b)

Decided 2026-11-22, after the group's third review. Tests: `crates/primitives/tests/select_lists.rs` and
`crates/shadcn/src/select.rs`.

1. **A select's separator is hidden from assistive technology** (`SelectSeparator.vue:14-19`, research §9 Select,
   "Ẩn khỏi AT"). `primitives::select::separator` makes it `aria-hidden` and lets the pointer through, and
   shadcn's `select_separator` goes through it. The previous round had given it the `separator` role, which
   neither reka nor this ADR asked for.
2. **A select's group is named by its label** (`SelectGroup.vue:24-31`, `SelectLabel.vue:20-26`). The free
   functions `primitives::select::group(d, label)` and `group_label(d, label)` replace `Select::group`. shadcn's
   `select_group` and `select_label` are now parts: the group makes the label's `NodeRef` in its setup and
   provides it, and a label inside the group takes it. A group with no label points at a node that is not in
   the tree, so the relation is not told and the group is unnamed. A label outside any group is plain text.
   Test `a_group_is_named_by_its_label_and_a_separator_is_not_read`.
3. **A closing select has no exit animation.** This is a recorded difference from research §9 Select "Đóng",
   where reka's popup stays until its `animate-out` (`fade-out-0 zoom-out-95`, `SelectContent.vue:48-60`) ends.
   The reason is on the merits. The list is kept mounted while closed so its options stay known (the trigger's
   label, typeahead on a closed trigger). That is reka's `forceMount` case, and the Presence rule above says a
   part kept mounted is hidden at once as it closes. The engine plays an exit only for a subtree that leaves
   the tree (`gui::motion`, ADR-0007), and the list never leaves: open and closed, it is the same node at the
   same path. A layer's content leaving plays its exit only when the place it leaves is then empty
   (`motion::put_back`). Trying it showed that keying the closed list differently, or keeping it in the
   deferred layer, does not free that place. Playing the exit needs the engine to paint a kept node's closing
   frames before it turns invisible. That is a `gui` change, outside this group, and it would serve every
   kept-mounted part, not only this one. Until then the list closes as it did before M10 (it was
   `display: none`). This is a motion-only difference from shadcn-vue, so it gets no `verdict/better.tsv` entry:
   the verdict compares pages at rest, and the difference is not better.
4. **A scroll button's step timer stops when the button goes.** reka clears its timer as the button unmounts
   (`SelectScrollButtonImpl.vue`). Ours checks, at each step, that the list can still scroll that way (the
   button for it shows), and stops when it cannot, so an open list at its end no longer wakes the UI thread
   every 50 ms. Test `a_scroll_button_stops_stepping_once_the_list_cannot_scroll_its_way`.
5. **The select's placement claims are tested.** `select::POPPER` places shadcn's popper-mode list
   (`POPPER.side_offset(4.0).positioning()`, the same numbers as before). `Select::arrow` draws reka's 10 × 5
   arrow on the edge facing the trigger, pointing at the trigger's centre (test
   `a_popper_select_draws_reka_arrow_on_the_edge_facing_the_trigger`). Item-aligned placement centres the chosen
   label on the trigger and lines up the start edges, mirrored in right-to-left (test
   `an_item_aligned_list_lines_the_chosen_label_up_on_the_value_by_reading_direction`). Short of room below, it
   moves up to keep five rows, 10 px from the window's edge (test
   `an_item_aligned_list_short_of_room_below_moves_up_to_keep_five_rows`).
6. **The combobox's opening highlight has one owner.** The primitive `Combobox` highlights the chosen option
   after the first layout tells it the options. A command list, once laid out, brings its highlighted item into
   view (`Command::list_setup`), so the chosen row of a long list shows as the list opens.
   `Command::highlight_and_reveal` and shadcn's watch that called it are removed.
7. **A combobox trigger controls one real list.** `Command::list_known_as` gives the command's list node the
   combobox's content `NodeRef`. The trigger (`Combobox::button`, which points at it only while open) and the
   search field then name the same node. shadcn no longer adds a second `aria-controls`.
8. **Correction to the group's item 7 (tags input):** a duplicate marks the input invalid in state only
   (`TagsInput::is_invalid`, reka's `data-invalid`). Nothing tells assistive technology, as in reka. Item 7's
   "(state and AT only)" overstated it.

Pixels: none of this changes a look. Group and label are transparent components, the separator's box is
unchanged, and the popper numbers are the same.

## Group 5: fourth review fixes (M10 S6b)

Decided 2026-11-22, after the group's fourth review. Tests: `crates/primitives/tests/select_lists.rs`.

1. **A command's keys are the input method's while it composes** (research §9, Listbox filter and the group's IME
   note). `Command::new` now takes the field the keys come from (the search field, or the combobox's), and
   `Command::key` takes nothing while that field has a preedit, as `Combobox::key` and the listbox filter
   already did. shadcn's `command` and `combobox` route their keys through `Command::key`, never through
   `Combobox::key`, so before this the arrows, Home, End and Enter typed mid-composition (Vietnamese Telex)
   moved the highlight or picked. Test `a_composing_input_method_keeps_the_command_keys`.
2. **A tags input adds no draft on blur into the list its field controls** (reka's `TagsInputInput.vue:35-41`,
   research §9 TagsInput "Pointer"; it completes the group's item 7). `TagsInput::list` marks the list (its
   suggestions) and `TagsInput::list_ref` is the node the field's `controls` names. When the field loses focus
   with `add_on_blur`, the decision waits for the layout that writes the frame's focus flags: focus back in the
   field, or within the list, adds nothing, so a choice made there wins; anywhere else adds the draft. Test
   `focus_moving_into_the_list_the_field_controls_adds_no_draft`.
3. **A removal selects the next usable tag, else the one before**, by the same availability the arrows, Home and
   End read, so a disabled tag is never selected by Backspace or Delete. Test
   `a_removal_selects_the_next_usable_tag`.
4. **An autocomplete's combobox value is its text** (`None` while empty), through a `Held` adapter over the text,
   so `Autocomplete::combobox().value()` and its form value say what the field holds instead of a value nobody
   wrote.
5. **The combobox's docs say what the page keys do:** open, they go to the first and last option, as every list
   of the foundation (`Keymap::LIST`); closed, they are the field's.

Pixels: none of this changes a look.
