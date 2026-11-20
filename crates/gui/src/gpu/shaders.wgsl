// Every primitive of a scene, drawn by one pipeline: each instance is a
// quad over the pixels it may touch, and the fragment shader computes its
// coverage and colour from the primitive's fields, as GPUI draws quads and
// shadows with signed distances instead of rasterised paths. One pipeline
// keeps the scene's painting order with one instanced draw per run of
// primitives that share textures.
//
// Positions are physical window pixels. Colours in are straight alpha;
// colours out are premultiplied, blended `One, OneMinusSrcAlpha` in the
// target's sRGB-encoded values, as tiny-skia and browsers blend.

struct Globals {
    // The target's size in pixels, and where its top left sits in the window.
    size: vec2<f32>,
    origin: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
// PRIM vec4s per primitive (see `gpu::Prim`).
@group(0) @binding(1) var<storage, read> prims: array<vec4<f32>>;
// Two vec4s per gradient stop: (offset, 0, 0, 0), then the straight colour.
@group(0) @binding(2) var<storage, read> stops: array<vec4<f32>>;
@group(0) @binding(3) var mono_atlas: texture_2d<f32>;
@group(0) @binding(4) var color_atlas: texture_2d<f32>;
@group(0) @binding(5) var linear_sampler: sampler;
// An image, a rasterised shape or a layer, for the primitives that need one.
@group(1) @binding(0) var extra: texture_2d<f32>;

const PRIM: u32 = 16u;

const KIND_SOLID: u32 = 0u;
const KIND_SHADOW: u32 = 1u;
const KIND_GRADIENT: u32 = 2u;
const KIND_MONO: u32 = 3u;
const KIND_COLOR: u32 = 4u;
const KIND_IMAGE: u32 = 5u;
const KIND_RASTER: u32 = 6u;
const KIND_LAYER: u32 = 7u;
const KIND_TEXT_THROUGH: u32 = 8u;

const M_PI_F: f32 = 3.1415926;

fn field(id: u32, i: u32) -> vec4<f32> {
    return prims[id * PRIM + i];
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) id: u32,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) id: u32) -> Varying {
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    let quad = field(id, 0u);
    let at = quad.xy + unit * quad.zw - globals.origin;
    let device = at / globals.size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    var out: Varying;
    out.position = vec4<f32>(device, 0.0, 1.0);
    out.id = id;
    return out;
}

// ---- shapes ----

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

// Signed distance from `p` to a rounded rectangle, negative inside.
fn rrect_sdf(p: vec2<f32>, rect: vec4<f32>, radii: vec4<f32>) -> f32 {
    let half_size = rect.zw / 2.0;
    let center = rect.xy + half_size;
    let center_to_point = p - center;
    let r = min(pick_corner(center_to_point, radii), min(half_size.x, half_size.y));
    let corner_to_point = abs(center_to_point) - half_size;
    let q = corner_to_point + r;
    if (r <= 0.0) {
        return max(q.x, q.y);
    }
    return length(max(vec2<f32>(0.0), q)) + min(0.0, max(q.x, q.y)) - r;
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

fn clip_cov(id: u32, p: vec2<f32>) -> f32 {
    let clip = field(id, 7u);
    if (clip.z < 0.0) {
        return 1.0;
    }
    return rrect_cov(p, clip, field(id, 8u));
}

// ---- shadows: Raph Levien's blurred rounded rectangle, as GPUI draws them ----

fn erf7(input: f32) -> f32 {
    let x = clamp(input, -10.0, 10.0) * (2.0 / sqrt(M_PI_F));
    let xx = x * x;
    let y = x + (0.24295 + (0.03395 + 0.0104 * xx) * xx) * (x * xx);
    return y / sqrt(1.0 + y * y);
}

fn blurred_rrect(p: vec2<f32>, rect: vec4<f32>, radii_in: vec4<f32>, sigma: f32) -> f32 {
    let size = rect.zw;
    let s = max(sigma * sqrt(2.0), 1e-6);
    let s_inv = 1.0 / s;
    let min_edge = min(size.x, size.y);
    let r_max = 0.5 * min_edge;
    let radii = min(radii_in, vec4<f32>(r_max));
    let r0 = min(sqrt(radii * radii + 1.15 * 1.15 * s * s), vec4<f32>(r_max));
    let r1v = min(sqrt(radii * radii + 4.0 * s * s), vec4<f32>(r_max));
    let half_extent = 0.5 * s_inv * size;
    let delta = 1.25 * s * (exp(-half_extent.x * half_extent.x) - exp(-half_extent.y * half_extent.y));
    let adjusted = vec2<f32>(size.x + min(delta, 0.0), size.y - max(delta, 0.0));
    let peak = s_inv * 0.5 * (max(adjusted.x, adjusted.y) - 0.5 * radii);
    let scalev = 0.5 * vec4<f32>(erf7(peak.x), erf7(peak.y), erf7(peak.z), erf7(peak.w));
    let exponentv = 2.0 * r1v / r0;

    let center_to_point = p - (rect.xy + size / 2.0);
    let r1 = pick_corner(center_to_point, r1v);
    let exponent = pick_corner(center_to_point, exponentv);
    let scale = pick_corner(center_to_point, scalev);
    let p0 = abs(center_to_point) - (0.5 * adjusted - r1);
    let p1 = max(p0, vec2<f32>(0.0));
    var d_pos: f32;
    if (p1.x > 0.0 && p1.y > 0.0) {
        d_pos = pow(pow(p1.x, exponent) + pow(p1.y, exponent), 1.0 / exponent);
    } else {
        d_pos = p1.x + p1.y;
    }
    let d_neg = min(max(p0.x, p0.y), 0.0);
    let d = d_pos + d_neg - r1;
    return scale * (erf7(s_inv * (min_edge + d)) - erf7(s_inv * d));
}

// ---- colour ----

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

// The gradient of primitive `id` at `p`, straight alpha.
fn gradient_at(id: u32, p: vec2<f32>) -> vec4<f32> {
    return gradient_at_line(id, p, field(id, 10u));
}

fn filtered(px: vec4<f32>, kind: f32, amount: f32) -> vec4<f32> {
    // On premultiplied channels, clamped to alpha, as the CPU renderer filters.
    var rgb = px.rgb;
    if (kind == 1.0) {
        rgb = rgb * amount;
    } else if (kind == 2.0) {
        let lum = 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b;
        rgb = rgb + (vec3<f32>(lum) - rgb) * amount;
    } else {
        return px;
    }
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(px.a)), px.a);
}

@fragment
fn fs_main(input: Varying) -> @location(0) vec4<f32> {
    let id = input.id;
    let p = input.position.xy + globals.origin;
    let head = field(id, 1u);
    let kind = u32(head.x);
    let color = field(id, 2u);
    let a = field(id, 3u);
    let a_radii = field(id, 4u);
    let b = field(id, 5u);
    let b_radii = field(id, 6u);
    var out = vec4<f32>(0.0);
    switch kind {
        case KIND_SOLID: {
            var cov = rrect_cov(p, a, a_radii);
            if (b.z > 0.0 && b.w > 0.0) {
                cov = max(cov - rrect_cov(p, b, b_radii), 0.0);
            }
            out = premul(color) * cov;
        }
        case KIND_SHADOW: {
            let sigma = field(id, 9u).x;
            var alpha: f32;
            if (sigma <= 0.0) {
                alpha = rrect_cov(p, a, a_radii);
            } else {
                alpha = blurred_rrect(p, a, a_radii, sigma);
            }
            // CSS draws a shadow only outside the box casting it.
            alpha = alpha * (1.0 - rrect_cov(p, b, b_radii));
            out = premul(color) * alpha;
        }
        case KIND_GRADIENT: {
            out = premul(gradient_at(id, p)) * rrect_cov(p, a, a_radii);
        }
        case KIND_MONO: {
            let uv = field(id, 9u);
            let texel = vec2<i32>(uv.xy + floor(p - a.xy));
            let coverage = textureLoad(mono_atlas, texel, 0).r;
            out = premul(color) * coverage;
        }
        case KIND_COLOR: {
            let uv = field(id, 9u);
            let texel = vec2<i32>(uv.xy + floor(p - a.xy));
            out = textureLoad(color_atlas, texel, 0) * color.a;
        }
        case KIND_IMAGE: {
            // field 9: where the whole image lands, covering the box.
            let placed = field(id, 9u);
            let uv = (p - placed.xy) / placed.zw;
            out = textureSampleLevel(extra, linear_sampler, uv, 0.0) * rrect_cov(p, a, a_radii);
        }
        case KIND_RASTER: {
            let texel = vec2<i32>(floor(p - a.xy));
            out = textureLoad(extra, texel, 0);
        }
        case KIND_LAYER: {
            // field 9, 10.xy: the inverse of the group's transform; 11: two
            // filters; 12: the mask's line; 1.z, 1.w: the mask's stops.
            let m = field(id, 9u);
            let t = field(id, 10u);
            let q = vec2<f32>(m.x * p.x + m.z * p.y + t.x, m.y * p.x + m.w * p.y + t.y);
            let inside = rrect_cov(q, a, vec4<f32>(0.0));
            let size = vec2<f32>(textureDimensions(extra));
            var px = textureSampleLevel(extra, linear_sampler, (q - a.xy) / size, 0.0) * inside;
            if (head.w > 0.0) {
                px = px * gradient_at_line(id, q, field(id, 12u)).a;
            }
            let filters = field(id, 11u);
            px = filtered(px, filters.x, filters.y);
            px = filtered(px, filters.z, filters.w);
            out = px * color.a;
        }
        case KIND_TEXT_THROUGH: {
            // a: the region; b: the node's background, in `color`; 9: where
            // the gradient shows; 1.y: 1 with a background, 2 with a gradient.
            let flags = u32(head.y);
            var fill = vec4<f32>(0.0);
            if ((flags & 1u) != 0u) {
                fill = premul(color) * rrect_cov(p, b, vec4<f32>(0.0));
            }
            if ((flags & 2u) != 0u) {
                let g = premul(gradient_at(id, p)) * rrect_cov(p, field(id, 9u), vec4<f32>(0.0));
                fill = g + fill * (1.0 - g.a);
            }
            let texel = vec2<i32>(floor(p - a.xy));
            out = fill * textureLoad(extra, texel, 0).a;
        }
        default: {}
    }
    return out * clip_cov(id, p);
}

// `gradient_at` along another line (a group's mask, placed in its own field).
fn gradient_at_line(id: u32, p: vec2<f32>, line: vec4<f32>) -> vec4<f32> {
    let head = field(id, 1u);
    let first = u32(head.z);
    let count = u32(head.w);
    let d = line.zw - line.xy;
    let t = clamp(dot(p - line.xy, d) / max(dot(d, d), 1e-6), 0.0, 1.0);
    var color = stops[(first + count - 1u) * 2u + 1u];
    for (var i = 0u; i + 1u < count; i += 1u) {
        let o0 = stops[(first + i) * 2u].x;
        let o1 = stops[(first + i + 1u) * 2u].x;
        if (t <= o1) {
            let c0 = stops[(first + i) * 2u + 1u];
            let c1 = stops[(first + i + 1u) * 2u + 1u];
            if (o1 - o0 <= 1e-6) {
                color = select(c0, c1, t >= o1);
            } else {
                color = mix_oklab(c0, c1, saturate((t - o0) / (o1 - o0)));
            }
            if (t < o0) {
                color = c0;
            }
            break;
        }
    }
    return color;
}
