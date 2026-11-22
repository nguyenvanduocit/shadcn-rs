//! What a view of a world draws (ADR-0017), worked out on the CPU alone:
//! the objects meeting it in draw order, each at its level of detail (not
//! drawn, a box of its colour, a path simplified for its level, a path cut
//! about the view when it is huge, text as curves or a grey bar), runs of
//! boxes gathered into one draw. A canvas drawn straight plans its view on
//! the render thread each frame; a tile is planned once on a raster worker
//! ([`super::planner`]) and its steps rasterized on the render thread within
//! a frame's budget (`gpu::world`).

use std::sync::Arc;

use super::{Aabb, Body, LOD_BOX, LOD_DROP, Placed, REANCHOR, WorldSnapshot, turn};
use crate::color::Color;
use crate::shape::{Path, PathStyle};
use crate::text::GlyphKey;

/// Device pixels an em under which text is drawn as a grey bar.
pub const LOD_TEXT: f64 = 3.0;
/// Curve pixels per em of a glyph's outline (`text::CURVE_EM`).
const CURVE_EM: f64 = 1024.0;

/// A view of a world onto a target's device pixels: world point `center`
/// lands at `origin`, `k` device pixels a unit, turned by `cs` (cosine,
/// sine); `cut` is the target's rectangle drawn (x0, y0, x1, y1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub center: (f64, f64),
    pub k: f64,
    pub cs: (f64, f64),
    pub origin: (f64, f64),
    pub cut: [f32; 4],
}

impl View {
    /// World point `w` in the target's device pixels.
    #[must_use]
    pub fn to_dev(self, w: (f64, f64)) -> (f64, f64) {
        let d = turn(
            ((w.0 - self.center.0) * self.k, (w.1 - self.center.1) * self.k),
            self.cs,
        );
        (self.origin.0 + d.0, self.origin.1 + d.1)
    }

    /// Device point `d` in the world.
    #[must_use]
    pub fn to_world(self, d: (f64, f64)) -> (f64, f64) {
        let w = turn((d.0 - self.origin.0, d.1 - self.origin.1), (self.cs.0, -self.cs.1));
        (self.center.0 + w.0 / self.k, self.center.1 + w.1 / self.k)
    }

    /// The world the cut shows: the box around its corners.
    #[must_use]
    pub fn area(&self) -> Aabb {
        let [x0, y0, x1, y1] = self.cut.map(f64::from);
        [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
            .map(|p| self.to_world(p))
            .iter()
            .fold(
                [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
                |b, p| [b[0].min(p.0), b[1].min(p.1), b[2].max(p.0), b[3].max(p.1)],
            )
    }

    /// The matrix from coordinates about `anchor` (world units) to the
    /// target: computed in `f64`, holding only small numbers as `f32`.
    #[must_use]
    pub fn matrix(&self, anchor: (f64, f64)) -> [f32; 6] {
        let p = self.to_dev(anchor);
        let (c, s) = (self.cs.0 * self.k, self.cs.1 * self.k);
        [c, s, -s, c, p.0, p.1].map(crate::convert::f64_to_f32)
    }

    /// The device box `b` (world) covers.
    #[must_use]
    pub fn device_box(&self, b: &Aabb) -> [f32; 4] {
        let corners = [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])].map(|p| self.to_dev(p));
        let r = corners.iter().fold(
            [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
            |r, p| [r[0].min(p.0), r[1].min(p.1), r[2].max(p.0), r[3].max(p.1)],
        );
        r.map(crate::convert::f64_to_f32)
    }

    /// World box `b` as a box for the boxes draw: its area's own size about
    /// its centre (exact when the view is not turned; a turned box under
    /// [`LOD_BOX`] pixels keeps its area and place).
    fn small_box(&self, b: &Aabb) -> [f32; 4] {
        let c = self.to_dev(((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0));
        let (w, h) = ((b[2] - b[0]) * self.k, (b[3] - b[1]) * self.k);
        [c.0 - w / 2.0, c.1 - h / 2.0, w, h].map(crate::convert::f64_to_f32)
    }
}

/// One thing drawn, in order.
#[derive(Clone, Debug)]
pub enum Step {
    /// Boxes ([`super::boxes`]'s floats, the target's device pixels).
    Boxes(Vec<f32>),
    /// `path` (the object's, or simplified, or cut about the view) through `m`.
    Path {
        object: Arc<Placed>,
        path: Path,
        m: [f32; 6],
    },
    /// A glyph's curves through `m` (curve pixels, y up, to the target).
    Glyph { key: GlyphKey, m: [f32; 6], color: Color },
}

impl Step {
    /// What it costs to rasterize, in eighths of a small path's job: the
    /// unit a frame's raster budget is counted in.
    #[must_use]
    pub fn cost(&self) -> u32 {
        match self {
            Self::Boxes(b) => 1_u32.saturating_add(u32::try_from(b.len() / super::boxes::BOX / 64).unwrap_or(u32::MAX)),
            Self::Path { path, .. } => 8_u32.saturating_add(u32::try_from(path.verbs().len() / 2).unwrap_or(u32::MAX)),
            Self::Glyph { .. } => 2,
        }
    }

    /// The style a path step is painted with.
    #[must_use]
    pub fn style(&self) -> Option<&PathStyle> {
        match self {
            Self::Path { object, .. } => object.as_path().map(|p| &p.style),
            _ => None,
        }
    }
}

/// A run of small objects drawn as boxes: one draw while no other object
/// comes between them where they lie. Objects too small to draw (under
/// [`LOD_DROP`]) are not lost: their area and colour gather per device
/// pixel into the run's dust, a box a pixel of their average colour and
/// summed coverage (Datashader's aggregation), so a dense far-out view
/// keeps its tone instead of vanishing.
struct Run {
    cut: [f32; 4],
    data: Vec<f32>,
    /// Per device pixel (x, y) of the dust: the colour sums weighted by
    /// covered area times alpha, and that weight.
    dust: foldhash::HashMap<(i32, i32), [f32; 4]>,
    /// Which of 64 × 64 cells over `cut` the run's boxes and dust touch: an
    /// object drawn meanwhile that touches none of them leaves the run
    /// going (it and the boxes do not overlap, so their order shows nowhere).
    cells: [u64; 64],
}

impl Run {
    fn new(cut: [f32; 4]) -> Self {
        Self {
            cut,
            data: Vec::new(),
            dust: foldhash::HashMap::default(),
            cells: [0; 64],
        }
    }

    /// The cells rectangle `r` (x0, y0, x1, y1) touches: its rows and a mask
    /// of its columns.
    fn span(&self, r: [f32; 4]) -> Option<(std::ops::RangeInclusive<usize>, u64)> {
        let (w, h) = ((self.cut[2] - self.cut[0]) / 64.0, (self.cut[3] - self.cut[1]) / 64.0);
        if !(w > 0.0 && h > 0.0) || r[2] < self.cut[0] || r[3] < self.cut[1] || r[0] > self.cut[2] || r[1] > self.cut[3]
        {
            return None;
        }
        let cell = |v: f32, from: f32, side: f32| crate::convert::f32_to_u32(((v - from) / side).clamp(0.0, 63.0));
        let (c0, c1) = (cell(r[0], self.cut[0], w), cell(r[2], self.cut[0], w));
        let (r0, r1) = (cell(r[1], self.cut[1], h), cell(r[3], self.cut[1], h));
        let cols = (u64::MAX >> 63_u32.saturating_sub(c1.saturating_sub(c0))) << c0;
        let rows = usize::try_from(r0).unwrap_or(0)..=usize::try_from(r1).unwrap_or(0);
        Some((rows, cols))
    }

    /// Whether something over `r` (x0, y0, x1, y1) meets the run's boxes.
    fn meets(&self, r: [f32; 4]) -> bool {
        self.span(r)
            .is_some_and(|(rows, cols)| self.cells.get(rows).is_some_and(|c| c.iter().any(|m| m & cols != 0)))
    }

    /// Rectangle `r` (x, y, width, height) cut to the target (x0, y0, x1,
    /// y1), marked in the cells; `None` when nothing of it is left.
    fn cut_mark(&mut self, r: [f32; 4]) -> Option<[f32; 4]> {
        let x0 = r[0].max(self.cut[0]);
        let y0 = r[1].max(self.cut[1]);
        let x1 = (r[0] + r[2]).min(self.cut[2]);
        let y1 = (r[1] + r[3]).min(self.cut[3]);
        if !(x1 > x0 && y1 > y0) {
            return None;
        }
        if let Some((rows, cols)) = self.span([x0, y0, x1, y1]) {
            for c in self.cells.get_mut(rows).into_iter().flatten() {
                *c |= cols;
            }
        }
        Some([x0, y0, x1, y1])
    }

    /// Box `r` (x, y, width, height), cut to the target: an axis-aligned cut is exact.
    fn push(&mut self, r: [f32; 4], color: Color) {
        if let Some([x0, y0, x1, y1]) = self.cut_mark(r) {
            super::boxes::push(&mut self.data, [x0, y0, x1 - x0, y1 - y0], color);
        }
    }

    /// An object too small to draw over `r` (x, y, width, height): its
    /// area in each pixel it covers, weighted by its alpha, into the dust.
    fn dust(&mut self, r: [f32; 4], color: Color) {
        let Some([x0, y0, x1, y1]) = self.cut_mark(r) else {
            return;
        };
        // The pixels it covers: from the one holding its start to the one
        // before its end's ceiling.
        let lo = |v: f32| crate::convert::f64_to_i32(f64::from(v).floor());
        let hi = |v: f32| crate::convert::f64_to_i32(f64::from(v).ceil()).saturating_sub(1);
        for y in lo(y0)..=hi(y1) {
            for x in lo(x0)..=hi(x1) {
                let (fx, fy) = (crate::convert::i32_to_f32(x), crate::convert::i32_to_f32(y));
                let w = (x1.min(fx + 1.0) - x0.max(fx)).max(0.0);
                let h = (y1.min(fy + 1.0) - y0.max(fy)).max(0.0);
                let a = w * h * color.a;
                if a > 0.0 {
                    let cell = self.dust.entry((x, y)).or_insert([0.0; 4]);
                    cell[0] += color.r * a;
                    cell[1] += color.g * a;
                    cell[2] += color.b * a;
                    cell[3] += a;
                }
            }
        }
    }

    fn flush(&mut self, out: &mut Vec<Step>) {
        if !self.dust.is_empty() {
            // In pixel order, so a plan is the same every time it is made.
            let mut cells: Vec<((i32, i32), [f32; 4])> = self.dust.drain().collect();
            cells.sort_unstable_by_key(|(at, _)| (at.1, at.0));
            for ((x, y), [r, g, b, a]) in cells {
                let color = Color {
                    r: r / a,
                    g: g / a,
                    b: b / a,
                    a: a.min(1.0),
                };
                let (fx, fy) = (crate::convert::i32_to_f32(x), crate::convert::i32_to_f32(y));
                super::boxes::push(&mut self.data, [fx, fy, 1.0, 1.0], color);
            }
        }
        if !self.data.is_empty() {
            out.push(Step::Boxes(std::mem::take(&mut self.data)));
        }
        self.cells = [0; 64];
    }
}

/// How far a stroke reaches past its points (world units).
#[must_use]
pub fn stroke_reach(style: &PathStyle) -> f64 {
    match &style.draw {
        crate::shape::Draw::Stroke(s) => f64::from(s.width) * 0.5 * f64::from(s.miter_limit.max(1.0)),
        crate::shape::Draw::Fill => 0.0,
    }
}

/// Huge object `o` drawn from its source cut to around `view`, about
/// `anchor` (near the view): the path and the anchor.
#[must_use]
pub fn cut_about(o: &Placed, view: &View, anchor: (f64, f64)) -> Option<(Path, (f64, f64))> {
    let body = o.as_path()?;
    let area = view.area();
    let side = (area[2] - area[0]).max(area[3] - area[1]);
    let reach = stroke_reach(&body.style) + side / 8.0;
    let near = [area[0] - reach, area[1] - reach, area[2] + reach, area[3] + reach];
    let k = view.k.max(f64::MIN_POSITIVE);
    Some((
        body.source
            .near(anchor, near, 0.05 / k, stroke_reach(&body.style) > 0.0),
        anchor,
    ))
}

/// How a huge object is drawn: its path cut about the view and the anchor
/// it is cut about, or `None` to draw it whole.
pub type Huge<'a> = dyn FnMut(u32, &Arc<Placed>) -> Option<(Path, (f64, f64))> + 'a;

/// The steps drawing what tiles show of `world` meeting `view` (the
/// lifted objects left out), in draw order, into `out`; `huge` gives the
/// path and anchor of an object larger on screen than [`REANCHOR`] (cut
/// about the view, cached where that pays).
pub fn plan(world: &WorldSnapshot, view: &View, huge: &mut Huge<'_>, out: &mut Vec<Step>) {
    let visible = world.visible(&view.area());
    let objects = visible.iter().filter_map(|i| Some((*i, world.object(*i)?)));
    plan_objects(objects, view, huge, out);
}

/// The steps drawing everything of `world` meeting `view` as it is now:
/// the lifted objects too, each in its place in the order.
pub fn plan_live(world: &WorldSnapshot, view: &View, huge: &mut Huge<'_>, out: &mut Vec<Step>) {
    plan_objects(world.drawn(&view.area()).into_iter(), view, huge, out);
}

fn plan_objects<'a>(
    objects: impl Iterator<Item = (u32, &'a Arc<Placed>)>,
    view: &View,
    huge: &mut Huge<'_>,
    out: &mut Vec<Step>,
) {
    let mut run = Run::new(view.cut);
    for (i, o) in objects {
        let extent = (o.bbox[2] - o.bbox[0]).max(o.bbox[3] - o.bbox[1]) * view.k;
        match &o.body {
            Body::Path(body) => {
                if extent < LOD_DROP {
                    run.dust(view.small_box(&o.bbox), o.color);
                    continue;
                }
                if extent < LOD_BOX {
                    run.push(view.small_box(&o.bbox), o.color);
                    continue;
                }
                if run.meets(view.device_box(&o.bbox)) {
                    run.flush(out);
                }
                let (path, anchor) = if extent > REANCHOR {
                    match huge(i, o) {
                        Some(p) => p,
                        None => continue,
                    }
                } else {
                    (body.at_level(view.k, o.anchor), o.anchor)
                };
                out.push(Step::Path {
                    object: Arc::clone(o),
                    path,
                    m: view.matrix(anchor),
                });
            }
            Body::Text(t) => {
                let Some(shaped) = t.shaped.get() else { continue };
                let em = t.size * view.k;
                if em < LOD_TEXT || extent < LOD_DROP {
                    // A grey bar where each line's letters stand.
                    let (x, y) = o.anchor;
                    let whole = [[0.0, 0.0, shaped.width, shaped.height]];
                    let lines = if shaped.lines.is_empty() {
                        &whole[..]
                    } else {
                        &shaped.lines[..]
                    };
                    let color = t.style.color.opacity(0.5);
                    for l in lines {
                        let h = l[3] - l[1];
                        let bar = [
                            x + t.size * l[0],
                            y + t.size * (l[1] + h * 0.3),
                            x + t.size * l[2],
                            y + t.size * (l[1] + h * 0.7),
                        ];
                        let r = view.small_box(&bar);
                        if extent < LOD_DROP {
                            run.dust(r, color);
                        } else {
                            run.push(r, color);
                        }
                    }
                    continue;
                }
                if run.meets(view.device_box(&o.bbox)) {
                    run.flush(out);
                }
                let kk = em / CURVE_EM;
                let (c, s) = (view.cs.0 * kk, view.cs.1 * kk);
                for (key, pen, color) in &shaped.glyphs {
                    let at = view.to_dev((o.anchor.0 + pen.0 * t.size, o.anchor.1 + pen.1 * t.size));
                    out.push(Step::Glyph {
                        key: *key,
                        m: [c, s, s, -c, at.0, at.1].map(crate::convert::f64_to_f32),
                        color: *color,
                    });
                }
            }
        }
    }
    run.flush(out);
}
