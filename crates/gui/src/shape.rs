//! Vector shapes an element draws in its own box (chart lines, areas, bars,
//! dots, icons): a [`Path`] and the [`PathStyle`] it is painted with
//! (ADR-0017). Coordinates are logical pixels from the element's top left.
//!
//! A path is immutable and shared (`Arc`); its key is a hash of its
//! content, so a path built again with the same points is the same path to
//! the renderer, which keeps what it binned for it. Each curve's Wang
//! constant (its largest second difference) and control-polygon length are
//! kept with it: the renderer bounds, from those and a transform's largest
//! scale, how many segments and tiles the GPU may write.

use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::Arc;

use crate::color::Color;
use crate::geometry::Bounds;
use crate::scene::Gradient;
use crate::style::Corners;

/// A quarter circle's cubic control distance, per unit of radius.
const KAPPA: f32 = 0.552_284_8;

/// One step of a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Verb {
    /// Starts a subpath at the next point.
    Move,
    /// To the next point.
    Line,
    /// Through the next control point to the point after it.
    Quad,
    /// Through the next two control points to the point after them.
    Cubic,
    /// Back to the subpath's start.
    Close,
}

/// Which points a fill covers, as SVG's `fill-rule`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// One curve of a path in a renderer's terms: its points (two for a line,
/// three for a quadratic, four for a cubic), where it sits in its subpath.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    pub points: [[f32; 2]; 4],
    /// 2, 3 or 4 points.
    pub order: u8,
    /// Its largest second difference (0 for a line): Wang's formula's constant.
    pub wang: f32,
    /// Its control polygon's length, which its arc length never exceeds.
    pub length: f32,
}

impl Curve {
    fn new(points: &[[f32; 2]]) -> Self {
        let mut p = [[0.0; 2]; 4];
        for (d, s) in p.iter_mut().zip(points) {
            *d = *s;
        }
        let order = u8::try_from(points.len()).unwrap_or(4);
        let second = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| (a[0] - 2.0 * b[0] + c[0]).hypot(a[1] - 2.0 * b[1] + c[1]);
        let wang = match points {
            [a, b, c] => second(*a, *b, *c),
            [a, b, c, d] => second(*a, *b, *c).max(second(*b, *c, *d)),
            _ => 0.0,
        };
        let length = points
            .windows(2)
            .map(|w| match w {
                [a, b] => (b[0] - a[0]).hypot(b[1] - a[1]),
                _ => 0.0,
            })
            .sum();
        Self {
            points: p,
            order,
            wang,
            length,
        }
    }

    /// The first point.
    #[must_use]
    pub fn start(&self) -> [f32; 2] {
        self.points[0]
    }

    /// The last point.
    #[must_use]
    pub fn end(&self) -> [f32; 2] {
        let [a, b, c, d] = self.points;
        match self.order {
            0 | 1 => a,
            2 => b,
            3 => c,
            _ => d,
        }
    }

    /// The direction it leaves its start in (zero for a point).
    #[must_use]
    pub fn start_tangent(&self) -> [f32; 2] {
        let p0 = self.points[0];
        for q in self.points.iter().take(usize::from(self.order)).skip(1) {
            let d = [q[0] - p0[0], q[1] - p0[1]];
            if d[0] != 0.0 || d[1] != 0.0 {
                return d;
            }
        }
        [0.0, 0.0]
    }

    /// The direction it arrives at its end in (zero for a point).
    #[must_use]
    pub fn end_tangent(&self) -> [f32; 2] {
        let n = usize::from(self.order).min(4);
        let end = self.end();
        for q in self.points.iter().take(n.saturating_sub(1)).rev() {
            let d = [end[0] - q[0], end[1] - q[1]];
            if d[0] != 0.0 || d[1] != 0.0 {
                return d;
            }
        }
        [0.0, 0.0]
    }

    /// The point at `t`.
    #[must_use]
    pub fn eval(&self, t: f32) -> [f32; 2] {
        let p = self.points;
        let u = 1.0 - t;
        match self.order {
            2 => [u * p[0][0] + t * p[1][0], u * p[0][1] + t * p[1][1]],
            3 => {
                let (a, b, c) = (u * u, 2.0 * u * t, t * t);
                [
                    a * p[0][0] + b * p[1][0] + c * p[2][0],
                    a * p[0][1] + b * p[1][1] + c * p[2][1],
                ]
            }
            _ => {
                let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                [
                    a * p[0][0] + b * p[1][0] + c * p[2][0] + d * p[3][0],
                    a * p[0][1] + b * p[1][1] + c * p[2][1] + d * p[3][1],
                ]
            }
        }
    }

    /// The part from `t0` to `t1`, as a curve of the same order (de Casteljau).
    #[must_use]
    pub fn segment(&self, t0: f32, t1: f32) -> Self {
        let n = usize::from(self.order).clamp(2, 4);
        let split = |pts: &[[f32; 2]], t: f32| -> (Vec<[f32; 2]>, Vec<[f32; 2]>) {
            let mut left = Vec::with_capacity(pts.len());
            let mut right = Vec::with_capacity(pts.len());
            let mut level: Vec<[f32; 2]> = pts.to_vec();
            while !level.is_empty() {
                if let Some(f) = level.first() {
                    left.push(*f);
                }
                if let Some(l) = level.last() {
                    right.push(*l);
                }
                level = level
                    .windows(2)
                    .map(|w| match w {
                        [a, b] => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                        _ => [0.0; 2],
                    })
                    .collect();
            }
            right.reverse();
            (left, right)
        };
        let pts: Vec<[f32; 2]> = self.points.iter().take(n).copied().collect();
        let (_, tail) = split(&pts, t0);
        let rest = if t0 < 1.0 { (t1 - t0) / (1.0 - t0) } else { 0.0 };
        let (piece, _) = split(&tail, rest.clamp(0.0, 1.0));
        Self::new(&piece)
    }
}

/// A path's data: its verbs, their points, its bounds, its fill rule and
/// the key it is cached by.
#[derive(Debug, PartialEq)]
pub struct PathData {
    verbs: Vec<Verb>,
    points: Vec<[f32; 2]>,
    /// Min x, min y, max x, max y of every point (so of every curve).
    bounds: [f32; 4],
    fill_rule: FillRule,
    key: u64,
}

/// An immutable path, shared; equal when their content is.
#[derive(Clone, Debug)]
pub struct Path(Arc<PathData>);

impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || (self.0.key == other.0.key && self.0 == other.0)
    }
}

/// One subpath as curves.
#[derive(Clone, Debug, PartialEq)]
pub struct Subpath {
    pub curves: Vec<Curve>,
    /// Ended by a close: its end joins its start.
    pub closed: bool,
    /// Where it starts (a subpath of no curves is a point there).
    pub start: [f32; 2],
}

impl Path {
    /// A new path from its first point.
    #[must_use]
    pub fn builder() -> PathBuilder {
        PathBuilder::default()
    }

    /// A rectangle.
    #[must_use]
    pub fn rect(x: f32, y: f32, w: f32, h: f32) -> PathBuilder {
        Self::builder()
            .move_to(x, y)
            .line_to(x + w, y)
            .line_to(x + w, y + h)
            .line_to(x, y + h)
            .close()
    }

    /// A circle, as four cubic arcs.
    #[must_use]
    pub fn circle(cx: f32, cy: f32, r: f32) -> PathBuilder {
        Self::ellipse(cx, cy, r, r)
    }

    /// An ellipse, as four cubic arcs.
    #[must_use]
    pub fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> PathBuilder {
        let (kx, ky) = (KAPPA * rx, KAPPA * ry);
        Self::builder()
            .move_to(cx + rx, cy)
            .cubic_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry)
            .cubic_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy)
            .cubic_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry)
            .cubic_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy)
            .close()
    }

    /// A rectangle with rounded corners, clockwise from the top left.
    #[must_use]
    pub fn rounded_rect(b: Bounds, r: Corners) -> PathBuilder {
        let half = (b.width / 2.0).min(b.height / 2.0).max(0.0);
        let [tl, tr, br, bl] = [r.top_left, r.top_right, r.bottom_right, r.bottom_left].map(|v| v.clamp(0.0, half));
        Self::rounded_rect_builder(Self::builder(), b, [tl, tr, br, bl])
    }

    fn rounded_rect_builder(p: PathBuilder, b: Bounds, [tl, tr, br, bl]: [f32; 4]) -> PathBuilder {
        let (x0, y0, x1, y1) = (b.x, b.y, b.x + b.width, b.y + b.height);
        let k = 1.0 - KAPPA;
        let mut p = p.move_to(x0 + tl, y0).line_to(x1 - tr, y0);
        if tr > 0.0 {
            p = p.cubic_to(x1 - tr * k, y0, x1, y0 + tr * k, x1, y0 + tr);
        }
        p = p.line_to(x1, y1 - br);
        if br > 0.0 {
            p = p.cubic_to(x1, y1 - br * k, x1 - br * k, y1, x1 - br, y1);
        }
        p = p.line_to(x0 + bl, y1);
        if bl > 0.0 {
            p = p.cubic_to(x0 + bl * k, y1, x0, y1 - bl * k, x0, y1 - bl);
        }
        p = p.line_to(x0, y0 + tl);
        if tl > 0.0 {
            p = p.cubic_to(x0, y0 + tl * k, x0 + tl * k, y0, x0 + tl, y0);
        }
        p.close()
    }

    /// The region between two rounded rectangles (a border), filled even-odd.
    #[must_use]
    pub fn band(outer: Bounds, outer_radius: Corners, inner: Bounds, inner_radius: Corners) -> PathBuilder {
        let clamp = |b: Bounds, r: Corners| {
            let half = (b.width / 2.0).min(b.height / 2.0).max(0.0);
            [r.top_left, r.top_right, r.bottom_right, r.bottom_left].map(|v| v.clamp(0.0, half))
        };
        let p = Self::rounded_rect_builder(Self::builder(), outer, clamp(outer, outer_radius));
        if inner.is_empty() {
            return p;
        }
        Self::rounded_rect_builder(p, inner, clamp(inner, inner_radius)).fill_rule(FillRule::EvenOdd)
    }

    #[must_use]
    pub fn verbs(&self) -> &[Verb] {
        &self.0.verbs
    }

    #[must_use]
    pub fn points(&self) -> &[[f32; 2]] {
        &self.0.points
    }

    /// Min x, min y, max x, max y of every point; every curve lies inside.
    #[must_use]
    pub fn bounds(&self) -> [f32; 4] {
        self.0.bounds
    }

    #[must_use]
    pub fn fill_rule(&self) -> FillRule {
        self.0.fill_rule
    }

    /// A hash of the content: equal paths have equal keys.
    #[must_use]
    pub fn key(&self) -> u64 {
        self.0.key
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.points.is_empty()
    }

    /// The path as subpaths of curves.
    #[must_use]
    pub fn subpaths(&self) -> Vec<Subpath> {
        let mut out: Vec<Subpath> = Vec::new();
        let mut points = self.0.points.iter().copied();
        let mut current: Option<Subpath> = None;
        let mut pen = [0.0_f32; 2];
        for verb in &self.0.verbs {
            match verb {
                Verb::Move => {
                    out.extend(current.take());
                    pen = points.next().unwrap_or(pen);
                    current = Some(Subpath {
                        curves: Vec::new(),
                        closed: false,
                        start: pen,
                    });
                }
                Verb::Line | Verb::Quad | Verb::Cubic => {
                    let n = match verb {
                        Verb::Line => 1,
                        Verb::Quad => 2,
                        _ => 3,
                    };
                    let mut pts = vec![pen];
                    pts.extend(points.by_ref().take(n));
                    pen = pts.last().copied().unwrap_or(pen);
                    let sub = current.get_or_insert_with(|| Subpath {
                        curves: Vec::new(),
                        closed: false,
                        start: pts.first().copied().unwrap_or_default(),
                    });
                    sub.curves.push(Curve::new(&pts));
                }
                Verb::Close => {
                    if let Some(mut sub) = current.take() {
                        if sub.start != pen {
                            sub.curves.push(Curve::new(&[pen, sub.start]));
                        }
                        sub.closed = true;
                        pen = sub.start;
                        out.push(sub);
                    }
                }
            }
        }
        out.extend(current);
        out
    }

    /// The path cut into dashes along its length: `array` alternates on and
    /// off lengths (repeated twice when odd, as SVG does), starting
    /// `offset` into the pattern. Done once in the path's own space, where a
    /// dash pattern is defined: a zoom scales the dashes with the path.
    #[must_use]
    pub fn dashed(&self, dash: &Dash) -> Self {
        let mut array: Vec<f32> = dash.array.iter().map(|v| v.max(0.0)).collect();
        if array.len() % 2 == 1 {
            array.extend_from_within(..);
        }
        let period: f32 = array.iter().sum();
        if array.is_empty() || period <= 0.0 {
            return self.clone();
        }
        let mut b = Self::builder().fill_rule(self.fill_rule());
        for sub in self.subpaths() {
            // Where in the pattern the subpath starts.
            let mut phase = dash.offset.rem_euclid(period);
            let mut index = 0;
            while phase >= array.get(index).copied().unwrap_or(f32::MAX) {
                phase -= array.get(index).copied().unwrap_or(0.0);
                index = index.saturating_add(1).checked_rem(array.len()).unwrap_or(0);
            }
            let mut left = array.get(index).copied().unwrap_or(0.0) - phase;
            let mut on = index % 2 == 0;
            let mut open = false;
            for c in &sub.curves {
                // The curve as many short chords, their lengths to find t by length.
                let steps = 64_u16;
                let mut t0 = 0.0_f32;
                let mut prev = c.start();
                for i in 1..=steps {
                    let t1 = f32::from(i) / f32::from(steps);
                    let p = c.eval(t1);
                    let mut len = (p[0] - prev[0]).hypot(p[1] - prev[1]);
                    let mut t = t0;
                    while len > 0.0 {
                        let take = len.min(left);
                        let t_end = if len > 0.0 { t + (t1 - t) * (take / len) } else { t1 };
                        if on {
                            let piece = c.segment(t, t_end);
                            if !open {
                                let s = piece.start();
                                b = b.move_to(s[0], s[1]);
                                open = true;
                            }
                            b = b.curve(&piece);
                        }
                        left -= take;
                        len -= take;
                        t = t_end;
                        if left <= 0.0 {
                            index = index.saturating_add(1).checked_rem(array.len()).unwrap_or(0);
                            left = array.get(index).copied().unwrap_or(0.0);
                            on = !on;
                            open = false;
                        }
                    }
                    t0 = t1;
                    prev = p;
                }
            }
        }
        b.build()
    }
}

/// Builds a [`Path`]; [`PathBuilder::fill`] and [`PathBuilder::stroke`]
/// finish it as a [`Shape`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PathBuilder {
    verbs: Vec<Verb>,
    points: Vec<[f32; 2]>,
    fill_rule: FillRule,
    open: bool,
    /// The current subpath's start, where a close takes the pen.
    start: [f32; 2],
}

impl PathBuilder {
    #[must_use]
    pub fn move_to(mut self, x: f32, y: f32) -> Self {
        self.verbs.push(Verb::Move);
        self.points.push([x, y]);
        self.open = true;
        self.start = [x, y];
        self
    }

    fn ensure_open(&mut self) {
        if !self.open {
            // After a close the pen is back at the subpath's start.
            let at = if self.verbs.last() == Some(&Verb::Close) {
                self.start
            } else {
                self.points.last().copied().unwrap_or_default()
            };
            self.verbs.push(Verb::Move);
            self.points.push(at);
            self.open = true;
            self.start = at;
        }
    }

    #[must_use]
    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        self.ensure_open();
        self.verbs.push(Verb::Line);
        self.points.push([x, y]);
        self
    }

    #[must_use]
    pub fn quad_to(mut self, cx: f32, cy: f32, x: f32, y: f32) -> Self {
        self.ensure_open();
        self.verbs.push(Verb::Quad);
        self.points.extend([[cx, cy], [x, y]]);
        self
    }

    #[must_use]
    pub fn cubic_to(mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> Self {
        self.ensure_open();
        self.verbs.push(Verb::Cubic);
        self.points.extend([[c1x, c1y], [c2x, c2y], [x, y]]);
        self
    }

    /// `c` from the pen (its first point is taken to be the pen).
    #[must_use]
    pub fn curve(self, c: &Curve) -> Self {
        let p = c.points;
        match c.order {
            2 => self.line_to(p[1][0], p[1][1]),
            3 => self.quad_to(p[1][0], p[1][1], p[2][0], p[2][1]),
            _ => self.cubic_to(p[1][0], p[1][1], p[2][0], p[2][1], p[3][0], p[3][1]),
        }
    }

    #[must_use]
    pub fn close(mut self) -> Self {
        if self.open {
            self.verbs.push(Verb::Close);
            self.open = false;
        }
        self
    }

    #[must_use]
    pub fn fill_rule(mut self, rule: FillRule) -> Self {
        self.fill_rule = rule;
        self
    }

    /// The path, its key and bounds computed.
    #[must_use]
    pub fn build(self) -> Path {
        let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in &self.points {
            bounds = [
                bounds[0].min(p[0]),
                bounds[1].min(p[1]),
                bounds[2].max(p[0]),
                bounds[3].max(p[1]),
            ];
        }
        if self.points.is_empty() {
            bounds = [0.0; 4];
        }
        let mut h = foldhash::fast::FixedState::with_seed(0x9e37_79b9).build_hasher();
        self.verbs.hash(&mut h);
        self.fill_rule.hash(&mut h);
        for p in &self.points {
            p[0].to_bits().hash(&mut h);
            p[1].to_bits().hash(&mut h);
        }
        Path(Arc::new(PathData {
            verbs: self.verbs,
            points: self.points,
            bounds,
            fill_rule: self.fill_rule,
            key: h.finish(),
        }))
    }

    /// The path filled with `color`.
    #[must_use]
    pub fn fill(self, color: Color) -> Shape {
        Shape {
            path: self.build(),
            style: PathStyle::fill(Gradient::Solid(color)),
        }
    }

    /// The path's outline in `color`, `width` wide, with round caps and
    /// joins (as Lucide's icons and the charts draw theirs).
    #[must_use]
    pub fn stroke(self, color: Color, width: f32) -> Shape {
        Shape {
            path: self.build(),
            style: PathStyle::stroke(Gradient::Solid(color), Stroke::round(width)),
        }
    }
}

/// How an open stroke ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

/// How a stroke turns a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Join {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// A dash pattern: on and off lengths, and where in it the path starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Dash {
    pub array: Vec<f32>,
    pub offset: f32,
}

/// How a path's outline is drawn, as SVG's `stroke-*` properties.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub width: f32,
    pub cap: Cap,
    pub join: Join,
    pub miter_limit: f32,
    pub dash: Option<Dash>,
}

impl Stroke {
    /// SVG's defaults: butt caps, miter joins up to 4.
    #[must_use]
    pub fn new(width: f32) -> Self {
        Self {
            width,
            cap: Cap::Butt,
            join: Join::Miter,
            miter_limit: 4.0,
            dash: None,
        }
    }

    /// Round caps and joins.
    #[must_use]
    pub fn round(width: f32) -> Self {
        Self {
            cap: Cap::Round,
            join: Join::Round,
            ..Self::new(width)
        }
    }

    #[must_use]
    pub fn cap(mut self, cap: Cap) -> Self {
        self.cap = cap;
        self
    }

    #[must_use]
    pub fn join(mut self, join: Join) -> Self {
        self.join = join;
        self
    }

    #[must_use]
    pub fn miter_limit(mut self, limit: f32) -> Self {
        self.miter_limit = limit;
        self
    }

    #[must_use]
    pub fn dash(mut self, array: Vec<f32>, offset: f32) -> Self {
        self.dash = Some(Dash { array, offset });
        self
    }
}

/// Filled, or stroked along its outline.
#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    Fill,
    Stroke(Stroke),
}

/// What a path is painted with and how.
#[derive(Clone, Debug, PartialEq)]
pub struct PathStyle {
    pub paint: Gradient,
    pub draw: Draw,
}

impl PathStyle {
    #[must_use]
    pub fn fill(paint: Gradient) -> Self {
        Self {
            paint,
            draw: Draw::Fill,
        }
    }

    #[must_use]
    pub fn stroke(paint: Gradient, stroke: Stroke) -> Self {
        Self {
            paint,
            draw: Draw::Stroke(stroke),
        }
    }
}

/// A path and how it is painted: what an element of shapes draws.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub path: Path,
    pub style: PathStyle,
}

impl Shape {
    #[must_use]
    pub fn new(path: Path, style: PathStyle) -> Self {
        Self { path, style }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_content_is_one_path() {
        let a = Path::circle(10.0, 10.0, 4.0).build();
        let b = Path::circle(10.0, 10.0, 4.0).build();
        assert_eq!(a, b);
        assert_eq!(a.key(), b.key());
        assert_ne!(a.key(), Path::circle(10.0, 10.0, 5.0).build().key());
    }

    #[test]
    fn a_close_adds_the_closing_line() {
        let p = Path::builder()
            .move_to(0.0, 0.0)
            .line_to(4.0, 0.0)
            .line_to(4.0, 4.0)
            .close()
            .build();
        let subs = p.subpaths();
        assert_eq!(subs.len(), 1);
        let s = subs.first().expect("a subpath");
        assert!(s.closed);
        assert_eq!(s.curves.len(), 3);
        assert_eq!(s.curves.last().map(Curve::end), Some([0.0, 0.0]));
    }

    #[test]
    fn dashes_cut_by_length() {
        let p = Path::builder().move_to(0.0, 0.0).line_to(10.0, 0.0).build();
        let d = p.dashed(&Dash {
            array: vec![2.0, 3.0],
            offset: 0.0,
        });
        let subs = d.subpaths();
        // On 0–2, 5–7: two dashes.
        assert_eq!(subs.len(), 2, "{subs:?}");
        let ends: Vec<[f32; 2]> = subs
            .iter()
            .map(|s| [s.start[0], s.curves.last().map_or(0.0, |c| c.end()[0])])
            .collect();
        for (got, want) in ends.iter().zip([[0.0, 2.0], [5.0, 7.0]]) {
            assert!(
                (got[0] - want[0]).abs() < 1e-3 && (got[1] - want[1]).abs() < 1e-3,
                "{ends:?}"
            );
        }
    }
}
