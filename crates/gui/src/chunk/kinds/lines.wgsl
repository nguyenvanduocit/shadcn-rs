// Lines and areas (ADR-0018). A line draws straight into the main pass:
// one quad a device pixel column, as tall as the segments near it reach,
// whose fragments take the greatest exact box-filtered coverage of those
// segments (`line_geom.wgsl`), so joins and overlaps never darken, as
// Chrome fills one path, and no pixel the line does not touch is shaded.
// Areas go into the chunk's coverage texture (red, added: their pieces,
// the trapezoids from each segment down to its floor, tile the area), one
// instance a segment, composited once under the line.

@group(1) @binding(0) var<uniform> style: Style;
@group(1) @binding(1) var<storage, read> points: array<vec4<u32>>;
@group(1) @binding(2) var<storage, read> offsets: array<u32>;
// Per snapshot index: the segment's two controls as data deltas, from it
// and from the next point.
@group(1) @binding(3) var<storage, read> ctrl: array<vec4<f32>>;
@group(1) @binding(4) var<storage, read> fctrl: array<vec4<f32>>;
@group(1) @binding(5) var<storage, read> segs: array<Seg>;

struct StripVarying {
    @builtin(position) position: vec4<f32>,
    // The segments to look at: from, to.
    @location(0) @interpolate(flat) range: vec2<u32>,
}

// The segments whose coverage may reach texture pixel column `x`.
fn near(x: f32) -> vec2<u32> {
    let e = i32(x - style.first_col) + 1;
    let r = i32(style.reach);
    let last = i32(style.n) - 1;
    let lo = u32(clamp(e - r, 0, last));
    let hi = u32(clamp(e + r, 0, last));
    let k0 = max(offsets[lo], 1u) - 1u;
    let k1 = offsets[hi + 1u];
    let sub = max(style.sub, 1u);
    return vec2<u32>(k0 * sub, min(k1 * sub, style.cap));
}

@vertex
fn vs_strip(@builtin(vertex_index) vertex: u32, @builtin(instance_index) i: u32) -> StripVarying {
    var out: StripVarying;
    out.position = vec4<f32>(-2.0, -2.0, 0.0, 1.0);
    out.range = vec2<u32>(0u);
    let x = style.first_col - f32(style.reach) + f32(i);
    if (x < 0.0 || x >= chunk_item.size.x) {
        return out;
    }
    let range = near(x);
    var y0 = 1e30;
    var y1 = -1e30;
    // The first and last segment the column meets: its fragments look at
    // those and what lies between, not at all that may reach it.
    var k0 = range.y;
    var k1 = range.x;
    for (var k = range.x; k < range.y; k += 1u) {
        let b = segs[k].bx;
        if (b.x <= x + 1.0 && b.z >= x && b.y <= b.w) {
            y0 = min(y0, b.y);
            y1 = max(y1, b.w);
            k0 = min(k0, k);
            k1 = k + 1u;
        }
    }
    y0 = max(floor(y0), 0.0);
    y1 = min(ceil(y1), chunk_item.size.y);
    if (y1 <= y0) {
        return out;
    }
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    out.position = chunk_position(vec2<f32>(x + unit.x, mix(y0, y1, unit.y)));
    out.range = vec2<u32>(k0, k1);
    return out;
}

@fragment
fn fs_strip(input: StripVarying) -> @location(0) vec4<f32> {
    let p = chunk_local(input.position.xy);
    var cov = 0.0;
    for (var k = input.range.x; k < input.range.y; k += 1u) {
        let b = segs[k].bx;
        if (p.x + 0.5 < b.x || p.x - 0.5 > b.z || p.y + 0.5 < b.y || p.y - 0.5 > b.w) {
            continue;
        }
        cov = max(cov, seg_cover(segs[k], p));
        if (cov >= 1.0) {
            break;
        }
    }
    return premul(chunk_item.line) * clamp(cov, 0.0, 1.0) * chunk_fade(input.position.xy);
}

struct AreaVarying {
    @builtin(position) position: vec4<f32>,
    // The top's ends and the floor's ends, x shared.
    @location(0) @interpolate(flat) top: vec4<f32>,
    @location(1) @interpolate(flat) bottom: vec2<f32>,
}

@vertex
fn vs_area(@builtin(vertex_index) vertex: u32, @builtin(instance_index) i: u32) -> AreaVarying {
    var out: AreaVarying;
    out.position = vec4<f32>(-2.0, -2.0, 0.0, 1.0);
    let sub = max(style.sub, 1u);
    let k = i / sub;
    let j = i % sub;
    if (!connected(k)) {
        return out;
    }
    let c = curved(k);
    if (!c && j > 0u) {
        return out;
    }
    var n = 1u;
    if (c) {
        n = sub;
    }
    let fs = f32(n);
    var a = at_t(k, f32(j) / fs, false);
    var b = at_t(k, f32(j + 1u) / fs, false);
    var fa = at_t(k, f32(j) / fs, true);
    var fb = at_t(k, f32(j + 1u) / fs, true);
    if (b.x < a.x) {
        let t = a;
        a = b;
        b = t;
        let u = fa;
        fa = fb;
        fb = u;
    }
    if (b.x - a.x < 1e-6) {
        return out;
    }
    let y0 = min(min(a.y, b.y), min(fa.y, fb.y)) - 1.0;
    let y1 = max(max(a.y, b.y), max(fa.y, fb.y)) + 1.0;
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    let at = vec2<f32>(mix(floor(a.x), ceil(b.x), unit.x), mix(y0, y1, unit.y));
    out.position = chunk_position(at);
    out.top = vec4<f32>(a, b);
    out.bottom = vec2<f32>(fa.y, fb.y);
    return out;
}

@fragment
fn fs_area(input: AreaVarying) -> @location(0) vec4<f32> {
    let p = chunk_local(input.position.xy);
    let a = input.top.xy;
    let b = input.top.zw;
    let x0 = max(a.x, p.x - 0.5);
    let x1 = min(b.x, p.x + 0.5);
    if (x1 <= x0) {
        return vec4<f32>(0.0);
    }
    let mt = (b.y - a.y) / (b.x - a.x);
    let kt = a.y - mt * a.x;
    let mb = (input.bottom.y - input.bottom.x) / (b.x - a.x);
    let kb = input.bottom.x - mb * a.x;
    let c = p.y + 0.5;
    // The row's part between the line and the floor: what lies below the
    // upper of them less what lies below the lower.
    let cov = abs(area_below(x0, x1, mt, kt, c) - area_below(x0, x1, mb, kb, c));
    return vec4<f32>(clamp(cov, 0.0, 1.0), 0.0, 0.0, 0.0);
}
