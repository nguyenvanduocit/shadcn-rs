// Candles (ADR-0018): four series of one x (open, high, low, close). Per
// page, each pixel column's points by binary search on x, the greatest
// high and the least low through the pyramids, the open of its first
// point and the close of its last; the pages merged in order; one candle a
// column, drawn in the main pass: a body from open to close, a wick from
// high to low. A column of one point is that point's candle, as wide as
// asked; a column of several is their OHLC, a pixel wide.

struct M4 {
    cols: u32,
    npages: u32,
    pad0: u32,
    pad1: u32,
    x0: vec2<u32>,
    kx: f32,
    bx: f32,
    y1: vec2<f32>,
    ky: f32,
    by: f32,
    pad2: vec4<u32>,
}

struct PageP {
    len: u32,
    cap: u32,
    at: u32,
    y: u32,
    levels: u32,
    col0: u32,
    col1: u32,
    out: u32,
}

// Words of a partial: first, last, high key, high index, low key, low
// index; then x of first and last, open, high, low, close pixels.
const PARTIAL: u32 = 12u;

fn x_px_of(hi: u32, lo: u32) -> f32 {
    return m4.bx + x_delta(hi, lo, m4.x0) * m4.kx;
}

fn y_px_of(b: u32) -> f32 {
    return m4.by + (m4.y1.x - bitcast<f32>(b) + m4.y1.y) * m4.ky;
}
