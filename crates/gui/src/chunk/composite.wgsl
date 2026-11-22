// A data chunk's coverage texture onto the main pass (ADR-0018): its red
// channel is the areas' coverage (added: their pieces tile the area), its
// alpha the lines' (the greatest: joins and overlaps never darken), drawn
// as one primitive in the chunk's place, its colours from `chunk_item`,
// under its clip and fade.

@group(1) @binding(0) var chunk_coverage: texture_2d<f32>;

struct CompositeVarying {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_composite(@builtin(vertex_index) vertex: u32) -> CompositeVarying {
    var out: CompositeVarying;
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    out.position = chunk_position(unit * chunk_item.size);
    return out;
}

@fragment
fn fs_composite(input: CompositeVarying) -> @location(0) vec4<f32> {
    let p = chunk_local(input.position.xy);
    let texel = vec2<i32>(floor(p));
    let c = textureLoad(chunk_coverage, texel, 0);
    let b = chunk_item.bounds;
    let t = clamp((p.y - b.y) / max(b.w, 1e-3), 0.0, 1.0);
    let area = premul(mix(chunk_item.area_top, chunk_item.area_bottom, t)) * clamp(c.r, 0.0, 1.0);
    let line = premul(chunk_item.line) * clamp(c.a, 0.0, 1.0);
    let color = line + area * (1.0 - line.a);
    return color * chunk_fade(input.position.xy);
}
