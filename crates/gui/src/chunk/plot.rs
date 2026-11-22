//! Plots (ADR-0018): series of points owned by a [`crate::Scope`], and an
//! element that draws marks of them as GPU data chunks.
//!
//! ```ignore
//! let s = series();
//! s.append(&xs, &ys);
//! plot().x(0, 1_000).y_auto().mark(line(s).color(c)).build().width(pct(100.0)).height(256.0)
//! ```
//!
//! A [`Series`] is a `Copy` handle as a [`Signal`] is: appending to it
//! uploads only the new points and records only the plots that read it.
//! A plot's view (`x`, `y`) is a value or a closure bound to signals; its
//! y can follow the visible points ([`YAuto`], O(log n) through the
//! series' min/max tree). Hovering a plot finds the point under the
//! pointer on the CPU from the same tree, in the frame the pointer moved.

use std::hash::{BuildHasher, Hash, Hasher};
use std::rc::Rc;

use super::series::{SeriesSnapshot, SeriesState};
use super::{
    BarParams, Bars, CandleParams, Candles, ChunkKind, DataItem, Drawing, GridSnapshot, Heatmap, HeatmapParams, KindId,
    LineCurve, LineDash, LineJoin, LineParams, Lines, MarkView, Picture, Scatter, ScatterParams, Source, Words,
};
use crate::color::Color;
use crate::element::{Drawn, drawing};
use crate::geometry::Bounds;
use crate::reactive::{Signal, untracked};
use crate::scene::Item;

/// A series of points: x (whole units of the caller's choosing: indices,
/// milliseconds, nanoseconds) never going back, y a float; a y that is not
/// finite is a gap. Owned by the scope it was made in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Series(Signal<SeriesState>);

/// A series keeping every point it is given.
#[must_use]
pub fn series() -> Series {
    Series(Signal::new(SeriesState::new(None, false)))
}

impl Series {
    /// A series keeping about the last `n` points: older ones go a whole
    /// page at a time, so a stream costs only what it appends.
    #[must_use]
    pub fn keeping(n: u64) -> Self {
        Self(Signal::new(SeriesState::new(Some(n), false)))
    }

    /// A series holding y in two floats (its high and low parts), for
    /// values whose f32 rounding shows (a price of 4 000.0001).
    #[must_use]
    pub fn precise() -> Self {
        Self(Signal::new(SeriesState::new(None, true)))
    }

    /// Append points (`xs` and `ys` pairwise).
    pub fn append(&self, xs: &[i64], ys: &[f64]) {
        if xs.is_empty() || ys.is_empty() {
            return;
        }
        self.0.update(|s| s.append(xs, ys));
    }

    /// Append one point.
    pub fn push(&self, x: i64, y: f64) {
        self.append(&[x], &[y]);
    }

    /// Replace every point.
    pub fn set(&self, xs: &[i64], ys: &[f64]) {
        self.0.update(|s| s.set(xs, ys));
    }

    /// What it holds now, tracked: a plot or binding that reads it runs
    /// again when it changes.
    #[must_use]
    pub fn snapshot(&self) -> SeriesSnapshot {
        self.0.with(SeriesState::snapshot)
    }

    /// What it holds now, untracked.
    #[must_use]
    pub fn snapshot_untracked(&self) -> SeriesSnapshot {
        self.0.with_untracked(SeriesState::snapshot)
    }

    /// Points it holds.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.0.with(SeriesState::len)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A grid of values for a heatmap, row by row from the top.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Grid(Signal<GridSnapshot>);

/// A grid of `width` by `height` values.
#[must_use]
pub fn grid(width: u32, height: u32, values: Vec<f32>) -> Grid {
    Grid(Signal::new(GridSnapshot::new(width, height, values)))
}

impl Grid {
    /// Replace its values.
    pub fn set(&self, width: u32, height: u32, values: Vec<f32>) {
        self.0.set(GridSnapshot::new(width, height, values));
    }

    #[must_use]
    pub fn snapshot(&self) -> GridSnapshot {
        self.0.get()
    }
}

/// How a plot's y follows what it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YAuto {
    /// Reach 0 too (bars, areas).
    pub zero: bool,
    /// Room above and below, as a share of the span.
    pub pad: f64,
}

impl Default for YAuto {
    fn default() -> Self {
        Self { zero: false, pad: 0.05 }
    }
}

type XFn = Rc<dyn Fn() -> (i64, i64)>;
type YFn = Rc<dyn Fn() -> (f64, f64)>;

#[derive(Clone)]
enum XRange {
    /// From the first point to the last of the first mark's series.
    Data,
    Fixed(i64, i64),
    Bound(XFn),
}

#[derive(Clone)]
enum YRange {
    Auto(YAuto),
    Fixed(f64, f64),
    Bound(YFn),
}

#[derive(Clone, Debug, PartialEq)]
enum MarkKind {
    Line(LineParams),
    Bars(BarParams),
    Candles(CandleParams),
    Scatter(ScatterParams),
    Heatmap(HeatmapParams),
    Rule(RuleParams),
}

/// Reference lines across the plot at data values (Observable Plot's
/// `ruleX`, `ruleY`), drawn as stroked paths, not data chunks.
#[derive(Clone, Debug, PartialEq)]
struct RuleParams {
    at: RuleAt,
    color: Color,
    width: f32,
    dash: Option<(Vec<f32>, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
enum RuleAt {
    /// Vertical lines at these x.
    X(Vec<i64>),
    /// Horizontal lines at these y.
    Y(Vec<f64>),
}

/// One mark of a plot: what it draws and of which data.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    kind: MarkKind,
    series: Vec<Series>,
    grid: Option<Grid>,
    floor: Option<Series>,
}

fn view_zero() -> MarkView {
    MarkView {
        x0: 0,
        x1: 1,
        y0: 0.0,
        y1: 1.0,
        base: None,
    }
}

/// A line through `s`'s points: 2 px wide, mitred, straight.
#[must_use]
pub fn line(s: Series) -> Mark {
    Mark {
        kind: MarkKind::Line(LineParams {
            view: view_zero(),
            color: Color::BLACK,
            width: 2.0,
            area: None,
            join: LineJoin::Miter,
            miter_limit: 4.0,
            curve: LineCurve::Linear,
            floor: false,
            dash: None,
        }),
        series: vec![s],
        grid: None,
        floor: None,
    }
}

/// The area under `s`'s points down to the plot's base (or a series below,
/// [`Mark::on`]), with no line until [`Mark::stroke`] gives one.
#[must_use]
pub fn area(s: Series) -> Mark {
    let mut m = line(s);
    if let MarkKind::Line(p) = &mut m.kind {
        p.width = 0.0;
        p.area = Some((Color::BLACK.opacity(0.4), Color::BLACK.opacity(0.4)));
    }
    m
}

/// A bar from each of `s`'s points down to the base: 8 px wide, centred.
#[must_use]
pub fn bars(s: Series) -> Mark {
    Mark {
        kind: MarkKind::Bars(BarParams {
            view: view_zero(),
            color: Color::BLACK,
            width: 8.0,
            offset: -4.0,
            radius: 0.0,
            floor: false,
        }),
        series: vec![s],
        grid: None,
        floor: None,
    }
}

/// Candles of four series sharing x: open, high, low and close.
#[must_use]
pub fn candles(open: Series, high: Series, low: Series, close: Series) -> Mark {
    Mark {
        kind: MarkKind::Candles(CandleParams {
            view: view_zero(),
            width: 7.0,
            wick: 1.0,
            up: Color::hex(0x0016_a34a),
            down: Color::hex(0x00dc_2626),
        }),
        series: vec![open, high, low, close],
        grid: None,
        floor: None,
    }
}

/// A dot at each of `s`'s points, 2 px in radius.
#[must_use]
pub fn scatter(s: Series) -> Mark {
    Mark {
        kind: MarkKind::Scatter(ScatterParams {
            view: view_zero(),
            color: Color::BLACK,
            radius: 2.0,
        }),
        series: vec![s],
        grid: None,
        floor: None,
    }
}

/// Vertical lines across the plot at each of `xs`: 1 px wide, black.
#[must_use]
pub fn rule_x(xs: &[i64]) -> Mark {
    rule(RuleAt::X(xs.to_vec()))
}

/// Horizontal lines across the plot at each of `ys` (an auto y reaches
/// them): 1 px wide, black.
#[must_use]
pub fn rule_y(ys: &[f64]) -> Mark {
    rule(RuleAt::Y(ys.to_vec()))
}

fn rule(at: RuleAt) -> Mark {
    Mark {
        kind: MarkKind::Rule(RuleParams {
            at,
            color: Color::BLACK,
            width: 1.0,
            dash: None,
        }),
        series: Vec::new(),
        grid: None,
        floor: None,
    }
}

/// `g`'s values over the plot's box, from `low` at 0 to `high` at 1.
#[must_use]
pub fn heatmap(g: Grid) -> Mark {
    Mark {
        kind: MarkKind::Heatmap(HeatmapParams {
            low: Color::WHITE,
            high: Color::BLACK,
            min: 0.0,
            max: 1.0,
        }),
        series: Vec::new(),
        grid: Some(g),
        floor: None,
    }
}

impl Mark {
    /// Its colour: a line's, an area's (flat), bars', dots'.
    #[must_use]
    pub fn color(mut self, c: Color) -> Self {
        match &mut self.kind {
            MarkKind::Line(p) => {
                if p.width > 0.0 || p.area.is_none() {
                    p.color = c;
                }
                if let Some(a) = &mut p.area
                    && p.width <= 0.0
                {
                    *a = (c, c);
                }
            }
            MarkKind::Bars(p) => p.color = c,
            MarkKind::Scatter(p) => p.color = c,
            MarkKind::Rule(p) => p.color = c,
            MarkKind::Candles(_) | MarkKind::Heatmap(_) => {}
        }
        self
    }

    /// A line's width; a bar's; a candle body's (logical pixels).
    #[must_use]
    pub fn width(mut self, w: f32) -> Self {
        match &mut self.kind {
            MarkKind::Line(p) => p.width = w,
            MarkKind::Bars(p) => {
                p.width = w;
                p.offset = -w / 2.0;
            }
            MarkKind::Candles(p) => p.width = w,
            MarkKind::Rule(p) => p.width = w,
            MarkKind::Scatter(_) | MarkKind::Heatmap(_) => {}
        }
        self
    }

    /// An area's fill, `top` at the plot's top fading to `bottom` at its bottom.
    #[must_use]
    pub fn fill(mut self, top: Color, bottom: Color) -> Self {
        if let MarkKind::Line(p) = &mut self.kind {
            p.area = Some((top, bottom));
        }
        self
    }

    /// An area's edge: a line of `color` and `width` along its top.
    #[must_use]
    pub fn stroke(mut self, color: Color, width: f32) -> Self {
        if let MarkKind::Line(p) = &mut self.kind {
            p.color = color;
            p.width = width;
        }
        self
    }

    /// d3's natural cubic spline through the points (`curveNatural`).
    #[must_use]
    pub fn natural(mut self) -> Self {
        if let MarkKind::Line(p) = &mut self.kind {
            p.curve = LineCurve::Natural;
        }
        self
    }

    /// A line dashed by `array` (SVG's `stroke-dasharray`, logical pixels)
    /// from `offset` into it; an empty or all-zero pattern draws it solid.
    #[must_use]
    pub fn dash(mut self, array: &[f32], offset: f32) -> Self {
        match &mut self.kind {
            MarkKind::Line(p) => p.dash = LineDash::new(array, offset),
            MarkKind::Rule(p) => {
                let solid = array.is_empty() || array.iter().sum::<f32>() <= 0.0;
                p.dash = (!solid).then(|| (array.to_vec(), offset));
            }
            _ => {}
        }
        self
    }

    /// How a line turns at its points.
    #[must_use]
    pub fn join(mut self, join: LineJoin) -> Self {
        if let MarkKind::Line(p) = &mut self.kind {
            p.join = join;
        }
        self
    }

    /// An area or bars standing on `below` (the series under it in a stack)
    /// rather than on the base.
    #[must_use]
    pub fn on(mut self, below: Series) -> Self {
        match &mut self.kind {
            MarkKind::Line(p) => p.floor = true,
            MarkKind::Bars(p) => p.floor = true,
            _ => return self,
        }
        self.floor = Some(below);
        self
    }

    /// Bars moved `offset` right of their point (logical pixels; their left
    /// edge's distance from it).
    #[must_use]
    pub fn offset(mut self, offset: f32) -> Self {
        if let MarkKind::Bars(p) = &mut self.kind {
            p.offset = offset;
        }
        self
    }

    /// Bars' far corners rounded; dots' radius.
    #[must_use]
    pub fn radius(mut self, r: f32) -> Self {
        match &mut self.kind {
            MarkKind::Bars(p) => p.radius = r,
            MarkKind::Scatter(p) => p.radius = r,
            _ => {}
        }
        self
    }

    /// Candles' colours, rising and falling.
    #[must_use]
    pub fn up_down(mut self, up: Color, down: Color) -> Self {
        if let MarkKind::Candles(p) = &mut self.kind {
            p.up = up;
            p.down = down;
        }
        self
    }

    /// A heatmap's colours and the values they stand for.
    #[must_use]
    pub fn scale(mut self, low: Color, high: Color, min: f32, max: f32) -> Self {
        if let MarkKind::Heatmap(p) = &mut self.kind {
            *p = HeatmapParams { low, high, min, max };
        }
        self
    }

    /// How far past the plot's box it may paint (logical pixels).
    fn reach(&self) -> f32 {
        match &self.kind {
            MarkKind::Line(p) => p.width * p.miter_limit.max(1.0) / 2.0 + 1.0,
            MarkKind::Bars(p) => (p.offset.abs() + p.width).max(0.0) + 1.0,
            MarkKind::Candles(p) => p.width / 2.0 + 1.0,
            MarkKind::Scatter(p) => p.radius + 1.0,
            MarkKind::Heatmap(_) => 0.0,
            MarkKind::Rule(p) => p.width / 2.0 + 1.0,
        }
    }

    /// Its data and parameters for `view`, or how its paths are drawn.
    fn resolve(&self, view: MarkView) -> Option<PlotMark> {
        if let MarkKind::Rule(p) = &self.kind {
            return Some(PlotMark::Rule { rule: p.clone(), view });
        }
        let mut sources: Vec<Source> = self.series.iter().map(|s| Source::Series(s.snapshot())).collect();
        if let Some(f) = self.floor {
            sources.push(Source::Series(f.snapshot()));
        }
        if let Some(g) = self.grid {
            sources.push(Source::Grid(g.snapshot()));
        }
        let (kind, words) = match &self.kind {
            MarkKind::Line(p) => (KindId::of::<Lines>(), LineParams { view, ..*p }.words()),
            MarkKind::Bars(p) => (KindId::of::<Bars>(), BarParams { view, ..*p }.words()),
            MarkKind::Candles(p) => (KindId::of::<Candles>(), CandleParams { view, ..*p }.words()),
            MarkKind::Scatter(p) => (KindId::of::<Scatter>(), ScatterParams { view, ..*p }.words()),
            MarkKind::Heatmap(p) => (KindId::of::<Heatmap>(), p.words()),
            MarkKind::Rule(_) => return None,
        };
        Some(PlotMark::Data {
            kind,
            params: words,
            sources,
            reach: self.reach(),
        })
    }

    fn is_scatter(&self) -> bool {
        matches!(self.kind, MarkKind::Scatter(_))
    }
}

/// The point under the pointer: which, where in data, where on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hover {
    /// The hovered mark, in the order given.
    pub mark: usize,
    /// The point's index in its series' snapshot.
    pub index: u32,
    pub x: i64,
    pub y: f64,
    /// Where it is drawn, window pixels.
    pub at: (f32, f32),
}

/// A plot of marks over one view.
#[derive(Clone)]
pub struct Plot {
    x: XRange,
    y: YRange,
    base: Option<f64>,
    marks: Vec<Mark>,
    hover: Option<Rc<dyn Fn(Option<Hover>)>>,
    hover_radius: f32,
}

/// A plot with no marks, showing x from the first point to the last and y
/// following the visible points.
#[must_use]
pub fn plot() -> Plot {
    Plot {
        x: XRange::Data,
        y: YRange::Auto(YAuto::default()),
        base: None,
        marks: Vec::new(),
        hover: None,
        hover_radius: 24.0,
    }
}

/// A plot's view resolved for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Resolved {
    x0: i64,
    x1: i64,
    y0: f64,
    y1: f64,
}

impl Plot {
    /// Show x from `x0` (left edge) to `x1` (right edge).
    #[must_use]
    pub fn x(mut self, x0: i64, x1: i64) -> Self {
        self.x = XRange::Fixed(x0, x1);
        self
    }

    /// Show the x range `f` returns, bound to the signals it reads.
    #[must_use]
    pub fn x_with(mut self, f: impl Fn() -> (i64, i64) + 'static) -> Self {
        self.x = XRange::Bound(Rc::new(f));
        self
    }

    /// Show y from `y0` (bottom) to `y1` (top).
    #[must_use]
    pub fn y(mut self, y0: f64, y1: f64) -> Self {
        self.y = YRange::Fixed(y0, y1);
        self
    }

    /// Show the y range `f` returns, bound to the signals it reads.
    #[must_use]
    pub fn y_with(mut self, f: impl Fn() -> (f64, f64) + 'static) -> Self {
        self.y = YRange::Bound(Rc::new(f));
        self
    }

    /// Follow the visible points' least and greatest y.
    #[must_use]
    pub fn y_auto(mut self, auto: YAuto) -> Self {
        self.y = YRange::Auto(auto);
        self
    }

    /// Areas and bars stand on y = `base` (default: the bottom).
    #[must_use]
    pub fn base(mut self, base: f64) -> Self {
        self.base = Some(base);
        self
    }

    /// Draw `m`, over the marks before it.
    #[must_use]
    pub fn mark(mut self, m: Mark) -> Self {
        self.marks.push(m);
        self
    }

    /// Told the point under the pointer (`None` when there is none or it
    /// leaves), in the frame it moved: of each line, area and bars the
    /// point nearest by x, of dots the nearest within `radius` pixels, and
    /// of those the nearest to the pointer.
    #[must_use]
    pub fn on_hover(mut self, radius: f32, f: impl Fn(Option<Hover>) + 'static) -> Self {
        self.hover = Some(Rc::new(f));
        self.hover_radius = radius;
        self
    }

    fn resolve(&self) -> Resolved {
        let first = self.marks.iter().find_map(|m| m.series.first().copied());
        let (x0, x1) = match &self.x {
            XRange::Fixed(a, b) => (*a, *b),
            XRange::Bound(f) => f(),
            XRange::Data => first
                .map(|s| s.snapshot())
                .and_then(|s| Some((s.get(0)?.0, s.get(s.len().checked_sub(1)?)?.0)))
                .unwrap_or((0, 1)),
        };
        let x1 = if x1 > x0 { x1 } else { x0.saturating_add(1) };
        let (y0, y1) = match &self.y {
            YRange::Fixed(a, b) => (*a, *b),
            YRange::Bound(f) => f(),
            YRange::Auto(auto) => {
                let mut lo = f64::INFINITY;
                let mut hi = f64::NEG_INFINITY;
                for m in &self.marks {
                    for s in m.series.iter().chain(m.floor.iter()) {
                        if let Some((a, b)) = s.snapshot().range_y(x0, x1.saturating_add(1)) {
                            lo = lo.min(f64::from(a));
                            hi = hi.max(f64::from(b));
                        }
                    }
                }
                for m in &self.marks {
                    if let MarkKind::Rule(RuleParams { at: RuleAt::Y(ys), .. }) = &m.kind {
                        for y in ys.iter().filter(|y| y.is_finite()) {
                            lo = lo.min(*y);
                            hi = hi.max(*y);
                        }
                    }
                }
                if auto.zero || self.base.is_some() {
                    let b = self.base.unwrap_or(0.0);
                    lo = lo.min(b);
                    hi = hi.max(b);
                }
                if lo > hi {
                    (0.0, 1.0)
                } else {
                    let span = (hi - lo).max(f64::MIN_POSITIVE.max(hi.abs() * 1e-9));
                    let pad = span * auto.pad;
                    let lo = if auto.zero && lo >= 0.0 { lo } else { lo - pad };
                    let hi = if auto.zero && hi <= 0.0 { hi } else { hi + pad };
                    if hi > lo { (lo, hi) } else { (lo - 0.5, hi + 0.5) }
                }
            }
        };
        Resolved { x0, x1, y0, y1 }
    }

    /// The plot's element (give it a size).
    #[must_use]
    pub fn build(self) -> Drawn {
        let spec = Rc::new(self);
        let draws = Rc::clone(&spec);
        let el = drawing(move || {
            let v = draws.resolve();
            let view = MarkView {
                x0: v.x0,
                x1: v.x1,
                y0: v.y0,
                y1: v.y1,
                base: draws.base,
            };
            let marks = draws.marks.iter().filter_map(|m| m.resolve(view)).collect();
            Drawing::new(PlotPicture { marks })
        });
        let Some(hover) = spec.hover.clone() else { return el };
        let leave = Rc::clone(&hover);
        el.on_pointer_move(move |e| hover(untracked(|| spec.hit(e.x, e.y, e.bounds))))
            .on_hover(move |inside| {
                if !inside {
                    leave(None);
                }
            })
    }

    /// The point under window pixel `(px, py)` of a plot in `b`.
    fn hit(&self, px: f32, py: f32, b: Bounds) -> Option<Hover> {
        let v = self.resolve();
        let span = i128::from(v.x1).saturating_sub(i128::from(v.x0)).max(1);
        let kx = f64::from(b.width.max(1e-3)) / super::kinds::i128_f64(span);
        let ky = f64::from(b.height.max(1e-3)) / (v.y1 - v.y0).abs().max(f64::MIN_POSITIVE);
        let to_x = |px: f32| -> i64 {
            let d = (f64::from(px - b.x) / kx).round();
            v.x0.saturating_add(f64_to_i64(d))
        };
        let x = to_x(px);
        let y = v.y1 - f64::from(py - b.y) / ky;
        let screen = |xi: i64, yi: f64| {
            let dx = super::kinds::i128_f64(i128::from(xi).saturating_sub(i128::from(v.x0)));
            (
                b.x + crate::convert::f64_to_f32(dx * kx),
                b.y + crate::convert::f64_to_f32((v.y1 - yi) * ky),
            )
        };
        let r = f64::from(self.hover_radius);
        let mut best: Option<(f64, Hover)> = None;
        for (k, m) in self.marks.iter().enumerate().rev() {
            let Some(s) = m.series.first() else { continue };
            let snap = s.snapshot_untracked();
            let found = if m.is_scatter() {
                let reach = f64_to_i64((r / kx).ceil()).max(0);
                let lo = snap.lower_bound(x.saturating_sub(reach));
                let hi = snap.lower_bound(x.saturating_add(reach).saturating_add(1));
                snap.nearest_within(lo, hi, (x, y), (kx, ky), r)
            } else {
                snap.nearest(x)
            };
            let Some(i) = found else { continue };
            let Some((xi, yi)) = snap.get(i) else { continue };
            let at = screen(xi, yi);
            let d = f64::from(at.0 - px).hypot(f64::from(at.1 - py));
            if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((
                    d,
                    Hover {
                        mark: k,
                        index: i,
                        x: xi,
                        y: yi,
                        at,
                    },
                ));
            }
        }
        best.map(|(_, h)| h)
    }
}

impl crate::element::IntoElement for Plot {
    fn into_element(self) -> crate::element::Element {
        self.build().into_element()
    }
}

#[derive(Debug, PartialEq)]
enum PlotMark {
    /// A data chunk.
    Data {
        kind: KindId,
        params: Words,
        sources: Vec<Source>,
        reach: f32,
    },
    /// Rules, as paths over the view.
    Rule { rule: RuleParams, view: MarkView },
}

impl RuleParams {
    /// The rules as one path in `area` (its top left the origin), and how
    /// they are stroked; `None` when none is within the view.
    fn path(&self, view: &MarkView, area: Bounds) -> Option<(crate::figure::Path, crate::figure::PathStyle)> {
        let (w, h) = (f64::from(area.width), f64::from(area.height));
        let f = crate::convert::f64_to_f32;
        let mut b = crate::figure::Path::builder();
        let mut any = false;
        match &self.at {
            RuleAt::Y(ys) => {
                let span = view.y1 - view.y0;
                for y in ys {
                    let t = (view.y1 - y) / span;
                    if t.is_finite() && (0.0..=1.0).contains(&t) {
                        let py = f(t * h);
                        b = b.move_to(0.0, py).line_to(f(w), py);
                        any = true;
                    }
                }
            }
            RuleAt::X(xs) => {
                let span = super::kinds::i128_f64(i128::from(view.x1).saturating_sub(i128::from(view.x0)).max(1));
                for x in xs {
                    let t = super::kinds::i128_f64(i128::from(*x).saturating_sub(i128::from(view.x0))) / span;
                    if (0.0..=1.0).contains(&t) {
                        let px = f(t * w);
                        b = b.move_to(px, 0.0).line_to(px, f(h));
                        any = true;
                    }
                }
            }
        }
        if !any {
            return None;
        }
        let mut stroke = crate::figure::Stroke::new(self.width);
        if let Some((array, offset)) = &self.dash {
            stroke = stroke.dash(array.clone(), *offset);
        }
        Some((
            b.build(),
            crate::figure::PathStyle::stroke(crate::scene::Gradient::Solid(self.color), stroke),
        ))
    }
}

/// A plot as one frame drew it.
#[derive(Debug, PartialEq)]
struct PlotPicture {
    marks: Vec<PlotMark>,
}

impl Picture for PlotPicture {
    fn draw(&self, area: Bounds, _scale: f32, key: u64, out: &mut Vec<Item>) {
        for (i, m) in self.marks.iter().enumerate() {
            match m {
                PlotMark::Data {
                    kind,
                    params,
                    sources,
                    reach,
                } => {
                    let mut h = foldhash::fast::FixedState::with_seed(0x0d47_a1c3).build_hasher();
                    key.hash(&mut h);
                    i.hash(&mut h);
                    out.push(Item::Data(std::sync::Arc::new(DataItem {
                        kind: *kind,
                        key: h.finish(),
                        bounds: area,
                        region: area.outset(*reach),
                        params: params.to_vec(),
                        sources: sources.clone(),
                        readback: None,
                    })));
                }
                PlotMark::Rule { rule, view } => {
                    if let Some((path, style)) = rule.path(view, area) {
                        out.push(Item::Path {
                            origin: (area.x, area.y),
                            path,
                            style,
                        });
                    }
                }
            }
        }
    }
}

/// An item of kind `K` standing alone, for a caller drawing its own
/// [`Picture`] of a kind of its own.
#[must_use]
pub fn item<K: ChunkKind>(key: u64, bounds: Bounds, region: Bounds, params: &K::Params, sources: Vec<Source>) -> Item {
    Item::Data(std::sync::Arc::new(DataItem::new::<K>(
        key, bounds, region, params, sources,
    )))
}

#[expect(
    clippy::as_conversions,
    reason = "a float offset to whole x units, saturating as intended"
)]
fn f64_to_i64(v: f64) -> i64 {
    v as i64
}
