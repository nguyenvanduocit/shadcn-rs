//! The path passes of `path_compute.wgsl` and the coverage of
//! `path_render.wgsl` in plain Rust, step for step, for the tests to hold
//! the GPU to; and an oracle in `f64` that needs no tiles at all: the exact
//! area each line leaves to its right in a pixel, summed over every line,
//! the fill rule applied to the sum.
#![expect(
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    reason = "a transcription of the shaders, whose integer and float steps it repeats"
)]

use super::{
    C_DOT, C_END_CAP, C_JOIN_IN, C_START_CAP, CurveInfo, StrokeParams, TOLERANCE, TileRect, line_bounds, records,
    scales, segment_bound,
};
use crate::shape::{Cap, Draw, FillRule, Join, Path, PathStyle};

const TILE: f32 = 16.0;
const FIX: f32 = 256.0;
const TILE_FIX: i32 = 4096;
const PIECE: f32 = 64.0;
const NO_EDGE: f32 = -1.0e30;

type V = [f32; 2];

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1]]
}
fn mul(a: V, k: f32) -> V {
    [a[0] * k, a[1] * k]
}
fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn len(a: V) -> f32 {
    dot(a, a).sqrt()
}
fn norm(a: V) -> V {
    mul(a, 1.0 / len(a))
}
fn left_normal(u: V) -> V {
    [-u[1], u[0]]
}
fn rotate(v: V, a: f32) -> V {
    let (s, c) = a.sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
}

/// One job as the passes see it.
struct Job {
    m: [f32; 6],
    rect: TileRect,
    stroke: Option<StrokeParams>,
}

impl Job {
    fn apply(&self, p: V) -> V {
        super::apply(self.m, p)
    }
}

struct Out<'a> {
    job: &'a Job,
    lines: &'a mut Vec<[f32; 4]>,
    count: u32,
    bound: u32,
}

fn snap(p: V) -> V {
    [(p[0] * FIX).round() / FIX, (p[1] * FIX).round() / FIX]
}

fn put(out: &mut Out<'_>, a: V, b: V) {
    if a == b {
        return;
    }
    if out.count < out.bound {
        out.lines.push([a[0], a[1], b[0], b[1]]);
        out.count += 1;
    }
}

fn pieces(out: &mut Out<'_>, a: V, b: V) {
    let d = sub(b, a);
    let n = (d[0].abs().max(d[1].abs()) / PIECE).ceil().max(1.0) as u32;
    let mut prev = snap(a);
    for i in 1..=n {
        let next = if i == n {
            snap(b)
        } else {
            snap(add(a, mul(d, i as f32 / n as f32)))
        };
        put(out, prev, next);
        prev = next;
    }
}

fn emit(out: &mut Out<'_>, a_in: V, b_in: V) {
    let r = out.job.rect;
    let x0 = r.x0 as f32 * TILE;
    let y0 = r.y0 as f32 * TILE;
    let x1 = x0 + r.w as f32 * TILE;
    let y1 = y0 + r.h as f32 * TILE;
    let left = x0 - 1.0 / FIX;
    let top = y0 - 1.0 / FIX;
    let (mut a, mut b) = (a_in, b_in);
    if a[1].max(b[1]) < top || a[1].min(b[1]) > y1 {
        return;
    }
    if a[1] < top {
        a = [a[0] + (b[0] - a[0]) * (top - a[1]) / (b[1] - a[1]), top];
    } else if a[1] > y1 {
        a = [a[0] + (b[0] - a[0]) * (y1 - a[1]) / (b[1] - a[1]), y1];
    }
    if b[1] < top {
        b = [b[0] + (a[0] - b[0]) * (top - b[1]) / (a[1] - b[1]), top];
    } else if b[1] > y1 {
        b = [b[0] + (a[0] - b[0]) * (y1 - b[1]) / (a[1] - b[1]), y1];
    }
    if a[0].min(b[0]) >= x1 {
        return;
    }
    if a[0] > x1 {
        a = [x1, a[1] + (b[1] - a[1]) * (x1 - a[0]) / (b[0] - a[0])];
    }
    if b[0] > x1 {
        b = [x1, b[1] + (a[1] - b[1]) * (x1 - b[0]) / (a[0] - b[0])];
    }
    if a[0] < left && b[0] < left {
        pieces(out, [left, a[1]], [left, b[1]]);
        return;
    }
    if a[0] < left {
        let c = [left, a[1] + (b[1] - a[1]) * (left - a[0]) / (b[0] - a[0])];
        pieces(out, [left, a[1]], c);
        pieces(out, c, b);
        return;
    }
    if b[0] < left {
        let c = [left, b[1] + (a[1] - b[1]) * (left - b[0]) / (a[0] - b[0])];
        pieces(out, a, c);
        pieces(out, c, [left, b[1]]);
        return;
    }
    pieces(out, a, b);
}

fn eval(p: &[V; 4], order: u32, t: f32) -> V {
    let u = 1.0 - t;
    match order {
        2 => add(mul(p[0], u), mul(p[1], t)),
        3 => add(add(mul(p[0], u * u), mul(p[1], 2.0 * u * t)), mul(p[2], t * t)),
        _ => add(
            add(mul(p[0], u * u * u), mul(p[1], 3.0 * u * u * t)),
            add(mul(p[2], 3.0 * u * t * t), mul(p[3], t * t * t)),
        ),
    }
}

fn wang(p: &[V; 4], order: u32, tol: f32) -> u32 {
    match order {
        3 => {
            let m = len(add(sub(p[0], mul(p[1], 2.0)), p[2]));
            (0.25 * m / tol).sqrt().ceil().max(1.0) as u32
        }
        4 => {
            let m = len(add(sub(p[0], mul(p[1], 2.0)), p[2])).max(len(add(sub(p[1], mul(p[2], 2.0)), p[3])));
            (0.75 * m / tol).sqrt().ceil().max(1.0) as u32
        }
        _ => 1,
    }
}

#[expect(clippy::too_many_arguments, reason = "as the shader's `connect`")]
fn connect(out: &mut Out<'_>, p: V, n0: V, n1: V, side: f32, from: V, to: V, round_only: bool) {
    let s = out
        .job
        .stroke
        .unwrap_or_else(|| StrokeParams::new(&crate::shape::Stroke::new(1.0), 1.0));
    let h = s.half;
    let cross = n0[0] * n1[1] - n0[1] * n1[0];
    let back = cross.abs() < 1e-6 && dot(n0, n1) < 0.0;
    let outer = cross * side < 0.0 || (back && side > 0.0);
    if !outer {
        let c = out.job.apply(p);
        emit(out, from, c);
        emit(out, c, to);
        return;
    }
    let join = if round_only { Join::Round } else { s.join };
    let a = mul(n0, side);
    let b = mul(n1, side);
    match join {
        Join::Miter => {
            let cosine = dot(a, b).clamp(-1.0, 1.0);
            let ratio = 1.0 / ((1.0 + cosine) * 0.5).max(1e-12).sqrt();
            if ratio <= s.miter_limit {
                let m = out.job.apply(add(p, mul(add(a, b), h / (1.0 + cosine))));
                emit(out, from, m);
                emit(out, m, to);
                return;
            }
            emit(out, from, to);
        }
        Join::Bevel => emit(out, from, to),
        Join::Round => {
            if back {
                half_turn(out, p, a, [a[1], -a[0]], from, to);
            } else {
                arc(out, p, a, b, from, to);
            }
        }
    }
}

fn arc(out: &mut Out<'_>, p: V, a: V, b: V, from: V, to: V) {
    let s = out
        .job
        .stroke
        .unwrap_or_else(|| StrokeParams::new(&crate::shape::Stroke::new(1.0), 1.0));
    let angle = (a[0] * b[1] - a[1] * b[0]).atan2(dot(a, b));
    let steps = ((angle.abs() / s.theta).ceil() as u32).clamp(1, s.arc_bound);
    let mut prev = from;
    for i in 1..steps {
        let q = out
            .job
            .apply(add(p, mul(rotate(a, angle * i as f32 / steps as f32), s.half)));
        emit(out, prev, q);
        prev = q;
    }
    emit(out, prev, to);
}

fn half_turn(out: &mut Out<'_>, p: V, a: V, toward: V, from: V, to: V) {
    let s = out
        .job
        .stroke
        .unwrap_or_else(|| StrokeParams::new(&crate::shape::Stroke::new(1.0), 1.0));
    let pi = std::f32::consts::PI;
    let sense = (a[0] * toward[1] - a[1] * toward[0]).signum();
    let steps = ((pi / s.theta).ceil() as u32).clamp(2, s.arc_bound);
    let mut prev = from;
    for i in 1..steps {
        let q = out
            .job
            .apply(add(p, mul(rotate(a, sense * pi * i as f32 / steps as f32), s.half)));
        emit(out, prev, q);
        prev = q;
    }
    emit(out, prev, to);
}

/// What `flatten` leaves for `joins`: ends and directions.
#[derive(Clone, Copy, Default)]
struct Head {
    start_plus: V,
    start_minus: V,
    tail_plus: V,
    tail_minus: V,
    start: V,
    end: V,
    first_u: V,
    last_u: V,
    has: bool,
}

fn flatten(job: &Job, rec: &super::Record, info: &CurveInfo, s: f32, lines: &mut Vec<[f32; 4]>) -> Head {
    let (a_slots, _) = line_bounds(info, s, job.stroke.as_ref());
    let mut out = Out {
        job,
        lines,
        count: 0,
        bound: a_slots,
    };
    let order = rec.order;
    let l = rec.points;
    let d = l.map(|p| job.apply(p));
    let last = (order - 1) as usize;
    let n = wang(&d, order, TOLERANCE).clamp(1, segment_bound(info, s));
    let mut head = Head::default();
    let Some(st) = job.stroke else {
        let end = d[last];
        let mut prev = d[0];
        for i in 1..=n {
            let next = if i == n {
                end
            } else {
                eval(&d, order, i as f32 / n as f32)
            };
            emit(&mut out, prev, next);
            prev = next;
        }
        return head;
    };
    let h = st.half;
    let lend = l[last];
    let a = l[0];
    let mut b = if n == 1 { lend } else { eval(&l, order, 1.0 / n as f32) };
    let mut u = sub(b, a);
    let has = len(u) > 0.0;
    if has {
        u = norm(u);
    }
    let first_u = u;
    let mut start_plus = job.apply(add(a, mul(left_normal(u), h)));
    let mut start_minus = job.apply(sub(a, mul(left_normal(u), h)));
    head.start_plus = start_plus;
    head.start_minus = start_minus;
    for i in 1..=n {
        let mut c = b;
        let mut v = u;
        if i < n {
            c = if i + 1 == n {
                lend
            } else {
                eval(&l, order, (i + 1) as f32 / n as f32)
            };
            let w = sub(c, b);
            if len(w) > 0.0 {
                v = norm(w);
            }
        }
        let n0 = left_normal(u);
        let n1 = left_normal(v);
        let joint = i < n && u != v;
        let (end_plus, end_minus, next_plus, next_minus);
        if joint && dot(n0, n1) >= st.smooth {
            let m = mul(add(n0, n1), h / (1.0 + dot(n0, n1)));
            end_plus = job.apply(add(b, m));
            end_minus = job.apply(sub(b, m));
            next_plus = end_plus;
            next_minus = end_minus;
        } else {
            end_plus = job.apply(add(b, mul(n0, h)));
            end_minus = job.apply(sub(b, mul(n0, h)));
            next_plus = job.apply(add(b, mul(n1, h)));
            next_minus = job.apply(sub(b, mul(n1, h)));
        }
        emit(&mut out, start_plus, end_plus);
        emit(&mut out, end_minus, start_minus);
        if joint && dot(n0, n1) < st.smooth {
            connect(&mut out, b, n0, n1, 1.0, end_plus, next_plus, true);
            connect(&mut out, b, n0, n1, -1.0, next_minus, end_minus, true);
        }
        if i == n {
            head.tail_plus = end_plus;
            head.tail_minus = end_minus;
        }
        start_plus = next_plus;
        start_minus = next_minus;
        b = c;
        u = v;
    }
    head.start = l[0];
    head.end = lend;
    head.first_u = first_u;
    head.last_u = u;
    head.has = has;
    head
}

fn joins(
    job: &Job,
    rec: &super::Record,
    info: &CurveInfo,
    s: f32,
    heads: &[Head],
    me: usize,
    lines: &mut Vec<[f32; 4]>,
) {
    let Some(st) = job.stroke else { return };
    let (_, b_slots) = line_bounds(info, s, job.stroke.as_ref());
    let mut out = Out {
        job,
        lines,
        count: 0,
        bound: b_slots,
    };
    let h = st.half;
    let head = heads[me];
    let p = head.start;
    let u = head.first_u;
    if rec.flags & C_DOT != 0 {
        let p = rec.points[0];
        match st.cap {
            Cap::Round => {
                let a = [0.0, 1.0];
                let s0 = job.apply(add(p, mul(a, h)));
                let e = job.apply(sub(p, mul(a, h)));
                half_turn(&mut out, p, a, [-1.0, 0.0], s0, e);
                half_turn(&mut out, p, mul(a, -1.0), [1.0, 0.0], e, s0);
            }
            Cap::Square => {
                let c = [[-h, -h], [h, -h], [h, h], [-h, h]].map(|o| job.apply(add(p, o)));
                emit(&mut out, c[0], c[1]);
                emit(&mut out, c[1], c[2]);
                emit(&mut out, c[2], c[3]);
                emit(&mut out, c[3], c[0]);
            }
            Cap::Butt => {}
        }
        return;
    }
    if !head.has {
        return;
    }
    if rec.flags & C_JOIN_IN != 0 {
        let prev = heads[rec.prev as usize];
        let pu = prev.last_u;
        connect(
            &mut out,
            p,
            left_normal(pu),
            left_normal(u),
            1.0,
            prev.tail_plus,
            head.start_plus,
            false,
        );
        connect(
            &mut out,
            p,
            left_normal(pu),
            left_normal(u),
            -1.0,
            head.start_minus,
            prev.tail_minus,
            false,
        );
    } else if rec.flags & C_START_CAP != 0 {
        let n = left_normal(u);
        match st.cap {
            Cap::Round => half_turn(
                &mut out,
                p,
                mul(n, -1.0),
                mul(u, -1.0),
                head.start_minus,
                head.start_plus,
            ),
            Cap::Square => {
                let a = job.apply(sub(sub(p, mul(n, h)), mul(u, h)));
                let b = job.apply(sub(add(p, mul(n, h)), mul(u, h)));
                emit(&mut out, head.start_minus, a);
                emit(&mut out, a, b);
                emit(&mut out, b, head.start_plus);
            }
            Cap::Butt => emit(&mut out, head.start_minus, head.start_plus),
        }
    }
    if rec.flags & C_END_CAP != 0 {
        let e = head.end;
        let v = head.last_u;
        let n = left_normal(v);
        match st.cap {
            Cap::Round => half_turn(&mut out, e, n, v, head.tail_plus, head.tail_minus),
            Cap::Square => {
                let a = job.apply(add(add(e, mul(n, h)), mul(v, h)));
                let b = job.apply(add(sub(e, mul(n, h)), mul(v, h)));
                emit(&mut out, head.tail_plus, a);
                emit(&mut out, a, b);
                emit(&mut out, b, head.tail_minus);
            }
            Cap::Butt => emit(&mut out, head.tail_plus, head.tail_minus),
        }
    }
}

/// The lines `flatten` and `joins` make of `path` drawn with `style`
/// through `m` (binned coordinates), for tile rectangle `rect`.
pub(crate) fn lines(path: &Path, style: &PathStyle, m: [f32; 6], rect: TileRect) -> Vec<[f32; 4]> {
    let (s, _) = scales(m);
    let stroke = match &style.draw {
        Draw::Fill => None,
        Draw::Stroke(st) => Some(StrokeParams::new(st, s)),
    };
    let dashed = match &style.draw {
        Draw::Stroke(st) => st.dash.as_ref().map(|d| path.dashed(d)),
        Draw::Fill => None,
    };
    let path = dashed.as_ref().unwrap_or(path);
    let job = Job { m, rect, stroke };
    let recs = records(path, stroke.is_some());
    let mut out = Vec::new();
    let heads: Vec<Head> = recs.iter().map(|(r, i)| flatten(&job, r, i, s, &mut out)).collect();
    for (k, (r, i)) in recs.iter().enumerate() {
        joins(&job, r, i, s, &heads, k, &mut out);
    }
    out
}

/// One tile as the binning passes leave it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Tile {
    pub backdrop: i32,
    /// Lines, where each crosses the tile's left edge (or `NO_EDGE`), and
    /// the least x of its part right of that edge, sorted by it.
    pub entries: Vec<([f32; 4], f32, f32)>,
}

pub(crate) struct Binned {
    pub rect: TileRect,
    pub tiles: Vec<Tile>,
    pub even_odd: bool,
}

fn floor_div(a: i32, b: i32) -> i32 {
    a.div_euclid(b)
}

fn edge_y(l: [f32; 4], x: f32, r: i32) -> f32 {
    let top = r as f32 * TILE;
    let y = if l[2] != l[0] {
        l[1] + (l[3] - l[1]) * (x - l[0]) / (l[2] - l[0])
    } else {
        l[1]
    };
    y.clamp(top, top + TILE)
}

/// The tiles line `l` passes, with where it enters each across its left
/// edge, and the winding it adds to tile rows: `count` and `write`'s walk.
fn walk(l: [f32; 4], mut visit: impl FnMut(i32, i32, f32), mut backdrop: impl FnMut(i32, i32, i32)) {
    let x0 = (l[0] * FIX).round() as i32;
    let y0 = (l[1] * FIX).round() as i32;
    let x1 = (l[2] * FIX).round() as i32;
    let y1 = (l[3] * FIX).round() as i32;
    let (dx, dy) = (x1 - x0, y1 - y0);
    let mut c = floor_div(x0, TILE_FIX);
    let mut r = floor_div(y0, TILE_FIX);
    let ce = floor_div(x1, TILE_FIX);
    let re = floor_div(y1, TILE_FIX);
    let sx = dx.signum();
    let sy = dy.signum();
    let mut edge = NO_EDGE;
    for _ in 0..64 {
        let done = c == ce && r == re;
        let mut go_x = false;
        if !done {
            if c == ce {
                go_x = false;
            } else if r == re {
                go_x = true;
            } else {
                let nx = if sx > 0 { (c + 1) * TILE_FIX } else { c * TILE_FIX };
                let ny = if sy > 0 { (r + 1) * TILE_FIX } else { r * TILE_FIX };
                go_x = (nx - x0).abs() * dy.abs() <= (ny - y0).abs() * dx.abs();
            }
        }
        let mut here = edge;
        if !done && go_x && sx < 0 {
            here = edge_y(l, c as f32 * TILE, r);
        }
        visit(c, r, here);
        if done {
            break;
        }
        edge = NO_EDGE;
        if go_x {
            if sx > 0 {
                c += 1;
                edge = edge_y(l, c as f32 * TILE, r);
            } else {
                c -= 1;
            }
        } else if sy > 0 {
            r += 1;
            backdrop(c, r, 1);
        } else {
            backdrop(c, r, -1);
            r -= 1;
        }
    }
}

/// `count`, `scan`, `write` and `compact`'s sort: the tiles of `rect`.
pub(crate) fn bin(lines: &[[f32; 4]], rect: TileRect, rule: FillRule) -> Binned {
    let (w, h) = (rect.w as i32, rect.h as i32);
    let mut tiles = vec![Tile::default(); (rect.w * rect.h) as usize];
    let index = |c: i32, r: i32| {
        let (tx, ty) = (c - rect.x0, r - rect.y0);
        (tx >= 0 && ty >= 0 && tx < w && ty < h).then(|| (ty * w + tx) as usize)
    };
    let mut deltas = vec![0_i32; tiles.len()];
    for l in lines {
        if l[0] == l[2] && l[1] == l[3] {
            continue;
        }
        let mut visits = Vec::new();
        walk(
            *l,
            |c, r, e| visits.push((c, r, e)),
            |c, r, d| {
                let col = (c + 1).max(rect.x0);
                if let Some(i) = index(col, r) {
                    deltas[i] += d;
                }
            },
        );
        for (c, r, e) in visits {
            if let Some(i) = index(c, r) {
                let left = c as f32 * TILE;
                tiles[i].entries.push((*l, e, l[0].min(l[2]).max(left)));
            }
        }
    }
    for row in 0..h {
        let mut winding = 0;
        for col in 0..w {
            let i = (row * w + col) as usize;
            winding += deltas[i];
            tiles[i].backdrop = winding;
        }
    }
    for t in &mut tiles {
        t.entries.sort_by(|a, b| a.2.total_cmp(&b.2));
    }
    Binned {
        rect,
        tiles,
        even_odd: rule == FillRule::EvenOdd,
    }
}

fn right_area(u: f32) -> f32 {
    if u <= 0.0 {
        u
    } else if u >= 1.0 {
        0.5
    } else {
        u - 0.5 * u * u
    }
}

fn line_area(l: [f32; 4], px: f32, py: f32) -> f32 {
    let (mut a, mut b) = ([l[0], l[1]], [l[2], l[3]]);
    if a[1] == b[1] || (a[0] < 0.0 && b[0] < 0.0) {
        return 0.0;
    }
    if a[0] < 0.0 {
        a = [0.0, a[1] + (b[1] - a[1]) * (0.0 - a[0]) / (b[0] - a[0])];
    } else if b[0] < 0.0 {
        b = [0.0, b[1] + (a[1] - b[1]) * (0.0 - b[0]) / (a[0] - b[0])];
    }
    let d = (b[1] - a[1]).signum();
    let ya = a[1].min(b[1]).max(py);
    let yb = a[1].max(b[1]).min(py + 1.0);
    if yb <= ya {
        return 0.0;
    }
    let inv = (b[0] - a[0]) / (b[1] - a[1]);
    let xa = a[0] + (ya - a[1]) * inv - px;
    let xb = a[0] + (yb - a[1]) * inv - px;
    let avg = if (xb - xa).abs() > 1e-6 {
        (right_area(xb) - right_area(xa)) / (xb - xa)
    } else {
        (1.0 - 0.5 * (xa + xb)).clamp(0.0, 1.0)
    };
    d * (yb - ya) * avg
}

/// `path_render.wgsl`'s coverage of pixel (`x`, `y`) (binned coordinates).
pub(crate) fn coverage(b: &Binned, x: i32, y: i32) -> f32 {
    let (c, r) = (floor_div(x, 16), floor_div(y, 16));
    let (tx, ty) = (c - b.rect.x0, r - b.rect.y0);
    if tx < 0 || ty < 0 || tx >= b.rect.w as i32 || ty >= b.rect.h as i32 {
        return 0.0;
    }
    let tile = &b.tiles[(ty * b.rect.w as i32 + tx) as usize];
    let origin = [c as f32 * TILE, r as f32 * TILE];
    let px = (x - c * 16) as f32;
    let py = (y - r * 16) as f32;
    let mut acc = tile.backdrop as f32;
    for (l, e, min_x) in &tile.entries {
        if min_x - origin[0] >= px + 1.0 {
            break;
        }
        let l = [l[0] - origin[0], l[1] - origin[1], l[2] - origin[0], l[3] - origin[1]];
        if *e > -1.0e29 {
            let s = -(l[2] - l[0]).signum();
            acc += s * (py + 1.0 - (e - origin[1])).clamp(0.0, 1.0);
        }
        acc += line_area(l, px, py);
    }
    rule(f64::from(acc), b.even_odd) as f32
}

fn rule(acc: f64, even_odd: bool) -> f64 {
    if even_odd {
        (acc - 2.0 * (0.5 * acc).round()).abs()
    } else {
        acc.abs().min(1.0)
    }
}

/// The oracle: in `f64`, with no tiles, the area pixel (`x`, `y`) has right
/// of each line (signed by its direction), summed, the fill rule applied.
pub(crate) fn exact(lines: &[[f32; 4]], x: i32, y: i32, even_odd: bool) -> f64 {
    let (px, py) = (f64::from(x), f64::from(y));
    let mut acc = 0.0;
    for l in lines {
        let [ax, ay, bx, by] = l.map(f64::from);
        if ay == by {
            continue;
        }
        let d = (by - ay).signum();
        let (y0, y1) = (ay.min(by).max(py), ay.max(by).min(py + 1.0));
        if y1 <= y0 {
            continue;
        }
        let at = |yy: f64| ax + (bx - ax) * (yy - ay) / (by - ay);
        // ∫ clamp(px + 1 − x(y), 0, 1) dy over [y0, y1], exactly: x(y) is
        // linear, so split where it crosses px and px + 1.
        let mut cuts = vec![y0, y1];
        for xx in [px, px + 1.0] {
            if bx != ax {
                let yy = ay + (by - ay) * (xx - ax) / (bx - ax);
                if yy > y0 && yy < y1 {
                    cuts.push(yy);
                }
            }
        }
        cuts.sort_by(f64::total_cmp);
        let g = |xx: f64| (px + 1.0 - xx).clamp(0.0, 1.0);
        let mut area = 0.0;
        for w in cuts.windows(2) {
            let (s, e) = (w[0], w[1]);
            // Linear on each piece: the trapezoid rule is exact.
            area += (e - s) * (g(at(s)) + g(at(e))) / 2.0;
        }
        acc += d * area;
    }
    rule(acc, even_odd)
}
