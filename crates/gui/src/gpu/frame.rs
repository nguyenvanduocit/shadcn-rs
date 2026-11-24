//! A frame on the GPU (ADR-0016): what each effect needs, where each chunk
//! draws, what changed since the canvas was last drawn, and the passes that
//! draw it.
//!
//! **Effects.** An effect gets a layer of its own only when its result
//! cannot be had otherwise: a filter or a mask (they read the group's
//! pixels as one), an opacity below one over primitives that overlap
//! (faded one by one, the overlap would show), a scale or turn of what
//! cannot be drawn through it (a shadow, an image, a colour glyph,
//! text-through). Otherwise its opacity is multiplied into its primitives
//! ("folded", the property table's effect entry) and they draw straight,
//! a transformed one through its matrix (quads as paths, text from its
//! curves). Which of these an effect is follows what it holds each frame;
//! a layer whose content is what it was is composited as kept, so a fade
//! or a move of a layered group draws none of it again.
//!
//! **Damage.** Each entry of the canvas (a chunk, a layer's composite) has a
//! signature of everything that decides its pixels and the rectangle it
//! covers; an entry whose signature or rectangle changed, appeared or went
//! is damage, old and new rectangles both. The rectangles are merged
//! ([`super::damage::group`]) and only they are drawn, scissored, over what
//! the canvas holds: the background, then every entry that meets them.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::chunks::{Seg, line_and_stops};
use super::clips::{ClipTree, ENTRY, IDENTITY, compose, intersect, mapped};
use super::damage::{Rect, group};
use super::path::{self, Paint};
use super::record;
use super::words::Table;
use super::{Extra, GpuRenderer, Op, SLUG, SlugKey, Texture, glyph_matrix, icon_parts, phys, push_op, radii, rgba};
use crate::color::Color;
use crate::figure::{Path, PathStyle};
use crate::geometry::Bounds;
use crate::image::Pixels;
use crate::scene::{Changed, ChunkId, Clip, Drawn, Gradient, Item, Marks, PixelRect, Properties, Scene};
use crate::style::{Corners, Filter};
use crate::text::gamma::MaskGamma;
use crate::text::{GlyphKey, Glyphs};

/// Layers are kept while used in the last this many frames…
const LAYER_FRAMES: u64 = 120;
/// …and while all of them take at most this many bytes.
const LAYER_BUDGET: usize = 64 << 20;

/// This frame's tables: the clips, stops and curve glyphs paths and curve
/// text read (drawn this frame only).
pub(super) struct FrameData {
    /// Clip entries as [`super::clips`] lays them out, each mapped from
    /// its draw's target and naming the entry of the next clip of its
    /// effective chain; clip 0 is none.
    pub pclips: Vec<f32>,
    pclip_ids: foldhash::HashMap<(u32, u32), u32>,
    pub stops: Vec<f32>,
    pub slugs: Vec<f32>,
    /// What lies below, read back by this frame's passes (`read.rs`).
    pub reads: Vec<super::read::Read>,
}

impl Default for FrameData {
    fn default() -> Self {
        Self {
            pclips: vec![0.0; ENTRY],
            pclip_ids: foldhash::HashMap::default(),
            stops: Vec::new(),
            slugs: Vec::new(),
            reads: Vec::new(),
        }
    }
}

impl FrameData {
    /// Empty for a new frame, its memory kept.
    pub(super) fn reset(&mut self) {
        self.pclips.clear();
        self.pclips.resize(ENTRY, 0.0);
        self.pclip_ids.clear();
        self.stops.clear();
        self.slugs.clear();
        self.reads.clear();
    }

    /// The entry of clip `id`'s walk for a target in the space of
    /// transform `until`, which `to_canvas` maps to the canvas: its walk
    /// followed up to the first clip already written for that target,
    /// then each clip written from there down, after its next.
    pub(super) fn pclip(
        &mut self,
        tree: &ClipTree,
        id: u32,
        until: u32,
        to_canvas: impl FnOnce() -> Option<[f32; 6]>,
    ) -> u32 {
        let mut walk = Vec::new();
        let mut at = id;
        let mut next = loop {
            if at == 0 {
                break 0;
            }
            if let Some(e) = self.pclip_ids.get(&(at, until)) {
                break *e;
            }
            walk.push(at);
            at = tree.next(at);
        };
        if walk.is_empty() {
            return next;
        }
        let to = to_canvas();
        for c in walk.into_iter().rev() {
            let mut entry = tree.entry_in(c, to);
            entry[16] = f32::from_bits(next);
            next = u32::try_from(self.pclips.len() / ENTRY).unwrap_or(0);
            self.pclips.extend_from_slice(&entry);
            self.pclip_ids.insert((c, until), next);
        }
        next
    }
}

/// The property tables (`props` in `shaders.wgsl`), kept across frames:
/// each state slot's transform, clip and effect, each transform's offset,
/// each clip's rectangle and radii, each effect's folded opacity
/// (physical). A frame planned whole writes them whole; a frame planned
/// from what changed writes the entries that did, and only they go up.
#[derive(Clone, Default, PartialEq)]
pub(super) struct Tables {
    slot_states: Vec<[u32; 4]>,
    transforms: Vec<[f32; 2]>,
    clips: Vec<[f32; ENTRY]>,
    effects: Vec<f32>,
    /// Where each table started when last laid out whole (vec4s), and the
    /// slots it was laid out for: entries are patched in place while the
    /// shape holds.
    laid: Option<([u32; 4], u32)>,
}

impl Tables {
    /// Where each table starts (vec4s) for `slots` state slots: slots,
    /// transforms, clips, effects, each rounded up, so that an entry more
    /// or less moves no other table.
    fn bases(&self, slots: u32) -> ([u32; 4], u32) {
        let n = u32::try_from(self.slot_states.len())
            .unwrap_or(u32::MAX)
            .max(slots)
            .div_ceil(256)
            .saturating_mul(256);
        let len = |v: usize| {
            u32::try_from(v)
                .unwrap_or(u32::MAX)
                .div_ceil(64)
                .max(1)
                .saturating_mul(64)
        };
        let t = len(self.transforms.len());
        let c = len(self.clips.len());
        (
            [
                0,
                n,
                n.saturating_add(t),
                n.saturating_add(t).saturating_add(c.saturating_mul(PCLIP_VEC4S)),
            ],
            n,
        )
    }

    /// The tables as words, laid out at `bases` with `n` slots.
    fn words(&self, bases: [u32; 4], n: u32) -> Vec<u32> {
        let mut words = Vec::with_capacity(usize::try_from(bases[3]).unwrap_or(0).saturating_mul(4));
        for i in 0..usize::try_from(n).unwrap_or(0) {
            words.extend(self.slot_states.get(i).copied().unwrap_or([0; 4]));
        }
        let pad = |words: &mut Vec<u32>, to: u32| {
            let to = usize::try_from(to).unwrap_or(0).saturating_mul(4);
            words.resize(to.max(words.len()), 0);
        };
        for [x, y] in &self.transforms {
            words.extend([x.to_bits(), y.to_bits(), 0, 0]);
        }
        pad(&mut words, bases[2]);
        for c in &self.clips {
            words.extend(c.map(f32::to_bits));
        }
        pad(&mut words, bases[3]);
        for a in &self.effects {
            words.extend([a.to_bits(), 0, 0, 0]);
        }
        let e = u32::try_from(self.effects.len())
            .unwrap_or(u32::MAX)
            .div_ceil(64)
            .max(1)
            .saturating_mul(64);
        pad(&mut words, bases[3].saturating_add(e));
        words
    }

    /// Bring `table` up to these tables for `slots` slots: the entries
    /// patched, or all of it when its shape changed; the bytes written, and
    /// where each table starts.
    fn upload(&mut self, table: &mut Table, device: &wgpu::Device, queue: &wgpu::Queue, slots: u32) -> (u64, [u32; 4]) {
        let (bases, n) = self.bases(slots);
        if self.laid == Some((bases, n)) {
            return (table.flush(queue), bases);
        }
        let words = self.words(bases, n);
        self.laid = Some((bases, n));
        (table.upload(device, queue, &words), bases)
    }

    /// Write `words` at vec4 `at` of the table laid out, or lay it out
    /// again when it does not hold it.
    fn patch(&mut self, table: &mut Table, at: Option<u32>, words: &[u32]) {
        let Some(at) = at else { return };
        if self.laid.is_some() && !table.patch(usize::try_from(at).unwrap_or(usize::MAX).saturating_mul(4), words) {
            self.laid = None;
        }
    }

    /// Lay the tables out again at the next upload (what the GPU holds is
    /// not what they say).
    fn relay(&mut self) {
        self.laid = None;
    }

    fn base(&self, k: usize) -> Option<u32> {
        self.laid.and_then(|(b, _)| b.get(k).copied())
    }

    /// State slot `slot` paints under `state`.
    pub(super) fn set_slot(&mut self, table: &mut Table, slot: u32, state: [u32; 4]) {
        let Ok(i) = usize::try_from(slot) else { return };
        if self.slot_states.len() <= i {
            self.slot_states.resize(i.saturating_add(1), [0; 4]);
        }
        if let Some(s) = self.slot_states.get_mut(i) {
            *s = state;
        }
        let inside = self.laid.is_some_and(|(_, n)| slot < n);
        if inside {
            self.patch(table, Some(slot), &state);
        } else {
            self.laid = None;
        }
    }

    fn set_transform(&mut self, table: &mut Table, id: u32, offset: [f32; 2]) {
        let Some(t) = usize::try_from(id).ok().and_then(|i| self.transforms.get_mut(i)) else {
            return;
        };
        *t = offset;
        let at = self.base(1).map(|b| b.saturating_add(id));
        self.patch(table, at, &[offset[0].to_bits(), offset[1].to_bits(), 0, 0]);
    }

    fn set_clip(&mut self, table: &mut Table, id: u32, clip: [f32; ENTRY]) {
        let Some(c) = usize::try_from(id).ok().and_then(|i| self.clips.get_mut(i)) else {
            return;
        };
        *c = clip;
        let at = self.base(2).map(|b| b.saturating_add(id.saturating_mul(PCLIP_VEC4S)));
        self.patch(table, at, &clip.map(f32::to_bits));
    }

    fn set_effect(&mut self, table: &mut Table, id: u32, fold: f32) {
        let Some(e) = usize::try_from(id).ok().and_then(|i| self.effects.get_mut(i)) else {
            return;
        };
        *e = fold;
        let at = self.base(3).map(|b| b.saturating_add(id));
        self.patch(table, at, &[fold.to_bits(), 0, 0, 0]);
    }

    /// Every table written from `scene` and `plan` (a frame planned whole),
    /// to go up whole.
    fn write_whole(
        &mut self,
        scene: &Scene,
        plan: &Plan,
        kept: &foldhash::HashMap<ChunkId, super::chunks::Kept>,
        tree: &ClipTree,
    ) {
        let props = &scene.properties;
        let s = scene.scale;
        self.transforms = props.transforms.iter().map(|t| transform_entry(t, s)).collect();
        self.clips = tree.entries().collect();
        self.effects = plan.fx.iter().map(|f| f.fold).collect();
        for c in &scene.chunks {
            if let Some(k) = kept.get(&c.id) {
                let Ok(i) = usize::try_from(k.slot) else { continue };
                if self.slot_states.len() <= i {
                    self.slot_states.resize(i.saturating_add(1), [0; 4]);
                }
                if let Some(slot) = self.slot_states.get_mut(i) {
                    *slot = [c.state.transform, c.state.clip, c.state.effect, 0];
                }
            }
        }
        self.laid = None;
    }
}

fn transform_entry(t: &crate::scene::TransformNode, s: f32) -> [f32; 2] {
    [t.offset.0 * s, t.offset.1 * s]
}

use super::clips::ENTRY_VEC4S as PCLIP_VEC4S;

/// A text-through mask to draw: into chunk `chunk`'s mask `k`.
pub(super) struct MaskJob {
    pub chunk: ChunkId,
    pub k: u32,
    pub origin: (i32, i32),
    pub size: (u32, u32),
    pub ops: Vec<Op>,
}

/// A layer kept: its texture and the signature of what it holds.
pub(super) struct CachedLayer {
    pub texture: Texture,
    content: u64,
    used: u64,
}

/// What a frame decided for an effect.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fx {
    layered: bool,
    /// The opacity of it and of the effects above it up to the first with
    /// a layer, multiplied into what it holds.
    fold: f32,
    /// Shows nothing (no opacity, no region).
    skip: bool,
    /// Its own opacity is below one and folded into what it holds.
    folded: bool,
}

impl Default for Fx {
    fn default() -> Self {
        Self {
            layered: false,
            fold: 1.0,
            skip: false,
            folded: false,
        }
    }
}

/// Where a chunk draws: into the canvas (effect 0) or a layer, from its
/// kept records, or through a matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Route {
    Skip,
    Quads { target: u32 },
    Matrix { target: u32, m: [f32; 6] },
}

impl Route {
    /// The layer (0: the canvas) it draws into, if it draws.
    fn target(self) -> Option<u32> {
        match self {
            Self::Skip => None,
            Self::Quads { target } | Self::Matrix { target, .. } => Some(target),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Chunk(usize),
    Layer(u32),
}

/// Where an entry is: the canvas's at a position, or in a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryAt {
    None,
    Root(usize),
    Layer(u32),
}

struct LayerPlan {
    entries: Vec<Entry>,
    region: PixelRect,
    /// Its map into the layer it composites into (or the canvas).
    m: Option<[f32; 6]>,
    content: u64,
    /// The layer it composites into (0: the canvas), and where its entry is.
    outer: u32,
    place: EntryAt,
}

/// A frame's decisions, kept across frames: a frame planned from what
/// changed ([`GpuRenderer::replan`]) visits only the chunks, effects and
/// layers that it reaches through the indexes here.
pub(super) struct Plan {
    fx: Vec<Fx>,
    routes: Vec<Route>,
    root: Vec<Entry>,
    layers: foldhash::HashMap<u32, LayerPlan>,
    /// The canvas's entries.
    shown: Vec<Shown>,
    /// Each chunk's effects, innermost first, and the chunks under each
    /// effect (anywhere in their chains).
    chains: Vec<Vec<u32>>,
    members: Vec<Vec<usize>>,
    /// Where each chunk's entry is.
    place: Vec<EntryAt>,
    /// The chunks planned: their ids and states.
    ids: Vec<ChunkId>,
    states: Vec<crate::scene::State>,
    /// The chunks drawn under each transform and effect (their state).
    by_transform: foldhash::HashMap<u32, Vec<usize>>,
    by_effect: foldhash::HashMap<u32, Vec<usize>>,
    /// The layers holding chunks drawn under each clip, drawn again when
    /// its revision changes, and the canvas's chunk entries under each,
    /// keyed again ([`Shown::rev`]): a clip changing signs no chunk.
    clip_layers: foldhash::HashMap<u32, Vec<u32>>,
    by_clip: foldhash::HashMap<u32, Vec<usize>>,
    /// The chunks showing canvases, and what their tiles were at.
    canvases: Vec<(usize, u64)>,
    /// Effects folded (devtools).
    folded: usize,
    /// What it was planned for.
    scale: u32,
    viewport: PixelRect,
    /// Chunks, effects, layers, transforms and clips met in a frame.
    seen: Marks,
    seen_fx: Marks,
    seen_t: Marks,
}

/// A canvas entry as planned: its key, the signature of what it draws
/// (its own and the key before it), the revision of the clip chain it
/// draws under ([`ClipTree::rev`]) and the pixels it may touch, cut to
/// what that chain keeps. A change of any is damage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Shown {
    key: u64,
    sig: u64,
    rev: u64,
    rect: Option<Rect>,
}

impl Shown {
    /// What a later frame compares it by.
    fn mark(&self) -> u64 {
        let mut h = DefaultHasher::new();
        (self.sig, self.rev).hash(&mut h);
        h.finish()
    }
}

/// Where a pass draws.
#[derive(Clone, Copy, Debug)]
enum PassTarget {
    Canvas,
    Layer(u32),
    Mask(ChunkId, u32),
    /// The window's frame, the canvas copied onto it first.
    Blit,
    Surface,
    /// The view [`GpuRenderer::submit`] is given.
    External,
    /// A canvas tile (`gpu::world`), a texture of the chunk store.
    Tile(crate::chunk::TextureId),
}

struct PassPlan {
    target: PassTarget,
    /// The map from its target's space to the canvas's, which clip
    /// entries map from (a layer's under a scale or turn).
    to_canvas: [f32; 6],
    origin: (i32, i32),
    size: (u32, u32),
    clear: Option<wgpu::Color>,
    parts: Vec<(Option<Rect>, Vec<Op>)>,
}

/// What a frame's passes reuse: each chunk's draws, each layer's composite.
#[derive(Default)]
struct Built {
    ops: foldhash::HashMap<usize, Vec<Op>>,
    composites: foldhash::HashMap<u32, Option<Op>>,
}

pub(super) fn pixel_rect(r: PixelRect) -> [f32; 4] {
    [
        crate::convert::i32_to_f32(r.x),
        crate::convert::i32_to_f32(r.y),
        crate::convert::u32_to_f32(r.width),
        crate::convert::u32_to_f32(r.height),
    ]
}

/// `r` as x0, y0, x1, y1.
pub(super) fn rect_of(r: PixelRect) -> [f32; 4] {
    let p = pixel_rect(r);
    [p[0], p[1], p[0] + p[2], p[1] + p[3]]
}

/// `r` inside `within`, never empty.
fn clamp_rect(r: PixelRect, within: PixelRect) -> PixelRect {
    let right = |q: PixelRect| q.x.saturating_add(i32::try_from(q.width).unwrap_or(i32::MAX));
    let bottom = |q: PixelRect| q.y.saturating_add(i32::try_from(q.height).unwrap_or(i32::MAX));
    let (x, y) = (r.x.max(within.x), r.y.max(within.y));
    let (x1, y1) = (right(r).min(right(within)), bottom(r).min(bottom(within)));
    PixelRect {
        x,
        y,
        width: u32::try_from(x1.saturating_sub(x)).unwrap_or(0).max(1),
        height: u32::try_from(y1.saturating_sub(y)).unwrap_or(0).max(1),
    }
}

/// What of the canvas to draw: `rects` merged; all of `screen` for a
/// canvas made anew, or one drawn whole (`whole`) when anything changed.
fn whole_or(rects: Vec<Rect>, screen: Rect, whole: bool, fresh: bool) -> Vec<Rect> {
    if fresh || (whole && rects.iter().any(|r| !r.intersect(&screen).is_empty())) {
        return vec![screen];
    }
    if whole {
        return Vec::new();
    }
    group(rects, screen)
}

/// Effect `id`'s fold and whether it shows nothing, from `fx` (its own
/// decision, not yet folded, and its parent's, folded).
fn folded_by(props: &Properties, fx: &[Fx], id: u32) -> (f32, bool) {
    let node = props.effect(id);
    let parent = node.map_or(0, |n| n.parent);
    let up = usize::try_from(parent).ok().and_then(|p| fx.get(p)).copied();
    let me = usize::try_from(id)
        .ok()
        .and_then(|i| fx.get(i))
        .copied()
        .unwrap_or_default();
    let fold = if me.layered {
        1.0
    } else {
        node.map_or(1.0, |n| n.opacity) * up.map_or(1.0, |f| f.fold)
    };
    (fold, me.skip || up.is_some_and(|f| f.skip))
}

fn hash_f(h: &mut DefaultHasher, v: &[f32]) {
    for f in v {
        f.to_bits().hash(h);
    }
}

pub(super) fn offset_of(props: &Properties, transform: u32, s: f32) -> (f32, f32) {
    let o = props.transform(transform).offset;
    (o.0 * s, o.1 * s)
}

fn round_layer(size: (u32, u32)) -> (u32, u32) {
    // Sizes in steps of 128 pixels, so a group that grows keeps its texture.
    let round = |v: u32| v.div_ceil(128).saturating_mul(128).max(128);
    (round(size.0), round(size.1))
}

impl GpuRenderer {
    /// Ready for a frame: the text settings, the curve text check, this
    /// frame's tables, the images kept chunks show.
    fn begin(&mut self, text: &mut Glyphs) {
        self.sync_text(text);
        if self.banded_ok.is_none() {
            self.banded_ok = Some(self.slug_self_test(text));
        }
        self.fr.reset();
        self.data.begin();
        self.pump_raster(text);
        for key in self.image_refs.keys() {
            self.images.touch(key);
        }
    }

    /// What effect `id` needs, holding chunks `list`: a layer, or its
    /// opacity folded, or nothing shown (its fold is set later).
    fn effect_fx(&mut self, scene: &Scene, id: u32, list: &[usize], chains: &[Vec<u32>]) -> Option<Fx> {
        let node = scene.properties.effect(id)?;
        let skip = node.opacity <= 0.0 || node.region.is_none();
        let mut layered = !node.filters.is_empty() || node.mask.is_some();
        if node.transformed && !layered {
            layered = !list.iter().all(|i| {
                scene
                    .chunks
                    .get(*i)
                    .and_then(|c| self.kept.get(&c.id))
                    .is_none_or(|k| k.drawable)
            });
        }
        let mut folded = false;
        if node.opacity < 1.0 && !layered && !skip {
            layered = self.overlapping(scene, id, list, chains);
            folded = !layered;
        }
        Some(Fx {
            layered,
            fold: 1.0,
            skip,
            folded,
        })
    }

    /// Where chunk `c` (its effects `chain`) draws, by `fx`.
    fn route(&self, scene: &Scene, c: &crate::scene::Chunk, chain: &[u32], fx: &[Fx]) -> Route {
        let fx_of = |e: u32| {
            usize::try_from(e)
                .ok()
                .and_then(|e| fx.get(e))
                .copied()
                .unwrap_or_default()
        };
        if !self.kept.contains_key(&c.id) || chain.iter().any(|e| fx_of(*e).skip) {
            return Route::Skip;
        }
        // The innermost effect with a layer (the chain runs inside out).
        let target = chain.iter().copied().find(|e| fx_of(*e).layered).unwrap_or(0);
        let props = &scene.properties;
        let until = props.effect(target).map_or(0, |n| n.transform);
        match compose(props, c.state.transform, until) {
            None => Route::Quads { target },
            Some(m) => Route::Matrix { target, m },
        }
    }

    /// Layer `l`'s region and map into the layer `outer` it composites into.
    fn layer_shape(&self, scene: &Scene, l: u32, outer: u32, viewport: PixelRect) -> (PixelRect, Option<[f32; 6]>) {
        let props = &scene.properties;
        let max = self.device.limits().max_texture_dimension_2d;
        let node = props.effect(l);
        let region = node.and_then(|n| n.region).unwrap_or_default();
        let region = if region.width > max || region.height > max {
            clamp_rect(region, viewport)
        } else {
            region
        };
        let until = props.effect(outer).map_or(0, |n| n.transform);
        (region, node.and_then(|n| compose(props, n.transform, until)))
    }

    /// The hash of what the canvases kept chunk `k` shows hold now.
    fn canvas_revision(&self, k: &super::chunks::Kept) -> u64 {
        let mut h = DefaultHasher::new();
        for key in &k.canvases {
            self.world.revision(*key).hash(&mut h);
        }
        h.finish()
    }

    /// Decide every effect and chunk of `scene` (prepared), seen through
    /// `viewport`.
    #[expect(clippy::too_many_lines, reason = "the frame's decisions, in order")]
    fn plan(&mut self, scene: &Scene, viewport: PixelRect) -> Plan {
        let props = &scene.properties;
        let n_fx = props.effects.len();
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); n_fx];
        let mut chains: Vec<Vec<u32>> = Vec::with_capacity(scene.chunks.len());
        for (i, c) in scene.chunks.iter().enumerate() {
            let chain: Vec<u32> = props.effect_chain(c.state.effect, 0).map(|(id, _)| id).collect();
            for e in &chain {
                if let Some(m) = usize::try_from(*e).ok().and_then(|e| members.get_mut(e)) {
                    m.push(i);
                }
            }
            chains.push(chain);
        }
        let mut fx = vec![Fx::default(); n_fx];
        for e in 1..n_fx {
            let Some(list) = members.get(e).filter(|l| !l.is_empty()) else {
                continue;
            };
            let id = u32::try_from(e).unwrap_or(0);
            if let Some(f) = self.effect_fx(scene, id, list, &chains)
                && let Some(slot) = fx.get_mut(e)
            {
                *slot = f;
            }
        }
        let folded = fx.iter().filter(|f| f.folded).count();
        self.drawn.folded = self.drawn.folded.max(folded);
        // Folds, parents first: an effect's parent is folded before it.
        let mut done = vec![false; n_fx];
        if let Some(d) = done.first_mut() {
            *d = true;
        }
        for e in 1..n_fx {
            let mut path = Vec::new();
            let mut at = u32::try_from(e).unwrap_or(0);
            while usize::try_from(at).ok().and_then(|a| done.get(a)) == Some(&false) && path.len() < 4096 {
                path.push(at);
                at = props.effect(at).map_or(0, |n| n.parent);
            }
            for id in path.into_iter().rev() {
                let Ok(i) = usize::try_from(id) else { continue };
                let (fold, skip) = folded_by(props, &fx, id);
                if let Some(f) = fx.get_mut(i) {
                    f.fold = fold;
                    f.skip = skip;
                }
                if let Some(d) = done.get_mut(i) {
                    *d = true;
                }
            }
        }
        let fx_of = |e: u32| {
            usize::try_from(e)
                .ok()
                .and_then(|e| fx.get(e))
                .copied()
                .unwrap_or_default()
        };
        // Routes and entries.
        let mut routes = Vec::with_capacity(scene.chunks.len());
        let mut place = Vec::with_capacity(scene.chunks.len());
        let mut root: Vec<Entry> = Vec::new();
        let mut layers: foldhash::HashMap<u32, LayerPlan> = foldhash::HashMap::default();
        for (i, (c, chain)) in scene.chunks.iter().zip(&chains).enumerate() {
            let route = self.route(scene, c, chain, &fx);
            routes.push(route);
            if route == Route::Skip {
                place.push(EntryAt::None);
                continue;
            }
            let mut outer = 0_u32;
            for l in chain.iter().rev().copied().filter(|e| fx_of(*e).layered) {
                if !layers.contains_key(&l) {
                    let (region, m) = self.layer_shape(scene, l, outer, viewport);
                    let at = match layers.get_mut(&outer) {
                        Some(o) if outer != 0 => {
                            o.entries.push(Entry::Layer(l));
                            EntryAt::Layer(outer)
                        }
                        _ => {
                            root.push(Entry::Layer(l));
                            EntryAt::Root(root.len().saturating_sub(1))
                        }
                    };
                    layers.insert(
                        l,
                        LayerPlan {
                            entries: Vec::new(),
                            region,
                            m,
                            content: 0,
                            outer,
                            place: at,
                        },
                    );
                }
                outer = l;
            }
            place.push(match layers.get_mut(&outer) {
                Some(o) if outer != 0 => {
                    o.entries.push(Entry::Chunk(i));
                    EntryAt::Layer(outer)
                }
                _ => {
                    root.push(Entry::Chunk(i));
                    EntryAt::Root(root.len().saturating_sub(1))
                }
            });
        }
        // What each chunk is drawn under, to find it from what changed.
        let mut by_transform: foldhash::HashMap<u32, Vec<usize>> = foldhash::HashMap::default();
        let mut clip_layers: foldhash::HashMap<u32, Vec<u32>> = foldhash::HashMap::default();
        let mut by_effect: foldhash::HashMap<u32, Vec<usize>> = foldhash::HashMap::default();
        let mut canvases = Vec::new();
        let mut by_clip: foldhash::HashMap<u32, Vec<usize>> = foldhash::HashMap::default();
        for (i, c) in scene.chunks.iter().enumerate() {
            by_transform.entry(c.state.transform).or_default().push(i);
            // Under a clip: drawn into a layer, the layer is drawn again
            // when the clip's revision changes (its chain's, whatever in it
            // changed); on the canvas, its entry is keyed again.
            if c.state.clip != 0 {
                match place.get(i) {
                    Some(EntryAt::Layer(l)) => {
                        let list = clip_layers.entry(c.state.clip).or_default();
                        if !list.contains(l) {
                            list.push(*l);
                        }
                    }
                    Some(EntryAt::Root(k)) => by_clip.entry(c.state.clip).or_default().push(*k),
                    _ => {}
                }
            }
            by_effect.entry(c.state.effect).or_default().push(i);
            if let Some(k) = self.kept.get(&c.id)
                && !k.canvases.is_empty()
            {
                canvases.push((i, self.canvas_revision(k)));
            }
        }
        let mut plan = Plan {
            fx,
            routes,
            root,
            layers,
            shown: Vec::new(),
            chains,
            members,
            place,
            ids: scene.chunks.iter().map(|c| c.id).collect(),
            states: scene.chunks.iter().map(|c| c.state).collect(),
            by_transform,
            by_effect,
            clip_layers,
            by_clip,
            canvases,
            folded,
            scale: scene.scale.to_bits(),
            viewport,
            seen: Marks::default(),
            seen_fx: Marks::default(),
            seen_t: Marks::default(),
        };
        // Signatures: layers inside out, then the canvas's entries.
        let ids: Vec<u32> = plan.layers.keys().copied().collect();
        for l in ids {
            self.layer_content(scene, &mut plan, l, 0);
        }
        let mut shown = Vec::with_capacity(plan.root.len());
        for k in 0..plan.root.len() {
            shown.push(self.root_entry(scene, &plan, k));
        }
        plan.shown = shown;
        plan
    }

    /// The canvas's entry `k` of `plan`.
    fn root_entry(&self, scene: &Scene, plan: &Plan, k: usize) -> Shown {
        let key_of = |e: Entry| match e {
            Entry::Chunk(i) => scene.chunks.get(i).map_or(0, |c| c.id.0),
            Entry::Layer(l) => {
                let mut h = DefaultHasher::new();
                (l, 0x1a7e_u32).hash(&mut h);
                h.finish()
            }
        };
        let Some(&e) = plan.root.get(k) else {
            return Shown::default();
        };
        let pred = k
            .checked_sub(1)
            .and_then(|p| plan.root.get(p))
            .map_or(0, |p| key_of(*p));
        let (sig, rev, rect) = match e {
            Entry::Chunk(i) => (
                self.chunk_sig(scene, plan, i, (0.0, 0.0)),
                scene.chunks.get(i).map_or(0, |c| self.clip_tree.rev(c.state.clip)),
                self.chunk_rect(scene, plan, i),
            ),
            Entry::Layer(l) => (
                self.composite_sig(scene, plan, l, (0.0, 0.0)),
                0,
                self.layer_rect(plan, l),
            ),
        };
        let mut h = DefaultHasher::new();
        (sig, pred).hash(&mut h);
        Shown {
            key: key_of(e),
            sig: h.finish(),
            rev,
            rect,
        }
    }

    /// Bring `plan` (of the scene last drawn) up to `scene`, of which
    /// `changed` changed: only the chunks, effects and layers it reaches are
    /// decided and signed again, and the property tables' entries it names
    /// written. The damage, or `None` when the change reaches what only a
    /// whole plan decides (an effect gaining or losing a layer, a chunk
    /// under another state).
    #[expect(
        clippy::too_many_lines,
        reason = "chunks, effects, routes, layers, entries, in order"
    )]
    fn replan(
        &mut self,
        scene: &Scene,
        plan: &mut Plan,
        changed: &Changed,
        clips: &super::clips::Update,
    ) -> Option<Vec<Rect>> {
        let props = &scene.properties;
        let s = scene.scale;
        let n = scene.chunks.len();
        if n != plan.routes.len() || props.effects.len() != plan.fx.len() || s.to_bits() != plan.scale {
            return None;
        }
        let u = |i: usize| u32::try_from(i).unwrap_or(u32::MAX);
        // The chunks reached.
        plan.seen.begin(n);
        let mut dirty: Vec<usize> = Vec::new();
        for &i in &changed.chunks {
            let c = scene.chunks.get(i)?;
            if plan.ids.get(i) != Some(&c.id) || plan.states.get(i) != Some(&c.state) {
                return None;
            }
            if plan.seen.mark(u(i)) {
                dirty.push(i);
            }
        }
        plan.seen_t.begin(props.transforms.len());
        for &t in &changed.transforms {
            plan.seen_t.mark(t);
            for &i in plan.by_transform.get(&t).into_iter().flatten() {
                if plan.seen.mark(u(i)) {
                    dirty.push(i);
                }
            }
        }
        // What a canvas's tiles show is part of what its chunk shows.
        for k in 0..plan.canvases.len() {
            let Some(&(i, was)) = plan.canvases.get(k) else {
                continue;
            };
            let now = scene
                .chunks
                .get(i)
                .and_then(|c| self.kept.get(&c.id))
                .map_or(0, |kept| self.canvas_revision(kept));
            if now != was {
                if let Some(c) = plan.canvases.get_mut(k) {
                    c.1 = now;
                }
                if plan.seen.mark(u(i)) {
                    dirty.push(i);
                }
            }
        }
        // The effects reached: what changed, and what holds a chunk reached.
        plan.seen_fx.begin(plan.fx.len());
        let mut effects: Vec<u32> = Vec::new();
        for &e in &changed.effects {
            if plan.seen_fx.mark(e) {
                effects.push(e);
            }
        }
        for &i in &dirty {
            for &e in plan.chains.get(i).into_iter().flatten() {
                if plan.seen_fx.mark(e) {
                    effects.push(e);
                }
            }
        }
        // Parents first, so a fold reads its parent's of this frame.
        let depth = |e: u32| props.effect_chain(e, 0).count();
        effects.sort_by_key(|e| depth(*e));
        for &e in &effects {
            let Ok(i) = usize::try_from(e) else { continue };
            if e == 0 {
                continue;
            }
            let was = plan.fx.get(i).copied().unwrap_or_default();
            let list = plan.members.get(i).map_or(&[][..], Vec::as_slice);
            let mut next = if list.is_empty() {
                Fx::default()
            } else {
                self.effect_fx(scene, e, list, &plan.chains)?
            };
            if let Some(slot) = plan.fx.get_mut(i) {
                *slot = next;
            }
            let (fold, skip) = folded_by(props, &plan.fx, e);
            next.fold = fold;
            next.skip = skip;
            if next.layered != was.layered || next.skip != was.skip {
                // Entries come and go: only a whole plan places them.
                return None;
            }
            if let Some(slot) = plan.fx.get_mut(i) {
                *slot = next;
            }
            if next.folded != was.folded {
                plan.folded = if next.folded {
                    plan.folded.saturating_add(1)
                } else {
                    plan.folded.saturating_sub(1)
                };
            }
            if next.fold != was.fold {
                self.tables.set_effect(&mut self.props, e, next.fold);
                // What it holds draws at another opacity: the chunks drawn
                // under it are signed again (under an effect of its
                // subtree, they are reached through that one).
                for &c in plan.by_effect.get(&e).into_iter().flatten() {
                    if plan.seen.mark(u(c)) {
                        dirty.push(c);
                    }
                }
            }
        }
        self.drawn.signed = dirty.len();
        self.drawn.folded = self.drawn.folded.max(plan.folded);
        // The property tables' entries that changed.
        for &t in &changed.transforms {
            let entry = transform_entry(props.transform(t), s);
            self.tables.set_transform(&mut self.props, t, entry);
        }
        // The clips that changed ([`ClipTree::update`]): their entries
        // written, the layers drawn under them drawn again, and the canvas
        // entries under them keyed again below; their damage is what
        // each chain kept and keeps.
        for &c in &clips.written {
            self.tables.set_clip(&mut self.props, c, self.clip_tree.entry(c));
            self.drawn.clips_written = self.drawn.clips_written.saturating_add(1);
        }
        let mut clip_stale: Vec<u32> = Vec::new();
        for c in &clips.moved {
            clip_stale.extend(plan.clip_layers.get(c).into_iter().flatten());
        }
        // Routes: a chunk keeps its layer, or the plan is made whole.
        for &i in &dirty {
            let c = scene.chunks.get(i)?;
            let chain = plan.chains.get(i)?;
            let next = self.route(scene, c, chain, &plan.fx);
            let was = plan.routes.get(i).copied()?;
            if next.target() != was.target() {
                return None;
            }
            if let Some(r) = plan.routes.get_mut(i) {
                *r = next;
            }
        }
        // Layers: their shape where their effect, transform or clip
        // changed (composited again), their content where what they hold
        // or their region did (drawn again).
        let viewport = plan.viewport;
        let mut stale: Vec<u32> = Vec::new();
        let mut moved: Vec<u32> = Vec::new();
        let ids: Vec<u32> = plan.layers.keys().copied().collect();
        for l in ids {
            let Some(node) = props.effect(l) else { continue };
            let touched = plan.seen_fx.has(l) || plan.seen_t.has(node.transform);
            if !touched {
                continue;
            }
            let outer = plan.layers.get(&l).map_or(0, |lp| lp.outer);
            let (region, m) = self.layer_shape(scene, l, outer, viewport);
            if let Some(lp) = plan.layers.get_mut(&l) {
                // Its texels: where it lies changed, or what clips them
                // when it moved against the canvas, where clips are.
                let clipped = || {
                    lp.entries.iter().any(
                        |e| matches!(e, Entry::Chunk(i) if scene.chunks.get(*i).is_some_and(|c| c.state.clip != 0)),
                    )
                };
                if lp.region != region || clipped() {
                    stale.push(l);
                }
                lp.region = region;
                lp.m = m;
            }
            moved.push(l);
        }
        stale.extend(clip_stale);
        let mut roots: Vec<usize> = Vec::new();
        for &i in &dirty {
            match plan.place.get(i) {
                Some(EntryAt::Root(k)) => roots.push(*k),
                Some(EntryAt::Layer(l)) => stale.push(*l),
                _ => {}
            }
        }
        // A layer that changed changes the one it composites into.
        moved.extend_from_slice(&stale);
        let mut k = 0;
        while let Some(&l) = moved.get(k) {
            k = k.saturating_add(1);
            if let Some(EntryAt::Layer(outer)) = plan.layers.get(&l).map(|lp| lp.place)
                && !stale.contains(&outer)
            {
                stale.push(outer);
                moved.push(outer);
            }
        }
        stale.sort_unstable();
        stale.dedup();
        moved.sort_unstable();
        moved.dedup();
        for l in &stale {
            if let Some(lp) = plan.layers.get_mut(l) {
                lp.content = 0;
            }
        }
        for &l in &stale {
            self.layer_content(scene, plan, l, 0);
        }
        for &l in &moved {
            if let Some(EntryAt::Root(k)) = plan.layers.get(&l).map(|lp| lp.place) {
                roots.push(k);
            }
        }
        roots.sort_unstable();
        roots.dedup();
        // The canvas's entries signed again, and those under a clip that
        // changed keyed again (its revision, its rectangle cut anew): their
        // damage, with the clips' own.
        let mut rects = clips.damage.clone();
        let mut rekey: Vec<usize> = Vec::new();
        for c in &clips.moved {
            rekey.extend(plan.by_clip.get(c).into_iter().flatten());
        }
        rekey.sort_unstable();
        rekey.dedup();
        for k in rekey {
            if roots.binary_search(&k).is_ok() {
                continue;
            }
            let Some(Entry::Chunk(i)) = plan.root.get(k).copied() else {
                continue;
            };
            let rev = scene.chunks.get(i).map_or(0, |c| self.clip_tree.rev(c.state.clip));
            let rect = self.chunk_rect(scene, plan, i);
            let Some(slot) = plan.shown.get_mut(k) else { continue };
            let next = Shown { rev, rect, ..*slot };
            self.reshow(slot, next, &mut rects);
        }
        for k in roots {
            let next = self.root_entry(scene, plan, k);
            let Some(slot) = plan.shown.get_mut(k) else { continue };
            self.reshow(slot, next, &mut rects);
        }
        Some(rects)
    }

    /// Canvas entry `slot` becomes `next`: its damage when it changed.
    fn reshow(&mut self, slot: &mut Shown, next: Shown, rects: &mut Vec<Rect>) {
        let (old, new) = (slot.rect.unwrap_or_default(), next.rect.unwrap_or_default());
        if slot.mark() != next.mark() || old != new {
            rects.push(old);
            rects.push(new);
        }
        *slot = next;
        self.shown.insert(next.key, (next.mark(), new));
    }

    /// Whether the primitives under effect `e` (chunks `list`) overlap,
    /// each moved by its offset: kept while they are what they were.
    fn overlapping(&mut self, scene: &Scene, e: u32, list: &[usize], chains: &[Vec<u32>]) -> bool {
        let props = &scene.properties;
        let s = scene.scale;
        let mut h = DefaultHasher::new();
        for i in list {
            let Some(c) = scene.chunks.get(*i) else { continue };
            c.id.hash(&mut h);
            self.kept.get(&c.id).map(|k| k.epoch).hash(&mut h);
            let o = offset_of(props, c.state.transform, s);
            hash_f(&mut h, &[o.0, o.1]);
        }
        let key = h.finish();
        if let Some((k, v)) = self.overlaps.get(&e)
            && *k == key
        {
            return *v;
        }
        let mut answer = false;
        // What is turned or scaled inside it may land anywhere: as if it overlapped.
        'nested: for i in list {
            for id in chains.get(*i).into_iter().flatten() {
                if *id == e {
                    break;
                }
                if props.effect(*id).is_some_and(|n| n.transformed) {
                    answer = true;
                    break 'nested;
                }
            }
        }
        if !answer {
            let mut rects: Vec<[f32; 4]> = Vec::new();
            for i in list {
                let Some(c) = scene.chunks.get(*i) else { continue };
                let Some(k) = self.kept.get(&c.id) else { continue };
                let (ox, oy) = offset_of(props, c.state.transform, s);
                rects.extend(k.rects.iter().map(|r| [r[0] + ox, r[1] + oy, r[2] + ox, r[3] + oy]));
            }
            rects.sort_by(|a, b| a[0].total_cmp(&b[0]));
            // Touching is not overlapping: a hundredth of a pixel is allowed.
            const EPS: f32 = 0.01;
            let mut active: Vec<[f32; 4]> = Vec::new();
            'sweep: for r in rects {
                active.retain(|a| a[2] > r[0] + EPS);
                for a in &active {
                    if a[1] < r[3] - EPS && r[1] < a[3] - EPS {
                        answer = true;
                        break 'sweep;
                    }
                }
                active.push(r);
            }
        }
        self.overlaps.insert(e, (key, answer));
        answer
    }

    /// The signature of what layer `l` holds, relative to its region
    /// (stored in the plan; nested layers first).
    fn layer_content(&self, scene: &Scene, plan: &mut Plan, l: u32, depth: u32) -> u64 {
        if let Some(lp) = plan.layers.get(&l)
            && lp.content != 0
        {
            return lp.content;
        }
        let Some((entries, region)) = plan.layers.get(&l).map(|lp| (lp.entries.clone(), lp.region)) else {
            return 0;
        };
        let origin = (
            crate::convert::i32_to_f32(region.x),
            crate::convert::i32_to_f32(region.y),
        );
        // The layer's space, which its texture holds whatever moves, scales
        // or turns it.
        let to_canvas = scene
            .properties
            .effect(l)
            .and_then(|n| compose(&scene.properties, n.transform, 0));
        let mut walks = foldhash::HashMap::default();
        let mut h = DefaultHasher::new();
        (region.width, region.height).hash(&mut h);
        for e in entries {
            match e {
                Entry::Chunk(i) => {
                    self.chunk_sig(scene, plan, i, origin).hash(&mut h);
                    // Its clip chain as the layer's texels see it (clips
                    // are placed in the canvas, the texels in the layer).
                    let clip = scene.chunks.get(i).map_or(0, |c| c.state.clip);
                    if clip != 0 {
                        self.clip_tree.key_in(clip, to_canvas, origin, &mut walks).hash(&mut h);
                    }
                }
                Entry::Layer(c) if depth < 64 => {
                    self.layer_content(scene, plan, c, depth.saturating_add(1));
                    self.composite_sig(scene, plan, c, origin).hash(&mut h);
                }
                Entry::Layer(_) => {}
            }
        }
        let sig = h.finish().max(1);
        if let Some(lp) = plan.layers.get_mut(&l) {
            lp.content = sig;
        }
        sig
    }

    /// Everything that decides chunk `i`'s pixels, its target's origin at `origin`.
    fn chunk_sig(&self, scene: &Scene, plan: &Plan, i: usize, origin: (f32, f32)) -> u64 {
        let props = &scene.properties;
        let s = scene.scale;
        let mut h = DefaultHasher::new();
        let Some(c) = scene.chunks.get(i) else { return 0 };
        c.id.hash(&mut h);
        if let Some(k) = self.kept.get(&c.id) {
            k.epoch.hash(&mut h);
            for key in &k.canvases {
                self.world.revision(*key).hash(&mut h);
            }
        }
        let (ox, oy) = offset_of(props, c.state.transform, s);
        let fold = usize::try_from(c.state.effect)
            .ok()
            .and_then(|e| plan.fx.get(e))
            .map_or(1.0, |f| f.fold);
        hash_f(&mut h, &[fold]);
        match plan.routes.get(i) {
            Some(Route::Matrix { m, .. }) => {
                let mut g = path::then([1.0, 0.0, 0.0, 1.0, ox, oy], *m);
                g[4] -= origin.0;
                g[5] -= origin.1;
                hash_f(&mut h, &g);
            }
            _ => hash_f(&mut h, &[ox - origin.0, oy - origin.1]),
        }
        // Not its clip: its entry's key and its layer's content hold its
        // chain's revision ([`Shown::rev`]), so a clip changing signs no
        // chunk again.
        h.finish()
    }

    /// The canvas pixels chunk `i` may touch: what it draws, cut to what
    /// its clip chain keeps (a change of the chain is damage of its own).
    fn chunk_rect(&self, scene: &Scene, plan: &Plan, i: usize) -> Option<Rect> {
        let props = &scene.properties;
        let s = scene.scale;
        let c = scene.chunks.get(i)?;
        let b = self.kept.get(&c.id)?.bounds?;
        let (ox, oy) = offset_of(props, c.state.transform, s);
        let r = match plan.routes.get(i)? {
            Route::Skip => return None,
            Route::Quads { .. } => [b[0] + ox, b[1] + oy, b[2] + ox, b[3] + oy],
            Route::Matrix { m, .. } => {
                let mut r = mapped(b, path::then([1.0, 0.0, 0.0, 1.0, ox, oy], *m));
                r = [r[0] - 1.0, r[1] - 1.0, r[2] + 1.0, r[3] + 1.0];
                r
            }
        };
        let r = match self.clip_tree.bounds(c.state.clip) {
            Some(k) => intersect(r, [k[0] - 1.0, k[1] - 1.0, k[2] + 1.0, k[3] + 1.0]),
            None => r,
        };
        Rect::covering(r)
    }

    /// The pixels layer `l`'s composite covers in its target.
    fn layer_quad(&self, plan: &Plan, l: u32) -> Option<[f32; 4]> {
        let lp = plan.layers.get(&l)?;
        let r = rect_of(lp.region);
        let q = lp.m.map_or(r, |m| mapped(r, m));
        Some([q[0] - 1.0, q[1] - 1.0, q[2] + 1.0, q[3] + 1.0])
    }

    fn layer_rect(&self, plan: &Plan, l: u32) -> Option<Rect> {
        Rect::covering(self.layer_quad(plan, l)?)
    }

    /// Everything that decides layer `l`'s composite, its target's origin at `origin`.
    fn composite_sig(&self, scene: &Scene, plan: &Plan, l: u32, origin: (f32, f32)) -> u64 {
        let s = scene.scale;
        let mut h = DefaultHasher::new();
        let Some(lp) = plan.layers.get(&l) else { return 0 };
        lp.content.hash(&mut h);
        let r = pixel_rect(lp.region);
        hash_f(&mut h, &[r[0] - origin.0, r[1] - origin.1, r[2], r[3]]);
        if let Some(mut m) = lp.m {
            m[4] -= origin.0;
            m[5] -= origin.1;
            hash_f(&mut h, &m);
        }
        if let Some(n) = scene.properties.effect(l) {
            let up = usize::try_from(n.parent)
                .ok()
                .and_then(|p| plan.fx.get(p))
                .map_or(1.0, |f| f.fold);
            hash_f(&mut h, &[n.opacity, up]);
            for f in &n.filters {
                match f {
                    Filter::Brightness(k) => hash_f(&mut h, &[1.0, *k]),
                    Filter::Grayscale(a) => hash_f(&mut h, &[2.0, *a]),
                }
            }
            if let Some(g) = &n.mask {
                let (line, stops) = line_and_stops(g, s);
                hash_f(
                    &mut h,
                    &[
                        line[0] - origin.0,
                        line[1] - origin.1,
                        line[2] - origin.0,
                        line[3] - origin.1,
                    ],
                );
                for (o, c) in stops {
                    hash_f(&mut h, &[o]);
                    hash_f(&mut h, &c);
                }
            }
        }
        h.finish()
    }

    /// What of the canvas this frame redraws: what changed since it was last drawn.
    fn damage(&mut self, plan: &Plan, clips: Vec<Rect>, screen: Rect, whole: bool, fresh: bool) -> Vec<Rect> {
        let mut rects = clips;
        let mut next: foldhash::HashMap<u64, (u64, Rect)> = foldhash::HashMap::default();
        next.reserve(plan.shown.len());
        for e in &plan.shown {
            let (key, sig, rect) = (e.key, e.mark(), e.rect.unwrap_or_default());
            next.insert(key, (sig, rect));
            match self.shown.remove(&key) {
                Some((s0, r0)) if s0 == sig && r0 == rect => {}
                Some((_, r0)) => {
                    rects.push(r0);
                    rects.push(rect);
                }
                None => rects.push(rect),
            }
        }
        rects.extend(self.shown.drain().map(|(_, (_, r))| r));
        self.shown = next;
        whole_or(rects, screen, whole, fresh)
    }

    /// Bring the canvas up to `scene`, of which `changed` changed since the
    /// last frame: plan and sign only what that reaches (the whole scene on
    /// a first frame, a new canvas, new text settings, or a change only a
    /// whole plan places), draw its damage, and (with a window) copy the
    /// canvas onto the window's frame.
    #[expect(clippy::too_many_lines, reason = "a frame's plan, damage and passes, in order")]
    pub(super) fn render_scene(&mut self, scene: &Scene, changed: &Changed, text: &mut Glyphs) {
        self.drawn = Drawn::default();
        let text_was = self.text_generation;
        self.begin(text);
        let bg = scene.background.map(|c| c.to_rgba8());
        let opaque = bg.is_some_and(|c| c[3] == 255);
        let max = self.device.limits().max_texture_dimension_2d;
        if scene.size.0 > max || scene.size.1 > max {
            // Larger than a texture may be (a whole page, headless): there
            // is no canvas to keep, only captures in bands.
            self.canvas = None;
            self.shown.clear();
            self.planned = None;
            return;
        }
        let fresh = self.ensure_canvas(scene.size, bg);
        let (w, h) = (scene.size.0.max(1), scene.size.1.max(1));
        let viewport = PixelRect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        let screen = Rect {
            x0: 0,
            y0: 0,
            x1: i32::try_from(w).unwrap_or(i32::MAX),
            y1: i32::try_from(h).unwrap_or(i32::MAX),
        };
        // A canvas that keeps nothing under what it draws (a background
        // not opaque) is drawn whole when anything changed.
        let whole = fresh || !opaque;
        let mut planned = self
            .planned
            .take()
            .filter(|p| !changed.all && !fresh && text_was == self.text_generation && p.viewport == viewport);
        // The clip tree first: both planners read it, and its damage.
        self.unpark_tree();
        let clips = self.clip_tree.update(
            &scene.properties,
            scene.scale,
            planned.is_none(),
            &changed.clips,
            &changed.transforms,
        );
        self.drawn.clip_chain = clips.longest;
        let mut rects = None;
        if let Some(plan) = planned.as_mut() {
            self.prepare_changed(scene, &changed.chunks, text);
            if let Some(damage) = self.replan(scene, plan, changed, &clips) {
                if self.cold_plan {
                    self.check_plan(scene, plan);
                }
                rects = Some(whole_or(damage, screen, whole, false));
            }
        }
        let (plan, rects) = match (planned, rects) {
            (Some(plan), Some(rects)) => (plan, rects),
            _ => {
                self.prepare(scene, text, false);
                let plan = self.plan(scene, viewport);
                self.drawn.signed = scene.chunks.len();
                self.drawn.planned_whole = true;
                self.drawn.clips_written = scene.properties.clips.len();
                self.tables.write_whole(scene, &plan, &self.kept, &self.clip_tree);
                self.layered = plan.layers.keys().copied().collect();
                let rects = self.damage(&plan, clips.damage, screen, whole, fresh);
                (plan, rects)
            }
        };
        // A blurred backdrop on the canvas reads all of its box: damage
        // that meets it holds it whole (`read.rs`).
        let rects = if whole || self.blurring == 0 {
            rects
        } else {
            let root: Vec<usize> = plan
                .root
                .iter()
                .filter_map(|e| match e {
                    Entry::Chunk(c) => Some(*c),
                    Entry::Layer(_) => None,
                })
                .collect();
            self.with_reads(scene, &root, rects)
        };
        let mut passes = self.mask_passes();
        if !rects.is_empty() {
            self.drawn.damage = rects.iter().map(|r| u64::try_from(r.area()).unwrap_or(0)).sum();
            self.drawn.damage_rects = rects.len();
            let visible = rect_of(viewport);
            let mut built = Built::default();
            let mut parts = Vec::with_capacity(rects.len());
            let background = if whole {
                None
            } else {
                Some(self.background_op(scene.background.unwrap_or(Color::WHITE), (w, h)))
            };
            for r in &rects {
                let mut ops = Vec::new();
                // What the last chunk painting all of `r` opaque hides is
                // not drawn: an app's root background hides the canvas
                // background and all under it, a pass over the damage
                // fewer (each a full read and blend of its pixels).
                let hidden = plan
                    .root
                    .iter()
                    .rposition(|e| matches!(e, Entry::Chunk(c) if opaque_over(scene, *c, *r)));
                if let Some(op) = &background
                    && hidden.is_none()
                {
                    push_op(&mut ops, op.clone());
                }
                for (k, (e, Shown { rect, .. })) in plan.root.iter().zip(&plan.shown).enumerate() {
                    if hidden.is_some_and(|h| k < h) || !rect.is_some_and(|q| !q.intersect(r).is_empty()) {
                        continue;
                    }
                    let mut entry = self.entry_ops(scene, &plan, *e, visible, &mut built, &mut passes, text);
                    // The hiding rectangle, its chunk's first quad, written
                    // over the part rather than blended.
                    if hidden == Some(k)
                        && let Some(Op::Quads {
                            extra: Extra::None,
                            instances,
                        }) = entry.first_mut()
                        && instances.start < instances.end
                    {
                        let first = instances.start;
                        instances.start = first.saturating_add(1);
                        if instances.start >= instances.end {
                            entry.remove(0);
                        }
                        push_op(&mut ops, Op::Fill(first));
                    }
                    for op in entry {
                        push_op(&mut ops, op);
                    }
                }
                parts.push(((!whole).then_some(*r), ops));
            }
            passes.push(PassPlan {
                target: PassTarget::Canvas,
                to_canvas: IDENTITY,
                origin: (0, 0),
                size: (w, h),
                clear: whole.then(|| clear_color(scene.background)),
                parts,
            });
        }
        // Tiles rastered at the frame's start draw first (the canvas pass
        // samples them), and even with no damage: a tile split across
        // frames is not shown until its last part.
        if !self.world.jobs.is_empty() {
            let tiles = self.tile_passes();
            passes.splice(0..0, tiles);
        }
        // Presented only when what the window shows changes: damage, or a
        // display that does not show the canvas. An overlay drawn, or gone,
        // presents itself ([`Self::overlay_scene`], `end_frame`). A frame
        // texture held for pacing is not a reason ([`super::Window`]).
        let present = self.window.as_ref().is_some_and(|w| w.stale || !rects.is_empty());
        if present {
            passes.push(self.blit_pass());
        }
        self.planned = Some(plan);
        if passes.is_empty() && self.masks.is_empty() {
            return;
        }
        self.submit(&passes, None);
    }

    /// The incremental plan `plan` against one made whole from `scene`
    /// (`GUI_COLD_CHECK`): the same decisions, signatures and tables.
    fn check_plan(&mut self, scene: &Scene, plan: &Plan) {
        // The whole plan and tables from a clip tree decided anew from the
        // scene, not the one this frame brought up to it.
        let mut fresh = ClipTree::default();
        fresh.update(&scene.properties, scene.scale, true, &[], &[]);
        let kept_tree = std::mem::replace(&mut self.clip_tree, fresh);
        let cold = self.plan(scene, plan.viewport);
        let mut tables = self.tables.clone();
        tables.write_whole(scene, &cold, &self.kept, &self.clip_tree);
        self.clip_tree = kept_tree;
        let frame = self.frame;
        assert!(cold.fx == plan.fx, "frame {frame}: effects differ from a whole plan");
        assert!(
            cold.routes == plan.routes,
            "frame {frame}: routes differ from a whole plan"
        );
        assert!(
            cold.root == plan.root,
            "frame {frame}: entries differ from a whole plan"
        );
        let k = (0..cold.shown.len().max(plan.shown.len())).find(|k| cold.shown.get(*k) != plan.shown.get(*k));
        assert!(
            k.is_none(),
            "frame {frame}: canvas entry {k:?} differs from a whole plan: {:?} against {:?}",
            k.and_then(|k| plan.shown.get(k)),
            k.and_then(|k| cold.shown.get(k))
        );
        assert!(
            cold.folded == plan.folded,
            "frame {frame}: folds differ from a whole plan"
        );
        let same = tables.transforms == self.tables.transforms
            && tables.clips == self.tables.clips
            && tables.effects == self.tables.effects;
        assert!(same, "frame {frame}: property tables differ from a whole plan");
        for c in &scene.chunks {
            if let Some(k) = self.kept.get(&c.id) {
                let at = usize::try_from(k.slot).ok();
                let mine = at.and_then(|i| self.tables.slot_states.get(i));
                assert!(
                    mine == Some(&[c.state.transform, c.state.clip, c.state.effect, 0]),
                    "frame {frame}: chunk {:?}'s state slot differs from its state",
                    c.id
                );
            }
        }
    }

    /// The pass that copies the canvas onto the window's frame texture:
    /// pixel for pixel, at the canvas's own size. A surface of another
    /// size (resized, the frame laid out at the new size not committed
    /// yet) is cleared to the canvas's background first, and shows the
    /// last frame in its corner over it.
    /// Copy the canvas onto the window's frame, over an overlay gone.
    pub(super) fn blit(&mut self) {
        self.submit(&[self.blit_pass()], None);
    }

    fn blit_pass(&self) -> PassPlan {
        let (size, background) = self.canvas.as_ref().map_or(((1, 1), None), |c| (c.size, c.background));
        let other = self.window.as_ref().is_some_and(|w| w.size() != size);
        PassPlan {
            target: PassTarget::Blit,
            to_canvas: IDENTITY,
            origin: (0, 0),
            size,
            clear: other.then(|| {
                let [r, g, b, a] = background.unwrap_or([255; 4]).map(|v| f64::from(v) / 255.0);
                wgpu::Color {
                    r: r * a,
                    g: g * a,
                    b: b * a,
                    a,
                }
            }),
            parts: Vec::new(),
        }
    }

    /// Paint `scene` over the window's frame as shown, this frame only.
    /// With `generation`, what it shows is drawn again only when that
    /// changed or the frame presents anyway: an overlay unchanged over a
    /// frame that changed nothing presents nothing.
    pub(super) fn overlay_scene(&mut self, scene: &Scene, generation: Option<u64>, text: &mut Glyphs) {
        let Some(size) = self.canvas.as_ref().map(|c| c.size) else {
            return;
        };
        let Some(blitted) = self.window.as_ref().map(|w| w.blitted) else {
            return;
        };
        if generation.is_some() && generation == self.overlay_shown && !blitted && self.overlaid.1 {
            // Still on screen as drawn.
            self.overlaid.0 = true;
            return;
        }
        self.overlay_shown = generation;
        self.prepare(scene, text, true);
        let viewport = PixelRect {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        };
        let ops = self.overlay_ops(scene, rect_of(viewport), text);
        let mut passes = self.mask_passes();
        if !blitted {
            passes.push(self.blit_pass());
        }
        passes.push(PassPlan {
            target: PassTarget::Surface,
            to_canvas: IDENTITY,
            origin: (0, 0),
            size,
            clear: None,
            parts: vec![(None, ops)],
        });
        self.submit(&passes, None);
        self.overlaid.0 = true;
    }

    /// The draws of an overlay's chunks (root state, drawn straight).
    fn overlay_ops(&mut self, scene: &Scene, visible: [f32; 4], text: &mut Glyphs) -> Vec<Op> {
        let mut ops = Vec::new();
        for c in &scene.chunks {
            let Some(k) = self.kept.get(&c.id) else { continue };
            let (segs, base, display, slot) = (
                k.segs.clone(),
                k.index.map_or(0, |i| i.start),
                Arc::clone(&k.display),
                k.slot,
            );
            self.tables.set_slot(&mut self.props, slot, [0; 4]);
            for seg in segs {
                match seg {
                    Seg::Quads { extra, start, end } => push_op(
                        &mut ops,
                        Op::Quads {
                            extra,
                            instances: base.saturating_add(start)..base.saturating_add(end),
                        },
                    ),
                    Seg::Item(n) => {
                        if let Some(item) = usize::try_from(n).ok().and_then(|n| display.items.get(n)) {
                            if matches!(item, Item::Backdrop { .. } | Item::Blended { .. }) {
                                self.read_ops(&mut ops, item, scene.scale, (0.0, 0.0), slot, visible);
                            } else {
                                self.item_ops(&mut ops, item, scene.scale, (0.0, 0.0), visible, 0, 1.0, text);
                            }
                        }
                    }
                }
            }
        }
        ops
    }

    /// `scene`, and `overlay` over it, drawn whole aside and read back: a
    /// scene taller than a texture may be (a whole page, for the verdict)
    /// is drawn in bands, each read back into its rows.
    pub(super) fn capture_scene(
        &mut self,
        scene: &Scene,
        overlay: Option<&Scene>,
        text: &mut Glyphs,
    ) -> Option<Pixels> {
        let (w, h) = (scene.size.0.max(1), scene.size.1.max(1));
        let max = self.device.limits().max_texture_dimension_2d;
        if w > max {
            return None;
        }
        let drawn = self.drawn;
        self.begin(text);
        let epoch = self.epoch;
        self.prepare(scene, text, false);
        if self.epoch != epoch {
            // It drew what the canvas does not show: the next frame plans whole.
            self.planned = None;
        }
        if let Some(o) = overlay {
            self.prepare(o, text, true);
        }
        let whole = PixelRect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        // Its clip tree is its own: the canvas's is parked until it ends
        // (or until the next frame, should it end early).
        let canvas_tree = std::mem::take(&mut self.clip_tree);
        self.parked_tree.get_or_insert(canvas_tree);
        self.clip_tree.update(&scene.properties, scene.scale, true, &[], &[]);
        let plan = self.plan(scene, whole);
        // The frame's tables stay the canvas's: the capture's are its own.
        let tables = self.tables.clone();
        self.tables.write_whole(scene, &plan, &self.kept, &self.clip_tree);
        let band = h.min(max);
        let mut out = Pixels::new(w, h)?;
        let target = self.offscreen((w, band));
        let mut built = Built::default();
        let mut top = 0;
        while top < h {
            let rows = band.min(h.saturating_sub(top));
            let origin = (0, i32::try_from(top).ok()?);
            let viewport = PixelRect {
                x: 0,
                y: origin.1,
                width: w,
                height: band,
            };
            let visible = rect_of(viewport);
            built.ops.clear();
            let mut passes = self.mask_passes();
            let mut ops = Vec::new();
            for e in plan.root.clone() {
                for op in self.entry_ops(scene, &plan, e, visible, &mut built, &mut passes, text) {
                    push_op(&mut ops, op);
                }
            }
            if let Some(o) = overlay {
                for op in self.overlay_ops(o, visible, text) {
                    push_op(&mut ops, op);
                }
            }
            passes.extend(self.tile_passes());
            passes.push(PassPlan {
                target: PassTarget::External,
                to_canvas: IDENTITY,
                origin,
                size: (w, band),
                clear: Some(clear_color(scene.background)),
                parts: vec![(None, ops)],
            });
            self.submit(&passes, Some(&target));
            let part = self.read_back(&target, (w, band))?;
            let row = usize::try_from(w.saturating_mul(4)).ok()?;
            let start = usize::try_from(top).ok()?.saturating_mul(row);
            let len = usize::try_from(rows).ok()?.saturating_mul(row);
            out.data_mut()
                .get_mut(start..start.saturating_add(len))?
                .copy_from_slice(part.data().get(..len)?);
            top = top.saturating_add(band);
        }
        // A capture is not a frame: what the last frame did stays as it was.
        self.drawn = drawn;
        self.unpark_tree();
        self.tables = tables;
        self.tables.relay();
        Some(out.with_scale(scene.scale))
    }

    /// The canvas's clip tree back, if a capture parked it.
    fn unpark_tree(&mut self) {
        if let Some(t) = self.parked_tree.take() {
            self.clip_tree = t;
        }
    }

    /// The canvas tiles rasterized this frame, as passes before the ones
    /// that show them.
    fn tile_passes(&mut self) -> Vec<PassPlan> {
        std::mem::take(&mut self.world.jobs)
            .into_iter()
            .map(|(tex, ops, first)| PassPlan {
                target: PassTarget::Tile(tex),
                to_canvas: IDENTITY,
                origin: (0, 0),
                size: (super::world::TILE, super::world::TILE),
                clear: first.then_some(wgpu::Color::TRANSPARENT),
                parts: vec![(None, ops)],
            })
            .collect()
    }

    /// The text-through masks waiting to be drawn, as passes.
    fn mask_passes(&mut self) -> Vec<PassPlan> {
        std::mem::take(&mut self.pending_masks)
            .into_iter()
            .map(|m| PassPlan {
                target: PassTarget::Mask(m.chunk, m.k),
                to_canvas: IDENTITY,
                origin: m.origin,
                size: m.size,
                clear: Some(wgpu::Color::TRANSPARENT),
                parts: vec![(None, m.ops)],
            })
            .collect()
    }

    /// The draws of canvas or layer entry `e`, where `visible` shows; a
    /// layer's passes go to `passes` first.
    #[expect(clippy::too_many_arguments, reason = "an entry and what its draws are built with")]
    fn entry_ops(
        &mut self,
        scene: &Scene,
        plan: &Plan,
        e: Entry,
        visible: [f32; 4],
        built: &mut Built,
        passes: &mut Vec<PassPlan>,
        text: &mut Glyphs,
    ) -> Vec<Op> {
        match e {
            Entry::Chunk(i) => {
                if let Some(ops) = built.ops.get(&i) {
                    return ops.clone();
                }
                let ops = self.chunk_ops(scene, plan, i, visible, text);
                built.ops.insert(i, ops.clone());
                ops
            }
            Entry::Layer(l) => self.layer_op(scene, plan, l, built, passes, text).into_iter().collect(),
        }
    }

    /// Layer `l` up to date (drawn again only when what it holds changed),
    /// and the draw compositing it.
    fn layer_op(
        &mut self,
        scene: &Scene,
        plan: &Plan,
        l: u32,
        built: &mut Built,
        passes: &mut Vec<PassPlan>,
        text: &mut Glyphs,
    ) -> Option<Op> {
        if let Some(op) = built.composites.get(&l) {
            return op.clone();
        }
        // Not drawn twice in a frame, whatever happens below.
        built.composites.insert(l, None);
        let lp = plan.layers.get(&l)?;
        let (content, region, entries) = (lp.content, lp.region, lp.entries.clone());
        let size = round_layer((region.width, region.height));
        let frame = self.frame;
        let kept = self
            .layers
            .get_mut(&l)
            .filter(|c| c.content == content && c.texture.size == size)
            .map(|c| c.used = frame)
            .is_some();
        if kept {
            self.drawn.layers_kept = self.drawn.layers_kept.saturating_add(1);
        } else {
            let visible = rect_of(region);
            let mut ops = Vec::new();
            let mut inner = Built::default();
            for e in entries {
                for op in self.entry_ops(scene, plan, e, visible, &mut inner, passes, text) {
                    push_op(&mut ops, op);
                }
            }
            let texture = match self.layers.remove(&l) {
                Some(c) if c.texture.size == size => c.texture,
                _ => self.layer_texture(size),
            };
            self.layers.insert(
                l,
                CachedLayer {
                    texture,
                    content,
                    used: frame,
                },
            );
            passes.push(PassPlan {
                target: PassTarget::Layer(l),
                to_canvas: scene
                    .properties
                    .effect(l)
                    .and_then(|n| compose(&scene.properties, n.transform, 0))
                    .unwrap_or(IDENTITY),
                origin: (region.x, region.y),
                size,
                clear: Some(wgpu::Color::TRANSPARENT),
                parts: vec![(None, ops)],
            });
            self.drawn.layers = self.drawn.layers.saturating_add(1);
        }
        let op = self.composite_op(scene, plan, l);
        built.composites.insert(l, op.clone());
        op
    }

    /// A record of this frame only: its instance.
    pub(super) fn transient_record(&mut self, words: &[u32]) -> Option<u32> {
        let len = u32::try_from(words.len()).ok()?;
        let r = self.records.alloc(len);
        self.records.write(r.start, words);
        let i = self.index.alloc(1);
        self.index.write(i.start, &[r.start]);
        self.transient.push(r);
        self.transient_index.push(i);
        Some(i.start)
    }

    /// A state slot of this frame only.
    fn transient_slot(&mut self, state: [u32; 4]) -> u32 {
        let slot = self.slots.take();
        self.transient_slots.push(slot);
        self.tables.set_slot(&mut self.props, slot, state);
        slot
    }

    /// The canvas's background over `size`.
    fn background_op(&mut self, color: Color, size: (u32, u32)) -> Op {
        let mut words = Vec::new();
        record::Solid {
            slot: 0,
            color: rgba(color),
            rect: [
                0.0,
                0.0,
                crate::convert::u32_to_f32(size.0),
                crate::convert::u32_to_f32(size.1),
            ],
            radii: None,
            inner: None,
            curve: None,
        }
        .push(&mut words);
        let at = self.transient_record(&words).unwrap_or(0);
        Op::Quads {
            extra: Extra::None,
            instances: at..at.saturating_add(1),
        }
    }

    /// The draw compositing layer `l` into its target.
    fn composite_op(&mut self, scene: &Scene, plan: &Plan, l: u32) -> Option<Op> {
        let lp = plan.layers.get(&l)?;
        let node = scene.properties.effect(l)?;
        let s = scene.scale;
        let region = pixel_rect(lp.region);
        let fwd = lp.m.unwrap_or(IDENTITY);
        let inverse = path::invert(fwd).unwrap_or([0.0; 6]);
        let q = self.layer_quad(plan, l)?;
        let mut filters = [0.0; 4];
        for (slot, f) in filters.as_chunks_mut::<2>().0.iter_mut().zip(&node.filters) {
            *slot = match f {
                Filter::Brightness(k) => [1.0, *k],
                Filter::Grayscale(a) => [2.0, *a],
            };
        }
        // What clips it clipped its content, inside the layer.
        let slot = self.transient_slot([0, 0, node.parent, 0]);
        let mut words = Vec::new();
        record::Layer {
            slot,
            opacity: node.opacity,
            quad: [q[0], q[1], q[2] - q[0], q[3] - q[1]],
            region,
            inverse,
            filters,
            mask: node.mask.as_ref().map(|g| line_and_stops(g, s)),
        }
        .push(&mut words);
        let at = self.transient_record(&words)?;
        Some(Op::Quads {
            extra: Extra::Layer(l),
            instances: at..at.saturating_add(1),
        })
    }

    /// Chunk `i`'s draws this frame, where `visible` (its target's pixels) shows.
    fn chunk_ops(&mut self, scene: &Scene, plan: &Plan, i: usize, visible: [f32; 4], text: &mut Glyphs) -> Vec<Op> {
        let props = &scene.properties;
        let s = scene.scale;
        let mut ops = Vec::new();
        let Some(c) = scene.chunks.get(i) else { return ops };
        let Some(k) = self.kept.get(&c.id) else { return ops };
        let (segs, base, display, slot) = (
            k.segs.clone(),
            k.index.map_or(0, |i| i.start),
            Arc::clone(&k.display),
            k.slot,
        );
        let fold = usize::try_from(c.state.effect)
            .ok()
            .and_then(|e| plan.fx.get(e))
            .map_or(1.0, |f| f.fold);
        let off = offset_of(props, c.state.transform, s);
        match plan.routes.get(i).copied().unwrap_or(Route::Skip) {
            Route::Skip => {}
            Route::Quads { target } => {
                let until = props.effect(target).map_or(0, |n| n.transform);
                let mut device = None;
                for seg in segs {
                    match seg {
                        Seg::Quads { extra, start, end } => push_op(
                            &mut ops,
                            Op::Quads {
                                extra,
                                instances: base.saturating_add(start)..base.saturating_add(end),
                            },
                        ),
                        Seg::Item(n) => {
                            let Some(item) = usize::try_from(n).ok().and_then(|n| display.items.get(n)) else {
                                continue;
                            };
                            if matches!(item, Item::Backdrop { .. } | Item::Blended { .. }) {
                                // Read in its box, whatever clips it: the
                                // draw is clipped by its slot.
                                self.read_ops(&mut ops, item, s, off, slot, visible);
                                continue;
                            }
                            let clip = *device.get_or_insert_with(|| self.pclip(props, c.state.clip, until));
                            // Cut to what its chain keeps where the target
                            // is the canvas's space.
                            let shown = match self.clip_tree.bounds(c.state.clip) {
                                Some(k) if compose(props, until, 0).is_none() => intersect(visible, k),
                                _ => visible,
                            };
                            self.item_ops(&mut ops, item, s, off, shown, clip, fold, text);
                        }
                    }
                }
            }
            Route::Matrix { target, m } => {
                let until = props.effect(target).map_or(0, |n| n.transform);
                let g = path::then([1.0, 0.0, 0.0, 1.0, off.0, off.1], m);
                // Its one chain, across the scales and turns: each clip
                // tested in its own space.
                let clip = self.pclip(props, c.state.clip, until);
                for item in &display.items {
                    self.matrix_ops(&mut ops, item, s, g, visible, clip, 0, fold, text);
                }
            }
        }
        ops
    }

    /// The path-and-curve clip entry of clip `id`'s walk, for a target in
    /// the space of transform `until` (the clip tree's entry, mapped from
    /// that target rather than the canvas; its next's entry after it).
    fn pclip(&mut self, props: &Properties, id: u32, until: u32) -> u32 {
        self.fr.pclip(&self.clip_tree, id, until, || compose(props, until, 0))
    }

    /// The draws of a chunk's path or curve glyph moved by `off`, drawn this frame.
    #[expect(clippy::too_many_arguments, reason = "an item and how it is drawn")]
    pub(super) fn item_ops(
        &mut self,
        ops: &mut Vec<Op>,
        item: &Item,
        s: f32,
        off: (f32, f32),
        visible: [f32; 4],
        device_clip: u32,
        alpha: f32,
        text: &mut Glyphs,
    ) {
        match item {
            Item::Path { origin, path, style } => {
                let m = [s, 0.0, 0.0, s, origin.0.mul_add(s, off.0), origin.1.mul_add(s, off.1)];
                self.push_path(ops, path, style, m, visible, device_clip, 0, alpha);
            }
            Item::Glyph { key, x, y, color } => {
                let m = glyph_matrix(
                    key,
                    crate::convert::i32_to_f32(*x) + off.0,
                    crate::convert::i32_to_f32(*y) + off.1,
                    None,
                );
                self.push_slug(ops, text, key, m, *color, alpha, device_clip, 0);
            }
            Item::Data(d) => {
                self.data.note(d, s, off);
                if let Some(at) = self
                    .data
                    .prepare(&self.device, &self.queue, d, s, off, device_clip, alpha)
                {
                    push_op(ops, Op::Data(at));
                }
            }
            Item::World(v) => self.world_ops(ops, v, s, off, visible, device_clip, alpha, text),
            _ => {}
        }
    }

    /// `item` drawn through `g` (its space to the target, a scale or a
    /// turn in it): quads as paths, glyphs as curves, icons as their paths.
    #[expect(clippy::too_many_arguments, reason = "an item and where it is drawn")]
    fn matrix_ops(
        &mut self,
        ops: &mut Vec<Op>,
        item: &Item,
        s: f32,
        g: [f32; 6],
        visible: [f32; 4],
        device: u32,
        local: u32,
        alpha: f32,
        text: &mut Glyphs,
    ) {
        let fill = |c: Color| PathStyle::fill(Gradient::Solid(c));
        let phys_bounds = |b: Bounds| Bounds {
            x: b.x * s,
            y: b.y * s,
            width: b.width * s,
            height: b.height * s,
        };
        let scaled = |r: Corners| Corners {
            top_left: r.top_left * s,
            top_right: r.top_right * s,
            bottom_right: r.bottom_right * s,
            bottom_left: r.bottom_left * s,
        };
        match item {
            Item::Rect { bounds, color } => {
                let path = Path::rounded_rect(phys_bounds(*bounds), Corners::default()).build();
                self.push_path(ops, &path, &fill(*color), g, visible, device, local, alpha);
            }
            Item::RoundedRect { bounds, radius, color } => {
                let path = Path::rounded_rect(phys_bounds(*bounds), scaled(*radius)).build();
                self.push_path(ops, &path, &fill(*color), g, visible, device, local, alpha);
            }
            Item::Band {
                outer,
                outer_radius,
                inner,
                inner_radius,
                color,
            } => {
                let path = Path::band(
                    phys_bounds(*outer),
                    scaled(*outer_radius),
                    phys_bounds(*inner),
                    scaled(*inner_radius),
                )
                .build();
                self.push_path(ops, &path, &fill(*color), g, visible, device, local, alpha);
            }
            Item::Superellipse {
                outer,
                outer_radius,
                inner,
                inner_radius,
                k,
                color,
            } => {
                let path = Path::superellipse_band(
                    phys_bounds(*outer),
                    scaled(*outer_radius),
                    phys_bounds(*inner),
                    scaled(*inner_radius),
                    *k,
                )
                .build();
                self.push_path(ops, &path, &fill(*color), g, visible, device, local, alpha);
            }
            Item::Gradient {
                bounds,
                radius,
                gradient,
            } => {
                let path = Path::rounded_rect(phys_bounds(*bounds), scaled(*radius)).build();
                // The gradient's line in the path's (physical) space.
                let paint = match gradient {
                    Gradient::Linear { start, end, stops } => Gradient::Linear {
                        start: (start.0 * s, start.1 * s),
                        end: (end.0 * s, end.1 * s),
                        stops: stops.clone(),
                    },
                    solid @ Gradient::Solid(_) => solid.clone(),
                };
                self.push_path(ops, &path, &PathStyle::fill(paint), g, visible, device, local, alpha);
            }
            Item::Glyph { key, x, y, color } => {
                let m = glyph_matrix(
                    key,
                    crate::convert::i32_to_f32(*x),
                    crate::convert::i32_to_f32(*y),
                    Some(g),
                );
                self.push_slug(ops, text, key, m, *color, alpha, device, local);
            }
            Item::Icon {
                icon,
                size,
                color,
                stroke_width,
                x,
                y,
            } => {
                let Some(drawing) = icon.drawing() else { return };
                let k = crate::convert::u32_to_f32(*size) / drawing.view.max(1e-3);
                let m = path::then(
                    [
                        k,
                        0.0,
                        0.0,
                        k,
                        crate::convert::i32_to_f32(*x),
                        crate::convert::i32_to_f32(*y),
                    ],
                    g,
                );
                for (part, style) in icon_parts(drawing, *stroke_width, *color) {
                    self.push_path(ops, &part, &style, m, visible, device, local, alpha);
                }
            }
            Item::Path { origin, path, style } => {
                let m = path::then([s, 0.0, 0.0, s, origin.0 * s, origin.1 * s], g);
                self.push_path(ops, path, style, m, visible, device, local, alpha);
            }
            // An effect is drawn through its matrix only when it holds none of these.
            // A read is never drawn through a matrix: an effect over it takes a layer.
            Item::Shadow { .. }
            | Item::Image { .. }
            | Item::TextThrough { .. }
            | Item::Data(_)
            | Item::World(_)
            | Item::Backdrop { .. }
            | Item::Blended { .. } => {}
        }
    }

    /// Path `path` through `m` (its space to the target).
    #[expect(clippy::too_many_arguments, reason = "a path and how it is drawn")]
    pub(super) fn push_path(
        &mut self,
        ops: &mut Vec<Op>,
        path: &Path,
        style: &PathStyle,
        m: [f32; 6],
        visible: [f32; 4],
        device_clip: u32,
        local_clip: u32,
        alpha: f32,
    ) {
        let (color, gradient) = path::paint_of(&style.paint, &mut self.fr.stops);
        let paint = Paint {
            color,
            gradient,
            alpha,
            device_clip,
            local_clip,
        };
        if let Some(d) = self
            .paths
            .draw(&self.device, &self.queue, path, style, m, visible, &paint, false)
        {
            push_op(ops, Op::Path(d.instances));
        }
    }

    /// Glyph `key` as curves, through `m` (curve pixels, y up, to the target).
    #[expect(clippy::too_many_arguments, reason = "a glyph and how it is drawn")]
    pub(super) fn push_slug(
        &mut self,
        ops: &mut Vec<Op>,
        text: &mut Glyphs,
        key: &GlyphKey,
        m: [f32; 6],
        color: Color,
        alpha: f32,
        device_clip: u32,
        local_clip: u32,
    ) {
        let skey = SlugKey::of(key, self.darkened);
        let Some(blob) = self.slugs.blob(&self.device, &self.queue, skey, || text.curves(key)) else {
            return;
        };
        let [r, g, b, _] = color.to_rgba8();
        let row =
            crate::convert::u32_to_f32(u32::try_from(MaskGamma::luminance_row(&self.gamma, [r, g, b])).unwrap_or(0));
        let index = u32::try_from(self.fr.slugs.len() / (SLUG * 4)).unwrap_or(u32::MAX);
        let mut c = rgba(color);
        c[3] *= alpha;
        self.fr.slugs.extend_from_slice(&blob.extents);
        self.fr.slugs.extend_from_slice(&[m[0], m[1], m[2], m[3]]);
        self.fr
            .slugs
            .extend_from_slice(&[m[4], m[5], f32::from_bits(blob.range.start), row]);
        self.fr.slugs.extend_from_slice(&c);
        self.fr.slugs.extend_from_slice(&[
            f32::from_bits(device_clip),
            f32::from_bits(local_clip),
            f32::from_bits(self.paths.clip_skip.word()),
            0.0,
        ]);
        push_op(ops, Op::Slug(index..index.saturating_add(1)));
    }

    /// Let go of layers not composited for a while, and the oldest past the budget.
    pub(super) fn end_layers(&mut self) {
        let frame = self.frame;
        let live = &self.layered;
        self.layers
            .retain(|e, l| live.contains(e) && frame.saturating_sub(l.used) < LAYER_FRAMES);
        let mut total: usize = self.layers.values().map(|l| l.texture.bytes()).sum();
        while total > LAYER_BUDGET {
            let Some((&oldest, _)) = self.layers.iter().min_by_key(|(_, l)| l.used) else {
                break;
            };
            if let Some(l) = self.layers.remove(&oldest) {
                total = total.saturating_sub(l.texture.bytes());
            }
        }
        for slot in std::mem::take(&mut self.transient_slots) {
            self.slots.give(slot);
        }
    }

    /// Upload what changed and draw `passes` (into `external` where they say).
    #[expect(clippy::too_many_lines, reason = "a frame's uploads and passes, in order")]
    fn submit(&mut self, passes: &[PassPlan], external: Option<&wgpu::Texture>) {
        let (device, queue) = (&self.device, &self.queue);
        let (mut bytes, bases) = self.tables.upload(&mut self.props, device, queue, self.slots.len());
        bytes = bytes
            .saturating_add(self.records.upload(device, queue))
            .saturating_add(self.index.upload(device, queue));
        let pclips = super::floats_to_bytes(&self.fr.pclips);
        bytes = bytes.saturating_add(self.pclips.update(device, queue, &pclips));
        let stops: &[f32] = if self.fr.stops.is_empty() {
            &[0.0; 8]
        } else {
            &self.fr.stops
        };
        bytes = bytes.saturating_add(self.stops.update(device, queue, &super::floats_to_bytes(stops)));
        let slugs: &[f32] = if self.fr.slugs.is_empty() {
            &[0.0; 4]
        } else {
            &self.fr.slugs
        };
        bytes = bytes.saturating_add(
            self.slug_instances
                .update(device, queue, &super::floats_to_bytes(slugs)),
        );
        let stride = usize::try_from(self.uniform_stride).unwrap_or(256);
        let mut globals = Vec::new();
        let mut block = |size: (u32, u32), origin: (i32, i32), to_canvas: [f32; 6]| {
            let mut b = vec![0_u8; stride];
            let [a, b_, c, d, e, f] = to_canvas;
            let block = super::contract::Globals {
                size: [crate::convert::u32_to_f32(size.0), crate::convert::u32_to_f32(size.1)],
                origin: [
                    crate::convert::i32_to_f32(origin.0),
                    crate::convert::i32_to_f32(origin.1),
                ],
                bases,
                to_canvas: [a, b_, c, d],
                to_canvas_t: [e, f],
                pad: [0.0; 2],
            };
            for (d, v) in b.iter_mut().zip(bytemuck::bytes_of(&block)) {
                *d = *v;
            }
            globals.extend(b);
        };
        for pass in passes {
            block(pass.size, pass.origin, pass.to_canvas);
        }
        // The icon coverage pages' passes, after the frame's.
        let side = self.mono.side();
        block((side, side), (0, 0), IDENTITY);
        if globals != self.globals_shown {
            self.globals.write(device, queue, &globals);
            bytes = bytes.saturating_add(u64::try_from(globals.len()).unwrap_or(0));
            self.globals_shown = globals;
        }
        bytes = bytes.saturating_add(self.paths.upload(&self.device, &self.queue));
        bytes = bytes.saturating_add(self.prepare_reads());
        self.refresh_bind_groups();
        let surface = if passes
            .iter()
            .any(|p| matches!(p.target, PassTarget::Blit | PassTarget::Surface))
        {
            self.window_view()
        } else {
            None
        };
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gui frame"),
        });
        self.paths.flush(&mut encoder);
        if let Some(c) = &self.bind_groups.compute
            && self.paths.pending.is_some()
        {
            let stamp = self.data.stamp(crate::chunk::runtime::Stamped::Paths);
            let stamp = stamp.and_then(|at| Some((self.data.query_set()?, at)));
            self.paths
                .encode(&mut encoder, &self.pipelines.compute, &c.group, stamp);
        }
        self.data.encode(
            &self.device,
            &self.queue,
            &mut encoder,
            (&self.globals.buffer, self.globals.generation),
            (&self.pclips.buffer, self.pclips.generation),
        );
        bytes = bytes.saturating_add(std::mem::take(&mut self.data.store.uploaded));
        let stride = self.uniform_stride;
        let pass_offset = |i: usize| u32::try_from(u64::try_from(i).unwrap_or(0).saturating_mul(stride)).unwrap_or(0);
        // Icons into their coverage pages first: the frame's passes read them.
        let mask_globals = passes.len();
        if let Some(paths) = &self.bind_groups.path {
            for m in &self.masks {
                let page = self.mono.page_view(m.page);
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("gui icon coverage"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &page,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                rp.set_pipeline(&self.pipelines.mask);
                rp.set_bind_group(0, &paths.group, &[pass_offset(mask_globals)]);
                rp.draw(0..4, m.instances.clone());
            }
        }
        self.masks.clear();
        let slug = if self.banded_ok == Some(false) {
            &self.pipelines.slug_full
        } else {
            &self.pipelines.slug_banded
        };
        let external_view = external.map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()));
        for (i, pass) in passes.iter().enumerate() {
            let stamp = self.data.stamp(crate::chunk::runtime::Stamped::Main);
            let target = match pass.target {
                PassTarget::Canvas => self.canvas.as_ref().map(|c| &c.view),
                PassTarget::Layer(l) => self.layers.get(&l).map(|c| &c.texture.view),
                PassTarget::Mask(id, k) => self
                    .kept
                    .get(&id)
                    .and_then(|kept| kept.masks.get(usize::try_from(k).ok()?))
                    .map(|t| &t.view),
                PassTarget::Blit | PassTarget::Surface => surface.as_ref(),
                PassTarget::External => external_view.as_ref(),
                PassTarget::Tile(id) => self.data.store.texture(id).map(|t| &t.view),
            };
            let Some(target) = target else { continue };
            // What a read copies from: the targets that can be read back.
            let source = match pass.target {
                PassTarget::Canvas => self.canvas.as_ref().map(|c| &c.texture),
                PassTarget::Layer(l) => self.layers.get(&l).map(|c| &c.texture.texture),
                PassTarget::External => external,
                _ => None,
            };
            let load = match pass.clear {
                Some(c) => wgpu::LoadOp::Clear(c),
                None => wgpu::LoadOp::Load,
            };
            let stamps = stamp.and_then(|at| Some((self.data.query_set()?, at)));
            // A read ends the pass and begins it again: the first part
            // stamps its start, the last its end.
            let reads = pass
                .parts
                .iter()
                .flat_map(|(_, ops)| ops)
                .filter(|o| matches!(o, Op::Read(_)))
                .count();
            let begin = |encoder: &mut wgpu::CommandEncoder, load: wgpu::LoadOp<wgpu::Color>, part: usize| {
                encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("gui pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: target,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        timestamp_writes: stamps.filter(|_| part == 0 || part == reads).map(|(set, at)| {
                            wgpu::RenderPassTimestampWrites {
                                query_set: set,
                                beginning_of_pass_write_index: (part == 0).then_some(at),
                                end_of_pass_write_index: (part == reads).then(|| at.saturating_add(1)),
                            }
                        }),
                        ..Default::default()
                    })
                    .forget_lifetime()
            };
            let mut part = 0_usize;
            let mut rp = Some(begin(&mut encoder, load, part));
            if matches!(pass.target, PassTarget::Blit) {
                if let (Some(c), Some(rp)) = (&self.canvas, rp.as_mut()) {
                    // Pixel for pixel, the canvas's size at most: the
                    // rest of a larger surface is the pass's clear.
                    let (sw, sh) = self.window.as_ref().map_or(c.size, |w| w.size());
                    rp.set_scissor_rect(0, 0, c.size.0.min(sw), c.size.1.min(sh));
                    rp.set_pipeline(&self.pipelines.blit);
                    rp.set_bind_group(0, &c.blit, &[]);
                    rp.draw(0..3, 0..1);
                }
                continue;
            }
            // An overlay over a surface not the canvas's size (a resize the
            // UI has not laid out yet) draws at the canvas's size, in the
            // corner with the canvas blitted there, cut to the surface.
            let mut drawn = pass.size;
            let mut fitted = false;
            if matches!(pass.target, PassTarget::Surface)
                && let Some(s) = self.window.as_ref().map(|w| w.size())
                && s != pass.size
                && let Some(rp) = rp.as_mut()
            {
                fitted = true;
                drawn = (pass.size.0.min(s.0), pass.size.1.min(s.1));
                let (w, h) = (
                    crate::convert::u32_to_f32(pass.size.0),
                    crate::convert::u32_to_f32(pass.size.1),
                );
                rp.set_viewport(0.0, 0.0, w, h, 0.0, 1.0);
                rp.set_scissor_rect(0, 0, drawn.0, drawn.1);
            }
            let offset = pass_offset(i);
            for (scissor, ops) in &pass.parts {
                let mut clipped = None;
                if let Some(r) = scissor {
                    let (w, h) = (
                        i32::try_from(drawn.0).unwrap_or(i32::MAX),
                        i32::try_from(drawn.1).unwrap_or(i32::MAX),
                    );
                    let (x0, y0) = (r.x0.clamp(0, w), r.y0.clamp(0, h));
                    let (x1, y1) = (r.x1.clamp(x0, w), r.y1.clamp(y0, h));
                    let (Ok(x), Ok(y), Ok(sw), Ok(sh)) = (
                        u32::try_from(x0),
                        u32::try_from(y0),
                        u32::try_from(x1.saturating_sub(x0)),
                        u32::try_from(y1.saturating_sub(y0)),
                    ) else {
                        continue;
                    };
                    if sw == 0 || sh == 0 {
                        continue;
                    }
                    clipped = Some((x, y, sw, sh));
                    if let Some(rp) = rp.as_mut() {
                        rp.set_scissor_rect(x, y, sw, sh);
                    }
                }
                // Which pipeline is bound: switched only when the next draw needs another.
                let mut bound: Option<u8> = None;
                let mut extra_bound: Option<Extra> = None;
                for d in ops {
                    if let Op::Read(k) = d {
                        // The pass ends (its draws are what is below), the
                        // read is copied and blurred, and it goes on over
                        // what it drew.
                        drop(rp.take());
                        if let Some(source) = source {
                            self.encode_read(&mut encoder, source, *k, pass.origin, pass.size, *scissor);
                        }
                        part = part.saturating_add(1);
                        let mut again = begin(&mut encoder, wgpu::LoadOp::Load, part);
                        if fitted {
                            let (w, h) = (
                                crate::convert::u32_to_f32(pass.size.0),
                                crate::convert::u32_to_f32(pass.size.1),
                            );
                            again.set_viewport(0.0, 0.0, w, h, 0.0, 1.0);
                            again.set_scissor_rect(0, 0, drawn.0, drawn.1);
                        }
                        if let Some((x, y, w, h)) = clipped {
                            again.set_scissor_rect(x, y, w, h);
                        }
                        rp = Some(again);
                        bound = None;
                        extra_bound = None;
                        continue;
                    }
                    let Some(rp) = rp.as_mut() else { continue };
                    match d {
                        Op::Quads { extra, instances } => {
                            let Some(g) = &self.bind_groups.quad else { continue };
                            if bound != Some(0) {
                                rp.set_pipeline(&self.pipelines.quad);
                                rp.set_bind_group(0, &g.group, &[offset]);
                                bound = Some(0);
                                extra_bound = None;
                            }
                            if extra_bound != Some(*extra) {
                                let group = match extra {
                                    Extra::None => &self.blank.bind,
                                    Extra::Image(k) => self
                                        .images
                                        .peek(k)
                                        .and_then(Option::as_ref)
                                        .map_or(&self.blank.bind, |i| &i.texture.bind),
                                    Extra::Layer(l) => self.layers.get(l).map_or(&self.blank.bind, |t| &t.texture.bind),
                                    Extra::Mask(id, k) => self
                                        .kept
                                        .get(id)
                                        .and_then(|kept| kept.masks.get(usize::try_from(*k).ok()?))
                                        .map_or(&self.blank.bind, |t| &t.bind),
                                    Extra::Read(k) => self.reads.texture(*k).map_or(&self.blank.bind, |t| &t.bind),
                                };
                                rp.set_bind_group(1, group, &[]);
                                extra_bound = Some(*extra);
                            }
                            rp.draw(0..4, instances.clone());
                        }
                        Op::Path(instances) => {
                            let Some(g) = &self.bind_groups.path else { continue };
                            if bound != Some(1) {
                                rp.set_pipeline(&self.pipelines.path);
                                rp.set_bind_group(0, &g.group, &[offset]);
                                bound = Some(1);
                            }
                            rp.draw(0..4, instances.clone());
                        }
                        Op::Slug(instances) => {
                            let Some(g) = &self.bind_groups.slug else { continue };
                            if bound != Some(2) {
                                rp.set_pipeline(slug);
                                rp.set_bind_group(0, &g.group, &[offset]);
                                bound = Some(2);
                            }
                            rp.draw(0..4, instances.clone());
                        }
                        Op::Fill(instance) => {
                            let Some(g) = &self.bind_groups.quad else { continue };
                            rp.set_pipeline(&self.pipelines.fill);
                            rp.set_bind_group(0, &g.group, &[offset]);
                            rp.set_bind_group(1, &self.blank.bind, &[]);
                            rp.draw(0..4, *instance..instance.saturating_add(1));
                            bound = None;
                            extra_bound = None;
                        }
                        Op::Data(d) => {
                            self.data.replay(rp, *d, offset);
                            bound = None;
                            extra_bound = None;
                        }
                        Op::Read(_) => {}
                    }
                }
            }
        }
        self.data.finish(&mut encoder);
        let uploads = self.data.store.take_uploads();
        self.queue.submit(uploads.into_iter().chain([encoder.finish()]));
        self.data.after_submit(&self.queue);
        if passes.iter().any(|p| matches!(p.target, PassTarget::Blit))
            && let Some(w) = self.window.as_mut()
        {
            if surface.is_some() {
                w.blitted = true;
            } else {
                // Meant to present and got no texture: what the canvas
                // holds is owed to the window, and the next frame presents
                // it, damage or not.
                w.stale = true;
            }
        }
        self.drawn.submitted = true;
        self.drawn.uploaded = self.drawn.uploaded.saturating_add(bytes);
    }

    /// Make again the bind groups whose buffers or textures were replaced.
    #[expect(clippy::too_many_lines, reason = "one bind group per pipeline")]
    fn refresh_bind_groups(&mut self) {
        let globals = |size: u64| wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &self.globals.buffer,
                offset: 0,
                size: wgpu::BufferSize::new(size),
            }),
        };
        fn buffer(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            }
        }
        let p = &self.paths;
        let mut made = 0_u64;
        let fresh =
            |slot: &mut Option<super::Cached>, key: Vec<u64>, make: &dyn Fn() -> wgpu::BindGroup, made: &mut u64| {
                if slot.as_ref().is_some_and(|c| c.key == key) {
                    return;
                }
                *made = made.saturating_add(1);
                *slot = Some(super::Cached { key, group: make() });
            };
        let quad_key = vec![
            self.globals.generation,
            self.records.generation,
            self.index.generation,
            self.props.generation,
            self.mono.generation,
            self.color.generation,
        ];
        fresh(
            &mut self.bind_groups.quad,
            quad_key,
            &|| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gui quads"),
                    layout: &self.layouts.quad,
                    entries: &[
                        globals(super::GLOBALS),
                        buffer(1, &self.records.buffer),
                        buffer(2, &self.index.buffer),
                        buffer(3, &self.props.buffer),
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(&self.mono.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::TextureView(&self.color.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: wgpu::BindingResource::TextureView(&self.gamma_view),
                        },
                    ],
                })
            },
            &mut made,
        );
        let path_key = vec![
            self.globals.generation,
            p.jobs.generation,
            p.draws.generation,
            p.tiles.generation,
            p.entries.generation,
            p.instances.generation,
            self.stops.generation,
            self.pclips.generation,
        ];
        fresh(
            &mut self.bind_groups.path,
            path_key,
            &|| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gui paths"),
                    layout: &self.layouts.path,
                    entries: &[
                        globals(super::contract::TARGET_GLOBALS),
                        buffer(1, &p.jobs.buffer),
                        buffer(2, &p.draws.buffer),
                        buffer(3, &p.tiles.buffer),
                        buffer(4, &p.entries.buffer),
                        buffer(5, &p.instances.buffer),
                        buffer(6, &self.stops.buffer),
                        buffer(7, &self.pclips.buffer),
                    ],
                })
            },
            &mut made,
        );
        let compute_key = vec![
            p.params.generation,
            p.curves.generation,
            p.work.generation,
            p.jobs.generation,
            p.scratch.generation,
            p.lines.generation,
            p.tiles.generation,
            p.entries.generation,
            p.instances.generation,
            p.state.generation,
        ];
        fresh(
            &mut self.bind_groups.compute,
            compute_key,
            &|| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gui path passes"),
                    layout: &self.layouts.compute,
                    entries: &[
                        buffer(0, &p.params.buffer),
                        buffer(1, &p.curves.buffer),
                        buffer(2, &p.work.buffer),
                        buffer(3, &p.jobs.buffer),
                        buffer(4, &p.scratch.buffer),
                        buffer(5, &p.lines.buffer),
                        buffer(6, &p.tiles.buffer),
                        buffer(7, &p.entries.buffer),
                        buffer(8, &p.instances.buffer),
                        buffer(9, &p.state.buffer),
                    ],
                })
            },
            &mut made,
        );
        let slug_key = vec![
            self.globals.generation,
            self.slug_instances.generation,
            self.slugs.blobs.generation,
            self.pclips.generation,
        ];
        fresh(
            &mut self.bind_groups.slug,
            slug_key,
            &|| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gui curve text"),
                    layout: &self.layouts.slug,
                    entries: &[
                        globals(super::contract::TARGET_GLOBALS),
                        buffer(1, &self.slug_instances.buffer),
                        buffer(2, &self.slugs.blobs.buffer),
                        buffer(3, &self.pclips.buffer),
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(&self.gamma_view),
                        },
                    ],
                })
            },
            &mut made,
        );
        self.bind_groups_made = self.bind_groups_made.saturating_add(made);
    }

    /// Whether banded curve text draws what the full scan draws on this
    /// adapter: one glyph drawn both ways and read back, once (ADR-0017).
    pub(super) fn slug_self_test(&mut self, text: &mut Glyphs) -> bool {
        let Some(key) = text.test_glyph(64.0) else { return true };
        let draw = |gpu: &mut Self, text: &mut Glyphs, full: bool| -> Option<Vec<u8>> {
            gpu.fr = FrameData::default();
            let mut ops = Vec::new();
            let m = glyph_matrix(&key, 8.0, 56.0, None);
            gpu.push_slug(&mut ops, text, &key, m, Color::BLACK, 1.0, 0, 0);
            let target = gpu.offscreen((64, 64));
            let banded = gpu.banded_ok;
            gpu.banded_ok = Some(!full);
            gpu.submit(
                &[PassPlan {
                    target: PassTarget::External,
                    to_canvas: IDENTITY,
                    origin: (0, 0),
                    size: (64, 64),
                    clear: Some(wgpu::Color::WHITE),
                    parts: vec![(None, ops)],
                }],
                Some(&target),
            );
            gpu.banded_ok = banded;
            gpu.read_back(&target, (64, 64)).map(|p| p.data().to_vec())
        };
        let drawn = self.drawn;
        let result = match (draw(self, text, false), draw(self, text, true)) {
            (Some(banded), Some(full)) => banded.iter().zip(&full).all(|(a, b)| a.abs_diff(*b) <= 2),
            _ => true,
        };
        self.drawn = drawn;
        result
    }
}

/// What a target is cleared to: `background` premultiplied, else nothing.
fn clear_color(background: Option<Color>) -> wgpu::Color {
    background.map_or(wgpu::Color::TRANSPARENT, |c| {
        let [r, g, b, a] = c.to_rgba8().map(|v| f64::from(v) / 255.0);
        wgpu::Color {
            r: r * a,
            g: g * a,
            b: b * a,
            a,
        }
    })
}

/// A path-and-curve clip entry for `c` (logical, snapped), in the target or,
/// with `g`, in the space `g` maps to the target, ending its walk; its id.
pub(super) fn clip_entry(clips: &mut Vec<f32>, c: Clip, s: f32, g: Option<[f32; 6]>) -> u32 {
    let rect = phys(c.bounds.snapped(s), s);
    let r = radii(c.radius, c.bounds, s);
    let (map, scale) = match g {
        None => (IDENTITY, 1.0),
        Some(g) => (path::invert(g).unwrap_or([0.0; 6]), path::scales(g).0),
    };
    let id = u32::try_from(clips.len() / ENTRY).unwrap_or(0);
    clips.extend_from_slice(&rect);
    clips.extend_from_slice(&r);
    clips.extend_from_slice(&[map[0], map[1], map[2], map[3]]);
    clips.extend_from_slice(&[map[4], map[5], scale, c.curve]);
    clips.extend_from_slice(&[0.0; 4]);
    id
}

/// Whether chunk `c` begins by painting all of `r` (physical pixels) opaque
/// where it stands in the window: an opaque rectangle under the root state.
fn opaque_over(scene: &Scene, c: usize, r: Rect) -> bool {
    let Some(chunk) = scene.chunks.get(c) else {
        return false;
    };
    let root = scene
        .properties
        .transforms
        .first()
        .is_none_or(|t| t.offset == (0.0, 0.0) && t.matrix.is_none());
    if chunk.state != crate::scene::State::default() || !root {
        return false;
    }
    let (bounds, color) = match chunk.display.items.first() {
        Some(Item::Rect { bounds, color }) => (*bounds, *color),
        Some(Item::RoundedRect { bounds, radius, color }) if radius.is_zero() => (*bounds, *color),
        _ => return false,
    };
    let [x, y, w, h] = phys(bounds, scene.scale);
    let f = crate::convert::i32_to_f32;
    color.a >= 1.0 && x <= f(r.x0) && y <= f(r.y0) && x + w >= f(r.x1) && y + h >= f(r.y1)
}
