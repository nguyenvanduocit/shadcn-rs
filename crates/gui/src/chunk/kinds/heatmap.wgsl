// A heatmap (ADR-0018): a grid of values in a float texture, filling the
// chunk's bounds; each pixel the box-filtered mix of the cells it covers
// (up to 4 × 4 of them), each cell's colour mixed between `low` and `high`
// in OKLab by where its value lies between `min` and `max`.

struct HeatStyle {
    low: vec4<f32>,
    high: vec4<f32>,
    min: f32,
    max: f32,
    cols: f32,
    rows: f32,
}

@group(1) @binding(0) var<uniform> heat: HeatStyle;
@group(1) @binding(1) var grid: texture_2d<f32>;

struct HeatVarying {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_heat(@builtin(vertex_index) vertex: u32) -> HeatVarying {
    var out: HeatVarying;
    let b = chunk_item.bounds;
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    out.position = chunk_position(vec2<f32>(floor(b.x), floor(b.y)) + unit * vec2<f32>(ceil(b.z) + 2.0, ceil(b.w) + 2.0));
    return out;
}

fn cell_color(c: vec2<i32>) -> vec4<f32> {
    let v = textureLoad(grid, c, 0).r;
    if (v != v) {
        return vec4<f32>(0.0);
    }
    let t = clamp((v - heat.min) / max(heat.max - heat.min, 1e-30), 0.0, 1.0);
    return premul(mix_oklab(heat.low, heat.high, t));
}

@fragment
fn fs_heat(input: HeatVarying) -> @location(0) vec4<f32> {
    let p = chunk_local(input.position.xy);
    let b = chunk_item.bounds;
    let size = vec2<f32>(heat.cols, heat.rows);
    let cell = b.zw / size;
    // The pixel's square in grid units, inside the grid.
    let lo = clamp((p - 0.5 - b.xy) / cell, vec2<f32>(0.0), size);
    let hi = clamp((p + 0.5 - b.xy) / cell, vec2<f32>(0.0), size);
    if (hi.x <= lo.x || hi.y <= lo.y) {
        return vec4<f32>(0.0);
    }
    var acc = vec4<f32>(0.0);
    let steps = 4;
    let d = (hi - lo) / f32(steps);
    for (var j = 0; j < steps; j += 1) {
        for (var i = 0; i < steps; i += 1) {
            let q = lo + (vec2<f32>(f32(i), f32(j)) + 0.5) * d;
            acc += cell_color(vec2<i32>(floor(min(q, size - 0.5))));
        }
    }
    // Covered area of the pixel times the cells' mean colour.
    let area = (hi.x - lo.x) * (hi.y - lo.y) * cell.x * cell.y;
    return acc / f32(steps * steps) * clamp(area, 0.0, 1.0) * chunk_fade(input.position.xy);
}
