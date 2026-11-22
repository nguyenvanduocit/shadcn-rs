// A canvas' small objects as boxes of their colour (ADR-0017): one
// instance a box, its coverage the exact area of each pixel it covers.

struct BoxP {
    count: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

@group(1) @binding(0) var<uniform> box_p: BoxP;
// Eight floats a box: x, y, width, height from the item's bounds, colour.
@group(1) @binding(1) var<storage, read> boxes: array<vec4<f32>>;

struct BoxVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) rect: vec4<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
}

@vertex
fn vs_box(@builtin(vertex_index) vertex: u32, @builtin(instance_index) k: u32) -> BoxVarying {
    var out: BoxVarying;
    let r = boxes[k * 2u];
    let rect = vec4<f32>(r.xy + chunk_item.bounds.xy, r.zw);
    out.rect = rect;
    out.color = boxes[k * 2u + 1u];
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    let lo = floor(rect.xy);
    let hi = ceil(rect.xy + rect.zw);
    out.position = chunk_position(lo + unit * (hi - lo));
    return out;
}

@fragment
fn fs_box(input: BoxVarying) -> @location(0) vec4<f32> {
    let p = chunk_local(input.position.xy);
    let r = input.rect;
    let over = clamp(min(p + 0.5, r.xy + r.zw) - max(p - 0.5, r.xy), vec2<f32>(0.0), vec2<f32>(1.0));
    return premul(input.color) * (over.x * over.y) * chunk_fade(input.position.xy);
}
