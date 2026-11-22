//! `gui`'s own data chunk kinds (ADR-0018): lines and areas, bars, candles,
//! scatter, heatmaps. Each is a [`ChunkKind`] like a user's; its
//! parameters are a fixed array of words, made by its params type.

mod candles;
mod heatmap;
pub(crate) mod m4;
pub(crate) mod scatter;

use super::runtime::{
    Bind, BindKind, ChunkCompute, ChunkDraw, ChunkUpload, ComputeId, DataOp, DrawId, PipelineBuilder, Target,
};
use super::store::BufferId;
use super::{ChunkKind, FrameInfo};
use crate::color::Color;

pub use self::candles::{CandleParams, Candles};
pub use self::heatmap::{Heatmap, HeatmapParams};
pub use self::m4::MAX_COLUMNS;
pub use self::scatter::{Scatter, ScatterParams};

/// Words of every kind's parameters.
pub type Words = [u32; 64];

/// What part of the data a mark shows, and how data maps to its bounds:
/// x from `x0` (left) to `x1` (right), y from `y0` (bottom) to `y1` (top);
/// areas and bars stand on `base` (`None`: the bottom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkView {
    pub x0: i64,
    pub x1: i64,
    pub y0: f64,
    pub y1: f64,
    pub base: Option<f64>,
}

impl MarkView {
    fn put(&self, w: &mut Words) {
        let (a, b) = super::store::split(self.x0);
        let (c, d) = super::store::split(self.x1);
        let f = |v: f64| {
            let hi = crate::convert::f64_to_f32(v);
            (hi.to_bits(), crate::convert::f64_to_f32(v - f64::from(hi)).to_bits())
        };
        let (e, g) = f(self.y0);
        let (h, i) = f(self.y1);
        let (j, k) = self.base.map_or((NAN, NAN), f);
        for (slot, v) in w.iter_mut().zip([a, b, c, d, e, g, h, i, j, k]) {
            *slot = v;
        }
    }

    fn get(w: &Words) -> Self {
        let i64_of = |hi: u32, lo: u32| i64::from_ne_bytes(((u64::from(hi) << 32) | u64::from(lo)).to_ne_bytes());
        let f = |hi: u32, lo: u32| f64::from(f32::from_bits(hi)) + f64::from(f32::from_bits(lo));
        let at = |i: usize| w.get(i).copied().unwrap_or(0);
        Self {
            x0: i64_of(at(0), at(1)),
            x1: i64_of(at(2), at(3)),
            y0: f(at(4), at(5)),
            y1: f(at(6), at(7)),
            base: (at(8) != NAN).then(|| f(at(8), at(9))),
        }
    }
}

const NAN: u32 = 0x7fc0_0000;

/// A colour as four words from `at`.
fn put_color(w: &mut Words, at: usize, c: Color) {
    for (slot, v) in w.iter_mut().skip(at).zip([c.r, c.g, c.b, c.a]) {
        *slot = v.to_bits();
    }
}

/// The colour at `at`.
fn color(w: &Words, at: usize) -> Color {
    let f = |i: usize| f32::from_bits(w.get(at.saturating_add(i)).copied().unwrap_or(0));
    Color {
        r: f(0),
        g: f(1),
        b: f(2),
        a: f(3),
    }
}

/// A colour as the four floats a shader reads.
fn rgba(c: Color) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

/// How a line turns at a point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineJoin {
    /// Mitred up to the limit, then bevelled (SVG's default).
    #[default]
    Miter,
    Bevel,
    Round,
}

/// What runs between a line's points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineCurve {
    #[default]
    Linear,
    /// d3's natural cubic spline (`curveNatural`), for series of at most
    /// [`CURVE_MAX`] points; longer ones run straight.
    Natural,
}

/// The most points a natural curve is drawn through.
pub const CURVE_MAX: u32 = 65_536;

/// A line, an area under it, or both (source 0 the series; source 1, with
/// `floor`, the series the area stands on: the one below in a stack).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineParams {
    pub view: MarkView,
    /// The line's colour and width (logical pixels); a width of 0 draws no line.
    pub color: Color,
    pub width: f32,
    /// The area's colour at the top and the bottom of the bounds; `None`, no area.
    pub area: Option<(Color, Color)>,
    pub join: LineJoin,
    pub miter_limit: f32,
    pub curve: LineCurve,
    pub floor: bool,
    /// Dashes along the line: up to two on and off lengths and where the
    /// pattern starts (logical pixels), restarting at every break.
    pub dash: Option<LineDash>,
}

/// A line's dash pattern (logical pixels): on and off lengths in turn,
/// `count` of `array` used (even), and how far into it a run starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineDash {
    pub array: [f32; 8],
    pub count: u32,
    pub offset: f32,
}

impl LineDash {
    /// The pattern of `array` from `offset`, as SVG's `stroke-dasharray`
    /// reads it: an odd count is repeated to make it even. `None` (a solid
    /// line) for an empty, negative or all-zero pattern, and for one of
    /// more than eight lengths once even.
    #[must_use]
    pub fn new(array: &[f32], offset: f32) -> Option<Self> {
        let valid = array.iter().all(|v| v.is_finite() && *v >= 0.0);
        if !valid || array.iter().sum::<f32>() <= 0.0 || !offset.is_finite() {
            return None;
        }
        let even: Vec<f32> = if array.len() % 2 == 1 {
            array.iter().chain(array).copied().collect()
        } else {
            array.to_vec()
        };
        if even.len() > 8 {
            return None;
        }
        let mut a = [0.0; 8];
        for (d, v) in a.iter_mut().zip(&even) {
            *d = *v;
        }
        Some(Self {
            array: a,
            count: u32::try_from(even.len()).unwrap_or(0),
            offset,
        })
    }
}

impl LineParams {
    #[must_use]
    pub fn words(&self) -> Words {
        let mut w = [0; 64];
        self.view.put(&mut w);
        let (top, bottom) = self.area.unwrap_or((Color::TRANSPARENT, Color::TRANSPARENT));
        let join = match self.join {
            LineJoin::Miter => 0,
            LineJoin::Bevel => 1,
            LineJoin::Round => 2,
        };
        let flags = u32::from(self.curve == LineCurve::Natural)
            | (join << 1)
            | (u32::from(self.floor) << 3)
            | (u32::from(self.area.is_some()) << 4)
            | (u32::from(self.dash.is_some()) << 5);
        put_color(&mut w, 10, self.color);
        put_color(&mut w, 14, top);
        put_color(&mut w, 18, bottom);
        for (slot, v) in w
            .iter_mut()
            .skip(22)
            .zip([self.width.to_bits(), flags, self.miter_limit.to_bits()])
        {
            *slot = v;
        }
        if let Some(d) = self.dash {
            for (slot, v) in w.iter_mut().skip(25).zip(d.array.iter().chain([&d.offset])) {
                *slot = v.to_bits();
            }
            if let Some(slot) = w.get_mut(34) {
                *slot = d.count;
            }
        }
        w
    }

    #[must_use]
    pub fn from_words(w: &Words) -> Self {
        let at = |i: usize| w.get(i).copied().unwrap_or(0);
        let flags = at(23);
        Self {
            view: MarkView::get(w),
            color: color(w, 10),
            width: f32::from_bits(at(22)),
            area: (flags & 16 != 0).then(|| (color(w, 14), color(w, 18))),
            join: match (flags >> 1) & 3 {
                1 => LineJoin::Bevel,
                2 => LineJoin::Round,
                _ => LineJoin::Miter,
            },
            miter_limit: f32::from_bits(at(24)),
            curve: if flags & 1 != 0 {
                LineCurve::Natural
            } else {
                LineCurve::Linear
            },
            floor: flags & 8 != 0,
            dash: (flags & 32 != 0).then(|| LineDash {
                array: std::array::from_fn(|i| f32::from_bits(at(i.saturating_add(25)))),
                offset: f32::from_bits(at(33)),
                count: at(34),
            }),
        }
    }
}

/// Bars from each point down to the base (or the floor series, source 1):
/// `width` wide, moved `offset` right of the point (logical pixels), the
/// far corners rounded by `radius`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarParams {
    pub view: MarkView,
    pub color: Color,
    pub width: f32,
    pub offset: f32,
    pub radius: f32,
    pub floor: bool,
}

impl BarParams {
    #[must_use]
    pub fn words(&self) -> Words {
        let mut w = [0; 64];
        self.view.put(&mut w);
        put_color(&mut w, 10, self.color);
        for (slot, v) in w.iter_mut().skip(14).zip([
            self.width.to_bits(),
            self.offset.to_bits(),
            self.radius.to_bits(),
            u32::from(self.floor),
        ]) {
            *slot = v;
        }
        w
    }

    #[must_use]
    pub fn from_words(w: &Words) -> Self {
        let at = |i: usize| w.get(i).copied().unwrap_or(0);
        Self {
            view: MarkView::get(w),
            color: color(w, 10),
            width: f32::from_bits(at(14)),
            offset: f32::from_bits(at(15)),
            radius: f32::from_bits(at(16)),
            floor: at(17) != 0,
        }
    }
}

/// d3's natural cubic spline through `k` on one axis: each segment's two
/// controls (its `controlPoints`, solved with the Thomas algorithm).
fn natural(k: &[f64]) -> Vec<(f64, f64)> {
    let n = k.len().saturating_sub(1);
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        let (a, b) = (k.first().copied().unwrap_or(0.0), k.get(1).copied().unwrap_or(0.0));
        return vec![(a + (b - a) / 3.0, a + 2.0 * (b - a) / 3.0)];
    }
    let at = |i: usize| k.get(i).copied().unwrap_or(0.0);
    let mut a = vec![0.0; n];
    let mut b = vec![0.0; n];
    let mut r = vec![0.0; n];
    let last = n.saturating_sub(1);
    for i in 0..n {
        let (ai, bi, ri) = if i == 0 {
            (0.0, 2.0, at(0) + 2.0 * at(1))
        } else if i == last {
            (2.0, 7.0, 8.0 * at(last) + at(n))
        } else {
            (1.0, 4.0, 4.0 * at(i) + 2.0 * at(i.saturating_add(1)))
        };
        if let (Some(x), Some(y), Some(z)) = (a.get_mut(i), b.get_mut(i), r.get_mut(i)) {
            *x = ai;
            *y = bi;
            *z = ri;
        }
    }
    for i in 1..n {
        let m = a.get(i).copied().unwrap_or(0.0) / b.get(i.saturating_sub(1)).copied().unwrap_or(1.0);
        let pr = r.get(i.saturating_sub(1)).copied().unwrap_or(0.0);
        if let (Some(y), Some(z)) = (b.get_mut(i), r.get_mut(i)) {
            *y -= m;
            *z -= m * pr;
        }
    }
    let mut first = vec![0.0; n];
    if let Some(f) = first.get_mut(last) {
        *f = r.get(last).copied().unwrap_or(0.0) / b.get(last).copied().unwrap_or(1.0);
    }
    for i in (0..last).rev() {
        let next = first.get(i.saturating_add(1)).copied().unwrap_or(0.0);
        if let Some(f) = first.get_mut(i) {
            *f = (r.get(i).copied().unwrap_or(0.0) - next) / b.get(i).copied().unwrap_or(1.0);
        }
    }
    let mut second = vec![0.0; n];
    for i in 0..last {
        if let Some(s) = second.get_mut(i) {
            *s = 2.0 * at(i.saturating_add(1)) - first.get(i.saturating_add(1)).copied().unwrap_or(0.0);
        }
    }
    if let Some(s) = second.get_mut(last) {
        *s = f64::midpoint(at(n), first.get(last).copied().unwrap_or(0.0));
    }
    first.into_iter().zip(second).collect()
}

/// The controls of every segment of `series` as data deltas (from its
/// first point, from its second), as `ctrl` in `lines.wgsl` reads them:
/// a natural spline through each run of valid points.
fn controls(series: &super::series::SeriesSnapshot) -> Vec<[f32; 4]> {
    let n = series.len();
    let mut out = vec![[0.0_f32; 4]; usize::try_from(n).unwrap_or(0)];
    let origin = series.get(0).map_or(0, |(x, _)| x);
    let mut run: Vec<(u32, f64, f64)> = Vec::new();
    let flush = |run: &mut Vec<(u32, f64, f64)>, out: &mut Vec<[f32; 4]>| {
        let xs: Vec<f64> = run.iter().map(|p| p.1).collect();
        let ys: Vec<f64> = run.iter().map(|p| p.2).collect();
        for (k, ((cx1, cx2), (cy1, cy2))) in natural(&xs).into_iter().zip(natural(&ys)).enumerate() {
            let (Some(a), Some(b)) = (run.get(k), run.get(k.saturating_add(1))) else {
                continue;
            };
            if let Some(o) = usize::try_from(a.0).ok().and_then(|i| out.get_mut(i)) {
                *o = [cx1 - a.1, cy1 - a.2, cx2 - b.1, cy2 - b.2].map(crate::convert::f64_to_f32);
            }
        }
        run.clear();
    };
    for i in 0..n {
        let Some((x, y)) = series.get(i) else { continue };
        if y.is_finite() {
            run.push((i, i64_delta(x, origin), y));
        } else {
            flush(&mut run, &mut out);
        }
    }
    flush(&mut run, &mut out);
    out
}

fn i64_delta(x: i64, origin: i64) -> f64 {
    i128_f64(i128::from(x).saturating_sub(i128::from(origin)))
}

/// An x span or delta as a float.
#[expect(clippy::as_conversions, reason = "an i128 to a float, rounding as intended")]
pub(crate) fn i128_f64(v: i128) -> f64 {
    v as f64
}

/// Lines and areas.
pub struct Lines {
    m4: m4::M4Pipes,
    segments: ComputeId,
    arcs: ComputeId,
    strip: DrawId,
    area: DrawId,
}

#[derive(Default)]
pub struct LinesState {
    pub(crate) m4: m4::M4State,
    style: Option<BufferId>,
    ctrl: Option<BufferId>,
    fctrl: Option<BufferId>,
    segs: Option<BufferId>,
    sub: u32,
    /// Segments laid out, and the pixel columns the line's quads stand on.
    cap: u32,
    strips: u32,
    dashed: bool,
    live: bool,
}

/// Bytes of a laid-out segment (`Seg` in `line_geom.wgsl`).
const SEG: u64 = 7 * 16;

impl ChunkKind for Lines {
    type Params = Words;
    type State = LinesState;

    fn pipelines(ctx: &mut PipelineBuilder<'_>) -> Self {
        use BindKind::{Storage, StorageWrite, Uniform};
        let binds = [Uniform, Storage, Storage, Storage, Storage, Storage];
        let geom = include_str!("line_geom.wgsl");
        let src = format!("{}\n{geom}", include_str!("lines.wgsl"));
        let segs = format!("{geom}\n{}", include_str!("line_segments.wgsl"));
        let cbinds = [Uniform, Storage, Storage, Storage, Storage, StorageWrite];
        Self {
            m4: m4::M4Pipes::new(ctx),
            segments: ctx.compute("gui line segments", &segs, "segments", &cbinds),
            arcs: ctx.compute("gui line arcs", &segs, "arcs", &cbinds),
            strip: ctx.draw("gui lines", &src, "vs_strip", "fs_strip", &binds, Target::Main),
            area: ctx.draw("gui areas", &src, "vs_area", "fs_area", &binds, Target::Area),
        }
    }

    fn update(&self, ctx: &mut ChunkUpload<'_>, state: &mut LinesState, params: &Words, ops: &[DataOp]) {
        let p = LineParams::from_words(params);
        let frame = ctx.frame();
        state.live = state.m4.update(&self.m4, ctx, &p.view, 0, p.floor.then_some(1), &frame);
        if !state.live {
            return;
        }
        let curve = p.curve == LineCurve::Natural && ctx.series(0).is_some_and(|s| s.snapshot.len() <= CURVE_MAX);
        state.sub = if curve { 16 } else { 1 };
        let mut fcurve = false;
        if curve {
            let stale = state.ctrl.is_none() || !ops.is_empty();
            if stale && let Some(s) = ctx.series(0) {
                let c = controls(s.snapshot);
                let bytes = u64::try_from(c.len()).unwrap_or(0).max(1).saturating_mul(16);
                if let Some(b) = ctx.buffer(&mut state.ctrl, bytes) {
                    ctx.write(b, 0, bytemuck::cast_slice(&c));
                }
            }
            if p.floor
                && let Some(s) = ctx.series(1)
            {
                fcurve = true;
                if stale || state.fctrl.is_none() {
                    let c = controls(s.snapshot);
                    let bytes = u64::try_from(c.len()).unwrap_or(0).max(1).saturating_mul(16);
                    if let Some(b) = ctx.buffer(&mut state.fctrl, bytes) {
                        ctx.write(b, 0, bytemuck::cast_slice(&c));
                    }
                }
            }
        }
        if state.ctrl.is_none() {
            // Bound though unread: a straight line reads no controls.
            if let Some(b) = ctx.buffer(&mut state.ctrl, 16) {
                ctx.write(b, 0, &[0; 16]);
            }
        }
        let join = match p.join {
            LineJoin::Miter => 0_u32,
            LineJoin::Bevel => 1,
            LineJoin::Round => 2,
        };
        let hw = p.width * frame.scale * 0.5;
        let limit = p.miter_limit.max(1.0);
        // How far a segment's coverage reaches past its points: its quad's
        // along-reach (a miter's tip, else the half width) and a pixel; a
        // curve's pieces may bow a column past their points.
        let along = if p.join == LineJoin::Miter { hw * limit } else { hw };
        let reach = crate::convert::f32_to_u32((along + 1.0).ceil()).saturating_add(u32::from(curve));
        state.cap = state.m4.capacity().saturating_mul(state.sub.max(1));
        state.strips = state.m4.cols.saturating_add(reach.saturating_mul(2));
        state.dashed = p.dash.is_some();
        let s = frame.scale;
        let (dash, period, phase) = p.dash.map_or(([0.0; 8], 0.0, 0.0), |d| {
            let a = d.array.map(|v| v * s);
            (a, a.iter().sum::<f32>(), d.offset * s)
        });
        let first_col = frame.bounds[0].floor();
        let mut style: Vec<u32> = vec![
            state.m4.n,
            state.sub,
            join,
            u32::from(curve),
            hw.to_bits(),
            limit.to_bits(),
            state.m4.scale.0.to_bits(),
            state.m4.scale.1.to_bits(),
            u32::from(fcurve),
            first_col.to_bits(),
            reach,
            state.cap,
        ];
        style.extend(dash.map(f32::to_bits));
        let phase = if period > 0.0 { phase.rem_euclid(period) } else { 0.0 };
        let count = p.dash.map_or(0, |d| d.count);
        style.extend([period.to_bits(), phase.to_bits(), u32::from(state.dashed), count]);
        if let Some(b) = ctx.buffer(&mut state.style, 96) {
            ctx.write(b, 0, bytemuck::cast_slice(&style));
        }
        if p.width > 0.0 {
            ctx.buffer(&mut state.segs, u64::from(state.cap.max(1)).saturating_mul(SEG));
        }
    }

    fn compute(&self, pass: &mut ChunkCompute<'_>, state: &LinesState, _: &Words, _: &FrameInfo) {
        if !state.live {
            return;
        }
        state.m4.compute(&self.m4, pass);
        if let Some(binds) = self.binds(state) {
            if state.segs.is_none() {
                return;
            }
            pass.dispatch(self.segments, &binds, [state.cap.div_ceil(64), 1, 1]);
            if state.dashed {
                pass.dispatch(self.arcs, &binds, [1, 1, 1]);
            }
        }
    }

    fn draw(&self, pass: &mut ChunkDraw<'_>, state: &LinesState, params: &Words, _: &FrameInfo) {
        let p = LineParams::from_words(params);
        if !state.live {
            return;
        }
        let Some(binds) = self.binds(state) else { return };
        let instances = state.m4.capacity().saturating_mul(state.sub.max(1));
        let (top, bottom) = p.area.unwrap_or((Color::TRANSPARENT, Color::TRANSPARENT));
        if p.area.is_some() {
            pass.draw(self.area, &binds, 0..4, 0..instances);
            // The area under the line: composited first, the line over it.
            pass.composite(top, bottom, Color::TRANSPARENT);
        }
        pass.colors(top, bottom, p.color);
        if p.width > 0.0 && state.segs.is_some() {
            pass.draw(self.strip, &binds, 0..4, 0..state.strips);
        }
    }
}

impl Lines {
    /// What the passes and draws of a line bind, in `line_geom.wgsl`'s order.
    fn binds(&self, state: &LinesState) -> Option<[Bind; 6]> {
        let (Some(style), Some(points), Some(offsets), Some(ctrl)) =
            (state.style, state.m4.points, state.m4.offsets, state.ctrl)
        else {
            return None;
        };
        let fctrl = state.fctrl.unwrap_or(ctrl);
        // An area alone lays out no segments; its draws bind the points again.
        let segs = state.segs.unwrap_or(points);
        Some([
            Bind::Buffer(style),
            Bind::Buffer(points),
            Bind::Buffer(offsets),
            Bind::Buffer(ctrl),
            Bind::Buffer(fctrl),
            Bind::Buffer(segs),
        ])
    }
}

/// Bars.
pub struct Bars {
    m4: m4::M4Pipes,
    bar: DrawId,
}

#[derive(Default)]
pub struct BarsState {
    pub(crate) m4: m4::M4State,
    style: Option<BufferId>,
    live: bool,
}

impl ChunkKind for Bars {
    type Params = Words;
    type State = BarsState;

    fn pipelines(ctx: &mut PipelineBuilder<'_>) -> Self {
        use BindKind::{Storage, Uniform};
        Self {
            m4: m4::M4Pipes::new(ctx),
            bar: ctx.draw(
                "gui bars",
                include_str!("bars.wgsl"),
                "vs_bar",
                "fs_bar",
                &[Uniform, Storage, Storage],
                Target::Main,
            ),
        }
    }

    fn update(&self, ctx: &mut ChunkUpload<'_>, state: &mut BarsState, params: &Words, _: &[DataOp]) {
        let p = BarParams::from_words(params);
        let frame = ctx.frame();
        state.live = state.m4.update(&self.m4, ctx, &p.view, 0, p.floor.then_some(1), &frame);
        if !state.live {
            return;
        }
        let s = frame.scale;
        let mut style: Vec<u32> = vec![
            state.m4.n,
            state.m4.cols.saturating_add(1),
            (p.width * s).to_bits(),
            (p.offset * s).to_bits(),
            (p.radius * s).to_bits(),
            0,
            0,
            0,
        ];
        style.extend(rgba(p.color).map(f32::to_bits));
        if let Some(b) = ctx.buffer(&mut state.style, 48) {
            ctx.write(b, 0, bytemuck::cast_slice(&style));
        }
    }

    fn compute(&self, pass: &mut ChunkCompute<'_>, state: &BarsState, _: &Words, _: &FrameInfo) {
        if state.live {
            state.m4.compute(&self.m4, pass);
        }
    }

    fn draw(&self, pass: &mut ChunkDraw<'_>, state: &BarsState, _: &Words, _: &FrameInfo) {
        let (Some(style), Some(points), Some(offsets)) = (state.style, state.m4.points, state.m4.offsets) else {
            return;
        };
        if state.live {
            pass.draw(
                self.bar,
                &[Bind::Buffer(style), Bind::Buffer(points), Bind::Buffer(offsets)],
                0..4,
                0..state.m4.capacity(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_round_trip() {
        let p = LineParams {
            view: MarkView {
                x0: -5_000_000_000_123,
                x1: 9_000_000_000_000_000,
                y0: -1.5,
                y1: 1e9 + 0.25,
                base: Some(3.0),
            },
            color: Color::hex(0x01_02_03),
            width: 2.0,
            area: Some((Color::hex(0x05_06_07).opacity(0.5), Color::hex(0x09_0a_0b))),
            join: LineJoin::Round,
            miter_limit: 4.0,
            curve: LineCurve::Natural,
            floor: true,
            dash: LineDash::new(&[4.0, 2.5], 1.5),
        };
        assert_eq!(LineParams::from_words(&p.words()), p);
    }

    #[test]
    fn natural_controls_match_d3() {
        // d3's curveNatural through (0,0) (1,1) (2,0): controls of the
        // first segment at x 1/3·…, symmetric about x = 1.
        let c = natural(&[0.0, 1.0, 2.0]);
        assert_eq!(c.len(), 2);
        let ys = natural(&[0.0, 1.0, 0.0]);
        assert!((ys[0].0 - 0.5).abs() < 1e-9 && (ys[0].1 - 1.0).abs() < 1e-9, "{ys:?}");
        assert!((ys[1].0 - 1.0).abs() < 1e-9 && (ys[1].1 - 0.5).abs() < 1e-9, "{ys:?}");
    }
}
