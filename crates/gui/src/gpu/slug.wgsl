// Derived from HarfBuzz's hb-gpu-fragment.wgsl, under this notice:
//
//   Copyright (C) 2026  Behdad Esfahbod
//
//   This is part of HarfBuzz, a text shaping library.
//
//   Permission is hereby granted, without written agreement and without
//   license or royalty fees, to use, copy, modify, and distribute this
//   software and its documentation for any purpose, provided that the
//   above copyright notice and the following two paragraphs appear in
//   all copies of this software.
//
//   IN NO EVENT SHALL THE COPYRIGHT HOLDER BE LIABLE TO ANY PARTY FOR
//   DIRECT, INDIRECT, SPECIAL, INCIDENTAL, OR CONSEQUENTIAL DAMAGES
//   ARISING OUT OF THE USE OF THIS SOFTWARE AND ITS DOCUMENTATION, EVEN
//   IF THE COPYRIGHT HOLDER HAS BEEN ADVISED OF THE POSSIBILITY OF SUCH
//   DAMAGE.
//
//   THE COPYRIGHT HOLDER SPECIFICALLY DISCLAIMS ANY WARRANTIES, INCLUDING,
//   BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND
//   FITNESS FOR A PARTICULAR PURPOSE.  THE SOFTWARE PROVIDED HEREUNDER IS
//   ON AN "AS IS" BASIS, AND THE COPYRIGHT HOLDER HAS NO OBLIGATION TO
//   PROVIDE MAINTENANCE, SUPPORT, UPDATES, ENHANCEMENTS, OR MODIFICATIONS.

// Curve text (ADR-0017, P9): glyph coverage computed from the glyph's
// quadratic curves in the fragment shader, at any size and under any
// transform. A port of HarfBuzz's hb-gpu fragment shader (Behdad Esfahbod,
// 2026; `references/harfbuzz/src/hb-gpu-fragment.wgsl`, notice above and in
// THIRD_PARTY.md), itself Eric Lengyel's Slug: a horizontal and a vertical
// ray from the pixel centre, each crossing a band's curves sorted so the
// loop stops at the first one out of reach, their coverages weighted by
// how near each crossing is. Two variants: banded, and a full scan of every
// curve (for an adapter whose compiler gets the banded loop wrong; chosen
// by a self-test at first use).
//
// Blob layout (vec4<i32> texels, from the glyph's location; coordinates in
// UNITS per curve pixel, y up):
//   0: extents (min x, min y, max x, max y)
//   1: horizontal bands, vertical bands, curves, where the full list starts
//   2…: band headers (count, descending list, ascending list, split),
//       horizontal then vertical; then the lists (curve offsets in .x);
//       then curves, sharing end points: (p1, p2), (p3, next p2).

struct Globals {
    size: vec2<f32>,
    origin: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
// SLUG vec4s per glyph: extents (curve px, y up), map a b c d (curve px →
// window), (e, f, blob, gamma row), colour, (device clip, local clip, 0, 0).
@group(0) @binding(1) var<storage, read> slugs: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> blobs: array<vec4<i32>>;
@group(0) @binding(3) var<storage, read> clips: array<vec4<f32>>;
@group(0) @binding(4) var gamma_table: texture_2d<f32>;

// `SLUG` (`gpu::slug::SLUG`), `CURVE_EM` (`text::CURVE_EM`, curve pixels per
// em) and `UNITS` (quantization steps per curve pixel) are prepended from Rust
// (`gpu/contract.rs`).
const INV_UNITS: f32 = 1.0 / UNITS;

struct SlugVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) em: vec2<f32>,
    @location(1) @interpolate(flat) id: u32,
    // Its two clips, each 0 when the glyph's quad lies wholly inside its chain.
    @location(2) @interpolate(flat) clips_of: vec2<u32>,
}

fn sf(id: u32, i: u32) -> vec4<f32> {
    return slugs[id * SLUG + i];
}

@vertex
fn vs_slug(@builtin(vertex_index) vertex: u32, @builtin(instance_index) id: u32) -> SlugVarying {
    let ext = sf(id, 0u);
    let m = sf(id, 1u);
    let t = sf(id, 2u);
    // Grown by a device pixel each way, in curve pixels: the least scale
    // of the map turns one device pixel into this many.
    let p = m.x * m.x + m.y * m.y + m.z * m.z + m.w * m.w;
    let q = m.x * m.w - m.y * m.z;
    let least = sqrt(max((p - sqrt(max(p * p - 4.0 * q * q, 0.0))) * 0.5, 1e-12));
    let grow = 1.0 / least;
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    let em = mix(ext.xy - vec2<f32>(grow), ext.zw + vec2<f32>(grow), unit);
    let window = vec2<f32>(m.x * em.x + m.z * em.y + t.x, m.y * em.x + m.w * em.y + t.y);
    let at = window - globals.origin;
    var out: SlugVarying;
    let lo = ext.xy - vec2<f32>(grow);
    let hi = ext.zw + vec2<f32>(grow);
    let to_window = mat2x2<f32>(m.xy, m.zw);
    let a = to_window * lo + t.xy;
    let b = to_window * vec2<f32>(hi.x, lo.y) + t.xy;
    let c = to_window * vec2<f32>(lo.x, hi.y) + t.xy;
    let d = to_window * hi + t.xy;
    let clip = sf(id, 4u);
    out.clips_of = vec2<u32>(slug_clip_needed(bitcast<u32>(clip.x), a, b, c, d), slug_clip_needed(bitcast<u32>(clip.y), a, b, c, d));
    // Never skipped (`path::ClipSkip`, tests).
    if (bitcast<u32>(clip.z) == 2u) {
        out.clips_of = bitcast<vec2<u32>>(clip.xy);
    }
    out.position = vec4<f32>(at / globals.size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.em = em;
    out.id = id;
    return out;
}

fn fetch(at: i32) -> vec4<i32> {
    return blobs[at];
}

fn calc_root_code(y1: f32, y2: f32, y3: f32) -> u32 {
    let i1 = bitcast<u32>(y1) >> 31u;
    let i2 = bitcast<u32>(y2) >> 30u;
    let i3 = bitcast<u32>(y3) >> 29u;
    var shift = (i2 & 2u) | (i1 & ~2u);
    shift = (i3 & 4u) | (shift & ~4u);
    return (0x2E74u >> shift) & 0x0101u;
}

fn solve_horiz_poly(a: vec2<f32>, b: vec2<f32>, p1: vec2<f32>) -> vec2<f32> {
    let ra = 1.0 / a.y;
    let rb = 0.5 / b.y;
    let d = sqrt(max(b.y * b.y - a.y * p1.y, 0.0));
    var t1 = (b.y - d) * ra;
    var t2 = (b.y + d) * ra;
    if (a.y == 0.0) {
        t1 = p1.y * rb;
        t2 = t1;
    }
    return vec2<f32>((a.x * t1 - b.x * 2.0) * t1 + p1.x, (a.x * t2 - b.x * 2.0) * t2 + p1.x);
}

fn solve_vert_poly(a: vec2<f32>, b: vec2<f32>, p1: vec2<f32>) -> vec2<f32> {
    let ra = 1.0 / a.x;
    let rb = 0.5 / b.x;
    let d = sqrt(max(b.x * b.x - a.x * p1.x, 0.0));
    var t1 = (b.x - d) * ra;
    var t2 = (b.x + d) * ra;
    if (a.x == 0.0) {
        t1 = p1.x * rb;
        t2 = t1;
    }
    return vec2<f32>((a.y * t1 - b.y * 2.0) * t1 + p1.y, (a.y * t2 - b.y * 2.0) * t2 + p1.y);
}

fn calc_coverage(xcov: f32, ycov: f32, xwgt: f32, ywgt: f32) -> f32 {
    let c = max(abs(xcov * xwgt + ycov * ywgt) / max(xwgt + ywgt, 1.0 / 65536.0), min(abs(xcov), abs(ycov)));
    return clamp(c, 0.0, 1.0);
}

struct Curve {
    q12: vec4<f32>,
    q3: vec2<f32>,
}

fn curve_at(loc: i32, offset: i32) -> Curve {
    let raw12 = fetch(loc + offset);
    let raw3 = fetch(loc + offset + 1);
    return Curve(vec4<f32>(raw12) * INV_UNITS, vec2<f32>(f32(raw3.x), f32(raw3.y)) * INV_UNITS);
}

// One sample's coverage. `full`: every curve, no bands, no early exit.
fn slug_single(rc: vec2<f32>, ppe: vec2<f32>, loc: i32, full: bool) -> f32 {
    let h0 = fetch(loc);
    let h1 = fetch(loc + 1);
    let ext = vec4<f32>(h0) * INV_UNITS;
    let nh = h1.x;
    let nv = h1.y;
    let size = ext.zw - ext.xy;
    let band_scale = vec2<f32>(f32(nv), f32(nh)) / max(size, vec2<f32>(1.0 / 65536.0));
    let band = clamp(vec2<i32>((rc - ext.xy) * band_scale), vec2<i32>(0), vec2<i32>(nv - 1, nh - 1));

    var xcov = 0.0;
    var xwgt = 0.0;
    var count: i32;
    var list: i32;
    var left = false;
    if (full) {
        count = h1.z;
        list = h1.w;
    } else {
        let hb = fetch(loc + 2 + band.y);
        count = hb.x;
        left = rc.x < f32(hb.w) * INV_UNITS;
        list = select(hb.y, hb.z, left);
    }
    for (var i = 0; i < count; i += 1) {
        let c = curve_at(loc, fetch(loc + list + i).x);
        let p12 = c.q12 - vec4<f32>(rc, rc);
        let p3 = c.q3 - rc;
        if (!full) {
            if (left) {
                if (min(min(p12.x, p12.z), p3.x) * ppe.x > 0.5) {
                    break;
                }
            } else if (max(max(p12.x, p12.z), p3.x) * ppe.x < -0.5) {
                break;
            }
        }
        let code = calc_root_code(p12.y, p12.w, p3.y);
        if (code != 0u) {
            let a = c.q12.xy - c.q12.zw * 2.0 + c.q3;
            let b = c.q12.xy - c.q12.zw;
            let r = solve_horiz_poly(a, b, p12.xy) * ppe.x;
            var cov: vec2<f32>;
            if (left) {
                cov = clamp(vec2<f32>(0.5) - r, vec2<f32>(0.0), vec2<f32>(1.0));
            } else {
                cov = clamp(r + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(1.0));
            }
            if ((code & 1u) != 0u) {
                xcov += cov.x;
                xwgt = max(xwgt, clamp(1.0 - abs(r.x) * 2.0, 0.0, 1.0));
            }
            if (code > 1u) {
                xcov -= cov.y;
                xwgt = max(xwgt, clamp(1.0 - abs(r.y) * 2.0, 0.0, 1.0));
            }
        }
    }
    if (left) {
        xcov = -xcov;
    }

    var ycov = 0.0;
    var ywgt = 0.0;
    var down = false;
    if (!full) {
        let vb = fetch(loc + 2 + nh + band.x);
        count = vb.x;
        down = rc.y < f32(vb.w) * INV_UNITS;
        list = select(vb.y, vb.z, down);
    }
    for (var i = 0; i < count; i += 1) {
        let c = curve_at(loc, fetch(loc + list + i).x);
        let p12 = c.q12 - vec4<f32>(rc, rc);
        let p3 = c.q3 - rc;
        if (!full) {
            if (down) {
                if (min(min(p12.y, p12.w), p3.y) * ppe.y > 0.5) {
                    break;
                }
            } else if (max(max(p12.y, p12.w), p3.y) * ppe.y < -0.5) {
                break;
            }
        }
        let code = calc_root_code(p12.x, p12.z, p3.x);
        if (code != 0u) {
            let a = c.q12.xy - c.q12.zw * 2.0 + c.q3;
            let b = c.q12.xy - c.q12.zw;
            let r = solve_vert_poly(a, b, p12.xy) * ppe.y;
            var cov: vec2<f32>;
            if (down) {
                cov = clamp(vec2<f32>(0.5) - r, vec2<f32>(0.0), vec2<f32>(1.0));
            } else {
                cov = clamp(r + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(1.0));
            }
            if ((code & 1u) != 0u) {
                ycov -= cov.x;
                ywgt = max(ywgt, clamp(1.0 - abs(r.x) * 2.0, 0.0, 1.0));
            }
            if (code > 1u) {
                ycov += cov.y;
                ywgt = max(ywgt, clamp(1.0 - abs(r.y) * 2.0, 0.0, 1.0));
            }
        }
    }
    if (down) {
        ycov = -ycov;
    }
    return calc_coverage(xcov, ycov, xwgt, ywgt);
}

// Coverage with hb-gpu's four extra samples below 16 device ppem.
fn slug(rc: vec2<f32>, ppe: vec2<f32>, loc: i32, full: bool, ppem: f32) -> f32 {
    var c = slug_single(rc, ppe, loc, full);
    if (ppem < 16.0) {
        let d = (1.0 / ppe) * (1.0 / 3.0);
        let msaa = 0.25 * (
            slug_single(rc + vec2<f32>(-d.x, -d.y), ppe, loc, full) +
            slug_single(rc + vec2<f32>(d.x, -d.y), ppe, loc, full) +
            slug_single(rc + vec2<f32>(-d.x, d.y), ppe, loc, full) +
            slug_single(rc + vec2<f32>(d.x, d.y), ppe, loc, full));
        c = mix(c, msaa, smoothstep(16.0, 8.0, ppem));
    }
    return c;
}

// Clip entry `id` and the rest of its chain, their coverages multiplied.
fn slug_clip(id: u32, p: vec2<f32>) -> f32 {
    var cov = 1.0;
    var e = id;
    let n = arrayLength(&clips) / PCLIP;
    for (var d = 0u; d < n && e != 0u; d = d + 1u) {
        let at = e * PCLIP;
        cov = cov * clip_entry_cov(clips[at], clips[at + 1u], clips[at + 2u], clips[at + 3u], p);
        e = bitcast<u32>(clips[at + 4u].x);
    }
    return cov;
}

// `id`, or 0 when the quad of corners `a`…`d` lies wholly inside each
// clip of its chain (as a quad's is dropped, `shaders.wgsl`).
fn slug_clip_needed(id: u32, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, d: vec2<f32>) -> u32 {
    var e = id;
    let n = arrayLength(&clips) / PCLIP;
    for (var k = 0u; k < n && e != 0u; k = k + 1u) {
        let at = e * PCLIP;
        if (!entry_holds(clips[at], clips[at + 1u], clips[at + 2u], clips[at + 3u], a, b, c, d)) {
            return id;
        }
        e = bitcast<u32>(clips[at + 4u].x);
    }
    return 0u;
}

struct SlugShaded {
    color: vec4<f32>,
    mask: vec4<f32>,
}

fn shade_slug(input: SlugVarying, full: bool) -> SlugShaded {
    let id = input.id;
    let t = sf(id, 2u);
    var color = sf(id, 3u);
    // Marked where a clip's test was skipped (`path::ClipSkip`, tests).
    let clip = bitcast<vec4<u32>>(sf(id, 4u));
    let skipped = (clip.x != 0u && input.clips_of.x == 0u) || (clip.y != 0u && input.clips_of.y == 0u);
    if (clip.z == 1u && skipped) {
        color = vec4<f32>(0.0, 1.0, 0.0, 1.0);
    }
    // Device pixels per curve pixel along each axis, at this fragment.
    let ppe = 1.0 / max(fwidth(input.em), vec2<f32>(1e-9));
    let ppem = min(ppe.x, ppe.y) * CURVE_EM;
    let loc = bitcast<i32>(t.z);
    var cov = slug(input.em, ppe, loc, full, ppem);
    // Through the gamma table, as the atlas's glyphs are.
    cov = textureLoad(gamma_table, vec2<i32>(i32(round(saturate(cov) * 255.0)), i32(t.w)), 0).r;
    let window = input.position.xy + globals.origin;
    cov *= slug_clip(input.clips_of.x, window) * slug_clip(input.clips_of.y, window);
    let out = premul(color) * cov;
    return SlugShaded(out, vec4<f32>(out.a));
}

fn shade_slug_banded(input: SlugVarying) -> SlugShaded {
    return shade_slug(input, false);
}

fn shade_slug_full(input: SlugVarying) -> SlugShaded {
    return shade_slug(input, true);
}
