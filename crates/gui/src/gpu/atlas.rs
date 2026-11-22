//! Atlas pages for small rasters drawn many times (ADR-0016): glyph and
//! icon coverage (one channel), colour glyphs and subpixel coverage (four).
//! An atlas is one texture array; each layer is a page packed with
//! etagere's shelf allocator, as GPUI packs its atlases. A full atlas adds a
//! page, copying the pages it has on the GPU into a texture array one layer
//! longer (nothing is rasterized again); at its most pages it empties the
//! page that is worth least, by how often it was drawn over how long ago
//! (`hits / (1 + age)`), never one drawn this frame or read by a chunk
//! kept on the GPU. Pages are never cleared wholesale and never doubled.

use etagere::{AllocId, AtlasAllocator, size2};

pub(crate) struct Atlas {
    label: &'static str,
    format: wgpu::TextureFormat,
    bytes_per_pixel: u32,
    side: u32,
    max_pages: u32,
    texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pages: Vec<Page>,
    /// Bumped each time the texture is made anew (a page added): bind
    /// groups holding the old view are made again.
    pub(crate) generation: u64,
}

struct Page {
    alloc: AtlasAllocator,
    hits: u64,
    used: u64,
    /// Chunks kept on the GPU that read it: it is not emptied while any do.
    readers: u32,
    /// Bumped when the page is emptied: slots of before are gone.
    generation: u32,
}

/// Where a raster sits: its page (layer) and top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    pub(crate) id: AllocId,
    pub(crate) page: u32,
    pub(crate) x: u32,
    pub(crate) y: u32,
    generation: u32,
}

/// What an allocation did beside finding room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Made {
    /// The room was there.
    Found,
    /// A page was added (the texture is new).
    Grew,
    /// Page `n` was emptied for it: what it held must be forgotten.
    Evicted(u32),
}

impl Atlas {
    pub(crate) fn new(
        device: &wgpu::Device,
        label: &'static str,
        format: wgpu::TextureFormat,
        bytes_per_pixel: u32,
        side: u32,
        max_pages: u32,
    ) -> Self {
        let (texture, view) = pages(device, label, format, side, 1);
        Self {
            label,
            format,
            bytes_per_pixel,
            side,
            max_pages: max_pages.max(1),
            texture,
            view,
            pages: vec![Page::new(side)],
            generation: 0,
        }
    }

    /// Room for a `width`×`height` raster drawn in `frame`, with a pixel of
    /// space around it so neighbours never bleed into each other; none when
    /// it is larger than a page or every page was drawn this frame.
    pub(crate) fn allocate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        frame: u64,
    ) -> Option<(Slot, Made)> {
        let (w, h) = (width.saturating_add(1), height.saturating_add(1));
        if w > self.side || h > self.side {
            return None;
        }
        for (i, p) in (0_u32..).zip(self.pages.iter_mut()) {
            if let Some(s) = p.take(i, w, h, frame) {
                return Some((s, Made::Found));
            }
        }
        if u32::try_from(self.pages.len()).unwrap_or(u32::MAX) < self.max_pages {
            self.add_page(device, queue);
            let i = u32::try_from(self.pages.len().saturating_sub(1)).unwrap_or(0);
            let s = self.pages.last_mut()?.take(i, w, h, frame)?;
            return Some((s, Made::Grew));
        }
        // Empty the page worth least, never one drawn this frame or read by
        // a chunk kept.
        let (victim, _) = (0_u32..)
            .zip(&self.pages)
            .filter(|(_, p)| p.used < frame && p.readers == 0)
            .map(|(i, p)| {
                let age = frame.saturating_sub(p.used);
                (
                    i,
                    crate::convert::u64_to_f64(p.hits) / crate::convert::u64_to_f64(age.saturating_add(1)),
                )
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        let p = self.pages.get_mut(usize::try_from(victim).ok()?)?;
        p.alloc = AtlasAllocator::new(size2(side_i32(self.side), side_i32(self.side)));
        p.hits = 0;
        p.generation = p.generation.wrapping_add(1);
        let s = p.take(victim, w, h, frame)?;
        Some((s, Made::Evicted(victim)))
    }

    /// A page more: the texture made one layer longer, the pages it had
    /// copied into it on the GPU.
    fn add_page(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let n = u32::try_from(self.pages.len()).unwrap_or(u32::MAX);
        let (texture, view) = pages(device, self.label, self.format, self.side, n.saturating_add(1));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gui atlas growth"),
        });
        encoder.copy_texture_to_texture(
            self.texture.as_image_copy(),
            texture.as_image_copy(),
            wgpu::Extent3d {
                width: self.side,
                height: self.side,
                depth_or_array_layers: n,
            },
        );
        queue.submit([encoder.finish()]);
        self.texture = texture;
        self.view = view;
        self.pages.push(Page::new(self.side));
        self.generation = self.generation.saturating_add(1);
    }

    /// `slot` drawn in `frame`.
    pub(crate) fn touch(&mut self, slot: &Slot, frame: u64) {
        if let Some(p) = self.pages.get_mut(usize::try_from(slot.page).unwrap_or(usize::MAX)) {
            p.hits = p.hits.saturating_add(1);
            p.used = p.used.max(frame);
        }
    }

    /// Page `page` is read by one more chunk kept on the GPU: it is not
    /// emptied while the chunk lives.
    pub(crate) fn pin_page(&mut self, page: u32) {
        if let Some(p) = self.pages.get_mut(usize::try_from(page).unwrap_or(usize::MAX)) {
            p.readers = p.readers.saturating_add(1);
        }
    }

    /// Page `page` is read by one chunk fewer, which last read it in `frame`.
    pub(crate) fn unpin_page(&mut self, page: u32, frame: u64) {
        if let Some(p) = self.pages.get_mut(usize::try_from(page).unwrap_or(usize::MAX)) {
            p.readers = p.readers.saturating_sub(1);
            p.used = p.used.max(frame);
        }
    }

    pub(crate) fn free(&mut self, slot: Slot) {
        if let Some(p) = self.pages.get_mut(usize::try_from(slot.page).unwrap_or(usize::MAX))
            && p.generation == slot.generation
        {
            p.alloc.deallocate(slot.id);
        }
    }

    /// Write `data` (rows of `width` pixels) into `slot`.
    pub(crate) fn upload(&self, queue: &wgpu::Queue, slot: Slot, width: u32, height: u32, data: &[u8]) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: slot.x,
                    y: slot.y,
                    z: slot.page,
                },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width.saturating_mul(self.bytes_per_pixel)),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// A view of page `page` alone, to draw into.
    pub(crate) fn page_view(&self, page: u32) -> wgpu::TextureView {
        self.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("gui atlas page"),
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: page,
            array_layer_count: Some(1),
            ..Default::default()
        })
    }

    pub(crate) fn side(&self) -> u32 {
        self.side
    }

    #[cfg(test)]
    pub(crate) fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// The pages' pixels held on the GPU.
    pub(crate) fn bytes(&self) -> usize {
        usize::try_from(
            u64::from(self.side)
                .saturating_mul(u64::from(self.side))
                .saturating_mul(u64::from(self.bytes_per_pixel))
                .saturating_mul(u64::try_from(self.pages.len()).unwrap_or(0)),
        )
        .unwrap_or(usize::MAX)
    }
}

impl Page {
    fn new(side: u32) -> Self {
        Self {
            alloc: AtlasAllocator::new(size2(side_i32(side), side_i32(side))),
            hits: 0,
            used: 0,
            readers: 0,
            generation: 0,
        }
    }

    fn take(&mut self, page: u32, w: u32, h: u32, frame: u64) -> Option<Slot> {
        let a = self.alloc.allocate(size2(side_i32(w), side_i32(h)))?;
        self.hits = self.hits.saturating_add(1);
        self.used = self.used.max(frame);
        Some(Slot {
            id: a.id,
            page,
            x: u32::try_from(a.rectangle.min.x).unwrap_or(0),
            y: u32::try_from(a.rectangle.min.y).unwrap_or(0),
            generation: self.generation,
        })
    }
}

fn side_i32(v: u32) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

fn pages(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    side: u32,
    layers: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some(label),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    (texture, view)
}
