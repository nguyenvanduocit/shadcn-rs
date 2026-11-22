//! The GPU side of data chunks' data (ADR-0018), on the render thread:
//! buffers and textures kinds ask for, each at most [`super::PAGE_BYTES`],
//! and every series page drawn, kept once however many chunks read it.
//!
//! A series page's buffer holds, as `u32` words, its points' x high words,
//! x low words, y (and y's low part, for a precise series), then its
//! pyramid levels (six words a node, [`super::series::Node`]), laid out for
//! its capacity ([`PageLayout`]). Only what was appended since the last
//! upload is written (the new points and the pyramid nodes over them); a
//! page that outgrows its buffer moves, on the GPU, into one twice as
//! large.
//! Writes go through one staging belt into an upload encoder submitted
//! just before the frame's, in the order they were made (a page's move to
//! a larger buffer is a copy in the same encoder), so a frame's many small
//! writes make no buffer of their own. Past [`FLUSH`] bytes the uploads so
//! far are submitted alone, so a very large first load is split across
//! submits.

use std::sync::Arc;

use super::series::{FANOUT, MAX_PAGE, MIN_PAGE, Page, SeriesSnapshot};

/// Bytes written before the queue is flushed with a submit of its own.
pub const FLUSH: u64 = 64 << 20;
/// A page not drawn for this many frames lets go of its buffer.
const PAGE_FRAMES: u64 = 240;

/// A buffer issued to a kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferId(pub(crate) u32);

/// A texture issued to a kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureId(pub(crate) u32);

pub(crate) struct StoreBuffer {
    pub buffer: wgpu::Buffer,
    pub generation: u64,
}

pub(crate) struct StoreTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
    pub generation: u64,
}

/// Where each part of a page's words starts, for a capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageLayout {
    pub cap: u32,
    pub x_hi: u32,
    pub x_lo: u32,
    pub y: u32,
    /// y's low parts; equal to `y` when the series is not precise.
    pub lo: u32,
    /// Level 1 of the pyramid.
    pub levels: u32,
    pub words: u32,
}

impl PageLayout {
    #[must_use]
    pub fn new(cap: usize, precise: bool) -> Self {
        let c = u32::try_from(cap).unwrap_or(u32::MAX);
        let lo = if precise {
            c.saturating_mul(3)
        } else {
            c.saturating_mul(2)
        };
        let levels = if precise {
            c.saturating_mul(4)
        } else {
            c.saturating_mul(3)
        };
        let mut words = levels;
        let mut n = cap;
        while n > 1 {
            n = n.div_ceil(FANOUT);
            words = words.saturating_add(u32::try_from(n.saturating_mul(6)).unwrap_or(u32::MAX));
        }
        Self {
            cap: c,
            x_hi: 0,
            x_lo: c,
            y: c.saturating_mul(2),
            lo,
            levels,
            words: words.max(4),
        }
    }

    /// Where level `k` (from 1) starts.
    #[must_use]
    pub fn level(&self, k: usize) -> u32 {
        let mut at = self.levels;
        let mut n = usize::try_from(self.cap).unwrap_or(0);
        for _ in 1..k {
            n = n.div_ceil(FANOUT);
            at = at.saturating_add(u32::try_from(n.saturating_mul(6)).unwrap_or(u32::MAX));
        }
        at
    }
}

/// A series page on the GPU.
pub(crate) struct GpuPage {
    pub buffer: wgpu::Buffer,
    pub layout: PageLayout,
    /// Points written.
    pub len: usize,
    pub generation: u64,
    pub used: u64,
}

/// A series' pages as a frame draws them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidentPage {
    pub seq: u64,
    /// Where the page starts in the snapshot, and its points in it.
    pub at: u32,
    pub len: u32,
    pub layout: PageLayout,
    /// x of its first and last point.
    pub x_first: i64,
    pub x_last: i64,
}

pub(crate) struct Store {
    buffers: Vec<Option<StoreBuffer>>,
    free_buffers: Vec<u32>,
    textures: Vec<Option<StoreTexture>>,
    free_textures: Vec<u32>,
    pub(crate) pages: foldhash::HashMap<u64, GpuPage>,
    generation: u64,
    /// Bytes written this frame, and since the last flush.
    pub uploaded: u64,
    since_flush: u64,
    frame: u64,
    belt: Option<wgpu::util::StagingBelt>,
    uploads: Option<wgpu::CommandEncoder>,
}

impl Store {
    pub(crate) fn new() -> Self {
        Self {
            belt: None,
            uploads: None,
            buffers: Vec::new(),
            free_buffers: Vec::new(),
            textures: Vec::new(),
            free_textures: Vec::new(),
            pages: foldhash::HashMap::default(),
            generation: 0,
            uploaded: 0,
            since_flush: 0,
            frame: 0,
        }
    }

    fn next_generation(&mut self) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.generation
    }

    pub(crate) fn buffer(&self, id: BufferId) -> Option<&StoreBuffer> {
        self.buffers.get(usize::try_from(id.0).ok()?)?.as_ref()
    }

    pub(crate) fn texture(&self, id: TextureId) -> Option<&StoreTexture> {
        self.textures.get(usize::try_from(id.0).ok()?)?.as_ref()
    }

    /// A buffer of at least `size` bytes (at most a page) in `slot`: the one
    /// there when it is large enough, else a new one (its contents gone).
    pub(crate) fn ensure_buffer(
        &mut self,
        device: &wgpu::Device,
        slot: &mut Option<BufferId>,
        size: u64,
        usage: wgpu::BufferUsages,
    ) -> Option<BufferId> {
        let size = size.max(16).next_multiple_of(16);
        if size > super::PAGE_BYTES {
            return None;
        }
        if let Some(id) = *slot
            && self.buffer(id).is_some_and(|b| b.buffer.size() >= size)
        {
            return Some(id);
        }
        let generation = self.next_generation();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gui chunk data"),
            size: size.next_power_of_two().min(super::PAGE_BYTES).max(size),
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entry = StoreBuffer { buffer, generation };
        let id = match *slot {
            Some(id) => {
                if let Some(b) = self.buffers.get_mut(usize::try_from(id.0).ok()?) {
                    *b = Some(entry);
                }
                id
            }
            None => {
                let id = if let Some(i) = self.free_buffers.pop() {
                    if let Some(b) = self.buffers.get_mut(usize::try_from(i).ok()?) {
                        *b = Some(entry);
                    }
                    i
                } else {
                    self.buffers.push(Some(entry));
                    u32::try_from(self.buffers.len().saturating_sub(1)).ok()?
                };
                BufferId(id)
            }
        };
        *slot = Some(id);
        Some(id)
    }

    pub(crate) fn release_buffer(&mut self, id: BufferId) {
        if let Some(b) = usize::try_from(id.0).ok().and_then(|i| self.buffers.get_mut(i))
            && b.take().is_some()
        {
            self.free_buffers.push(id.0);
        }
    }

    pub(crate) fn ensure_texture(
        &mut self,
        device: &wgpu::Device,
        slot: &mut Option<TextureId>,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> Option<TextureId> {
        let size = (size.0.max(1), size.1.max(1));
        if let Some(id) = *slot
            && self.texture(id).is_some_and(|t| t.size == size && t.format == format)
        {
            return Some(id);
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui chunk texture"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: usage | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let generation = self.next_generation();
        let entry = StoreTexture {
            texture,
            view,
            size,
            format,
            generation,
        };
        let id = match *slot {
            Some(id) => {
                if let Some(t) = self.textures.get_mut(usize::try_from(id.0).ok()?) {
                    *t = Some(entry);
                }
                id
            }
            None => {
                let id = if let Some(i) = self.free_textures.pop() {
                    if let Some(t) = self.textures.get_mut(usize::try_from(i).ok()?) {
                        *t = Some(entry);
                    }
                    i
                } else {
                    self.textures.push(Some(entry));
                    u32::try_from(self.textures.len().saturating_sub(1)).ok()?
                };
                TextureId(id)
            }
        };
        *slot = Some(id);
        Some(id)
    }

    pub(crate) fn release_texture(&mut self, id: TextureId) {
        if let Some(t) = usize::try_from(id.0).ok().and_then(|i| self.textures.get_mut(i))
            && t.take().is_some()
        {
            self.free_textures.push(id.0);
        }
    }

    /// Write `data` at `offset` of `id`, counted.
    pub(crate) fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, id: BufferId, offset: u64, data: &[u8]) {
        let Some(b) = self.buffer(id) else { return };
        let end = offset.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
        if data.is_empty() || end > b.buffer.size() {
            return;
        }
        let buffer = b.buffer.clone();
        self.stage(device, queue, &buffer, offset, data);
    }

    /// `data` into `target` at `offset`, in the upload encoder, counted
    /// (nothing when empty).
    pub(crate) fn stage(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::Buffer,
        offset: u64,
        data: &[u8],
    ) {
        let len = u64::try_from(data.len()).unwrap_or(0);
        let Some(size) = wgpu::BufferSize::new(len) else { return };
        if !len.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT) || !offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT) {
            // Not a whole number of words: the queue's own staging takes it,
            // after what was staged so far.
            self.flush(queue);
            queue.write_buffer(target, offset, data);
        } else {
            let (belt, enc) = self.encoders(device);
            belt.write_buffer(enc, target, offset, size).copy_from_slice(data);
        }
        self.count(queue, len);
    }

    /// The belt and the upload encoder, made the first time.
    fn encoders(&mut self, device: &wgpu::Device) -> (&mut wgpu::util::StagingBelt, &mut wgpu::CommandEncoder) {
        let belt = self
            .belt
            .get_or_insert_with(|| wgpu::util::StagingBelt::new(device.clone(), 1 << 20));
        let enc = self.uploads.get_or_insert_with(|| {
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gui chunk uploads"),
            })
        });
        (belt, enc)
    }

    /// The uploads staged since the last, to submit before the frame's
    /// commands (the belt's chunks come back once they ran).
    pub(crate) fn take_uploads(&mut self) -> Option<wgpu::CommandBuffer> {
        let enc = self.uploads.take()?;
        if let Some(belt) = self.belt.as_mut() {
            belt.finish_and_recall_on_submit(&enc);
        }
        Some(enc.finish())
    }

    /// Submit what was staged so far, alone.
    fn flush(&mut self, queue: &wgpu::Queue) {
        if let Some(cb) = self.take_uploads() {
            queue.submit([cb]);
        }
        self.since_flush = 0;
    }

    /// `bytes` written: past [`FLUSH`] since the last, the uploads are submitted.
    fn count(&mut self, queue: &wgpu::Queue, bytes: u64) {
        self.uploaded = self.uploaded.saturating_add(bytes);
        self.since_flush = self.since_flush.saturating_add(bytes);
        if self.since_flush >= FLUSH {
            self.flush(queue);
        }
    }

    pub(crate) fn write_texture(&mut self, queue: &wgpu::Queue, id: TextureId, data: &[u8], bytes_per_row: u32) {
        // After what was staged so far, as written.
        self.flush(queue);
        let Some(t) = self.texture(id) else { return };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &t.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: t.size.0,
                height: t.size.1,
                depth_or_array_layers: 1,
            },
        );
        self.count(queue, u64::try_from(data.len()).unwrap_or(0));
    }

    /// Bring `series`' pages up to it on the GPU: what each holds beyond
    /// what was written. The pages as the frame draws them.
    pub(crate) fn sync_series(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        series: &SeriesSnapshot,
    ) -> Vec<ResidentPage> {
        let precise = series.precise();
        let mut out = Vec::new();
        for (page, n, at) in series.pages() {
            if n == 0 {
                continue;
            }
            let layout = self.sync_page(device, queue, page, n, precise);
            let d = page.read();
            out.push(ResidentPage {
                seq: page.seq,
                at,
                len: u32::try_from(n).unwrap_or(u32::MAX),
                layout,
                x_first: d.x.first().copied().unwrap_or(0),
                x_last: d.x.get(n.saturating_sub(1)).copied().unwrap_or(0),
            });
        }
        out
    }

    fn sync_page(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        page: &Arc<Page>,
        n: usize,
        precise: bool,
    ) -> PageLayout {
        let frame = self.frame;
        let fits = self
            .pages
            .get(&page.seq)
            .is_some_and(|g| g.layout.cap >= u32::try_from(n).unwrap_or(u32::MAX));
        if !fits {
            let cap = n.next_power_of_two().clamp(MIN_PAGE, MAX_PAGE);
            let layout = PageLayout::new(cap, precise);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gui series page"),
                size: u64::from(layout.words).saturating_mul(4),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            // What the old buffer holds moves over on the GPU: only new
            // points are ever uploaded.
            self.encoders(device);
            let kept = match (self.pages.get(&page.seq), self.uploads.as_mut()) {
                (Some(old), Some(enc)) => grow(enc, old, &buffer, layout, precise),
                _ => 0,
            };
            let generation = self.next_generation();
            self.pages.insert(
                page.seq,
                GpuPage {
                    buffer,
                    layout,
                    len: kept,
                    generation,
                    used: frame,
                },
            );
        }
        let Some(g) = self.pages.get_mut(&page.seq) else {
            return PageLayout::new(MIN_PAGE, precise);
        };
        g.used = frame;
        let layout = g.layout;
        let from = g.len;
        if from >= n {
            return layout;
        }
        g.len = n;
        let buffer = g.buffer.clone();
        let page = Arc::clone(page);
        let d = page.read();
        let mut put = |word: u32, data: &[u32]| {
            self.stage(
                device,
                queue,
                &buffer,
                u64::from(word).saturating_mul(4),
                bytemuck::cast_slice(data),
            );
        };
        let w = |i: usize| u32::try_from(i).unwrap_or(u32::MAX);
        // In slices of a million points, so the words made stay small.
        let mut at = from;
        while at < n {
            let end = n.min(at.saturating_add(1 << 20));
            let xs = d.x.get(at..end).unwrap_or(&[]);
            let hi: Vec<u32> = xs.iter().map(|x| split(*x).0).collect();
            let lo: Vec<u32> = xs.iter().map(|x| split(*x).1).collect();
            put(layout.x_hi.saturating_add(w(at)), &hi);
            put(layout.x_lo.saturating_add(w(at)), &lo);
            let ys: Vec<u32> = d.y.get(at..end).unwrap_or(&[]).iter().map(|y| y.to_bits()).collect();
            put(layout.y.saturating_add(w(at)), &ys);
            if precise && let Some(l) = d.lo.as_ref() {
                let ls: Vec<u32> = l.get(at..end).unwrap_or(&[]).iter().map(|y| y.to_bits()).collect();
                put(layout.lo.saturating_add(w(at)), &ls);
            }
            at = end;
        }
        // The pyramid nodes over the new points (the one over the old tail too).
        let mut span = 1_usize;
        for (k, level) in d.levels.iter().enumerate() {
            span = span.saturating_mul(FANOUT);
            let a = from.checked_div(span).unwrap_or(0);
            let b = n.div_ceil(span).min(level.len());
            if a >= b {
                continue;
            }
            let words: Vec<u32> = level
                .get(a..b)
                .unwrap_or(&[])
                .iter()
                .flat_map(|node| node.words())
                .collect();
            put(
                layout.level(k.saturating_add(1)).saturating_add(w(a).saturating_mul(6)),
                &words,
            );
        }
        drop(d);
        layout
    }

    pub(crate) fn page(&self, seq: u64) -> Option<&GpuPage> {
        self.pages.get(&seq)
    }

    /// The frame is done: let go of pages not drawn for a while.
    pub(crate) fn end_frame(&mut self) {
        self.frame = self.frame.saturating_add(1);
        let frame = self.frame;
        self.pages.retain(|_, p| frame.saturating_sub(p.used) < PAGE_FRAMES);
        self.since_flush = 0;
    }

    /// Bytes it holds on the GPU.
    pub(crate) fn bytes(&self) -> u64 {
        let b: u64 = self.buffers.iter().flatten().map(|b| b.buffer.size()).sum();
        let p: u64 = self.pages.values().map(|p| p.buffer.size()).sum();
        let t: u64 = self
            .textures
            .iter()
            .flatten()
            .map(|t| {
                let px = u64::from(t.size.0).saturating_mul(u64::from(t.size.1));
                px.saturating_mul(u64::from(t.format.block_copy_size(None).unwrap_or(4)))
            })
            .sum();
        b.saturating_add(p).saturating_add(t)
    }
}

/// Copy `old`'s points and pyramid into `new` (laid out as `layout`) in
/// the upload encoder `enc`, before the writes that follow; the points
/// copied.
fn grow(enc: &mut wgpu::CommandEncoder, old: &GpuPage, new: &wgpu::Buffer, layout: PageLayout, precise: bool) -> usize {
    let n = old.len;
    if n == 0 {
        return 0;
    }
    let bytes = |words: usize| u64::try_from(words).unwrap_or(0).saturating_mul(4);
    let mut copy = |from: u32, to: u32, words: usize| {
        if words > 0 {
            enc.copy_buffer_to_buffer(
                &old.buffer,
                u64::from(from).saturating_mul(4),
                new,
                u64::from(to).saturating_mul(4),
                bytes(words),
            );
        }
    };
    let o = old.layout;
    copy(o.x_hi, layout.x_hi, n);
    copy(o.x_lo, layout.x_lo, n);
    copy(o.y, layout.y, n);
    if precise {
        copy(o.lo, layout.lo, n);
    }
    let mut count = usize::try_from(o.cap).unwrap_or(0);
    let mut len = n;
    let mut k = 1_usize;
    while count > 1 {
        count = count.div_ceil(FANOUT);
        len = len.div_ceil(FANOUT);
        copy(o.level(k), layout.level(k), len.saturating_mul(6));
        k = k.saturating_add(1);
    }
    n
}

/// An `i64` as its high word (the sign's) and low word, as the GPU reads it.
#[must_use]
pub fn split(x: i64) -> (u32, u32) {
    let b = x.to_ne_bytes();
    let u = u64::from_ne_bytes(b);
    let hi = u32::try_from(u >> 32).unwrap_or(0);
    let lo = u32::try_from(u & 0xffff_ffff).unwrap_or(0);
    (hi, lo)
}
