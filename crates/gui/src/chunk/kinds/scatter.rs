//! Scatter: points culled and compacted in order per page, drawn by
//! indirect draws (`scatter*.wgsl`).

use super::super::runtime::{
    Bind, BindKind, ChunkCompute, ChunkDraw, ChunkUpload, ComputeId, DataOp, DrawId, PipelineBuilder, Target,
};
use super::super::series::NONE;
use super::super::store::{BufferId, split};
use super::super::{ChunkKind, FrameInfo};
use super::m4::BLOCK;
use super::{MarkView, Words, color, put_color, rgba};
use crate::color::Color;

/// Points of a block (256 threads, 16 each).
const BLOCK_POINTS: u32 = 4096;

/// Dots of `radius` (logical pixels) at source 0's points, in `color`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScatterParams {
    pub view: MarkView,
    pub color: Color,
    pub radius: f32,
}

impl ScatterParams {
    #[must_use]
    pub fn words(&self) -> Words {
        let mut w = [0; 64];
        self.view.put(&mut w);
        put_color(&mut w, 10, self.color);
        if let Some(r) = w.get_mut(14) {
            *r = self.radius.to_bits();
        }
        w
    }

    #[must_use]
    pub fn from_words(w: &Words) -> Self {
        let at = |i: usize| w.get(i).copied().unwrap_or(0);
        Self {
            view: MarkView::get(w),
            color: color(w, 10),
            radius: f32::from_bits(at(14)),
        }
    }
}

pub struct Scatter {
    reduce: ComputeId,
    scan: ComputeId,
    compact: ComputeId,
    draw: DrawId,
}

/// One page's buffers and dispatch.
#[derive(Default)]
pub(crate) struct PageBufs {
    sums: Option<BufferId>,
    starts: Option<BufferId>,
    pub(crate) shown: Option<BufferId>,
}

#[derive(Default)]
pub struct ScatterState {
    params: Option<BufferId>,
    pub(crate) args: Option<BufferId>,
    style: Option<BufferId>,
    pub(crate) bufs: Vec<PageBufs>,
    /// Per page: its seq, its block, its blocks of points.
    runs: Vec<(u64, u64, u32)>,
}

impl ChunkKind for Scatter {
    type Params = Words;
    type State = ScatterState;

    fn pipelines(ctx: &mut PipelineBuilder<'_>) -> Self {
        use BindKind::{Storage, StorageWrite, Uniform};
        let common = concat!(include_str!("x64.wgsl"), include_str!("scatter_common.wgsl"));
        let cull = |name: &str, src: &str| {
            format!(
                "{common}{src}{}",
                if name == "scan" { include_str!("scan.wgsl") } else { "" }
            )
        };
        Self {
            reduce: ctx.compute(
                "gui scatter cull",
                &cull("reduce", include_str!("scatter_cull.wgsl")),
                "reduce",
                &[Uniform, Uniform, Storage, StorageWrite],
            ),
            scan: ctx.compute(
                "gui scatter scan",
                &format!(
                    "{}{}{}",
                    include_str!("scatter_common.wgsl")
                        .split("const NONE")
                        .next()
                        .unwrap_or(""),
                    include_str!("scatter_scan.wgsl"),
                    include_str!("scan.wgsl")
                ),
                "scan",
                &[Uniform, Storage, StorageWrite, StorageWrite],
            ),
            compact: ctx.compute(
                "gui scatter compact",
                &cull("compact", include_str!("scatter_compact.wgsl")),
                "compact",
                &[Uniform, Uniform, Storage, Storage, StorageWrite],
            ),
            draw: ctx.draw(
                "gui scatter",
                &format!("{common}{}", include_str!("scatter_draw.wgsl")),
                "vs_dot",
                "fs_dot",
                &[Uniform, Uniform, Storage, Storage, Uniform],
                Target::Main,
            ),
        }
    }

    fn update(&self, ctx: &mut ChunkUpload<'_>, state: &mut ScatterState, params: &Words, _: &[DataOp]) {
        state.runs.clear();
        let p = ScatterParams::from_words(params);
        let frame = ctx.frame();
        let Some(series) = ctx.series(0) else { return };
        if p.view.x1 <= p.view.x0 {
            return;
        }
        let [bx, by, bw, bh] = frame.bounds;
        let span = i128::from(p.view.x1).saturating_sub(i128::from(p.view.x0)).max(1);
        let kx = crate::convert::f64_to_f32(f64::from(bw) / super::i128_f64(span));
        let ky = crate::convert::f64_to_f32(f64::from(bh) / (p.view.y1 - p.view.y0).abs().max(f64::MIN_POSITIVE));
        let r = p.radius * frame.scale;
        // The points whose x can show: the view, widened by a dot's radius.
        let reach = crate::convert::f32_to_i32((f64_to_f32_safe(f64::from(r + 1.0) / f64::from(kx.max(1e-30)))).ceil());
        let lo_x = p.view.x0.saturating_sub(i64::from(reach.max(0)));
        let hi_x = p.view.x1.saturating_add(i64::from(reach.max(0)));
        let lo = series.snapshot.lower_bound(lo_x);
        let hi = series.snapshot.lower_bound(hi_x.saturating_add(1));
        let pages = series.pages.to_vec();
        let x0 = split(p.view.x0);
        let y1 = crate::convert::f64_to_f32(p.view.y1);
        let y1lo = crate::convert::f64_to_f32(p.view.y1 - f64::from(y1));
        let view: [u32; 12] = [
            x0.0,
            x0.1,
            kx.to_bits(),
            bx.to_bits(),
            y1.to_bits(),
            y1lo.to_bits(),
            ky.to_bits(),
            by.to_bits(),
            crate::convert::u32_to_f32(frame.size.0).to_bits(),
            crate::convert::u32_to_f32(frame.size.1).to_bits(),
            r.to_bits(),
            0,
        ];
        let mut blocks: Vec<(u64, [u32; 8])> = Vec::new();
        for page in &pages {
            let end = page.at.saturating_add(page.len);
            let (a, b) = (lo.max(page.at), hi.min(end));
            if a >= b {
                continue;
            }
            let count = b.saturating_sub(a);
            let n_blocks = count.div_ceil(BLOCK_POINTS);
            let slot = u32::try_from(state.runs.len()).unwrap_or(0);
            let block = BLOCK.saturating_mul(u64::from(slot).saturating_add(1));
            let lay = page.layout;
            blocks.push((
                block,
                [
                    page.len,
                    lay.cap,
                    lay.y,
                    if lay.lo == lay.y { NONE } else { lay.lo },
                    a.saturating_sub(page.at),
                    count,
                    n_blocks,
                    slot,
                ],
            ));
            state.runs.push((page.seq, block, n_blocks));
        }
        let n = state.runs.len();
        if n == 0 {
            return;
        }
        state.bufs.resize_with(n, PageBufs::default);
        let blocks_n = u64::try_from(n).unwrap_or(0).saturating_add(1);
        let (Some(pb), Some(_), Some(sb)) = (
            ctx.buffer(&mut state.params, BLOCK.saturating_mul(blocks_n)),
            ctx.buffer(&mut state.args, u64::try_from(n).unwrap_or(0).saturating_mul(16)),
            ctx.buffer(&mut state.style, 16),
        ) else {
            state.runs.clear();
            return;
        };
        ctx.write(pb, 0, bytemuck::cast_slice(&view));
        for ((block, words), (bufs, run)) in blocks.iter().zip(state.bufs.iter_mut().zip(&state.runs)) {
            ctx.write(pb, *block, bytemuck::cast_slice(words));
            let count = words.get(5).copied().unwrap_or(0);
            let ok = ctx.buffer(&mut bufs.sums, u64::from(run.2).saturating_mul(4)).is_some()
                && ctx
                    .buffer(&mut bufs.starts, u64::from(run.2).saturating_add(1).saturating_mul(4))
                    .is_some()
                && ctx
                    .buffer(&mut bufs.shown, u64::from(count).saturating_mul(4))
                    .is_some();
            if !ok {
                state.runs.clear();
                return;
            }
        }
        ctx.write(sb, 0, bytemuck::cast_slice(&rgba(p.color)));
    }

    fn compute(&self, pass: &mut ChunkCompute<'_>, state: &ScatterState, _: &Words, _: &FrameInfo) {
        let (Some(params), Some(args)) = (state.params, state.args) else {
            return;
        };
        let view = Bind::Range(params, 0, 48);
        for ((seq, block, n_blocks), bufs) in state.runs.iter().zip(&state.bufs) {
            let (Some(sums), Some(starts), Some(shown)) = (bufs.sums, bufs.starts, bufs.shown) else {
                continue;
            };
            let pp = Bind::Range(params, *block, 32);
            pass.dispatch(
                self.reduce,
                &[view, pp, Bind::Page(*seq), Bind::Buffer(sums)],
                [*n_blocks, 1, 1],
            );
            pass.dispatch(
                self.scan,
                &[pp, Bind::Buffer(sums), Bind::Buffer(starts), Bind::Buffer(args)],
                [1, 1, 1],
            );
            pass.dispatch(
                self.compact,
                &[view, pp, Bind::Page(*seq), Bind::Buffer(starts), Bind::Buffer(shown)],
                [*n_blocks, 1, 1],
            );
        }
    }

    fn draw(&self, pass: &mut ChunkDraw<'_>, state: &ScatterState, _: &Words, _: &FrameInfo) {
        let (Some(params), Some(args), Some(style)) = (state.params, state.args, state.style) else {
            return;
        };
        for (k, ((seq, block, _), bufs)) in state.runs.iter().zip(&state.bufs).enumerate() {
            let Some(shown) = bufs.shown else { continue };
            pass.draw_indirect(
                self.draw,
                &[
                    Bind::Range(params, 0, 48),
                    Bind::Range(params, *block, 32),
                    Bind::Page(*seq),
                    Bind::Buffer(shown),
                    Bind::Range(style, 0, 16),
                ],
                args,
                u64::try_from(k).unwrap_or(0).saturating_mul(16),
            );
        }
    }
}

fn f64_to_f32_safe(v: f64) -> f32 {
    crate::convert::f64_to_f32(v.clamp(-1e30, 1e30))
}
