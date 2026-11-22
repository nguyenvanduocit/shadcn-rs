// The persistent canvas onto the window's surface (ADR-0016): one triangle
// covering the target, each pixel read from the canvas as it is. The
// canvas holds the frame across frames, and only damage is drawn into it;
// the surface needs no `COPY_DST` for this.

@group(0) @binding(0) var canvas: texture_2d<f32>;

@vertex
fn vs_blit(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    return vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}

@fragment
fn fs_blit(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(canvas, vec2<i32>(floor(position.xy)), 0);
}
