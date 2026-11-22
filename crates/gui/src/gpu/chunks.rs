//! Paint chunks kept on the GPU (ADR-0016): a chunk's items are encoded
//! once into quad records ([`super::record`]) and instances, written into
//! buffers kept across frames, and replayed while the chunk is the same (its
//! recording, the text settings, whether it may draw subpixel text). What a
//! chunk draws each frame anew is only what cannot be kept as a quad: a
//! vector path (its tiles are kept by [`super::path`]) and a glyph drawn
//! from its curves. A chunk also keeps what the renderer decides with
//! (where its primitives lie, whether all of it can be drawn transformed)
//! and the atlas pages and images it reads, which are kept while it lives.

use std::sync::Arc;

use super::frame::MaskJob;
use super::record::{self, KIND_COLOR, KIND_LCD, KIND_MASK, KIND_MONO};
use super::words::Span;
use super::{Extra, GpuRenderer, Page, Texture, image_key, phys, radii, rgba, wants_curves};
use crate::image::ImageSource;
use crate::paint::grow;
use crate::scene::{Chunk, ChunkId, Display, Gradient, Item, PixelRect, Scene};
use crate::text::gamma::MaskGamma;
use crate::text::{Coverage, Glyphs};

/// A chunk as the GPU holds it.
pub(super) struct Kept {
    /// What it was encoded from.
    pub generation: u64,
    pub straight: bool,
    pub text: Option<u64>,
    pub scale: u32,
    /// Its encoding's own number: what damage compares.
    pub epoch: u64,
    /// Its state slot.
    pub slot: u32,
    pub records: Option<Span>,
    pub index: Option<Span>,
    pub segs: Vec<Seg>,
    /// Where its primitives lie in its space (physical x0, y0, x1, y1),
    /// what an effect over it asks to know whether they overlap.
    pub rects: Vec<[f32; 4]>,
    /// What it may paint in its space, reach included.
    pub bounds: Option<[f32; 4]>,
    /// The atlas pages it reads: coverage, colour.
    pub pages: [u64; 2],
    pub images: Vec<ImageSource>,
    /// The canvases it shows (their keys): their tiles' readiness is part
    /// of what it shows.
    pub canvases: Vec<u64>,
    /// Its text-through masks.
    pub masks: Vec<Texture>,
    /// Every item can be drawn under a scale or a turn (quads as paths,
    /// glyphs as curves).
    pub drawable: bool,
    pub display: Arc<Display>,
    pub seen: u64,
    /// Of this frame only (an overlay): let go when the frame ends.
    pub transient: bool,
    /// It holds its atlas pages and images ([`GpuRenderer::unpin`]).
    pub pinned: bool,
}

/// A run of what a chunk draws.
#[derive(Clone, Copy, Debug)]
pub(super) enum Seg {
    /// Instances `start..end` of the chunk's.
    Quads { extra: Extra, start: u32, end: u32 },
    /// Item `n`, drawn each frame it is (a path, curve text).
    Item(u32),
}

/// Records being encoded.
#[derive(Default)]
pub(super) struct Encoding {
    pub words: Vec<u32>,
    /// Where each record starts in `words`.
    pub starts: Vec<u32>,
    pub segs: Vec<Seg>,
    pub rects: Vec<[f32; 4]>,
    pub bounds: Option<[f32; 4]>,
    pub pages: [u64; 2],
    pub images: Vec<ImageSource>,
    pub canvases: Vec<u64>,
    pub drawable: bool,
    /// Text-through masks: their region and items.
    pub masks: Vec<(PixelRect, Vec<Item>)>,
}

impl Encoding {
    fn record(&mut self, extra: Extra, push: impl FnOnce(&mut Vec<u32>)) {
        let at = u32::try_from(self.words.len()).unwrap_or(u32::MAX);
        push(&mut self.words);
        self.starts.push(at);
        let n = u32::try_from(self.starts.len()).unwrap_or(u32::MAX);
        if let Some(Seg::Quads { extra: e, end, .. }) = self.segs.last_mut()
            && *e == extra
            && *end == n.saturating_sub(1)
        {
            *end = n;
            return;
        }
        self.segs.push(Seg::Quads {
            extra,
            start: n.saturating_sub(1),
            end: n,
        });
    }

    fn rect(&mut self, r: [f32; 4]) {
        if r[2] > r[0] && r[3] > r[1] {
            self.rects.push(r);
        }
    }

    fn reach(&mut self, r: Option<[f32; 4]>) {
        if let Some(r) = r {
            self.bounds = Some(
                self.bounds
                    .map_or(r, |b| [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[2]), b[3].max(r[3])]),
            );
        }
    }
}

/// `r` (x, y, width, height) as x0, y0, x1, y1.
fn corners(r: [f32; 4]) -> [f32; 4] {
    [r[0], r[1], r[0] + r[2], r[1] + r[3]]
}

impl GpuRenderer {
    /// Every chunk of `scene` on the GPU: kept when it was encoded from what
    /// it is now, else encoded. What a chunk leaving the scene read (its
    /// atlas pages) is let go first, so making room for what is new may
    /// take it; what the chunks staying read is never taken.
    pub(super) fn prepare(&mut self, scene: &Scene, text: &mut Glyphs, transient: bool) {
        let opaque = scene.background.is_some_and(|c| c.to_rgba8()[3] == 255);
        let frame = self.frame;
        let (generation, scale) = (self.text_generation, scene.scale);
        let mut fresh: Vec<(&Chunk, bool)> = Vec::new();
        for chunk in &scene.chunks {
            let straight = opaque && chunk.state.effect == 0;
            match self.kept.get_mut(&chunk.id) {
                Some(k) if fits(k, chunk, straight, generation, scale) => {
                    k.seen = frame;
                    k.transient |= transient;
                }
                _ => fresh.push((chunk, straight)),
            }
            if transient {
                self.transient_ids.push(chunk.id);
            }
        }
        if !transient {
            self.swept = true;
            let leaving: Vec<ChunkId> = self
                .kept
                .iter()
                .filter(|(_, k)| k.seen != frame && k.pinned)
                .map(|(id, _)| *id)
                .collect();
            for id in leaving {
                if let Some(mut k) = self.kept.remove(&id) {
                    self.unpin(&mut k);
                    self.kept.insert(id, k);
                }
            }
        }
        for (chunk, straight) in fresh {
            self.encode(chunk, straight, scene.scale, text, transient);
        }
    }

    /// The chunks of `scene` at `chunks` (recorded again) on the GPU: what
    /// a frame planned from what changed encodes.
    pub(super) fn prepare_changed(&mut self, scene: &Scene, chunks: &[usize], text: &mut Glyphs) {
        let opaque = scene.background.is_some_and(|c| c.to_rgba8()[3] == 255);
        for &i in chunks {
            let Some(chunk) = scene.chunks.get(i) else { continue };
            let straight = opaque && chunk.state.effect == 0;
            if !self
                .kept
                .get(&chunk.id)
                .is_some_and(|k| fits(k, chunk, straight, self.text_generation, scene.scale))
            {
                self.encode(chunk, straight, scene.scale, text, false);
            }
        }
    }

    /// `k` no longer holds the atlas pages it reads, nor its images.
    pub(super) fn unpin(&mut self, k: &mut Kept) {
        if !k.pinned {
            return;
        }
        k.pinned = false;
        // It read them through the last frame, not this one.
        let frame = self.frame.saturating_sub(1);
        for (bits, atlas) in [(k.pages[0], &mut self.mono), (k.pages[1], &mut self.color)] {
            for page in 0..64_u32 {
                if bits & (1_u64 << page) != 0 {
                    atlas.unpin_page(page, frame);
                }
            }
        }
        for source in &k.images {
            let key = image_key(source);
            if let Some(r) = self.image_refs.get_mut(&key) {
                *r = r.saturating_sub(1);
                if *r == 0 {
                    self.image_refs.remove(&key);
                }
            }
        }
    }

    /// `k` holds the atlas pages it reads, and its images, while it lives.
    fn pin(&mut self, k: &mut Kept) {
        if k.pinned {
            return;
        }
        k.pinned = true;
        for (bits, atlas) in [(k.pages[0], &mut self.mono), (k.pages[1], &mut self.color)] {
            for page in 0..64_u32 {
                if bits & (1_u64 << page) != 0 {
                    atlas.pin_page(page);
                }
            }
        }
        for source in &k.images {
            let r = self.image_refs.entry(image_key(source)).or_default();
            *r = r.saturating_add(1);
        }
    }

    /// Encode `chunk`, keeping its slot and, when they fit, its ranges.
    fn encode(&mut self, chunk: &Chunk, straight: bool, s: f32, text: &mut Glyphs, transient: bool) {
        let mut old = self.kept.remove(&chunk.id);
        if let Some(k) = old.as_mut() {
            self.unpin(k);
        }
        let slot = old.as_ref().map_or_else(|| self.slots.take(), |k| k.slot);
        let mut enc = Encoding {
            drawable: true,
            ..Encoding::default()
        };
        self.encode_items(&chunk.display.items, s, straight, slot, chunk.id, &mut enc, text);
        self.epoch = self.epoch.saturating_add(1);
        if !transient {
            self.drawn.encoded = self.drawn.encoded.saturating_add(1);
        }
        let words = u32::try_from(enc.words.len()).unwrap_or(u32::MAX);
        let count = u32::try_from(enc.starts.len()).unwrap_or(u32::MAX);
        let (old_records, old_index) = old.as_ref().map_or((None, None), |k| (k.records, k.index));
        let records = place(&mut self.records, old_records, words);
        let index = place(&mut self.index, old_index, count);
        if let Some(r) = records {
            self.records.write(r.start, &enc.words);
            if let Some(i) = index {
                let at: Vec<u32> = enc.starts.iter().map(|w| w.saturating_add(r.start)).collect();
                self.index.write(i.start, &at);
            }
        }
        let masks = self.encode_masks(chunk.id, &enc.masks, s, text);
        let mut kept = Kept {
            generation: chunk.display.generation,
            straight,
            text: self.text_generation,
            scale: s.to_bits(),
            epoch: self.epoch,
            slot,
            records,
            index,
            segs: enc.segs,
            rects: enc.rects,
            bounds: enc.bounds,
            pages: enc.pages,
            images: enc.images,
            canvases: enc.canvases,
            masks,
            drawable: enc.drawable,
            display: Arc::clone(&chunk.display),
            seen: self.frame,
            transient,
            pinned: false,
        };
        self.pin(&mut kept);
        self.kept.insert(chunk.id, kept);
    }

    /// The text-through masks of chunk `id`: a texture each, drawn this
    /// frame from records of this frame only.
    fn encode_masks(
        &mut self,
        id: ChunkId,
        masks: &[(PixelRect, Vec<Item>)],
        s: f32,
        text: &mut Glyphs,
    ) -> Vec<Texture> {
        let mut out = Vec::new();
        for (k, (region, items)) in (0_u32..).zip(masks) {
            let size = (region.width.max(1), region.height.max(1));
            out.push(self.layer_texture(size));
            let mut enc = Encoding::default();
            self.encode_items(items, s, false, 0, id, &mut enc, text);
            let ops = self.transient_ops(&enc, items, s, *region, text);
            self.pending_masks.push(MaskJob {
                chunk: id,
                k,
                origin: (region.x, region.y),
                size,
                ops,
            });
        }
        out
    }

    /// Records of this frame only for `enc` (a mask, an overlay), and the
    /// draws of its items, where `region` shows.
    fn transient_ops(
        &mut self,
        enc: &Encoding,
        items: &[Item],
        s: f32,
        region: PixelRect,
        text: &mut Glyphs,
    ) -> Vec<super::Op> {
        let words = u32::try_from(enc.words.len()).unwrap_or(u32::MAX);
        let count = u32::try_from(enc.starts.len()).unwrap_or(u32::MAX);
        let records = (words > 0).then(|| self.records.alloc(words));
        let index = (count > 0).then(|| self.index.alloc(count));
        if let (Some(r), Some(i)) = (records, index) {
            self.records.write(r.start, &enc.words);
            let at: Vec<u32> = enc.starts.iter().map(|w| w.saturating_add(r.start)).collect();
            self.index.write(i.start, &at);
        }
        self.transient.extend(records);
        self.transient_index.extend(index);
        let visible = super::frame::rect_of(region);
        let mut ops = Vec::new();
        for seg in &enc.segs {
            match *seg {
                Seg::Quads { extra, start, end } => {
                    let base = index.map_or(0, |i| i.start);
                    super::push_op(
                        &mut ops,
                        super::Op::Quads {
                            extra,
                            instances: base.saturating_add(start)..base.saturating_add(end),
                        },
                    );
                }
                Seg::Item(n) => {
                    if let Some(item) = usize::try_from(n).ok().and_then(|n| items.get(n)) {
                        self.item_ops(&mut ops, item, s, (0.0, 0.0), visible, 0, 1.0, text);
                    }
                }
            }
        }
        ops
    }

    /// Encode `items` (in their space) as records under `slot`.
    #[expect(clippy::too_many_arguments, reason = "items and how they are encoded")]
    #[expect(clippy::too_many_lines, reason = "one arm per primitive")]
    pub(super) fn encode_items(
        &mut self,
        items: &[Item],
        s: f32,
        straight: bool,
        slot: u32,
        id: ChunkId,
        enc: &mut Encoding,
        text: &mut Glyphs,
    ) {
        for (n, item) in (0_u32..).zip(items) {
            enc.reach(item.reach(s));
            match item {
                Item::Rect { bounds, color } => {
                    let rect = phys(*bounds, s);
                    enc.rect(corners(rect));
                    enc.record(Extra::None, |out| {
                        record::Solid {
                            slot,
                            color: rgba(*color),
                            rect,
                            radii: None,
                            inner: None,
                        }
                        .push(out);
                    });
                }
                Item::RoundedRect { bounds, radius, color } => {
                    let rect = phys(*bounds, s);
                    enc.rect(corners(rect));
                    enc.record(Extra::None, |out| {
                        record::Solid {
                            slot,
                            color: rgba(*color),
                            rect,
                            radii: Some(radii(*radius, s)),
                            inner: None,
                        }
                        .push(out);
                    });
                }
                Item::Band {
                    outer,
                    outer_radius,
                    inner,
                    inner_radius,
                    color,
                } => {
                    let rect = phys(*outer, s);
                    enc.rect(corners(rect));
                    let cut = (!inner.is_empty()).then(|| (phys(*inner, s), radii(*inner_radius, s)));
                    enc.record(Extra::None, |out| {
                        record::Solid {
                            slot,
                            color: rgba(*color),
                            rect,
                            radii: Some(radii(*outer_radius, s)),
                            inner: cut,
                        }
                        .push(out);
                    });
                }
                Item::Shadow { bounds, radius, shadow } => {
                    let shape = bounds.translated(shadow.x, shadow.y).outset(shadow.spread);
                    if shape.is_empty() {
                        continue;
                    }
                    enc.drawable = false;
                    if let Some(r) = item.reach(s) {
                        enc.rect(r);
                    }
                    let sigma = shadow.blur * s / 2.0;
                    enc.record(Extra::None, |out| {
                        record::Shadow {
                            slot,
                            color: rgba(shadow.color),
                            shape: phys(shape, s),
                            shape_radii: radii(grow(*radius, shadow.spread), s),
                            caster: phys(*bounds, s),
                            caster_radii: radii(*radius, s),
                            sigma,
                        }
                        .push(out);
                    });
                }
                Item::Gradient {
                    bounds,
                    radius,
                    gradient,
                } => {
                    let rect = phys(*bounds, s);
                    enc.rect(corners(rect));
                    let (line, stops) = line_and_stops(gradient, s);
                    enc.record(Extra::None, |out| {
                        record::Gradient {
                            slot,
                            rect,
                            radii: radii(*radius, s),
                            line,
                            stops,
                        }
                        .push(out);
                    });
                }
                Item::Glyph { key, x, y, color } => {
                    if key.is_color() {
                        enc.drawable = false;
                    }
                    if wants_curves(key, false) {
                        if let Some(r) = item.reach(s) {
                            enc.rect(r);
                        }
                        enc.segs.push(Seg::Item(n));
                        continue;
                    }
                    if key.is_color() && !self.foreground.contains_key(key) {
                        self.foreground.insert(*key, text.uses_foreground(key));
                    }
                    let gs = self.glyph_slot(key, *color, straight);
                    let Some(g) = self.glyph(gs, *color, text) else {
                        continue;
                    };
                    let at = (
                        crate::convert::i32_to_f32(x.saturating_add(g.left)),
                        crate::convert::i32_to_f32(y.saturating_sub(g.top)),
                    );
                    enc.rect([
                        at.0,
                        at.1,
                        at.0 + crate::convert::u32_to_f32(g.width),
                        at.1 + crate::convert::u32_to_f32(g.height),
                    ]);
                    let kind = match (g.page, gs.coverage) {
                        (Page::Mono, _) => KIND_MONO,
                        (Page::Color, Coverage::Lcd(_)) => KIND_LCD,
                        (Page::Color, Coverage::Gray) => KIND_COLOR,
                    };
                    let [r, gr, b, _] = color.to_rgba8();
                    let row = |v: usize| u32::try_from(v).unwrap_or(0);
                    let [cr, cg, cb] = MaskGamma::channel_rows(&self.gamma, [r, gr, b]);
                    mark(&mut enc.pages, g.page, g.slot.page);
                    let words = record::Glyph {
                        kind,
                        page: g.slot.page,
                        rows: [
                            row(MaskGamma::luminance_row(&self.gamma, [r, gr, b])),
                            row(cr),
                            row(cg),
                            row(cb),
                        ],
                        slot,
                        color: rgba(*color),
                        at,
                        size: (g.width, g.height),
                        texel: (g.slot.x, g.slot.y),
                    }
                    .words();
                    enc.record(Extra::None, |out| out.extend(words));
                }
                Item::Icon {
                    icon,
                    size,
                    color,
                    stroke_width,
                    x,
                    y,
                } => {
                    let Some(g) = self.icon(*icon, *size, *stroke_width) else {
                        continue;
                    };
                    let at = (crate::convert::i32_to_f32(*x), crate::convert::i32_to_f32(*y));
                    enc.rect([
                        at.0,
                        at.1,
                        at.0 + crate::convert::u32_to_f32(g.width),
                        at.1 + crate::convert::u32_to_f32(g.height),
                    ]);
                    mark(&mut enc.pages, Page::Mono, g.slot.page);
                    let words = record::Glyph {
                        kind: KIND_MASK,
                        page: g.slot.page,
                        rows: [0; 4],
                        slot,
                        color: rgba(*color),
                        at,
                        size: (g.width, g.height),
                        texel: (g.slot.x, g.slot.y),
                    }
                    .words();
                    enc.record(Extra::None, |out| out.extend(words));
                }
                Item::Image { source, bounds, radius } => {
                    enc.drawable = false;
                    if !self.image(source) {
                        continue;
                    }
                    enc.images.push(source.clone());
                    let rect = phys(*bounds, s);
                    enc.rect(corners(rect));
                    let (iw, ih) = source.size();
                    let (iw, ih) = (
                        crate::convert::u32_to_f32(iw).max(1.0),
                        crate::convert::u32_to_f32(ih).max(1.0),
                    );
                    let fit = (rect[2] / iw).max(rect[3] / ih);
                    let placed = [
                        rect[0] + (rect[2] - iw * fit) / 2.0,
                        rect[1] + (rect[3] - ih * fit) / 2.0,
                        iw * fit,
                        ih * fit,
                    ];
                    enc.record(Extra::Image(image_key(source)), |out| {
                        record::Image {
                            slot,
                            rect,
                            radii: radii(*radius, s),
                            placed,
                        }
                        .push(out);
                    });
                }
                Item::Path { .. } => {
                    if let Some(r) = item.reach(s) {
                        enc.rect(r);
                    }
                    enc.segs.push(Seg::Item(n));
                }
                Item::Data(_) | Item::World(_) => {
                    // Drawn by its kind each frame, through no matrix: under
                    // a scale or turn its effect takes a layer.
                    enc.drawable = false;
                    if let Item::World(v) = item {
                        enc.canvases.push(v.key);
                    }
                    if let Some(r) = item.reach(s) {
                        enc.rect(r);
                    }
                    enc.segs.push(Seg::Item(n));
                }
                Item::TextThrough {
                    region,
                    mask,
                    background,
                    fill,
                } => {
                    enc.drawable = false;
                    let k = u32::try_from(enc.masks.len()).unwrap_or(u32::MAX);
                    enc.masks.push((*region, mask.clone()));
                    let r = super::frame::pixel_rect(*region);
                    enc.rect(corners(r));
                    let fill = fill.as_ref().map(|(g, shown)| {
                        let (line, stops) = line_and_stops(g, s);
                        (phys(*shown, s), line, stops)
                    });
                    let through = record::TextThrough {
                        slot,
                        region: r,
                        background: background.map(|(b, c)| (rgba(c), phys(b, s))),
                        fill,
                    };
                    enc.record(Extra::Mask(id, k), |out| through.push(out));
                }
            }
        }
    }

    /// Let go of the chunks this frame did not draw, and of what was this
    /// frame's only.
    pub(super) fn end_kept(&mut self) {
        let frame = self.frame;
        let gone: Vec<ChunkId> = if std::mem::take(&mut self.swept) {
            // The scene was gone over whole: what it did not hold leaves.
            self.kept
                .iter()
                .filter(|(_, k)| k.seen != frame || k.transient)
                .map(|(id, _)| *id)
                .collect()
        } else {
            // Planned from what changed, the scene holds what it held:
            // only what was drawn for this frame alone leaves.
            self.transient_ids
                .iter()
                .copied()
                .filter(|id| self.kept.get(id).is_some_and(|k| k.transient))
                .collect()
        };
        self.transient_ids.clear();
        let any = !gone.is_empty();
        for id in gone {
            let Some(mut k) = self.kept.remove(&id) else { continue };
            self.unpin(&mut k);
            self.slots.give(k.slot);
            if let Some(r) = k.records {
                self.records.release(r);
            }
            if let Some(i) = k.index {
                self.index.release(i);
            }
        }
        for r in std::mem::take(&mut self.transient) {
            self.records.release(r);
        }
        for i in std::mem::take(&mut self.transient_index) {
            self.index.release(i);
        }
        if any {
            let kept = &self.kept;
            self.pending_masks.retain(|m| kept.contains_key(&m.chunk));
        }
    }
}

/// Whether `k` was encoded from `chunk` as it is now: under text settings
/// `text`, at `scale`.
fn fits(k: &Kept, chunk: &Chunk, straight: bool, text: Option<u64>, scale: f32) -> bool {
    k.generation == chunk.display.generation && k.straight == straight && k.text == text && k.scale == scale.to_bits()
}

/// `len` words of `words`, in `old` when it is that long.
fn place(words: &mut super::words::Words, old: Option<Span>, len: u32) -> Option<Span> {
    match old {
        Some(o) if o.len == len => Some(o),
        _ => {
            if let Some(o) = old {
                words.release(o);
            }
            (len > 0).then(|| words.alloc(len))
        }
    }
}

/// Atlas page `page` of `atlas` is read.
fn mark(pages: &mut [u64; 2], atlas: Page, page: u32) {
    let i = match atlas {
        Page::Mono => 0,
        Page::Color => 1,
    };
    if let Some(bits) = pages.get_mut(i)
        && page < 64
    {
        *bits |= 1_u64 << page;
    }
}

/// A gradient's line (physical) and stops (straight colours).
pub(super) fn line_and_stops(g: &Gradient, s: f32) -> ([f32; 4], Vec<(f32, [f32; 4])>) {
    match g {
        Gradient::Solid(c) => ([0.0; 4], vec![(0.0, rgba(*c))]),
        Gradient::Linear { start, end, stops } => (
            [start.0 * s, start.1 * s, end.0 * s, end.1 * s],
            stops.iter().map(|(o, c)| (*o, rgba(*c))).collect(),
        ),
    }
}
