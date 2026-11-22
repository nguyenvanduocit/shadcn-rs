// What lies below a backdrop, scaled down and blurred (`gpu/read.rs`): one
// triangle over the pass's viewport (the size the pass writes), each
// texel read from the other scratch texture. A halving is the mean of the
// 2×2 texels a texel covers; a blur pass one direction of a Gaussian. The
// source's edges are mirrored, as Chrome's backdrop filter mirrors them.
// Values are premultiplied and stay in the target's encoded values, as
// Chrome blurs.

struct Step {
    // The texel step of a blur pass, (1, 0) or (0, 1); (0, 0) halves.
    dir: vec2<f32>,
    sigma: f32,
    _pad: f32,
    // The texels of the source that hold what was read.
    size: vec2<f32>,
    _pad2: vec2<f32>,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> step: Step;

@vertex
fn vs_read(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    return vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}

// `i` reflected into `0..n`, the edge texel repeated (Skia's `kMirror`).
fn mirror(i: i32, n: i32) -> i32 {
    let period = 2 * max(n, 1);
    var m = i % period;
    if (m < 0) {
        m += period;
    }
    return select(m, period - 1 - m, m >= n);
}

fn texel(p: vec2<i32>) -> vec4<f32> {
    let n = vec2<i32>(step.size);
    return textureLoad(source, vec2<i32>(mirror(p.x, n.x), mirror(p.y, n.y)), 0);
}

@fragment
fn fs_read(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(position.xy));
    if (step.sigma <= 0.0) {
        let q = p * 2;
        return 0.25 * (texel(q) + texel(q + vec2<i32>(1, 0)) + texel(q + vec2<i32>(0, 1)) + texel(q + vec2<i32>(1, 1)));
    }
    let r = i32(ceil(3.0 * step.sigma));
    let d = vec2<i32>(step.dir);
    let k = -0.5 / (step.sigma * step.sigma);
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = -r; i <= r; i = i + 1) {
        let x = f32(i);
        let w = exp(k * x * x);
        sum += texel(p + d * i) * w;
        total += w;
    }
    return sum / total;
}
