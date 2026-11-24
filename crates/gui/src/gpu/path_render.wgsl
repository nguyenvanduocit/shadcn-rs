// Paths in the main pass (ADR-0017): one instance per tile of a binned path
// job (`path_compute.wgsl`), drawn in the scene's order. A tile with edges
// covers each pixel with the exact area the path's lines enclose in it,
// starting from the tile's backdrop (the winding at its top left), adding
// each line's crossing of the tile's left edge and the signed area right of
// each line within the pixel (as vello's fine rasterizer and FreeType's
// smooth rasterizer accumulate it), then applying the fill rule. A run of
// tiles wholly inside is one solid instance. Coverage is multiplied by the
// paint (a colour, a linear gradient in the path's space), the clips (one in
// the window, one in the path's own space) and an alpha.
//
// `fs_path` draws onto the window or a layer (its dual-source variant is
// appended by `gpu::module`); `fs_mask` draws coverage alone into an R8
// atlas page, for icons rasterized once.

struct Globals {
    size: vec2<f32>,
    origin: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> jobs: array<u32>;
@group(0) @binding(2) var<storage, read> draws: array<u32>;
@group(0) @binding(3) var<storage, read> tiles: array<u32>;
@group(0) @binding(4) var<storage, read> entries: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read> instances: array<vec4<u32>>;
@group(0) @binding(6) var<storage, read> stops: array<vec4<f32>>;
// Clip entries (`PCLIP` in `common.wgsl`), read in the target's space.
// Clip 0 is none.
@group(0) @binding(7) var<storage, read> clips: array<vec4<f32>>;

// `TILE`, `JOB`, `DRAW` and `F_EVEN_ODD` come from `gpu/path.rs` (`gpu/contract.rs`).

fn jf(job: u32, i: u32) -> f32 {
    return bitcast<f32>(jobs[job * JOB + i]);
}

fn ju(job: u32, i: u32) -> u32 {
    return jobs[job * JOB + i];
}

fn ji(job: u32, i: u32) -> i32 {
    return bitcast<i32>(jobs[job * JOB + i]);
}

fn df(job: u32, i: u32) -> f32 {
    return bitcast<f32>(draws[job * DRAW + i]);
}

fn du(job: u32, i: u32) -> u32 {
    return draws[job * DRAW + i];
}

struct PathVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) job: u32,
    @location(1) @interpolate(flat) tile: u32,
    @location(2) @interpolate(flat) solid: u32,
    // Its two clips, each 0 when the tile lies wholly inside its chain.
    @location(3) @interpolate(flat) clips_of: vec2<u32>,
}

@vertex
fn vs_path(@builtin(vertex_index) vertex: u32, @builtin(instance_index) id: u32) -> PathVarying {
    var out: PathVarying;
    let inst = instances[id];
    out.job = inst.x;
    out.tile = inst.y;
    out.solid = inst.w;
    if (inst.z == 0u) {
        // An empty tile: nothing to draw.
        out.position = vec4<f32>(-2.0, -2.0, 0.0, 1.0);
        return out;
    }
    let job = inst.x;
    let w = ju(job, 11u);
    let tx = ji(job, 9u) + i32(inst.y % w);
    let ty = ji(job, 10u) + i32(inst.y / w);
    let quad = vec4<f32>(f32(tx) * TILE, f32(ty) * TILE, f32(inst.z) * TILE, TILE);
    let unit = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));
    let offset = vec2<f32>(df(job, 0u), df(job, 1u));
    let at = quad.xy + unit * quad.zw + offset - globals.origin;
    let lo = quad.xy + offset;
    let hi = lo + quad.zw;
    out.clips_of = vec2<u32>(clip_needed(du(job, 20u), lo, hi), clip_needed(du(job, 21u), lo, hi));
    // Never skipped (`path::ClipSkip`, tests).
    if (du(job, 22u) == 2u) {
        out.clips_of = vec2<u32>(du(job, 20u), du(job, 21u));
    }
    out.position = vec4<f32>(at / globals.size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

// ∫ from 0 to u of clamp(1 − t, 0, 1): the area of a unit pixel row right
// of a vertical edge at u from the pixel's left.
fn right_area(u: f32) -> f32 {
    if (u <= 0.0) {
        return u;
    }
    if (u >= 1.0) {
        return 0.5;
    }
    return u - 0.5 * u * u;
}

// The signed area line `l` (tile-relative, its part left of the tile's
// left edge cut off) leaves right of it in the pixel row from `py` and
// right of `px`.
fn line_area(l_in: vec4<f32>, px: f32, py: f32) -> f32 {
    var a = l_in.xy;
    var b = l_in.zw;
    if (a.y == b.y) {
        return 0.0;
    }
    if (a.x < 0.0 && b.x < 0.0) {
        return 0.0;
    }
    if (a.x < 0.0) {
        a = vec2<f32>(0.0, a.y + (b.y - a.y) * (0.0 - a.x) / (b.x - a.x));
    } else if (b.x < 0.0) {
        b = vec2<f32>(0.0, b.y + (a.y - b.y) * (0.0 - b.x) / (a.x - b.x));
    }
    let d = sign(b.y - a.y);
    let ya = max(min(a.y, b.y), py);
    let yb = min(max(a.y, b.y), py + 1.0);
    if (yb <= ya) {
        return 0.0;
    }
    let inv = (b.x - a.x) / (b.y - a.y);
    let xa = a.x + (ya - a.y) * inv - px;
    let xb = a.x + (yb - a.y) * inv - px;
    var avg: f32;
    if (abs(xb - xa) > 1e-6) {
        avg = (right_area(xb) - right_area(xa)) / (xb - xa);
    } else {
        avg = clamp(1.0 - 0.5 * (xa + xb), 0.0, 1.0);
    }
    return d * (yb - ya) * avg;
}

// The path's coverage of the pixel centred on `p` (binned coordinates).
fn coverage(job: u32, tile: u32, p: vec2<f32>) -> f32 {
    let w = ju(job, 11u);
    let origin = vec2<f32>(f32(ji(job, 9u) + i32(tile % w)), f32(ji(job, 10u) + i32(tile / w))) * TILE;
    let ti = (ju(job, 8u) + tile) * 4u;
    var acc = f32(bitcast<i32>(tiles[ti]));
    let n = tiles[ti + 1u];
    let off = ju(job, 13u) + tiles[ti + 2u];
    let q = p - origin;
    let px = floor(q.x);
    let py = floor(q.y);
    for (var i = 0u; i < n; i += 1u) {
        let e = entries[(off + i) * 2u + 1u];
        // Sorted by least x: the rest lie right of this pixel.
        if (e.y - origin.x >= px + 1.0) {
            break;
        }
        let l = entries[(off + i) * 2u] - vec4<f32>(origin, origin);
        if (e.x > -1.0e29) {
            // Where it crosses the tile's left edge: the winding changes
            // from there down, all across the tile.
            let s = -sign(l.z - l.x);
            acc += s * clamp(py + 1.0 - (e.x - origin.y), 0.0, 1.0);
        }
        acc += line_area(l, px, py);
    }
    if ((ju(job, 16u) & F_EVEN_ODD) != 0u) {
        return abs(acc - 2.0 * round(0.5 * acc));
    }
    return min(abs(acc), 1.0);
}

// Clip entry `id` and the rest of its chain, their coverages multiplied.
fn clip_coverage(id: u32, p: vec2<f32>) -> f32 {
    var cov = 1.0;
    var e = id;
    let n = arrayLength(&clips) / PCLIP;
    for (var d = 0u; d < n && e != 0u; d = d + 1u) {
        let at = e * PCLIP;
        cov = cov * clip_entry_cov(clips[at], clips[at + 1u], clips[at + 2u], clips[at + 3u], p);
        e = bitcast<u32>(clips[at + 4u].x);
    }
    return cov;
}

// `id`, or 0 when the box from `lo` to `hi` lies wholly inside each clip
// of its chain (as a quad's is dropped, `shaders.wgsl`).
fn clip_needed(id: u32, lo: vec2<f32>, hi: vec2<f32>) -> u32 {
    var e = id;
    let n = arrayLength(&clips) / PCLIP;
    for (var d = 0u; d < n && e != 0u; d = d + 1u) {
        let at = e * PCLIP;
        if (!entry_holds(clips[at], clips[at + 1u], clips[at + 2u], clips[at + 3u], lo, vec2<f32>(hi.x, lo.y), vec2<f32>(lo.x, hi.y), hi)) {
            return id;
        }
        e = bitcast<u32>(clips[at + 4u].x);
    }
    return 0u;
}

struct PathShaded {
    color: vec4<f32>,
    mask: vec4<f32>,
}

fn shade_path(input: PathVarying) -> PathShaded {
    let job = input.job;
    let window = input.position.xy + globals.origin;
    let p = window - vec2<f32>(df(job, 0u), df(job, 1u));
    var cov = 1.0;
    if (input.solid == 0u) {
        cov = coverage(job, input.tile, p);
    }
    cov *= df(job, 15u) * clip_coverage(input.clips_of.x, window) * clip_coverage(input.clips_of.y, window);
    var color = vec4<f32>(df(job, 8u), df(job, 9u), df(job, 10u), df(job, 11u));
    if (du(job, 12u) == 1u) {
        // A linear gradient, placed in the path's own space.
        let q = vec2<f32>(
            df(job, 2u) * window.x + df(job, 4u) * window.y + df(job, 6u),
            df(job, 3u) * window.x + df(job, 5u) * window.y + df(job, 7u),
        );
        let line = vec4<f32>(df(job, 16u), df(job, 17u), df(job, 18u), df(job, 19u));
        color = gradient_stops(du(job, 13u), du(job, 14u), q, line);
    }
    // Marked where a clip's test was skipped (`path::ClipSkip`, tests).
    let skipped = (du(job, 20u) != 0u && input.clips_of.x == 0u) || (du(job, 21u) != 0u && input.clips_of.y == 0u);
    if (du(job, 22u) == 1u && skipped) {
        color = vec4<f32>(0.0, 1.0, 0.0, 1.0);
    }
    let out = premul(color) * cov;
    return PathShaded(out, vec4<f32>(out.a));
}

// Coverage alone, for an icon drawn into an R8 atlas page.
@fragment
fn fs_mask(input: PathVarying) -> @location(0) vec4<f32> {
    let job = input.job;
    let window = input.position.xy + globals.origin;
    let p = window - vec2<f32>(df(job, 0u), df(job, 1u));
    var cov = 1.0;
    if (input.solid == 0u) {
        cov = coverage(job, input.tile, p);
    }
    return vec4<f32>(cov * df(job, 15u));
}
