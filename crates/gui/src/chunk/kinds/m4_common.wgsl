// What every M4 pass of a chunk reads (ADR-0018): the view and how data
// maps to the chunk's texture pixels. Extended column 0 is the point
// before the view, 1..=cols the device pixel columns, cols + 1 the point
// after it.

struct M4 {
    cols: u32,
    npages: u32,
    before: u32,
    after: u32,
    // The view's first x, as (high, low) words.
    x0: vec2<u32>,
    // x pixel = bx + (x - x0) · kx.
    kx: f32,
    bx: f32,
    // The view's top y (high, low parts): y pixel = by + (y1 - y) · ky.
    y1: vec2<f32>,
    ky: f32,
    by: f32,
    // Where an area or bar stands when it has no floor series (pixels).
    base: f32,
    floor_on: u32,
    // Extended columns: cols + 2.
    n: u32,
    pad: u32,
}

// Words of an extended column's M4: its count, then four points of
// (x pixel, y pixel, floor pixel, index).
const COLUMN: u32 = 20u;
