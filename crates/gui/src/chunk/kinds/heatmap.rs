//! Heatmaps: a grid source drawn into its bounds (`heatmap.wgsl`).

use super::super::runtime::{
    Bind, BindKind, ChunkCompute, ChunkDraw, ChunkUpload, DataOp, DrawId, PipelineBuilder, Target,
};
use super::super::store::{BufferId, TextureId};
use super::super::{ChunkKind, FrameInfo};
use super::{Words, color, put_color, rgba};
use crate::color::Color;

/// Source 0's grid, its values from `min` (in `low`) to `max` (in `high`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeatmapParams {
    pub low: Color,
    pub high: Color,
    pub min: f32,
    pub max: f32,
}

impl HeatmapParams {
    #[must_use]
    pub fn words(&self) -> Words {
        let mut w = [0; 64];
        put_color(&mut w, 0, self.low);
        put_color(&mut w, 4, self.high);
        for (slot, v) in w.iter_mut().skip(8).zip([self.min.to_bits(), self.max.to_bits()]) {
            *slot = v;
        }
        w
    }

    #[must_use]
    pub fn from_words(w: &Words) -> Self {
        let at = |i: usize| w.get(i).copied().unwrap_or(0);
        Self {
            low: color(w, 0),
            high: color(w, 4),
            min: f32::from_bits(at(8)),
            max: f32::from_bits(at(9)),
        }
    }
}

pub struct Heatmap {
    draw: DrawId,
}

#[derive(Default)]
pub struct HeatmapState {
    grid: Option<TextureId>,
    style: Option<BufferId>,
}

impl ChunkKind for Heatmap {
    type Params = Words;
    type State = HeatmapState;

    fn pipelines(ctx: &mut PipelineBuilder<'_>) -> Self {
        Self {
            draw: ctx.draw(
                "gui heatmap",
                include_str!("heatmap.wgsl"),
                "vs_heat",
                "fs_heat",
                &[BindKind::Uniform, BindKind::Texture],
                Target::Main,
            ),
        }
    }

    fn update(&self, ctx: &mut ChunkUpload<'_>, state: &mut HeatmapState, params: &Words, ops: &[DataOp]) {
        let p = HeatmapParams::from_words(params);
        let Some(g) = ctx.grid(0).cloned() else { return };
        let d = g.data();
        if d.width == 0 || d.height == 0 {
            return;
        }
        if (!ops.is_empty() || state.grid.is_none())
            && let Some(t) = ctx.texture(&mut state.grid, (d.width, d.height), wgpu::TextureFormat::R32Float)
        {
            ctx.write_texture(t, bytemuck::cast_slice(&d.values), d.width.saturating_mul(4));
        }
        let mut style: Vec<u32> = Vec::with_capacity(12);
        style.extend(rgba(p.low).map(f32::to_bits));
        style.extend(rgba(p.high).map(f32::to_bits));
        style.extend([
            p.min.to_bits(),
            p.max.to_bits(),
            crate::convert::u32_to_f32(d.width).to_bits(),
            crate::convert::u32_to_f32(d.height).to_bits(),
        ]);
        if let Some(b) = ctx.buffer(&mut state.style, 48) {
            ctx.write(b, 0, bytemuck::cast_slice(&style));
        }
    }

    fn compute(&self, _: &mut ChunkCompute<'_>, _: &HeatmapState, _: &Words, _: &FrameInfo) {}

    fn draw(&self, pass: &mut ChunkDraw<'_>, state: &HeatmapState, _: &Words, _: &FrameInfo) {
        if let (Some(style), Some(grid)) = (state.style, state.grid) {
            pass.draw(self.draw, &[Bind::Buffer(style), Bind::Texture(grid)], 0..4, 0..1);
        }
    }
}
