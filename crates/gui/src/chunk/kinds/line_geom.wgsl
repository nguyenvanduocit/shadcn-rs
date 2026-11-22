// The geometry of a line's segments (ADR-0018), shared by the pass that
// lays them out (`line_segments.wgsl`) and the draws (`lines.wgsl`). The
// including module binds `style`, `points`, `offsets`, `ctrl` and `fctrl`.
//
// A segment is its two points' strip, cut at each end by its join's
// bisector (the next segment starts there, so a miter join is where their
// outer edges meet) or by a butt cap; a bevel join adds the plane through
// both outer corners, a round join a disc. Two points connect when they
// share a column (M4 keeps a column's points in order) or follow each
// other in the series: a gap between them (NaN, a missing point) breaks
// the line.

struct Style {
    // Extended columns: the total is offsets[n].
    n: u32,
    // Pieces a curved segment is drawn in (1: straight).
    sub: u32,
    // 0 miter, 1 bevel, 2 round.
    join: u32,
    // Natural curve controls are bound.
    curve: u32,
    hw: f32,
    limit: f32,
    kx: f32,
    ky: f32,
    fcurve: u32,
    // The texture pixel column of extended column 1 (the view's first).
    first_col: f32,
    // Columns a segment's coverage may reach past its points' own.
    reach: u32,
    // Segments the buffer holds.
    cap: u32,
    // The dash pattern (device pixels: on and off lengths in turn, `count`
    // of them), its period and where a run starts in it; `dashed` 0: solid.
    dash: array<vec4<f32>, 2>,
    period: f32,
    phase: f32,
    dashed: u32,
    count: u32,
}

fn total() -> u32 {
    return offsets[style.n];
}

fn pt(k: u32) -> vec3<f32> {
    let p = points[k * 2u];
    return vec3<f32>(bitcast<f32>(p.x), bitcast<f32>(p.y), bitcast<f32>(p.z));
}

fn idx(k: u32) -> u32 {
    return points[k * 2u].w;
}

fn col(k: u32) -> u32 {
    return points[k * 2u + 1u].x;
}

// Point k runs to point k + 1.
fn connected(k: u32) -> bool {
    if (k + 1u >= total()) {
        return false;
    }
    return col(k) == col(k + 1u) || idx(k + 1u) == idx(k) + 1u;
}

fn curved(k: u32) -> bool {
    return style.curve != 0u && style.sub > 1u && idx(k + 1u) == idx(k) + 1u;
}

fn bez(a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, d: vec2<f32>, t: f32) -> vec2<f32> {
    let u = 1.0 - t;
    return u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
}

// Segment k at t, on its line (y) or its floor (z).
fn at_t(k: u32, t: f32, floor: bool) -> vec2<f32> {
    let a = pt(k);
    let b = pt(k + 1u);
    var ya = a.y;
    var yb = b.y;
    if (floor) {
        ya = a.z;
        yb = b.z;
    }
    let p0 = vec2<f32>(a.x, ya);
    let p3 = vec2<f32>(b.x, yb);
    if (!curved(k)) {
        return mix(p0, p3, t);
    }
    var c = ctrl[idx(k)];
    if (floor) {
        if (style.fcurve == 0u) {
            return mix(p0, p3, t);
        }
        c = fctrl[idx(k)];
    }
    let s = vec2<f32>(style.kx, -style.ky);
    return bez(p0, p0 + c.xy * s, p3 + c.zw * s, p3, t);
}

fn perp(d: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(-d.y, d.x);
}

// The end at `p` of a segment going `d` (away from the end), its
// neighbour going `d0` into it (`has` false: a cap). Returns the end
// plane, its bevel, and how far past `p` the quad must reach.
struct End {
    plane: vec4<f32>,
    bevel: vec4<f32>,
    reach: f32,
}

fn end_of(p: vec2<f32>, d: vec2<f32>, d0: vec2<f32>, has: bool) -> End {
    var e: End;
    let hw = style.hw;
    e.bevel = vec4<f32>(0.0);
    if (!has) {
        // Butt: inside where dot(q - p, d) >= 0.
        e.plane = vec4<f32>(-d, -dot(p, d), 0.0);
        e.reach = 1.0;
        return e;
    }
    if (style.join == 2u) {
        e.plane = vec4<f32>(-d, -dot(p, d), 2.0);
        e.reach = hw + 1.0;
        return e;
    }
    let m = normalize(d0 + d);
    if (dot(d0, d) < -0.9999 || any(m != m)) {
        // Turning back: a bevel at the point.
        e.plane = vec4<f32>(-d, -dot(p, d), 1.0);
        e.reach = hw + 1.0;
        return e;
    }
    e.plane = vec4<f32>(-m, -dot(p, m), 1.0);
    let cos_half = max(dot(m, d), 1e-4);
    let ratio = 1.0 / cos_half;
    if (style.join == 1u || ratio > style.limit) {
        let s = sign(d0.x * d.y - d0.y * d.x);
        let o = normalize(-s * perp(d0) - s * perp(d));
        e.bevel = vec4<f32>(o, dot(p, o) + hw * dot(o, -s * perp(d)), 1.0);
        e.reach = hw + 1.0;
    } else {
        e.reach = hw * ratio + 1.0;
    }
    return e;
}


// A segment as laid out once a frame (`segments`): its ends, its end
// planes and bevels, the box its coverage lies in (empty: hidden), and
// where along the line it starts (device pixels from its run's start) and
// how long it is.
struct Seg {
    ab: vec4<f32>,
    ea: vec4<f32>,
    eb: vec4<f32>,
    ba: vec4<f32>,
    bb: vec4<f32>,
    bx: vec4<f32>,
    arc: vec4<f32>,
}

// How much of the pixel at `p` is inside plane `e` (its kind's way).
fn end_cover(p: vec2<f32>, e: vec4<f32>) -> f32 {
    let dist = e.z - dot(p, e.xy);
    if (e.w == 0.0) {
        return half_plane_cov(dist, e.xy);
    }
    if (e.w == 2.0) {
        if (dist >= 0.0) {
            return 1.0;
        }
        return -1.0;
    }
    // A join: cut at its bisector, half a pixel past it, so that the two
    // segments overlap there and the greatest is kept.
    return select(0.0, 1.0, dist >= -0.5);
}

// The exact area of the unit pixel square centred at 0 on the side of a
// line `d` from its centre (along a unit normal `n`) where the normal
// points away.
fn half_plane_cov(d: f32, n: vec2<f32>) -> f32 {
    let a = max(abs(n.x), abs(n.y));
    let b = min(abs(n.x), abs(n.y));
    let s = 0.5 * (a + b);
    if (d <= -s) {
        return 0.0;
    }
    if (d >= s) {
        return 1.0;
    }
    if (b < 1e-4) {
        return clamp(0.5 + d / max(a, 1e-6), 0.0, 1.0);
    }
    let t = d + s;
    let r = max(t, 0.0) * max(t, 0.0) - max(t - a, 0.0) * max(t - a, 0.0) - max(t - b, 0.0) * max(t - b, 0.0)
        + max(t - a - b, 0.0) * max(t - a - b, 0.0);
    return clamp(r / (2.0 * a * b), 0.0, 1.0);
}

// How much of a pixel `w` long along the line about arc length `s` the dash
// pattern keeps on: the pattern's on length in `s - w/2 .. s + w/2`.
fn dash_on(s: f32, w: f32) -> f32 {
    if (style.dashed == 0u || style.period <= 0.0) {
        return 1.0;
    }
    let lo = s - 0.5 * w;
    let hi = s + 0.5 * w;
    return clamp((dash_measure(hi) - dash_measure(lo)) / max(w, 1e-4), 0.0, 1.0);
}

fn dash_len(i: u32) -> f32 {
    return style.dash[i / 4u][i % 4u];
}

// The pattern's on length from the run's start to `s` (the phase added).
fn dash_measure(s: f32) -> f32 {
    let t = s + style.phase;
    let periods = floor(t / style.period);
    var r = t - periods * style.period;
    var on = 0.0;
    var m = 0.0;
    for (var i = 0u; i < style.count; i += 2u) {
        let a = dash_len(i);
        let b = dash_len(i + 1u);
        on += a;
        m += clamp(r, 0.0, a);
        r -= a + b;
    }
    return periods * on + m;
}

// Segment `s`'s coverage of the pixel at `p` (texture pixels).
fn seg_cover(s: Seg, p: vec2<f32>) -> f32 {
    let a = s.ab.xy;
    let b = s.ab.zw;
    let d = normalize(b - a);
    let nrm = perp(d);
    let hw = style.hw;
    let dn = dot(p - a, nrm);
    var cov = clamp(half_plane_cov(hw - dn, nrm) + half_plane_cov(hw + dn, nrm) - 1.0, 0.0, 1.0);
    let ca = end_cover(p, s.ea);
    let cb = end_cover(p, s.eb);
    // Round ends: past the end, the disc about it.
    if (ca < 0.0) {
        cov = clamp(hw + 0.5 - length(p - a), 0.0, 1.0);
    } else {
        cov *= ca;
    }
    if (cb < 0.0) {
        cov = min(cov, clamp(hw + 0.5 - length(p - b), 0.0, 1.0));
    } else {
        cov *= cb;
    }
    if (s.ba.w > 0.0) {
        cov *= half_plane_cov(s.ba.z - dot(p, s.ba.xy), s.ba.xy);
    }
    if (s.bb.w > 0.0) {
        cov *= half_plane_cov(s.bb.z - dot(p, s.bb.xy), s.bb.xy);
    }
    if (style.dashed != 0u && cov > 0.0) {
        // The pixel's width along the segment, and where it falls on it.
        let u = clamp(dot(p - a, d), 0.0, s.arc.y);
        cov *= dash_on(s.arc.x + u, abs(d.x) + abs(d.y));
    }
    return cov;
}
