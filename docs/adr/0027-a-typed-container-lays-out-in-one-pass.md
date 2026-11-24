# 0027. A typed container lays out in one pass

Status: accepted under delegation, 2026-11-24 (revised the same day after three review rounds); not yet built (M12,
L2–L3). Amends ADR-0014 (the run-time choice of algorithm, and "Taffy breaks its API every minor release; we follow
it") and ADR-0026 (a small amendment: the control functions as methods of `Lay`, and a component's root is a box).
Settles what ADR-0026 left to it: the layout parameters of rows under a boxless `each`.

Who decided what:

- **2026-11-23, the user:** layout is typed, as Compose and Flutter have it (`docs/plan-2026-11-21.md`, M12;
  `docs/review-2026-11-23.md` §12.2). Port the value, not the CSS mechanism; Chrome is the reference, not the ceiling.
- **2026-11-24, the user:** approval of the details is delegated to the orchestrator.
- **2026-11-24, review round 1** (Codex and Claude, both failed it; both kept the zero minimum of `fill`, the engine
  of our own and the fixtures). The orchestrator's decisions on their findings are written in below: the `fit` slot,
  fills in an `AtMost` row, the hugging column of stretched rows, the pass claims of grid, stack and wrap, children
  outside slots, which mistakes are compile errors, the control functions, the CSS bridge, L3's expectations and the
  two engine bugs.
- **2026-11-24, review round 2** (both failed it again). Decided by the orchestrator and written in below: the pass
  bound (two kept measures and one layout per node), the shrink loop of `fit`, the `AtMost` row without a jump, a
  child's own size in px as its content, L3's format and mapping, the narrower bit-equality domain, item parameters
  out of `Style`, a key's saved slots in a cold rebuild, and what a `spacer` is. Two answers to round 1 stand as
  written and are recorded here as accepted: the aspect-ratio rule ("Aspect ratio"), and the count of `fit` sites,
  which is left to S8 because no search can make it.
- **2026-11-24, review round 3, the last** (Codex failed it with four findings, Claude passed it with three gaps;
  both recomputed the shrink loop and the `.typed` boxes and found them right). Decided by the orchestrator and
  written in below: the container as the unit that falls back to CSS, a fill's share as its outer size with a loop
  on the growing side, minimums and `fixed` under `AtMost` and `Unbounded`, the pass bound checked per node, and a
  column's item with no flex as `hug`. No review follows; the orchestrator verifies.

## Context

**The engine picks the algorithm while it runs.** A flex node tries `row::compute`, then `stack::compute`, then
`flex::compute` (`engine/mod.rs`, `run`); each fast path leaves with `return None` when the node is not its case (3 in
`row.rs`, 7 in `stack.rs`, 10 in `grid.rs`). `row.rs` is Taffy's flexbox for one line, kept bit-identical, so it still
probes: a flex base size, and a min-content size for every item whose minimum is automatic (`row.rs`,
`flex_base_size`). Whether a node is a relayout boundary is found from what the algorithms asked
(`Node::is_boundary`), never known beforehand.

**What the components use** (`crates/shadcn/src` and `crates/primitives/src`, counted 2026-11-24):

| Thing | Uses |
| --- | --- |
| `Display::Flex` / `Display::Grid` | 212 / 17 |
| `flex_wrap` | 5 |
| Baseline alignment | 0 |
| `flex_shrink(0…)` / another `flex_shrink` | 41 / 1 (`splitter.rs`) |
| `Flex::grow`, `flex_grow` | 24 |
| `min_width(0…)` | 55 |
| `Position::Absolute` | 44: 42 placed against their container's box by edge offsets or `inset` (indicators, thumbs, rails, close buttons, badges, arrows, a sheet's or dialog's content; 5 of them in `chart.rs` with offsets from data), 1 hidden text (`visually_hidden.rs`), 1 content matching its box (`aspect_ratio.rs`). Sorted by node name, not by reading each site |
| An `auto` margin | 13: `margin_left` 8 and `margin_start` 1 (push to the end of a row), `margin_top` 2 (push to the end of a column: `drawer.rs`, `sheet.rs`), `margin_x` 2 (centre: `drawer.rs`, `pagination.rs`) |
| `RowReverse`, `ColumnReverse` | 2 (`message.rs`, `calendar.rs`) |
| `aspect_ratio` | 12 |
| `pct(…)`, of which `pct(100…)` | 89, 81 |
| Explicit grid placement or span | 19 |
| `grid_auto_rows` | 1 (`card.rs`, `card_header`) |

Nearly everything is a row or a column that does not wrap. `min_width(0)` is written 55 times to undo one CSS rule.

**The fixtures (L1).** 23 fixtures, 534 boxes, in `crates/gui/tests/chrome_one_pass/`, laid out by Chrome 145
(`node crates/shadcn/verdict/one-pass.mjs`). Run through today's engine with `chrome_layout.rs`'s reader, 21 match
Chrome and 2 do not ("Bugs L2 fixes"). What Chrome does with the automatic minimum:

| Fixture | Automatic minimum | `min-width: 0` |
| --- | --- | --- |
| `min_auto_text_nowrap`: a one-line label that fills a 160 px row, then a badge | label 202.11 px; the badge starts at 210.11, outside the row | label 118.08 px; the badge ends at 160 |
| `min_auto_nested`: fill > row > row > label, 100 px left | 180.14 px at every level. Zero has to be said at all three levels before the label is 100 px | |
| `min_auto_weights`: three fills of weight 1 in 300 px, one holding 160 px | 160 / 70 / 70 | 100 / 100 / 100 |
| `min_auto_aspect_ratio`: a box 50 px high with ratio 2, 40 px left | 100 px (the ratio feeds the minimum) | 40 px |
| `min_auto_replaced`: an image 100 x 50, 40 px left | 100 px; 80 px with `height: 40px` or `width: 80px` (the smaller of content and transferred or specified size) | 40 x 20 |
| `min_auto_text_wrap`, `min_auto_text_nowrap`, with room | the same boxes | the same boxes |
| `fill_in_hug_row`: two one-line fills in a row that hugs | 62.52 / 170.56, their own widths | 116.53 / 116.55, the second label overflows its box |

So the automatic minimum changes nothing when there is room, and when there is none it pushes siblings out of the
row, breaks the weights, and has to be undone at every level of nesting.

**What it costs today.** L0 (`docs/l0-measure-counts-2026-11-24.md`) counted, on cold trees, what a parent asks a
child, by kind:

| Tree | Asked: min-content, max-content, placement | Of those, an algorithm ran |
| --- | --- | --- |
| A flex row of 8 text leaves | 8, 8, 8 | 8, 8, 9 |
| 4 rows of 4 growing cells, nested | 212, 164, 36 | 52, 84, 37 |
| A scroll box of 12 flex rows | 132, 24, 48 | 96, 0, 49 |
| 3 `fr` columns, 6 cards | 54, 30, 18 | 18, 6, 19 |

A placement is the layout a typed container keeps; every other column is a probe. L1's own count on three small trees
(`LayoutTree::stats`, calls / misses / leaf measures) shows what the automatic minimum adds: a paragraph under three
nested fills is 34 / 18 / 2 with it and 12 / 12 / 1 with `min-width: 0`; a column of 10 menu rows is 182 / 112 / 50
against 162 / 102 / 40.

**The engine is ours already.** Flex, block and the leaf are ported in full in `engine/`: no call to
`taffy::compute_flexbox_layout` or `compute_block_layout` is left in `crates/gui/src`. Taffy still runs the root, hidden
nodes, the cache wrapper, out-of-flow boxes and the general grid (`grid.rs`, `fallback`), and gives the vocabulary
(`LayoutInput`, the style traits). AGENTS.md still says "a retained node tree that implements Taffy's low-level traits
directly" (review §5, row 4).

## Options

For a fill's minimum:

1. **`fill` keeps CSS's automatic minimum, and a leaf returns its min-content with its measure.** Pixels stay Chrome's
   everywhere. But the minimum of a fill is the min-content of its whole subtree, which must be known before the fill
   is given its size: an intrinsic pass through every container below it. The three faults in the table stay.
2. **`fill` has a minimum of zero.** Compose's `weight` and Flutter's `Expanded`, which give a child its share and
   nothing else. Pixels differ from Chrome only where upstream wrote `flex-1` without `min-w-0` and the content is
   wider than its share.

For the engine:

1. **Go back to Taffy's algorithms** and keep only the tree. The cache keyed by what was read, the per-item memo
   (`stack.rs`), the parallel batch and the `Send` style cannot be had through its API (ADR-0014, "As built").
2. **Own the engine, keep following Taffy** bit for bit. Typed containers cannot be bit-identical to an algorithm that
   probes.
3. **Own the engine.** Chrome is the oracle. Taffy is a pinned dependency for what is not ported.

## Decision

### The containers

| Container | What it is |
| --- | --- |
| `row()`, `column()` | Children along one axis, each in a slot: `fixed`, `hug`, `fit` or `fill` |
| `stack()` | Children over one another in one box |
| `grid([Track; N])` | N columns, tracks in px or `fr`, children placed in order, one cell each, rows as high as their content |
| `wrap()` | Hugging children broken into lines, and at most one child that takes the rest of its line above a minimum in px (`tags_input`) |
| `flex()`, `css_grid()` | CSS flexbox and CSS grid, whole, as `flex.rs` and Taffy's grid lay them out today |
| Block and inline | Paragraphs, and boxes in a paragraph's lines (`block.rs`, `inline.rs`) |

- The caller names the semantics. There is no `Display` to set and no run-time choice: L2 deletes the chain
  `row → stack → flex` and the `return None`s.
- `engine/stack.rs` is a column today (stretched items, memoised per item). It becomes `column()`'s algorithm, and
  `row.rs` becomes `row()`'s. **The `stack()` of this ADR does not exist yet: L2 builds it.**
- A typed container takes no `aspect_ratio`, no percentage and no baseline alignment of its own.
- A builder closure receives `r: &mut Lay<Row>` (`Lay<Column>`, `Lay<Stack>`, `Lay<Grid<N>>`, `Lay<Wrap>`). It is
  called `Lay`, not "scope", because a scope is what owns reactive state (ADR-0026).
- `row().reverse()` and `column().reverse()` place the children from the other end (2 uses). Right-to-left comes from
  the direction the node inherits (ADR-0004), as `row.rs` reads it today; nothing is said per container.

### The protocol

Constraints go down and a size comes up.

- **A limit per axis:** `Exact(px)` (the parent fixed it), `AtMost(px)` (as large as the content and no larger than
  this; widths only, because only text wraps) or `Unbounded` (as large as the content).
- **Two questions.** A *layout* gives the child its limits, places what is inside it and returns its size. A
  *measure* asks only for its size under some limits and places nothing. A measure is kept in the node's cache
  (ADR-0014's `CacheTree`, `cache.rs`) per limits, and dropped by the layout marks of its subtree, so one node answers
  one measure once however many ancestors ask.
- **A measure is of the width alone, or of both sizes.** The width alone (`RequestedAxis::Horizontal`, which the
  cache key already holds as `axis`) answers "how wide is your content": a hugging column's widest row, the content
  of a `fit` or `fill` in a row, the `AtMost` test. Both sizes are needed where a height is the question: the line of
  a row whose height is its content's, and the content of a `fit` or `fill` in a column (its height at the width it
  has).
- A child's position is its parent's (ADR-0014).

**Slots along the main axis of a row** (a column is the transpose):

| Slot | In a row whose width is `Exact` | As CSS | Cost per child |
| --- | --- | --- | --- |
| `fixed(px)` | `Exact(px)` | `flex: none; width: px` | 1 layout |
| `hug` | `Unbounded`: its content's size, never squeezed | `flex: none` | 1 layout |
| `fit(weight)` | `Exact(content)` while the row has room. When the row overflows, the fits shrink by the loop below | `flex: 0 weight auto; min-width: 0` (CSS's default item, with no automatic minimum; Flutter's loose `Flexible`) | 1 width measure, 1 layout |
| `fill(weight)` | `Exact(share)`: what the others and the gaps leave, split by weight; the share is the slot's outer size; zero when nothing is left | `flex: weight 1 0%; min-width: 0`, in the domain "L3" names (weights that sum to 1 or more, no padding, border, minimum or maximum on a fill) | 1 layout |

- `fixed(px)` holds under every limit of the row, `Unbounded` included: a fixed slot is never asked for its content.
- A slot may carry `min(px)`, `max(px)` and its own cross alignment (`r.hug(child).align(…)`).
  - **A slot's size is always within its [min, max].** The content of a `hug`, a `fit` or a fill that hugs is raised
    to its `min(px)` and capped by its `max(px)` before anything is summed, compared or shrunk, under every limit of
    the row. A fill's share is clamped by the loop below.
  - **The floor** of a fit or a fill is its padding and border, or its `min(px)` where that is larger.
  - On a fill in an `Exact` row a minimum or maximum only moves numbers (`min_auto_padding`); nothing is measured
    for it. A slot sized by its content that carries one (a `hug`, or a `fit` or `fill` outside `Exact`) costs a
    width measure, as a `fit` does: its content is asked, clamped, and the child is laid out at `Exact` that.
- **A child's own main-axis size in px is its content size under `hug` and `fit`.** A button of `height(36.0)` in a
  column, a separator of `height(1.0)`: nothing is reported and nothing is measured. Under `fixed` and `fill` it
  conflicts with the slot ("What is a compile error").
- **`fill` shares by weight, not by CSS's growth factor, and a share is the outer size.** A lone `fill(0.5)` takes
  all that is left; CSS's `flex-grow: 0.5` takes half (`fill_weight_below_one`). Two fills of 1 are equal boxes
  whatever padding one of them has; CSS shares what the padding leaves and then adds it back (`min_auto_padding`
  row 5: 120 / 80). This is Compose's `weight` and Flutter's `Expanded`.
- **How fills grow.** A share below a fill's floor or above its `max(px)` is frozen there, and the others share what
  that leaves:

  ```text
  left     = the row's width − its padding and border − the gaps − the size of every fixed, hug and fit
             (no less than 0)
  unfrozen = every fill
  while unfrozen is not empty:
      total       = Σ weight over unfrozen
      share(slot) = left × weight / total           (0 when total is 0)
      size(slot)  = share, clamped to [floor, max]
      excess      = Σ (size − share) over unfrozen
      if excess == 0: every unfrozen slot takes its size; stop
      if excess > 0: each slot raised to its floor is frozen there
      else:          each slot cut to its max is frozen there
      frozen slots leave unfrozen; left = max(0, left − their sizes)
  ```

  | Fills | Row | Result | Chrome |
  | --- | --- | --- | --- |
  | 1 with 60 px of padding, and 1, beside `fixed(150.0)` | 200 px | shares 25 / 25; the first freezes at 60, nothing is left: 60 / 0, the row overflows by 10 | the same (`min_auto_padding` row 4) |
  | 0.5 with `max(60.0)`, and 0.5 | 200 px | shares 100 / 100; the first freezes at 60, the other takes the 140 left | 60 / 100 (`fill_weight_below_one` row 6) |
  | 1 with 40 px of padding, and 1 | 200 px | 100 / 100, nothing freezes | 120 / 80 (`min_auto_padding` row 5) |
  | 1 with `max(50.0)`, 1 and 2 | 200 px | 50 / 50 / 100, nothing freezes | the same (`min_auto_padding` row 3) |
- **How fits shrink.** A slot's content is its outer size (border box), within its [min, max].

  ```text
  deficit  = the slots' sizes + gaps + the row's padding and border − the row's width
  unfrozen = every fit                       (in an AtMost row that does not fit: every fit and every fill)
  while deficit > 0 and unfrozen is not empty:
      total = Σ weight × content over unfrozen
      if total == 0: stop                    (every weight left is zero: the row overflows)
      target(slot) = content − deficit × weight × content / total
      if no target is below its slot's floor: every unfrozen slot takes its target; stop
      each slot whose target is below its floor is frozen at its floor
      frozen slots leave unfrozen; deficit −= what they gave up
  ```

  Test cases. The first two are the reviewers' own inputs, and both reviewers recomputed them in round 3:

  | Fits (weight, content) | Row | Result |
  | --- | --- | --- |
  | (9, 100) and (1, 100) | 50 px | targets −35 and 85: the first freezes at 0, the 50 still missing come from the second: 0 / 50 |
  | (10, 10) and (1, 100) | 60 px (a deficit of 50) | targets −15 and 75: the first freezes at 0, the second gives the other 40: 0 / 60 |
  | (9, 10) and (1, 90) | 50 px | targets −15 and 65: the first freezes at 0, the 40 still missing come from the second: 0 / 50 |
  | (10, 100) and (1, 100) | 150 px | 54.55 / 95.45 |
  | the same, the first with `min(60.0)` | 150 px | the first freezes at 60, the second gives the other 10: 60 / 90 |
  | (0, 100) and (0, 100) | 150 px | nothing shrinks; the row overflows by 50 |
- **`hug` and `fit` children do not see one another.** Compose gives each what the earlier ones left
  (`references/androidx/…/foundation/layout/RowColumnMeasurePolicy.kt:132-137`); here each is measured `Unbounded`, so
  siblings stay independent and the pool can still ask alike children at once (`engine/mod.rs`, "Parallel"). A row
  whose fixed and hugging children alone are wider than it overflows at its end, as `flex-shrink: 0` does
  (`hug_shrink`).
- **A row whose width is `AtMost(v)`** (a tab list in a column that does not stretch it) first sums its slots' outer
  sizes: the px of each `fixed`, the content of each `hug`, `fit` and `fill` within its [min, max] (a fit of
  content 10 with `min(80.0)` counts as 80), the gaps, and its own padding and border. If the sum is no more than
  `v`, those are the widths and the row hugs. Otherwise the row is `v` wide,
  `fixed` and `hug` keep their size, and the fits **and the fills** shrink from their content by the loop above, a
  fill's weight being its shrink weight. Nothing is left over, and nothing jumps: three one-line fills of 20, 20 and
  161 px in 200 px lose 1 px between them (the long label 0.80 of it), where sharing 200 by weight would have cut it
  by 94. Under `Exact` a fill stays a share by weight. Cost: one width measure per fit and fill, then one layout
  each.
- **A row whose width is `Unbounded`** gives a `fixed(px)` its px and asks only the `hug`, `fit` and `fill` slots
  for their content, each within its [min, max]: a fill hugs, because there is nothing to share.
- **The cross axis.** A child aligned `Stretch` in a container whose cross size is `Exact` gets `Exact`. A child not
  stretched gets `AtMost(inner width)` in a column (CSS's fit-content: `stretch_cross`, `fill_in_hug_row`) and
  `Unbounded` in a row.
- **The container's own size** is the limit where it is `Exact`, else what its children took plus gaps, padding and
  border, and no more than an `AtMost`.

**Children outside slots.**

- `r.overlay(anchor, child)` is a child that takes no space (42 of the 44 `Position::Absolute` uses: a tab indicator,
  a slider thumb, a badge). It is laid out once, after the slots, against the container's final box, at the place
  `anchor` names (an edge or corner with offsets, or the whole box). It replaces `position` and `inset`. The other
  two are not overlays: hidden text keeps its primitive, and content that matches its box is `stack().matching`.
- `r.spacer()` is what an `auto` margin was, and it is not a fill:
  - it takes no gap: gaps lie between neighbours that are not spacers;
  - it gets only what is left after the fills have grown, so beside a fill it gets nothing, as an `auto` margin beside
    a growing item does; several spacers share that space equally;
  - in a row whose width is not `Exact` it is zero.

  The 9 margins that push to the end of a row (`margin_left`, `margin_start`) and the 2 that push to the end of a
  column (`margin_top`) are a spacer before the child. The 2 that centre (`margin_x`) are a spacer on each side, or
  the slot's cross alignment where the centred axis is the cross one.

### What is promised

**One layout per child, and no measure,** for a `row()` or `column()` when:

1. its main size is `Exact` and it has no `fit` slot, or its main size is `Unbounded`;
2. no slot sized by its content carries a `min(px)` or `max(px)` (a `hug`; a `fit` or `fill` outside `Exact`);
3. its cross size is `Exact`, or no child is stretched, or each stretched child's layout does not read its cross size
   (a text leaf, a sized leaf, a box with no children: what `MeasureReads` and `Node::heights_read` say today).

**Everywhere else the bound is: at most two kept measures, at distinct limits, and one layout, per node, at any
depth.** The cases that measure:

| Case | Measure | Why | Fixture |
| --- | --- | --- | --- |
| A child stretched across a line whose cross size is its content's, when the child's layout reads that size | Both sizes, at the child's main limit | A sibling supplies the final cross size | `stretch_again`; `stretch_cross`, first row |
| **A hugging column of stretched rows** | Width alone, `Unbounded` | The widest row supplies the width | Every menu, select and command list: `hug_column_stretched_rows` |
| A `fit` in a row; a `fit` or `fill` in an `AtMost` row; a slot sized by its content that carries a `min` or `max` | Width alone, `Unbounded` | The content sizes decide whether the row has room, and what is clamped | `hug_shrink`, `tab_list_at_most` |
| A `fit` in a column whose height is `Exact` | Both sizes, at its width | Its content is a height | `min_auto_column`, the column of items that clip |

- **An item of a column with no flex is `hug`, not `fit`** ("Slots for a port"), so it is not in the table: the rows
  of `rows_under_each`'s third list and the paragraphs of `stretch_cross`'s columns get one layout each.
- **Why two.** A node can sit in two cases at once: a row in a hugging column is asked its content width (the first
  measure) and, laid out at the column's width, asks its stretched child for a height at that child's share (the
  second, at other limits). No third question exists in the table, and a measure asked again at the same limits is
  a cache hit.
- **Why depth does not multiply.** The hugging column measures each row's content width once, takes the largest (no
  more than an `AtMost`), and lays each row out once at `Exact(width)`. A group inside it does the same, and its
  rows' measures are already kept, so a leaf under k such columns is still asked once, not k + 1 times.
- **A leaf's layout may reuse its measure** at the same limits (a paragraph measured at a width is laid out from the
  same lines). **A node with children always lays out separately:** a measure places nothing.
- **What `cache.rs` does today, and what L2 changes.** `NodeCache::get` answers a final layout only from the one
  `layout` entry, and only one asked for both axes (`k.axis == 3`); it answers a size probe only from `sizes` (16
  entries, replaced second-chance), so a kept layout does not answer a later measure at the same limits; a leaf that
  reads the width alone keeps what it measured at its last two widths (`measures`). L2 keeps this shape. A typed
  node's measures are `sizes` entries keyed by limits and `axis`; it needs two, the 16 stay for CSS nodes. A
  both-sizes entry answers a width-alone question at the same limits. The leaf's two-width memo is what lets its
  layout reuse its measure.

**`stack()`, `grid([Track; N])` and `wrap()` are not promised one pass** (the plan's M12 goal says the same):

- `stack()`: a child added with `s.child(align, child)` sizes the stack, which is as large as the largest of them on
  each axis that is not `Exact`. A child added with `s.matching(child)` is laid out afterwards at `Exact` the stack's
  size and does not size it. That is one layout per child. There is no stretched sizing child: stretching is
  `matching`.
- `grid([Track; N])` with an `Exact` width sizes its columns without asking any item (it is `grid.rs`'s `accept`
  case). Its rows are content-sized lines, so a cell stretched in its row follows the first row of the table above.
  With a width that is not `Exact`, it measures its cells first.
- `wrap()` lays each hugging child out once and then breaks lines; the child that takes the rest of its line is laid
  out at `Exact`. A child stretched in its line follows the table. Its height depends on its width.

**Aspect ratio.** A leaf with a ratio is allowed in a typed row or column only when exactly one of its axes is
definite before layout: a cross size that is `Exact`, or its own size in px, or a `fixed` or `fill` slot in an `Exact`
row with the cross axis not stretched. A ratio leaf stretched in a content-sized line makes each axis depend on the
other; that combination is a debug report and belongs in `flex()`. A container with a ratio is `flex()` or
`css_grid()` (`sizing.rs`).

**Counts (L2): the bound is checked per node.** A total cannot show it: one node asked five times hides behind four
asked once.

- **Per-node counters,** in test and debug builds only, kept beside the node's cache and cleared with it: the
  measures that ran, one count per distinct limits and kind (width alone, both sizes), and the layouts that ran.
- **The gate** lays a tree out cold and asserts for every node whose parent is a typed container: at most two
  measures, at distinct limits, each run once, and one layout.
- **The exemption is for a whole typed subtree under a CSS parent.** The CSS parent's min-content probe goes down
  through every node of that subtree ("The bridge"), so each node in it may answer one min-content question more.
  It is counted on its own, and the gate asserts one per node there and none anywhere else.
- **The totals stay** (`Stats::asked` and `Stats::ran`, by kind): calls, cache hits (asked less ran) and placements
  are reported, not gated. For the trees in L0's table a typed port asks no min-content at all.
- **What L0's counters must gain** (`engine/mod.rs`; L2 adds it before it uses them):
  1. the per-node counters above, and a walk that reads them (`LayoutTree`, test and debug only);
  2. one population for `asked` and `ran`. Today `ran` counts runs no parent asked for (the root's placement, a
     relayout boundary's own job) and `asked` does not, which is why L0's table has 8 placements asked and 9 run.
     Such a run is counted in both, as a placement, so `ran ≤ asked` holds per kind with no "+ 1";
  3. the min-content questions a CSS parent puts to a typed subtree, counted apart.

**Boundaries by type.** A child whose two limits are `Exact` is a relayout boundary: a `fixed` slot or a `fill` in an
`Exact` row, stretched in an `Exact` cross size, and every fixed-size leaf (`icon(16.0)`). A `spacer` is not one:
its size is what its row leaves, and it holds nothing to lay out. For a typed node
`Node::is_boundary` becomes a debug assertion that what the algorithm did is what the type said; for a CSS node it
stays the mechanism.

### `r.fill()` has a minimum of zero

Option 2. `fill` is "what is left, by weight", and that is the value being ported.

- Where the row has room, nothing changes: Chrome's boxes are the same with and without the automatic minimum
  (`min_auto_text_wrap` rows 3–4, `min_auto_text_nowrap` rows 4–5, `toolbar`, `rows_under_each`).
- Where it has none, a fill is its share. It never pushes a sibling out, the weights hold, and nesting needs no
  repeated `min_width(0)`. The 55 `min_width(0)` calls go away with the port.
- **A fill cuts its content only when every level down to the label is a fill, a fit, or clips.** A `hug` label
  inside a fill keeps its width and lies over the neighbours (`hug_label_in_fill` rows 1 and 4; the tab list of
  `rows_under_each` in Chrome). The port makes such a label a `fit` or a `fill`.
- **Slots for a port:**

  | Upstream | Slot |
  | --- | --- |
  | `flex-1`, with or without `min-w-0` | `fill(1.0)` |
  | `shrink-0`, `flex-none` | `hug`, or `fixed(px)` when it has a size |
  | No flex class, in a row (CSS's default item) | `fit(1.0)`; `hug` when it must never be cut |
  | No flex class, in a column | `hug`: CSS's automatic minimum there is the content's height, so the item never shrinks below it. `fit(1.0)` only when it clips (`overflow-hidden`, `min-h-0`) |
  | `ml-auto`, `mt-auto` | `spacer()` before it. Unlike an empty `fill(1.0)` it takes no gap and yields to every fill |
  | `mx-auto` | a `spacer()` on each side, or the slot's cross alignment |
  | `absolute` with offsets | `overlay` |
  | `self-*` | the slot's `.align(…)` |
  | `min-w-[…]`, `max-w-[…]` on the main axis | the slot's `.min(px)`, `.max(px)` |
  | `order-*` | source order; gui has no `order` and gets none |
  | `hidden` by a condition | `r.show` |
  | `w-full` in a column, `h-full` in a row | `Stretch` |

  How many sites need `fit` cannot be counted by a search: it is every child of a row with no flex setter whose row
  can overflow, and every child of a column that clips. Text meant to be cut is the visible part of it (17 uses of
  truncation or line clamping, 21 of `nowrap`).
  S8 counts per family.
- **The leaf that returns min-content with its measure (L0, `Measure::measure_pair`) is kept, for `flex()` and
  `css_grid()` only.** It is not part of the typed protocol, which asks for no min-content.

**Differences from Chrome, on purpose.** Each is recorded per box in the fixture's `.typed` file ("L3").

| Where | Chrome | Typed |
| --- | --- | --- |
| A fill with no room, upstream without `min-w-0` | keeps its content, pushes siblings out | its share |
| A hugging row whose one-line fills do not fit its parent (`tab_list_at_most`, 200 px) | the row is its min-content, 245.73 px, and overflows the column | the row is 200 px; each fill shrinks from its content: 50.88 / 59.68 / 89.44 |
| An item with no flex and a main size in px, in a row too narrow (`min_auto_box` row 4: `width: 80px`, 40 px left) | 80 px: its specified size is its automatic minimum | a `fit` of content 80: it shrinks to 40, its floor being its padding and border |
| A hugging row of `min-width: 0` fills that fit (`fill_in_hug_row` row 2, `tab_list_at_most` row 7) | equal shares of the total; a longer label overflows its box | each its content |
| A hugging column in a narrower parent (`hug_column_stretched_rows`, 120 px) | its min-content, 177.30 px | 120 px, labels cut |
| A label with no flex inside a fill (`rows_under_each`, `hug_label_in_fill` row 4) | keeps its width (automatic minimum) and paints over its neighbours | a `fit`: its trigger's width |
| Fill weights that sum to less than 1 (`fill_weight_below_one`: a lone `fill(0.5)` in 200 px) | 100 px, half of what is left | 200 px, all of it |
| A fill frozen at its `max`, when the weights left sum to less than 1 (`fill_weight_below_one` row 6: two of 0.5, one capped at 60) | 60 / 100 | 60 / 140 |
| Fills of which one has padding or a border (`min_auto_padding` row 5: two of 1 in 200 px, 40 px of padding in the first) | 120 / 80: what the padding leaves is shared, and the padding added back | 100 / 100: a share is the outer size |
| A shrinking `fit` with padding or a border | shrinks in proportion to its inner base size (the content box) | shrinks in proportion to its outer content |
| A column item with no flex whose own height in px is more than its content, in a column too short (`min_auto_column`, last column: three of 50 px holding 20 px, in 100 px) | 33.33 each: it shrinks towards its content | a `hug`: 50 each, the column overflows |
| A paragraph narrower than its widest word | the box overflows | gui breaks inside the word (`text/mod.rs`, `overflow-wrap: break-word`) |

Verdict demos have room, so B0 is expected to hold; verdict decides per family in S8.

### What is a compile error, and what is a report

**Does not compile:**

- a slot the container does not have: `r.fill` in a `stack`, a weight in a `grid`;
- a child added to a typed container without a slot (there is no `.child()` on one);
- an item parameter anywhere but in its container's `Lay`. The item fields leave `Style`: `flex_grow`,
  `flex_shrink`, `flex_basis` (and the `flex` shorthand), `align_self`, `justify_self`, and the grid placement
  (`grid_column`, `grid_row`, set by `grid_column_start`, `grid_row_start`, `grid_row_span` and their kin). They
  become item parameters the container owns, kept beside the child, so `style_mut`, `restyle` and
  `style_and_bindings` cannot reach them. Under `flex()` and `css_grid()` they are said through that container's
  `Lay`, with the same names. Under a typed container they are the slot and its `.align(…)`, `.min(px)`, `.max(px)`;
- `position` and `inset` on a child of a typed container: they are `overlay`;
- `order`: there is no such setter, on `Style` or on a slot. The order of the calls is the order of the children;
- `display`, `flex_direction` and `flex_wrap`, which no longer exist. A node that was `Display::None` by a condition
  is a `show`.

**A debug report, and the slot wins:**

- a main-axis `width` or `height`, in px or `pct`, on a child in a `fixed` or `fill` slot (under `hug` and `fit` a px
  size is the content; a cross-axis size is always the child's own);
- a ratio leaf in the combination above;
- a percentage on a child of a typed container whose size on that axis is not `Exact`.

The report names the setter's call. A setter that can conflict is `#[track_caller]` and stores its `Location` in the
op it records, at the call; the conflict is only known at mount, when the slot is, and the report is made there from
the stored location.

### What keeps CSS

| Behaviour | Where it lives | Why |
| --- | --- | --- |
| Wrapping beyond `wrap()`: items that grow from a basis per line, `align-content`, balanced lines | `flex()` | `wrap_fill` row 3; line breaking needs each item's hypothetical size first |
| `grid_auto_rows` (`card.rs`), explicit placement and spans (19 uses), content-sized tracks | `css_grid()` | `card_header_grid`: the action spans two rows the text sizes. Taffy's grid, through `grid::fallback` |
| `aspect_ratio` on a container, and on a leaf in the combination above | `flex()`, `css_grid()` | A ratio feeds the automatic minimum (`min_auto_aspect_ratio`, `min_auto_replaced`) |
| A percentage of a size that is not known | `flex()`, `css_grid()`, block | `pct(100)` (81 of 89 uses) becomes `Stretch` or `fill`. The rest keep CSS's rule: of an unknown size it is `auto` (`percent_indefinite`) |
| The automatic minimum itself | `flex()` | A site that needs it says so by its container |
| Baseline alignment | `flex()`, `css_grid()` | 0 uses today |
| Block and inline | `block.rs`, `inline.rs` | Text |

**The unit that falls back is the container.** A row or column in which any item needs CSS on its own account (a
grower with an automatic minimum, a basis that is neither zero nor its content, a percentage, a ratio outside
"Aspect ratio", an `auto` margin that a `spacer` does not say) is `flex()`, whole. Slots and CSS items never share a
line: a grower with an automatic minimum takes space from its neighbours by a rule a typed row does not have. A
child that is CSS only by its own display (a `css_grid()`, a `flex()` that wraps) is an ordinary slot of its typed
parent.

**The bridge between the two.**

- **A CSS container inside a typed one** is given one final layout, and before it whatever measure a typed child in
  its slot would be given: both sizes when it is stretched across a content-sized line and reads its cross size
  (`stretch_cross` row 1, the column that holds a grower), its width when it is a `fit` or sits in a hugging column
  or an `AtMost` row. The bound of two measures and one layout holds for the CSS container itself; what it asks
  below itself is CSS's and is not bounded here. `Exact` is a known dimension and `Unbounded` is max-content.
  `AtMost(v)` is `AvailableSpace::Definite(v)`, where CSS's answer is fit-content: the smaller of its
  max-content and `v`, and no less than its min-content. Today's engine does not clamp there (the first bug below);
  the bridge is built on the fixed arm.
- **A typed container inside a CSS one** answers the CSS parent's min-content and max-content probes through the same
  kept measure, dropped by the same layout marks. Max-content is its `Unbounded` measure. Min-content is computed
  from its children's: a row's is the sum (a `fixed` or `hug` child its size, a `fit` or `fill` its floor), a
  column's is the largest of its children's min-content. The recursion goes on down the tree; each node answers it
  once. These probes are counted at the boundary.

### gui owns its layout engine

Option 3.

- The algorithms in `engine/` are gui's. They started as Taffy 0.15.1's and are no longer required to equal it.
- **Chrome's fixtures are the oracle:** `crates/gui/tests/chrome_layout/` (54) and `chrome_one_pass/` (23). Where
  Chrome and Taffy differ, Chrome wins, as it already does for `aspect-ratio` (`leaf.rs`, `flex.rs`).
- **Taffy's 1 689 fixtures stay as a net under `flex()`** (`tests/taffy_fixtures.rs`): bit-identical to Taffy except
  where a listed Chrome fixture says otherwise (the two bugs below are the first). Typed containers are not held to
  them.
- **Taffy stays a dependency, pinned** (`taffy = "=0.15.1"` in `crates/gui/Cargo.toml`), for the vocabulary of the CSS
  engines and for the general grid. It is upgraded when we choose. ADR-0014's "we follow it" is withdrawn.
- The general grid is not ported now: 17 grid sites, most of which `grid([Track; N])` takes.
- AGENTS.md's layout paragraph is rewritten at L2 to say this.

### Control functions under a typed container

This reconciles ADR-0026's `show`, `each` and `dynamic_view` with slots, and amends ADR-0026 in two places.

- **The free functions stay** (`show`, `each`, `dynamic_view`, returning an element, as ADR-0026 has them). Under a
  CSS container their rows are direct children. Given to a slot, the whole range is one child: `r.hug(each(…))` puts
  the range in one box of the parent's kind and gap, its rows hugging.
- **Rows that take slots of their own use the method on `Lay`:**

  ```rust
  row(|r| {
      r.fixed(16.0, icon());
      r.each(items, key, |r, row| r.hug(trigger(row)));     // each row is a direct child of the row
      r.show(more, |r| r.fill(1.0, rest())).or_else(|r| r.spacer());
      r.dynamic_view(which, |r, k| r.fit(1.0, pane(k)));
  })
  ```

  The view takes `&mut` the same `Lay` its container gave, so a row can only be given a slot its real parent has.
  `Lay::show` returns a handle with `or_else(view)`, as the free `show` does.
- **One shape for a row's view.** The row is always `Row<T, K>`, by value; it is a `Copy` handle (ADR-0026 is
  amended to say so). The free `each` takes `Fn(Row<T, K>) -> Element`; `Lay::each` takes
  `Fn(&mut Lay<C>, Row<T, K>)`, the same with the builder in front and the slots in place of the returned element.
  The windowed list takes the free form.
- **A view decides per row.** A menu item is `hug` and a separator is `fixed(1.0)` in one `each`. A row with several
  roots calls `r` several times. A view that adds nothing is allowed.
- **The kind of slot is read untracked, once per key, and fixed for the mount.** A row whose slot kind depends on its
  item makes the kind part of its key, so a change of kind is a new row. A weight or a `fixed` size may be a binding
  (`impl IntoValue<f32>`).
- **A key keeps the list of slots its view emitted:** for each, its kind, its parameters and its child's
  description. The list may be empty. It is the key's saved description.
- **A cold rebuild replays that list and does not call the view again** (a cold frame, ADR-0028's `Check`; ADR-0026's
  cold-tree text is amended to say so). The slots of the cold tree are therefore the live tree's by construction, not
  by the view being pure. A debug assertion compares each slot's kind with the saved one on rebuild.
- **The slot is an item parameter the container owns,** kept beside the child and written only by the parent's `Lay`.
  `Styled` has no setter for it, and `restyle` (ADR-0026) cannot change it.
- **The parent sees a flat list.** A control node has no box (ADR-0026), so gaps and `justify` count rows
  (`rows_under_each`).
- **The view stays `Fn` and pure,** as ADR-0026 requires: it is still called once for every new key and every new
  mount.
- **A component's root is always a box** (amends ADR-0026's "on a boxless root"). A boxless range appears only inline
  in the body that builds its parent. A reusable part whose root would be a `show` gets a box. A family's typed
  parts (K6: `l.trigger(…)`) are functions called in that body with the family's own wrapper around `Lay`; they say
  the slot, and the caller never sees one.
- **The windowed list has no `Lay` to pass.** Its rows are island roots, not flex children (ADR-0014; ADR-0026,
  K4c). The list's type fixes each row's limits: the cross axis `Exact` (the list's inner width), the main axis
  `Unbounded`. The row's height goes to the `ExtentTree`. Its view is `Fn(Row<T, K>) -> Element`.
- **The slice exercises all of it before S8:** tabs (triggers through `l.trigger`, in a list that hugs and in one that
  is full width: `tab_list_at_most` is its acceptance case), select (items and separators in one `r.each`, in a
  hugging column of stretched rows), one virtual list page (no `Lay`).

### Bugs L2 fixes

Two fixtures differ from Chrome in today's engine. Both are inside what a typed `AtMost` promises, so L2 fixes them
in the CSS algorithms too, and each fix is a listed exception to "bit-identical to Taffy".

- **`fill_in_hug_row`, boxes 10–18.** A hugging row in a column aligned to its start is not clamped to fit-content:
  551.34 px in gui, 300 in Chrome (and 310.22 against 300 for boxes 10–12), so its paragraph does not wrap. The
  `Definite(_)` arm of the container's main size (`row.rs`, `container_main_size`; `flex.rs`,
  `determine_container_main_size`) takes the max-content size.
- **`percent_indefinite`, boxes 22–24.** A paragraph of `width: 50%` in a content-sized row gets the right width
  (111.04 px) and the height of its max-content width: 24 px, Chrome 48. The height is measured before the width
  settles. The typed protocol cannot do this: a width is decided before the one layout that gives the height.

Until then each is listed per box in `<fixture>.bugs`.

### L3: what is compared with what

- **One reader, two trees.** `chrome_layout.rs`'s reader builds a fixture as CSS (`flex()`, `css_grid()`, block). It
  also builds it typed, by this rule alone; no file says which subtree is which.

  | In the fixture | Typed |
  | --- | --- |
  | `display: flex`, no wrap, no ratio, no baseline | `row()` or `column()` |
  | An item with `flex: none` and a main size in px | `fixed(px)` |
  | An item with `flex: none` | `hug` |
  | An item with `flex: w` (basis 0) that has no automatic minimum: `min-width: 0` in a row, `min-height: 0` in a column, a `min-width` or `min-height` in px (the slot's `.min`), or `overflow` hidden or scroll on that axis | `fill(w)` |
  | An item of a row with no flex property, with or without a main size in px (the px is its content) | `fit(1.0)`; `fit(s)` for `flex-shrink: s` |
  | An item of a column with no flex property, with or without a main size in px | `hug`; `fit(1.0)` when it clips: `min-height: 0`, or `overflow` hidden or scroll on that axis |
  | `align-self`, `max-width`, `max-height` in px | the slot's `.align`, `.max` |
  | A child whose own `display` is a grid, or a flex that wraps, has a ratio or aligns to a baseline | a slot by the rows above, holding `css_grid()` or `flex()`; everything under it is CSS |
  | A flex container with any item that is a grower with an automatic minimum, has a basis other than `0` or `auto`, a percentage on either axis, a ratio or an `<img>` outside what "Aspect ratio" allows, or an `auto` margin | the container is `flex()`, and everything under it is CSS |

  The unit is the container: no line holds slots and CSS items together. So the fifth list of `tab_list_at_most`
  (boxes 15–18) and rows 1 and 6 of `min_auto_box` are `flex()` and must be Chrome's, and none of their boxes is in
  a `.typed` file.
- **CSS tree against Chrome:** every box within the reader's tolerance (1/64 px and a little), except the boxes in
  `<fixture>.bugs`, which must still differ.
- **Typed tree against Chrome:** every box within the same tolerance, the boxes of its CSS subtrees included (they
  are still laid out and still compared: boxes 10–12 of `fill_in_hug_row` must be Chrome's once the bridge is built
  on the fixed arm), except the boxes in `<fixture>.typed`.
- **`<fixture>.typed` lists geometry exceptions only:** `<box> = x y width height  # why`, a box index in document
  order from 0. The typed tree's box must equal the listed one within the tolerance, and the listed one must differ
  from Chrome's. Nine fixtures have one today (`tab_list_at_most`, `fill_in_hug_row`, `fill_weight_below_one`,
  `min_auto_box`, `min_auto_padding`, `min_auto_column`, `hug_column_stretched_rows`, `hug_label_in_fill`,
  `rows_under_each`). They were written by hand from the rules above and have run against nothing: L2 is their
  first check.
- **Typed against `flex()`, bit for bit,** only where the two are the same arithmetic. A line is in the domain when
  all of these hold:
  1. its main size is `Exact`;
  2. its slots are `fixed`, `hug` and `fill` only, with no `fit` that shrinks and no `spacer`;
  3. its fill weights sum to at least 1;
  4. no fill has padding, a border, a `min(px)` or a `max(px)`;
  5. no fill freezes in the loop of "How fills grow" (4 implies it while the row has room; it is listed because
     it is the cause).

  On the fixtures and on trees made from a seed in that domain, keeping `row.rs`'s order of float operations. Outside
  it the fixtures are the oracle: `fill_weight_below_one` for weights below 1 and for a capped fill, `min_auto_padding`
  row 5 for padded fills.
- **Differences and bugs are listed per box, never per fixture.** A listed box that matches again fails the test
  until it is taken off its list.
- **Verdict:** no demo below B0 (`crates/shadcn/verdict/baseline/vega.tsv`). "Pixels equal to B0" means that floor,
  not equal boxes where this ADR differs from Chrome on purpose.
- `one-pass.mjs` sets `overflow-wrap: break-word`, as gui sets all text, and `layout.mjs` does not. Review checked
  that it changes no fixture of `chrome_layout/`. L3 puts the two scripts on one page.

## Consequences

- S8 ports each family's layout once, to typed containers, with its move to run-once components. A site that cannot
  be typed stays `flex()` and says so by its name.
- `Style` loses `display`, `flex_direction`, `flex_wrap` and the flex and grid item fields, which become item
  parameters their container owns; with the port the `min_width(0)` calls go too.
- Some pixels move where a row or a hugging list is too narrow for its content. They move towards no overflow.
- `row.rs` and `stack.rs` stop being bit-identical to Taffy outside the domain L3 names.
- Two sets of algorithms remain: typed and CSS. That is the cost of keeping wrap, the card grid and text.
- A measure is a second kind of question again. It is bounded: at most two per node, at distinct limits, kept, and
  counted.

## Alternatives rejected

- **A slot that fills but never below its content.** It needs the content's min-content before the share is known:
  the intrinsic pass again. `hug`, `fit` and `flex()` cover the sites.
- **`hug` measured in what the earlier children left** (Compose). It orders siblings, so they cannot be laid out in
  parallel, and it differs from `flex: none` at 41 sites.
- **Refusing a fill in a hugging row** (Flutter's error,
  `references/flutter/packages/flutter/lib/src/rendering/flex.dart:1115`), **or giving it zero** (Compose shares out
  what is left of the row's minimum width, usually nothing: `RowColumnMeasurePolicy.kt:156-166`). A tab list hugs
  or fills depending on its caller; one slot must serve both.
- **Chrome's width for a hugging row that does not fit** (its min-content, overflowing). It is the overflow the zero
  minimum exists to end.
- **Re-measuring a hugging column's rows at every level.** A leaf under k such columns would be asked k + 1 times;
  the kept measure makes it one measure and one layout.
- **Sharing an `AtMost` row by weight as soon as it does not fit.** One px too many would cut a long label by 94.
  Shrinking from content is continuous.
- **A CSS item among typed slots** (falling back per item). The line would run two rules at once; the container
  falls back whole.
- **A fill's share as its inner size,** as CSS has it. Padding would then break the weights; Compose and Flutter
  share the outer size.
- **An item of a column with no flex as `fit`.** CSS never shrinks it below its content's height, so `fit` would
  squeeze every overflowing column that Chrome leaves alone.
- **A `spacer` that is a `fill(1.0)` with no child.** It would take a gap on each side and split the space with real
  fills; an `auto` margin does neither.
- **The slot said by the row's root on itself** (a setter on `Styled`). It compiles under any parent, which is the
  mistake `Lay` exists to prevent.
- **One slot for every row of an `each`** (an argument of `each`). A list of items and separators needs two.
- **A component with a boxless root that takes `Lay` as an argument.** Its slots would depend on a parent it cannot
  see; a root that is a box keeps a component usable anywhere.
- **A control node with a box.** ADR-0026 rejected it; it would also make every list one child of its parent.
- **Porting the general grid now.** 17 sites; no measurement asks for it.

## Open

- **Which sites move a pixel, and how many are `fit`.** The rules are decided; the sites are only known by porting
  and running verdict per family in S8.
- **The `.typed` expectations were computed by hand** and have run against nothing. Both reviewers recomputed those
  of round 2; the rows added after round 3 (`min_auto_padding` 16–17, `min_auto_column` 34–39,
  `fill_weight_below_one` 17) were reviewed by no one. L2 is the first check of all of them.
- **`overlay`'s anchor type** (which offsets and which sizes it can say) is fixed at L2 against the 42 sites, which
  L2 also reads one by one: the split of the 44 in "Context" is by node name.

## Revisit when

- Verdict shows a family that needs CSS's automatic minimum at many sites: reconsider a content floor as a declared
  slot.
- Measures are a visible share of layout time in a stress scene: give the hugging column a width from its data (a
  column model, as tables have).
- The Taffy pin blocks a dependency upgrade, or a grid site needs what Taffy's grid lacks: port the general grid.
