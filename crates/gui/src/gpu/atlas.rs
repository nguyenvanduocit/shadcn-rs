//! Atlas pages for small rasters drawn many times: glyph coverage (one
//! channel) and colour glyphs and icons (four), packed with etagere's shelf
//! allocator as GPUI packs its atlases. A page that fills first lets go of
//! what the last frames did not draw, then doubles.

use etagere::{AllocId, AtlasAllocator, size2};

/// The largest page side; a raster that does not fit is not drawn.
const MAX_SIDE: u32 = 8192;
/// A new page's side.
const FIRST_SIDE: u32 = 1024;

pub(crate) struct Atlas {
    format: wgpu::TextureFormat,
    bytes_per_pixel: u32,
    side: u32,
    texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    alloc: AtlasAllocator,
    /// Bumped each time the page is made anew: slots of an older one are gone.
    pub(crate) generation: u64,
}

/// Where a raster sits in a page.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Slot {
    pub(crate) id: AllocId,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) generation: u64,
}

impl Atlas {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, bytes_per_pixel: u32) -> Self {
        let (texture, view) = page(device, format, FIRST_SIDE);
        Self {
            format,
            bytes_per_pixel,
            side: FIRST_SIDE,
            texture,
            view,
            alloc: AtlasAllocator::new(size2(side_i32(FIRST_SIDE), side_i32(FIRST_SIDE))),
            generation: 0,
        }
    }

    /// Room for a `width`×`height` raster, with a pixel of space around it
    /// so neighbours never bleed into each other.
    pub(crate) fn allocate(&mut self, width: u32, height: u32) -> Option<Slot> {
        let a = self.alloc.allocate(size2(
            side_i32(width.saturating_add(1)),
            side_i32(height.saturating_add(1)),
        ))?;
        Some(Slot {
            id: a.id,
            x: u32::try_from(a.rectangle.min.x).unwrap_or(0),
            y: u32::try_from(a.rectangle.min.y).unwrap_or(0),
            generation: self.generation,
        })
    }

    pub(crate) fn free(&mut self, slot: Slot) {
        if slot.generation == self.generation {
            self.alloc.deallocate(slot.id);
        }
    }

    /// Make the page twice as large and empty; false when it is as large as it gets.
    pub(crate) fn grow(&mut self, device: &wgpu::Device) -> bool {
        if self.side >= MAX_SIDE {
            return false;
        }
        self.side = self.side.saturating_mul(2);
        let (texture, view) = page(device, self.format, self.side);
        self.texture = texture;
        self.view = view;
        self.alloc = AtlasAllocator::new(size2(side_i32(self.side), side_i32(self.side)));
        self.generation = self.generation.saturating_add(1);
        true
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
                    z: 0,
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

    /// The page's pixels held on the GPU.
    pub(crate) fn bytes(&self) -> usize {
        usize::try_from(
            u64::from(self.side)
                .saturating_mul(u64::from(self.side))
                .saturating_mul(u64::from(self.bytes_per_pixel)),
        )
        .unwrap_or(usize::MAX)
    }
}

fn side_i32(v: u32) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

fn page(device: &wgpu::Device, format: wgpu::TextureFormat, side: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gui atlas"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}
