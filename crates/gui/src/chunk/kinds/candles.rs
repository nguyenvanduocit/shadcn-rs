//! Candles: four series of one x (open, high, low, close), one candle a
//! device pixel column (`candles*.wgsl`).

use super::super::runtime::{
    Bind, BindKind, ChunkCompute, ChunkDraw, ChunkUpload, ComputeId, DataOp, DrawId, PipelineBuilder, Target,
};
use super::super::series::column_count;
use super::super::store::{BufferId, split};
use super::super::{ChunkKind, FrameInfo};
use super::m4::{BLOCK, MAX_COLUMNS, wgsl};
use super::{MarkView, Words, color, put_color, rgba};
use crate::color::Color;

/// Candles of sources 0 to 3 (open, high, low, close): bodies `width`
/// wide (logical pixels) where a column holds one, wicks `wick` wide,
/// `up` where the close is at or above the open, else `down`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CandleParams {
    pub view: MarkView,
    pub width: f32,
    pub wick: f32,
    pub up: Color,
    pub down: Color,
}

impl CandleParams {
    #[must_use]
    pub fn words(&self) -> Words {
        let mut w = [0; 64];
        self.view.put(&mut w);
        for (slot, v) in w.iter_mut().skip(10).zip([self.width.to_bits(), self.wick.to_bits()]) {
            *slot = v;
        }
        put_color(&mut w, 12, self.up);
        put_color(&mut w, 16, self.down);
        w
    }

    #[must_use]
    pub fn from_words(w: &Words) -> Self {
        let at = |i: usize| w.get(i).copied().unwrap_or(0);
        Self {
            view: MarkView::get(w),
            width: f32::from_bits(at(10)),
            wick: f32::from_bits(at(11)),
            up: color(w, 12),
            down: color(w, 16),
        }
    }
}

pub struct Candles {
    page: ComputeId,
    merge: ComputeId,
    draw: DrawId,
}

#[derive(Default)]
pub struct CandlesState {
    params: Option<BufferId>,
    bounds: Option<BufferId>,
    partials: Option<BufferId>,
    table: Option<BufferId>,
    candles: Option<BufferId>,
    style: Option<BufferId>,
    /// Per page: the four pages' seqs, its block, its columns.
    runs: Vec<([u64; 4], u64, u32)>,
    cols: u32,
}

impl ChunkKind for Candles {
    type Params = Words;
    type State = CandlesState;

    fn pipelines(ctx: &mut PipelineBuilder<'_>) -> Self {
        use BindKind::{Storage, StorageWrite, Uniform};
        let common = include_str!("candles.wgsl");
        Self {
            page: ctx.compute(
                "gui candles page",
                &wgsl(&[common, include_str!("candles_page.wgsl")], &["high", "low"]),
                "page_pass",
                &[
                    Uniform,
                    Uniform,
                    Storage,
                    Storage,
                    Storage,
                    Storage,
                    Storage,
                    StorageWrite,
                ],
            ),
            merge: ctx.compute(
                "gui candles merge",
                &wgsl(&[common, include_str!("candles_merge.wgsl")], &[]),
                "merge",
                &[Uniform, Storage, Storage, StorageWrite],
            ),
            draw: ctx.draw(
                "gui candles",
                include_str!("candles_draw.wgsl"),
                "vs_candle",
                "fs_candle",
                &[Uniform, Storage],
                Target::Main,
            ),
        }
    }

    fn update(&self, ctx: &mut ChunkUpload<'_>, state: &mut CandlesState, params: &Words, _: &[DataOp]) {
        state.runs.clear();
        state.cols = 0;
        let p = CandleParams::from_words(params);
        let frame = ctx.frame();
        let series: Vec<_> = (0..4).filter_map(|i| ctx.series(i)).collect();
        let [o, h, l, c] = series.as_slice() else { return };
        let same = [o, l, c].iter().all(|s| {
            s.pages.len() == h.pages.len()
                && s.pages
                    .iter()
                    .zip(h.pages)
                    .all(|(a, b)| a.len == b.len && a.layout.cap == b.layout.cap)
        });
        if !same || h.snapshot.is_empty() || p.view.x1 <= p.view.x0 {
            return;
        }
        let [bx, by, bw, bh] = frame.bounds;
        let first = bx.floor();
        let frac = bx - first;
        let cols = column_count(bw, frac).min(MAX_COLUMNS);
        let bounds = super::super::series::column_bounds(p.view.x0, p.view.x1, bw, frac);
        let span = i128::from(p.view.x1).saturating_sub(i128::from(p.view.x0)).max(1);
        let kx = crate::convert::f64_to_f32(f64::from(bw) / super::i128_f64(span));
        let ky = crate::convert::f64_to_f32(f64::from(bh) / (p.view.y1 - p.view.y0).abs().max(f64::MIN_POSITIVE));
        let y1 = crate::convert::f64_to_f32(p.view.y1);
        let y1lo = crate::convert::f64_to_f32(p.view.y1 - f64::from(y1));
        let mut table = Vec::new();
        let mut blocks: Vec<(u64, [u32; 8])> = Vec::new();
        let mut out = 0_u32;
        let pages: Vec<_> = (0..h.pages.len())
            .filter_map(|k| Some([o.pages.get(k)?, h.pages.get(k)?, l.pages.get(k)?, c.pages.get(k)?]))
            .collect();
        for quad in &pages {
            let [po, ph, pl, pc] = *quad;
            let (Some(b0), Some(b1)) = (bounds.first(), bounds.last()) else {
                continue;
            };
            if ph.x_last < *b0 || ph.x_first >= *b1 {
                continue;
            }
            let c0 = u32::try_from(bounds.partition_point(|b| *b <= ph.x_first).saturating_sub(1)).unwrap_or(0);
            let c1 = u32::try_from(bounds.partition_point(|b| *b <= ph.x_last))
                .unwrap_or(0)
                .min(cols);
            if c1 <= c0 {
                continue;
            }
            let block = BLOCK.saturating_mul(u64::try_from(blocks.len()).unwrap_or(0).saturating_add(1));
            let lay = ph.layout;
            blocks.push((block, [ph.len, lay.cap, ph.at, lay.y, lay.levels, c0, c1, out]));
            state
                .runs
                .push(([po.seq, ph.seq, pl.seq, pc.seq], block, c1.saturating_sub(c0)));
            table.extend([c0, c1, out, 0]);
            out = out.saturating_add(c1.saturating_sub(c0));
        }
        if state.runs.is_empty() {
            return;
        }
        let x0 = split(p.view.x0);
        let m4: [u32; 16] = [
            cols,
            u32::try_from(state.runs.len()).unwrap_or(0),
            0,
            0,
            x0.0,
            x0.1,
            kx.to_bits(),
            bx.to_bits(),
            y1.to_bits(),
            y1lo.to_bits(),
            ky.to_bits(),
            by.to_bits(),
            0,
            0,
            0,
            0,
        ];
        let n_blocks = u64::try_from(blocks.len()).unwrap_or(0).saturating_add(1);
        let (Some(pb), Some(bb), Some(_), Some(tb), Some(_), Some(sb)) = (
            ctx.buffer(&mut state.params, BLOCK.saturating_mul(n_blocks)),
            ctx.buffer(
                &mut state.bounds,
                u64::try_from(bounds.len()).unwrap_or(0).saturating_mul(8),
            ),
            ctx.buffer(&mut state.partials, u64::from(out.max(1)).saturating_mul(48)),
            ctx.buffer(
                &mut state.table,
                u64::try_from(table.len()).unwrap_or(0).saturating_mul(4),
            ),
            ctx.buffer(&mut state.candles, u64::from(cols).saturating_mul(32)),
            ctx.buffer(&mut state.style, 48),
        ) else {
            state.runs.clear();
            return;
        };
        ctx.write(pb, 0, bytemuck::cast_slice(&m4));
        for (block, words) in &blocks {
            ctx.write(pb, *block, bytemuck::cast_slice(words));
        }
        let bw_words: Vec<u32> = bounds
            .iter()
            .flat_map(|x| {
                let (a, b) = split(*x);
                [a, b]
            })
            .collect();
        ctx.write(bb, 0, bytemuck::cast_slice(&bw_words));
        ctx.write(tb, 0, bytemuck::cast_slice(&table));
        let s = frame.scale;
        let mut style: Vec<u32> = vec![cols, (p.width * s).to_bits(), (p.wick * s).max(1.0).to_bits(), 0];
        style.extend(rgba(p.up).map(f32::to_bits));
        style.extend(rgba(p.down).map(f32::to_bits));
        ctx.write(sb, 0, bytemuck::cast_slice(&style));
        state.cols = cols;
    }

    fn compute(&self, pass: &mut ChunkCompute<'_>, state: &CandlesState, _: &Words, _: &FrameInfo) {
        let (Some(params), Some(bounds), Some(partials), Some(table), Some(candles)) =
            (state.params, state.bounds, state.partials, state.table, state.candles)
        else {
            return;
        };
        if state.runs.is_empty() {
            return;
        }
        let m4 = Bind::Range(params, 0, 64);
        for (seqs, block, columns) in &state.runs {
            pass.dispatch(
                self.page,
                &[
                    m4,
                    Bind::Range(params, *block, 32),
                    Bind::Page(seqs[0]),
                    Bind::Page(seqs[1]),
                    Bind::Page(seqs[2]),
                    Bind::Page(seqs[3]),
                    Bind::Buffer(bounds),
                    Bind::Buffer(partials),
                ],
                [columns.div_ceil(64), 1, 1],
            );
        }
        pass.dispatch(
            self.merge,
            &[m4, Bind::Buffer(table), Bind::Buffer(partials), Bind::Buffer(candles)],
            [state.cols.div_ceil(64), 1, 1],
        );
    }

    fn draw(&self, pass: &mut ChunkDraw<'_>, state: &CandlesState, _: &Words, _: &FrameInfo) {
        if let (Some(style), Some(candles), false) = (state.style, state.candles, state.runs.is_empty()) {
            pass.draw(
                self.draw,
                &[Bind::Buffer(style), Bind::Buffer(candles)],
                0..4,
                0..state.cols,
            );
        }
    }
}
