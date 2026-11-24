# L0: how many times children are measured (2026-11-24)

Plan row L0 (M12). Counted with `Stats::asked` and `Stats::ran`
(`crates/gui/src/layout/engine/mod.rs`), by kind of question a parent puts
to a child:

- min-content: a size probe with an axis at min-content;
- max-content: a probe with an axis at max-content (none at min-content);
- definite: a probe in definite space;
- placement: a final layout (it places the child's own children).

`asked` counts every ask, the cache's answers included; `ran` counts the asks
no cache answered (an algorithm ran; the root's own run counts as a placement).
`measures` is the existing count of leaf measure passes (`Measure::measure`
calls). Trees: `layout::tests::asks` in `crates/gui/src/layout/tests.rs`,
laid out cold at 400 x 300 on a fresh `LayoutTree`.

## Counts

Format: asked [min, max, definite, placement] / ran [min, max, definite,
placement] / leaf measures.

| tree | asked | ran | measures before | measures after |
| --- | --- | --- | --- | --- |
| text row (8 text leaves in a flex row) | 8, 8, 0, 8 | 8, 8, 0, 9 | 24 | 16 |
| nested flex (4 rows of 4 growing cells) | 212, 164, 0, 36 | 52, 84, 0, 37 | 48 | 32 |
| card grid (3 fr columns, 6 cards) | 54, 30, 0, 18 | 18, 6, 0, 19 | 12 | 12 |
| list window (scroll box of 12 flex rows) | 132, 24, 0, 48 | 96, 0, 0, 49 | 72 | 48 |
| wrapping row (4 wrapping leaves, 150 px) | 4, 4, 0, 4 | 4, 4, 0, 5 | 12 | 8 |

The asks and runs are the same before and after (the second ask still
happens; it finds its answer kept). Only the leaf passes drop.

## What a row did

A flex row asked each text item twice before placing it: max-content for the
flex base size, min-content for the automatic minimum
(`row.rs::flex_base_size`). Each ask was a separate leaf measure, and the
wrapped min-content measure measured the unwrapped text again to learn its free
width, which the max-content ask had just found. Plus the final placement:
3 measures per text leaf.

## What changed

- `Measure::measure_pair(known, max, min)` answers both from one pass; the
  default is two `measure` calls. A text leaf (`leaf.rs`) overrides it: one
  paragraph, one unwrapped metrics call serving as the max-content answer and
  as the free width of the min-content one. Same calls, same arithmetic:
  bit-identical.
- `View::prime_pair` (engine) runs it for a leaf child that reads only its
  width, holds neither answer (node cache and the two-slot measure memo), has
  no children, no ratio, and is shown; it keeps both answers in the memo
  (`keep_measured`), so the two asks `row.rs` makes next find them.
- `row.rs` calls it only when it will ask both (no `min-width`, not a scroll
  container, base size not definite).

## What it does not do

- It does not remove the ask. The min-content ask still goes through
  `child_layout`, the node cache and `leaf::compute` (padding, ratio). A
  protocol where the leaf's one answer carries every probe (L2) would remove
  these too; the asked columns above are what L2 is measured against.
- Only rows with a **text** leaf item gain. An input leaf has no shared work:
  `measure_pair` measures it twice (it reports 2 passes, which `prime_pair`
  counts as 2), so an input in a row costs the same 3 measures as before.
  Grids, columns, blocks and images are untouched (card grid unchanged).
- A warm layout (children cached) primes nothing: it is skipped when the node
  cache or memo holds an answer.

## What the counters count (review round 1)

- `asked` counts every request a parent puts to a child: through
  `View::child_layout` (flex, grid, block) and through the paragraph's own
  `inline::lay` (min, max and placement requests to the elements in its
  lines). A hidden layout is neither asked nor ran (`Asks::count` skips it, for
  both).
- `ran` also counts runs that no parent asked for: a relayout boundary's own
  job, and a speculative run on a sibling (`speculate`), whose later ask hits
  the cache and is counted then. So `ran.total() <= asked.total() + 1` holds
  for a cold layout (the root's placement is the one), and is asserted.
- `measures` counts the passes that ran: a text pair is 1, a fallback pair
  (an input leaf, a known width) is 2.
- **Debug and test builds only.** The counters are
  `cfg(any(test, debug_assertions))`: in a release build `asked` and `ran` stay
  zero, so the engine pays nothing for them. L2 should measure with a debug or
  test build (the tests do), or lift the gate if it needs them in release;
  `calls`, `misses`, `measures` and `placements` stay counted in release.

## A kept answer survives a sizing-mode change

`NodeCache::store` cleared everything when the sizing mode of the input
changed (a leaf only placed, in its inherent mode, then probed as a row's
item), which dropped the answers `prime_pair` had just kept: the leaf was
measured again. The memo of what a leaf measured (width to answer) does not
depend on the mode, so the clear keeps it.

## Gates

Tests (`layout::tests::asks`, `crates/gui/src/layout/tests.rs`), each proven by
a targeted revert:

- `a_flex_row_measures_a_text_leaf_once_for_its_base_size_and_its_minimum`:
  per-kind counts and measures (fails if `prime_pair` or the text override
  falls back: 24 measures, not 16).
- `the_boxes_are_the_same_to_the_bit`: FNV hash of every box's bits, taken
  before the change, equal after.
- `a_pair_measure_is_two_measures_to_the_bit`: `measure_pair` against two
  `measure` calls, sizes and baselines to the bit, over plain, one word, empty,
  styled runs, font fallback, nowrap, truncate, line clamp, centred text, an
  input, a textarea, an empty textarea; and the passes (1 for text, 2 else).
- `a_retained_row_laid_again_after_its_text_and_width_change_is_a_fresh_one`:
  a retained tree after five text and width changes equals a fresh one.
- `a_pair_measure_kept_survives_a_change_of_sizing_mode`: a block's text
  children becoming a row's: 8 measures, not 12 without the cache fix.
- `a_paragraphs_requests_to_its_inline_children_are_asks`: fails without the
  `inline::lay` count.
