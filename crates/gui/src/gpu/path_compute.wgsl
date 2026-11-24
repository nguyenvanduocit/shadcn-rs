// Paths rasterized on the GPU (ADR-0017): the compute passes that turn a
// path job's curves into device-space line segments and bin them into
// 16×16 tiles, before the main pass covers each tile's pixels with exact
// area (`path_render.wgsl`). `gpu::path::reference` is the same algorithm in
// plain Rust, pass for pass, and the tests hold the two to each other.
//
// Passes, each a dispatch over the frame's dirty jobs:
//
//   transform  one thread per control point: local → device;
//   flatten    one thread per curve: a fill's curve into lines; a
//              stroke's into its two offset sides (Wang's subdivision,
//              round joints inside a curve when the turn is not smooth),
//              noting its ends for the next pass;
//   joins      one thread per stroked curve: the join with the curve
//              before, caps, and dots;
//   count      one thread per line slot: the tiles it passes (an exact
//              integer walk), how many lines each tile holds, and the
//              winding each tile row's top edge crossing adds to the tiles
//              on its right;
//   scan       one thread per tile row: winding prefix along the row (each
//              tile's backdrop), room for each tile's list;
//   write      one thread per line slot: the same walk, each line into the
//              lists of the tiles it passes, with where it crosses a
//              tile's left edge;
//   compact    one thread per tile row: each list sorted by its least x,
//              and one instance per tile: edges, a run of solid tiles, or
//              nothing.
//
// Every line is clipped to the job's tile rectangle (parts left of it
// projected onto its left side, so their winding still counts; parts above,
// below and right of it dropped), cut into pieces at most 64 px long and
// snapped to 1/256 px. The walk is in integers on those snapped
// coordinates, so `count` and `write` always agree, and a point two lines
// share is computed once, by one thread, so the tiles' windings close.

// From `gpu/path.rs` (`gpu/contract.rs`): `TILE`; the words of a job (`JOB`),
// a curve item (`ITEM`), a resident curve (`CURVE`) and a curve item's
// scratch (head+, head-, tail+, tail- (device), start and end (local), first
// and last direction (local), flags: `HEAD`); a job's flags (`F_STROKE`,
// `F_EVEN_ODD`, `CAP_SHIFT`, `CAP_MASK`, `JOIN_SHIFT`, `JOIN_MASK`, in word
// `JOB_FLAGS`), a curve item's (`C_*`), the caps (`CAP_*`) and joins
// (`JOIN_*`); and `FIX`, the snapping grid (steps per pixel).
const TILE_FIX: i32 = i32(TILE * FIX);
const PIECE: f32 = 64.0;
const NONE: u32 = 0xffffffffu;
const NO_EDGE: f32 = -1.0e30;

struct Params {
    n_points: u32,
    n_items: u32,
    n_lines: u32,
    n_rows: u32,
    line_ranges: u32,
    n_line_ranges: u32,
    row_ranges: u32,
    n_row_ranges: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> curves: array<u32>;
// Curve items (ITEM words: curve, job, first line slot, segment bound,
// slots for `flatten`, slots for `joins` after them), then line ranges and
// row ranges (first thread, job).
@group(0) @binding(2) var<storage, read> work: array<u32>;
@group(0) @binding(3) var<storage, read> jobs: array<u32>;
// Device points (4 per item), then heads (HEAD per item).
@group(0) @binding(4) var<storage, read_write> scratch: array<f32>;
@group(0) @binding(5) var<storage, read_write> lines: array<vec4<f32>>;
// Per tile: backdrop (i32 bits), count, list offset, cursor. Then per job
// state: the entries cursor.
@group(0) @binding(6) var<storage, read_write> tiles: array<atomic<u32>>;
@group(0) @binding(7) var<storage, read_write> entries: array<vec4<f32>>;
@group(0) @binding(8) var<storage, read_write> instances: array<vec4<u32>>;
@group(0) @binding(9) var<storage, read_write> state: array<atomic<u32>>;

fn jf(job: u32, i: u32) -> f32 {
    return bitcast<f32>(jobs[job * JOB + i]);
}

fn ju(job: u32, i: u32) -> u32 {
    return jobs[job * JOB + i];
}

fn ji(job: u32, i: u32) -> i32 {
    return bitcast<i32>(jobs[job * JOB + i]);
}

fn cf(curve: u32, i: u32) -> f32 {
    return bitcast<f32>(curves[curve * CURVE + i]);
}

fn cu(curve: u32, i: u32) -> u32 {
    return curves[curve * CURVE + i];
}

fn local_point(curve: u32, k: u32) -> vec2<f32> {
    return vec2<f32>(cf(curve, 2u * k), cf(curve, 2u * k + 1u));
}

fn apply(job: u32, p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        jf(job, 0u) * p.x + jf(job, 2u) * p.y + jf(job, 4u),
        jf(job, 1u) * p.x + jf(job, 3u) * p.y + jf(job, 5u),
    );
}

// ---- transform ----

@compute @workgroup_size(64)
fn transform(@builtin(global_invocation_id) gid: vec3<u32>) {
    let t = gid.x;
    if (t >= params.n_points) {
        return;
    }
    let item = t / 4u;
    let k = t % 4u;
    let curve = work[item * ITEM];
    let job = work[item * ITEM + 1u];
    let d = apply(job, local_point(curve, k));
    scratch[t * 2u] = d.x;
    scratch[t * 2u + 1u] = d.y;
}

fn device_point(item: u32, k: u32) -> vec2<f32> {
    return vec2<f32>(scratch[(item * 4u + k) * 2u], scratch[(item * 4u + k) * 2u + 1u]);
}

// ---- curves ----

fn eval(p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, p3: vec2<f32>, order: u32, t: f32) -> vec2<f32> {
    let u = 1.0 - t;
    if (order == 2u) {
        return p0 * u + p1 * t;
    }
    if (order == 3u) {
        return p0 * (u * u) + p1 * (2.0 * u * t) + p2 * (t * t);
    }
    return p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t);
}

// Wang's formula: segments that keep a degree-`order − 1` curve within
// `tol` of its chords.
fn wang(p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, p3: vec2<f32>, order: u32, tol: f32) -> u32 {
    if (order == 2u) {
        return 1u;
    }
    if (order == 3u) {
        let m = length(p0 - 2.0 * p1 + p2);
        return u32(max(ceil(sqrt(0.25 * m / tol)), 1.0));
    }
    let m = max(length(p0 - 2.0 * p1 + p2), length(p1 - 2.0 * p2 + p3));
    return u32(max(ceil(sqrt(0.75 * m / tol)), 1.0));
}

// ---- emitting lines: clipped, cut, snapped ----

struct Out {
    job: u32,
    base: u32,
    count: u32,
    bound: u32,
}

fn snap(p: vec2<f32>) -> vec2<f32> {
    return round(p * FIX) / FIX;
}

fn put(out: ptr<function, Out>, a: vec2<f32>, b: vec2<f32>) {
    if (all(a == b)) {
        return;
    }
    if ((*out).count < (*out).bound) {
        lines[(*out).base + (*out).count] = vec4<f32>(a, b);
        (*out).count += 1u;
    }
}

// `a`–`b` (already clipped) in pieces at most PIECE long, snapped.
fn pieces(out: ptr<function, Out>, a: vec2<f32>, b: vec2<f32>) {
    let d = b - a;
    let n = u32(max(ceil(max(abs(d.x), abs(d.y)) / PIECE), 1.0));
    var prev = snap(a);
    for (var i = 1u; i <= n; i += 1u) {
        var next: vec2<f32>;
        if (i == n) {
            next = snap(b);
        } else {
            next = snap(a + d * (f32(i) / f32(n)));
        }
        put(out, prev, next);
        prev = next;
    }
}

// Emit device line `a`–`b` for the job's tile rectangle.
fn emit(out: ptr<function, Out>, a_in: vec2<f32>, b_in: vec2<f32>) {
    let job = (*out).job;
    let x0 = f32(ji(job, 9u)) * TILE;
    let y0 = f32(ji(job, 10u)) * TILE;
    let x1 = x0 + f32(ju(job, 11u)) * TILE;
    let y1 = y0 + f32(ju(job, 12u)) * TILE;
    let left = x0 - 1.0 / FIX;
    let top = y0 - 1.0 / FIX;
    var a = a_in;
    var b = b_in;
    // Above and below the rectangle: dropped (the crossing of its top stays).
    if (max(a.y, b.y) < top || min(a.y, b.y) > y1) {
        return;
    }
    if (a.y < top) {
        a = vec2<f32>(a.x + (b.x - a.x) * (top - a.y) / (b.y - a.y), top);
    } else if (a.y > y1) {
        a = vec2<f32>(a.x + (b.x - a.x) * (y1 - a.y) / (b.y - a.y), y1);
    }
    if (b.y < top) {
        b = vec2<f32>(b.x + (a.x - b.x) * (top - b.y) / (a.y - b.y), top);
    } else if (b.y > y1) {
        b = vec2<f32>(b.x + (a.x - b.x) * (y1 - b.y) / (a.y - b.y), y1);
    }
    // Right of it: dropped.
    if (min(a.x, b.x) >= x1) {
        return;
    }
    if (a.x > x1) {
        a = vec2<f32>(x1, a.y + (b.y - a.y) * (x1 - a.x) / (b.x - a.x));
    }
    if (b.x > x1) {
        b = vec2<f32>(x1, b.y + (a.y - b.y) * (x1 - b.x) / (a.x - b.x));
    }
    // Left of it: projected onto a line just left of its left side.
    if (a.x < left && b.x < left) {
        pieces(out, vec2<f32>(left, a.y), vec2<f32>(left, b.y));
        return;
    }
    if (a.x < left) {
        let c = vec2<f32>(left, a.y + (b.y - a.y) * (left - a.x) / (b.x - a.x));
        pieces(out, vec2<f32>(left, a.y), c);
        pieces(out, c, b);
        return;
    }
    if (b.x < left) {
        let c = vec2<f32>(left, b.y + (a.y - b.y) * (left - b.x) / (a.x - b.x));
        pieces(out, a, c);
        pieces(out, c, vec2<f32>(left, b.y));
        return;
    }
    pieces(out, a, b);
}

// ---- flatten ----

fn left_normal(u: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(-u.y, u.x);
}

fn rotate(v: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
}

// How the offset side `side` (±1) turns from normal `n0` to `n1` at `p`,
// from device point `from` to device point `to`: outward by the job's join
// (`round_only` inside a curve), or through `p` on the inner side.
fn connect(out: ptr<function, Out>, p: vec2<f32>, n0: vec2<f32>, n1: vec2<f32>, side: f32,
           a_from: vec2<f32>, a_to: vec2<f32>, round_only: bool) {
    let job = (*out).job;
    let h = jf(job, 17u);
    let cross = n0.x * n1.y - n0.y * n1.x;
    // The side the turn bends away from is outer; a turn right back is
    // outer on the plus side.
    let back = abs(cross) < 1e-6 && dot(n0, n1) < 0.0;
    let outer = cross * side < 0.0 || (back && side > 0.0);
    if (!outer) {
        let c = apply(job, p);
        emit(out, a_from, c);
        emit(out, c, a_to);
        return;
    }
    var join = (ju(job, JOB_FLAGS) >> JOIN_SHIFT) & JOIN_MASK;
    if (round_only) {
        join = JOIN_ROUND;
    }
    let a = n0 * side;
    let b = n1 * side;
    if (join == JOIN_MITER) {
        let cosine = clamp(dot(a, b), -1.0, 1.0);
        let limit = jf(job, 18u);
        // The miter's length over the half width: 1 / cos(θ / 2).
        let ratio = inverseSqrt(max((1.0 + cosine) * 0.5, 1e-12));
        if (ratio <= limit) {
            let m = apply(job, p + (a + b) * (h / (1.0 + cosine)));
            emit(out, a_from, m);
            emit(out, m, a_to);
            return;
        }
        emit(out, a_from, a_to);
        return;
    }
    if (join == JOIN_BEVEL) {
        emit(out, a_from, a_to);
        return;
    }
    if (back) {
        // Round through the direction it came in.
        half_turn(out, p, a, vec2<f32>(a.y, -a.x), a_from, a_to);
        return;
    }
    arc(out, p, a, b, a_from, a_to);
}

// Round from device `from` (at p + h·a) to device `to` (at p + h·b), the short way.
fn arc(out: ptr<function, Out>, p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, a_from: vec2<f32>, a_to: vec2<f32>) {
    let job = (*out).job;
    let h = jf(job, 17u);
    let angle = atan2(a.x * b.y - a.y * b.x, dot(a, b));
    let steps = clamp(u32(ceil(abs(angle) / jf(job, 20u))), 1u, ju(job, 21u));
    var prev = a_from;
    for (var i = 1u; i < steps; i += 1u) {
        let q = apply(job, p + rotate(a, angle * f32(i) / f32(steps)) * h);
        emit(out, prev, q);
        prev = q;
    }
    emit(out, prev, a_to);
}

// A half turn from device `from` (p + h·a) round to device `to` (p − h·a),
// through p + h·toward.
fn half_turn(out: ptr<function, Out>, p: vec2<f32>, a: vec2<f32>, toward: vec2<f32>, a_from: vec2<f32>, a_to: vec2<f32>) {
    let job = (*out).job;
    let h = jf(job, 17u);
    let sense = sign(a.x * toward.y - a.y * toward.x);
    let steps = clamp(u32(ceil(3.14159265 / jf(job, 20u))), 2u, ju(job, 21u));
    var prev = a_from;
    for (var i = 1u; i < steps; i += 1u) {
        let q = apply(job, p + rotate(a, sense * 3.14159265 * f32(i) / f32(steps)) * h);
        emit(out, prev, q);
        prev = q;
    }
    emit(out, prev, a_to);
}

fn put_head(item: u32, i: u32, v: vec2<f32>) {
    let base = params.n_points * 2u + item * HEAD + i * 2u;
    scratch[base] = v.x;
    scratch[base + 1u] = v.y;
}

fn head(item: u32, i: u32) -> vec2<f32> {
    let base = params.n_points * 2u + item * HEAD + i * 2u;
    return vec2<f32>(scratch[base], scratch[base + 1u]);
}

@compute @workgroup_size(64)
fn flatten(@builtin(global_invocation_id) gid: vec3<u32>) {
    let item = gid.x;
    if (item >= params.n_items) {
        return;
    }
    let curve = work[item * ITEM];
    let job = work[item * ITEM + 1u];
    let bound = work[item * ITEM + 3u];
    var out = Out(job, work[item * ITEM + 2u], 0u, work[item * ITEM + 4u]);
    let order = cu(curve, 8u);
    let d0 = device_point(item, 0u);
    let d1 = device_point(item, 1u);
    let d2 = device_point(item, 2u);
    let d3 = device_point(item, 3u);
    let last = order - 1u;
    let tol = jf(job, 19u);
    let n = clamp(wang(d0, d1, d2, d3, order, tol), 1u, bound);
    let stroke = (ju(job, JOB_FLAGS) & F_STROKE) != 0u;
    if (!stroke) {
        let end = device_point(item, last);
        var prev = d0;
        for (var i = 1u; i <= n; i += 1u) {
            var next: vec2<f32>;
            if (i == n) {
                next = end;
            } else {
                next = eval(d0, d1, d2, d3, order, f32(i) / f32(n));
            }
            emit(&out, prev, next);
            prev = next;
        }
        return;
    }
    // A stroke: its sides offset in the path's own space, then transformed.
    let h = jf(job, 17u);
    let l0 = local_point(curve, 0u);
    let l1 = local_point(curve, 1u);
    let l2 = local_point(curve, 2u);
    let l3 = local_point(curve, 3u);
    let lend = local_point(curve, last);
    // The smallest turn that needs a round joint inside the curve.
    let smooth_cos = jf(job, 23u);
    var a = l0;
    var b = eval(l0, l1, l2, l3, order, 1.0 / f32(n));
    if (n == 1u) {
        b = lend;
    }
    var u = b - a;
    var has = length(u) > 0.0;
    if (has) {
        u = normalize(u);
    }
    let first_u = u;
    var start_plus = apply(job, a + left_normal(u) * h);
    var start_minus = apply(job, a - left_normal(u) * h);
    put_head(item, 0u, start_plus);
    put_head(item, 1u, start_minus);
    for (var i = 1u; i <= n; i += 1u) {
        var c = b;
        var v = u;
        if (i < n) {
            if (i + 1u == n) {
                c = lend;
            } else {
                c = eval(l0, l1, l2, l3, order, f32(i + 1u) / f32(n));
            }
            let w = c - b;
            if (length(w) > 0.0) {
                v = normalize(w);
            }
        }
        var end_plus: vec2<f32>;
        var end_minus: vec2<f32>;
        var next_plus: vec2<f32>;
        var next_minus: vec2<f32>;
        let n0 = left_normal(u);
        let n1 = left_normal(v);
        let joint = i < n && any(u != v);
        if (joint && dot(n0, n1) >= smooth_cos) {
            // A gentle turn: the two sides meet at the miter point.
            let m = (n0 + n1) * (h / (1.0 + dot(n0, n1)));
            end_plus = apply(job, b + m);
            end_minus = apply(job, b - m);
            next_plus = end_plus;
            next_minus = end_minus;
        } else {
            end_plus = apply(job, b + n0 * h);
            end_minus = apply(job, b - n0 * h);
            next_plus = apply(job, b + n1 * h);
            next_minus = apply(job, b - n1 * h);
        }
        emit(&out, start_plus, end_plus);
        emit(&out, end_minus, start_minus);
        if (joint && dot(n0, n1) < smooth_cos) {
            connect(&out, b, n0, n1, 1.0, end_plus, next_plus, true);
            connect(&out, b, n0, n1, -1.0, next_minus, end_minus, true);
        }
        if (i == n) {
            put_head(item, 2u, end_plus);
            put_head(item, 3u, end_minus);
        }
        start_plus = next_plus;
        start_minus = next_minus;
        a = b;
        b = c;
        u = v;
    }
    put_head(item, 4u, l0);
    put_head(item, 5u, lend);
    put_head(item, 6u, first_u);
    put_head(item, 7u, u);
    scratch[params.n_points * 2u + item * HEAD + 16u] = select(0.0, 1.0, has);
}

// ---- joins and caps ----

@compute @workgroup_size(64)
fn joins(@builtin(global_invocation_id) gid: vec3<u32>) {
    let item = gid.x;
    if (item >= params.n_items) {
        return;
    }
    let curve = work[item * ITEM];
    let job = work[item * ITEM + 1u];
    if ((ju(job, JOB_FLAGS) & F_STROKE) == 0u) {
        return;
    }
    // Joins and caps write after the curve's own lines.
    var out = Out(job, work[item * ITEM + 2u] + work[item * ITEM + 4u], 0u, work[item * ITEM + 5u]);
    let flags = cu(curve, 9u);
    let h = jf(job, 17u);
    let cap = (ju(job, JOB_FLAGS) >> CAP_SHIFT) & CAP_MASK;
    let p = head(item, 4u);
    let u = head(item, 6u);
    if ((flags & C_DOT) != 0u) {
        // A subpath of no length: a dot where its caps would be.
        if (cap == CAP_ROUND) {
            let a = vec2<f32>(0.0, 1.0);
            let s = apply(job, p + a * h);
            let e = apply(job, p - a * h);
            half_turn(&out, p, a, vec2<f32>(-1.0, 0.0), s, e);
            half_turn(&out, p, -a, vec2<f32>(1.0, 0.0), e, s);
        } else if (cap == CAP_SQUARE) {
            let c0 = apply(job, p + vec2<f32>(-h, -h));
            let c1 = apply(job, p + vec2<f32>(h, -h));
            let c2 = apply(job, p + vec2<f32>(h, h));
            let c3 = apply(job, p + vec2<f32>(-h, h));
            emit(&out, c0, c1);
            emit(&out, c1, c2);
            emit(&out, c2, c3);
            emit(&out, c3, c0);
        }
        return;
    }
    if (scratch[params.n_points * 2u + item * HEAD + 16u] == 0.0) {
        return;
    }
    let start_plus = head(item, 0u);
    let start_minus = head(item, 1u);
    if ((flags & C_JOIN_IN) != 0u) {
        // Joined to the curve before: its last direction and ends.
        let prev = item + cu(curve, 10u) - curve;
        let pu = head(prev, 7u);
        let tail_plus = head(prev, 2u);
        let tail_minus = head(prev, 3u);
        connect(&out, p, left_normal(pu), left_normal(u), 1.0, tail_plus, start_plus, false);
        connect(&out, p, left_normal(pu), left_normal(u), -1.0, start_minus, tail_minus, false);
    } else if ((flags & C_START_CAP) != 0u) {
        start_cap(&out, p, u, cap, start_minus, start_plus);
    }
    if ((flags & C_END_CAP) != 0u) {
        let e = head(item, 5u);
        let v = head(item, 7u);
        end_cap(&out, e, v, cap, head(item, 2u), head(item, 3u));
    }
}

// From the minus side's end to the plus side's start, at `p` going `u`.
fn start_cap(out: ptr<function, Out>, p: vec2<f32>, u: vec2<f32>, cap: u32, a_from: vec2<f32>, a_to: vec2<f32>) {
    let job = (*out).job;
    let h = jf(job, 17u);
    let n = left_normal(u);
    if (cap == CAP_ROUND) {
        half_turn(out, p, -n, -u, a_from, a_to);
    } else if (cap == CAP_SQUARE) {
        let a = apply(job, p - n * h - u * h);
        let b = apply(job, p + n * h - u * h);
        emit(out, a_from, a);
        emit(out, a, b);
        emit(out, b, a_to);
    } else {
        emit(out, a_from, a_to);
    }
}

// From the plus side's end to the minus side's start, at `p` going `u`.
fn end_cap(out: ptr<function, Out>, p: vec2<f32>, u: vec2<f32>, cap: u32, a_from: vec2<f32>, a_to: vec2<f32>) {
    let job = (*out).job;
    let h = jf(job, 17u);
    let n = left_normal(u);
    if (cap == CAP_ROUND) {
        half_turn(out, p, n, u, a_from, a_to);
    } else if (cap == CAP_SQUARE) {
        let a = apply(job, p + n * h + u * h);
        let b = apply(job, p - n * h + u * h);
        emit(out, a_from, a);
        emit(out, a, b);
        emit(out, b, a_to);
    } else {
        emit(out, a_from, a_to);
    }
}

// ---- binning ----

// The job a thread of a dispatch over ranges `first`..`first + count`
// (pairs: first thread, job) works for, and its index within the job.
fn find(first: u32, count: u32, t: u32) -> vec2<u32> {
    var lo = 0u;
    var hi = count;
    while (hi - lo > 1u) {
        let mid = (lo + hi) / 2u;
        if (work[first + mid * 2u] <= t) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    return vec2<u32>(work[first + lo * 2u + 1u], t - work[first + lo * 2u]);
}

fn floor_div(a: i32, b: i32) -> i32 {
    let q = a / b;
    if ((a % b != 0) && ((a < 0) != (b < 0))) {
        return q - 1;
    }
    return q;
}

fn tile_index(job: u32, c: i32, r: i32) -> i32 {
    let tx = c - ji(job, 9u);
    let ty = r - ji(job, 10u);
    if (tx < 0 || ty < 0 || tx >= i32(ju(job, 11u)) || ty >= i32(ju(job, 12u))) {
        return -1;
    }
    return i32(ju(job, 8u)) + ty * i32(ju(job, 11u)) + tx;
}

fn add_backdrop(job: u32, c: i32, r: i32, delta: i32) {
    // Into every tile of row r right of column c; columns left of the
    // rectangle start its first.
    let col = max(c + 1, ji(job, 9u));
    let t = tile_index(job, col, r);
    if (t >= 0) {
        atomicAdd(&tiles[u32(t) * 4u], bitcast<u32>(delta));
    }
}

// Walk the tiles line `l` passes, in order: counting (`writing` false) or
// writing each into its tiles' lists.
fn walk(job: u32, l: vec4<f32>, writing: bool) {
    let x0 = i32(round(l.x * FIX));
    let y0 = i32(round(l.y * FIX));
    let x1 = i32(round(l.z * FIX));
    let y1 = i32(round(l.w * FIX));
    let dx = x1 - x0;
    let dy = y1 - y0;
    var c = floor_div(x0, TILE_FIX);
    var r = floor_div(y0, TILE_FIX);
    let ce = floor_div(x1, TILE_FIX);
    let re = floor_div(y1, TILE_FIX);
    let sx = select(select(0, -1, dx < 0), 1, dx > 0);
    let sy = select(select(0, -1, dy < 0), 1, dy > 0);
    // Where the walk entered the current tile across its left edge (device y).
    var edge = NO_EDGE;
    for (var step = 0u; step < 64u; step += 1u) {
        let done = c == ce && r == re;
        // Which boundary comes next.
        var go_x = false;
        if (!done) {
            if (c == ce) {
                go_x = false;
            } else if (r == re) {
                go_x = true;
            } else {
                let nx = select(c * TILE_FIX, (c + 1) * TILE_FIX, sx > 0);
                let ny = select(r * TILE_FIX, (r + 1) * TILE_FIX, sy > 0);
                go_x = abs(nx - x0) * abs(dy) <= abs(ny - y0) * abs(dx);
            }
        }
        // Leaving across this tile's left edge also marks it.
        var here_edge = edge;
        if (!done && go_x && sx < 0) {
            here_edge = edge_y(l, f32(c) * TILE, r);
        }
        visit(job, l, c, r, here_edge, writing);
        if (done) {
            break;
        }
        edge = NO_EDGE;
        if (go_x) {
            if (sx > 0) {
                c += 1;
                edge = edge_y(l, f32(c) * TILE, r);
            } else {
                c -= 1;
            }
        } else {
            if (sy > 0) {
                r += 1;
                if (!writing) {
                    add_backdrop(job, c, r, 1);
                }
            } else {
                if (!writing) {
                    add_backdrop(job, c, r, -1);
                }
                r -= 1;
            }
        }
    }
}

// Where line `l` crosses x = `x`, kept within row `r`.
fn edge_y(l: vec4<f32>, x: f32, r: i32) -> f32 {
    let top = f32(r) * TILE;
    var y = l.y;
    if (l.z != l.x) {
        y = l.y + (l.w - l.y) * (x - l.x) / (l.z - l.x);
    }
    return clamp(y, top, top + TILE);
}

fn visit(job: u32, l: vec4<f32>, c: i32, r: i32, edge: f32, writing: bool) {
    let t = tile_index(job, c, r);
    if (t < 0) {
        return;
    }
    let ti = u32(t) * 4u;
    if (!writing) {
        atomicAdd(&tiles[ti + 1u], 1u);
        return;
    }
    let slot = atomicAdd(&tiles[ti + 3u], 1u);
    if (slot >= atomicLoad(&tiles[ti + 1u])) {
        return;
    }
    let at = ju(job, 13u) + atomicLoad(&tiles[ti + 2u]) + slot;
    if (at >= ju(job, 13u) + ju(job, 14u)) {
        return;
    }
    // The least x of the part right of the tile's left edge.
    let left = f32(c) * TILE;
    let min_x = max(min(l.x, l.z), left);
    entries[at * 2u] = l;
    entries[at * 2u + 1u] = vec4<f32>(edge, min_x, 0.0, 0.0);
}

@compute @workgroup_size(64)
fn count(@builtin(global_invocation_id) gid: vec3<u32>) {
    let t = gid.x;
    if (t >= params.n_lines) {
        return;
    }
    let f = find(params.line_ranges, params.n_line_ranges, t);
    let job = f.x;
    if (f.y >= ju(job, 7u)) {
        return;
    }
    let l = lines[ju(job, 6u) + f.y];
    if (all(l.xy == l.zw)) {
        return;
    }
    walk(job, l, false);
}

@compute @workgroup_size(64)
fn scan(@builtin(global_invocation_id) gid: vec3<u32>) {
    let t = gid.x;
    if (t >= params.n_rows) {
        return;
    }
    let f = find(params.row_ranges, params.n_row_ranges, t);
    let job = f.x;
    let row = f.y;
    let w = ju(job, 11u);
    if (row >= ju(job, 12u)) {
        return;
    }
    let first = ju(job, 8u) + row * w;
    var winding = 0u;
    var total = 0u;
    for (var i = 0u; i < w; i += 1u) {
        let ti = (first + i) * 4u;
        winding += atomicLoad(&tiles[ti]);
        atomicStore(&tiles[ti], winding);
        total += atomicLoad(&tiles[ti + 1u]);
    }
    var at = atomicAdd(&state[ju(job, 25u)], total);
    for (var i = 0u; i < w; i += 1u) {
        let ti = (first + i) * 4u;
        atomicStore(&tiles[ti + 2u], at);
        at += atomicLoad(&tiles[ti + 1u]);
    }
}

@compute @workgroup_size(64)
fn write(@builtin(global_invocation_id) gid: vec3<u32>) {
    let t = gid.x;
    if (t >= params.n_lines) {
        return;
    }
    let f = find(params.line_ranges, params.n_line_ranges, t);
    let job = f.x;
    if (f.y >= ju(job, 7u)) {
        return;
    }
    let l = lines[ju(job, 6u) + f.y];
    if (all(l.xy == l.zw)) {
        return;
    }
    walk(job, l, true);
}

fn covered(backdrop: i32, even_odd: bool) -> bool {
    if (even_odd) {
        return (backdrop & 1) != 0;
    }
    return backdrop != 0;
}

@compute @workgroup_size(64)
fn compact(@builtin(global_invocation_id) gid: vec3<u32>) {
    let t = gid.x;
    if (t >= params.n_rows) {
        return;
    }
    let f = find(params.row_ranges, params.n_row_ranges, t);
    let job = f.x;
    let row = f.y;
    let w = ju(job, 11u);
    if (row >= ju(job, 12u)) {
        return;
    }
    let even_odd = (ju(job, JOB_FLAGS) & F_EVEN_ODD) != 0u;
    let first = ju(job, 8u) + row * w;
    let inst = ju(job, 15u) + row * w;
    let entry_base = ju(job, 13u);
    var run_start = 0u;
    var run = 0u;
    for (var i = 0u; i < w; i += 1u) {
        let ti = (first + i) * 4u;
        let n = atomicLoad(&tiles[ti + 1u]);
        let backdrop = bitcast<i32>(atomicLoad(&tiles[ti]));
        let solid = n == 0u && covered(backdrop, even_odd);
        if (run > 0u && !solid) {
            instances[inst + run_start] = vec4<u32>(job, row * w + run_start, run, 1u);
            run = 0u;
        }
        if (solid) {
            if (run == 0u) {
                run_start = i;
            }
            run += 1u;
            instances[inst + i] = vec4<u32>(0u);
            continue;
        }
        if (n == 0u) {
            instances[inst + i] = vec4<u32>(0u);
            continue;
        }
        // Sort the tile's list by least x (insertion: lists are short).
        let off = entry_base + atomicLoad(&tiles[ti + 2u]);
        for (var a = 1u; a < n; a += 1u) {
            let l = entries[(off + a) * 2u];
            let e = entries[(off + a) * 2u + 1u];
            var b = a;
            while (b > 0u && entries[(off + b - 1u) * 2u + 1u].y > e.y) {
                entries[(off + b) * 2u] = entries[(off + b - 1u) * 2u];
                entries[(off + b) * 2u + 1u] = entries[(off + b - 1u) * 2u + 1u];
                b -= 1u;
            }
            entries[(off + b) * 2u] = l;
            entries[(off + b) * 2u + 1u] = e;
        }
        instances[inst + i] = vec4<u32>(job, row * w + i, 1u, 0u);
    }
    if (run > 0u) {
        instances[inst + run_start] = vec4<u32>(job, row * w + run_start, run, 1u);
    }
}
