// Linear gradients from a table of stops (two vec4s each: the offset, then
// the straight colour), mixed in OKLab between stops as CSS mixes them.
// Prepended by `gpu::module` to the modules that declare `stops`.

// The colour of the gradient of stops `first..first + count` along `line`
// (x0, y0, x1, y1) at `p`, padded past both ends.
fn gradient_stops(first: u32, count: u32, p: vec2<f32>, line: vec4<f32>) -> vec4<f32> {
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
