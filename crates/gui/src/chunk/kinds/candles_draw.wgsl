// One candle a column: its body (open to close) and its wick (high to low).

struct CandleStyle {
    cols: u32,
    width: f32,
    wick: f32,
    pad: f32,
    up: vec4<f32>,
    down: vec4<f32>,
}

@group(1) @binding(0) var<uniform> style: CandleStyle;
@group(1) @binding(1) var<storage, read> candles: array<vec4<u32>>;

struct CandleVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) body: vec4<f32>,
    @location(1) @interpolate(flat) wick: vec4<f32>,
    @location(2) @interpolate(flat) color: vec4<f32>,
}

@vertex
fn vs_candle(@builtin(vertex_index) vertex: u32, @builtin(instance_index) c: u32) -> CandleVarying {
    var out: CandleVarying;
    out.position = vec4<f32>(-2.0, -2.0, 0.0, 1.0);
    let a = candles[c * 2u];
    let b = candles[c * 2u + 1u];
    if (b.z == 0u || c >= style.cols) {
        return out;
    }
    let x0 = bitcast<f32>(a.x);
    let x1 = bitcast<f32>(a.y);
    let o = bitcast<f32>(a.z);
    let h = bitcast<f32>(a.w);
    let l = bitcast<f32>(b.x);
    let cl = bitcast<f32>(b.y);
    var left = floor(x0);
    var w = max(floor(x1) - left + 1.0, 1.0);
    if (b.z == 1u) {
        left = x0 - 0.5 * style.width;
        w = style.width;
    }
    let top = min(o, cl);
    let bottom = max(o, cl);
    out.body = vec4<f32>(left, top, w, max(bottom - top, 1.0));
    let mid = left + 0.5 * w;
    out.wick = vec4<f32>(mid - 0.5 * style.wick, min(h, l), style.wick, abs(l - h));
    if (cl <= o) {
        out.color = style.up;
    } else {
        out.color = style.down;
    }
    let y0 = min(min(h, l), top) - 1.0;
    let y1 = max(max(h, l), bottom) + 1.0;
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    let at = vec2<f32>(mix(floor(left) - 1.0, ceil(left + w) + 1.0, unit.x), mix(floor(y0), ceil(y1), unit.y));
    out.position = chunk_position(at);
    return out;
}

@fragment
fn fs_candle(input: CandleVarying) -> @location(0) vec4<f32> {
    let p = chunk_local(input.position.xy);
    let cov = max(rrect_cov(p, input.body, vec4<f32>(0.0)), rrect_cov(p, input.wick, vec4<f32>(0.0)));
    return premul(input.color) * cov * chunk_fade(input.position.xy);
}
