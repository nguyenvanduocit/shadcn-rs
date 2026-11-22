//! The data chunk kind a canvas tile is drawn by: a tile texture over a
//! quadrilateral (a tiling scaled and turned to where the camera has it),
//! in the canvas' place in the order, under its clip, cut to the canvas.

use crate::chunk::{
    Bind, BindKind, BufferId, ChunkCompute, ChunkDraw, ChunkKind, ChunkUpload, DataOp, DrawId, FrameInfo,
    PipelineBuilder, Target, TextureId, Words,
};

/// A tile: texture `tex` with its corners (top left, top right, bottom
/// left, bottom right) at `corners` and drawn only inside `cut` (x0, y0,
/// x1, y1), all device pixels from the item's bounds' corner.
pub(crate) fn words(tex: TextureId, corners: [[f32; 2]; 4], cut: [f32; 4]) -> Words {
    let mut w = [0; 64];
    let values = std::iter::once(tex.0)
        .chain([0, 0, 0])
        .chain(corners.into_iter().flatten().map(f32::to_bits))
        .chain(cut.map(f32::to_bits));
    for (slot, v) in w.iter_mut().zip(values) {
        *slot = v;
    }
    w
}

pub(crate) struct Tiles {
    draw: DrawId,
}

#[derive(Default)]
pub(crate) struct TileState {
    params: Option<BufferId>,
}

impl ChunkKind for Tiles {
    type Params = Words;
    type State = TileState;

    fn pipelines(ctx: &mut PipelineBuilder<'_>) -> Self {
        Self {
            draw: ctx.draw(
                "gui canvas tile",
                include_str!("tile.wgsl"),
                "vs_tile",
                "fs_tile",
                &[BindKind::Uniform, BindKind::Texture],
                Target::Main,
            ),
        }
    }

    fn update(&self, ctx: &mut ChunkUpload<'_>, state: &mut TileState, params: &Words, _: &[DataOp]) {
        if let Some(b) = ctx.buffer(&mut state.params, 64) {
            ctx.write(b, 0, bytemuck::cast_slice(params.get(..16).unwrap_or(&[])));
        }
    }

    fn compute(&self, _: &mut ChunkCompute<'_>, _: &TileState, _: &Words, _: &FrameInfo) {}

    fn draw(&self, pass: &mut ChunkDraw<'_>, state: &TileState, params: &Words, _: &FrameInfo) {
        if let Some(b) = state.params {
            let tex = TextureId(params.first().copied().unwrap_or(u32::MAX));
            pass.draw(self.draw, &[Bind::Buffer(b), Bind::Texture(tex)], 0..4, 0..1);
        }
    }
}
