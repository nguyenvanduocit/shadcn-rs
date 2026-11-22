//! What lies below, read back (ADR-0016's renderer, M10 S3): a backdrop
//! (CSS `backdrop-filter: blur() saturate()`) and a border blended with
//! what it covers (`mix-blend-mode: darken`, `lighten`). Neither is a
//! blend state: a blur reads its neighbours, and a separable blend mode
//! is not linear in the colour below (`darken` is `min`), so the pixels
//! below are copied out of the target first.
//!
//! **A read** ([`Read`]) is an op in a pass ([`super::Op::Read`]): the pass
//! ends, the read rectangle of its target is copied into a scratch
//! texture, a blur runs on it (halved while its sigma passes
//! [`MOST_SIGMA`], as Skia's GPU blur scales down, then one horizontal
//! and one vertical Gaussian pass, edges mirrored as Chrome's backdrop
//! filter mirrors them), and the pass begins again over what it drew. The
//! draw after it is a quad record ([`record::Read`]) that reads the result
//! inside the item's shape: saturated (a backdrop), or blended per channel
//! with its colour (Compositing 1's `B(cb, cs)`, exact, in the target's
//! encoded values as browsers blend).
//!
//! **What is below** is what the target holds when the read runs: the
//! canvas, or the layer of the nearest effect drawn aside (Filter Effects
//! 2's backdrop root). A blur reads all of its box, so canvas damage that
//! meets a blurred box grows to hold the box whole, in one part
//! ([`GpuRenderer::with_reads`]): every pixel it reads is drawn again
//! before it, never last frame's result. A blend reads only the pixel it
//! draws, which any damage part redraws first.

use super::damage::Rect;
use super::record::{self, READ_BACKDROP, READ_DARKEN, READ_LIGHTEN};
use super::{Extra, GpuRenderer, Op, Texture, phys, push_op, radii, rgba};
use crate::scene::Item;
use crate::shape::Blend;

/// The largest sigma (texels) a blur pass runs at: past it, the read is
/// halved first (Skia's `kMaxSigma` is 4 as well).
pub(super) const MOST_SIGMA: f32 = 4.0;
/// The most halvings: a 32× smaller copy.
const MOST_LEVELS: u32 = 5;
/// Bytes of one pass's parameters (a `Step` in `read.wgsl`).
const STEP: u64 = 32;

/// One read of a frame: the target's pixels `rect` covers (window,
/// physical), and the blur they get.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Read {
    pub rect: Rect,
    /// The blur's sigma at the scaled-down copy, 0 for none.
    pub sigma: f32,
    /// How many times the copy is halved before it is blurred.
    pub levels: u32,
}

impl Read {
    /// The scratch texture its result lands in: the copy goes to the first,
    /// each pass then swaps (two blur passes swap back).
    pub(super) fn result(&self) -> u8 {
        u8::from(self.levels % 2 == 1)
    }

    fn size(&self) -> (u32, u32) {
        let w = u32::try_from(self.rect.x1.saturating_sub(self.rect.x0)).unwrap_or(0);
        let h = u32::try_from(self.rect.y1.saturating_sub(self.rect.y0)).unwrap_or(0);
        (w, h)
    }

    /// Its size after `n` halvings.
    fn size_at(&self, n: u32) -> (u32, u32) {
        let (mut w, mut h) = self.size();
        for _ in 0..n {
            (w, h) = (w.div_ceil(2), h.div_ceil(2));
        }
        (w.max(1), h.max(1))
    }

    /// Its passes: the halvings, then the two blur passes if it blurs.
    fn steps(&self) -> u32 {
        self.levels.saturating_add(if self.sigma > 0.0 { 2 } else { 0 })
    }
}

/// How a Gaussian of `sigma` (physical) is run: the halvings first, and
/// the sigma left for the passes at that size. A halving is a 2×2 box
/// whose variance (`w²/12`, `w` its full-size width) is part of the blur,
/// so the passes run only what it leaves.
pub(super) fn levels(sigma: f32) -> (u32, f32) {
    if sigma <= 0.0 {
        return (0, 0.0);
    }
    let mut n = 0;
    let mut added = 0.0_f32;
    while n < MOST_LEVELS && sigma / 2.0_f32.powi(i32::try_from(n).unwrap_or(0)) > MOST_SIGMA {
        let w = 2.0_f32.powi(i32::try_from(n).unwrap_or(0).saturating_add(1));
        added += w * w / 12.0;
        n = n.saturating_add(1);
    }
    let left = (sigma * sigma - added).max(0.0).sqrt();
    (n, left / 2.0_f32.powi(i32::try_from(n).unwrap_or(0)))
}

/// The scratch textures reads land in, the passes' parameters, and the
/// bind groups the blur pipeline reads them through, kept across frames.
pub(super) struct Reads {
    textures: [Option<Texture>; 2],
    /// Reading texture 0 and 1, each with the parameters' buffer.
    groups: [Option<wgpu::BindGroup>; 2],
    /// The buffer generation the groups were made with.
    made: u64,
    steps: super::Scratch,
    /// Where each read's passes start among the frame's.
    first: Vec<u32>,
    words: Vec<u8>,
}

impl Reads {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        Self {
            textures: [None, None],
            groups: [None, None],
            made: u64::MAX,
            steps: super::Scratch::new(device, "gui reads", wgpu::BufferUsages::UNIFORM, 256 * 4),
            first: Vec::new(),
            words: Vec::new(),
        }
    }

    /// Scratch texture `k`, when there is one.
    pub(super) fn texture(&self, k: u8) -> Option<&Texture> {
        self.textures.get(usize::from(k)).and_then(Option::as_ref)
    }

    /// The bytes the scratch textures take.
    pub(super) fn bytes(&self) -> usize {
        self.textures.iter().flatten().map(Texture::bytes).sum()
    }
}

/// Window pixels `r` (x0, y0, x1, y1) as a [`Rect`] within `visible`.
fn within(r: [f32; 4], visible: [f32; 4]) -> Option<Rect> {
    let v = Rect::covering(visible)?;
    let r = Rect::covering(r)?.intersect(&v);
    (!r.is_empty()).then_some(r)
}

impl GpuRenderer {
    /// The draws of `item`, a read (a backdrop, a blended border) of a
    /// chunk whose state slot is `slot` and whose space is moved by `off`
    /// (physical), in a target whose `visible` pixels show: the read,
    /// then the quad showing it. Nothing when it covers no visible pixel.
    pub(super) fn read_ops(
        &mut self,
        ops: &mut Vec<Op>,
        item: &Item,
        s: f32,
        off: (f32, f32),
        slot: u32,
        visible: [f32; 4],
    ) {
        let (bounds, radius, curve, inner, mode, color, sigma, saturate) = match item {
            Item::Backdrop {
                bounds,
                radius,
                curve,
                backdrop,
            } => (
                *bounds,
                *radius,
                *curve,
                None,
                READ_BACKDROP,
                [0.0; 4],
                backdrop.blur.max(0.0) * s,
                backdrop.saturate.max(0.0),
            ),
            Item::Blended {
                outer,
                outer_radius,
                inner,
                inner_radius,
                curve,
                color,
                blend,
            } => {
                let mode = match blend {
                    Blend::Darken => READ_DARKEN,
                    Blend::Lighten => READ_LIGHTEN,
                    Blend::Normal => return,
                };
                let cut = (!inner.is_empty()).then(|| (phys(*inner, s), radii(*inner_radius, *inner, s)));
                (*outer, *outer_radius, *curve, cut, mode, rgba(*color), 0.0, 1.0)
            }
            _ => return,
        };
        let rect = phys(bounds, s);
        let window = [
            rect[0] + off.0,
            rect[1] + off.1,
            rect[0] + rect[2] + off.0,
            rect[1] + rect[3] + off.1,
        ];
        let Some(area) = within(window, visible) else { return };
        let (n, sigma) = levels(sigma);
        let read = Read {
            rect: area,
            sigma,
            levels: n,
        };
        let (vw, vh) = read.size_at(n);
        let mut words = Vec::with_capacity(27);
        record::Read {
            slot,
            mode,
            color,
            rect,
            radii: radii(radius, bounds, s),
            inner,
            origin: [crate::convert::i32_to_f32(area.x0), crate::convert::i32_to_f32(area.y0)],
            level: 2.0_f32.powi(i32::try_from(n).unwrap_or(0)),
            saturate,
            curve,
            valid: [crate::convert::u32_to_f32(vw), crate::convert::u32_to_f32(vh)],
        }
        .push(&mut words);
        let Some(at) = self.transient_record(&words) else {
            return;
        };
        let k = u32::try_from(self.fr.reads.len()).unwrap_or(u32::MAX);
        self.fr.reads.push(read);
        ops.push(Op::Read(k));
        push_op(
            ops,
            Op::Quads {
                extra: Extra::Read(read.result()),
                instances: at..at.saturating_add(1),
            },
        );
    }

    /// The scratch textures large enough for this frame's reads, and every
    /// pass's parameters written: before the frame's passes are encoded.
    pub(super) fn prepare_reads(&mut self) -> u64 {
        let reads = &self.fr.reads;
        if reads.is_empty() {
            return 0;
        }
        let stride = usize::try_from(self.uniform_stride).unwrap_or(256);
        let r = &mut self.reads;
        r.first.clear();
        r.words.clear();
        let (mut w, mut h) = (1_u32, 1_u32);
        let mut n = 0_u32;
        for read in reads {
            let (rw, rh) = read.size();
            (w, h) = (w.max(rw), h.max(rh));
            r.first.push(n);
            let mut step = |dir: [f32; 2], sigma: f32, size: (u32, u32)| {
                let at = r.words.len();
                r.words.resize(at.saturating_add(stride), 0);
                let values = [
                    dir[0],
                    dir[1],
                    sigma,
                    0.0,
                    crate::convert::u32_to_f32(size.0),
                    crate::convert::u32_to_f32(size.1),
                    0.0,
                    0.0,
                ];
                for (d, v) in r.words.iter_mut().skip(at).zip(super::floats_to_bytes(&values)) {
                    *d = v;
                }
            };
            for level in 0..read.levels {
                // Halving the copy at `level`: it reads that size.
                step([0.0, 0.0], 0.0, read.size_at(level));
            }
            if read.sigma > 0.0 {
                let at = read.size_at(read.levels);
                step([1.0, 0.0], read.sigma, at);
                step([0.0, 1.0], read.sigma, at);
            }
            n = n.saturating_add(read.steps());
        }
        let bytes = r.steps.update(&self.device, &self.queue, &r.words);
        // Grown to the largest read, rounded so a size that moves a little
        // keeps its textures.
        let round = |v: u32| v.div_ceil(256).saturating_mul(256).max(256);
        let size = (round(w), round(h));
        for k in 0..2 {
            let fits = r
                .textures
                .get(k)
                .and_then(Option::as_ref)
                .is_some_and(|t| t.size.0 >= size.0 && t.size.1 >= size.1);
            if !fits {
                if let Some(t) = r.textures.get_mut(k) {
                    *t = Some(super::texture(
                        &self.device,
                        &self.queue,
                        &self.layouts.extra,
                        size,
                        self.format,
                        None,
                    ));
                }
                r.made = u64::MAX;
                self.bind_groups_made = self.bind_groups_made.saturating_add(1);
            }
        }
        if r.made != r.steps.generation {
            for k in 0..2 {
                let Some(t) = r.textures.get(k).and_then(Option::as_ref) else {
                    continue;
                };
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gui read"),
                    layout: &self.layouts.read,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&t.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &r.steps.buffer,
                                offset: 0,
                                size: wgpu::BufferSize::new(STEP),
                            }),
                        },
                    ],
                });
                if let Some(g) = r.groups.get_mut(k) {
                    *g = Some(group);
                }
                self.bind_groups_made = self.bind_groups_made.saturating_add(1);
            }
            r.made = r.steps.generation;
        }
        bytes
    }

    /// Read `k` of this frame: `source` (a target whose top left lies at
    /// `origin` in the window, `size` texels) copied into the scratch
    /// texture and blurred, where `scissor` (target pixels) lets anything
    /// show. Encoded between two render passes of the target.
    pub(super) fn encode_read(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        k: u32,
        origin: (i32, i32),
        size: (u32, u32),
        scissor: Option<Rect>,
    ) {
        let Some(read) = usize::try_from(k).ok().and_then(|k| self.fr.reads.get(k)).copied() else {
            return;
        };
        let target = Rect {
            x0: 0,
            y0: 0,
            x1: i32::try_from(size.0).unwrap_or(i32::MAX),
            y1: i32::try_from(size.1).unwrap_or(i32::MAX),
        };
        let shifted = Rect {
            x0: read.rect.x0.saturating_sub(origin.0),
            y0: read.rect.y0.saturating_sub(origin.1),
            x1: read.rect.x1.saturating_sub(origin.0),
            y1: read.rect.y1.saturating_sub(origin.1),
        };
        let mut from = shifted.intersect(&target);
        if let Some(s) = scissor {
            if from.intersect(&s).is_empty() {
                // Nothing it draws shows in this part.
                return;
            }
            if read.sigma <= 0.0 && read.levels == 0 {
                // A blend reads only the pixels it draws.
                from = from.intersect(&s);
            }
        }
        if from.is_empty() {
            return;
        }
        let Some(first) = self.reads.texture(0) else { return };
        let (Ok(x), Ok(y), Ok(w), Ok(h), Ok(dx), Ok(dy)) = (
            u32::try_from(from.x0),
            u32::try_from(from.y0),
            u32::try_from(from.x1.saturating_sub(from.x0)),
            u32::try_from(from.y1.saturating_sub(from.y0)),
            u32::try_from(from.x0.saturating_sub(shifted.x0)),
            u32::try_from(from.y0.saturating_sub(shifted.y0)),
        ) else {
            return;
        };
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: source,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &first.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: dx, y: dy, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let first_step = self.reads.first.get(usize::try_from(k).unwrap_or(usize::MAX)).copied();
        let Some(first_step) = first_step else { return };
        let stride = self.uniform_stride;
        // Each pass reads one scratch texture and writes the other.
        let mut at = 0_u8;
        let mut pass = |step: u32, out: (u32, u32), at: u8| {
            let (Some(group), Some(dst)) = (
                self.reads.groups.get(usize::from(at)).and_then(Option::as_ref),
                self.reads.texture(at ^ 1),
            ) else {
                return;
            };
            let offset = u32::try_from(u64::from(step).saturating_mul(stride)).unwrap_or(0);
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gui read pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &dst.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            rp.set_viewport(
                0.0,
                0.0,
                crate::convert::u32_to_f32(out.0),
                crate::convert::u32_to_f32(out.1),
                0.0,
                1.0,
            );
            rp.set_scissor_rect(0, 0, out.0, out.1);
            rp.set_pipeline(&self.pipelines.read);
            rp.set_bind_group(0, group, &[offset]);
            rp.draw(0..3, 0..1);
        };
        let mut step = first_step;
        for level in 0..read.levels {
            pass(step, read.size_at(level.saturating_add(1)), at);
            at ^= 1;
            step = step.saturating_add(1);
        }
        if read.sigma > 0.0 {
            let out = read.size_at(read.levels);
            pass(step, out, at);
            pass(step.saturating_add(1), out, at ^ 1);
        }
    }

    /// Canvas damage `rects` grown so that each blurred box it meets lies
    /// whole inside one of them: a blur reads all of its box, and what a
    /// part does not draw again is last frame's result, not what is below.
    pub(super) fn with_reads(&self, scene: &crate::scene::Scene, root: &[usize], mut rects: Vec<Rect>) -> Vec<Rect> {
        if self.blurring == 0 || rects.is_empty() {
            return rects;
        }
        let s = scene.scale;
        let mut boxes = Vec::new();
        for &i in root {
            let Some(c) = scene.chunks.get(i) else { continue };
            let Some(k) = self.kept.get(&c.id) else { continue };
            if k.blurs.is_empty() {
                continue;
            }
            let off = super::frame::offset_of(&scene.properties, c.state.transform, s);
            for b in &k.blurs {
                if let Some(r) = Rect::covering([b[0] + off.0, b[1] + off.1, b[2] + off.0, b[3] + off.1]) {
                    boxes.push(r);
                }
            }
        }
        // Until no box meets a rectangle without lying in it (each round
        // joins rectangles, so it ends).
        for _ in 0..64 {
            let mut grown = false;
            for b in &boxes {
                let meets: Vec<usize> = (0..rects.len())
                    .filter(|&j| rects.get(j).is_some_and(|r| !r.intersect(b).is_empty()))
                    .collect();
                let held = meets.len() == 1
                    && meets
                        .first()
                        .and_then(|&j| rects.get(j))
                        .is_some_and(|r| r.union(b) == *r);
                if meets.is_empty() || held {
                    continue;
                }
                let mut joined = *b;
                for &j in meets.iter().rev() {
                    joined = joined.union(&rects.remove(j));
                }
                rects.push(joined);
                grown = true;
            }
            if !grown {
                break;
            }
        }
        rects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_blur_runs_at_full_size() {
        assert_eq!(levels(0.0), (0, 0.0));
        assert_eq!(levels(3.0), (0, 3.0));
    }

    #[test]
    fn a_large_blur_is_halved_and_keeps_its_variance() {
        // `backdrop-blur-2xl` at 2×: sigma 80.
        let (n, left) = levels(80.0);
        assert_eq!(n, 5);
        assert!(left <= MOST_SIGMA, "{left}");
        // The halvings' variance and the passes' add up to the blur's.
        let mut added = 0.0;
        for i in 0..n {
            let w = 2.0_f32.powi(i32::try_from(i).unwrap() + 1);
            added += w * w / 12.0;
        }
        let scale = 2.0_f32.powi(i32::try_from(n).unwrap());
        let total = added + (left * scale).powi(2);
        assert!((total - 6400.0).abs() < 1.0, "{total}");
    }

    #[test]
    fn a_read_lands_where_its_passes_leave_it() {
        let r = |levels, sigma| Read {
            rect: Rect {
                x0: 0,
                y0: 0,
                x1: 101,
                y1: 50,
            },
            sigma,
            levels,
        };
        assert_eq!(r(0, 0.0).result(), 0);
        assert_eq!(r(0, 2.0).result(), 0);
        assert_eq!(r(3, 2.0).result(), 1);
        assert_eq!(r(1, 2.0).size_at(1), (51, 25));
        assert_eq!(r(3, 2.0).steps(), 5);
    }
}
