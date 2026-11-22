// What every data chunk's draw pipeline shares (ADR-0018), prepended after
// `common.wgsl`: where the pass draws, where the chunk's texture lies in
// it, the chunk's clip and fade. A draw pipeline's own bindings are group 1.
//
// Positions a chunk draws at are in its texture's pixels: its region's
// device pixels from the region's top left, floored. In the main pass that
// texture's origin lies at `chunk_item.origin` (window pixels); in its
// coverage pass the target is the texture itself.

struct ChunkGlobals {
    size: vec2<f32>,
    origin: vec2<f32>,
}

struct ChunkItem {
    // The texture's top left in window pixels (0 in its coverage pass).
    origin: vec2<f32>,
    // The texture's size.
    size: vec2<f32>,
    // Where the data maps, in texture pixels: x, y, width, height.
    bounds: vec4<f32>,
    // The clip (window, `clips` below; 0 none), the fade, the scale.
    clip: u32,
    alpha: f32,
    scale: f32,
    coverage: u32,
    // The composite's colours: an area's at the top and bottom of
    // `bounds`, and the line's (sRGB, straight alpha).
    area_top: vec4<f32>,
    area_bottom: vec4<f32>,
    line: vec4<f32>,
}

@group(0) @binding(0) var<uniform> chunk_globals: ChunkGlobals;
@group(0) @binding(1) var<uniform> chunk_item: ChunkItem;
// Four vec4s per clip: rect, radii, the map from the window into the
// clip's space (a, b, c, d), (e, f, its scale, 0). Clip 0 is none.
@group(0) @binding(2) var<storage, read> chunk_clips: array<vec4<f32>>;

// Texture pixel `p` as a clip-space position in the pass.
fn chunk_position(p: vec2<f32>) -> vec4<f32> {
    let at = p + chunk_item.origin - chunk_globals.origin;
    return vec4<f32>(at / chunk_globals.size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}

// The texture pixel a fragment at `frag` (its `@builtin(position)`) is.
fn chunk_local(frag: vec2<f32>) -> vec2<f32> {
    return frag + chunk_globals.origin - chunk_item.origin;
}

fn chunk_clip_coverage(id: u32, p: vec2<f32>) -> f32 {
    if (id == 0u) {
        return 1.0;
    }
    let rect = chunk_clips[id * 4u];
    let radii = chunk_clips[id * 4u + 1u];
    let m = chunk_clips[id * 4u + 2u];
    let t = chunk_clips[id * 4u + 3u];
    let q = vec2<f32>(m.x * p.x + m.z * p.y + t.x, m.y * p.x + m.w * p.y + t.y);
    if (t.z == 1.0 && m.y == 0.0 && m.z == 0.0) {
        return rrect_cov(q, rect, radii);
    }
    return saturate(0.5 - rrect_sdf(q, rect, radii) * t.z);
}

// What a fragment at `frag` keeps of its colour: the clip and the fade.
fn chunk_fade(frag: vec2<f32>) -> f32 {
    return chunk_item.alpha * chunk_clip_coverage(chunk_item.clip, frag + chunk_globals.origin);
}

// The exact area of the unit pixel square centred at 0 on the side of a
// line `d` from its centre (along a unit normal `n`) where the normal
// points away: a half-plane's box-filtered coverage.
fn half_plane(d: f32, n: vec2<f32>) -> f32 {
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

// ∫ from x0 to x1 of clamp(c − (m·x + k), 0, 1) dx: the area of a pixel
// column's row below a line (y down), over that span.
fn area_below(x0: f32, x1: f32, m: f32, k: f32, c: f32) -> f32 {
    if (x1 <= x0) {
        return 0.0;
    }
    let u0 = c - (m * x0 + k);
    let u1 = c - (m * x1 + k);
    if (abs(u1 - u0) < 1e-6) {
        return clamp(u0, 0.0, 1.0) * (x1 - x0);
    }
    return (ramp_integral(u1) - ramp_integral(u0)) / (u1 - u0) * (x1 - x0);
}

// ∫ from 0 to u of clamp(t, 0, 1) dt.
fn ramp_integral(u: f32) -> f32 {
    if (u <= 0.0) {
        return 0.0;
    }
    if (u >= 1.0) {
        return u - 0.5;
    }
    return 0.5 * u * u;
}
