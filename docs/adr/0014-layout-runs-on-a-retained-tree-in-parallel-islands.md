# 0014. Layout runs on a retained tree, in parallel islands

Status: accepted, 2026-11-21; built 2026-11-21 (M4, see "As built"; cold goal met lightly loaded and, by estimate, idle; 2026-11-22 cold work cut 18%). Supersedes the layout half of ADR-0006's pipeline (`layout.rs`: "the
Taffy tree has rounding off" on a `TaffyTree` built every frame) and ADR-0008's per-frame rebuild of layout; keeps
ADR-0006's fractional layout, its measuring rules (`Breaks`, `min_content`, `ImageMeasure`) and its 1/64 px text
widths.

## Context

`gui` builds a new `TaffyTree` every frame, sets every style on it and computes the whole tree from the root
(`crates/gui/src/layout.rs`). The research (`docs/research-2026-11-21.md` §2, §5) measured what that costs:

- The stress `table` scene (60 023 nodes) spends 357 of its 388 ms steady frame in layout. It makes 1.05 million
  allocations, 443 MB a frame, and one backspace costs 554 ms. In the `grid` profile `Pipeline::lay_out` takes 537
  samples, and `Pipeline::input` another 329, because input lays the whole tree out again before routing an event.
- A benchmark of the `table` shape (scroll → table → body → 5 000 rows × 5 cells, ~65k nodes, release, Taffy HEAD):

  | Case | Time | Calls / misses / measures |
  | --- | --- | --- |
  | `TaffyTree` rebuilt every frame (today) | 26 ms build + 108 ms compute = 135 ms; 252 MB in 450k allocations | — |
  | Retained tree, rows as `fr` grids: first layout | 78–94 ms | 465k / 270k / 135k |
  | … nothing changed | 0.001 ms | 1 / 0 / 0 |
  | … one cell's text changed, same size | 2.3–4.3 ms | 35 060 / 20 / 5 |
  | … a cell wraps, its row grows 20 px | 27.6 ms | 110k / 5 027 / 7 |
  | … window 1 px narrower | 74 ms | ≈ first layout |
  | Rows as flex with fixed cells, one cell changed | 2.2 ms | 35 026 / 16 / 3 |
  | … relaying out only the fixed cell (a boundary) | 0.001 ms | 4 / 4 / 3 |
  | Whole table one `auto` grid, one cell changed | 122 ms | 575k / 175k / 75k |

- Three faults sit inside Taffy's own tree, not its algorithms:
  - `mark_dirty` clears every ancestor to the root (`references/taffy/src/tree/taffy_tree.rs:943-966`), so there is
    no boundary.
  - `set_style` dirties even an equal style (`:905-907`).
  - `resolve_calc_value` returns 0 (`:406`), and shadcn needs `max-w-[calc(100%-2rem)]`.
- The cache's final-layout slot compares `parent_size` even for a node that reads no percentage
  (`references/taffy/src/tree/cache.rs:278`). That is why one row growing 20 px reran 5 000 sibling rows.
- A list of a million 32 px rows is 32 million px tall, past 2²⁴, where `f32` steps by 2 px. Taffy is `f32`.

The algorithms themselves are good: Taffy's flex, grid, block and float (about 12k lines) pass 1 689 HTML fixtures
laid out by Chrome (`references/taffy/test_fixtures`). They see a tree only through traits
(`references/taffy/src/tree/traits.rs:147-358`), styles too (`style/mod.rs:77`), so a tree of our own can carry them.
Blitz does exactly that over its DOM (`references/blitz/packages/blitz-dom/src/layout/mod.rs:615-830`).

## Options

1. **Keep a `TaffyTree` across frames.** It is retained, but it keeps all three faults: no boundaries, no `calc()`, a
   copy of every style into 560-byte `taffy::Style`s, and `DetailedGridInfo` allocated per grid layout (`:553`).
2. **Another engine.**
   - Yoga is flex only; grid is in open PRs, with no release since 3.2.1 (2024-12).
   - Morphorm (vizia) and Torin (Freya) are not CSS.
   - Clay recomputes everything each frame.
   - GPUI clears its `TaffyTree` every frame (`references/zed/crates/gpui/src/taffy.rs:55`), our fault today.
3. **Our own engine,** as Ladybird wrote ~105k lines of Rust layout. It would buy nothing that option 4 does not
   give: the tree, the cache, the boundaries and the parallelism are ours in option 4, and Taffy's algorithms are
   right by Chrome's fixtures.
4. **A retained tree of our own implementing Taffy's low-level traits, Blitz-style, cut into islands at relayout
   boundaries and laid out in parallel.**

## Decision

Option 4.

- **One retained node tree,** patched by the reconciler and never rebuilt. A node's identity is position + key +
  component type. Our style implements Taffy's style traits, so nothing is copied. A setter compares the new style
  with the old before anything is dirtied. `resolve_calc_value` evaluates our `Calc`.
- **Our own `CacheTree`.** An entry records which inputs the node read (parent size, a percentage, available space in
  either axis, a baseline request) and matches on those only. A sibling's size change then misses no cache that never
  read it. Ladybird records the same thing (`references/ladybird/Libraries/LibWeb/Rust/src/fc_run_cache.rs:15-33`), and
  LayoutNG loosens its keys likewise. Taffy's repeated min/max-content probes are memoized per node in the same entry.
- **Results are immutable per node, and a node's offset lives at its parent,** as LayoutNG's fragments. Moving a node
  dirties nothing below it. A `HasNewLayout`-style flag lets paint skip a subtree that did not move.
- **Islands.** A node roots an island when all three hold:
  1. Its size does not depend on its content (Flutter's rule
     `!parentUsesSize || sizedByParent || constraints.isTight`, `references/flutter/packages/flutter/lib/src/rendering/object.dart:2870`).
  2. It is an independent formatting context that holds its absolutely positioned descendants.
  3. Its parent reads no baseline from it.

  Layer roots (popover, dialog, menu), scroll contents and each row of a virtual list are islands too.
  - Each island owns its own `Vec<Node>` arena.
  - Its parent keeps an `IslandStub` that computes the island's size from its input alone, so laying out the parent
    never touches the child's nodes.
  - Dirtying stops at the island root.
  - Boundaries decided by style apply in the sequential step. Boundaries found while laying out (tight constraints)
    apply the next frame, with hysteresis: tight twice to split, loose once to merge.
- **Parallel waves.** One layout pass runs in four steps:
  1. A sequential step on the UI thread applies setters and structure.
  2. Dirty islands lay out in waves, shallowest first. A wave is `par_iter_mut` over `Vec<Island>`, one
     `&mut Island` per job, on a dedicated `rayon` pool (¾ of the logical cores, at most 8, as
     `references/ladybird/Libraries/LibWeb/Rust/src/style_layout_pool.rs:14-29`).
  3. An island whose input changed is dirty for the next wave.
  4. Overflow is merged into scroll containers after the wave.

  This is safe Rust with no lock or atomic per node. Servo pays an `AtomicRefCell` per box
  (`references/servo/components/layout/layout_box_base.rs:44-57`); Ladybird uses `unsafe` columns. A layout job is
  `Send` and reads no signal. Reactive handles stay `!Send`, so the type system enforces that.
- **Inside a big island** on a cold layout, nodes are kept in pre-order and `split_at_mut` divides children's
  subtrees, through a batch hook proposed to Taffy (`compute_child_layouts`, sequential by default). Thresholds follow
  Ladybird (`layout/parallel_layout.rs:50-101`): at least 2 jobs of ≥16 nodes, ≥128 nodes in all.
- **Text.** Shaping is cached apart from line breaking on the text node, so it belongs to its island. Each pool
  thread has its own `parley::LayoutContext` and shares a `fontique::Collection`.
- **Virtualized lists and tables for millions of rows.**
  - A list is a leaf sized by its parent, so it is a boundary. Its rows are separate island roots, not Taffy children
    of the list.
  - **Heights** live in an `ExtentTree`: a B+ tree with run-compressed leaves, heights in 1/64 px (Blink's
    `LayoutUnit`), sums in `u64`. Offset to index, index to offset, edit and splice are O(log n). Unmeasured rows take
    a per-content-type estimate kept outside the tree.
  - **Scroll position** is an anchor `(key, index, offset)`, never an absolute y (Compose
    `LazyListScrollPosition.kt:32-36`, Zed `references/zed/crates/gpui/src/elements/list.rs:1448-1453`). Every `f32`
    is local to a base row near the viewport; `f64` exists only as derived output.
  - **Data** is a `Collection<T>`: an immutable shared snapshot plus a log of `Splice`, `Update`, `Move` and `Reset`.
    Sort and filter are a `View<T>` permutation built on a worker and delivered through `mailbox`. Keyed diff runs
    over the visible window only.
  - **Rows** leaving the window go to a pool per content type, keeping their nodes and paint resources. A reused row
    always runs `setup` again.
  - **Tables** have a column model: `Auto` measures the rows seen so far and only grows.
- **Debug checks.**
  - A debug mode lays out cold after each frame and compares with the cached result.
  - A debug check confirms an island root's stub size equals its laid-out size.
  - Results must be bit-identical across 1 thread, N threads, reversed job order and any island split.

## Consequences

- Expected, from the benchmark: a frame where nothing changed costs about 0.001 ms of layout. A changed cell inside a
  boundary costs its island, not its 5 000 siblings. A row growing 20 px costs one row, not 27.6 ms. Allocations per
  steady frame fall to near zero, since nothing is rebuilt.
- First layout and resize (78–94 ms and 74 ms on 65k nodes) divide across the pool. Lists of millions of rows cost
  O(changes · log n + window) per frame, with nothing proportional to n.
- `Pipeline::input` no longer lays out before routing: hit testing reads the retained results.
- `layout.rs` is rewritten. Every call site in `crates/gui`, `crates/shadcn` and the examples changes with it.
  - `table.rs` gets the column model.
  - `message_scroller.rs` is rebuilt on the list's tail-follow mode.
  - ADR-0008's two-frame cache rule counts pooled and prefetched rows as in use.
- Taffy breaks its API every minor release; we follow it. Where Taffy lacks something (sticky, writing modes,
  subgrid, `contain: size`), we implement it in our trait implementations or upstream.
- A wrong island (size actually content-dependent) gives a wrong layout, not only a slow one. The debug check above is
  what catches it.

## Revisit when

- The cold-versus-cached check or the thread-order check finds a difference that the cache key cannot express: the
  cache entry's recorded inputs are then incomplete.
- A layout Chrome produces cannot be expressed through Taffy's traits (inline formatting beyond Parley's inline boxes,
  writing modes): implement that algorithm in our trait implementation, or reconsider option 3 for that mode only.
- Taffy rejects the batch hook and big islands dominate cold layouts: keep a patched copy of the flex and grid loops.
- Scroll anchoring disagrees with Chromium's chosen anchor on a verdict page.

## As built (2026-11-21)

Built in `crates/gui/src/layout/` (`mod.rs` the reconciler and the frame, `engine/` the tree, its Taffy traits,
`cache.rs` and `calc.rs`, `boxstyle.rs`, `leaf.rs`) and `crates/gui/src/list/` (`extent_tree.rs`,
`virtual_list.rs`, `view.rs`, `columns.rs`), with the differences below.

- **The tree.** `LayoutTree` keeps one `engine::Node` per element across frames and patches it by place, key and
  kind (a positional fast path, a key index past the first mismatch). A node owns its children (`Vec<Node>`), so an
  island is a subtree owned by its root rather than a separate arena: `split_at_mut` over children gives each job a
  `&mut` subtree with no `unsafe` and no ids to remap. Out-of-flow descendants are reached through `FOREIGN` ids
  their containing block keeps. Taffy's `LayoutPartialTree`, `CacheTree`, the flex, grid and block container traits,
  `LayoutContainingBlock` (detailed grid info, for grid absolutes) and `RoundTree` are implemented on a view of one
  node; the style is read through Taffy's style traits.
- **`BoxStyle`.** Taffy's `CompactLength` holds a raw pointer (for `calc()`), so its styles are not `Send` and could
  not cross to the pool. `BoxStyle` keeps our own `Len` and answers Taffy's style traits from it (no copy into a
  `taffy::Style`); `reads_parent` records which axes its percentages read. The style diff is `BoxStyle`'s
  `PartialEq`, before anything is dirtied.
- **`calc()`** goes through a process-wide interner: `resolve_calc_value` gets an index, not a pointer. Dialog and
  alert dialog are `w-[calc(100%-2rem)]` with `max-w-lg` (`Length::Calc`), identical to the padded frame they
  replace at 1280, 500 and 360 px windows.
- **Cache.** `NodeCache` keys a result as Taffy does, but compares the parent size only on the axes the node's own
  percentages read (Ladybird's `fc_run_cache`); 16 probe slots replaced second-chance; every entry keeps the whole
  input it answered. A box in a block formatting context with floats, or with floats inside it, bypasses the cache
  (its answer depends on floats outside the key).
- **Boundaries are decided per change, and checked.** A changed node whose style did not change is laid out again
  alone when its parent fixed its size and used nothing else (Flutter's rule, `is_boundary`). Otherwise it is laid
  out again for every answer it gave and compared with the old ones (as Blink stops at a box whose size did not
  change); only a differing answer dirties the parent. A wrong boundary therefore costs time, never a wrong layout.
- **Parallel layout is speculative, by subtree.** On a cache miss of child `i`, the same input is put to its alike
  siblings (same display and position, no cached answer, not yet placed in this parent run) as jobs on the layout
  pool (rayon, ¾ of the cores, at most 8, `GUI_LAYOUT_THREADS`), grouped to at least 16 nodes, 128 in all
  (Ladybird's thresholds). What the parent then asks is a cache hit. Changed subtrees that wait at boundaries run as
  jobs the same way. Results are bit-identical across 1 and N threads and reversed job order (tested on a
  1 600-node tree and on seeded random trees).
- **Text in the pool.** Each pool thread has its own `TextSystem`, made when the pool starts; they share one
  `fontique` collection and source cache, so a font file is read once. A pool thread forgets what it shaped at the
  end of each pass: the tree's caches keep the answers.
- **Virtual list.** Rows are the children of a scroll container whose offset is local to the rows built, not
  separate roots: the list's size comes from its parent, so it is a boundary anyway, and the keyed diff runs over the
  window only. The window is decided at render from the anchor and the viewport last laid out (so rows may be
  components); after layout the list measures its rows into the `ExtentTree`, moves its anchor to where the wheel
  left the viewport and asks for another pass when the window must move (the same frame shows it). The window keeps
  its rows while they cover half the prefetch margin on each side: rows that left the viewport stay built, and the
  paint keeps the text of nodes within two clip heights of a clip shaped, so ADR-0008's two-frame rule counts them
  in use (L11; a scroll back and forth shapes nothing again, tested). The list draws its own scrollbar from the
  `u64` extent; a thumb drag anchors at the row under that fraction. Unmeasured rows take the mean of the rows
  measured. `aria-rowcount`/`aria-rowindex` (`Semantics::row_count`, `row_index`) carry the whole count.
- **Lists on data indices (L10).** `message_scroller` runs on the virtual list: the thread is a slice of messages,
  its rows are built for the window only (100 000 messages build fewer than 40 rows, tested), the list follows its
  tail while a reply streams, a new turn is anchored with a peek of the previous one, and `scroll_to_message` and
  `MessageScrollerVisibility` speak in message indices. `command` and `combobox` lists are one `CommandList` over
  `CommandSection`s: keyboard navigation, the highlighted item and scrolling it into view work on the filtered
  data's indices, not on what was laid out, and the rows go through the virtual list (a footer is not virtual). A row that
  leaves is not re-keyed for a new row: a pool by content type would re-path a subtree's identity, which the
  frame's paths, focus and motion key on.
- **Flex columns memoize their items.** A flex column (nowrap, known width, items in flow and stretched) runs
  `engine/stack.rs`, Taffy's flexbox algorithm for that case ported step for step (the same float order), which
  keeps each item's flex-basis answers and final layout per container input and reuses them while the item's cache
  is unchanged. A row growing 20 px no longer asks 5 000 siblings again (86 calls, was 35 079). Two older faults the
  differential test (`tests/flex_columns.rs`, 300 seeds, serial and parallel) found are fixed with it: a boundary is
  not laid out alone when an out-of-flow box sits in its subtree before or after the change, and a boundary whose
  scrollable overflow changed passes the change up. The Taffy fixtures compare the scrollable overflow too.
- **Entries record what the answer read of the height (L3).** A `cache::Reads` per node says whether its answers
  read the available height when no height is known, and whether its width probes read the height they are given;
  an entry may also carry `free_height`, set by the algorithm that gave it. Each is read from the algorithm's source,
  never guessed: a leaf passes the available height only to its measure (`Measure::reads`: text and inputs read the
  width alone, an image its height when its style sets one, everything by default); a single-line flex row passes it
  to its items, and to a stretched item's width probes as a known height, so it reads it only through items that do
  (`Node::heights_read`); a grid sets `free_height` when every item's minimum and limited min-content contributions
  are bit-identical and every row's base size equals its finite growth limit (`engine/grid.rs`). A change of what a
  node reads lays its parent out again; a checked answer that was free stands only if the new one is free too. The
  1 689 Taffy fixtures run with the horizontal Ahem measure declaring width-only. In benchmark 5.2 a text cell is
  measured 4 times instead of 10 (200k → 80k measures) and the row grid answers its min- and max-content probes
  from one run (350k → 170k algorithm runs).
- **Grids and flex rows on our own passes.** `engine/grid.rs` is Taffy 0.15.1's grid for fixed and
  `minmax(fixed, fr)` columns at a known width, auto rows, span-1 auto-placed items in flow (no out-of-flow box in
  the subtree: `DetailedGridInfo` cannot be built outside Taffy); `engine/row.rs` is its flexbox for single-line
  rows (either direction, known or unknown width, every item in flow, no baseline, aspect ratio or sizing keyword).
  Both are ported step for step in Taffy's float order, bit-identical, with each run's items in per-thread buffers
  (at most 8 kept, of at most 256 items) instead of Taffy's per-run vectors and named-line strings; anything else
  is the ported flexbox or Taffy's grid (next items), before any child is asked. Seeded differential tests against a fresh `TaffyTree`
  (`tests/grid_rows.rs`, `tests/flex_rows.rs`, 600 seeds × 3 schedules × 24 changes, with absolute cells) hold them.
- **Faults the grid and row tests found, fixed.** A size probe cleared the ids a cached final layout had given the
  out-of-flow boxes below it, so its containing block lost them after the space changed (`foreign` is now renamed
  only by final layouts); and a changed node whose subtree holds an out-of-flow box no longer stops at answers
  that agree, since its containing block's answer carries that box.
- **A probe that places a subtree unplaces the final layout above it.** Taffy's grid and flexbox find a
  baseline-aligned item's baseline with a final layout of the item (`perform_child_layout`, the grid's at min-content),
  also while the container is only probed for its size; a block places its floats so too. That final layout places
  the item's own children for the probe. A node's place lives on the node, so the container's final layout still
  cached (a later hit replays nothing) no longer matched what its subtree showed: 27 of 1 800 grid runs and 3 flex-row
  runs kept a stale place after a change. Each run of a container's algorithm for a final layout is counted
  (`Stats::placements`); a probe whose run counted one calls `NodeCache::unplace`, and a final layout is answered from
  the cache only while its node is placed for it (`placed`, set by the final layout kept). The stale answer stays an
  answer for checking a change (`take_check`), and `unplace` bumps the serial, so a flex column's memo of the item's
  final layout misses too. The hit then runs the algorithm again, which lays the item out where it goes. Blink has no
  such state to keep right, since its fragments are immutable and a baseline query builds a result of its own; the
  tree keeps places on the node, so it records which final layout they belong to. Only probes that run a final layout
  below pay for it: baseline alignment in Taffy's own grid and flexbox and floats; the ported passes run none.
  Regression test `a_probe_that_lays_out_a_baseline_cell_does_not_leave_its_children_stale`; the two baseline tests
  run with the rest.
- **Leaves on our own pass, with baselines and CSS's aspect ratio.** `engine/leaf.rs` is Taffy 0.15.1's
  `compute_leaf_layout` ported step for step, with two differences the Chrome fixtures (ADR-0023, I4) found. (1)
  `Measure::measure` answers a `Measured`: the content size and the first and last baselines from the content box's
  top, which the leaf moves down by its padding and border. Text gives them from `TextSystem::metrics`, each line's
  baseline by Blink's rounding (ascent and descent rounded, half the leading floored, as `draw` places glyphs: one
  function, `line_baseline`, serves both); an input its primary font's on its one line; an image none. Taffy's leaf
  has no baselines, so `align-items: baseline` aligned paragraphs by their bottom edge (`flex_baseline`). The cache
  keeps a final layout's whole output, baselines included, and a checked answer compares them, so a changed baseline
  lays the parent out again; a size probe carries none and none is read from one. (2) Taffy ends with `height =
  max(height, width / ratio)` whatever the height was, so a box sized both ways (`width: 120px; height: 40px;
  aspect-ratio: 1`, or a flex item or grid cell its container sized both ways) took its height from the ratio in its
  final layout, though its size probe, answered early, did not (`aspect_ratio`). Here a ratio decides an axis only
  when that axis is not definite (css-sizing-4 §5): from the other axis's size within its min and max, in the box
  `box-sizing` names, at least the content's size in that axis unless it is a scroll container or replaced
  (`Measure::replaced`), then clamped by the axis's own min and max. The differential tests' Taffy side lays leaves
  out with the same function (they test the tree and the ported containers, not the leaf); the Taffy fixtures keep
  Taffy's own leaf on that side, and all 1 689 still agree, so no fixture reaches what differs. What the containers
  decide before the leaf is asked is the next item's. Regression tests: `engine::leaf::tests`,
  `text::tests::metrics_give_the_baselines_draw_places_glyphs_on`, and the fixtures `aspect_ratio`,
  `aspect_ratio_auto`, `flex_baseline`, `grid_baseline`.
- **Block and flexbox on our own passes; CSS's aspect ratio in every container.** `engine/block.rs` is Taffy
  0.15.1's block algorithm (margin collapsing, floats and `clear`, formatting contexts, `align-content`, legacy
  `text-align`, `justify-self`) and `engine/flex.rs` its whole flexbox (multi-line, `wrap`, `wrap-reverse`, every
  direction, right to left, baseline alignment, absolute items, and `flex-wrap: balance` with `engine/balance.rs`),
  ported step for step in Taffy's float order on the tree's view; `compute_block_layout` and
  `compute_flexbox_layout` are no longer called (`row.rs` and `stack.rs` still take their cases first). Grid keeps
  its own pass and falls back to Taffy's `compute_grid_layout` for the rest. Where Taffy differs from Chrome it is
  not copied. Taffy resolves a ratio on a child's size, min and max wherever it reads them
  (`maybe_apply_aspect_ratio`) and passes the result down as definite, so a ratio size could not grow with its
  content and a min or max of the ratio-dependent axis was carried into an axis the style sizes. Here
  (`engine/sizing.rs`): a parent reads a child's min and max through the ratio only into an `auto` axis
  (`transfer`), and gives no child a size its ratio derives; the box derives it itself (`Ratio`, `with_ratio`): at
  the ratio's size, taken as definite, then at least what its content took there unless it is a scroll container or
  replaced, clamped by that axis's own min and max. Block and flex report that content from the run at the ratio's
  height, so a child's percentage height resolves against the ratio's height, as in Chrome (an image `h-full` in
  shadcn's `AspectRatio` fills it; a `height: 150%` child of a box the ratio makes 80 tall is 120, and so is the
  box). A width the ratio derives from a height is at least the min-content width. Taffy's grid had the same fault
  for its items: the grid pass and the fallback read items through `transfer` and give no ratio item a known size;
  a grid with its own ratio is the fallback's, which asks a content-size probe first, since Taffy's grid does not
  report its content (a percentage height inside such a grid resolves against nothing in that probe; no fixture
  has one). The Chrome fixtures `aspect_ratio_block`, `_column`, `_containers`, `_content`, `_flex`, `_grid`,
  `_percent` and `_wrap` all match, and `tests/chrome_layout.rs` has no known bug left. The 1 689 Taffy fixtures
  pass, ratio ones included: none reaches a difference (the harness now reads `flex-wrap: balance`, which it read
  as `nowrap`, so four `balance_row_percent_aspect_ratio` fixtures failed by one ulp). Seeded differential tests:
  `tests/block.rs` (block flow with floats, collapsing margins, new formatting contexts, flex and grid inside,
  500 seeds × 3 schedules × 16 changes; 20 000 seeds once), `tests/flex_wrap.rs` (multi-line flexbox, 500 seeds),
  and `tests/flex_columns.rs` now with columns and items the column pass declines; none generates a ratio, which
  is compared with Chrome. `tests/block.rs` compares with Taffy's algorithms on a tree of its own, since
  `TaffyTree`'s cache is wrong twice there: it answers a block laid out in its parent's formatting context from its
  cache, so the floats inside it are not placed and a later `clear` ignores them (CSS 2 §9.5.2), and it answers a
  final layout from its cache after a size probe placed the subtree for itself (a block placing floats, a baseline
  query), so the size of one run comes back over the children of another.
- **Faults `tests/block.rs` found, fixed.** (1) A block whose margins collapse through into its parent was a
  boundary when its parent fixed its size, but its parent also uses the margins: a child laid out with collapsible
  margins is not `tight`. (2) A box in a formatting context with floats lays out uncached, but the final layout
  kept from an earlier run still answered as placed: such a run now unplaces it. (3) A box in its parent's
  formatting context with a float inside was checked alone in a fresh formatting context of no width, where a right
  float lands at 0: its change now goes to its parent. (4) A subtree an out-of-flow box joined or left (`position`
  changed) passed its check on size answers, which carry no candidates, so its containing block kept the box: such a
  change now goes to its parent too.
  With the ports: verdict 99/100 over 389 demos, the same 5 FAIL, every demo's score as before (AspectRatioDemo
  100, once the ratio's height became its children's percentage basis); `GUI_COLD_CHECK=1` monkey 300 steps,
  seeds 1, 7, 42, no mismatch; every stress scene but `canvas` (M6's, over its allocation budget) passes.
  Benchmark 5.2's cold layout is unchanged within the shared machine's noise (10 runs × 5, alternated: median
  47.9 ms before, 46.3 ms after, the same 365k calls, 170k misses and 80k measures): its rows are grids on the
  grid pass, and only its scroll container and column changed algorithm.
- **Cold layout does less (2026-11-22).** Measured by instructions as well as time, since the shared machine's
  load moves time far more than the work (`layout_bench` now prints the process's retired instructions and CPU
  time per cold layout, `gui::instructions`). What each change bought, one thread, benchmark 5.2:
  - A child whose cache was just found not to hold the answer is not looked up a second time
    (`compute_missed`; Taffy's `compute_cached_layout` looked again on every miss), and `BoxStyle`'s length
    accessors are inlined into the algorithms: 550M → 509M instructions. A row's item keeps its maximum height
    resolved once (`max_cross_style`) instead of a Taffy length resolved again.
  - **The grid's final layout reuses its size run** (the step this ADR named): a grid keeps its items'
    contributions to its rows (`grid::Memo`, plain numbers: Taffy's track and length values may not leave their
    thread), keyed by the width its columns were sized for and each item's cache generation, dropped when its
    style or children change. Its rows are `auto`, so no contribution reads its height; the rows are sized from
    the kept contributions again under the new height (which of an item's two it adds, maximising, stretching),
    in the same float order. The final layout no longer asks its cells their min- and max-content heights again
    (365k → 315k calls): 509M → 453M.
  - A text measures once per width (`MeasureReads::TEXT`: the answer reads the width given alone, known or
    available alike, as `gui`'s text and inputs do; `NodeCache` keeps the last two, in place): its final layout
    at the width its height probe used measures nothing (80k → 60k measures). Little in the bench, whose
    measure is a few hundred instructions; a real paragraph's line breaking is far more.
  - Run names come from a block per thread (`next_run`), not one atomic all threads contend for.
  - **Tried and dropped:** keeping a flex row's items with their flex base sizes for its final layout (the
    other step this ADR named). Its cold case is a cell's final layout after its size probe at the same width;
    copying the items in and out cost what the three cache hits it saved did (453M → 451M, CPU time no better),
    since a single-line row's run is mostly its own arithmetic, not its items' answers. A column's memo
    (`stack.rs`) pays because it is asked again with the same input after a change; a cold row is not.
  - **The parallel schedule.** Groups are a quarter of the work per thread, as before (finer groups did not
    measure better). A batch started inside a job already on the pool runs its groups in place with that
    job's measuring context: a nested group stolen by a thread whose own job holds its text system made a new
    one, so the stress `random` scene (3 000 nodes, 20 edited at once) took 1.6 s a cycle on the pool against
    69 ms on one thread; now 0.43 s. It is still slower than one thread there: speculation puts a final layout
    to siblings whose sizes differ, and pool threads shape again what the UI thread's text system had shaped.
    Letting the calling thread take groups too (it waits in `install`) lowered the median a little but moved
    the jobs' allocations onto the UI thread's allocation budget (stress `random` went over it), so it was not
    kept.
  - Back to back, alternated, 9 cold layouts each, best (median). One thread at load 48–50 (10 cores: 4
    performance, 6 efficiency): before 32.6–36.9 ms (39–56), 546–550M instructions, CPU 32.6–36.9 ms; after
    28.2–34.3 ms (40–46), 450–453M, CPU 28.0–34.3 ms. Seven pool threads at load 38–41 (a verdict run beside
    it): before 18.5–33.6 ms (26–49), 566–569M instructions, CPU 59–63 ms; after 15.8–24.1 ms (39–44),
    468–471M, CPU 42–48 ms. Single-thread work fell 18%, not the 2–3× aimed at: per row the algorithms already
    run the fewest times they can (34 runs: a grid's size run and final layout, each cell's height probe and
    final layout, each text's max- and min-content widths, height and final layout), and what remains is each
    run's own cost (about 2 600 instructions on average), in code ported step for step to stay bit-identical.
    On an idle machine, from the CPU one thread needs (28 ms) and the pool's efficiency measured at low thread
    count (two threads: 0.67), eight threads would take about 28 / 8 / 0.67 ≈ 5–6 ms, plus the column's
    sequential pass over its 5 000 rows (about a fifth of the wall time on the pool): the goal holds idle and
    lightly loaded; on this machine at load 40–50 the best runs make it and the medians do not.
- **Memory.** Elements are `Rc`-shared: an instance's raw output, its expanded output, the frame's root and the
  pipeline's last tree are one tree, and expanding placeholders copies only the boxes on the way to them. The tree
  keeps only what paint reads of a resolved style (`PaintStyle`), style and text bindings are boxed, a node's
  `Behaviour` and its `NodePath` are shared with the frame, a transition keeps its run boxed, and the frame diff is
  matched front and back before hashing the middle and shared with the report. The stress `table` (60 023 nodes,
  all laid out) holds 358 MB (662 MB before these changes, 367 MB in M3).

| Measurement | Before (M3) | After (M4) | Goal |
| --- | --- | --- | --- |
| Benchmark 5.2, one cell changed, same size | 2.3–4.3 ms | 0.006–0.017 ms (5 calls) | ≤ 0.05 ms |
| Benchmark 5.2, a row 20 px taller | 27.6 ms | 0.28–0.31 ms (73 calls, 20 misses) | ≤ 0.5 ms |
| Benchmark 5.2, cold, ~60k nodes | 84–135 ms (`TaffyTree`) | 28–34 ms on 1 thread (450M instructions, was 550M); 15.8–24.1 ms on 7 threads at load 38–41 (10 cores, shared); idle estimate 5–6 ms | ≤ 15 ms on 8 cores (met lightly loaded; estimated idle) |
| Benchmark 5.2, nothing changed | 0.001 ms | 0 calls | 0 calls |
| Stress `table` steady (layout) | p50 320 ms (300 ms) | p50 96 ms (90 ms) | — |
| Stress `table` heap | 367 MB | 358 MB | ≤ 367 MB |
| Stress `grid` steady (layout) | p50 29.1 ms (22.2 ms) | p50 9.3 ms (5.1 ms) | — |
| Stress `churn` steady (layout) | p50 19.1 ms (13.8 ms) | p50 6.0 ms (2.7 ms) | — |
| Stress `list`, 10M rows | — | steady p50 1.7 ms, p95 2.0 ms; layout 0.14 ms | ≤ 8.33 ms a frame |
| Stress `bigtable`, 1M rows | — | steady p50 3.1 ms, p95 6.4 ms; layout 1.0 ms | ≤ 8.33 ms a frame |
| `docs --bench`, 85 pages, steady p95 median | 5.78 ms (1 FAIL: code-block 19.65 ms) | 3.28 ms (0 FAIL, worst 8.8 ms) | ≤ 16 ms |
| `docs --bench`, layout median, allocations median | 0.38 ms, 1 220 a frame | 0.17 ms, 685 a frame | — |
| Taffy fixtures (`references/taffy/tests/xml`) | — | all pass: cold, after each change, 3 schedules | as `TaffyTree` |
| Verdict | 99/100, 5 FAIL | 99/100, the same 5 FAIL | not lower |

Cold layout first did the work Taffy's algorithms ask for: 350k algorithm runs and 200k text measures for 60k nodes,
since the body column asks each row grid for its min-content height, its flex basis and its final layout, each under
another available height, and each grid run asked its cells again under that height. With entries that record what
they read, the grid and row passes and the two faults above fixed, it takes 170k runs and 80k measures (each text 4
times: min- and max-content width, height at its width, final layout), and one thread lays it out in 30–50 ms
against `TaffyTree`'s 96–130 ms in the same runs (82–91 ms before). On the shared 10-core machine, 7 or 8 pool threads
took 13.3–16.5 ms at load average 16–19 and 24–45 ms at load 32–39, so the goal holds only while the machine is lightly
loaded; it was not measured idle. What remains is per-run cost, not runs: the final layout of each cell and grid, the
cache lookups (about 9%), and the leaf measures. The next steps are to reuse the grid's track sizes from its size run
in its final layout when the height it is given is the one it answered, and to keep a flex row's item answers per
container input as `stack.rs` does for columns.
