@group(0) @binding(0) var<uniform> m4: M4;
@group(0) @binding(1) var<uniform> pp: PageP;
@group(0) @binding(2) var<storage, read> open: array<u32>;
@group(0) @binding(3) var<storage, read> high: array<u32>;
@group(0) @binding(4) var<storage, read> low: array<u32>;
@group(0) @binding(5) var<storage, read> close: array<u32>;
@group(0) @binding(6) var<storage, read> bounds: array<vec2<u32>>;
@group(0) @binding(7) var<storage, read_write> partials: array<u32>;

@compute @workgroup_size(64)
fn page_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let e = pp.col0 + gid.x;
    if (e >= pp.col1) {
        return;
    }
    let l = Lay(pp.len, pp.cap, pp.y, pp.levels);
    let lo = high_lower_bound(l, bounds[e]);
    let hi = high_lower_bound(l, bounds[e + 1u]);
    let h = high_fold(l, lo, hi);
    let w = low_fold(l, lo, hi);
    let s = (pp.out + gid.x) * PARTIAL;
    // The first and last point with all four valid: the high's.
    partials[s] = shifted(h.first, pp.at);
    partials[s + 1u] = shifted(h.last, pp.at);
    partials[s + 2u] = h.max;
    partials[s + 3u] = shifted(h.imax, pp.at);
    partials[s + 4u] = w.min;
    partials[s + 5u] = shifted(w.imin, pp.at);
    if (h.first == NONE) {
        return;
    }
    partials[s + 6u] = bitcast<u32>(x_px_of(high[h.first], high[pp.cap + h.first]));
    partials[s + 7u] = bitcast<u32>(x_px_of(high[h.last], high[pp.cap + h.last]));
    partials[s + 8u] = bitcast<u32>(y_px_of(open[pp.y + h.first]));
    partials[s + 9u] = bitcast<u32>(y_px_of(high[pp.y + h.imax]));
    if (w.imin != NONE) {
        partials[s + 10u] = bitcast<u32>(y_px_of(low[pp.y + w.imin]));
    } else {
        partials[s + 10u] = bitcast<u32>(y_px_of(high[pp.y + h.imax]));
    }
    partials[s + 11u] = bitcast<u32>(y_px_of(close[pp.y + h.last]));
}
