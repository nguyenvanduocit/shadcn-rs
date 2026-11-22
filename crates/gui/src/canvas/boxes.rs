//! The data chunk kind a canvas draws its small objects by (ADR-0017's
//! level of detail): under [`super::LOD_BOX`] pixels an object is a box of
//! its colour, and a run of such boxes is one instanced draw in the
//! canvas' place in the order, under its clip, rather than one path each.

use crate::chunk::{
    Bind, BindKind, BufferId, ChunkCompute, ChunkDraw, ChunkKind, ChunkUpload, DataOp, DrawId, FrameInfo,
    PipelineBuilder, Target, Words,
};
use crate::color::Color;

/// Floats a box: x, y, width, height (device pixels from the item's
/// bounds), and its colour (sRGB, straight alpha).
pub(crate) const BOX: usize = 8;

/// The params of `count` boxes.
pub(crate) fn words(count: u32) -> Words {
    let mut w = [0; 64];
    if let Some(c) = w.first_mut() {
        *c = count;
    }
    w
}

/// Box `rect` (device pixels from the item's bounds) of `color`, into `out`.
pub(crate) fn push(out: &mut Vec<f32>, rect: [f32; 4], color: Color) {
    out.extend(rect);
    out.extend([color.r, color.g, color.b, color.a]);
}

pub(crate) struct Boxes {
    draw: DrawId,
}

#[derive(Default)]
pub(crate) struct BoxState {
    params: Option<BufferId>,
    boxes: Option<BufferId>,
    count: u32,
}

impl ChunkKind for Boxes {
    type Params = Words;
    type State = BoxState;

    fn pipelines(ctx: &mut PipelineBuilder<'_>) -> Self {
        Self {
            draw: ctx.draw(
                "gui canvas boxes",
                include_str!("boxes.wgsl"),
                "vs_box",
                "fs_box",
                &[BindKind::Uniform, BindKind::Storage],
                Target::Main,
            ),
        }
    }

    fn update(&self, ctx: &mut ChunkUpload<'_>, state: &mut BoxState, params: &Words, _: &[DataOp]) {
        if let Some(b) = ctx.buffer(&mut state.params, 16) {
            ctx.write(b, 0, bytemuck::cast_slice(params.get(..4).unwrap_or(&[])));
        }
        let data = ctx.bytes(0).map(<[u8]>::to_vec).unwrap_or_default();
        state.count = params.first().copied().unwrap_or(0);
        let len = u64::try_from(data.len()).unwrap_or(0).max(32);
        if let Some(b) = ctx.buffer(&mut state.boxes, len) {
            ctx.write(b, 0, &data);
        }
    }

    fn compute(&self, _: &mut ChunkCompute<'_>, _: &BoxState, _: &Words, _: &FrameInfo) {}

    fn draw(&self, pass: &mut ChunkDraw<'_>, state: &BoxState, _: &Words, _: &FrameInfo) {
        if let (Some(p), Some(b)) = (state.params, state.boxes)
            && state.count > 0
        {
            pass.draw(self.draw, &[Bind::Buffer(p), Bind::Buffer(b)], 0..4, 0..state.count);
        }
    }
}
