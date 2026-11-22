//! Glyph outlines to coverage, on the CPU, once per glyph, size, variation
//! and subpixel position (ADR-0019). An outline is read from skrifa (hinted
//! by its autohinter below 2×), optionally thickened (a synthetic bold, or
//! macOS's stem darkening), and filled with exact area coverage: each line
//! segment adds the signed area it covers to an accumulation buffer, whose
//! running sum along a row is the pixel's coverage, as FreeType's smooth
//! rasterizer and font-rs compute it. Subpixel (LCD) coverage is the same
//! fill at three times the resolution across the stripes, run through
//! FreeType's default five-tap filter.

use super::settings::Subpixel;
use crate::convert::{f32_to_i32, i32_to_f32, u32_to_f32};

/// A point of an outline in device pixels, y down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Point {
    pub x: f32,
    pub y: f32,
}

/// What a point of a contour is, as FreeType tags outline points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tag {
    On,
    /// The control point of a quadratic curve.
    Quad,
    /// One of the two control points of a cubic curve.
    Cubic,
}

/// A glyph outline: closed contours of tagged points, in device pixels.
#[derive(Clone, Debug, Default)]
pub(crate) struct Outline {
    contours: Vec<Vec<(Point, Tag)>>,
    /// Applied to every point a pen hands in: font units already scaled to
    /// pixels, y flipped, a slant, a scale across stripes, an offset.
    transform: Affine,
}

/// `x' = xx·x + xy·y + dx`, `y' = yy·y + dy`: all a glyph needs (no rotation).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Affine {
    pub xx: f32,
    pub xy: f32,
    pub yy: f32,
    pub dx: f32,
    pub dy: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Self {
            xx: 1.0,
            xy: 0.0,
            yy: -1.0,
            dx: 0.0,
            dy: 0.0,
        }
    }
}

impl Affine {
    fn apply(&self, x: f32, y: f32) -> Point {
        Point {
            x: self.xx * x + self.xy * y + self.dx,
            y: self.yy * y + self.dy,
        }
    }
}

impl Outline {
    /// An empty outline whose pen input goes through `transform` (font
    /// pixels, y up, to device pixels, y down).
    pub(crate) fn new(transform: Affine) -> Self {
        Self {
            contours: Vec::new(),
            transform,
        }
    }

    fn push(&mut self, x: f32, y: f32, tag: Tag) {
        let p = self.transform.apply(x, y);
        if let Some(c) = self.contours.last_mut() {
            c.push((p, tag));
        }
    }

    /// Whether the outline has nothing to fill.
    pub(crate) fn is_empty(&self) -> bool {
        self.contours.iter().all(|c| c.len() < 2)
    }

    /// The outline's control box: every point, so every curve, lies in it.
    pub(crate) fn bounds(&self) -> Option<(Point, Point)> {
        let mut points = self.contours.iter().flatten().map(|(p, _)| *p);
        let first = points.next()?;
        Some(points.fold((first, first), |(lo, hi), p| {
            (
                Point {
                    x: lo.x.min(p.x),
                    y: lo.y.min(p.y),
                },
                Point {
                    x: hi.x.max(p.x),
                    y: hi.y.max(p.y),
                },
            )
        }))
    }

    /// Move every point by `(dx, dy)`.
    pub(crate) fn translate(&mut self, dx: f32, dy: f32) {
        for (p, _) in self.contours.iter_mut().flatten() {
            p.x += dx;
            p.y += dy;
        }
    }

    /// The signed area of the polygon through every point: positive when
    /// the outer contours turn one way, negative the other. Which way ink
    /// lies of an edge follows from it, as `FT_Outline_Get_Orientation`.
    fn orientation(&self) -> f32 {
        let mut area = 0.0;
        for c in &self.contours {
            let Some(&(last, _)) = c.last() else { continue };
            let mut prev = last;
            for &(p, _) in c {
                area += prev.x * p.y - p.x * prev.y;
                prev = p;
            }
        }
        area
    }

    /// Thicken every stem by `sx` horizontally and `sy` vertically on each
    /// side (strokes grow by twice that), moving each point along the
    /// bisector of its two edges so each edge moves out by the amount, as
    /// FreeType's `FT_Outline_EmboldenXY` does (without its shift right:
    /// the glyph grows on both sides, as Skia's fake bold and CoreText's
    /// smoothing do). Counters shrink by as much.
    pub(crate) fn embolden(&mut self, sx: f32, sy: f32) {
        if sx <= 0.0 && sy <= 0.0 {
            return;
        }
        let sign = if self.orientation() >= 0.0 { 1.0 } else { -1.0 };
        for c in &mut self.contours {
            let n = c.len();
            if n < 3 {
                continue;
            }
            let points: Vec<Point> = c.iter().map(|(p, _)| *p).collect();
            let unit = |from: Point, to: Point| {
                let (dx, dy) = (to.x - from.x, to.y - from.y);
                let len = dx.hypot(dy);
                (len > 1e-6).then(|| (dx / len, dy / len))
            };
            // The nearest other point before and after `i`, wrapping.
            let neighbour = |i: usize, back: bool| -> Option<Point> {
                let here = *points.get(i)?;
                (1..n).find_map(|k| {
                    let j = if back {
                        i.checked_add(n)?.checked_sub(k)?.checked_rem(n)?
                    } else {
                        i.checked_add(k)?.checked_rem(n)?
                    };
                    points.get(j).copied().filter(|p| *p != here)
                })
            };
            for (i, (p, _)) in c.iter_mut().enumerate() {
                let here = *p;
                let (Some(prev), Some(next)) = (neighbour(i, true), neighbour(i, false)) else {
                    continue;
                };
                let (Some(a), Some(b)) = (unit(prev, here), unit(here, next)) else {
                    continue;
                };
                // The cosine of the turn, plus one; a turn back on itself
                // (a spike) is left alone, as FreeType leaves it.
                let d = a.0 * b.0 + a.1 * b.1 + 1.0;
                if d <= 0.0625 {
                    continue;
                }
                // The sum of both edges' outward normals.
                let (mut nx, mut ny) = (a.1 + b.1, -(a.0 + b.0));
                nx *= sign;
                ny *= sign;
                // The sine of the turn bounds the move at sharp corners.
                let q = sign * (b.0 * a.1 - b.1 * a.0);
                let l = sx.min(sy);
                let (mx, my) = if q <= l * d {
                    (nx * sx / d, ny * sy / d)
                } else {
                    (nx * l * sx / q, ny * l * sy / q)
                };
                p.x = here.x + mx;
                p.y = here.y + my;
            }
        }
    }

    /// The outline as line segments, curves flattened to within a
    /// twentieth of a pixel.
    fn lines(&self, mut out: impl FnMut(Point, Point)) {
        for c in &self.contours {
            let Some(&(start, _)) = c.first() else { continue };
            let mut at = start;
            let mut i = 1;
            while let Some(&(p, tag)) = c.get(i) {
                match tag {
                    Tag::On => {
                        out(at, p);
                        at = p;
                        i = i.saturating_add(1);
                    }
                    Tag::Quad => {
                        let end = c.get(i.saturating_add(1)).map_or(start, |(e, _)| *e);
                        quad(at, p, end, &mut out);
                        at = end;
                        i = i.saturating_add(2);
                    }
                    Tag::Cubic => {
                        let c2 = c.get(i.saturating_add(1)).map_or(p, |(e, _)| *e);
                        let end = c.get(i.saturating_add(2)).map_or(start, |(e, _)| *e);
                        cubic(at, p, c2, end, &mut out);
                        at = end;
                        i = i.saturating_add(3);
                    }
                }
            }
            if at != start {
                out(at, start);
            }
        }
    }
}

impl Outline {
    /// The outline as quadratic curves `(start, control, end)`, contour by
    /// contour, each contour closed: lines as curves whose control is their
    /// middle, cubics split into quadratics within `tolerance` (as
    /// hb-gpu's cu2qu does), for curve text on the GPU (Slug, ADR-0017).
    pub(crate) fn quadratics(&self, tolerance: f32) -> Vec<Vec<[Point; 3]>> {
        let mid = |a: Point, b: Point| Point {
            x: (a.x + b.x) / 2.0,
            y: (a.y + b.y) / 2.0,
        };
        let mut out = Vec::new();
        for c in &self.contours {
            let Some(&(start, _)) = c.first() else { continue };
            let mut curves: Vec<[Point; 3]> = Vec::new();
            let mut at = start;
            let mut i = 1;
            while let Some(&(p, tag)) = c.get(i) {
                match tag {
                    Tag::On => {
                        if p != at {
                            curves.push([at, mid(at, p), p]);
                        }
                        at = p;
                        i = i.saturating_add(1);
                    }
                    Tag::Quad => {
                        let end = c.get(i.saturating_add(1)).map_or(start, |(e, _)| *e);
                        curves.push([at, p, end]);
                        at = end;
                        i = i.saturating_add(2);
                    }
                    Tag::Cubic => {
                        let c2 = c.get(i.saturating_add(1)).map_or(p, |(e, _)| *e);
                        let end = c.get(i.saturating_add(2)).map_or(start, |(e, _)| *e);
                        cubic_to_quads(at, p, c2, end, tolerance, &mut curves);
                        at = end;
                        i = i.saturating_add(3);
                    }
                }
            }
            if at != start {
                curves.push([at, mid(at, start), start]);
            }
            if !curves.is_empty() {
                out.push(curves);
            }
        }
        out
    }
}

/// Cubic `p0..p3` as `n` quadratics, `n` the least that keeps them within
/// `tol` of it: the error of one quadratic per piece is bounded by
/// `√3/36 · |p3 − 3p2 + 3p1 − p0|` scaled by the piece's length cubed.
fn cubic_to_quads(p0: Point, p1: Point, p2: Point, p3: Point, tol: f32, out: &mut Vec<[Point; 3]>) {
    let dx = p3.x - 3.0 * p2.x + 3.0 * p1.x - p0.x;
    let dy = p3.y - 3.0 * p2.y + 3.0 * p1.y - p0.y;
    let err = (3.0_f32.sqrt() / 36.0) * dx.hypot(dy);
    let n = (err / tol.max(1e-6)).cbrt().ceil().clamp(1.0, 64.0);
    let n = crate::convert::f32_to_u32(n);
    let at = |t: f32| {
        let mt = 1.0 - t;
        let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
        Point {
            x: a * p0.x + b * p1.x + c * p2.x + d * p3.x,
            y: a * p0.y + b * p1.y + c * p2.y + d * p3.y,
        }
    };
    let tangent = |t: f32| {
        let mt = 1.0 - t;
        Point {
            x: 3.0 * (mt * mt * (p1.x - p0.x) + 2.0 * mt * t * (p2.x - p1.x) + t * t * (p3.x - p2.x)),
            y: 3.0 * (mt * mt * (p1.y - p0.y) + 2.0 * mt * t * (p2.y - p1.y) + t * t * (p3.y - p2.y)),
        }
    };
    for k in 0..n {
        let t0 = u32_to_f32(k) / u32_to_f32(n);
        let t1 = u32_to_f32(k.saturating_add(1)) / u32_to_f32(n);
        let (a, b) = (at(t0), at(t1));
        let (ta, tb) = (tangent(t0), tangent(t1));
        let h = (t1 - t0) / 2.0;
        // The control point: where the end tangents' halves meet, averaged
        // (the midpoint quadratic of the piece).
        let c = Point {
            x: ((a.x + ta.x * h) + (b.x - tb.x * h)) / 2.0,
            y: ((a.y + ta.y * h) + (b.y - tb.y * h)) / 2.0,
        };
        out.push([a, c, b]);
    }
}

impl skrifa::outline::OutlinePen for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contours.push(Vec::new());
        self.push(x, y, Tag::On);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.push(x, y, Tag::On);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.push(cx0, cy0, Tag::Quad);
        self.push(x, y, Tag::On);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.push(cx0, cy0, Tag::Cubic);
        self.push(cx1, cy1, Tag::Cubic);
        self.push(x, y, Tag::On);
    }

    fn close(&mut self) {}
}

/// Flattening tolerance, in pixels.
const TOLERANCE: f32 = 0.05;

/// Segments for a curve whose control polygon bends by `dev` pixels.
fn segments(dev: f32) -> u32 {
    let n = (dev / (8.0 * TOLERANCE)).sqrt().ceil().clamp(1.0, 64.0);
    crate::convert::f32_to_u32(n)
}

fn quad(p0: Point, p1: Point, p2: Point, out: &mut impl FnMut(Point, Point)) {
    let dev = (p0.x - 2.0 * p1.x + p2.x).hypot(p0.y - 2.0 * p1.y + p2.y);
    let n = segments(dev * 2.0);
    let mut prev = p0;
    for k in 1..=n {
        let t = u32_to_f32(k) / u32_to_f32(n);
        let mt = 1.0 - t;
        let p = Point {
            x: mt * mt * p0.x + 2.0 * mt * t * p1.x + t * t * p2.x,
            y: mt * mt * p0.y + 2.0 * mt * t * p1.y + t * t * p2.y,
        };
        out(prev, p);
        prev = p;
    }
}

fn cubic(p0: Point, p1: Point, p2: Point, p3: Point, out: &mut impl FnMut(Point, Point)) {
    let d1 = (p0.x - 2.0 * p1.x + p2.x).hypot(p0.y - 2.0 * p1.y + p2.y);
    let d2 = (p1.x - 2.0 * p2.x + p3.x).hypot(p1.y - 2.0 * p2.y + p3.y);
    let n = segments(d1.max(d2) * 6.0);
    let mut prev = p0;
    for k in 1..=n {
        let t = u32_to_f32(k) / u32_to_f32(n);
        let mt = 1.0 - t;
        let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
        let p = Point {
            x: a * p0.x + b * p1.x + c * p2.x + d * p3.x,
            y: a * p0.y + b * p1.y + c * p2.y + d * p3.y,
        };
        out(prev, p);
        prev = p;
    }
}

/// Coverage of `outline`, which must lie in `0..width` × `0..height`
/// (exclusive of the last column, which takes the spill of the rightmost
/// edges), one byte per pixel, rows top to bottom.
pub(crate) fn fill(outline: &Outline, width: u32, height: u32) -> Vec<u8> {
    let mut acc = Accumulator::new(width, height);
    outline.lines(|a, b| acc.line(a, b));
    acc.coverage()
}

/// Signed areas each edge leaves in each pixel, summed along a row into
/// coverage (Raph Levien's font-rs).
struct Accumulator {
    width: usize,
    height: usize,
    cells: Vec<f32>,
}

impl Accumulator {
    fn new(width: u32, height: u32) -> Self {
        let width = usize::try_from(width).unwrap_or(0);
        let height = usize::try_from(height).unwrap_or(0);
        Self {
            width,
            height,
            cells: vec![0.0; width.saturating_mul(height).saturating_add(2)],
        }
    }

    fn add(&mut self, row: usize, x: i32, v: f32) {
        let Ok(x) = usize::try_from(x) else { return };
        if x >= self.width {
            return;
        }
        if let Some(c) = row
            .checked_mul(self.width)
            .and_then(|r| r.checked_add(x))
            .and_then(|i| self.cells.get_mut(i))
        {
            *c += v;
        }
    }

    fn line(&mut self, p0: Point, p1: Point) {
        if (p0.y - p1.y).abs() < f32::EPSILON {
            return;
        }
        let (dir, p0, p1) = if p0.y < p1.y { (1.0, p0, p1) } else { (-1.0, p1, p0) };
        let dxdy = (p1.x - p0.x) / (p1.y - p0.y);
        let mut x = p0.x;
        let top = p0.y.max(0.0);
        if p0.y < 0.0 {
            x -= p0.y * dxdy;
        }
        let first = f32_to_i32(top.floor());
        let last = f32_to_i32(p1.y.ceil()).min(i32::try_from(self.height).unwrap_or(i32::MAX));
        for y in first..last {
            let Ok(row) = usize::try_from(y) else { continue };
            let yf = i32_to_f32(y);
            let dy = (yf + 1.0).min(p1.y) - yf.max(p0.y);
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            let (x0, x1) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let x0floor = x0.floor();
            let x0i = f32_to_i32(x0floor);
            let x1ceil = x1.ceil();
            let x1i = f32_to_i32(x1ceil);
            if x1i <= x0i.saturating_add(1) {
                // Within one pixel: the area right of the edge's midpoint.
                let xmf = 0.5 * (x + xnext) - x0floor;
                self.add(row, x0i, d - d * xmf);
                self.add(row, x0i.saturating_add(1), d * xmf);
            } else {
                let s = (x1 - x0).recip();
                let x0f = x0 - x0floor;
                let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
                let x1f = x1 - x1ceil + 1.0;
                let am = 0.5 * s * x1f * x1f;
                self.add(row, x0i, d * a0);
                if x1i == x0i.saturating_add(2) {
                    self.add(row, x0i.saturating_add(1), d * (1.0 - a0 - am));
                } else {
                    let a1 = s * (1.5 - x0f);
                    self.add(row, x0i.saturating_add(1), d * (a1 - a0));
                    for xi in x0i.saturating_add(2)..x1i.saturating_sub(1) {
                        self.add(row, xi, d * s);
                    }
                    let a2 = a1 + i32_to_f32(x1i.saturating_sub(x0i).saturating_sub(3)) * s;
                    self.add(row, x1i.saturating_sub(1), d * (1.0 - a2 - am));
                }
                self.add(row, x1i, d * am);
            }
            x = xnext;
        }
    }

    /// Each row's running sum, its magnitude capped at full coverage: the
    /// nonzero rule wherever contours of one direction overlap.
    fn coverage(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width.saturating_mul(self.height));
        for row in self.cells.chunks(self.width.max(1)).take(self.height) {
            let mut acc = 0.0_f32;
            for c in row {
                acc += c;
                out.push(crate::convert::unit_to_u8(acc.abs().min(1.0)));
            }
        }
        out
    }
}

/// FreeType's default LCD filter (`FT_LCD_FILTER_DEFAULT`), in 256ths: a
/// subpixel's coverage spread over it and two either side, so a stem lights
/// all three stripes of a pixel without colour fringes.
const LCD_FILTER: [u32; 5] = [0x08, 0x4D, 0x56, 0x4D, 0x08];

/// Filter one line of subpixel coverage (three per pixel) with
/// [`LCD_FILTER`]; the line is `n` wide and the result as wide.
pub(crate) fn lcd_filter(line: &[u8]) -> Vec<u8> {
    let at = |i: usize, k: usize| -> u32 {
        // `i + k - 2`, the tap's subpixel.
        i.checked_add(k)
            .and_then(|j| j.checked_sub(2))
            .and_then(|j| line.get(j))
            .map_or(0, |v| u32::from(*v))
    };
    (0..line.len())
        .map(|i| {
            let sum = LCD_FILTER
                .iter()
                .enumerate()
                .fold(0_u32, |s, (k, w)| s.saturating_add(at(i, k).saturating_mul(*w)));
            u8::try_from(sum.saturating_add(128) / 256).unwrap_or(u8::MAX)
        })
        .collect()
}

fn vertical(stripes: Subpixel) -> bool {
    matches!(stripes, Subpixel::VRgb | Subpixel::VBgr)
}

fn reversed(stripes: Subpixel) -> bool {
    matches!(stripes, Subpixel::Bgr | Subpixel::VBgr)
}

/// Subpixel coverage `(r, g, b)` of a glyph filled at three times the
/// resolution across `stripes` (`coverage`, `width`×`height` of the tripled
/// raster), filtered and packed three subpixels to a pixel as RGBA, alpha the
/// most of the three. `width` (or `height`, for vertical stripes) is a
/// multiple of three.
pub(crate) fn lcd_pack(coverage: &[u8], width: u32, height: u32, stripes: Subpixel) -> (u32, u32, Vec<u8>) {
    let (w, h) = (
        usize::try_from(width).unwrap_or(0),
        usize::try_from(height).unwrap_or(0),
    );
    let pixel = |r: u8, g: u8, b: u8| {
        let (r, b) = if reversed(stripes) { (b, r) } else { (r, b) };
        [r, g, b, r.max(g).max(b)]
    };
    if vertical(stripes) {
        // Filter each column, then take rows three at a time.
        let mut filtered = vec![0_u8; w.saturating_mul(h)];
        for x in 0..w {
            let column: Vec<u8> = (0..h)
                .map(|y| {
                    y.checked_mul(w)
                        .and_then(|i| i.checked_add(x))
                        .and_then(|i| coverage.get(i))
                        .copied()
                        .unwrap_or(0)
                })
                .collect();
            for (y, v) in lcd_filter(&column).into_iter().enumerate() {
                if let Some(c) = y
                    .checked_mul(w)
                    .and_then(|i| i.checked_add(x))
                    .and_then(|i| filtered.get_mut(i))
                {
                    *c = v;
                }
            }
        }
        let out_h = h / 3;
        let mut out = Vec::with_capacity(w.saturating_mul(out_h).saturating_mul(4));
        let get = |y: usize, x: usize| {
            y.checked_mul(w)
                .and_then(|i| i.checked_add(x))
                .and_then(|i| filtered.get(i))
                .copied()
                .unwrap_or(0)
        };
        for y in 0..out_h {
            let y3 = y.saturating_mul(3);
            for x in 0..w {
                out.extend(pixel(
                    get(y3, x),
                    get(y3.saturating_add(1), x),
                    get(y3.saturating_add(2), x),
                ));
            }
        }
        return (width, u32::try_from(out_h).unwrap_or(0), out);
    }
    let out_w = w / 3;
    let mut out = Vec::with_capacity(out_w.saturating_mul(h).saturating_mul(4));
    for row in coverage.chunks(w.max(1)).take(h) {
        let line = lcd_filter(row);
        for &[r, g, b] in line.as_chunks::<3>().0 {
            out.extend(pixel(r, g, b));
        }
    }
    (u32::try_from(out_w).unwrap_or(0), height, out)
}

/// Skia's fake bold: the outline is stroked `size / 24` wide at 9 px and
/// below, `size / 32` from 36 px, in between interpolated
/// (`SkScalerContext`'s `kStdFakeBoldInterp`); returned per side.
pub(crate) fn fake_bold(ppem: f32) -> f32 {
    let t = ((ppem - 9.0) / (36.0 - 9.0)).clamp(0.0, 1.0);
    let ratio = 1.0 / 24.0 + (1.0 / 32.0 - 1.0 / 24.0) * t;
    ppem * ratio / 2.0
}

/// How much macOS's font smoothing thickens a stem at `ppem`, per side, in
/// x and y. Fitted to Chrome on macOS (CoreText's smoothing, black on
/// white) with the text sample (`verdict/text.sh`): it grows with the size
/// up to 0.45 px at 41 ppem and stays there; from 256 ppem Chrome draws
/// glyphs as paths, with none. Pathfinder's constants (0.0121 per ppem, at
/// most 0.3 px, none above 72 ppem, for macOS 10.13) left text 5 to 12%
/// lighter than Chrome draws it now. Vertically 1.25 times as much, as
/// Pathfinder measured.
pub(crate) fn stem_darkening(ppem: f32) -> (f32, f32) {
    if ppem >= 256.0 {
        return (0.0, 0.0);
    }
    let x = (0.1 + 0.0085 * ppem).min(0.45);
    (x, x * 1.25)
}

/// The whole pixels holding `bounds`, with a pixel to spare on the right
/// for the rasterizer's spill: `(left, top, width, height)`.
pub(crate) fn pixel_box(lo: Point, hi: Point) -> (i32, i32, u32, u32) {
    let left = f32_to_i32(lo.x.floor());
    let top = f32_to_i32(lo.y.floor());
    let right = f32_to_i32(hi.x.ceil()).saturating_add(1);
    let bottom = f32_to_i32(hi.y.ceil());
    let w = u32::try_from(right.saturating_sub(left)).unwrap_or(0);
    let h = u32::try_from(bottom.saturating_sub(top)).unwrap_or(0);
    (left, top, w, h)
}

/// A fractional offset as a float, for placing an outline in its box.
pub(crate) fn offset(v: i32) -> f32 {
    i32_to_f32(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrifa::outline::OutlinePen;

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Outline {
        // Pen space is y up; the default transform flips it.
        let mut o = Outline::new(Affine {
            yy: 1.0,
            ..Affine::default()
        });
        o.move_to(x0, y0);
        o.line_to(x1, y0);
        o.line_to(x1, y1);
        o.line_to(x0, y1);
        o.close();
        o
    }

    #[test]
    fn a_square_covers_exactly_its_area() {
        let o = square(1.25, 1.0, 3.75, 3.0);
        let cov = fill(&o, 6, 4);
        let row: Vec<u8> = cov[6..12].to_vec();
        // A quarter of pixel 1, all of 2, three quarters of 3.
        assert_eq!(row, vec![0, 191, 255, 191, 0, 0]);
        let total: u32 = cov.iter().map(|v| u32::from(*v)).sum();
        assert!((i64::from(total) - 2 * 637).abs() <= 4, "{total}");
    }

    #[test]
    fn overlapping_contours_of_one_direction_fill_once() {
        let mut o = square(0.0, 0.0, 4.0, 2.0);
        let b = square(2.0, 0.0, 6.0, 2.0);
        o.contours.extend(b.contours);
        let cov = fill(&o, 8, 2);
        assert_eq!(&cov[..7], &[255, 255, 255, 255, 255, 255, 0]);
    }

    #[test]
    fn emboldening_grows_a_stem_on_both_sides() {
        let mut o = square(2.0, 0.0, 4.0, 4.0);
        o.embolden(0.5, 0.0);
        let (lo, hi) = o.bounds().unwrap();
        assert!((lo.x - 1.5).abs() < 1e-4 && (hi.x - 4.5).abs() < 1e-4, "{lo:?} {hi:?}");
        assert!((lo.y - 0.0).abs() < 1e-4 && (hi.y - 4.0).abs() < 1e-4, "{lo:?} {hi:?}");
    }

    #[test]
    fn the_lcd_filter_spreads_a_subpixel_over_five_and_keeps_its_sum() {
        let out = lcd_filter(&[0, 0, 0, 255, 0, 0, 0]);
        assert_eq!(out, vec![0, 8, 77, 86, 77, 8, 0]);
        // A full pixel (three lit subpixels) in RGB order lights all three,
        // its centre most, and bleeds a little into the pixels beside it.
        let (w, h, px) = lcd_pack(&[0, 0, 0, 255, 255, 255, 0, 0, 0], 9, 1, Subpixel::Rgb);
        assert_eq!((w, h), (3, 1));
        assert_eq!(px[4..8], [170, 239, 170, 239]);
        assert!(px[2] > 0 && px[8] > 0, "{px:?}");
        let (_, _, bgr) = lcd_pack(&[0, 0, 0, 255, 255, 0, 0, 0, 0], 9, 1, Subpixel::Bgr);
        let (_, _, rgb) = lcd_pack(&[0, 0, 0, 255, 255, 0, 0, 0, 0], 9, 1, Subpixel::Rgb);
        assert_eq!([rgb[4], rgb[6]], [bgr[6], bgr[4]], "BGR mirrors red and blue");
    }

    #[test]
    fn stem_darkening_grows_to_a_cap_and_stops_where_chrome_draws_paths() {
        let (x, y) = stem_darkening(12.0);
        assert!((x - 0.202).abs() < 1e-4 && (y - 0.2525).abs() < 1e-4, "{x} {y}");
        assert_eq!(stem_darkening(100.0), (0.45, 0.5625));
        assert_eq!(stem_darkening(256.0), (0.0, 0.0));
        assert!((fake_bold(9.0) - 9.0 / 48.0).abs() < 1e-6);
    }
}
