// What every pipeline of the renderer shares: signed distances and area
// coverage of rounded rectangles, and colour (sRGB, OKLab, premultiplying).
// Prepended to each module by `gpu::contract`.

const M_PI_F: f32 = 3.1415926;

fn pick_corner(center_to_point: vec2<f32>, radii: vec4<f32>) -> f32 {
    // radii: top left, top right, bottom right, bottom left.
    if (center_to_point.x < 0.0) {
        if (center_to_point.y < 0.0) {
            return radii.x;
        }
        return radii.w;
    }
    if (center_to_point.y < 0.0) {
        return radii.y;
    }
    return radii.z;
}

// Signed distance from `p` to a rounded rectangle, negative inside. The
// radii are CSS's used values (`shape::Shape::used`): the two on a side
// never sum past it, but one may pass half of it when its neighbour is
// small, so beyond the common case each corner is tested in its own square.
fn rrect_sdf(p: vec2<f32>, rect: vec4<f32>, radii: vec4<f32>) -> f32 {
    let half_size = rect.zw / 2.0;
    let center = rect.xy + half_size;
    let center_to_point = p - center;
    let corner_to_point = abs(center_to_point) - half_size;
    let most = max(max(radii.x, radii.y), max(radii.z, radii.w));
    if (most <= min(half_size.x, half_size.y)) {
        // Every corner lies in its own quadrant: the quadrant's decides.
        let r = pick_corner(center_to_point, radii);
        let q = corner_to_point + r;
        if (r <= 0.0) {
            return max(q.x, q.y);
        }
        return length(max(vec2<f32>(0.0), q)) + min(0.0, max(q.x, q.y)) - r;
    }
    return shape_sdf(p, rect, radii, 1.0);
}

// The curve exponent of CSS's `superellipse(k)`, `|k|` bounded by
// `shape::CURVE_LIMIT` (6) as hit testing and paths bound it.
fn curve_n(k: f32) -> f32 {
    return exp2(min(abs(k), 6.0));
}

// Signed distance (approximate off the curve, exact on it) from a corner's
// curve, `q` the point's distance into the box from the corner along each
// axis, `r` its radius, `k` CSS's superellipse parameter (1 round, 2
// squircle, 0 bevel, below 0 concave).
fn corner_sdf(q: vec2<f32>, r: f32, k: f32) -> f32 {
    if (k == 1.0) {
        return length(vec2<f32>(r) - q) - r;
    }
    let n = curve_n(k);
    if (k > 0.0) {
        // Centred on the corner's inner point: outside where the norm passes 1.
        let a = max((vec2<f32>(r) - q) / r, vec2<f32>(1e-6));
        let g = max(pow(pow(a.x, n) + pow(a.y, n), 1.0 / n), 1e-6);
        let grad = length(vec2<f32>(pow(a.x, n - 1.0), pow(a.y, n - 1.0))) / pow(g, n - 1.0);
        return r * (g - 1.0) / max(grad, 1e-6);
    }
    // Concave: centred on the corner itself, inside where the norm passes 1.
    let a = max(q / r, vec2<f32>(1e-6));
    let g = max(pow(pow(a.x, n) + pow(a.y, n), 1.0 / n), 1e-6);
    let grad = length(vec2<f32>(pow(a.x, n - 1.0), pow(a.y, n - 1.0))) / pow(g, n - 1.0);
    return r * (1.0 - g) / max(grad, 1e-6);
}

// Signed distance from `p` to a box whose corners of used radii `radii`
// curve as `superellipse(k)`: the box's own, cut by each corner's curve
// within that corner's square.
fn shape_sdf(p: vec2<f32>, rect: vec4<f32>, radii: vec4<f32>, k: f32) -> f32 {
    let half_size = rect.zw / 2.0;
    let d = abs(p - (rect.xy + half_size)) - half_size;
    var sdf = length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
    let lo = rect.xy;
    let hi = rect.xy + rect.zw;
    // Each corner: the point's distance into the box from it.
    let qs = array<vec2<f32>, 4>(
        p - lo,
        vec2<f32>(hi.x - p.x, p.y - lo.y),
        hi - p,
        vec2<f32>(p.x - lo.x, hi.y - p.y),
    );
    let rs = array<f32, 4>(radii.x, radii.y, radii.z, radii.w);
    for (var i = 0u; i < 4u; i = i + 1u) {
        let r = rs[i];
        let q = qs[i];
        if (r > 0.0 && q.x < r && q.y < r) {
            sdf = max(sdf, corner_sdf(q, r, k));
        }
    }
    return sdf;
}

// How much of the pixel centred on `p` a box with corners `superellipse(k)` covers.
fn shape_cov(p: vec2<f32>, rect: vec4<f32>, radii: vec4<f32>, k: f32) -> f32 {
    if (k == 1.0) {
        return rrect_cov(p, rect, radii);
    }
    if (rect.z <= 0.0 || rect.w <= 0.0) {
        return 0.0;
    }
    return saturate(0.5 - shape_sdf(p, rect, radii, k));
}

// How much of the pixel centred on `p` a rounded rectangle covers.
fn rrect_cov(p: vec2<f32>, rect: vec4<f32>, radii: vec4<f32>) -> f32 {
    if (rect.z <= 0.0 || rect.w <= 0.0) {
        return 0.0;
    }
    let corners = max(max(radii.x, radii.y), max(radii.z, radii.w));
    if (corners <= 0.0) {
        // An axis-aligned rectangle covers the pixel's area exactly.
        let lo = clamp(p - 0.5, rect.xy, rect.xy + rect.zw);
        let hi = clamp(p + 0.5, rect.xy, rect.xy + rect.zw);
        let size = max(hi - lo, vec2<f32>(0.0));
        return size.x * size.y;
    }
    return saturate(0.5 - rrect_sdf(p, rect, radii));
}

// Signed distance from `p` to a box of corners `superellipse(k)` (any `k`).
fn curved_sdf(p: vec2<f32>, rect: vec4<f32>, radii: vec4<f32>, k: f32) -> f32 {
    if (k == 1.0) {
        return rrect_sdf(p, rect, radii);
    }
    return shape_sdf(p, rect, radii, k);
}

// Vec4s a clip entry takes (`gpu::clips::ENTRY`): its rectangle, its
// radii, the map from the space it is read in (the canvas, or a path's or
// curve glyph's target) into its own (a, b, c, d), (e, f, its scale, its
// corners' curve), then (the next clip of its effective chain, its depth,
// its parent, 0). Each shader walks an entry's chain to its end (0), or, in a
// layer, to its first clip as shallow as the layer's output clip
// (`Globals::stop`); a chain is an ancestor list, so the walk ends, and
// the table's length bounds it.
// (`PCLIP` itself is prepended from Rust: `gpu/contract.rs`.)

// Point `p` through an entry's map (a, b, c, d) and (e, f).
fn entry_point(m: vec4<f32>, t: vec4<f32>, p: vec2<f32>) -> vec2<f32> {
    return mat2x2<f32>(m.xy, m.zw) * p + t.xy;
}

// A clip entry's map (`m`: a, b, c, d; `t`: e, f, scale, curve) as a
// target whose space `tl`, `tt` map to the canvas sees it: from the
// target's pixels into the clip's space, and (`own`, the target's pixels
// not the canvas's) its scale out of that space into them, so coverage is
// measured in the pixels drawn (`ClipTree::entry_in` writes the same).
struct Seen {
    m: vec4<f32>,
    t: vec4<f32>,
}

fn entry_seen(m: vec4<f32>, t: vec4<f32>, tl: mat2x2<f32>, tt: vec2<f32>, own: bool) -> Seen {
    let el = mat2x2<f32>(m.xy, m.zw);
    let lin = el * tl;
    let off = el * tt + t.xy;
    var scale = t.z;
    if (own) {
        // The larger scale out of the clip's space: the inverse of the
        // smaller one into it.
        let a = lin[0];
        let b = lin[1];
        let sum = dot(a, a) + dot(b, b);
        let det = abs(a.x * b.y - a.y * b.x);
        let most = sqrt((sum + sqrt(max(sum * sum - 4.0 * det * det, 0.0))) / 2.0);
        scale = most / max(det, 1e-12);
    }
    return Seen(vec4<f32>(lin[0], lin[1]), vec4<f32>(off, scale, t.w));
}

// How much of the pixel at point `p` (the space the entry is read in) a
// clip entry keeps.
fn clip_entry_cov(rect: vec4<f32>, radii: vec4<f32>, m: vec4<f32>, t: vec4<f32>, p: vec2<f32>) -> f32 {
    // A clip of no width or height keeps nothing, by area or by distance.
    if (rect.z <= 0.0 || rect.w <= 0.0) {
        return 0.0;
    }
    let q = entry_point(m, t, p);
    if (t.z == 1.0 && m.y == 0.0 && m.z == 0.0) {
        return shape_cov(q, rect, radii, t.w);
    }
    // In a scaled or turned space: by distance, one device pixel wide.
    return saturate(0.5 - curved_sdf(q, rect, radii, t.w) * t.z);
}

// Whether the quad of corners `a`, `b`, `c`, `d` (the space the entry is
// read in) lies wholly where the entry keeps every pixel: inside its
// rectangle less its corners and half a device pixel of its edges. A
// draw wholly so inside each clip of its chain tests none per fragment.
// The rule `crate::clip::holds` leaves an ancestor out of a chain by.
fn entry_holds(
    rect: vec4<f32>,
    radii: vec4<f32>,
    m: vec4<f32>,
    t: vec4<f32>,
    a: vec2<f32>,
    b: vec2<f32>,
    c: vec2<f32>,
    d: vec2<f32>,
) -> bool {
    let r = max(max(max(radii.x, radii.y), max(radii.z, radii.w)), 0.5 / max(t.z, 1e-6));
    let lo = rect.xy + vec2<f32>(r);
    let hi = rect.xy + rect.zw - vec2<f32>(r);
    let qa = entry_point(m, t, a);
    let qb = entry_point(m, t, b);
    let qc = entry_point(m, t, c);
    let qd = entry_point(m, t, d);
    let least = min(min(qa, qb), min(qc, qd));
    let most = max(max(qa, qb), max(qc, qd));
    return all(least >= lo) && all(most <= hi);
}

fn premul(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

fn to_linear(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn to_gamma(linear: f32) -> f32 {
    let c = clamp(linear, 0.0, 1.0);
    if (c <= 0.0031308) {
        return 12.92 * c;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

fn cbrt(x: f32) -> f32 {
    return sign(x) * pow(abs(x), 1.0 / 3.0);
}

fn to_oklab(c: vec3<f32>) -> vec3<f32> {
    let r = to_linear(c.r);
    let g = to_linear(c.g);
    let b = to_linear(c.b);
    let l = cbrt(0.41222146 * r + 0.53633255 * g + 0.051445995 * b);
    let m = cbrt(0.2119035 * r + 0.6806995 * g + 0.10739696 * b);
    let s = cbrt(0.08830246 * r + 0.28171885 * g + 0.6299787 * b);
    return vec3<f32>(
        0.21045426 * l + 0.7936178 * m - 0.004072047 * s,
        1.9779985 * l - 2.4285922 * m + 0.4505937 * s,
        0.025904037 * l + 0.78277177 * m - 0.80867577 * s,
    );
}

fn from_oklab(lab: vec3<f32>) -> vec3<f32> {
    let l_ = lab.x + 0.39633778 * lab.y + 0.21580376 * lab.z;
    let m_ = lab.x - 0.105561346 * lab.y - 0.06385417 * lab.z;
    let s_ = lab.x - 0.08948418 * lab.y - 1.2914855 * lab.z;
    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;
    return vec3<f32>(
        to_gamma(4.0767417 * l3 - 3.3077116 * m3 + 0.23096994 * s3),
        to_gamma(-1.268438 * l3 + 2.6097574 * m3 - 0.34131938 * s3),
        to_gamma(-0.0041960863 * l3 - 0.7034186 * m3 + 1.7076147 * s3),
    );
}

// `a` to `b` by `t` in OKLab with premultiplied alpha (`Color::mix_oklab`).
fn mix_oklab(a: vec4<f32>, b: vec4<f32>, t: f32) -> vec4<f32> {
    let alpha = a.a + (b.a - a.a) * t;
    if (alpha <= 0.0) {
        return vec4<f32>(0.0);
    }
    let lab_a = to_oklab(a.rgb);
    let lab_b = to_oklab(b.rgb);
    let lab = (lab_a * a.a + (lab_b * b.a - lab_a * a.a) * t) / alpha;
    return vec4<f32>(from_oklab(lab), alpha);
}
