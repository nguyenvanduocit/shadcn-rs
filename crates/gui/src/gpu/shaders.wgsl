// The scene's closed-form primitives (rects, rounded rects, bands, shadows,
// gradients, atlas glyphs and icons, images, layers), drawn by the quad
// pipeline: each instance is a quad over the pixels it may touch, and the
// fragment shader computes its coverage and colour from the primitive's
// record, as GPUI draws quads and shadows with signed distances. Paths and
// curve text have their own pipelines (`path_render.wgsl`, `slug.wgsl`),
// drawn in the same pass in the scene's order.
//
// Records are kept on the GPU across frames (ADR-0016): a paint chunk's
// records are written once and replayed while it is unchanged. They are
// typed and compact (`gpu::record`): a glyph is 8 words, a rounded rect 12,
// a band 20, written in the chunk's paint space. A record names its state
// slot, which names a transform (the offset its space is moved by: a
// scroll, a moving translation), a clip and an effect (the opacity folded
// into it) in the property tables, so a scroll or a fade writes one table
// entry and no record. A clip's entry is its own rounded rectangle in its
// own space, the map from the canvas into it, and the next clip of its
// effective chain (`gpu::clips`): the shaders walk the chain to its end,
// so moving an outer clip writes its entry alone, and one chain crosses
// scales and turns. A pass's target maps to the canvas by `to_canvas`.
// The vertex shader moves the quad by its offset, clamps it to the
// rectangles of the chain's clips that lie in its target's space moved,
// and drops the clip when the quad lies wholly inside each one's inner
// rectangle; only a quad across a clip's edge or corner tests the chain
// per fragment, each clip's coverage multiplied.
//
// Positions are physical pixels. Colours in are straight alpha; colours
// out are premultiplied, blended `One, OneMinusSrcAlpha` in the target's
// sRGB-encoded values, as browsers blend. With dual-source blending the
// second output is what each channel of the target keeps
// (`OneMinusSrc1`): the alpha for every primitive, and each stripe's own
// coverage for subpixel (LCD) text, so one blend state serves all.
//
// The fragment stage's entry point for primitives is appended by
// `gpu::module`, with or without dual-source blending (ADR-0019); `fs_fill`
// is the one that writes an opaque rectangle over all it covers.

struct Globals {
    // The target's size in pixels, and where its top left sits in the window.
    size: vec2<f32>,
    origin: vec2<f32>,
    // Where each property table starts in `props` (vec4s): state slots,
    // transforms, clips, effects.
    bases: vec4<u32>,
    // The map from the target's space to the canvas's: (a, b, c, d), (e, f).
    to_canvas: vec4<f32>,
    to_canvas_t: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
// Records, as words (see `gpu::record`).
@group(0) @binding(1) var<storage, read> records: array<u32>;
// One word per instance: where its record starts.
@group(0) @binding(2) var<storage, read> index: array<u32>;
// The property tables: a slot is (transform, clip, effect) ids; a
// transform its offset; a clip its entry (`PCLIP` vec4s, `common.wgsl`);
// an effect its alpha.
@group(0) @binding(3) var<storage, read> props: array<vec4<f32>>;
// Atlas pages (ADR-0016): coverage (glyphs, icons), and colour or subpixel.
@group(0) @binding(4) var mono_atlas: texture_2d_array<f32>;
@group(0) @binding(5) var color_atlas: texture_2d_array<f32>;
@group(0) @binding(6) var linear_sampler: sampler;
// Skia's `SkMaskGamma` preblend: 256 coverages by 8 luminances of the text
// colour, glyph coverage corrected as Chrome corrects it on this OS.
@group(0) @binding(7) var gamma_table: texture_2d<f32>;
// An image or a layer, for the primitives that need one.
@group(1) @binding(0) var extra: texture_2d<f32>;

const KIND_SOLID: u32 = 0u;
const KIND_SHADOW: u32 = 1u;
const KIND_GRADIENT: u32 = 2u;
const KIND_MONO: u32 = 3u;
const KIND_COLOR: u32 = 4u;
const KIND_IMAGE: u32 = 5u;
const KIND_MASK: u32 = 6u;
const KIND_LAYER: u32 = 7u;
const KIND_TEXT_THROUGH: u32 = 8u;
const KIND_LCD: u32 = 9u;
// What lies below, read back into `extra` (`gpu/read.rs`).
const KIND_READ: u32 = 10u;
const READ_BACKDROP: u32 = 0u;
const READ_DARKEN: u32 = 1u;
const READ_LIGHTEN: u32 = 2u;

const F_RADII: u32 = 256u;
const F_INNER: u32 = 512u;
// A solid record's corners curve as `superellipse(k)`, its last word `k`
// (a shadow's and an image's too).
const F_CURVE: u32 = 1024u;

fn word(at: u32) -> u32 {
    return records[at];
}

fn wf(at: u32) -> f32 {
    return bitcast<f32>(records[at]);
}

fn w4(at: u32) -> vec4<f32> {
    return vec4<f32>(wf(at), wf(at + 1u), wf(at + 2u), wf(at + 3u));
}

// A colour packed as four halves in two words.
fn wcolor(at: u32) -> vec4<f32> {
    return vec4<f32>(unpack2x16float(records[at]), unpack2x16float(records[at + 1u]));
}

fn is_glyph(kind: u32) -> bool {
    return kind == KIND_MONO || kind == KIND_COLOR || kind == KIND_MASK || kind == KIND_LCD;
}

fn outset(r: vec4<f32>, d: f32) -> vec4<f32> {
    return vec4<f32>(r.xy - d, r.zw + 2.0 * d);
}

// The pixels record `at` may touch, in its space.
fn quad_of(at: u32, kind: u32) -> vec4<f32> {
    if (is_glyph(kind)) {
        let wh = word(at + 6u);
        return vec4<f32>(wf(at + 4u), wf(at + 5u), f32(wh & 0xffffu), f32(wh >> 16u));
    }
    switch kind {
        case KIND_SHADOW: {
            return outset(w4(at + 4u), wf(at + 20u) * 3.0 + 1.0);
        }
        case KIND_TEXT_THROUGH, KIND_LAYER: {
            return w4(at + 4u);
        }
        default: {
            return outset(w4(at + 4u), 1.0);
        }
    }
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) at: u32,
    // The offset that took the record's space to the window.
    @location(1) @interpolate(flat) offset: vec2<f32>,
    // The opacity of the effects folded into it.
    @location(2) @interpolate(flat) alpha: f32,
    // Its clip, 0 when the quad needs none.
    @location(3) @interpolate(flat) clip: u32,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Varying {
    let at = index[instance];
    let kind = word(at) & 0xffu;
    let slot = bitcast<vec4<u32>>(props[globals.bases.x + word(at + 1u)]);
    let offset = props[globals.bases.y + slot.x].xy;
    let alpha = props[globals.bases.w + slot.z].x;
    var quad = quad_of(at, kind);
    quad = vec4<f32>(quad.xy + offset, quad.zw);
    var clip = 0u;
    if (slot.y != 0u) {
        var lo = quad.xy;
        var hi = quad.xy + quad.zw;
        var inside = true;
        let tl = mat2x2<f32>(globals.to_canvas.xy, globals.to_canvas.zw);
        let tt = globals.to_canvas_t;
        let a = tl * lo + tt;
        let b = tl * vec2<f32>(hi.x, lo.y) + tt;
        let c = tl * vec2<f32>(lo.x, hi.y) + tt;
        let d = tl * hi + tt;
        var id = slot.y;
        let n = (globals.bases.w - globals.bases.z) / PCLIP;
        for (var k = 0u; k < n && id != 0u; k = k + 1u) {
            let at = globals.bases.z + id * PCLIP;
            let rect = props[at];
            let m = props[at + 2u];
            let t = props[at + 3u];
            inside = inside && entry_holds(rect, props[at + 1u], m, t, a, b, c, d);
            // Nothing outside it is drawn: neither is it shaded. From the
            // target into the clip's space: its map after the target's.
            let el = mat2x2<f32>(m.xy, m.zw);
            let lin = el * tl;
            let off = el * tt + t.xy;
            if (lin[0].y == 0.0 && lin[1].x == 0.0 && lin[0].x > 0.0 && lin[1].y > 0.0) {
                let scale = vec2<f32>(lin[0].x, lin[1].y);
                lo = max(lo, floor((rect.xy - off) / scale));
                hi = min(hi, ceil((rect.xy + rect.zw - off) / scale));
            }
            id = bitcast<u32>(props[at + 4u].x);
        }
        if (!inside) {
            clip = slot.y;
        }
        quad = vec4<f32>(lo, max(hi - lo, vec2<f32>(0.0)));
    }
    if (alpha <= 0.0) {
        quad = vec4<f32>(0.0);
    }
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    let p = quad.xy + unit * quad.zw - globals.origin;
    let device = p / globals.size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    var out: Varying;
    out.position = vec4<f32>(device, 0.0, 1.0);
    out.at = at;
    out.offset = offset;
    out.alpha = alpha;
    out.clip = clip;
    return out;
}

// ---- clips ----

// The coverage of clip `clip` and the rest of its chain, multiplied, at
// point `p` of the target.
fn clip_cov(clip: u32, p: vec2<f32>) -> f32 {
    var cov = 1.0;
    var id = clip;
    let q = mat2x2<f32>(globals.to_canvas.xy, globals.to_canvas.zw) * p + globals.to_canvas_t;
    let n = (globals.bases.w - globals.bases.z) / PCLIP;
    for (var k = 0u; k < n && id != 0u; k = k + 1u) {
        let at = globals.bases.z + id * PCLIP;
        cov = cov * clip_entry_cov(props[at], props[at + 1u], props[at + 2u], props[at + 3u], q);
        id = bitcast<u32>(props[at + 4u].x);
    }
    return cov;
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

// ---- text ----

// Coverage `c` corrected by row `row` of the gamma table.
fn gamma(c: f32, row: u32) -> f32 {
    return textureLoad(gamma_table, vec2<i32>(i32(round(saturate(c) * 255.0)), i32(row)), 0).r;
}

// ---- gradients: stops inline after a record, three words each (the
// offset, then the straight colour as four halves) ----

fn stop_offset(first: u32, i: u32) -> f32 {
    return wf(first + i * 3u);
}

fn stop_color(first: u32, i: u32) -> vec4<f32> {
    return wcolor(first + i * 3u + 1u);
}

// The colour of the gradient of `count` stops from word `first` along
// `line` (x0, y0, x1, y1) at `p`, padded past both ends, mixed in OKLab.
fn record_gradient(first: u32, count: u32, p: vec2<f32>, line: vec4<f32>) -> vec4<f32> {
    let d = line.zw - line.xy;
    let t = clamp(dot(p - line.xy, d) / max(dot(d, d), 1e-6), 0.0, 1.0);
    var color = stop_color(first, count - 1u);
    for (var i = 0u; i + 1u < count; i += 1u) {
        let o0 = stop_offset(first, i);
        let o1 = stop_offset(first, i + 1u);
        if (t <= o1) {
            let c0 = stop_color(first, i);
            let c1 = stop_color(first, i + 1u);
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

// A box of corners `superellipse(k)` (not round) blurred by `sigma`: the
// rectangle's blur, exact and separable, cut where the curve is by the
// blur of the edge nearest (an approximation the draft leaves open; a
// round box takes `blurred_rrect`).
fn blurred_shape(p: vec2<f32>, rect: vec4<f32>, radii: vec4<f32>, k: f32, sigma: f32) -> f32 {
    let s = 1.0 / (sigma * sqrt(2.0));
    let lo = rect.xy;
    let hi = rect.xy + rect.zw;
    let x = 0.5 * (erf7((p.x - lo.x) * s) - erf7((p.x - hi.x) * s));
    let y = 0.5 * (erf7((p.y - lo.y) * s) - erf7((p.y - hi.y) * s));
    let edge = 0.5 * (1.0 - erf7(shape_sdf(p, rect, radii, k) * s));
    return min(x * y, edge);
}

// `saturate(k)` of Filter Effects 1 on a premultiplied colour (the matrix
// is linear, so alpha need not be divided out), clamped to its alpha.
fn saturated(px: vec4<f32>, k: f32) -> vec4<f32> {
    if (k == 1.0) {
        return px;
    }
    let r = vec3<f32>(0.213 + 0.787 * k, 0.715 - 0.715 * k, 0.072 - 0.072 * k);
    let g = vec3<f32>(0.213 - 0.213 * k, 0.715 + 0.285 * k, 0.072 - 0.072 * k);
    let b = vec3<f32>(0.213 - 0.213 * k, 0.715 - 0.715 * k, 0.072 + 0.928 * k);
    let rgb = vec3<f32>(dot(r, px.rgb), dot(g, px.rgb), dot(b, px.rgb));
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(px.a)), px.a);
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

// A primitive's premultiplied colour, and what of each channel of the
// target it covers (its alpha, but per stripe for subpixel text).
struct Shaded {
    color: vec4<f32>,
    mask: vec4<f32>,
}

fn shade(input: Varying) -> Shaded {
    let at = input.at;
    // Where the pixel is in the window, and in the record's space.
    let window = input.position.xy + globals.origin;
    let p = window - input.offset;
    let head = word(at);
    let kind = head & 0xffu;
    let color = wcolor(at + 2u);
    var out = vec4<f32>(0.0);
    var lcd = vec4<f32>(-1.0);
    switch kind {
        case KIND_SOLID: {
            let a = w4(at + 4u);
            var a_radii = vec4<f32>(0.0);
            if ((head & F_RADII) != 0u) {
                a_radii = w4(at + 8u);
            }
            var k = 1.0;
            if ((head & F_CURVE) != 0u) {
                k = wf(at + select(12u, 20u, (head & F_INNER) != 0u));
            }
            var cov = shape_cov(p, a, a_radii, k);
            if ((head & F_INNER) != 0u) {
                let b = w4(at + 12u);
                if (b.z > 0.0 && b.w > 0.0) {
                    cov = max(cov - shape_cov(p, b, w4(at + 16u), k), 0.0);
                }
            }
            out = premul(color) * cov;
        }
        case KIND_SHADOW: {
            let a = w4(at + 4u);
            let a_radii = w4(at + 8u);
            let sigma = wf(at + 20u);
            var k = 1.0;
            if ((head & F_CURVE) != 0u) {
                k = wf(at + 21u);
            }
            var alpha: f32;
            if (sigma <= 0.0) {
                alpha = shape_cov(p, a, a_radii, k);
            } else if (k == 1.0) {
                alpha = blurred_rrect(p, a, a_radii, sigma);
            } else {
                alpha = blurred_shape(p, a, a_radii, k, sigma);
            }
            // CSS draws a shadow only outside the box casting it.
            alpha = alpha * (1.0 - shape_cov(p, w4(at + 12u), w4(at + 16u), k));
            out = premul(color) * alpha;
        }
        case KIND_GRADIENT: {
            let count = (head >> 8u) & 0xffu;
            let g = record_gradient(at + 16u, count, p, w4(at + 12u));
            out = premul(g) * rrect_cov(p, w4(at + 4u), w4(at + 8u));
        }
        case KIND_MONO, KIND_MASK, KIND_COLOR, KIND_LCD: {
            let page = i32((head >> 8u) & 0xffu);
            let slot = word(at + 7u);
            let origin = vec2<f32>(wf(at + 4u), wf(at + 5u));
            let texel = vec2<i32>(vec2<f32>(f32(slot & 0xffffu), f32(slot >> 16u)) + floor(p - origin));
            if (kind == KIND_MONO) {
                let row = (head >> 16u) & 0xfu;
                out = premul(color) * gamma(textureLoad(mono_atlas, texel, page, 0).r, row);
            } else if (kind == KIND_MASK) {
                // An icon's coverage, tinted.
                out = premul(color) * textureLoad(mono_atlas, texel, page, 0).r;
            } else if (kind == KIND_LCD) {
                let c = textureLoad(color_atlas, texel, page, 0).rgb;
                let rows = vec3<u32>((head >> 20u) & 0xfu, (head >> 24u) & 0xfu, (head >> 28u) & 0xfu);
                let cov = vec3<f32>(gamma(c.r, rows.x), gamma(c.g, rows.y), gamma(c.b, rows.z)) * color.a;
                let alpha = max(cov.r, max(cov.g, cov.b));
                out = vec4<f32>(color.rgb * cov, alpha);
                lcd = vec4<f32>(cov, alpha);
            } else {
                out = textureLoad(color_atlas, texel, page, 0) * color.a;
            }
        }
        case KIND_IMAGE: {
            // Where the whole image lands, covering the box.
            let placed = w4(at + 12u);
            let uv = (p - placed.xy) / placed.zw;
            var k = 1.0;
            if ((head & F_CURVE) != 0u) {
                k = wf(at + 16u);
            }
            out = textureSampleLevel(extra, linear_sampler, uv, 0.0) * shape_cov(p, w4(at + 4u), w4(at + 8u), k);
        }
        case KIND_LAYER: {
            // The group's region, the inverse of its transform, two
            // filters, and its mask's line and stops.
            let a = w4(at + 8u);
            let m = w4(at + 12u);
            let t = vec2<f32>(wf(at + 16u), wf(at + 17u));
            let q = vec2<f32>(m.x * window.x + m.z * window.y + t.x, m.y * window.x + m.w * window.y + t.y);
            let inside = rrect_cov(q, a, vec4<f32>(0.0));
            let size = vec2<f32>(textureDimensions(extra));
            var px = textureSampleLevel(extra, linear_sampler, (q - a.xy) / size, 0.0) * inside;
            if ((head & F_RADII) != 0u) {
                let count = (head >> 16u) & 0xffu;
                px = px * record_gradient(at + 26u, count, q, w4(at + 22u)).a;
            }
            let filters = w4(at + 18u);
            px = filtered(px, filters.x, filters.y);
            px = filtered(px, filters.z, filters.w);
            out = px * color.a;
        }
        case KIND_TEXT_THROUGH: {
            // The region; the node's background, in `color`; where the
            // gradient shows, its line and stops.
            let a = w4(at + 4u);
            var fill = vec4<f32>(0.0);
            if ((head & F_RADII) != 0u) {
                fill = premul(color) * rrect_cov(p, w4(at + 8u), vec4<f32>(0.0));
            }
            if ((head & F_INNER) != 0u) {
                let count = (head >> 16u) & 0xffu;
                let g = premul(record_gradient(at + 20u, count, p, w4(at + 16u)))
                    * rrect_cov(p, w4(at + 12u), vec4<f32>(0.0));
                fill = g + fill * (1.0 - g.a);
            }
            let texel = vec2<i32>(floor(p - a.xy));
            out = fill * textureLoad(extra, texel, 0).a;
        }
        case KIND_READ: {
            // Its shape (an outer box, an inner one cut out), where the
            // texture's first texel lies in the window, how many window
            // pixels a texel covers, `saturate(k)`, the curve, and the
            // texels that hold what was read.
            let k = wf(at + 24u);
            var cov = shape_cov(p, w4(at + 4u), w4(at + 8u), k);
            if ((head & F_INNER) != 0u) {
                let b = w4(at + 12u);
                if (b.z > 0.0 && b.w > 0.0) {
                    cov = max(cov - shape_cov(p, b, w4(at + 16u), k), 0.0);
                }
            }
            let origin = vec2<f32>(wf(at + 20u), wf(at + 21u));
            let level = wf(at + 22u);
            let valid = vec2<f32>(wf(at + 25u), wf(at + 26u));
            let mode = (head >> 16u) & 0xffu;
            if (mode == READ_BACKDROP) {
                // Between texel centres, never past what was read.
                let q = clamp((window - origin) / level, vec2<f32>(0.5), valid - 0.5);
                let px = textureSampleLevel(extra, linear_sampler, q / vec2<f32>(textureDimensions(extra)), 0.0);
                out = saturated(px, wf(at + 23u)) * cov;
            } else {
                // Compositing 1's separable blend: what shows is
                // `(1 - ab)·Cs + ab·B(Cb, Cs)`, source-over the backdrop.
                let texel = clamp(vec2<i32>(floor(window - origin)), vec2<i32>(0), vec2<i32>(valid) - 1);
                let below = textureLoad(extra, texel, 0);
                let ab = below.a;
                var cb = vec3<f32>(0.0);
                if (ab > 0.0) {
                    cb = below.rgb / ab;
                }
                var mixed = min(cb, color.rgb);
                if (mode == READ_LIGHTEN) {
                    mixed = max(cb, color.rgb);
                }
                let a = color.a * cov;
                out = vec4<f32>(((1.0 - ab) * color.rgb + ab * mixed) * a, a);
            }
        }
        default: {}
    }
    let cov = clip_cov(input.clip, window) * input.alpha;
    var mask = vec4<f32>(out.a);
    if (lcd.a >= 0.0) {
        mask = lcd;
    }
    return Shaded(out * cov, mask * cov);
}

// An opaque rectangle covering all of the scissor, under no clip or
// effect (`frame.rs`'s `opaque_over`): written over the target, not
// blended, as the blend of an opaque fragment would leave it.
@fragment
fn fs_fill(input: Varying) -> @location(0) vec4<f32> {
    return premul(wcolor(input.at + 2u));
}
