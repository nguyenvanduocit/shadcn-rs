# 0031. Path coverage is the area of the filled and clipped region

Status: proposed 2026-11-24 (M15). Acceptance would supersede the fragment coverage formula and the area oracle in
[ADR-0017](0017-paths-are-rasterized-on-the-gpu.md). The clip snapping defect falls under the existing rule in
[ADR-0020](0020-gui-snaps-to-pixels-only-under-translation-and-scale.md).

## What the GPU drew

Local headless GPU readback on 2026-11-24. The probe used a 16 × 16 scene at display scale 1, one white filled
`Item::Path` over black, and read the red channel of pixel (2, 2) after `draw_and_wait`. In the table, *left* is
the rectangle from x = 2 to 2.5 and y = 2 to 3. Each strip is a closed contour of the same path. All expected
values follow from straight edges at binary fractions. They can be reproduced independently of a reference renderer.

| Geometry in that pixel | Fill or clip | GPU, 0–255 | Expected, 0–255 |
|---|---|---:|---:|
| Left once | Non-zero, control | 128 | 127–128 |
| Strips at x = [2, 2.25] and [2.75, 3], opposite orientations | Non-zero | 0 | 127–128 |
| Left twice in the same orientation | Even-odd | 255 | 0 |
| Left twice in the same orientation | Non-zero | 255 | 127–128 |
| Left, with a clip keeping the same half | Transformed clip, integer local bounds | 64 | 127–128 |
| Left, with a clip keeping the opposite half | Transformed clip, integer local bounds | 64 | 0 |
| Left, with a clip keeping the same half | Transformed clip, fractional local bounds | 0 | 127–128 |
| Left, with a clip keeping the opposite half | Transformed clip, fractional local bounds | 128 | 0 |

For the two integer-bound clips use x = [0, 1] or [1, 4], y = [0, 4], radius zero, scale 2 about (−0.5, 0).
For the fractional-bound clips use x = [0, 1.25] or [1.25, 4], the same y range and scale 2 about (0, 0).
Attach the transform to the clip alone. Both constructions put the shared boundary at device x = 2.5.

Three independent operations explain the results.

1. The fragment shader sums signed edge contributions within a pixel and then applies the fill rule. Opposite
   contributions can cancel across disjoint regions. Repeated windings can count the same area twice.
   The CPU function `exact` integrates line contributions in f64 and applies the same rule to the sum. The existing
   test passed during the probe. Its result establishes agreement on its corpus and can miss this shared error.
   [Shader](../../crates/gui/src/gpu/path_render.wgsl) · [CPU reference](../../crates/gui/src/gpu/path/reference.rs) · [GPU tests](../../crates/gui/src/gpu/tests.rs)
2. The path shader multiplies path coverage by geometric clip coverage. Two halves have the same product whether
   they coincide or are disjoint. Both tests with integer local bounds isolate this loss of subpixel position.
3. `ClipTree::decide` snaps local bounds before applying the transform. In the fractional fixture it moves the
   shared boundary from device x = 2.5 to 2. A fix to snapping alone would still leave the integer-bound pair.
   [Clip tree](../../crates/gui/src/gpu/clips.rs)

## Proposed decision

For a device pixel, apply the path's non-zero or even-odd predicate at each point and take the area that also lies
inside every geometric clip. Group opacity and alpha masks retain their compositing semantics. Keep fractional clip
geometry through non-grid-preserving transforms as required by ADR-0020.

A scalar accumulated area suffices in a pixel where winding only takes the values 0 and 1, or 0 and −1, and the clip
contains the whole pixel. Other fast paths need their own equivalence argument. A single signed sum cannot recover
the general filled region. Independently averaged shape and clip coverages cannot recover their intersection either.

Use an independent test oracle for flattened line geometry. Split the pixel into horizontal bands at endpoints,
edge crossings and crossings with the pixel sides. Within each band, sweep edges in x order, group coincident
crossings, and integrate the widths of the intervals satisfying the fill and geometric clip predicates.
Hand-computed fixtures above check the oracle itself. Compare coverage from the same input edges first.
Verify stroke expansion separately using known cap and join geometry and offline outlines from rust-skia where
its stroke-to-fill operation succeeds. Its API reports hairlines separately. Curved outlines introduce a flattening
error that must be accounted for independently of the pixel integration tolerance.
[rust-skia stroke-to-fill at b98ab0b4](https://github.com/rust-skia/rust-skia/blob/b98ab0b4/skia-safe/src/core/path_utils.rs)

## Alternatives and verification

Comparing screenshots alone mixes outline, sampling and composition differences. Adding precision to the accumulator
preserves the two mathematical ambiguities above. Skia Graphite attempts direct geometric intersection for supported
shapes before selecting other clip paths. That is an implementation reference for retaining geometry. The GPU
algorithm for general contours and curved clips remains open. Any proposed route must cover coincident edges,
opposite orientations, dense tiles and fractional transforms while preserving the retained renderer's frame budgets.
[Graphite clip selection at 9aafbb3](https://github.com/google/skia/blob/9aafbb3/src/gpu/graphite/ClipStack.cpp)

Close M15's three findings separately. Use the fixtures above with a maximum pixel error of one channel level.
Compare incremental frames with a fresh redraw while changing the clip transform. Exercise local GPU and CI
lavapipe, check the verdict per demo, and record render-path bench and stress results. The measurements above cover
one local backend. Visual impact at real stroke joins, across demos and on other backends remains unmeasured.
