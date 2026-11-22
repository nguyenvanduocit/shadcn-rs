// Scatter (ADR-0018): points culled to the chunk's texture and compacted
// in data order by an ordered prefix sum (never an atomic append), so
// translucent points overlap as they come and every frame equals one drawn
// cold; drawn with an indirect draw whose count the GPU wrote.

struct View {
    x0: vec2<u32>,
    kx: f32,
    bx: f32,
    y1: vec2<f32>,
    ky: f32,
    by: f32,
    // The texture's size, the radius (pixels).
    w: f32,
    h: f32,
    r: f32,
    pad: f32,
}

struct PageP {
    len: u32,
    cap: u32,
    y: u32,
    // y's low parts, or NONE.
    lo: u32,
    // The points looked at: the first, how many, in blocks of 4096.
    first: u32,
    count: u32,
    blocks: u32,
    // The page's slot (its draw arguments).
    slot: u32,
}

const NONE: u32 = 0xffffffffu;
const PER: u32 = 16u;

fn x_px(i: u32) -> f32 {
    let hi = page[i];
    let lo = page[pp.cap + i];
    return view.bx + x_delta(hi, lo, view.x0) * view.kx;
}

fn y_px(i: u32) -> f32 {
    let b = page[pp.y + i];
    var lo = 0.0;
    if (pp.lo != NONE) {
        lo = bitcast<f32>(page[pp.lo + i]);
    }
    return view.by + ((view.y1.x - bitcast<f32>(b)) + (view.y1.y - lo)) * view.ky;
}

fn shown(k: u32) -> bool {
    if (k >= pp.count) {
        return false;
    }
    let i = pp.first + k;
    if ((page[pp.y + i] & 0x7f800000u) == 0x7f800000u) {
        return false;
    }
    let x = x_px(i);
    let y = y_px(i);
    return x >= -view.r - 1.0 && x <= view.w + view.r + 1.0 && y >= -view.r - 1.0 && y <= view.h + view.r + 1.0;
}
