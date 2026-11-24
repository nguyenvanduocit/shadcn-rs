# K6a: the `use_context` calls, classified (2026-11-24)

Plan row K6a (wave 1). Every `gui::use_context` call in the workspace was
found and classified. Round 1 review fixed the classes, the report's place
and the tests (below).

- (a) A meaningful default by design, or a part that works alone: left as it
  is.
- (b) A part that must have its parent: read with
  `primitives::context::use_root_at(part, root, at)`, the crate's one rule
  for it (reka's `inject` that throws). A part outside its root is a
  reported error naming the part, the root it needs and `at`, the place in
  the caller's code that made the part; the part then renders its look
  without behaviour. It does not panic: the workspace denies panics in
  library code.
- (c) Something else: left as it is.

## How the report works

- A public part constructor that can report is `#[track_caller]`.
  `Part::new` keeps `Location::caller()` (`PartProps::at`); a part that is
  not a `Part` (a struct made into an element, a bare component) keeps it in
  its props. The setup passes it to `use_root_at`
  (`part::part_root(props, part, root)` for a `Part`).
- Reports are kept once per (part, root, place), per thread. Two different
  orphans, or one part made at two places, are separate reports. The part
  and root names and the place are `&'static`, so the set of what was
  reported (and the list a test drains) has one entry per call site in the
  source: it is bounded by the program, not by what runs.
- `primitives::context::take_missing_roots()` drains what was reported since
  the last call; tests drain before they mount and assert the exact list.
- The three menus share their parts (`dropdown_menu.rs`); `MenuFamily`
  (dropdown, context, menubar) makes each public part name its own part and
  roots (`context_menu_radio_item` needs `context_menu` and
  `context_menu_radio_group`).

Gate: `crates/shadcn/tests/parts_need_roots.rs`, 74 tests: each part made
alone (or in its outer root without the inner one it also needs: radio item
in a menu, questionnaire choice/input/error in a questionnaire, navigation
trigger/content in a navigation menu) reports exactly the expected (part,
root) pairs, made at the test file. One test mounts different orphans and
two places of one part; one mounts one place three times (one report).

## Changed to (b)

| file | part -> root | what K6's typed parts replace |
| --- | --- | --- |
| `command.rs` | `command_input`, `command_list` -> `command` | the scope `command` hands |
| `sidebar.rs` | `sidebar`, `sidebar_trigger`, `sidebar_rail`, `sidebar_inset`, `sidebar_content`, `sidebar_group_label`, `sidebar_group_action`, `sidebar_menu_sub`, `sidebar_menu_action`, `sidebar_menu_badge` -> `sidebar_provider` | the scope `sidebar_provider` hands (`group_action`/`menu_sub` and `action`/`badge` now each name their own part) |
| `collapsible.rs`, `trace.rs` | `trace_collapsible_trigger`, `trace_collapsible_content` -> `collapsible` | the scope `collapsible` hands |
| `carousel.rs` | `carousel_content`, `_item`, `_previous`, `_next` -> `carousel` | the scope `carousel` hands |
| `questionnaire.rs` | `questionnaire_progress`, `_item`, `_choice`, `_input`, `_error`, `_previous/_skip/_next/_submit`, `_reset` -> `questionnaire`; `_choice`, `_input`, `_error` -> `questionnaire_item` | the scopes `questionnaire` and the item hand |
| `dropdown_menu.rs` (+ `context_menu.rs`, `menubar.rs`) | `*_content`, `*_sub_content`, `*_item`, `*_checkbox_item`, `*_radio_item`, `*_sub`, `*_sub_trigger` -> the menu root of its family; `*_radio_item` -> `*_radio_group` | the scope a menu root hands |
| `dropdown_menu.rs`, `context_menu.rs`, `menubar.rs` | `dropdown_menu_trigger` -> `dropdown_menu`; `context_menu_trigger` -> `context_menu`; `menubar_trigger` -> `menubar_menu` | the scope the root hands |
| `navigation_menu.rs` | `navigation_menu_item` -> `navigation_menu`; `_trigger`, `_content` -> `navigation_menu_item` | the scopes the root and the item hand |
| `select.rs` | `select_value`, `select_content`, `select_item` -> `select` | the scope `select` hands |
| `tags_input.rs` | `tags_input_item`, `_item_delete`, `_input` -> `tags_input` | the scope `tags_input` hands |
| `resizable.rs` | `resizable_panel`, `resizable_handle` -> `resizable_panel_group` | the scope the group hands |
| `combobox.rs` | `combobox_trigger`, `_list`, `_input`, `_items`, `_item_indicator` -> `combobox` | the scope `combobox` hands |
| `collapsible.rs` | `collapsible_trigger`, `collapsible_trigger_as`, `collapsible_content` -> `collapsible` | the scope `collapsible` hands |
| `avatar.rs` | `avatar_image`, `avatar_fallback` -> `avatar` | the scope `avatar` hands |
| `toggle_group.rs` | `toggle_group_item` -> `toggle_group` | the scope `toggle_group` hands |
| `message_scroller.rs` | `message_scroller_viewport`, `message_scroller_button` -> `message_scroller_provider` | the scope the provider hands |

Outside its root, a (b) part still renders its look: `avatar_image` shows a
decoded picture, `avatar_fallback` shows after its delay and
`trace_collapsible_content` is shown, as the older `collapsible_content`
does. That is the "look without behaviour" the report promises, not a valid
use: the report is there.

Count: 52 calls in all; 30 of them were changed to (b) in round 0, and
`sidebar_menu_button` (below) came back out.

## (a) Default by design, or works alone, unchanged

| file | reason |
| --- | --- |
| `sidebar.rs` (`sidebar_menu_button`) | the collapsed-state styling is optional: `tests/callbacks.rs` mounts a menu button alone and it works. Round 1 moved it back from (b) |
| `design/handle.rs`, `theme/handle.rs`, `code_block/highlighter.rs`, `tooltip` (primitives) | an ambient value with a default |
| `select.rs` (`SelectPosition`, `GroupLabel`) | a default; a label outside a group is a plain label |
| `dropdown_menu.rs` (`ContextMenu` in content) | only a context menu's content has one |
| `menubar.rs` (`menubar_menu`'s `Menubar`) | listed in the plan |
| `navigation_menu.rs` (`Panels`, link's `Panel` and `NavigationMenu`) | a viewport is optional; a link has three valid places |
| `sidebar.rs` (`item_interaction`) | a button outside an item has none |
| `primitives/direction.rs`, `primitives/drawer.rs`, `primitives/splitter.rs` | a default; nesting is optional |

Re-check of the (b) list against what runs alone: the existing tests'
stderr reports only `sidebar_menu_button` (now (a)) and the two alert dialog
buttons, which this change does not touch; `docs --shots` of every page
reports no (b) part outside its root.

## (c) Something else, unchanged

`primitives/context.rs` (`use_root_at` itself), `message_scroller.rs`
(`use_message_scroller`, a public hook returning `Option`),
`examples/docs/main.rs`, `examples/docs/pages/sidebar.rs`,
`gui/src/engine.rs` and `gui/src/component/tests.rs` (tests and the docs app).

## Left for K6

- A menu part of another family inside a menu (a `context_menu_content`
  inside a `dropdown_menu`) passes the lookup, because the families share
  one `Menu` context. Only K6's typed parts (a part made from the scope its
  parent hands it) can refuse it.

## Open

- A missing root is a report, not a panic. K6's typed parts remove the case.
- `primitives::context::use_root(part, root)` (the place is a library line,
  one report per part) is still read by parts K6a did not change: `tabs`,
  `dialog` and `sheet`, `drawer`, `popover`, `tooltip`, `hover_card`,
  `accordion`, `stepper`, `pagination`, `calendar`, `range_calendar`,
  `radio_group`, `input_otp`, `pin_input`, `number_field`, and the
  `alert_dialog` content. Only the alert dialog
  action and cancel keep their place. K6 replaces all of them.
- `dialog_content` and `sheet_content` share `modal_content`, which names
  `dialog_content`.
- Revert proofs (the part back to a bare `use_context`, tests fail, then
  restored): `toggle_group_item` (`toggle_group.rs`, 1 test fails),
  `select_content` (`select.rs`, 1 fails), the shared `menu_level`
  (`dropdown_menu.rs`, 21 fail across dropdown, context and menubar). The
  other places had no report before, so their tests cannot pass without it.
- `cargo clippy -p shadcn -p primitives --all-targets -- -D warnings` stops
  on `crates/gui/src/window.rs:1038` (another agent's `as` cast) and
  `crates/shadcn/tests/caret_blink.rs` (another agent's new test); with
  `--no-deps` and the test targets of this crate, it is clean.
