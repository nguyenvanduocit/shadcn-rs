//! The GPU renderer: a [`Scene`] drawn with wgpu (Metal on macOS, Vulkan or
//! GL on Linux, DX12 on Windows), learned from GPUI's renderers. Every
//! primitive is an instance of one quad pipeline whose fragment shader
//! computes coverage from signed distances (rounded boxes, borders, rings,
//! clips) or reads it from an atlas (glyphs, icons); box shadows are
//! Gaussian-blurred rounded boxes in closed form. Groups (opacity, masks,
//! filters, transforms) are drawn into layer textures first and composited
//! where the scene puts them. Per frame the CPU only lays out the
//! instances: no pixel is touched by it.
//!
//! The CPU renderer ([`crate::cpu`]) paints the same scene; the two are held
//! to each other's pixels.

mod atlas;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use cosmic_text::{CacheKey, SwashContent};
use tiny_skia::Pixmap;

use self::atlas::{Atlas, Slot};
use crate::cache::FrameCache;
use crate::color::Color;
use crate::convert::{i32_to_f32, u32_to_f32};
use crate::geometry::Bounds;
use crate::icon::Icon;
use crate::image::ImageSource;
use crate::inspect::Raster;
use crate::paint::grow;
use crate::scene::{Clip, Gradient, Group, Held, Item, PixelRect, Renderer, Scene};
use crate::shape::{PathCommand, Shape};
use crate::style::{Corners, Filter};
use crate::text::TextSystem;

/// vec4s per primitive (`PRIM` in the shader).
const PRIM: usize = 16;
/// Uniform offsets must be multiples of this.
const UNIFORM_STRIDE: u64 = 256;

const KIND_SOLID: f32 = 0.0;
const KIND_SHADOW: f32 = 1.0;
const KIND_GRADIENT: f32 = 2.0;
const KIND_MONO: f32 = 3.0;
const KIND_COLOR: f32 = 4.0;
const KIND_IMAGE: f32 = 5.0;
const KIND_RASTER: f32 = 6.0;
const KIND_LAYER: f32 = 7.0;
const KIND_TEXT_THROUGH: f32 = 8.0;

/// Why the GPU renderer could not start.
#[derive(Debug)]
pub struct GpuError(String);

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GpuError {}

/// A window's surface and the frame being drawn into it.
struct Window {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    frame: Option<wgpu::SurfaceTexture>,
}

/// Which atlas page a raster went to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Mono,
    Color,
}

#[derive(Clone, Copy, Debug)]
struct Placed {
    page: Page,
    slot: Slot,
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    used: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct IconKey {
    icon: Icon,
    size: u32,
    rgba: [u8; 4],
    stroke: u32,
}

struct Texture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
}

/// An image on the GPU; holds the image so its address names no other.
struct ImageTexture {
    _source: ImageSource,
    texture: Texture,
}

/// Shapes rasterised once, wherever they are drawn.
struct ShapeRaster {
    texture: Texture,
}

/// The texture an instance reads beside the atlases.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Extra {
    None,
    Image(usize),
    Shapes(u64),
    Layer(usize),
}

struct Draw {
    extra: Extra,
    instances: std::ops::Range<u32>,
}

enum Target {
    Main,
    Layer(usize),
}

struct Pass {
    target: Target,
    origin: (i32, i32),
    clear: wgpu::Color,
    draws: Vec<Draw>,
}

/// Turns a scene into instances, stops and passes.
#[derive(Default)]
struct Encoder {
    prims: Vec<f32>,
    stops: Vec<f32>,
    passes: Vec<Pass>,
    /// The region each layer of this frame covers.
    layers: Vec<PixelRect>,
    /// The part of the scene drawn.
    viewport: PixelRect,
}

/// One primitive's sixteen vec4s.
struct Prim([[f32; 4]; PRIM]);

impl Prim {
    fn new(kind: f32, quad: [f32; 4]) -> Self {
        let mut f = [[0.0; 4]; PRIM];
        f[0] = quad;
        f[1] = [kind, 0.0, 0.0, 0.0];
        // No clip.
        f[7] = [0.0, 0.0, -1.0, -1.0];
        Self(f)
    }
}

/// The GPU renderer, for a window or headless.
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    globals_layout: wgpu::BindGroupLayout,
    extra_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// A transparent pixel, for draws that read no extra texture.
    blank: wgpu::BindGroup,
    window: Option<Window>,
    mono: Atlas,
    color: Atlas,
    glyphs: HashMap<CacheKey, Option<Placed>>,
    icons: HashMap<IconKey, Option<Placed>>,
    images: FrameCache<usize, Option<ImageTexture>>,
    shapes: FrameCache<u64, Option<ShapeRaster>>,
    /// Layer textures kept for the next frames, by size: as many as the
    /// last two frames used at once.
    layer_pool: Vec<Texture>,
    /// The most layers one scene used this frame, and in the frame before.
    layers_peak: (usize, usize),
    /// Time spent waiting for the surface's next texture, since last asked.
    waited: std::time::Duration,
    prim_buffer: wgpu::Buffer,
    stop_buffer: wgpu::Buffer,
    globals_buffer: wgpu::Buffer,
    frame: u64,
}

impl GpuRenderer {
    /// A renderer drawing into `window`.
    ///
    /// # Errors
    ///
    /// When no adapter or device answers, or the surface cannot be made.
    pub fn for_window(window: Arc<winit::window::Window>) -> Result<Self, GpuError> {
        let instance = instance();
        let size = window.inner_size();
        let surface = instance
            .create_surface(window)
            .map_err(|e| GpuError(format!("surface: {e}")))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .map_err(|e| GpuError(format!("adapter: {e}")))?;
        let caps = surface.get_capabilities(&adapter);
        // Blending happens in the sRGB-encoded values, as tiny-skia and
        // browsers blend: never an sRGB view, which would blend in linear light.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .ok_or_else(|| GpuError("no 8-bit unorm surface format".to_owned()))?;
        let mut renderer = Self::with_adapter(&adapter, format)?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps
                .alpha_modes
                .iter()
                .copied()
                .find(|m| *m == wgpu::CompositeAlphaMode::Opaque)
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&renderer.device, &config);
        renderer.window = Some(Window {
            surface,
            config,
            frame: None,
        });
        Ok(renderer)
    }

    /// A renderer with no window: it draws only what [`Renderer::capture`] reads back.
    ///
    /// # Errors
    ///
    /// When no adapter or device answers.
    pub fn headless() -> Result<Self, GpuError> {
        let instance = instance();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|e| GpuError(format!("adapter: {e}")))?;
        Self::with_adapter(&adapter, wgpu::TextureFormat::Rgba8Unorm)
    }

    fn with_adapter(adapter: &wgpu::Adapter, format: wgpu::TextureFormat) -> Result<Self, GpuError> {
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("gui"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| GpuError(format!("device: {e}")))?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gui shaders"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });
        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let storage_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gui globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
                storage_entry(1),
                storage_entry(2),
                texture_entry(3),
                texture_entry(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let extra_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gui extra"),
            entries: &[texture_entry(0)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gui"),
            bind_group_layouts: &[Some(&globals_layout), Some(&extra_layout)],
            immediate_size: 0,
        });
        let blend = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gui primitives"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: blend,
                        alpha: blend,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gui linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let blank_texture = texture(
            &device,
            &queue,
            (1, 1),
            wgpu::TextureFormat::Rgba8Unorm,
            Some(&[0, 0, 0, 0]),
        );
        let blank = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gui blank"),
            layout: &extra_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&blank_texture.view),
            }],
        });
        let buffer = |label: &str, usage: wgpu::BufferUsages, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Ok(Self {
            mono: Atlas::new(&device, wgpu::TextureFormat::R8Unorm, 1),
            color: Atlas::new(&device, wgpu::TextureFormat::Rgba8Unorm, 4),
            prim_buffer: buffer("gui prims", wgpu::BufferUsages::STORAGE, 1 << 16),
            stop_buffer: buffer("gui stops", wgpu::BufferUsages::STORAGE, 1 << 12),
            globals_buffer: buffer("gui globals", wgpu::BufferUsages::UNIFORM, UNIFORM_STRIDE * 16),
            device,
            queue,
            format,
            pipeline,
            globals_layout,
            extra_layout,
            sampler,
            blank,
            window: None,
            glyphs: HashMap::new(),
            icons: HashMap::new(),
            images: FrameCache::default(),
            shapes: FrameCache::default(),
            layer_pool: Vec::new(),
            layers_peak: (0, 0),
            waited: std::time::Duration::ZERO,
            frame: 0,
        })
    }

    /// Make the window's surface `size` physical pixels.
    pub fn resize(&mut self, size: (u32, u32)) {
        let Some(w) = self.window.as_mut() else { return };
        let size = (size.0.max(1), size.1.max(1));
        if (w.config.width, w.config.height) != size {
            w.frame = None;
            w.config.width = size.0;
            w.config.height = size.1;
            w.surface.configure(&self.device, &w.config);
        }
    }

    /// The window's frame texture, taken from the surface when first asked for in a frame.
    fn window_view(&mut self) -> Option<wgpu::TextureView> {
        let w = self.window.as_mut()?;
        if w.frame.is_none() {
            let asked = std::time::Instant::now();
            let texture = match w.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    w.surface.configure(&self.device, &w.config);
                    match w.surface.get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                        _ => return None,
                    }
                }
                _ => return None,
            };
            w.frame = Some(texture);
            self.waited = self.waited.saturating_add(asked.elapsed());
        }
        let frame = w.frame.as_ref()?;
        Some(frame.texture.create_view(&wgpu::TextureViewDescriptor::default()))
    }

    /// Draw `scene` into `view` (`scene.size`), over what it holds unless
    /// the scene has a background.
    fn draw(&mut self, scene: &Scene, text: &mut TextSystem, view: &wgpu::TextureView) {
        self.draw_part(scene, text, view, (0, 0), scene.size);
    }

    /// Draw the part of `scene` from `origin`, `size` physical pixels, into `view`.
    fn draw_part(
        &mut self,
        scene: &Scene,
        text: &mut TextSystem,
        view: &wgpu::TextureView,
        origin: (i32, i32),
        size: (u32, u32),
    ) {
        // Rasters first: a page that has to grow empties, so make them again.
        for _ in 0..3 {
            if self.prepare(&scene.items, scene, text) {
                break;
            }
        }
        let mut enc = Encoder {
            viewport: PixelRect {
                x: origin.0,
                y: origin.1,
                width: size.0,
                height: size.1,
            },
            ..Encoder::default()
        };
        let draws = self.encode(&mut enc, &scene.items, scene);
        enc.passes.push(Pass {
            target: Target::Main,
            origin,
            clear: scene.background.map_or(wgpu::Color::TRANSPARENT, |c| {
                let c = c.to_rgba8().map(|v| f64::from(v) / 255.0);
                let [r, g, b, a] = c;
                wgpu::Color {
                    r: r * a,
                    g: g * a,
                    b: b * a,
                    a,
                }
            }),
            draws,
        });
        let load_main = scene.background.is_none();
        let layers: Vec<Texture> = enc
            .layers
            .iter()
            .map(|r| self.take_layer((r.width, r.height)))
            .collect();
        self.layers_peak.0 = self.layers_peak.0.max(layers.len());
        self.upload(&enc, size, &layers);
        let globals = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gui globals"),
            layout: &self.globals_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.globals_buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(16),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.prim_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.stop_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.mono.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&self.color.view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut extras: HashMap<Extra, wgpu::BindGroup> = HashMap::new();
        for pass in &enc.passes {
            for d in &pass.draws {
                if d.extra == Extra::None || extras.contains_key(&d.extra) {
                    continue;
                }
                let view = match d.extra {
                    Extra::None => None,
                    Extra::Image(k) => self.images.peek(&k).and_then(Option::as_ref).map(|i| &i.texture.view),
                    Extra::Shapes(k) => self.shapes.peek(&k).and_then(Option::as_ref).map(|s| &s.texture.view),
                    Extra::Layer(i) => layers.get(i).map(|l| &l.view),
                };
                if let Some(view) = view {
                    extras.insert(
                        d.extra,
                        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("gui extra"),
                            layout: &self.extra_layout,
                            entries: &[wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(view),
                            }],
                        }),
                    );
                }
            }
        }
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gui frame"),
        });
        for (i, pass) in enc.passes.iter().enumerate() {
            let target = match pass.target {
                Target::Main => view,
                Target::Layer(l) => match layers.get(l) {
                    Some(t) => &t.view,
                    None => continue,
                },
            };
            let load = match pass.target {
                Target::Main if load_main => wgpu::LoadOp::Load,
                _ => wgpu::LoadOp::Clear(pass.clear),
            };
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
                ..Default::default()
            });
            rp.set_pipeline(&self.pipeline);
            let offset = u32::try_from(u64::try_from(i).unwrap_or(0).saturating_mul(UNIFORM_STRIDE)).unwrap_or(0);
            rp.set_bind_group(0, &globals, &[offset]);
            for d in &pass.draws {
                let extra = extras.get(&d.extra).unwrap_or(&self.blank);
                rp.set_bind_group(1, extra, &[]);
                rp.draw(0..4, d.instances.clone());
            }
        }
        self.queue.submit([encoder.finish()]);
        self.layer_pool.extend(layers);
    }

    /// Make sure every raster `items` draws is on the GPU; false when an
    /// atlas page had to grow, which empties it (prepare again).
    fn prepare(&mut self, items: &[Item], scene: &Scene, text: &mut TextSystem) -> bool {
        for item in items {
            let ok = match item {
                Item::Glyph { key, .. } => self.glyph(*key, text).is_ok(),
                Item::Icon {
                    icon,
                    size,
                    color,
                    stroke_width,
                    ..
                } => self.icon(*icon, *size, *color, *stroke_width).is_ok(),
                Item::Image { source, .. } => {
                    self.image(source);
                    true
                }
                Item::Shapes { origin, shapes, .. } => {
                    self.shapes(*origin, shapes, scene);
                    true
                }
                Item::TextThrough { mask, .. } => self.prepare(mask, scene, text),
                Item::Group(g) => self.prepare(&g.items, scene, text),
                _ => true,
            };
            if !ok {
                return false;
            }
        }
        true
    }

    /// Room in `page` for `w`×`h`: what the last frames did not use is let
    /// go first; `Err` when the page grew (and emptied).
    fn allocate(&mut self, page: Page, w: u32, h: u32) -> Result<Option<Slot>, ()> {
        let atlas = match page {
            Page::Mono => &mut self.mono,
            Page::Color => &mut self.color,
        };
        if let Some(slot) = atlas.allocate(w, h) {
            return Ok(Some(slot));
        }
        let frame = self.frame;
        let stale = |p: &Option<Placed>| p.is_some_and(|p| p.page == page && frame.saturating_sub(p.used) >= 2);
        let mut freed = Vec::new();
        self.glyphs.retain(|_, p| {
            if stale(p) {
                freed.extend(p.map(|p| p.slot));
                false
            } else {
                true
            }
        });
        self.icons.retain(|_, p| {
            if stale(p) {
                freed.extend(p.map(|p| p.slot));
                false
            } else {
                true
            }
        });
        let atlas = match page {
            Page::Mono => &mut self.mono,
            Page::Color => &mut self.color,
        };
        for s in freed {
            atlas.free(s);
        }
        if let Some(slot) = atlas.allocate(w, h) {
            return Ok(Some(slot));
        }
        if atlas.grow(&self.device) {
            self.glyphs.retain(|_, p| p.is_none_or(|p| p.page != page));
            self.icons.retain(|_, p| p.is_none_or(|p| p.page != page));
            return Err(());
        }
        Ok(None)
    }

    fn glyph(&mut self, key: CacheKey, text: &mut TextSystem) -> Result<Option<Placed>, ()> {
        let frame = self.frame;
        if let Some(p) = self.glyphs.get_mut(&key) {
            if let Some(p) = p {
                p.used = frame;
            }
            return Ok(*p);
        }
        let Some(image) = text.raster(key) else {
            self.glyphs.insert(key, None);
            return Ok(None);
        };
        let (w, h) = (image.placement.width, image.placement.height);
        if w == 0 || h == 0 {
            self.glyphs.insert(key, None);
            return Ok(None);
        }
        let (page, data) = match image.content {
            SwashContent::Mask => (Page::Mono, image.data),
            SwashContent::Color | SwashContent::SubpixelMask => {
                // Straight to premultiplied, as the colour page holds.
                let mut data = image.data;
                for [r, g, b, a] in data.as_chunks_mut::<4>().0 {
                    *r = crate::cpu::mul_u8(*r, *a);
                    *g = crate::cpu::mul_u8(*g, *a);
                    *b = crate::cpu::mul_u8(*b, *a);
                }
                (Page::Color, data)
            }
        };
        let Some(slot) = self.allocate(page, w, h)? else {
            self.glyphs.insert(key, None);
            return Ok(None);
        };
        match page {
            Page::Mono => self.mono.upload(&self.queue, slot, w, h, &data),
            Page::Color => self.color.upload(&self.queue, slot, w, h, &data),
        }
        let placed = Placed {
            page,
            slot,
            width: w,
            height: h,
            left: image.placement.left,
            top: image.placement.top,
            used: frame,
        };
        self.glyphs.insert(key, Some(placed));
        Ok(Some(placed))
    }

    fn icon(&mut self, icon: Icon, size: u32, color: Color, stroke_width: f32) -> Result<Option<Placed>, ()> {
        let key = IconKey {
            icon,
            size,
            rgba: color.to_rgba8(),
            stroke: stroke_width.to_bits(),
        };
        let frame = self.frame;
        if let Some(p) = self.icons.get_mut(&key) {
            if let Some(p) = p {
                p.used = frame;
            }
            return Ok(*p);
        }
        let Some(pixmap) = crate::icon::rasterise(icon, size, color, stroke_width) else {
            self.icons.insert(key, None);
            return Ok(None);
        };
        let (w, h) = (pixmap.width(), pixmap.height());
        let Some(slot) = self.allocate(Page::Color, w, h)? else {
            self.icons.insert(key, None);
            return Ok(None);
        };
        self.color.upload(&self.queue, slot, w, h, pixmap.data());
        let placed = Placed {
            page: Page::Color,
            slot,
            width: w,
            height: h,
            left: 0,
            top: 0,
            used: frame,
        };
        self.icons.insert(key, Some(placed));
        Ok(Some(placed))
    }

    fn image(&mut self, source: &ImageSource) {
        let key = std::rc::Rc::as_ptr(&source.0).addr();
        let (device, queue) = (&self.device, &self.queue);
        self.images.get_or_insert_with(key, || {
            let size = source.size();
            (size.0 > 0 && size.1 > 0).then(|| ImageTexture {
                _source: source.clone(),
                texture: texture(
                    device,
                    queue,
                    size,
                    wgpu::TextureFormat::Rgba8Unorm,
                    Some(source.0.data()),
                ),
            })
        });
    }

    fn shapes(&mut self, origin: (f32, f32), shapes: &[Shape], scene: &Scene) {
        let Some((key, _, size, transform)) = shape_raster_key(origin, shapes, scene) else {
            return;
        };
        let (device, queue) = (&self.device, &self.queue);
        self.shapes.get_or_insert_with(key, || {
            let mut pixmap = Pixmap::new(size.0, size.1)?;
            crate::cpu::paint_shapes(&mut pixmap, transform, shapes, None);
            Some(ShapeRaster {
                texture: texture(
                    device,
                    queue,
                    size,
                    wgpu::TextureFormat::Rgba8Unorm,
                    Some(pixmap.data()),
                ),
            })
        });
    }

    /// A layer texture at least `size`, from the pool when one fits.
    fn take_layer(&mut self, size: (u32, u32)) -> Texture {
        // Sizes in steps of 128 pixels, so a group that grows reuses its layer.
        let round = |v: u32| v.div_ceil(128).saturating_mul(128).max(128);
        let want = (round(size.0), round(size.1));
        if let Some(i) = self.layer_pool.iter().position(|t| t.size == want) {
            return self.layer_pool.swap_remove(i);
        }
        texture(&self.device, &self.queue, want, self.format, None)
    }

    fn upload(&mut self, enc: &Encoder, main: (u32, u32), layers: &[Texture]) {
        let prim_bytes = floats_to_bytes(&enc.prims);
        let stop_bytes = floats_to_bytes(if enc.stops.is_empty() { &[0.0; 8] } else { &enc.stops });
        grow_buffer(
            &self.device,
            &mut self.prim_buffer,
            prim_bytes.len().max(16),
            wgpu::BufferUsages::STORAGE,
        );
        grow_buffer(
            &self.device,
            &mut self.stop_buffer,
            stop_bytes.len(),
            wgpu::BufferUsages::STORAGE,
        );
        if !prim_bytes.is_empty() {
            self.queue.write_buffer(&self.prim_buffer, 0, &prim_bytes);
        }
        self.queue.write_buffer(&self.stop_buffer, 0, &stop_bytes);
        let mut globals = Vec::new();
        for pass in &enc.passes {
            let size = match pass.target {
                Target::Main => main,
                Target::Layer(i) => layers.get(i).map_or((1, 1), |t| t.size),
            };
            let mut block = [0.0_f32; 64];
            block[0] = u32_to_f32(size.0);
            block[1] = u32_to_f32(size.1);
            block[2] = i32_to_f32(pass.origin.0);
            block[3] = i32_to_f32(pass.origin.1);
            globals.extend(floats_to_bytes(&block));
        }
        grow_buffer(
            &self.device,
            &mut self.globals_buffer,
            globals.len(),
            wgpu::BufferUsages::UNIFORM,
        );
        self.queue.write_buffer(&self.globals_buffer, 0, &globals);
    }

    /// The instances `items` make, grouped into draws; groups add their passes to `enc`.
    #[expect(clippy::too_many_lines, reason = "one arm per primitive")]
    fn encode(&self, enc: &mut Encoder, items: &[Item], scene: &Scene) -> Vec<Draw> {
        let s = scene.scale;
        let mut draws: Vec<Draw> = Vec::new();
        let mut push = |enc: &mut Encoder, prim: Prim, extra: Extra, clip: Option<Clip>| {
            let mut prim = prim;
            if let Some(c) = clip {
                let b = c.bounds.snapped(s);
                let rect = phys(b, s);
                prim.0[7] = rect;
                prim.0[8] = radii(c.radius, s);
                // Nothing outside the clip is drawn: neither is it shaded.
                let quad = intersect(prim.0[0], rect);
                if quad[2] <= 0.0 || quad[3] <= 0.0 {
                    return;
                }
                prim.0[0] = quad;
            }
            let index = u32::try_from(enc.prims.len() / (PRIM * 4)).unwrap_or(u32::MAX);
            for f in prim.0 {
                enc.prims.extend_from_slice(&f);
            }
            match draws.last_mut() {
                Some(d) if d.extra == extra && d.instances.end == index => d.instances.end = index.saturating_add(1),
                _ => draws.push(Draw {
                    extra,
                    instances: index..index.saturating_add(1),
                }),
            }
        };
        for item in items {
            match item {
                Item::Rect { bounds, color, clip } => {
                    let rect = phys(*bounds, s);
                    let mut p = Prim::new(KIND_SOLID, outset(rect, 1.0));
                    p.0[2] = rgba(*color);
                    p.0[3] = rect;
                    push(enc, p, Extra::None, *clip);
                }
                Item::RoundedRect {
                    bounds,
                    radius,
                    color,
                    clip,
                } => {
                    let rect = phys(*bounds, s);
                    let mut p = Prim::new(KIND_SOLID, outset(rect, 1.0));
                    p.0[2] = rgba(*color);
                    p.0[3] = rect;
                    p.0[4] = radii(*radius, s);
                    push(enc, p, Extra::None, *clip);
                }
                Item::Band {
                    outer,
                    outer_radius,
                    inner,
                    inner_radius,
                    color,
                    clip,
                } => {
                    let rect = phys(*outer, s);
                    let mut p = Prim::new(KIND_SOLID, outset(rect, 1.0));
                    p.0[2] = rgba(*color);
                    p.0[3] = rect;
                    p.0[4] = radii(*outer_radius, s);
                    if !inner.is_empty() {
                        p.0[5] = phys(*inner, s);
                        p.0[6] = radii(*inner_radius, s);
                    }
                    push(enc, p, Extra::None, *clip);
                }
                Item::Shadow {
                    bounds,
                    radius,
                    shadow,
                    clip,
                } => {
                    let shape = Bounds {
                        x: bounds.x + shadow.x,
                        y: bounds.y + shadow.y,
                        ..*bounds
                    }
                    .outset(shadow.spread);
                    if shape.is_empty() {
                        continue;
                    }
                    let sigma = shadow.blur * s / 2.0;
                    let rect = phys(shape, s);
                    let mut p = Prim::new(KIND_SHADOW, outset(rect, sigma.mul_add(3.0, 1.0)));
                    p.0[2] = rgba(shadow.color);
                    p.0[3] = rect;
                    p.0[4] = radii(grow(*radius, shadow.spread), s);
                    p.0[5] = phys(*bounds, s);
                    p.0[6] = radii(*radius, s);
                    p.0[9] = [sigma, 0.0, 0.0, 0.0];
                    push(enc, p, Extra::None, *clip);
                }
                Item::Gradient {
                    bounds,
                    radius,
                    gradient,
                    clip,
                } => {
                    let rect = phys(*bounds, s);
                    let mut p = Prim::new(KIND_GRADIENT, outset(rect, 1.0));
                    p.0[3] = rect;
                    p.0[4] = radii(*radius, s);
                    set_gradient(enc, &mut p, gradient, s, 10);
                    push(enc, p, Extra::None, *clip);
                }
                Item::Glyph { key, x, y, color, clip } => {
                    let Some(Some(g)) = self.glyphs.get(key) else { continue };
                    let rect = [
                        i32_to_f32(x.saturating_add(g.left)),
                        i32_to_f32(y.saturating_sub(g.top)),
                        u32_to_f32(g.width),
                        u32_to_f32(g.height),
                    ];
                    let kind = if g.page == Page::Mono { KIND_MONO } else { KIND_COLOR };
                    let mut p = Prim::new(kind, rect);
                    p.0[2] = rgba(*color);
                    p.0[3] = rect;
                    p.0[9] = [u32_to_f32(g.slot.x), u32_to_f32(g.slot.y), 0.0, 0.0];
                    push(enc, p, Extra::None, *clip);
                }
                Item::Icon {
                    icon,
                    size,
                    color,
                    stroke_width,
                    x,
                    y,
                    clip,
                } => {
                    let key = IconKey {
                        icon: *icon,
                        size: *size,
                        rgba: color.to_rgba8(),
                        stroke: stroke_width.to_bits(),
                    };
                    let Some(Some(g)) = self.icons.get(&key) else { continue };
                    let rect = [
                        i32_to_f32(*x),
                        i32_to_f32(*y),
                        u32_to_f32(g.width),
                        u32_to_f32(g.height),
                    ];
                    let mut p = Prim::new(KIND_COLOR, rect);
                    p.0[2] = [1.0; 4];
                    p.0[3] = rect;
                    p.0[9] = [u32_to_f32(g.slot.x), u32_to_f32(g.slot.y), 0.0, 0.0];
                    push(enc, p, Extra::None, *clip);
                }
                Item::Image {
                    source,
                    bounds,
                    radius,
                    clip,
                } => {
                    let key = std::rc::Rc::as_ptr(&source.0).addr();
                    if !self.images.peek(&key).is_some_and(Option::is_some) {
                        continue;
                    }
                    let rect = phys(*bounds, s);
                    let (iw, ih) = source.size();
                    let (iw, ih) = (u32_to_f32(iw).max(1.0), u32_to_f32(ih).max(1.0));
                    let fit = (rect[2] / iw).max(rect[3] / ih);
                    let placed = [
                        rect[0] + (rect[2] - iw * fit) / 2.0,
                        rect[1] + (rect[3] - ih * fit) / 2.0,
                        iw * fit,
                        ih * fit,
                    ];
                    let mut p = Prim::new(KIND_IMAGE, outset(rect, 1.0));
                    p.0[3] = rect;
                    p.0[4] = radii(*radius, s);
                    p.0[9] = placed;
                    push(enc, p, Extra::Image(key), *clip);
                }
                Item::Shapes { origin, shapes, clip } => {
                    // The key holds the shapes and where they fall within a
                    // pixel, not where: the same raster serves them anywhere.
                    let Some((key, at, ..)) = shape_raster_key(*origin, shapes, scene) else {
                        continue;
                    };
                    let Some(Some(r)) = self.shapes.peek(&key) else {
                        continue;
                    };
                    let rect = [
                        i32_to_f32(at.0),
                        i32_to_f32(at.1),
                        u32_to_f32(r.texture.size.0),
                        u32_to_f32(r.texture.size.1),
                    ];
                    let mut p = Prim::new(KIND_RASTER, rect);
                    p.0[3] = rect;
                    push(enc, p, Extra::Shapes(key), *clip);
                }
                Item::TextThrough {
                    region,
                    mask,
                    background,
                    fill,
                    clip,
                } => {
                    let layer = self.layer(enc, mask, *region, scene);
                    let rect = pixel_rect(*region);
                    let mut p = Prim::new(KIND_TEXT_THROUGH, rect);
                    p.0[3] = rect;
                    let mut flags = 0.0;
                    if let Some((b, c)) = background {
                        flags += 1.0;
                        p.0[2] = rgba(*c);
                        p.0[5] = phys(*b, s);
                    }
                    if let Some((g, shown)) = fill {
                        flags += 2.0;
                        p.0[9] = phys(*shown, s);
                        set_gradient(enc, &mut p, g, s, 10);
                    }
                    p.0[1][1] = flags;
                    push(enc, p, Extra::Layer(layer), *clip);
                }
                Item::Group(group) => {
                    let layer = self.layer(enc, &group.items, group.region, scene);
                    let p = composite(enc, group, s);
                    push(enc, p, Extra::Layer(layer), group.clip);
                }
            }
        }
        draws
    }

    /// Encode `items` into a new layer covering `region`; its index.
    fn layer(&self, enc: &mut Encoder, items: &[Item], region: PixelRect, scene: &Scene) -> usize {
        // A layer larger than a texture may be keeps to the part drawn.
        let max = self.device.limits().max_texture_dimension_2d;
        let region = if region.width > max || region.height > max {
            clamp_rect(region, enc.viewport)
        } else {
            region
        };
        let index = enc.layers.len();
        enc.layers.push(region);
        let draws = self.encode(enc, items, scene);
        enc.passes.push(Pass {
            target: Target::Layer(index),
            origin: (region.x, region.y),
            clear: wgpu::Color::TRANSPARENT,
            draws,
        });
        index
    }

    /// Read `view` (`size`, this renderer's format) back as a pixmap.
    fn read_back(&self, texture: &wgpu::Texture, size: (u32, u32)) -> Option<Pixmap> {
        let row = size.0.saturating_mul(4);
        let padded = row.div_ceil(256).saturating_mul(256);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gui read back"),
            size: u64::from(padded).saturating_mul(u64::from(size.1)),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gui read back"),
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(size.1),
                },
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| drop(tx.send(r)));
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let mapped = slice.get_mapped_range().ok()?;
        let bgra = self.format == wgpu::TextureFormat::Bgra8Unorm;
        let mut data = Vec::with_capacity(usize::try_from(row.saturating_mul(size.1)).ok()?);
        for line in mapped.chunks(usize::try_from(padded).ok()?) {
            let line = line.get(..usize::try_from(row).ok()?)?;
            if bgra {
                for [b, g, r, a] in line.as_chunks::<4>().0 {
                    data.extend_from_slice(&[*r, *g, *b, *a]);
                }
            } else {
                data.extend_from_slice(line);
            }
        }
        drop(mapped);
        buffer.unmap();
        Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(size.0, size.1)?)
    }
}

impl Renderer for GpuRenderer {
    fn render(&mut self, scene: &Scene, text: &mut TextSystem) {
        let Some(view) = self.window_view() else { return };
        self.draw(scene, text, &view);
    }

    fn end_frame(&mut self) {
        if let Some(frame) = self.window.as_mut().and_then(|w| w.frame.take()) {
            self.queue.present(frame);
        }
        self.images.end_frame();
        self.shapes.end_frame();
        self.frame = self.frame.saturating_add(1);
        // Layers the last two frames did not need go back to the driver, as
        // the CPU renderer lets its layers go.
        let (now, before) = self.layers_peak;
        self.layer_pool.truncate(now.max(before));
        self.layers_peak = (0, now);
    }

    fn waited(&mut self) -> std::time::Duration {
        std::mem::take(&mut self.waited)
    }

    fn held(&self) -> Held {
        let count = |n: usize, bytes: usize| Raster { count: n, bytes };
        let glyphs = self.glyphs.values().flatten().count();
        let icons = self.icons.values().flatten().count();
        let tex_bytes = |t: &Texture| {
            usize::try_from(
                u64::from(t.size.0)
                    .saturating_mul(u64::from(t.size.1))
                    .saturating_mul(4),
            )
            .unwrap_or(0)
        };
        Held {
            glyphs: count(glyphs, self.mono.bytes()),
            icons: count(icons, self.color.bytes()),
            shadows: Raster::default(),
            images: count(
                self.images.len(),
                self.images
                    .values()
                    .flatten()
                    .map(|i| tex_bytes(&i.texture))
                    .chain(self.shapes.values().flatten().map(|s| tex_bytes(&s.texture)))
                    .sum(),
            ),
            layers: count(self.layer_pool.len(), self.layer_pool.iter().map(tex_bytes).sum()),
        }
    }

    fn capture(&mut self, scene: &Scene, text: &mut TextSystem) -> Option<Pixmap> {
        // A scene taller than a texture may be (a whole page, for the
        // verdict) is drawn in bands, each read back into its rows.
        let (w, h) = (scene.size.0.max(1), scene.size.1.max(1));
        let max = self.device.limits().max_texture_dimension_2d;
        if w > max {
            return None;
        }
        let band = h.min(max);
        let mut out = Pixmap::new(w, h)?;
        let target = self.offscreen((w, band));
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut top = 0;
        while top < h {
            let rows = band.min(h.saturating_sub(top));
            let origin = (0, i32::try_from(top).ok()?);
            self.draw_part(scene, text, &view, origin, (w, band));
            let part = self.read_back(&target, (w, band))?;
            let row = usize::try_from(w.saturating_mul(4)).ok()?;
            let start = usize::try_from(top).ok()?.saturating_mul(row);
            let len = usize::try_from(rows).ok()?.saturating_mul(row);
            out.data_mut()
                .get_mut(start..start.saturating_add(len))?
                .copy_from_slice(part.data().get(..len)?);
            top = top.saturating_add(band);
        }
        Some(out)
    }
}

impl GpuRenderer {
    /// Draw `scene` aside and wait for the GPU to finish it, reading
    /// nothing back: what a window's frame costs, measured headless.
    pub fn draw_and_wait(&mut self, scene: &Scene, text: &mut TextSystem) {
        let target = self.offscreen(scene.size);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        self.draw(scene, text, &view);
        drop(self.device.poll(wgpu::PollType::wait_indefinitely()));
    }

    /// A texture to draw a `size` frame into aside.
    fn offscreen(&self, size: (u32, u32)) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui capture"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }
}

fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

/// A texture `size`, filled with `data` (rows of `size.0` RGBA pixels) when given.
fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    data: Option<&[u8]>,
) -> Texture {
    let extent = wgpu::Extent3d {
        width: size.0.max(1),
        height: size.1.max(1),
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gui texture"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    if let Some(data) = data {
        queue.write_texture(
            texture.as_image_copy(),
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(extent.width.saturating_mul(4)),
                rows_per_image: Some(extent.height),
            },
            extent,
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Texture {
        _texture: texture,
        view,
        size: (extent.width, extent.height),
    }
}

fn grow_buffer(device: &wgpu::Device, buffer: &mut wgpu::Buffer, needed: usize, usage: wgpu::BufferUsages) {
    let needed = u64::try_from(needed).unwrap_or(u64::MAX);
    if buffer.size() >= needed {
        return;
    }
    *buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gui buffer"),
        size: needed.next_power_of_two(),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
}

fn floats_to_bytes(floats: &[f32]) -> Vec<u8> {
    floats.iter().flat_map(|f| f.to_ne_bytes()).collect()
}

/// `b` (logical) in physical pixels.
fn phys(b: Bounds, s: f32) -> [f32; 4] {
    [b.x * s, b.y * s, b.width * s, b.height * s]
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

fn pixel_rect(r: PixelRect) -> [f32; 4] {
    [
        i32_to_f32(r.x),
        i32_to_f32(r.y),
        u32_to_f32(r.width),
        u32_to_f32(r.height),
    ]
}

fn radii(r: Corners, s: f32) -> [f32; 4] {
    [
        r.top_left.max(0.0) * s,
        r.top_right.max(0.0) * s,
        r.bottom_right.max(0.0) * s,
        r.bottom_left.max(0.0) * s,
    ]
}

fn outset(r: [f32; 4], d: f32) -> [f32; 4] {
    [r[0] - d, r[1] - d, d.mul_add(2.0, r[2]), d.mul_add(2.0, r[3])]
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

fn rgba(c: Color) -> [f32; 4] {
    [
        c.r.clamp(0.0, 1.0),
        c.g.clamp(0.0, 1.0),
        c.b.clamp(0.0, 1.0),
        c.a.clamp(0.0, 1.0),
    ]
}

/// Put `g`'s stops in `enc` and its line (physical) in field `line` of `p`.
fn set_gradient(enc: &mut Encoder, p: &mut Prim, g: &Gradient, s: f32, line: usize) {
    let first = u32_to_f32(u32::try_from(enc.stops.len() / 8).unwrap_or(0));
    let stops: Vec<(f32, Color)> = match g {
        Gradient::Solid(c) => {
            if let Some(f) = p.0.get_mut(line) {
                *f = [0.0; 4];
            }
            vec![(0.0, *c)]
        }
        Gradient::Linear { start, end, stops } => {
            if let Some(f) = p.0.get_mut(line) {
                *f = [start.0 * s, start.1 * s, end.0 * s, end.1 * s];
            }
            stops.clone()
        }
    };
    for (offset, c) in &stops {
        enc.stops.extend_from_slice(&[*offset, 0.0, 0.0, 0.0]);
        enc.stops.extend_from_slice(&rgba(*c));
    }
    p.0[1][2] = first;
    p.0[1][3] = u32_to_f32(u32::try_from(stops.len()).unwrap_or(0));
}

/// The instance compositing `group`'s layer.
fn composite(enc: &mut Encoder, group: &Group, s: f32) -> Prim {
    let region = pixel_rect(group.region);
    // Forward: q ↦ o + R·S·(q − o); the shader maps back from the window.
    let (fwd, inv) = match group.transform {
        None => ([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        Some(t) => {
            let (sin, cos) = t.rotate.to_radians().sin_cos();
            let k = t.scale;
            // Columns: x' = a·x + c·y + e, y' = b·x + d·y + f.
            let (a, b, c, d) = (cos * k, sin * k, -sin * k, cos * k);
            let (ox, oy) = t.origin;
            let e = ox - (a * ox + c * oy);
            let f = oy - (b * ox + d * oy);
            let det = a * d - b * c;
            if det.abs() < 1e-6 {
                ([0.0; 6], [0.0; 6])
            } else {
                let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
                let ie = -(ia * e + ic * f);
                let if_ = -(ib * e + id * f);
                ([a, b, c, d, e, f], [ia, ib, ic, id, ie, if_])
            }
        }
    };
    let corners = [
        (region[0], region[1]),
        (region[0] + region[2], region[1]),
        (region[0], region[1] + region[3]),
        (region[0] + region[2], region[1] + region[3]),
    ]
    .map(|(x, y)| (fwd[0] * x + fwd[2] * y + fwd[4], fwd[1] * x + fwd[3] * y + fwd[5]));
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in corners {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let mut p = Prim::new(KIND_LAYER, outset([x0, y0, x1 - x0, y1 - y0], 1.0));
    p.0[2] = [0.0, 0.0, 0.0, group.opacity.clamp(0.0, 1.0)];
    p.0[3] = region;
    p.0[9] = [inv[0], inv[1], inv[2], inv[3]];
    p.0[10] = [inv[4], inv[5], 0.0, 0.0];
    let mut filters = [0.0; 4];
    for (slot, f) in filters.as_chunks_mut::<2>().0.iter_mut().zip(&group.filters) {
        *slot = match f {
            Filter::Brightness(k) => [1.0, *k],
            Filter::Grayscale(a) => [2.0, *a],
        };
    }
    p.0[11] = filters;
    if let Some(mask) = &group.mask {
        set_gradient(enc, &mut p, mask, s, 12);
    }
    p
}

/// Shapes' raster: its cache key, top left and size (physical), and the
/// transform that draws them into it.
type ShapePlacement = (u64, (i32, i32), (u32, u32), tiny_skia::Transform);

/// Where shapes drawn from `origin` land, and how they are rasterised:
/// their cache key, top left and size (physical), and the transform that
/// draws them into their raster.
fn shape_raster_key(origin: (f32, f32), shapes: &[Shape], scene: &Scene) -> Option<ShapePlacement> {
    let s = scene.scale;
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut reach = 1.0_f32;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for shape in shapes {
        if let Some((c, w)) = shape.stroke {
            reach = reach.max(w / 2.0 + 1.0);
            c.to_rgba8().hash(&mut hasher);
            w.to_bits().hash(&mut hasher);
        }
        if let Some(c) = shape.fill {
            c.to_rgba8().hash(&mut hasher);
        }
        for c in &shape.commands {
            let points: &[f32] = match c {
                PathCommand::MoveTo(x, y) | PathCommand::LineTo(x, y) => &[*x, *y],
                PathCommand::QuadTo(a, b, x, y) => &[*a, *b, *x, *y],
                PathCommand::CubicTo(a, b, c2, d, x, y) => &[*a, *b, *c2, *d, *x, *y],
                PathCommand::Close => &[],
            };
            points.len().hash(&mut hasher);
            for [x, y] in points.as_chunks::<2>().0 {
                x0 = x0.min(*x);
                y0 = y0.min(*y);
                x1 = x1.max(*x);
                y1 = y1.max(*y);
                x.to_bits().hash(&mut hasher);
                y.to_bits().hash(&mut hasher);
            }
        }
    }
    if x0 > x1 || y0 > y1 {
        return None;
    }
    let (w, h) = (u32_to_f32(scene.size.0), u32_to_f32(scene.size.1));
    let left = ((origin.0 + x0 - reach) * s).floor().clamp(0.0, w);
    let top = ((origin.1 + y0 - reach) * s).floor().clamp(0.0, h);
    let right = ((origin.0 + x1 + reach) * s).ceil().clamp(0.0, w);
    let bottom = ((origin.1 + y1 + reach) * s).ceil().clamp(0.0, h);
    if right <= left || bottom <= top {
        return None;
    }
    let size = (
        crate::convert::f32_to_u32(right - left),
        crate::convert::f32_to_u32(bottom - top),
    );
    let (tx, ty) = (origin.0.mul_add(s, -left), origin.1.mul_add(s, -top));
    tx.to_bits().hash(&mut hasher);
    ty.to_bits().hash(&mut hasher);
    s.to_bits().hash(&mut hasher);
    size.hash(&mut hasher);
    let at = (crate::convert::f32_to_i32(left), crate::convert::f32_to_i32(top));
    Some((
        hasher.finish(),
        at,
        size,
        tiny_skia::Transform::from_row(s, 0.0, 0.0, s, tx, ty),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::CpuRenderer;
    use crate::scene::GroupTransform;

    /// A scene painted by both renderers; `None` without a GPU.
    fn both(scene: &Scene) -> Option<(Pixmap, Pixmap)> {
        let mut gpu = GpuRenderer::headless().ok()?;
        let mut text = TextSystem::new();
        let g = gpu.capture(scene, &mut text)?;
        let c = CpuRenderer::default().capture(scene, &mut text)?;
        Some((c, g))
    }

    fn at(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
        let c = p.pixel(x, y).unwrap_or_else(|| panic!("({x}, {y}) is outside"));
        [c.red(), c.green(), c.blue(), c.alpha()]
    }

    #[test]
    fn a_turned_group_lands_where_the_cpu_puts_it() {
        let mut scene = Scene::new((100, 100), 1.0, Some(Color::WHITE));
        scene.items.push(Item::Group(Box::new(Group {
            region: PixelRect {
                x: 40,
                y: 40,
                width: 20,
                height: 20,
            },
            items: vec![Item::Rect {
                bounds: Bounds {
                    x: 40.0,
                    y: 40.0,
                    width: 20.0,
                    height: 20.0,
                },
                color: Color::BLACK,
                clip: None,
            }],
            mask: None,
            filters: vec![],
            opacity: 1.0,
            transform: Some(GroupTransform {
                origin: (50.0, 50.0),
                scale: 1.0,
                rotate: 45.0,
            }),
            clip: None,
        })));
        let Some((cpu, gpu)) = both(&scene) else { return };
        for (x, y) in [(50, 50), (50, 38), (38, 50), (41, 41), (59, 59), (50, 62)] {
            let (c, g) = (at(&cpu, x, y), at(&gpu, x, y));
            assert!(
                c.iter().zip(g).all(|(a, b)| a.abs_diff(b) <= 40),
                "({x}, {y}): cpu {c:?}, gpu {g:?}"
            );
        }
    }
}
