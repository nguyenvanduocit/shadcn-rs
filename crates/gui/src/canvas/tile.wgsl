// A canvas tile drawn where it lands (ADR-0017): its texture stretched
// over a quadrilateral (the tiling scaled and turned to the camera),
// sampled bilinearly, already premultiplied (a tile is painted as the
// canvas is), cut to the canvas' whole pixels.

struct TileP {
    tex: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
    // Corners from the item's bounds: top left, top right, bottom left, bottom right.
    c01: vec4<f32>,
    c23: vec4<f32>,
    // Where it may draw: x0, y0, x1, y1 from the item's bounds.
    cut: vec4<f32>,
}

@group(1) @binding(0) var<uniform> tile: TileP;
@group(1) @binding(1) var tile_tex: texture_2d<f32>;

struct TileVarying {
    @builtin(position) position: vec4<f32>,
    // Texels, 0 to 256 across the tile.
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_tile(@builtin(vertex_index) vertex: u32) -> TileVarying {
    var out: TileVarying;
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    var corner = tile.c01.xy;
    if (vertex == 1u) {
        corner = tile.c01.zw;
    } else if (vertex == 2u) {
        corner = tile.c23.xy;
    } else if (vertex == 3u) {
        corner = tile.c23.zw;
    }
    out.position = chunk_position(chunk_item.bounds.xy + corner);
    out.uv = unit * vec2<f32>(textureDimensions(tile_tex));
    return out;
}

fn texel(c: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(tile_tex));
    return textureLoad(tile_tex, clamp(c, vec2<i32>(0), size - 1), 0);
}

@fragment
fn fs_tile(input: TileVarying) -> @location(0) vec4<f32> {
    let p = chunk_local(input.position.xy) - chunk_item.bounds.xy;
    if (p.x < tile.cut.x || p.y < tile.cut.y || p.x >= tile.cut.z || p.y >= tile.cut.w) {
        discard;
    }
    let q = input.uv - 0.5;
    let f = floor(q);
    let w = q - f;
    let c = vec2<i32>(f);
    let top = mix(texel(c), texel(c + vec2<i32>(1, 0)), w.x);
    let bottom = mix(texel(c + vec2<i32>(0, 1)), texel(c + vec2<i32>(1, 1)), w.x);
    return mix(top, bottom, w.y) * chunk_fade(input.position.xy);
}
