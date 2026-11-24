// M4 over a gang of pages (ADR-0018): one thread for each extended column
// a page of the gang meets, its points' range found by binary search on x
// and summed through the pyramid, then the candidates' pixels. The gang's
// pages are bound side by side (`page_w` and `fpage_w` pick one, emitted
// by `m4.rs`), so one dispatch covers them all: dispatches of a pass run
// one after another, and a dispatch a page left the GPU waiting on each.
// Partials of all pages are merged in page order by `m4_merge.wgsl`.

struct PageP {
    len: u32,
    cap: u32,
    // The page's first point in the snapshot.
    at: u32,
    y: u32,
    levels: u32,
    // y's low parts, or NONE.
    lo: u32,
    col0: u32,
    col1: u32,
    // Its first partial.
    out: u32,
    // The floor series' page: its y and y low parts (or NONE).
    fy: u32,
    flo: u32,
    pad: u32,
}

// The gang: its first page in `pps`, how many, and its threads (the
// partials its pages write, which follow each other).
struct Gang {
    first: u32,
    count: u32,
    threads: u32,
    pad: u32,
}

@group(0) @binding(0) var<uniform> m4: M4;
@group(0) @binding(1) var<uniform> gang: Gang;
@group(0) @binding(2) var<storage, read> pps: array<PageP>;
@group(0) @binding(3) var<storage, read> bounds: array<vec2<u32>>;
@group(0) @binding(4) var<storage, read_write> partials: array<u32>;

// Which of the gang's pages this thread reads.
var<private> cur: u32;

// `PARTIAL`, the words of a partial (a node, then (x, y, floor) pixels of its
// first, least, greatest and last point), is prepended from `m4.rs`.

fn x_px(pp: PageP, i: u32) -> f32 {
    let hi = page_w(i);
    let lo = page_w(pp.cap + i);
    return m4.bx + x_delta(hi, lo, m4.x0) * m4.kx;
}

fn y_px_of(y: f32, lo: f32) -> f32 {
    return m4.by + ((m4.y1.x - y) + (m4.y1.y - lo)) * m4.ky;
}

fn y_px(pp: PageP, i: u32) -> f32 {
    let y = bitcast<f32>(page_w(pp.y + i));
    var lo = 0.0;
    if (pp.lo != NONE) {
        lo = bitcast<f32>(page_w(pp.lo + i));
    }
    return y_px_of(y, lo);
}

fn fy_px(pp: PageP, i: u32) -> f32 {
    if (m4.floor_on == 0u) {
        return m4.base;
    }
    let b = fpage_w(pp.fy + i);
    if (!finite(b)) {
        return m4.base;
    }
    var lo = 0.0;
    if (pp.flo != NONE) {
        lo = bitcast<f32>(fpage_w(pp.flo + i));
    }
    return y_px_of(bitcast<f32>(b), lo);
}

fn put(pp: PageP, slot: u32, j: u32, i: u32) {
    let at = slot + 6u + j * 3u;
    if (i == NONE) {
        return;
    }
    partials[at] = bitcast<u32>(x_px(pp, i));
    partials[at + 1u] = bitcast<u32>(y_px(pp, i));
    partials[at + 2u] = bitcast<u32>(fy_px(pp, i));
}

@compute @workgroup_size(64)
fn page_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let g = gid.x;
    if (g >= gang.threads) {
        return;
    }
    // The page whose partials hold thread g's: the last to start at or
    // before it.
    let base = pps[gang.first].out;
    var k = 0u;
    loop {
        if (k + 1u >= gang.count || pps[gang.first + k + 1u].out > base + g) {
            break;
        }
        k += 1u;
    }
    cur = k;
    let pp = pps[gang.first + k];
    let w = base + g - pp.out;
    let e = pp.col0 + w;
    if (e >= pp.col1) {
        return;
    }
    let l = Lay(pp.len, pp.cap, pp.y, pp.levels);
    var lo = 0u;
    var hi = 0u;
    if (e == 0u) {
        if (m4.before != NONE && m4.before >= pp.at && m4.before < pp.at + pp.len) {
            lo = m4.before - pp.at;
            hi = lo + 1u;
        }
    } else if (e == m4.cols + 1u) {
        if (m4.after != NONE && m4.after >= pp.at && m4.after < pp.at + pp.len) {
            lo = m4.after - pp.at;
            hi = lo + 1u;
        }
    } else {
        let c = e - 1u;
        lo = page_lower_bound(l, bounds[c]);
        hi = page_lower_bound(l, bounds[c + 1u]);
    }
    let node = page_fold(l, lo, hi);
    let slot = (pp.out + w) * PARTIAL;
    partials[slot] = node.min;
    partials[slot + 1u] = node.max;
    partials[slot + 2u] = shifted(node.imin, pp.at);
    partials[slot + 3u] = shifted(node.imax, pp.at);
    partials[slot + 4u] = shifted(node.first, pp.at);
    partials[slot + 5u] = shifted(node.last, pp.at);
    put(pp, slot, 0u, node.first);
    put(pp, slot, 1u, node.imin);
    put(pp, slot, 2u, node.imax);
    put(pp, slot, 3u, node.last);
}
