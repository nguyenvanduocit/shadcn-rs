// Bars in the main pass (ADR-0018): one instance a compact M4 point inside
// the view, a rectangle from the point to its floor, its far corners
// rounded, coverage exact on its straight edges.

struct BarStyle {
    n: u32,
    // Extended columns less one: the column after the view.
    after: u32,
    width: f32,
    offset: f32,
    radius: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
    color: vec4<f32>,
}

@group(1) @binding(0) var<uniform> style: BarStyle;
@group(1) @binding(1) var<storage, read> points: array<vec4<u32>>;
@group(1) @binding(2) var<storage, read> offsets: array<u32>;

struct BarVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) rect: vec4<f32>,
    @location(1) @interpolate(flat) radii: vec4<f32>,
}

@vertex
fn vs_bar(@builtin(vertex_index) vertex: u32, @builtin(instance_index) k: u32) -> BarVarying {
    var out: BarVarying;
    out.position = vec4<f32>(-2.0, -2.0, 0.0, 1.0);
    if (k >= offsets[style.n]) {
        return out;
    }
    let e = points[k * 2u + 1u].x;
    if (e == 0u || e >= style.after) {
        return out;
    }
    let p = points[k * 2u];
    let x = bitcast<f32>(p.x) + style.offset;
    let y = bitcast<f32>(p.y);
    let f = bitcast<f32>(p.z);
    let top = min(y, f);
    let h = abs(f - y);
    let r = min(style.radius, 0.5 * min(style.width, h));
    if (y <= f) {
        out.radii = vec4<f32>(r, r, 0.0, 0.0);
    } else {
        out.radii = vec4<f32>(0.0, 0.0, r, r);
    }
    out.rect = vec4<f32>(x, top, style.width, h);
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    let at = vec2<f32>(floor(x) - 1.0, floor(top) - 1.0) + unit * vec2<f32>(ceil(style.width) + 3.0, ceil(h) + 3.0);
    out.position = chunk_position(at);
    return out;
}

@fragment
fn fs_bar(input: BarVarying) -> @location(0) vec4<f32> {
    let p = chunk_local(input.position.xy);
    let cov = rrect_cov(p, input.rect, input.radii);
    return premul(style.color) * cov * chunk_fade(input.position.xy);
}
