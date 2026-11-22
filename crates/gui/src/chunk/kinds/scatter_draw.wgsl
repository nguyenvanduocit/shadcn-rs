struct Dot {
    color: vec4<f32>,
}

@group(1) @binding(0) var<uniform> view: View;
@group(1) @binding(1) var<uniform> pp: PageP;
@group(1) @binding(2) var<storage, read> page: array<u32>;
@group(1) @binding(3) var<storage, read> shown_in: array<u32>;
@group(1) @binding(4) var<uniform> dot_style: Dot;

struct DotVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) centre: vec2<f32>,
}

@vertex
fn vs_dot(@builtin(vertex_index) vertex: u32, @builtin(instance_index) k: u32) -> DotVarying {
    var out: DotVarying;
    let i = shown_in[k];
    let c = vec2<f32>(x_px(i), y_px(i));
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    let reach = view.r + 1.0;
    out.position = chunk_position(c + (unit * 2.0 - 1.0) * reach);
    out.centre = c;
    return out;
}

@fragment
fn fs_dot(input: DotVarying) -> @location(0) vec4<f32> {
    let p = chunk_local(input.position.xy);
    let cov = clamp(view.r + 0.5 - length(p - input.centre), 0.0, 1.0);
    return premul(dot_style.color) * cov * chunk_fade(input.position.xy);
}
