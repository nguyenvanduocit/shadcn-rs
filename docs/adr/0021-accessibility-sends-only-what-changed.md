# 0021. Accessibility sends only what changed

Status: accepted, 2026-11-21; built 2026-11-21 (M7, see "As built"); the macOS accessibility tree is checked from outside the process since 2026-11-22. Open: the upstream adapter work, and what only a person listening can check (see "Checked from outside the process"). Supersedes ADR-0009's "a frame is an AccessKit tree" (`access::tree`
rebuilt whole every frame while assistive technology listens); keeps its semantics, ids from node paths, actions as
input and IME handling.

## Context

ADR-0009 builds the whole AccessKit tree each frame while a screen reader listens (`access::tree`, sent through
`update_if_active` in `window.rs`). That cost is linear in the frame's nodes. ADR-0009 names the triggers that now
apply:

- A screen reader user cannot move by word or line inside an input, or hear its caret.
- A list larger than the screen cannot be explored.
- Rebuilding grows with the page.

`gui` now targets lists of millions of rows (ADR-0013's collections, `research_notes/Hiệu năng cực hạn/million_lists.md`),
and a retained tree whose nodes know which phase changed (ADR-0013).

AccessKit's contract for a `TreeUpdate` (`references/accesskit`):

- Only new or changed nodes should be sent.
- A node sent replaces the old one whole, so every field must be set.
- A parent whose children change is sent with its full child list. Removing a child from it removes the child's
  whole subtree.
- Focus is in every update.
- A sender and receiver out of step panic.

Masonry follows this with per-widget `needs_accessibility`/`request_accessibility` flags and skips clean subtrees
(`references/xilem/masonry_core/src/passes/accessibility.rs:32-69`). Servo sends only changed nodes after the first
update.

## Options

1. **Keep rebuilding the whole tree.** Simple, but it costs the whole frame, and it cannot expose a million-row list.
2. **Diff the rebuilt tree against the last one** and send the differences. The update is right, but every frame
   still builds the whole tree.
3. **An a11y phase per node.** A node's semantics are a phase of the retained node, like layout and paint
   (ADR-0013). A write that dirties it marks the node; the update is the dirty nodes plus parents whose child lists
   changed.

## Decision

Option 3.

**The update.**

- The previous frame's nodes are kept.
- A signal read while computing a node's role, name, state or children dirties that node's a11y phase.
- The post-flush sends the dirty nodes, complete, plus any parent whose child list changed.
- A screen reader connecting gets a full tree once, and the kept state resets.
- Ids stay hashes of node paths. Debug builds assert there are no collisions.

**Text fields expose `TextRun` children.**

- Each line of a field is a `Role::TextRun` with `character_lengths`, `word_starts`, `character_positions` and the
  selection.
- Run ids are `hash(parent, line index)`, so an unchanged run is not resent.
- `ActionRequest::SetTextSelection` comes back as input.
- Parley's `PlainEditor` produces these runs, as Masonry's `TextArea` uses it (`references/xilem/masonry/src/widgets/text_area.rs:1064`).

**Virtualized lists.**

- The tree holds the rows on screen plus pinned rows: focused, active, the screen reader's position, and IME.
- Each row carries its index in the whole list, and the list its row count.
- Today only `size_of_set` and `position_in_set` reach the platforms. No adapter reads `row_count` or `row_index`
  (`references/accesskit/accesskit/src/lib.rs:2260, :2273`), and Windows has no grid, table or virtualized item. **We contribute
  that support to AccessKit's adapters** as part of this decision, not after it.
- Scroll actions (`ScrollIntoView`, scroll by page) from the screen reader move the list's anchor and build the rows
  that come into view.

**IME.** `set_ime_cursor_area` is set every frame the caret or composition moves, not only on focus. It uses the
editor's composition area, as Masonry calls it after layout.

## Consequences

Expected; this is not built yet.

- **An a11y update costs what changed.** A hover that changes no semantics sends nothing; typing sends one input and
  its runs.
- **VoiceOver, NVDA and Orca can move by character, word and line in inputs and follow the caret.** This is verified
  by a person at the keyboard on each platform, and headless by snapshots of the a11y dump that devtools prints.
- **A million-row list announces its true size and position** once the upstream adapter work lands. Until it ships,
  screen readers see set size and position only.
- **Tests:**
  - incremental updates applied to a mirror tree equal a full rebuild at every scenario step;
  - removing a subtree leaves no orphan;
  - focus is present in every update.

## Revisit when

- An adapter or screen reader mishandles partial updates of a node class: send that class whole.
- Live regions are needed (streamed replies, toasts): an AccessKit live setting on the receiving node.
- A link inside a paragraph must be reachable alone: a `Link` child per span.
- AccessKit's own virtualization support lands upstream in a different shape than the one we contributed.

## As built (2026-11-21)

Built in `crates/gui/src/access.rs` (`Accessibility`, tests in `access/tests.rs`), `text/access.rs` (`TextSystem::text_runs`),
`list/virtual_list.rs`, `interact.rs` (actions), `scroll.rs` and `layout/mod.rs` (focus inside a scroll child), with
`window.rs`/`app.rs` holding the state. Differences from the decision:

- **Where a node's a11y phase is dirtied.** `Accessibility` keeps, per node id, the inputs its AccessKit node was
  built from (role, name, value, states, actions, box, a field's text, caret and area, row index and set) and its
  child list, and, per frame node, what it last read (shown or not, the space and list its children are read in, its
  focus) and the frame's child lists. Since ADR-0013's C4 the frame is written in place with a change log: an update
  after one that read the same frame structure reads only the logged nodes, their subtrees where what their children
  are read under changed, and the tabs above them (a tab is named by the text inside it), and sends those whose
  inputs differ plus the parents whose list of shown children differs. A frame rebuilt (nodes came, went or moved)
  is read whole, still sending only what differs. `AccessStats::visited` counts the frame nodes read;
  `GUI_COLD_CHECK=1` compares the kept tree with one read whole each update.
- **Scrolling sends one node.** Not in the decision: a scroll container carries AccessKit's `transform` to its
  content and the boxes inside it are relative to that content (quantized to 1/64 px), as Chromium keeps scroll
  offsets apart from the boxes under them. A wheel step sends the container alone.
- **Reconnecting.** `InitialTreeRequested` and `AccessibilityDeactivated` reset the kept state (`WindowHandler::
  access_reset`); the next update is the whole tree with `TreeInfo`. A deactivation followed by a reactivation
  before the deactivation event is handled could still meet a placeholder adapter with a partial update; the
  proxy-based adapter gives no way to close that window.
- **Ids** hash node paths with foldhash at a fixed seed (faster than SipHash, the same in every run); a text run's id
  hashes its field's id and its index. Debug builds assert that no two nodes of a frame, and no run and node, share one.
- **Text runs** come from the pieces Parley laid out for painting (not `PlainEditor`, which ADR-0019's as-built
  dropped): one `TextRun` per line, split where the direction changes and at 256 characters (`word_starts` are `u8`),
  in logical order, characters being grapheme clusters; the hard line break ends its line's last run; an empty
  field has one empty run for the caret. Lengths, positions, widths and word starts follow Parley's editor example
  (`references/parley/examples/editor/src/accessibility.rs`). Runs are rebuilt only when the field's inputs change
  and only runs that differ are sent. `SetTextSelection` maps run positions back to bytes of what the field shows,
  then to its value (password masking, composition).
- **Rows.** `virtual_list` puts the row index on a row element that names a row's role (`Row`, `ListItem`,
  `ListBoxOption`, …), leaves one that names another role alone (a heading among options) and wraps any other in a
  row whose role the list's implies (`List` → `ListItem`, `Table`/`Grid`/`RowGroup` → `Row`, `ListBox` →
  `ListBoxOption`). Each row gets `row_index`, `position_in_set` and `size_of_set` (the list's row count); the list
  `row_count`. `Semantics::set` (`aria_pos_in_set`) overrides the place for an option among the options of a list
  that holds headings (the command list). shadcn's table is a `Table` of `RowGroup`s, `Row`s, `ColumnHeader`s and
  `Cell`s; its virtual body a `RowGroup` with the row count.
- **Pinned rows.** The row holding focus (found by layout: `ScrollChild::focus_within`) and the row its owner names
  active (`ListState::set_active`; the command list's highlighted option) stay built when they leave the window,
  absolutely placed above the viewport where nothing paints or hits them, in reading order before or after the
  window. An IME composition is in the focused row, so it is pinned with it. AccessKit reports no screen reader
  cursor, so "the screen reader's position" is not pinned apart from focus.
- **ScrollIntoView** is an action on every node inside a scroll container: a virtual list moves to the row
  (`ScrollHandle::on_reveal`), other containers scroll by as little as shows the node.
- **Not built:** `active_descendant` from the command input to its highlighted option (it needs the option's id at
  render); the contribution to AccessKit's adapters below.

Upstream gaps found (plan M8 U1; not opened as issues or PRs): no adapter reads `row_count`/`row_index`
(`references/accesskit/accesskit/src/lib.rs:2260, :2273`); Windows has no grid, table or virtualized item pattern;
`accesskit_winit`'s proxy adapter cannot answer `InitialTreeRequested` synchronously, which leaves the reactivation
race above.

Measured (release, `stress --a11y`, Apple Silicon dev machine under shared load; "adapter" is the update applied to
`accesskit_consumer`'s tree, what a platform adapter does first):

| Scene | Before: whole tree every frame | Steady frame now | Script step now |
| --- | --- | --- | --- |
| `grid` (2 500 buttons) | 5 008 nodes, built 0.86–1.58 ms, adapter 1.22 ms | 0 nodes, 0.61–0.97 ms, adapter 0 | 2–3 nodes (hover, click), adapter 0.01 ms |
| `bigtable` (1 M rows) | 530 nodes, 0.09–0.32 ms, adapter 0.41 ms | 0 nodes, 0.04–0.06 ms | p50 1–103, max 782 nodes (a jump rebuilds the window) |
| `dashboard` | 55 nodes, 0.02 ms | 0 nodes, 0.01 ms | p50 16, max 615 (opening a page) |

Building an update on `grid` then still cost about as much as building the whole tree, because the comparison
walked every frame node. Since C4 (ADR-0013) it reads only the nodes the frame logged: a steady `grid` frame builds
in 0.00 ms (from 0.61–0.97 ms) and a script step (hover, click) in 0.01 ms at p50, 2–3 nodes sent; a label bound
to a signal on one of 200 rows reads 1 node and sends 1 (`accessibility_sends_and_reads_only_the_node_that_changed`). Tests: an update holds
exactly the changed nodes and parents (`an_update_holds_…`), incremental updates applied to `accesskit_consumer`
equal the whole tree with focus at every step (`incremental_updates_keep_…`), a scroll sends one node, text runs and
`SetTextSelection`, list rows and pinning.

**Checked from outside the process (macOS, 2026-11-22).** `crates/gui/tools/ax_check/run.sh` builds the docs and
stress examples, launches each in a window, and runs `ax_check.swift` (`swift`, the AXUIElement API) against the
app's process. It reads what VoiceOver reads, quits the app, and exits 0 when every check passed, 1 when one
failed, 3 without accessibility permission for the terminal (System Settings > Privacy & Security >
Accessibility). `AX_CHECK_BUILD=0` reuses the binaries built. Each check prints PASS, FAIL, or GAP. A GAP is
something `accesskit_macos` 0.27 has no code to expose, and it does not fail the run. "say:" lines are what VoiceOver
would announce, derived from role description, title, value and selection. Run here: 71 PASS, 0 FAIL, 11 GAP, exit
0, both apps quit.

- **Text field** (docs Input page) and **text area** (Textarea page): AXValue, AXNumberOfCharacters,
  AXSelectedText, AXSelectedTextRange, AXInsertionPointLineNumber. The parameterized attributes are exactly those the
  adapter gives a node with text runs: AXStringForRange, AXAttributedStringForRange, AXRangeForLine, AXLineForIndex,
  AXRangeForIndex, AXRangeForPosition, AXBoundsForRange, AXStyleRangeForIndex, AXReplaceRangeWithText.
  - Setting the value reads back: `"hello brave world"`, 17 characters.
  - The caret set at (2,0), moved to (3,0), and the selection (0,5) read back; AXSelectedText is "hello".
  - Each of the three words reads back by range. Line 0 is (0,17).
  - In the text area, the second line is line 1: AXRangeForLine(1) = (16,17), AXLineForIndex(18) = 1.
  - Announced: "Nguyễn Văn A, text field"; "hello, selected"; VO+L "hello brave world"; in the text area "first line
    here", then "second line there".
- **Focus moves:** setting AXFocused moves the app's AXFocusedUIElement to the field and to a second field, and a
  Tab posted to the process moves it on ("Preview, 1, tab").
- **Lists and tables:** the command list is an AXList with AXRows (6), and the docs table and the stress `bigtable`
  (1 000 000 rows, 34 built) are AXTable with AXRow children. GAPs: no AXRowCount, no AXIndex, no AXARIAPosInSet or
  AXARIASetSize. `accesskit_macos` reads none of `row_count`, `row_index`, `position_in_set` and `size_of_set`, so
  VoiceOver cannot say "row N of M", and a million-row table does not announce its size on macOS. This is the
  upstream gap above (plan M8 U1). AXRows is given only to containers with selectable children, so a table has none.
- **Found:** a command option is AXStaticText with an empty title, named only by the text inside it, and the
  Input demo's fields have no accessible name (announced by their value, or as "text field" alone). Both are the
  page's semantics, not the adapter's.
- **What only a person listening can check:** VoiceOver's actual speech and verbosity, including whether it reads
  the text inside a nameless option or row; VO+arrow moving by character, word and line as a user does it, and the
  rotor; typing through the keyboard and an input method while VoiceOver follows the caret; focus-change
  announcements (notifications, which attributes do not show); NVDA on Windows and Orca on Linux.
