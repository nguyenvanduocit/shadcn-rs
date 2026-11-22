//! An infinite canvas (ADR-0017): a world of vector objects in `f64`
//! coordinates under an `f64` camera, drawn sharp at any zoom from 10⁻⁹
//! to 10⁹ without jitter.
//!
//! - Every object keeps its `f64` source and an anchor, its box's corner
//!   (`f64`); what the GPU sees is its path in small `f32` coordinates
//!   about the anchor (holding only the object's own extent) and one
//!   matrix, computed each frame in `f64` relative to the camera
//!   ([`Camera::matrix`]), so no `f32` ever holds a world coordinate.
//! - An object whose size on screen passes [`REANCHOR`] pixels is drawn
//!   from its source anchored at the camera's centre, so its `f32` path
//!   never holds more than the screen's few thousand pixels of detail.
//! - Culling and hit testing query a dynamic AABB tree ([`aabb`]); the draw
//!   order is fractional z keys ([`zkey`]), so a raise or an insert never
//!   renumbers the rest.
//! - Level of detail by size on screen: under [`LOD_BOX`] pixels an object
//!   is a box of its colour, and a run of boxes is one instanced draw
//!   (`boxes`); under [`LOD_DROP`] its area and colour gather per device
//!   pixel with the others' into one box a pixel.
//! - A canvas past [`TILE_ABOVE`] visible objects draws from a tile cache
//!   on the render thread (`gpu::world`): 256 px tiles rasterized by the
//!   same path renderer, raster scale stepping at most 2× toward the zoom
//!   while it moves and snapping to a tiling within 1.2×, the exact zoom
//!   rastered when it settles, a missing tile filled by a coarser one
//!   scaled up, never blank. A tile is kept while no change reaches it.
//! - Objects being dragged or edited are lifted out of the tiles
//!   ([`World::lift`]) and drawn live each frame in their place in the
//!   order; a drag rasters no tile ([`canvas_editor`]).

pub mod aabb;
pub(crate) mod boxes;
pub(crate) mod plan;
pub(crate) mod planner;
pub(crate) mod tile;
pub mod zkey;

use std::rc::Rc;
use std::sync::Arc;

use self::aabb::{Aabb, AabbTree};
use self::zkey::ZKey;
use crate::chunk::{Drawing, Picture};
use crate::color::Color;
use crate::element::{Drawn, drawing};
use crate::figure::{Path, PathBuilder, PathStyle};
use crate::geometry::Bounds;
use crate::reactive::Signal;
use crate::scene::Gradient;
use crate::scene::Item;

/// Screen pixels past which an object is drawn anchored at the camera.
pub const REANCHOR: f64 = 262_144.0;
/// Objects smaller on screen than this (device pixels) are not drawn on
/// their own: their area and colour join the pixel they fall in.
pub const LOD_DROP: f64 = 0.5;
/// Objects smaller on screen than this are drawn as a box of their colour.
pub const LOD_BOX: f64 = 4.0;
/// Visible objects past which a canvas draws from tiles.
pub const TILE_ABOVE: usize = 4096;

/// Where the canvas looks: the world point at its centre, logical pixels
/// per world unit, and how far the view is turned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub center: (f64, f64),
    pub zoom: f64,
    /// Radians the world is turned on screen, clockwise (y points down).
    pub rotation: f64,
    /// The camera is between gestures: a tiled canvas rasters at its exact
    /// zoom and turn, a moving one keeps the tiles it has.
    pub settled: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: (0.0, 0.0),
            zoom: 1.0,
            rotation: 0.0,
            settled: true,
        }
    }
}

/// `v` turned by the angle whose cosine and sine are `cs`.
#[must_use]
pub(crate) fn turn(v: (f64, f64), cs: (f64, f64)) -> (f64, f64) {
    (v.0 * cs.0 - v.1 * cs.1, v.0 * cs.1 + v.1 * cs.0)
}

/// The cosine and sine of `angle`, exact at no turn.
#[must_use]
pub(crate) fn cos_sin(angle: f64) -> (f64, f64) {
    if angle == 0.0 {
        (1.0, 0.0)
    } else {
        (angle.cos(), angle.sin())
    }
}

impl Camera {
    /// The camera's turn as a cosine and a sine.
    #[must_use]
    pub fn turn(&self) -> (f64, f64) {
        cos_sin(self.rotation)
    }

    /// World point `w` on screen (logical pixels from the view's top left,
    /// the view `size`).
    #[must_use]
    pub fn to_screen(&self, w: (f64, f64), size: (f64, f64)) -> (f64, f64) {
        let d = turn(
            ((w.0 - self.center.0) * self.zoom, (w.1 - self.center.1) * self.zoom),
            self.turn(),
        );
        (d.0 + size.0 / 2.0, d.1 + size.1 / 2.0)
    }

    /// Screen point `s` in the world.
    #[must_use]
    pub fn to_world(&self, s: (f64, f64), size: (f64, f64)) -> (f64, f64) {
        let (c, sn) = self.turn();
        let d = turn((s.0 - size.0 / 2.0, s.1 - size.1 / 2.0), (c, -sn));
        (d.0 / self.zoom + self.center.0, d.1 / self.zoom + self.center.1)
    }

    /// The world the view shows: the box around its four corners.
    #[must_use]
    pub fn visible(&self, size: (f64, f64)) -> Aabb {
        let corners = [(0.0, 0.0), (size.0, 0.0), (0.0, size.1), size].map(|p| self.to_world(p, size));
        corners.iter().fold(
            [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
            |b, p| [b[0].min(p.0), b[1].min(p.1), b[2].max(p.0), b[3].max(p.1)],
        )
    }

    /// The matrix from coordinates about `anchor` (world units) to the
    /// view's pixels (`origin` its top left, `scale` device pixels a
    /// logical): computed in `f64`, given as `f32` holding only small
    /// numbers.
    #[must_use]
    pub fn matrix(&self, anchor: (f64, f64), size: (f64, f64), origin: (f64, f64), scale: f64) -> [f32; 6] {
        let s = self.to_screen(anchor, size);
        let k = self.zoom * scale;
        let (c, sn) = self.turn();
        let f = crate::convert::f64_to_f32;
        [
            f(k * c),
            f(k * sn),
            f(-k * sn),
            f(k * c),
            f((origin.0 + s.0) * scale),
            f((origin.1 + s.1) * scale),
        ]
    }

    /// Zoom by `factor` about screen point `at`.
    #[must_use]
    pub fn zoomed(&self, factor: f64, at: (f64, f64), size: (f64, f64)) -> Self {
        let w = self.to_world(at, size);
        let zoom = (self.zoom * factor).clamp(1e-12, 1e12);
        let next = Self { zoom, ..*self };
        next.keeping(w, at, size)
    }

    /// Turn by `angle` radians (clockwise) about screen point `at`.
    #[must_use]
    pub fn rotated(&self, angle: f64, at: (f64, f64), size: (f64, f64)) -> Self {
        let w = self.to_world(at, size);
        let next = Self {
            rotation: (self.rotation + angle).rem_euclid(std::f64::consts::TAU),
            ..*self
        };
        next.keeping(w, at, size)
    }

    /// Moved so world point `w` stands at screen point `at`.
    fn keeping(self, w: (f64, f64), at: (f64, f64), size: (f64, f64)) -> Self {
        let (c, sn) = self.turn();
        let d = turn((at.0 - size.0 / 2.0, at.1 - size.1 / 2.0), (c, -sn));
        Self {
            center: (w.0 - d.0 / self.zoom, w.1 - d.1 / self.zoom),
            ..self
        }
    }

    /// Panned by screen pixels `d` (the world follows the pointer).
    #[must_use]
    pub fn panned(&self, d: (f64, f64)) -> Self {
        let (c, sn) = self.turn();
        let w = turn(d, (c, -sn));
        Self {
            center: (self.center.0 - w.0 / self.zoom, self.center.1 - w.1 / self.zoom),
            ..*self
        }
    }
}

/// A path in world coordinates (`f64`), as given.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldPath {
    verbs: Vec<crate::figure::Verb>,
    points: Vec<[f64; 2]>,
}

impl WorldPath {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn move_to(mut self, x: f64, y: f64) -> Self {
        self.verbs.push(crate::figure::Verb::Move);
        self.points.push([x, y]);
        self
    }

    #[must_use]
    pub fn line_to(mut self, x: f64, y: f64) -> Self {
        self.verbs.push(crate::figure::Verb::Line);
        self.points.push([x, y]);
        self
    }

    #[must_use]
    pub fn quad_to(mut self, cx: f64, cy: f64, x: f64, y: f64) -> Self {
        self.verbs.push(crate::figure::Verb::Quad);
        self.points.extend([[cx, cy], [x, y]]);
        self
    }

    #[must_use]
    pub fn cubic_to(mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) -> Self {
        self.verbs.push(crate::figure::Verb::Cubic);
        self.points.extend([[c1x, c1y], [c2x, c2y], [x, y]]);
        self
    }

    #[must_use]
    pub fn close(mut self) -> Self {
        self.verbs.push(crate::figure::Verb::Close);
        self
    }

    /// A rectangle.
    #[must_use]
    pub fn rect(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self::new()
            .move_to(x, y)
            .line_to(x + w, y)
            .line_to(x + w, y + h)
            .line_to(x, y + h)
            .close()
    }

    /// It moved by `d`.
    #[must_use]
    pub fn translated(&self, d: (f64, f64)) -> Self {
        Self {
            verbs: self.verbs.clone(),
            points: self.points.iter().map(|p| [p[0] + d.0, p[1] + d.1]).collect(),
        }
    }

    /// Its bounds.
    #[must_use]
    pub fn bounds(&self) -> Aabb {
        self.points.iter().fold(
            [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
            |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])],
        )
    }

    /// The part of it that can show in `area`, flattened to `tolerance`
    /// (world units) and cut to `area` in `f64`, as an `f32` path about
    /// `anchor`: how an object far larger than the screen is drawn, so
    /// no `f32` holds a point far from what is seen. A fill's contours
    /// are clipped to the box whole (Sutherland–Hodgman keeps every
    /// winding number inside it); a stroke's lines are cut where they
    /// leave it (give `area` room for the stroke's width).
    #[must_use]
    pub fn near(&self, anchor: (f64, f64), area: Aabb, tolerance: f64, stroke: bool) -> Path {
        let mut b: PathBuilder = Path::builder();
        let f = |p: [f64; 2]| {
            (
                crate::convert::f64_to_f32(p[0] - anchor.0),
                crate::convert::f64_to_f32(p[1] - anchor.1),
            )
        };
        for (points, closed) in self.flattened(area, tolerance) {
            if stroke {
                // Runs of the polyline inside the box, each its own subpath.
                let mut open = false;
                let n = points.len();
                let edges = points.windows(2).map(|w| (w.first().copied(), w.get(1).copied()));
                let closing = (closed && n > 2).then(|| (points.last().copied(), points.first().copied()));
                for (a, z) in edges.chain(closing) {
                    let (Some(a), Some(z)) = (a, z) else { continue };
                    match clip_segment(a, z, &area) {
                        Some((p, q, cut_end)) => {
                            if !open || p != a {
                                let (x, y) = f(p);
                                b = b.move_to(x, y);
                            }
                            let (x, y) = f(q);
                            b = b.line_to(x, y);
                            open = !cut_end;
                        }
                        None => open = false,
                    }
                }
            } else {
                let clipped = clip_polygon(&points, &area);
                let mut first = true;
                for p in &clipped {
                    let (x, y) = f(*p);
                    b = if first { b.move_to(x, y) } else { b.line_to(x, y) };
                    first = false;
                }
                if !first {
                    b = b.close();
                }
            }
        }
        b.build()
    }

    /// Its contours as polylines (closed or not), curves flattened to
    /// `tolerance`; a piece of curve whose control points all lie outside
    /// `area` on one side is its chord (the curve lies in their hull).
    fn flattened(&self, area: Aabb, tolerance: f64) -> Vec<(Vec<[f64; 2]>, bool)> {
        use crate::figure::Verb;
        let mut out: Vec<(Vec<[f64; 2]>, bool)> = Vec::new();
        let mut cur: Vec<[f64; 2]> = Vec::new();
        let mut pts = self.points.iter().copied();
        let tol = tolerance.max(f64::MIN_POSITIVE);
        let last = |cur: &Vec<[f64; 2]>| cur.last().copied().unwrap_or([0.0, 0.0]);
        for v in &self.verbs {
            match v {
                Verb::Move => {
                    if cur.len() > 1 {
                        out.push((std::mem::take(&mut cur), false));
                    }
                    cur.clear();
                    if let Some(p) = pts.next() {
                        cur.push(p);
                    }
                }
                Verb::Line => {
                    if let Some(p) = pts.next() {
                        cur.push(p);
                    }
                }
                Verb::Quad => {
                    if let (Some(c), Some(p)) = (pts.next(), pts.next()) {
                        let a = last(&cur);
                        // As a cubic: one subdivision for both.
                        let c1 = [a[0] + (c[0] - a[0]) * 2.0 / 3.0, a[1] + (c[1] - a[1]) * 2.0 / 3.0];
                        let c2 = [p[0] + (c[0] - p[0]) * 2.0 / 3.0, p[1] + (c[1] - p[1]) * 2.0 / 3.0];
                        flatten_cubic([a, c1, c2, p], &area, tol, 0, &mut cur);
                    }
                }
                Verb::Cubic => {
                    if let (Some(c1), Some(c2), Some(p)) = (pts.next(), pts.next(), pts.next()) {
                        let a = last(&cur);
                        flatten_cubic([a, c1, c2, p], &area, tol, 0, &mut cur);
                    }
                }
                Verb::Close => {
                    if cur.len() > 1 {
                        let start = cur.first().copied();
                        out.push((std::mem::take(&mut cur), true));
                        cur.extend(start);
                    }
                }
            }
        }
        if cur.len() > 1 {
            out.push((cur, false));
        }
        out
    }

    /// Its runs of lines simplified (Douglas–Peucker) so no dropped point
    /// lies farther than `tolerance` from what is kept; curves, ends and
    /// every subpath's start stay as they are.
    #[must_use]
    pub fn simplified(&self, tolerance: f64) -> Self {
        use crate::figure::Verb;
        let mut out = Self::new();
        let mut pts = self.points.iter().copied();
        // The run of lines going: its points, the first already in `out`.
        let mut run: Vec<[f64; 2]> = Vec::new();
        let flush = |out: &mut Self, run: &mut Vec<[f64; 2]>| {
            if run.len() > 1 {
                for p in douglas_peucker(run, tolerance).into_iter().skip(1) {
                    out.verbs.push(Verb::Line);
                    out.points.push(p);
                }
            }
            let last = run.last().copied();
            run.clear();
            run.extend(last);
        };
        for v in &self.verbs {
            match v {
                Verb::Line => {
                    if let Some(p) = pts.next() {
                        run.push(p);
                    }
                }
                other => {
                    flush(&mut out, &mut run);
                    let n = match other {
                        Verb::Move => 1,
                        Verb::Quad => 2,
                        Verb::Cubic => 3,
                        Verb::Line | Verb::Close => 0,
                    };
                    out.verbs.push(*other);
                    for _ in 0..n {
                        if let Some(p) = pts.next() {
                            out.points.push(p);
                        }
                    }
                    run.clear();
                    if *other == Verb::Close {
                        // The subpath starts again where it began: a line
                        // after a close starts from its move.
                        if let Some(start) = out.subpath_start() {
                            run.push(start);
                        }
                    } else {
                        run.extend(out.points.last().copied());
                    }
                }
            }
        }
        flush(&mut out, &mut run);
        out
    }

    /// Where the last subpath began.
    fn subpath_start(&self) -> Option<[f64; 2]> {
        use crate::figure::Verb;
        let mut at = 0_usize;
        let mut start = None;
        for v in &self.verbs {
            match v {
                Verb::Move => {
                    start = self.points.get(at).copied();
                    at = at.saturating_add(1);
                }
                Verb::Line => at = at.saturating_add(1),
                Verb::Quad => at = at.saturating_add(2),
                Verb::Cubic => at = at.saturating_add(3),
                Verb::Close => {}
            }
        }
        start
    }

    /// It as an `f32` path about `anchor`, scaled by `k` (1 keeps world units).
    #[must_use]
    pub fn about(&self, anchor: (f64, f64), k: f64) -> Path {
        let f = |p: &[f64; 2]| {
            (
                crate::convert::f64_to_f32((p[0] - anchor.0) * k),
                crate::convert::f64_to_f32((p[1] - anchor.1) * k),
            )
        };
        let mut b: PathBuilder = Path::builder();
        let mut pts = self.points.iter();
        for v in &self.verbs {
            use crate::figure::Verb;
            b = match v {
                Verb::Move => match pts.next() {
                    Some(p) => {
                        let (x, y) = f(p);
                        b.move_to(x, y)
                    }
                    None => b,
                },
                Verb::Line => match pts.next() {
                    Some(p) => {
                        let (x, y) = f(p);
                        b.line_to(x, y)
                    }
                    None => b,
                },
                Verb::Quad => match (pts.next(), pts.next()) {
                    (Some(c), Some(p)) => {
                        let ((cx, cy), (x, y)) = (f(c), f(p));
                        b.quad_to(cx, cy, x, y)
                    }
                    _ => b,
                },
                Verb::Cubic => match (pts.next(), pts.next(), pts.next()) {
                    (Some(c1), Some(c2), Some(p)) => {
                        let ((ax, ay), (bx, by), (x, y)) = (f(c1), f(c2), f(p));
                        b.cubic_to(ax, ay, bx, by, x, y)
                    }
                    _ => b,
                },
                Verb::Close => b.close(),
            };
        }
        b.build()
    }
}

/// The points of polyline `run` Douglas–Peucker keeps within `tol`: its
/// ends, and each farthest point past `tol` from the chord between kept
/// ones (iterative, so a long run cannot overflow the stack).
fn douglas_peucker(run: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    let n = run.len();
    if n < 3 {
        return run.to_vec();
    }
    let mut keep = vec![false; n];
    if let Some(k) = keep.first_mut() {
        *k = true;
    }
    if let Some(k) = keep.last_mut() {
        *k = true;
    }
    let mut stack = vec![(0_usize, n.saturating_sub(1))];
    while let Some((a, z)) = stack.pop() {
        let (Some(pa), Some(pz)) = (run.get(a).copied(), run.get(z).copied()) else {
            continue;
        };
        let (dx, dy) = (pz[0] - pa[0], pz[1] - pa[1]);
        let len = dx.hypot(dy);
        let mut far = (0.0_f64, a);
        for (i, p) in run.iter().enumerate().take(z).skip(a.saturating_add(1)) {
            let d = if len > 0.0 {
                ((p[0] - pa[0]) * dy - (p[1] - pa[1]) * dx).abs() / len
            } else {
                (p[0] - pa[0]).hypot(p[1] - pa[1])
            };
            if d > far.0 {
                far = (d, i);
            }
        }
        if far.0 > tol {
            if let Some(k) = keep.get_mut(far.1) {
                *k = true;
            }
            stack.push((a, far.1));
            stack.push((far.1, z));
        }
    }
    run.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Push cubic `c` (its start already pushed) as lines within `tol`.
fn flatten_cubic(c: [[f64; 2]; 4], area: &Aabb, tol: f64, depth: u32, out: &mut Vec<[f64; 2]>) {
    let [a, b, d, e] = c;
    let outside = |i: usize, below: bool| {
        c.iter().all(|p| {
            let v = p.get(i).copied().unwrap_or(0.0);
            let edge = if below {
                area.get(i)
            } else {
                area.get(i.saturating_add(2))
            }
            .copied()
            .unwrap_or(0.0);
            if below { v < edge } else { v > edge }
        })
    };
    let away = outside(0, true) || outside(0, false) || outside(1, true) || outside(1, false);
    // How far the control points stray from the chord bounds the curve's.
    let dist = |p: [f64; 2]| {
        let (dx, dy) = (e[0] - a[0], e[1] - a[1]);
        let len = dx.hypot(dy);
        if len > 0.0 {
            ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len
        } else {
            (p[0] - a[0]).hypot(p[1] - a[1])
        }
    };
    if away || depth >= 24 || dist(b).max(dist(d)) * 0.75 <= tol {
        out.push(e);
        return;
    }
    let mid = |p: [f64; 2], q: [f64; 2]| [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
    let (ab, bd, de) = (mid(a, b), mid(b, d), mid(d, e));
    let (abd, bde) = (mid(ab, bd), mid(bd, de));
    let m = mid(abd, bde);
    flatten_cubic([a, ab, abd, m], area, tol, depth.saturating_add(1), out);
    flatten_cubic([m, bde, de, e], area, tol, depth.saturating_add(1), out);
}

/// Segment `a`–`z` inside `area` (Liang–Barsky): its ends, and whether it
/// was cut before `z`.
fn clip_segment(a: [f64; 2], z: [f64; 2], area: &Aabb) -> Option<([f64; 2], [f64; 2], bool)> {
    let (dx, dy) = (z[0] - a[0], z[1] - a[1]);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [
        (-dx, a[0] - area[0]),
        (dx, area[2] - a[0]),
        (-dy, a[1] - area[1]),
        (dy, area[3] - a[1]),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    if t0 > t1 {
        return None;
    }
    let at = |t: f64| {
        if t <= 0.0 {
            a
        } else if t >= 1.0 {
            z
        } else {
            [a[0] + dx * t, a[1] + dy * t]
        }
    };
    Some((at(t0), at(t1), t1 < 1.0))
}

/// Closed polygon `points` cut to `area` (Sutherland–Hodgman).
fn clip_polygon(points: &[[f64; 2]], area: &Aabb) -> Vec<[f64; 2]> {
    let mut poly = points.to_vec();
    // Each edge: which coordinate, the bound, whether inside is above it.
    for (i, bound, above) in [
        (0, area[0], true),
        (0, area[2], false),
        (1, area[1], true),
        (1, area[3], false),
    ] {
        let inside = |p: &[f64; 2]| {
            let v = p.get(i).copied().unwrap_or(0.0);
            if above { v >= bound } else { v <= bound }
        };
        let cross = |p: [f64; 2], q: [f64; 2]| {
            let (pv, qv) = (p.get(i).copied().unwrap_or(0.0), q.get(i).copied().unwrap_or(0.0));
            let t = if qv == pv { 0.0 } else { (bound - pv) / (qv - pv) };
            let mut r = [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
            if let Some(c) = r.get_mut(i) {
                *c = bound;
            }
            r
        };
        let mut out = Vec::with_capacity(poly.len().saturating_add(4));
        let mut prev = match poly.last() {
            Some(p) => *p,
            None => return Vec::new(),
        };
        for p in &poly {
            match (inside(&prev), inside(p)) {
                (true, true) => out.push(*p),
                (true, false) => out.push(cross(prev, *p)),
                (false, true) => {
                    out.push(cross(prev, *p));
                    out.push(*p);
                }
                (false, false) => {}
            }
            prev = *p;
        }
        poly = out;
    }
    poly
}

/// An object as the world holds it: what it is, its anchor, its box, the
/// colour it shows as when it is a box ([`LOD_BOX`]), its place in the order.
#[derive(Debug)]
pub struct Placed {
    pub body: Body,
    /// Its box's corner: its `f32` coordinates are about it.
    pub anchor: (f64, f64),
    pub bbox: Aabb,
    pub color: Color,
    pub z: ZKey,
}

/// What an object is.
#[derive(Debug)]
pub enum Body {
    Path(PathBody),
    Text(TextBody),
}

/// A path: its `f64` source, its path about the anchor, how it is painted,
/// and its simplified forms by level of detail.
#[derive(Debug)]
pub struct PathBody {
    pub source: WorldPath,
    pub path: Path,
    pub style: PathStyle,
    /// The path simplified for a level ([`PathBody::at_level`]), the last few used.
    levels: std::sync::Mutex<Vec<(i32, Option<Path>)>>,
}

/// Points a path's lines must number before it is simplified per level.
pub const SIMPLIFY_FROM: usize = 64;
/// How far a simplified line may stray, in device pixels at the least
/// scale of its level (at most twice this at the greatest).
pub const SIMPLIFY_PX: f64 = 0.125;

impl PathBody {
    fn new(source: WorldPath, anchor: (f64, f64), style: PathStyle) -> Self {
        Self {
            path: source.about(anchor, 1.0),
            source,
            style,
            levels: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// The same path moved by `d`: its `f32` path and its simplified forms
    /// are about its anchor, which moves with it, so they are kept.
    fn moved(&self, d: (f64, f64)) -> Self {
        let levels = self
            .levels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        Self {
            source: self.source.translated(d),
            path: self.path.clone(),
            style: self.style.clone(),
            levels: std::sync::Mutex::new(levels),
        }
    }

    /// The path to draw at `k` device pixels a world unit: below the
    /// level where nothing would be lost, its runs of lines simplified
    /// (Douglas–Peucker) within [`SIMPLIFY_PX`] of a device pixel at the
    /// level `floor(log2 k)`, made once a level and kept for the last four.
    #[must_use]
    pub fn at_level(&self, k: f64, anchor: (f64, f64)) -> Path {
        if self.source.points.len() < SIMPLIFY_FROM || !(k.is_finite() && k > 0.0) {
            return self.path.clone();
        }
        let level = level_of(k);
        let mut levels = self.levels.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(i) = levels.iter().position(|(l, _)| *l == level) {
            let hit = levels.remove(i);
            let out = hit.1.clone().unwrap_or_else(|| self.path.clone());
            levels.push(hit);
            return out;
        }
        let tolerance = SIMPLIFY_PX / f64::from(level).exp2();
        let simple = self.source.simplified(tolerance);
        // Kept only when it drops a quarter of the points or more.
        let made = (simple.points.len().saturating_mul(4) <= self.source.points.len().saturating_mul(3))
            .then(|| simple.about(anchor, 1.0));
        if levels.len() >= 4 {
            levels.remove(0);
        }
        levels.push((level, made.clone()));
        made.unwrap_or_else(|| self.path.clone())
    }
}

/// The level of detail of `k` device pixels a unit: `floor(log2 k)`.
#[must_use]
pub fn level_of(k: f64) -> i32 {
    crate::convert::f64_to_i32(k.log2().floor())
}

/// Text: its content, how it is set, its size, the width it wraps to, and
/// its glyphs once shaped.
#[derive(Debug)]
pub struct TextBody {
    pub content: String,
    pub style: crate::style::ResolvedText,
    /// World units per em.
    pub size: f64,
    /// World units its lines wrap to (`None`: one line).
    pub width: Option<f64>,
    /// Its glyphs, shaped once by the first paint that has the text system
    /// ([`crate::chunk::Picture::shape`]).
    pub shaped: std::sync::OnceLock<Shaped>,
}

impl TextBody {
    fn moved(&self) -> Self {
        let shaped = std::sync::OnceLock::new();
        if let Some(s) = self.shaped.get() {
            shaped.set(s.clone()).ok();
        }
        Self {
            content: self.content.clone(),
            style: self.style,
            size: self.size,
            width: self.width,
            shaped,
        }
    }
}

/// Text shaped: each glyph's key and pen (ems from the text's top left,
/// y down), its colour; its widest line and its height (ems); each line's
/// box (ems: x0, y0, x1, y1).
#[derive(Clone, Debug, Default)]
pub struct Shaped {
    pub glyphs: Vec<(crate::text::GlyphKey, (f64, f64), Color)>,
    pub width: f64,
    pub height: f64,
    pub lines: Vec<[f64; 4]>,
}

/// Text in a world: `content` with its top left at `at`, `size` world
/// units per em, set as `style` sets it (its size there is ignored), on
/// one line or wrapped to `width` world units.
#[derive(Clone, Debug)]
pub struct WorldText {
    pub content: String,
    pub at: (f64, f64),
    pub size: f64,
    pub width: Option<f64>,
    pub style: crate::style::ResolvedText,
}

impl WorldText {
    #[must_use]
    pub fn new(content: impl Into<String>, at: (f64, f64), size: f64) -> Self {
        Self {
            content: content.into(),
            at,
            size,
            width: None,
            style: crate::style::ResolvedText::root(Color::BLACK),
        }
    }

    /// Wrap its lines to `width` world units, as a paragraph of UI text
    /// wraps to its box (Parley's line breaking, the style's alignment).
    #[must_use]
    pub fn width(mut self, width: f64) -> Self {
        self.width = Some(width);
        self
    }

    #[must_use]
    pub fn color(mut self, color: Color) -> Self {
        self.style.color = color;
        self
    }

    #[must_use]
    pub fn weight(mut self, weight: u16) -> Self {
        self.style.weight = weight;
        self
    }

    #[must_use]
    pub fn align(mut self, align: crate::style::TextAlign) -> Self {
        self.style.align = align;
        self
    }
}

/// The em size text is shaped at, in device pixels: positions quantised to
/// a quarter of a pixel there are within 1/4096 em.
pub(crate) const SHAPE_PPEM: f32 = 1024.0;

impl Placed {
    /// It as a path, when it is one.
    #[must_use]
    pub fn as_path(&self) -> Option<&PathBody> {
        match &self.body {
            Body::Path(p) => Some(p),
            Body::Text(_) => None,
        }
    }

    /// Everything it can ink: its box, and for a stroke the reach of its
    /// miters past it.
    #[must_use]
    pub fn ink(&self) -> Aabb {
        let extra = match &self.body {
            Body::Path(p) => match &p.style.draw {
                crate::figure::Draw::Stroke(s) => (plan::stroke_reach(&p.style) - f64::from(s.width) / 2.0).max(0.0),
                crate::figure::Draw::Fill => 0.0,
            },
            Body::Text(_) => 0.0,
        };
        let b = self.bbox;
        [b[0] - extra, b[1] - extra, b[2] + extra, b[3] + extra]
    }

    /// The same object moved by `d` world units.
    fn moved(&self, d: (f64, f64)) -> Self {
        let b = self.bbox;
        Self {
            body: match &self.body {
                Body::Path(p) => Body::Path(p.moved(d)),
                Body::Text(t) => Body::Text(t.moved()),
            },
            anchor: (self.anchor.0 + d.0, self.anchor.1 + d.1),
            bbox: [b[0] + d.0, b[1] + d.1, b[2] + d.0, b[3] + d.1],
            color: self.color,
            z: self.z.clone(),
        }
    }
}

/// An object's id in its world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(pub u32);

/// Objects a page of a base holds.
const PAGE: usize = 1024;
/// Objects added or moved since the base's tree was copied past which it
/// is copied again.
const FOLD: usize = 512;

/// A page of objects, shared between snapshots until one of it changes.
type Page = Arc<Vec<Option<Arc<Placed>>>>;

/// What tiles draw of a world. Made again for each version, but never in
/// `O(objects)` for one edit: the objects are pages shared until one of
/// theirs changes (copy on write), the order is kept across moves, and the
/// tree is a copy shared until [`FOLD`] objects were added or moved since,
/// those found by a scan meanwhile. Lifting an object, or moving a lifted
/// one, makes a new snapshot over the same base.
#[derive(Debug)]
pub struct Base {
    pages: Vec<Page>,
    /// Each object's place in the draw order (rising with it, not dense).
    rank: Arc<Vec<u32>>,
    tree: Arc<AabbTree>,
    /// Objects added or moved since `tree` was copied (sorted): its boxes
    /// for them are stale or missing.
    fresh: Arc<[u32]>,
    /// Everything's box (a bound: it grows with the world, and is made
    /// tight again when the tree is copied).
    pub extent: Aabb,
}

impl Base {
    /// Object `i`.
    #[must_use]
    pub fn object(&self, i: u32) -> Option<&Arc<Placed>> {
        let i = usize::try_from(i).ok()?;
        self.pages.get(i / PAGE)?.get(i % PAGE)?.as_ref()
    }

    fn rank(&self, i: u32) -> u32 {
        self.rank
            .get(usize::try_from(i).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or(0)
    }

    fn is_fresh(&self, i: u32) -> bool {
        !self.fresh.is_empty() && self.fresh.binary_search(&i).is_ok()
    }

    /// Each object whose box may meet `area`.
    fn query(&self, area: &Aabb, mut f: impl FnMut(u32)) {
        self.tree.query(area, |i| {
            if !self.is_fresh(i) {
                f(i);
            }
        });
        for i in self.fresh.iter() {
            if self.object(*i).is_some_and(|o| aabb::overlaps(&o.bbox, area)) {
                f(*i);
            }
        }
    }

    /// About how many objects meet `area`, at most `limit`.
    #[must_use]
    pub fn count(&self, area: &Aabb, limit: usize) -> usize {
        self.tree.count(area, limit).saturating_add(self.fresh.len()).min(limit)
    }
}

/// Changes a world keeps the boxes of, for tiles to know whether a change
/// reached them; a tile older than the oldest kept is stale everywhere.
pub const DAMAGE_KEPT: usize = 1024;

/// Where versions changed what tiles show: each version and the world box.
pub type DamageList = Arc<[(u64, Aabb)]>;

/// A world as one frame drew it.
#[derive(Debug)]
pub struct WorldSnapshot {
    pub id: u64,
    /// What tiles show: moves with every change but one to a lifted object.
    pub version: u64,
    /// Moves with every change to a lifted object.
    pub live_version: u64,
    pub base: Arc<Base>,
    /// The lifted objects as they are now, by id: left out of what tiles
    /// draw and drawn live, over the tiles, each frame.
    pub live: Vec<(u32, Arc<Placed>)>,
    /// Where each version changed what tiles show (oldest first): the
    /// version made and the world box changed.
    pub damage: DamageList,
    /// Versions before this are older than the kept damage.
    pub damage_from: u64,
    /// Text not yet shaped when it was made.
    pub unshaped: Vec<u32>,
}

impl WorldSnapshot {
    /// The objects tiles draw meeting `area` (world), in draw order: the
    /// lifted ones left out.
    #[must_use]
    pub fn visible(&self, area: &Aabb) -> Vec<u32> {
        let mut out = Vec::new();
        self.base.query(area, |i| {
            let meets = self.base.object(i).is_some_and(|o| aabb::overlaps(&o.bbox, area));
            if meets && !self.lifted(i) {
                out.push(i);
            }
        });
        out.sort_unstable_by_key(|i| self.base.rank(*i));
        out
    }

    /// Everything meeting `area` as it is now, in draw order: what tiles
    /// draw and the lifted objects where they are, each in its place.
    #[must_use]
    pub fn drawn(&self, area: &Aabb) -> Vec<(u32, &Arc<Placed>)> {
        let base = self.visible(area);
        let mut live: Vec<(u32, &Arc<Placed>)> = self
            .live
            .iter()
            .filter(|(_, o)| aabb::overlaps(&o.bbox, area))
            .map(|(i, o)| (*i, o))
            .collect();
        live.sort_by(|a, b| a.1.z.cmp(&b.1.z));
        let mut out = Vec::with_capacity(base.len().saturating_add(live.len()));
        let mut live = live.into_iter().peekable();
        for i in base {
            let Some(o) = self.object(i) else { continue };
            while let Some(l) = live.next_if(|l| l.1.z < o.z) {
                out.push(l);
            }
            out.push((i, o));
        }
        out.extend(live);
        out
    }

    /// Object `i` as tiles have it (a lifted object where it was lifted).
    #[must_use]
    pub fn object(&self, i: u32) -> Option<&Arc<Placed>> {
        self.base.object(i)
    }

    /// Whether object `i` is lifted.
    #[must_use]
    pub fn lifted(&self, i: u32) -> bool {
        !self.live.is_empty() && self.live.binary_search_by_key(&i, |(j, _)| *j).is_ok()
    }

    /// Lifted object `i` as it is now.
    #[must_use]
    pub fn live_object(&self, i: u32) -> Option<&Arc<Placed>> {
        let at = self.live.binary_search_by_key(&i, |(j, _)| *j).ok()?;
        self.live.get(at).map(|(_, o)| o)
    }

    /// Everything's box.
    #[must_use]
    pub fn extent(&self) -> Aabb {
        self.base.extent
    }

    /// Whether nothing tiles show changed in world box `area` from version
    /// `since` to this one.
    #[must_use]
    pub fn clean(&self, area: &Aabb, since: u64) -> bool {
        if since >= self.version {
            return true;
        }
        if since < self.damage_from {
            return false;
        }
        self.damage
            .iter()
            .rev()
            .take_while(|(v, _)| *v > since)
            .all(|(_, b)| !aabb::overlaps(b, area))
    }

    /// The boxes changed after version `since` (all of them, as far as
    /// kept, when it is older than the kept damage).
    pub fn damaged_since(&self, since: u64) -> impl Iterator<Item = &Aabb> {
        self.damage
            .iter()
            .rev()
            .take_while(move |(v, _)| *v > since)
            .map(|(_, b)| b)
    }

    /// The topmost object under world point `at` (within `slop` world
    /// units of its box) that `pick` takes: lifted ones where they are now.
    #[must_use]
    pub fn hit_where(&self, at: (f64, f64), slop: f64, pick: impl Fn(ObjectId) -> bool) -> Option<ObjectId> {
        let area = [at.0 - slop, at.1 - slop, at.0 + slop, at.1 + slop];
        self.drawn(&area)
            .iter()
            .rev()
            .map(|(i, _)| ObjectId(*i))
            .find(|id| pick(*id))
    }
}

/// What a world holds (the UI thread's side).
#[derive(Debug)]
pub struct WorldState {
    id: u64,
    version: u64,
    live_version: u64,
    objects: Vec<Option<(Arc<Placed>, u32)>>,
    free: Vec<u32>,
    tree: AabbTree,
    order: std::collections::BTreeMap<ZKey, u32>,
    /// What bases share: the objects in pages, their ranks (and the next
    /// rank on top), the tree as last copied and what changed since, the
    /// extent.
    pages: Vec<Page>,
    rank: Arc<Vec<u32>>,
    next_rank: u32,
    reranked: bool,
    tree_copy: Arc<AabbTree>,
    fresh: std::collections::BTreeSet<u32>,
    extent: Aabb,
    /// Lifted objects as they are now (their slots in `objects` hold them
    /// as they were lifted).
    lifted: std::collections::BTreeMap<u32, Arc<Placed>>,
    damage: std::collections::VecDeque<(u64, Aabb)>,
    damage_from: u64,
    /// Text not shaped when the last snapshot was made, for the next paint
    /// to shape (pruned as snapshots are made).
    unshaped: std::cell::RefCell<Vec<u32>>,
    /// The base of this version, and the snapshot of this version and live
    /// version, made when first read: caches, not state a reader is told of.
    base: std::cell::RefCell<Option<Arc<Base>>>,
    damage_list: std::cell::RefCell<Option<DamageList>>,
    snapshot: std::cell::RefCell<Option<Arc<WorldSnapshot>>>,
}

/// A canvas' world: a `Copy` handle owned by the scope it was made in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct World(Signal<WorldState>);

/// An empty world.
#[must_use]
pub fn world() -> World {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    World(Signal::new(WorldState {
        id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        version: 0,
        live_version: 0,
        objects: Vec::new(),
        free: Vec::new(),
        tree: AabbTree::new(0.05),
        order: std::collections::BTreeMap::new(),
        pages: Vec::new(),
        rank: Arc::new(Vec::new()),
        next_rank: 0,
        reranked: false,
        tree_copy: Arc::new(AabbTree::new(0.05)),
        fresh: std::collections::BTreeSet::new(),
        extent: NOTHING,
        lifted: std::collections::BTreeMap::new(),
        damage: std::collections::VecDeque::new(),
        damage_from: 0,
        unshaped: std::cell::RefCell::new(Vec::new()),
        base: std::cell::RefCell::new(None),
        damage_list: std::cell::RefCell::new(None),
        snapshot: std::cell::RefCell::new(None),
    }))
}

/// The paint's colour, for an object drawn as a box.
fn color_of(style: &PathStyle) -> Color {
    match &style.paint {
        Gradient::Solid(c) => *c,
        _ => Color::BLACK.opacity(0.5),
    }
}

fn place(source: WorldPath, style: PathStyle, z: ZKey) -> Placed {
    let bbox = source.bounds();
    // Its anchor: its box's corner, so the path about it holds only the
    // object's own extent (`f32` keeps it to 2⁻²⁴ of that).
    let anchor = if bbox[0].is_finite() {
        (bbox[0], bbox[1])
    } else {
        (0.0, 0.0)
    };
    // Stroked paths reach half their width past their points.
    let reach = match &style.draw {
        crate::figure::Draw::Stroke(s) => f64::from(s.width) / 2.0,
        crate::figure::Draw::Fill => 0.0,
    };
    let bbox = [bbox[0] - reach, bbox[1] - reach, bbox[2] + reach, bbox[3] + reach];
    Placed {
        color: color_of(&style),
        body: Body::Path(PathBody::new(source, anchor, style)),
        anchor,
        bbox,
        z,
    }
}

/// Text placed: its box until shaped is a generous guess (an em a
/// character on one line; wrapped, as many lines of its width as an em a
/// character fills, and one more; ascent and descent), as culling and
/// damage only need it to hold the ink.
fn place_text(t: WorldText, z: ZKey) -> Placed {
    let chars = crate::convert::usize_to_f64(t.content.chars().count().max(1));
    let line = 1.5 * t.size;
    let (wide, tall) = match t.width {
        Some(w) if w > 0.0 => {
            // A word longer than the width overflows it, as Parley lets it.
            let longest = t
                .content
                .split_whitespace()
                .map(|w| w.chars().count())
                .max()
                .unwrap_or(1);
            let lines = (chars * t.size / w).ceil() + 1.0;
            (w.max(crate::convert::usize_to_f64(longest) * t.size), lines * line)
        }
        _ => (chars * t.size, line),
    };
    Placed {
        color: t.style.color.opacity(0.5),
        anchor: t.at,
        bbox: [t.at.0, t.at.1 - 0.25 * t.size, t.at.0 + wide, t.at.1 + tall],
        body: Body::Text(TextBody {
            content: t.content,
            style: t.style,
            size: t.size,
            width: t.width,
            shaped: std::sync::OnceLock::new(),
        }),
        z,
    }
}

/// The same object again with order key `z`.
fn replaced(old: &Placed, z: ZKey) -> Placed {
    Placed {
        z,
        ..old.moved((0.0, 0.0))
    }
}

/// The union of two boxes (an empty box, infinite, adds nothing).
fn union(a: Aabb, b: Aabb) -> Aabb {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

const NOTHING: Aabb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];

impl World {
    /// Add an object on top; its id.
    pub fn add(&self, path: WorldPath, style: PathStyle) -> ObjectId {
        let mut id = ObjectId(0);
        self.0.update(|w| {
            let z = ZKey::after(w.order.keys().next_back());
            let p = place(path, style, z);
            let ink = p.ink();
            id = w.insert(p);
            w.restructured();
            w.changed(ink);
        });
        id
    }

    /// Add text on top; its id. It is shaped by the next paint.
    pub fn add_text(&self, text: WorldText) -> ObjectId {
        let mut id = ObjectId(0);
        self.0.update(|w| {
            let z = ZKey::after(w.order.keys().next_back());
            let p = place_text(text, z);
            let ink = p.ink();
            id = w.insert(p);
            w.unshaped.get_mut().push(id.0);
            w.restructured();
            w.changed(ink);
        });
        id
    }

    /// Add many objects on top, in order; their ids.
    pub fn extend(&self, objects: impl IntoIterator<Item = (WorldPath, PathStyle)>) -> Vec<ObjectId> {
        let mut ids = Vec::new();
        self.0.update(|w| {
            let mut ink = NOTHING;
            for (path, style) in objects {
                let z = ZKey::after(w.order.keys().next_back());
                let p = place(path, style, z);
                ink = union(ink, p.ink());
                ids.push(w.insert(p));
            }
            w.restructured();
            w.changed(ink);
        });
        ids
    }

    /// Move object `id` to `path` (a path's; text keeps its own).
    pub fn set_path(&self, id: ObjectId, path: WorldPath) {
        self.0.update(|w| {
            let Some(now) = w.current(id.0) else { return };
            let Some(body) = now.as_path() else { return };
            let p = place(path, body.style.clone(), now.z.clone());
            w.replace(id.0, &now, p);
        });
    }

    /// Move object `id` by `d` world units: a path's source, a text's
    /// place; what was drawn of it (its `f32` path, simplified forms and
    /// shaped glyphs, all about its anchor) is kept.
    pub fn translate(&self, id: ObjectId, d: (f64, f64)) {
        self.0.update(|w| {
            let Some(now) = w.current(id.0) else { return };
            let p = now.moved(d);
            w.replace(id.0, &now, p);
        });
    }

    /// Lift object `id` out of the tiles into the live layer, while it is
    /// dragged or edited (Figma's and tldraw's interaction layer): it is
    /// drawn each frame where it is, in its place in the order, and the
    /// tiles are rastered again only where it was; moving it from then on
    /// rasters nothing. [`Self::land`] puts it back.
    pub fn lift(&self, id: ObjectId) {
        self.0.update(|w| {
            let Some(Some((p, _))) = w.objects.get(usize::try_from(id.0).unwrap_or(usize::MAX)) else {
                return;
            };
            if w.lifted.contains_key(&id.0) {
                return;
            }
            let p = Arc::clone(p);
            let ink = p.ink();
            w.lifted.insert(id.0, p);
            w.live_changed();
            w.changed(ink);
        });
    }

    /// Put lifted object `id` back into the tiles where it is now: the
    /// tiles are rastered again only where it lands.
    pub fn land(&self, id: ObjectId) {
        self.0.update(|w| w.land(id.0));
    }

    /// Put every lifted object back.
    pub fn land_all(&self) {
        self.0.update(|w| {
            let ids: Vec<u32> = w.lifted.keys().copied().collect();
            for i in ids {
                w.land(i);
            }
        });
    }

    /// The lifted objects.
    #[must_use]
    pub fn lifted(&self) -> Vec<ObjectId> {
        self.0
            .with_untracked(|w| w.lifted.keys().map(|i| ObjectId(*i)).collect())
    }

    /// Put object `id` just above `below` (`None`: at the bottom).
    pub fn raise_above(&self, id: ObjectId, below: Option<ObjectId>) {
        self.0.update(|w| {
            let Some(Some((old, proxy))) = w.objects.get(usize::try_from(id.0).unwrap_or(usize::MAX)).cloned() else {
                return;
            };
            w.order.remove(&old.z);
            let lo = below.and_then(|b| {
                w.objects
                    .get(usize::try_from(b.0).ok()?)?
                    .as_ref()
                    .map(|(p, _)| p.z.clone())
            });
            let hi = match &lo {
                Some(l) => w
                    .order
                    .range((std::ops::Bound::Excluded(l), std::ops::Bound::Unbounded))
                    .next()
                    .map(|(k, _)| k.clone()),
                None => w.order.keys().next().cloned(),
            };
            let z = ZKey::between(lo.as_ref(), hi.as_ref());
            let p = replaced(&old, z.clone());
            w.order.insert(z.clone(), id.0);
            if let Some(live) = w.lifted.get(&id.0) {
                let moved = Arc::new(replaced(live, z));
                w.lifted.insert(id.0, moved);
                w.live_changed();
            }
            let ink = p.ink();
            w.set_slot(id.0, Some((Arc::new(p), proxy)));
            w.reranked = true;
            w.restructured();
            w.changed(ink);
        });
    }

    /// Take object `id` out.
    pub fn remove(&self, id: ObjectId) {
        self.0.update(|w| {
            if w.lifted.remove(&id.0).is_some() {
                w.live_changed();
            }
            let Some(Some((p, proxy))) = w.objects.get(usize::try_from(id.0).unwrap_or(usize::MAX)).cloned() else {
                return;
            };
            w.set_slot(id.0, None);
            {
                w.tree.remove(proxy);
                w.order.remove(&p.z);
                w.free.push(id.0);
                w.restructured();
                w.changed(p.ink());
            }
        });
    }

    /// The topmost object under world point `at`, within `slop` world units
    /// of its box (exact `f64` boxes; a path's own shape is not tested).
    #[must_use]
    pub fn hit(&self, at: (f64, f64), slop: f64) -> Option<ObjectId> {
        self.hit_where(at, slop, |_| true)
    }

    /// The topmost object under `at` that `pick` takes (a design tool's
    /// unlocked objects): lifted ones where they are now.
    #[must_use]
    pub fn hit_where(&self, at: (f64, f64), slop: f64, pick: impl Fn(ObjectId) -> bool) -> Option<ObjectId> {
        self.snapshot_untracked().hit_where(at, slop, pick)
    }

    /// Objects it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.with(|w| w.order.len())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// What it holds now, tracked.
    #[must_use]
    pub fn snapshot(&self) -> Arc<WorldSnapshot> {
        self.0.with(|_| ());
        self.snapshot_untracked()
    }

    fn snapshot_untracked(&self) -> Arc<WorldSnapshot> {
        self.0.with_untracked(|w| {
            let mut cache = w.snapshot.borrow_mut();
            Arc::clone(cache.get_or_insert_with(|| w.build()))
        })
    }
}

impl WorldState {
    fn insert(&mut self, p: Placed) -> ObjectId {
        let i = self
            .free
            .pop()
            .unwrap_or_else(|| u32::try_from(self.objects.len()).unwrap_or(u32::MAX));
        let proxy = self.tree.insert(p.bbox, i);
        let top = self.order.keys().next_back().is_none_or(|last| p.z > *last);
        self.order.insert(p.z.clone(), i);
        if top {
            // On top, as adding puts it: its rank follows every other.
            let rank = Arc::make_mut(&mut self.rank);
            let at = usize::try_from(i).unwrap_or(usize::MAX);
            if at >= rank.len() {
                rank.resize(at.saturating_add(1), 0);
            }
            if let Some(r) = rank.get_mut(at) {
                *r = self.next_rank;
            }
            self.next_rank = self.next_rank.saturating_add(1);
        } else {
            self.reranked = true;
        }
        self.set_slot(i, Some((Arc::new(p), proxy)));
        ObjectId(i)
    }

    /// Slot `i` holds `v` now: in the objects, in its page (copied first
    /// if a snapshot shares it), noted fresh for the tree copy.
    fn set_slot(&mut self, i: u32, v: Option<(Arc<Placed>, u32)>) {
        let at = usize::try_from(i).unwrap_or(usize::MAX);
        if at >= self.objects.len() {
            self.objects.resize_with(at.saturating_add(1), || None);
        }
        let shown = v.as_ref().map(|(p, _)| Arc::clone(p));
        if let Some(p) = &shown {
            let b = p.bbox;
            self.extent = union(self.extent, b);
            self.fresh.insert(i);
        }
        if let Some(slot) = self.objects.get_mut(at) {
            *slot = v;
        }
        let (page, within) = (at / PAGE, at % PAGE);
        while self.pages.len() <= page {
            self.pages.push(Arc::new(vec![None; PAGE]));
        }
        if let Some(slot) = self
            .pages
            .get_mut(page)
            .and_then(|pg| Arc::make_mut(pg).get_mut(within))
        {
            *slot = shown;
        }
    }

    /// Object `i` as it is now: lifted, or in the tiles.
    fn current(&self, i: u32) -> Option<Arc<Placed>> {
        if let Some(p) = self.lifted.get(&i) {
            return Some(Arc::clone(p));
        }
        self.objects
            .get(usize::try_from(i).ok()?)?
            .as_ref()
            .map(|(p, _)| Arc::clone(p))
    }

    /// Object `i`, now `was`, becomes `p`: in the live layer alone when it
    /// is lifted (no tile changes), else in the tiles where it was and is.
    fn replace(&mut self, i: u32, was: &Placed, p: Placed) {
        if let Some(live) = self.lifted.get_mut(&i) {
            *live = Arc::new(p);
            self.live_changed();
            return;
        }
        let ink = union(was.ink(), p.ink());
        let Some(Some((_, proxy))) = self.objects.get(usize::try_from(i).unwrap_or(usize::MAX)) else {
            return;
        };
        let proxy = *proxy;
        self.tree.update(proxy, p.bbox);
        self.set_slot(i, Some((Arc::new(p), proxy)));
        self.restructured();
        self.changed(ink);
    }

    fn land(&mut self, i: u32) {
        let Some(now) = self.lifted.remove(&i) else { return };
        self.live_changed();
        let Some(Some((was, proxy))) = self.objects.get(usize::try_from(i).unwrap_or(usize::MAX)).cloned() else {
            return;
        };
        if !Arc::ptr_eq(&was, &now) {
            self.tree.update(proxy, now.bbox);
            self.set_slot(i, Some((Arc::clone(&now), proxy)));
            self.restructured();
        }
        self.changed(now.ink());
    }

    /// What tiles show changed within world box `area`.
    fn changed(&mut self, area: Aabb) {
        self.version = self.version.saturating_add(1);
        if area.iter().all(|v| v.is_finite()) && area[0] <= area[2] && area[1] <= area[3] {
            self.damage.push_back((self.version, area));
        }
        while self.damage.len() > DAMAGE_KEPT {
            if let Some((v, _)) = self.damage.pop_front() {
                self.damage_from = v;
            }
        }
        *self.damage_list.get_mut() = None;
        *self.snapshot.get_mut() = None;
    }

    /// The objects, their order or their boxes changed: the base is made
    /// again (the ranks too when the order changed other than on top, the
    /// tree copy when enough changed since it was made).
    fn restructured(&mut self) {
        if self.reranked {
            let mut rank = vec![0_u32; self.objects.len()];
            for (r, (_, i)) in self.order.iter().enumerate() {
                if let Some(slot) = rank.get_mut(usize::try_from(*i).unwrap_or(usize::MAX)) {
                    *slot = u32::try_from(r).unwrap_or(u32::MAX);
                }
            }
            self.next_rank = u32::try_from(self.order.len()).unwrap_or(u32::MAX);
            self.rank = Arc::new(rank);
            self.reranked = false;
        }
        if self.fresh.len() > FOLD {
            self.tree_copy = Arc::new(self.tree.clone());
            self.fresh.clear();
            self.extent = self
                .objects
                .iter()
                .flatten()
                .fold(NOTHING, |b, (p, _)| union(b, p.bbox));
        }
        *self.base.get_mut() = None;
        *self.snapshot.get_mut() = None;
    }

    /// The live layer changed.
    fn live_changed(&mut self) {
        self.live_version = self.live_version.saturating_add(1);
        *self.snapshot.get_mut() = None;
    }

    fn build_base(&self) -> Arc<Base> {
        Arc::new(Base {
            pages: self.pages.clone(),
            rank: Arc::clone(&self.rank),
            tree: Arc::clone(&self.tree_copy),
            fresh: self.fresh.iter().copied().collect(),
            extent: self.extent,
        })
    }

    fn build(&self) -> Arc<WorldSnapshot> {
        let base = Arc::clone(self.base.borrow_mut().get_or_insert_with(|| self.build_base()));
        let damage = Arc::clone(
            self.damage_list
                .borrow_mut()
                .get_or_insert_with(|| self.damage.iter().copied().collect()),
        );
        Arc::new(WorldSnapshot {
            id: self.id,
            version: self.version,
            live_version: self.live_version,
            base,
            live: self.lifted.iter().map(|(i, p)| (*i, Arc::clone(p))).collect(),
            damage,
            damage_from: self.damage_from,
            unshaped: {
                let mut u = self.unshaped.borrow_mut();
                u.retain(|i| {
                    self.objects
                        .get(usize::try_from(*i).unwrap_or(usize::MAX))
                        .and_then(Option::as_ref)
                        .is_some_and(|(p, _)| matches!(&p.body, Body::Text(t) if t.shaped.get().is_none()))
                });
                u.clone()
            },
        })
    }
}

/// A canvas as one frame shows it: the world, the camera, how it is drawn.
#[derive(Clone, Debug)]
pub struct WorldView {
    pub key: u64,
    /// The view's box (logical, its paint chunk's space).
    pub bounds: Bounds,
    pub world: Arc<WorldSnapshot>,
    pub camera: Camera,
    /// Draw from tiles (many objects visible, the camera moving).
    pub tiled: bool,
}

impl PartialEq for WorldView {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.bounds == other.bounds
            && self.camera == other.camera
            && self.tiled == other.tiled
            && same_world(&self.world, &other.world)
    }
}

fn same_world(a: &WorldSnapshot, b: &WorldSnapshot) -> bool {
    a.id == b.id && a.version == b.version && a.live_version == b.live_version
}

#[derive(Debug)]
struct CanvasPicture {
    world: Arc<WorldSnapshot>,
    camera: Camera,
}

impl PartialEq for CanvasPicture {
    fn eq(&self, other: &Self) -> bool {
        self.camera == other.camera && same_world(&self.world, &other.world)
    }
}

impl Picture for CanvasPicture {
    fn shape(&self, text: &mut crate::text::TextSystem) {
        let base = self.world.unshaped.iter().filter_map(|i| self.world.object(*i));
        for o in base.chain(self.world.live.iter().map(|(_, o)| o)) {
            if let Body::Text(t) = &o.body
                && t.shaped.get().is_none()
            {
                t.shaped.set(shape_text(t, text)).ok();
            }
        }
    }

    fn draw(&self, area: Bounds, _scale: f32, key: u64, out: &mut Vec<Item>) {
        let size = (f64::from(area.width), f64::from(area.height));
        let visible = self.world.tree_count(&self.camera.visible(size));
        out.push(Item::World(Arc::new(WorldView {
            key,
            bounds: area,
            world: Arc::clone(&self.world),
            camera: self.camera,
            tiled: visible > TILE_ABOVE,
        })));
    }
}

/// Shape `t` with `text`: its glyphs set at [`SHAPE_PPEM`] device pixels
/// an em (as the text system sets a paragraph: fallback, bidi, and lines
/// broken to its width as UI text breaks to its box), their pens and its
/// lines in ems from the top left.
fn shape_text(t: &TextBody, text: &mut crate::text::TextSystem) -> Shaped {
    let mut style = t.style;
    // Shaped at 16 px and scale 64: an em of 1024 device pixels.
    style.size = 16.0;
    style.line_height = style.line_height.max(1.0) / t.style.size.max(1.0) * 16.0;
    style.nowrap = t.width.is_none();
    let width = t
        .width
        .filter(|w| *w > 0.0 && t.size > 0.0)
        .map(|w| crate::convert::f64_to_f32(w / t.size * f64::from(style.size)));
    let scale = SHAPE_PPEM / style.size;
    let paragraph = || crate::text::Paragraph::from(t.content.as_str());
    let mut items = Vec::new();
    let mut target = crate::text::Target {
        items: &mut items,
        scale,
        size: (1, 1),
        cull: None,
        snap: false,
    };
    text.draw(&mut target, paragraph(), &style, width, (0.0, 0.0), 1.0);
    let (w, h) = text.measure(paragraph(), &style, width);
    let px = f64::from(style.size);
    let lines = text
        .selection(paragraph(), &style, width, 0, t.content.len())
        .iter()
        .map(|b| {
            [
                f64::from(b.x) / px,
                f64::from(b.y) / px,
                f64::from(b.right()) / px,
                f64::from(b.bottom()) / px,
            ]
        })
        .collect();
    let em = f64::from(SHAPE_PPEM);
    let glyphs = items
        .iter()
        .filter_map(|item| match item {
            Item::Glyph { key, x, y, color } => {
                let pen = f64::from(*x) + f64::from(key.bin) / 4.0;
                Some((*key, (pen / em, f64::from(*y) / em), *color))
            }
            _ => None,
        })
        .collect();
    Shaped {
        glyphs,
        width: f64::from(w) / px,
        height: f64::from(h) / px,
        lines,
    }
}

impl WorldSnapshot {
    /// About how many objects meet `area` (stopping past the tiling bound).
    fn tree_count(&self, area: &Aabb) -> usize {
        self.base.count(area, TILE_ABOVE.saturating_add(1))
    }
}

/// A canvas showing `world` through `camera`: dragging pans it, the wheel
/// zooms it about the pointer, and the camera settles 150 ms after the
/// last move (give it a size).
#[must_use]
pub fn canvas(world: World, camera: Signal<Camera>) -> Drawn {
    canvas_with(world, camera, None)
}

/// A canvas whose objects `movable` takes are dragged (as a design tool
/// drags what is not locked): pressing on one lifts it into the live layer
/// ([`World::lift`]), moving moves it there, releasing lands it; pressing
/// elsewhere pans.
#[must_use]
pub fn canvas_editor(world: World, camera: Signal<Camera>, movable: impl Fn(ObjectId) -> bool + 'static) -> Drawn {
    canvas_with(world, camera, Some(Rc::new(movable)))
}

/// What a drag on a canvas is doing.
#[derive(Clone, Copy)]
enum Gesture {
    Pan((f32, f32)),
    /// Moving an object: the world point under the pointer when it last moved.
    Move(ObjectId, (f64, f64)),
}

type Movable = Rc<dyn Fn(ObjectId) -> bool>;

fn canvas_with(world: World, camera: Signal<Camera>, movable: Option<Movable>) -> Drawn {
    let gesture: Rc<std::cell::Cell<Option<Gesture>>> = Rc::new(std::cell::Cell::new(None));
    let settle: Rc<std::cell::Cell<Option<crate::clock::TimerId>>> = Rc::new(std::cell::Cell::new(None));
    let moved = {
        let settle = Rc::clone(&settle);
        move || {
            if let Some(t) = settle.take() {
                crate::clock::cancel(t);
            }
            settle.set(Some(crate::clock::after(
                std::time::Duration::from_millis(150),
                move || {
                    camera.update(|c| c.settled = true);
                },
            )));
        }
    };
    let pan = moved.clone();
    drawing(move || {
        Drawing::new(CanvasPicture {
            world: world.snapshot(),
            camera: camera.get(),
        })
    })
    .on_drag(move |e| {
        use crate::events::DragPhase;
        let size = (f64::from(e.bounds.width), f64::from(e.bounds.height));
        let at = |x: f32, y: f32| {
            camera
                .get_untracked()
                .to_world((f64::from(x - e.bounds.x), f64::from(y - e.bounds.y)), size)
        };
        if e.phase == DragPhase::Start {
            let cam = camera.get_untracked();
            let w = at(e.x, e.y);
            // Within 3 px of an object's box.
            let hit = movable
                .as_ref()
                .and_then(|m| world.hit_where(w, 3.0 / cam.zoom.max(f64::MIN_POSITIVE), |id| m(id)));
            gesture.set(Some(match hit {
                Some(id) => {
                    world.lift(id);
                    Gesture::Move(id, w)
                }
                None => Gesture::Pan((e.x, e.y)),
            }));
            return;
        }
        match gesture.get() {
            Some(Gesture::Pan((x, y))) => {
                let (dx, dy) = (f64::from(e.x - x), f64::from(e.y - y));
                camera.update(|c| {
                    *c = c.panned((dx, dy));
                    c.settled = false;
                });
                pan();
                gesture.set((e.phase != DragPhase::End).then_some(Gesture::Pan((e.x, e.y))));
            }
            Some(Gesture::Move(id, last)) => {
                let w = at(e.x, e.y);
                if w != last {
                    world.translate(id, (w.0 - last.0, w.1 - last.1));
                }
                if e.phase == DragPhase::End {
                    world.land(id);
                    gesture.set(None);
                } else {
                    gesture.set(Some(Gesture::Move(id, w)));
                }
            }
            None => {}
        }
    })
    .on_wheel(move |e| {
        let size = (f64::from(e.bounds.width), f64::from(e.bounds.height));
        let at = (f64::from(e.x - e.bounds.x), f64::from(e.y - e.bounds.y));
        let factor = (-f64::from(e.dy) / 240.0).exp2();
        camera.update(|c| {
            *c = c.zoomed(factor, at, size);
            c.settled = false;
        });
        moved();
    })
}

#[cfg(test)]
mod tests;
