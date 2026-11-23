//! The GPU renderer: a [`Scene`] drawn with wgpu on Metal, Vulkan or DX12
//! (no OpenGL: a machine with none of them uses lavapipe), learned from
//! GPUI's renderers and from Chromium's compositor (ADR-0016). Three kinds
//! of primitive share each pass, in the scene's order:
//!
//! - quads (`shaders.wgsl`): rounded boxes, borders, rings, shadows,
//!   gradients and images, whose coverage comes from signed distances in
//!   closed form, and glyphs and icons read from multi-page atlases;
//! - paths (`path_render.wgsl`): vector paths flattened, stroked and binned
//!   into tiles by compute passes (`path_compute.wgsl`, [`path`]) and
//!   covered with exact area (ADR-0017); icons are rasterized by the same
//!   passes once into the coverage atlas;
//! - curve text (`slug.wgsl`, [`slug`]): glyphs above 128 device ppem or
//!   under a transform that is not a translation, drawn from their curves.
//!
//! What is kept across frames:
//!
//! - each paint chunk's quad records ([`record`]), written once into a
//!   buffer kept on the GPU ([`words`]) and replayed while the chunk is the
//!   same; a scroll, a moving translation or a fade writes an entry of the
//!   property tables (offsets, clips, folded opacity) and no record;
//! - the frame, in a canvas texture: a frame redraws only what changed
//!   ([`damage`]), scissored, over what the canvas holds, and is copied to
//!   the window's surface by one triangle. A frame with nothing changed
//!   submits nothing;
//! - effect layers: an effect gets a layer of its own only when it needs one
//!   (a filter, a mask, a fade over primitives that overlap, a transform of
//!   what cannot be drawn transformed); else its opacity is multiplied into
//!   its primitives and they draw straight. A layer whose content did not
//!   change is composited as it was kept.
//!
//! Every pipeline is built when the renderer is made, bind groups are made
//! again only when a buffer or texture they hold is replaced, and no pixel
//! is touched by the CPU.

mod arena;
mod atlas;
mod chunks;
mod clips;
mod cold;
mod damage;
mod frame;
pub(crate) mod path;
mod read;
mod record;
mod slug;
#[cfg(test)]
mod tests;
mod words;
mod world;

pub use self::world::RASTER_BUDGET;

use std::collections::HashMap;
use std::sync::Arc;

use self::arena::Scratch;
use self::atlas::{Atlas, Made, Slot};
use self::chunks::Kept;
use self::damage::Rect;
use self::frame::{CachedLayer, FrameData, MaskJob};
use self::path::{ComputePipelines, Paint, Paths};
use self::slug::{SLUG, SlugKey, Slugs};
use self::words::{Table, Words};
use crate::cache::FrameCache;
use crate::color::Color;
use crate::convert::u32_to_f32;
use crate::figure::{Draw as PathDraw, Path, PathStyle, Stroke};
use crate::geometry::Bounds;
use crate::icon::Icon;
use crate::image::{ImageSource, Pixels};
use crate::inspect::Raster;
use crate::scene::{Changed, ChunkId, Drawn, Held, Renderer, Scene};
use crate::style::Corners;
use crate::text::gamma::{GammaParams, MaskGamma};
use crate::text::settings::Subpixel;
use crate::text::{Coverage, GlyphContent, GlyphKey, Glyphs};

/// The largest text, in device pixels per em, drawn in subpixel (LCD)
/// coverage: Skia's `SK_MAX_SIZE_FOR_LCDTEXT`.
const MAX_LCD_PPEM: f32 = 48.0;
/// The largest text kept as bitmaps in the atlas; larger is drawn as
/// curves (glifo's cap, ADR-0016).
const MAX_ATLAS_PPEM: f32 = 128.0;
/// Bytes of globals a pass reads (`Globals` in `shaders.wgsl`).
const GLOBALS: u64 = 64;

/// Why the GPU renderer could not start.
#[derive(Debug)]
pub struct GpuError(String);

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GpuError {}

/// How a renderer is made: what tests vary.
#[derive(Clone, Copy, Debug)]
pub struct GpuOptions {
    /// Use dual-source blending where the adapter has it (subpixel text);
    /// off, every primitive blends by its alpha and text is grey.
    pub dual_source: bool,
    /// An atlas page's side, in pixels (at most the adapter's texture size).
    pub atlas_side: u32,
    /// The most pages an atlas holds before it evicts one.
    pub max_pages: u32,
}

impl Default for GpuOptions {
    fn default() -> Self {
        Self {
            dual_source: true,
            atlas_side: 1024,
            max_pages: 8,
        }
    }
}

/// A window's surface and the frame being drawn into it.
struct Window {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    frame: Option<wgpu::SurfaceTexture>,
    /// The canvas was copied onto this frame's texture.
    blitted: bool,
}

/// The frame as drawn, kept across frames: only damage is drawn into it.
struct Canvas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// What reads it onto the surface.
    blit: wgpu::BindGroup,
    size: (u32, u32),
    /// The background it was cleared to; another redraws it whole.
    background: Option<[u8; 4]>,
}

/// Which atlas a raster went to.
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
}

/// A glyph as the atlas holds it: its key, the coverage it was drawn in,
/// and, for a colour glyph that paints with the text's colour, that colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GlyphSlot {
    key: GlyphKey,
    coverage: Coverage,
    foreground: Option<[u8; 4]>,
}

/// An icon as the coverage atlas holds it: tinted when drawn, so its colour
/// is not part of it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct IconKey {
    icon: Icon,
    size: u32,
    stroke: u32,
}

/// A texture an instance reads beside the atlases, with the bind group
/// that holds it, made once with it.
struct Texture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
    size: (u32, u32),
}

impl Texture {
    fn bytes(&self) -> usize {
        usize::try_from(
            u64::from(self.size.0)
                .saturating_mul(u64::from(self.size.1))
                .saturating_mul(4),
        )
        .unwrap_or(0)
    }
}

/// An image on the GPU; holds the image so its address names no other.
struct ImageTexture {
    _source: ImageSource,
    texture: Texture,
}

/// The texture a quad reads beside the atlases.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Extra {
    None,
    Image(usize),
    /// An effect's layer.
    Layer(u32),
    /// A chunk's text-through mask.
    Mask(ChunkId, u32),
    /// A scratch texture what lies below was read into (`read.rs`).
    Read(u8),
}

/// A run of instances of one pipeline.
#[derive(Clone, Debug, PartialEq)]
enum Op {
    /// Quads: instances are positions in the index buffer.
    Quads {
        extra: Extra,
        instances: std::ops::Range<u32>,
    },
    Path(std::ops::Range<u32>),
    Slug(std::ops::Range<u32>),
    /// An opaque rectangle covering the part it is drawn in, written
    /// without blending: the quad instance.
    Fill(u32),
    /// A data chunk's draws (ADR-0018), by their place in this frame's.
    Data(u32),
    /// What lies below read back, by its place in this frame's reads
    /// (`read.rs`): the pass ends and begins again around it.
    Read(u32),
}

/// An icon rasterized into a coverage page: the page and the path
/// instances that draw it.
struct MaskDraw {
    page: u32,
    instances: std::ops::Range<u32>,
}

/// Every pipeline, built when the renderer is made (ADR-0016).
struct Pipelines {
    quad: wgpu::RenderPipeline,
    /// Opaque rectangles over a whole part, not blended ([`Op::Fill`]).
    fill: wgpu::RenderPipeline,
    path: wgpu::RenderPipeline,
    /// Paths into an R8 coverage page (icons).
    mask: wgpu::RenderPipeline,
    slug_banded: wgpu::RenderPipeline,
    slug_full: wgpu::RenderPipeline,
    /// The canvas onto the surface.
    blit: wgpu::RenderPipeline,
    /// What lies below, scaled down and blurred (`read.rs`).
    read: wgpu::RenderPipeline,
    compute: ComputePipelines,
}

struct Layouts {
    quad: wgpu::BindGroupLayout,
    extra: wgpu::BindGroupLayout,
    path: wgpu::BindGroupLayout,
    compute: wgpu::BindGroupLayout,
    slug: wgpu::BindGroupLayout,
    blit: wgpu::BindGroupLayout,
    read: wgpu::BindGroupLayout,
}

/// A bind group and the generations of what it holds.
struct Cached {
    key: Vec<u64>,
    group: wgpu::BindGroup,
}

#[derive(Default)]
struct BindGroups {
    quad: Option<Cached>,
    path: Option<Cached>,
    compute: Option<Cached>,
    slug: Option<Cached>,
}

/// Ids handed out and given back (a chunk's state slot).
#[derive(Default)]
struct Ids {
    free: Vec<u32>,
    /// One past the highest handed out; 0 is the root's.
    next: u32,
}

impl Ids {
    fn take(&mut self) -> u32 {
        self.free.pop().unwrap_or_else(|| {
            self.next = self.next.max(1).saturating_add(1);
            self.next.saturating_sub(1)
        })
    }

    fn give(&mut self, id: u32) {
        if id != 0 {
            self.free.push(id);
        }
    }

    fn len(&self) -> u32 {
        self.next.max(1)
    }
}

/// What a device's lost callback runs: whoever must hear of the loss
/// at once, as it comes (on a thread of the driver's).
type LostWaker = Arc<std::sync::Mutex<Option<Box<dyn Fn() + Send>>>>;

/// The GPU renderer, for a window or headless.
pub struct GpuRenderer {
    /// What it was made from, to make it again when its device is lost
    /// (ADR-0015's invariant 7).
    instance: wgpu::Instance,
    options: GpuOptions,
    /// Set by the device's lost callback (a driver reset, a GPU removed).
    lost: Arc<std::sync::atomic::AtomicBool>,
    /// What the lost callback runs after setting `lost` ([`Self::wake_on_lost`]),
    /// kept across [`Self::rebuild`].
    wake: LostWaker,
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    /// Dual-source blending is on: subpixel text can be drawn.
    dual_source: bool,
    pipelines: Pipelines,
    layouts: Layouts,
    sampler: wgpu::Sampler,
    /// A transparent pixel, for draws that read no extra texture.
    blank: Texture,
    window: Option<Window>,
    canvas: Option<Canvas>,
    mono: Atlas,
    color: Atlas,
    glyphs: foldhash::HashMap<GlyphSlot, Option<Placed>>,
    /// Which colour glyphs take the text's colour (keyed by it).
    foreground: foldhash::HashMap<GlyphKey, bool>,
    /// The gamma table glyph coverage is corrected by, and what it was made from.
    gamma_texture: wgpu::Texture,
    gamma_view: wgpu::TextureView,
    gamma: GammaParams,
    /// Text is thickened as macOS thickens it (curve text keys on size then).
    darkened: bool,
    /// The stripe order subpixel text is drawn in, when it is.
    lcd: Option<Subpixel>,
    /// The text settings' generation the glyphs and table were made at.
    text_generation: Option<u64>,
    icons: HashMap<IconKey, Option<Placed>>,
    /// Icons rasterized and not yet drawn into their page.
    masks: Vec<MaskDraw>,
    images: FrameCache<usize, Option<ImageTexture>>,
    /// Time spent waiting for the surface's next texture, since last asked.
    waited: std::time::Duration,
    /// The chunks' records and, one word an instance, where each starts.
    records: Words,
    index: Words,
    /// The property tables (`props` in `shaders.wgsl`) on the GPU, and as
    /// the CPU keeps them.
    props: Table,
    tables: frame::Tables,
    /// The last frame's plan, which the next one brings up to date.
    planned: Option<frame::Plan>,
    /// Each frame planned from what changed is compared with one planned
    /// whole (`GUI_COLD_CHECK`).
    cold_plan: bool,
    /// The images kept chunks show, and how many show each.
    image_refs: foldhash::HashMap<usize, u32>,
    /// Chunks drawn for this frame only (an overlay's).
    transient_ids: Vec<ChunkId>,
    /// This frame went over the whole scene: what it did not hold leaves.
    swept: bool,
    /// Each chunk kept on the GPU, by id.
    kept: foldhash::HashMap<ChunkId, Kept>,
    /// State slots, one a chunk.
    slots: Ids,
    /// Bumped at each encoding: a chunk's encoding has its own.
    epoch: u64,
    /// Records, instances and state slots of this frame only, given back
    /// when it ends.
    transient: Vec<words::Span>,
    transient_index: Vec<words::Span>,
    transient_slots: Vec<u32>,
    /// Text-through masks to draw before what reads them.
    pending_masks: Vec<MaskJob>,
    /// What each part of the canvas shows: an entry's signature and the
    /// pixels it covers (the damage of the next frame is what differs).
    shown: foldhash::HashMap<u64, (u64, Rect)>,
    /// Layers kept with what they hold, and the effects the last frame
    /// drew in a layer (a layer of any other is given back).
    layers: foldhash::HashMap<u32, CachedLayer>,
    layered: foldhash::HashSet<u32>,
    /// Whether an effect's primitives overlap, by what they were.
    overlaps: foldhash::HashMap<u32, (u64, bool)>,
    /// This frame's tables for paths and curve text, and its reads.
    fr: FrameData,
    /// Where reads of what lies below land (`read.rs`).
    reads: read::Reads,
    /// Kept chunks that blur a backdrop: canvas damage then grows around them.
    blurring: usize,
    /// The scene's clips as the shaders walk them, kept across frames.
    clip_tree: clips::ClipTree,
    /// The canvas's clip tree while a capture draws with its own.
    parked_tree: Option<clips::ClipTree>,
    pclips: Scratch,
    stops: Scratch,
    slug_instances: Scratch,
    globals: Scratch,
    /// The globals as last written.
    globals_shown: Vec<u8>,
    /// Bytes between two passes' globals (the adapter's uniform alignment).
    uniform_stride: u64,
    /// Data chunks (ADR-0018): their kinds, GPU state and data.
    data: crate::chunk::runtime::Runtime,
    /// Canvases' tiles and re-anchored paths (ADR-0017).
    world: world::WorldTiles,
    paths: Paths,
    slugs: Slugs,
    /// Whether banded curve text matched the full scan on this adapter
    /// (asked once, at first use).
    banded_ok: Option<bool>,
    bind_groups: BindGroups,
    /// Bind groups made since the renderer was: none in a steady frame.
    bind_groups_made: u64,
    /// Glyphs and icons rasterized since the renderer was made.
    rasters: u64,
    /// What the last frame did.
    drawn: Drawn,
    /// An overlay was drawn over this frame, and over the last.
    overlaid: (bool, bool),
    frame: u64,
}

impl GpuRenderer {
    /// Wait for chunks' reads and send what came back; whether any are
    /// still out (the chunk tests).
    #[cfg(test)]
    pub(crate) fn wait_reads(&mut self) -> bool {
        self.data.deliver_reads(&self.device, true)
    }

    /// The device, queue and data-chunk runtime, for the chunk tests.
    #[cfg(test)]
    pub(crate) fn chunk_probe(&self) -> (&wgpu::Device, &wgpu::Queue, &crate::chunk::runtime::Runtime) {
        (&self.device, &self.queue, &self.data)
    }

    /// A renderer drawing into `window`.
    ///
    /// # Errors
    ///
    /// When no adapter or device answers, the adapter cannot run compute,
    /// or the surface cannot be made.
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
        // Blending happens in the sRGB-encoded values, as browsers blend:
        // never an sRGB view, which would blend in linear light.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .ok_or_else(|| GpuError("no 8-bit unorm surface format".to_owned()))?;
        let mut renderer =
            Self::with_adapter(&instance, &adapter, format, GpuOptions::default(), LostWaker::default())?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            // One frame in flight: what is drawn shows at the next vsync
            // (ADR-0016's latency of one frame).
            desired_maximum_frame_latency: 1,
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
            blitted: false,
        });
        Ok(renderer)
    }

    /// A renderer with no window: it draws into its canvas, and
    /// [`Renderer::capture`] reads a scene back.
    ///
    /// # Errors
    ///
    /// When no adapter or device answers, or the adapter cannot run compute.
    pub fn headless() -> Result<Self, GpuError> {
        Self::headless_with(GpuOptions::default())
    }

    /// [`Self::headless`], made as `options` say.
    ///
    /// # Errors
    ///
    /// As [`Self::headless`].
    pub fn headless_with(options: GpuOptions) -> Result<Self, GpuError> {
        let instance = instance();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|e| GpuError(format!("adapter: {e}")))?;
        Self::with_adapter(
            &instance,
            &adapter,
            wgpu::TextureFormat::Rgba8Unorm,
            options,
            LostWaker::default(),
        )
    }

    /// Whether its device was lost: nothing it holds draws any more, and
    /// [`Self::rebuild`] makes it anew.
    #[must_use]
    pub fn lost(&self) -> bool {
        self.lost.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Run `wake` each time a device of its is lost, as the loss is
    /// reported (on a thread of the driver's), after [`Self::lost`] turns
    /// true: a render thread asleep on its channel is woken to make the
    /// device again (ADR-0015's invariant 7). Kept across [`Self::rebuild`].
    pub fn wake_on_lost(&self, wake: impl Fn() + Send + 'static) {
        *self.wake.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Box::new(wake));
    }

    /// Its device, to lose it from another thread (tests).
    #[cfg(test)]
    pub(crate) fn device(&self) -> wgpu::Device {
        self.device.clone()
    }

    /// Lose the device, as a driver reset would (tests, the devtools).
    pub fn lose(&self) {
        self.device.destroy();
        // The lost callback runs once the queue is drained.
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok();
    }

    /// Make everything on a new device (ADR-0015's invariant 7): the same
    /// instance and, for a window, the same surface, configured again. What
    /// it kept (chunks, atlases, layers, images) is gone; the next frame
    /// encodes and rasterizes all of it again, drawing the canvas whole.
    ///
    /// # Errors
    ///
    /// When no adapter or device answers yet (the GPU is still resetting);
    /// it is left as it was, to try again.
    pub fn rebuild(&mut self) -> Result<(), GpuError> {
        let compatible = self.window.as_ref().map(|w| &w.surface);
        let adapter = pollster::block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: compatible,
            ..Default::default()
        }))
        .map_err(|e| GpuError(format!("adapter: {e}")))?;
        let mut fresh = Self::with_adapter(
            &self.instance,
            &adapter,
            self.format,
            self.options,
            Arc::clone(&self.wake),
        )?;
        if let Some(mut w) = self.window.take() {
            // A texture of the lost device is never presented.
            w.frame = None;
            w.blitted = false;
            w.surface.configure(&fresh.device, &w.config);
            fresh.window = Some(w);
        }
        fresh.frame = self.frame;
        *self = fresh;
        Ok(())
    }

    #[expect(clippy::too_many_lines, reason = "every pipeline and layout, made once")]
    fn with_adapter(
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        format: wgpu::TextureFormat,
        options: GpuOptions,
        wake: LostWaker,
    ) -> Result<Self, GpuError> {
        let needed = wgpu::DownlevelFlags::COMPUTE_SHADERS | wgpu::DownlevelFlags::INDIRECT_EXECUTION;
        if !adapter.get_downlevel_capabilities().flags.contains(needed) {
            return Err(GpuError("the adapter cannot run compute shaders".to_owned()));
        }
        let dual_source = options.dual_source && adapter.features().contains(wgpu::Features::DUAL_SOURCE_BLENDING);
        let limits = adapter.limits();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("gui"),
            // Timestamps time data chunks' passes where the adapter has them (ADR-0018).
            required_features: if dual_source {
                wgpu::Features::DUAL_SOURCE_BLENDING
            } else {
                wgpu::Features::empty()
            } | (adapter.features() & wgpu::Features::TIMESTAMP_QUERY),
            required_limits: limits.clone(),
            ..Default::default()
        }))
        .map_err(|e| GpuError(format!("device: {e}")))?;
        let lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let on_lost = Arc::clone(&lost);
        let waker = Arc::clone(&wake);
        device.set_device_lost_callback(move |_, _| {
            on_lost.store(true, std::sync::atomic::Ordering::Release);
            // A thread asleep waiting for work hears of it now, not at
            // its next message.
            if let Some(wake) = waker.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
                wake();
            }
        });
        let texture_entry = |binding: u32, dimension: wgpu::TextureViewDimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: dimension,
                multisampled: false,
            },
            count: None,
        };
        let storage = |binding: u32, visibility: wgpu::ShaderStages, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let globals_entry = |size: u64| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let d2 = wgpu::TextureViewDimension::D2;
        let array = wgpu::TextureViewDimension::D2Array;
        let layout = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let layouts = Layouts {
            quad: layout(
                "gui quads",
                &[
                    globals_entry(GLOBALS),
                    storage(1, vf, true),
                    storage(2, vf, true),
                    storage(3, vf, true),
                    texture_entry(4, array),
                    texture_entry(5, array),
                    wgpu::BindGroupLayoutEntry {
                        binding: 6,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    texture_entry(7, d2),
                ],
            ),
            extra: layout("gui extra", &[texture_entry(0, d2)]),
            path: layout(
                "gui paths",
                &[
                    globals_entry(16),
                    storage(1, vf, true),
                    storage(2, vf, true),
                    storage(3, vf, true),
                    storage(4, vf, true),
                    storage(5, vf, true),
                    storage(6, vf, true),
                    storage(7, vf, true),
                ],
            ),
            compute: {
                let c = wgpu::ShaderStages::COMPUTE;
                layout(
                    "gui path passes",
                    &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: c,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: wgpu::BufferSize::new(32),
                            },
                            count: None,
                        },
                        storage(1, c, true),
                        storage(2, c, true),
                        storage(3, c, true),
                        storage(4, c, false),
                        storage(5, c, false),
                        storage(6, c, false),
                        storage(7, c, false),
                        storage(8, c, false),
                        storage(9, c, false),
                    ],
                )
            },
            slug: layout(
                "gui curve text",
                &[
                    globals_entry(16),
                    storage(1, vf, true),
                    storage(2, vf, true),
                    storage(3, vf, true),
                    texture_entry(4, d2),
                ],
            ),
            blit: layout("gui canvas", &[texture_entry(0, d2)]),
            read: layout(
                "gui read",
                &[
                    texture_entry(0, d2),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: wgpu::BufferSize::new(32),
                        },
                        count: None,
                    },
                ],
            ),
        };
        let pipeline_layout = |label: &str, groups: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                immediate_size: 0,
            })
        };
        let quad_layout = pipeline_layout("gui quads", &[Some(&layouts.quad), Some(&layouts.extra)]);
        let path_layout = pipeline_layout("gui paths", &[Some(&layouts.path)]);
        let compute_layout = pipeline_layout("gui path passes", &[Some(&layouts.compute)]);
        let slug_layout = pipeline_layout("gui curve text", &[Some(&layouts.slug)]);
        let blit_layout = pipeline_layout("gui canvas", &[Some(&layouts.blit)]);
        let read_layout = pipeline_layout("gui read", &[Some(&layouts.read)]);
        let common = include_str!("common.wgsl");
        let stops = include_str!("stops.wgsl");
        let quad_module = module(
            &device,
            "gui quads",
            &[common, include_str!("shaders.wgsl")],
            &[("fs_main", "Varying", "shade")],
            dual_source,
        );
        let path_module = module(
            &device,
            "gui paths",
            &[common, stops, include_str!("path_render.wgsl")],
            &[("fs_path", "PathVarying", "shade_path")],
            dual_source,
        );
        let slug_module = module(
            &device,
            "gui curve text",
            &[common, include_str!("slug.wgsl")],
            &[
                ("fs_slug", "SlugVarying", "shade_slug_banded"),
                ("fs_slug_full", "SlugVarying", "shade_slug_full"),
            ],
            dual_source,
        );
        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gui canvas"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
        });
        let read_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gui read"),
            source: wgpu::ShaderSource::Wgsl(include_str!("read.wgsl").into()),
        });
        let compute_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gui path passes"),
            source: wgpu::ShaderSource::Wgsl(include_str!("path_compute.wgsl").into()),
        });
        let alpha = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        // What each channel keeps of the target is the second output's
        // (ADR-0019): one blend state for every primitive.
        let blend = wgpu::BlendState {
            color: if dual_source {
                wgpu::BlendComponent {
                    dst_factor: wgpu::BlendFactor::OneMinusSrc1,
                    ..alpha
                }
            } else {
                alpha
            },
            alpha,
        };
        let render = |label: &str,
                      layout: &wgpu::PipelineLayout,
                      module: &wgpu::ShaderModule,
                      vs: &str,
                      fs: &str,
                      format: wgpu::TextureFormat,
                      blend: Option<wgpu::BlendState>,
                      topology: wgpu::PrimitiveTopology| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let strip = wgpu::PrimitiveTopology::TriangleStrip;
        // Coverage onto coverage: the union of an icon's parts.
        let union = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrc,
            operation: wgpu::BlendOperation::Add,
        };
        let pipelines = Pipelines {
            quad: render(
                "gui quads",
                &quad_layout,
                &quad_module,
                "vs_main",
                "fs_main",
                format,
                Some(blend),
                strip,
            ),
            fill: render(
                "gui fills",
                &quad_layout,
                &quad_module,
                "vs_main",
                "fs_fill",
                format,
                None,
                strip,
            ),
            path: render(
                "gui paths",
                &path_layout,
                &path_module,
                "vs_path",
                "fs_path",
                format,
                Some(blend),
                strip,
            ),
            mask: render(
                "gui path coverage",
                &path_layout,
                &path_module,
                "vs_path",
                "fs_mask",
                wgpu::TextureFormat::R8Unorm,
                Some(wgpu::BlendState {
                    color: union,
                    alpha: union,
                }),
                strip,
            ),
            slug_banded: render(
                "gui curve text",
                &slug_layout,
                &slug_module,
                "vs_slug",
                "fs_slug",
                format,
                Some(blend),
                strip,
            ),
            slug_full: render(
                "gui curve text, full scan",
                &slug_layout,
                &slug_module,
                "vs_slug",
                "fs_slug_full",
                format,
                Some(blend),
                strip,
            ),
            blit: render(
                "gui canvas",
                &blit_layout,
                &blit_module,
                "vs_blit",
                "fs_blit",
                format,
                None,
                wgpu::PrimitiveTopology::TriangleList,
            ),
            read: render(
                "gui read",
                &read_layout,
                &read_module,
                "vs_read",
                "fs_read",
                format,
                None,
                wgpu::PrimitiveTopology::TriangleList,
            ),
            compute: ComputePipelines::new(&device, &compute_layout, &compute_module),
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gui linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let blank = texture(
            &device,
            &queue,
            &layouts.extra,
            (1, 1),
            wgpu::TextureFormat::Rgba8Unorm,
            Some(&[0, 0, 0, 0]),
        );
        let gamma = GammaParams::linux();
        let gamma_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui gamma"),
            size: wgpu::Extent3d {
                width: 256,
                height: 8,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let gamma_view = gamma_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let side = options.atlas_side.clamp(64, limits.max_texture_dimension_2d);
        let max_pages = options.max_pages.clamp(1, limits.max_texture_array_layers.min(64));
        let storage_usage = wgpu::BufferUsages::STORAGE;
        Ok(Self {
            instance: instance.clone(),
            options,
            lost,
            wake,
            dual_source,
            foreground: foldhash::HashMap::default(),
            gamma_texture,
            gamma_view,
            gamma,
            darkened: false,
            lcd: None,
            text_generation: None,
            mono: Atlas::new(
                &device,
                "gui coverage atlas",
                wgpu::TextureFormat::R8Unorm,
                1,
                side,
                max_pages,
            ),
            color: Atlas::new(
                &device,
                "gui colour atlas",
                wgpu::TextureFormat::Rgba8Unorm,
                4,
                side,
                max_pages,
            ),
            records: Words::new(&device, "gui records", 1 << 16),
            index: Words::new(&device, "gui instances", 1 << 12),
            props: Table::new(&device, "gui properties"),
            tables: frame::Tables::default(),
            planned: None,
            cold_plan: cfg!(debug_assertions) && std::env::var_os("GUI_COLD_CHECK").is_some(),
            image_refs: foldhash::HashMap::default(),
            transient_ids: Vec::new(),
            swept: false,
            kept: foldhash::HashMap::default(),
            slots: Ids::default(),
            epoch: 0,
            transient: Vec::new(),
            transient_index: Vec::new(),
            transient_slots: Vec::new(),
            pending_masks: Vec::new(),
            shown: foldhash::HashMap::default(),
            layers: foldhash::HashMap::default(),
            layered: foldhash::HashSet::default(),
            overlaps: foldhash::HashMap::default(),
            fr: FrameData::default(),
            reads: read::Reads::new(&device),
            blurring: 0,
            clip_tree: clips::ClipTree::default(),
            parked_tree: None,
            pclips: Scratch::new(&device, "gui clips", storage_usage, 1 << 12),
            stops: Scratch::new(&device, "gui stops", storage_usage, 1 << 12),
            slug_instances: Scratch::new(&device, "gui curve glyphs", storage_usage, 1 << 12),
            globals: Scratch::new(&device, "gui globals", wgpu::BufferUsages::UNIFORM, 256 * 16),
            globals_shown: Vec::new(),
            uniform_stride: u64::from(limits.min_uniform_buffer_offset_alignment).max(GLOBALS),
            data: crate::chunk::runtime::Runtime::new(
                &device,
                &queue,
                format,
                limits.min_uniform_buffer_offset_alignment,
            ),
            world: world::WorldTiles::default(),
            paths: Paths::new(&device),
            slugs: Slugs::new(&device),
            banded_ok: None,
            bind_groups: BindGroups::default(),
            bind_groups_made: 1,
            rasters: 0,
            drawn: Drawn::default(),
            overlaid: (false, false),
            device,
            queue,
            format,
            pipelines,
            layouts,
            sampler,
            blank,
            window: None,
            canvas: None,
            glyphs: foldhash::HashMap::default(),
            icons: HashMap::new(),
            masks: Vec::new(),
            images: FrameCache::default(),
            waited: std::time::Duration::ZERO,
            frame: 1,
        })
    }

    /// Make the window's surface `size` physical pixels.
    pub fn resize(&mut self, size: (u32, u32)) {
        let Some(w) = self.window.as_mut() else { return };
        let size = (size.0.max(1), size.1.max(1));
        if (w.config.width, w.config.height) != size {
            w.frame = None;
            w.blitted = false;
            w.config.width = size.0;
            w.config.height = size.1;
            w.surface.configure(&self.device, &w.config);
        }
    }

    /// Take the window's next frame texture now, waiting for the display
    /// if it has none free (ADR-0015: the render thread's vsync); how long
    /// it waited, or `None` with no window or no texture. The frame is
    /// presented when it ends, drawn into or not.
    pub fn acquire(&mut self) -> Option<std::time::Duration> {
        let before = self.waited;
        self.window_view()?;
        Some(self.waited.saturating_sub(before))
    }

    /// Whether a window frame texture is held for this frame.
    fn holds_frame(&self) -> bool {
        self.window.as_ref().is_some_and(|w| w.frame.is_some())
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
            w.blitted = false;
            self.waited = self.waited.saturating_add(asked.elapsed());
        }
        let frame = w.frame.as_ref()?;
        Some(frame.texture.create_view(&wgpu::TextureViewDescriptor::default()))
    }

    /// The canvas, `size` and cleared to `background`: made anew (and so
    /// to be drawn whole) when either differs; whether it was.
    fn ensure_canvas(&mut self, size: (u32, u32), background: Option<[u8; 4]>) -> bool {
        let size = (size.0.max(1), size.1.max(1));
        if let Some(c) = self.canvas.as_mut()
            && c.size == size
        {
            let same = c.background == background;
            c.background = background;
            return !same;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui canvas"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let blit = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gui canvas"),
            layout: &self.layouts.blit,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        self.bind_groups_made = self.bind_groups_made.saturating_add(1);
        self.canvas = Some(Canvas {
            texture,
            view,
            blit,
            size,
            background,
        });
        true
    }

    /// Forget what an atlas held on a page it emptied. No chunk kept on the
    /// GPU reads the page: pages they read are touched every frame.
    fn evicted(&mut self, atlas: Page, page: u32) {
        let gone = |p: &Option<Placed>| p.is_some_and(|p| p.page == atlas && p.slot.page == page);
        self.glyphs.retain(|_, p| !gone(p));
        self.icons.retain(|_, p| !gone(p));
    }

    /// Room in `atlas` for `w`×`h`.
    fn allocate(&mut self, atlas: Page, w: u32, h: u32) -> Option<Slot> {
        let frame = self.frame;
        let a = match atlas {
            Page::Mono => &mut self.mono,
            Page::Color => &mut self.color,
        };
        let (slot, made) = a.allocate(&self.device, &self.queue, w, h, frame)?;
        if let Made::Evicted(page) = made {
            self.evicted(atlas, page);
        }
        Some(slot)
    }

    /// Bring the gamma table and the glyphs up to the text settings: a
    /// change (the user turned subpixel text on) makes them anew, and every
    /// chunk is encoded again.
    fn sync_text(&mut self, text: &Glyphs) {
        if self.text_generation == Some(text.generation()) {
            return;
        }
        let (params, table) = text.gamma();
        self.gamma = *params;
        self.upload_gamma(table);
        let settings = text.settings();
        self.lcd = settings.subpixel.filter(|_| self.dual_source && settings.antialias);
        self.darkened = settings.stem_darkening;
        let mono = &mut self.mono;
        let color = &mut self.color;
        for p in self.glyphs.drain().filter_map(|(_, p)| p) {
            match p.page {
                Page::Mono => mono.free(p.slot),
                Page::Color => color.free(p.slot),
            }
        }
        self.foreground.clear();
        self.text_generation = Some(text.generation());
    }

    fn upload_gamma(&self, table: &MaskGamma) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.gamma_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &table.texture_bytes(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(8),
            },
            wgpu::Extent3d {
                width: 256,
                height: 8,
                depth_or_array_layers: 1,
            },
        );
    }

    /// How glyph `key` in `color` is drawn: in subpixel coverage only
    /// under Chrome's conditions (straight onto the opaque window, at most
    /// 48 device pixels, not a colour glyph, the user has it on), and for a
    /// colour glyph that paints with the text's colour, keyed by it.
    fn glyph_slot(&self, key: &GlyphKey, color: Color, straight: bool) -> GlyphSlot {
        let coverage = match self.lcd {
            Some(stripes) if straight && !key.is_color() && f32::from_bits(key.ppem) <= MAX_LCD_PPEM => {
                Coverage::Lcd(stripes)
            }
            _ => Coverage::Gray,
        };
        let foreground =
            (key.is_color() && self.foreground.get(key).copied().unwrap_or(false)).then(|| color.to_rgba8());
        GlyphSlot {
            key: *key,
            coverage,
            foreground,
        }
    }

    fn glyph(&mut self, slot: GlyphSlot, color: Color, text: &mut Glyphs) -> Option<Placed> {
        let frame = self.frame;
        if let Some(p) = self.glyphs.get(&slot) {
            if let Some(p) = p {
                let atlas = match p.page {
                    Page::Mono => &mut self.mono,
                    Page::Color => &mut self.color,
                };
                atlas.touch(&p.slot, frame);
            }
            return *p;
        }
        self.rasters = self.rasters.saturating_add(1);
        let Some(image) = text.raster(&slot.key, slot.coverage, color) else {
            self.glyphs.insert(slot, None);
            return None;
        };
        let (w, h) = (image.width, image.height);
        if w == 0 || h == 0 {
            self.glyphs.insert(slot, None);
            return None;
        }
        // Subpixel coverage sits in the colour atlas too: four bytes a pixel.
        let page = match image.content {
            GlyphContent::Mask => Page::Mono,
            GlyphContent::Color | GlyphContent::Lcd => Page::Color,
        };
        let Some(atlas_slot) = self.allocate(page, w, h) else {
            self.glyphs.insert(slot, None);
            return None;
        };
        match page {
            Page::Mono => self.mono.upload(&self.queue, atlas_slot, w, h, &image.data),
            Page::Color => self.color.upload(&self.queue, atlas_slot, w, h, &image.data),
        }
        let placed = Placed {
            page,
            slot: atlas_slot,
            width: w,
            height: h,
            left: image.left,
            top: image.top,
        };
        self.glyphs.insert(slot, Some(placed));
        Some(placed)
    }

    /// Icon `icon` at `size` device pixels in the coverage atlas: rasterized
    /// once, by the path passes, the first time it is drawn.
    fn icon(&mut self, icon: Icon, size: u32, stroke_width: f32) -> Option<Placed> {
        let key = IconKey {
            icon,
            size,
            stroke: stroke_width.to_bits(),
        };
        let frame = self.frame;
        if let Some(p) = self.icons.get(&key) {
            if let Some(p) = p {
                self.mono.touch(&p.slot, frame);
            }
            return *p;
        }
        let Some(drawing) = icon.drawing() else {
            self.icons.insert(key, None);
            return None;
        };
        if size == 0 {
            self.icons.insert(key, None);
            return None;
        }
        let Some(slot) = self.allocate(Page::Mono, size, size) else {
            self.icons.insert(key, None);
            return None;
        };
        self.rasters = self.rasters.saturating_add(1);
        // The slot starts empty; the parts' coverages add up in it.
        let n = usize::try_from(size.saturating_mul(size)).unwrap_or(0);
        self.mono.upload(&self.queue, slot, size, size, &vec![0; n]);
        let k = u32_to_f32(size) / drawing.view.max(1e-3);
        let m = [k, 0.0, 0.0, k, u32_to_f32(slot.x), u32_to_f32(slot.y)];
        let visible = [
            u32_to_f32(slot.x),
            u32_to_f32(slot.y),
            u32_to_f32(slot.x.saturating_add(size)),
            u32_to_f32(slot.y.saturating_add(size)),
        ];
        let paint = Paint {
            color: [1.0; 4],
            gradient: None,
            alpha: 1.0,
            device_clip: 0,
            local_clip: 0,
        };
        for (part, style) in icon_parts(drawing, stroke_width, Color::WHITE) {
            if let Some(d) = self
                .paths
                .draw(&self.device, &self.queue, &part, &style, m, visible, &paint, true)
            {
                self.masks.push(MaskDraw {
                    page: slot.page,
                    instances: d.instances,
                });
            }
        }
        let placed = Placed {
            page: Page::Mono,
            slot,
            width: size,
            height: size,
            left: 0,
            top: 0,
        };
        self.icons.insert(key, Some(placed));
        Some(placed)
    }

    /// `source` on the GPU, marked used this frame; whether it is.
    fn image(&mut self, source: &ImageSource) -> bool {
        let key = image_key(source);
        let (device, queue, layout) = (&self.device, &self.queue, &self.layouts.extra);
        let made = &mut self.bind_groups_made;
        self.images
            .get_or_insert_with(key, || {
                let size = source.size();
                (size.0 > 0 && size.1 > 0).then(|| {
                    *made = made.saturating_add(1);
                    ImageTexture {
                        _source: source.clone(),
                        texture: texture(
                            device,
                            queue,
                            layout,
                            size,
                            wgpu::TextureFormat::Rgba8Unorm,
                            Some(source.0.data()),
                        ),
                    }
                })
            })
            .is_some()
    }

    /// A texture `size` to draw a layer or mask into.
    fn layer_texture(&mut self, size: (u32, u32)) -> Texture {
        self.bind_groups_made = self.bind_groups_made.saturating_add(1);
        texture(&self.device, &self.queue, &self.layouts.extra, size, self.format, None)
    }

    /// Read `texture` (`size`, this renderer's format) back.
    fn read_back(&self, texture: &wgpu::Texture, size: (u32, u32)) -> Option<Pixels> {
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
        Pixels::from_vec(size.0, size.1, data)
    }

    /// Draw `scene` into the canvas and wait for the GPU to finish it,
    /// reading nothing back: what a window's frame costs, measured headless.
    pub fn draw_and_wait(&mut self, scene: &Scene, changed: &Changed, text: &mut Glyphs) {
        self.render(scene, changed, text);
        let asked = std::time::Instant::now();
        drop(self.device.poll(wgpu::PollType::wait_indefinitely()));
        self.drawn.gpu_wait = asked.elapsed();
    }

    /// The canvas as drawn, read back (tests: what damage left there).
    pub fn read_canvas(&self) -> Option<Pixels> {
        let c = self.canvas.as_ref()?;
        self.read_back(&c.texture, c.size)
    }

    /// Bind groups made since the renderer was (devtools, tests).
    #[must_use]
    pub fn bind_groups_made(&self) -> u64 {
        self.bind_groups_made
    }

    /// Glyphs and icons rasterized since the renderer was made.
    #[must_use]
    pub fn rasters(&self) -> u64 {
        self.rasters
    }

    /// Whether dual-source blending is on (subpixel text can be drawn).
    #[must_use]
    pub fn dual_source(&self) -> bool {
        self.dual_source
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

impl Renderer for GpuRenderer {
    fn render(&mut self, scene: &Scene, changed: &Changed, text: &mut Glyphs) {
        self.render_scene(scene, changed, text);
        self.check_chunks_cold();
    }

    fn overlay(&mut self, scene: &Scene, text: &mut Glyphs) {
        self.overlay_scene(scene, text);
    }

    fn end_frame(&mut self) {
        if let Some(w) = self.window.as_mut()
            && let Some(frame) = w.frame.take()
        {
            w.blitted = false;
            self.queue.present(frame);
        }
        self.images.end_frame();
        self.paths.end_frame();
        self.slugs.end_frame();
        self.end_kept();
        self.end_layers();
        self.data.end_frame();
        self.data.deliver_reads(&self.device, false);
        self.drawn.raster = self.world.spent();
        self.drawn.tiles_done = self.world.done();
        self.world.end_frame(&mut self.data.store);
        self.frame = self.frame.saturating_add(1);
        self.overlaid = (false, self.overlaid.0);
    }

    fn waited(&mut self) -> std::time::Duration {
        std::mem::take(&mut self.waited)
    }

    fn held(&self) -> Held {
        let count = |n: usize, bytes: usize| Raster { count: n, bytes };
        let glyphs = self.glyphs.values().flatten().count();
        let icons = self.icons.values().flatten().count();
        let masks: usize = self.kept.values().flat_map(|k| &k.masks).map(Texture::bytes).sum();
        Held {
            glyphs: count(
                glyphs.saturating_add(self.slugs.held()),
                self.color.bytes().saturating_add(self.slugs.bytes()),
            ),
            icons: count(icons, self.mono.bytes()),
            images: count(
                self.images.len(),
                self.images.values().flatten().map(|i| i.texture.bytes()).sum(),
            ),
            paths: count(self.paths.jobs_held(), self.paths.bytes()),
            layers: count(
                self.layers.len(),
                self.layers
                    .values()
                    .map(|l| l.texture.bytes())
                    .sum::<usize>()
                    .saturating_add(masks)
                    .saturating_add(self.reads.bytes())
                    .saturating_add(self.canvas.as_ref().map_or(0, |c| {
                        usize::try_from(
                            u64::from(c.size.0)
                                .saturating_mul(u64::from(c.size.1))
                                .saturating_mul(4),
                        )
                        .unwrap_or(0)
                    })),
            ),
            // Data chunks (ADR-0018) are chunks too: their state and data.
            chunks: count(
                self.kept.len().saturating_add(self.data.held().0),
                usize::try_from(
                    self.records
                        .used_bytes()
                        .saturating_add(self.index.used_bytes())
                        .saturating_add(self.data.held().1),
                )
                .unwrap_or(0),
            ),
        }
    }

    fn drawn(&self) -> Drawn {
        let (gpu, gpu_passes) = self.data.gpu_times();
        Drawn {
            gpu,
            gpu_passes,
            ..self.drawn
        }
    }

    fn capture(&mut self, scene: &Scene, overlay: Option<&Scene>, text: &mut Glyphs) -> Option<Pixels> {
        self.capture_scene(scene, overlay, text)
    }
}

/// A shader module of `parts` (concatenated), with a fragment entry point
/// for each of `entries` (name, input type, shading function): with
/// dual-source blending, whose second output is what of each channel of
/// the target is kept (ADR-0019), or without.
fn module(
    device: &wgpu::Device,
    label: &str,
    parts: &[&str],
    entries: &[(&str, &str, &str)],
    dual_source: bool,
) -> wgpu::ShaderModule {
    let mut source = String::new();
    if dual_source {
        source.push_str("enable dual_source_blending;\n");
    }
    for p in parts {
        source.push_str(p);
        source.push('\n');
    }
    if dual_source {
        source.push_str(
            "struct DualOut {\n    @location(0) @blend_src(0) color: vec4<f32>,\n    \
             @location(0) @blend_src(1) mask: vec4<f32>,\n}\n",
        );
    }
    for (name, input, shade) in entries {
        if dual_source {
            source.push_str(&format!(
                "@fragment\nfn {name}(input: {input}) -> DualOut {{\n    let s = {shade}(input);\n    \
                 return DualOut(s.color, s.mask);\n}}\n"
            ));
        } else {
            source.push_str(&format!(
                "@fragment\nfn {name}(input: {input}) -> @location(0) vec4<f32> {{\n    \
                 return {shade}(input).color;\n}}\n"
            ));
        }
    }
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

/// Metal, Vulkan and DX12 only (`WGPU_BACKEND` may narrow them): no OpenGL.
///
/// One per process, as Chrome's GPU process keeps one: every renderer (each
/// window, each headless one, a rebuilt one) asks it for its adapter and
/// device. Drivers make and tear down an instance under process-wide state
/// (Mesa's lavapipe crashes when several threads do it at once).
fn instance() -> wgpu::Instance {
    static INSTANCE: std::sync::OnceLock<wgpu::Instance> = std::sync::OnceLock::new();
    INSTANCE
        .get_or_init(|| {
            let ours = wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12;
            wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::from_env().map_or(ours, |b| b & ours),
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            })
        })
        .clone()
}

/// A texture `size`, filled with `data` (rows of `size.0` RGBA pixels) when
/// given, and the bind group that reads it.
fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
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
        // A layer is read back by what draws over it (`read.rs`).
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC
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
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gui extra"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&view),
        }],
    });
    Texture {
        texture,
        view,
        bind,
        size: (extent.width, extent.height),
    }
}

/// What names an image on the GPU: the address of its pixels.
fn image_key(source: &ImageSource) -> usize {
    Arc::as_ptr(&source.0).addr()
}

fn floats_to_bytes(floats: &[f32]) -> Vec<u8> {
    floats.iter().flat_map(|f| f.to_ne_bytes()).collect()
}

/// Whether glyph `key` is drawn as curves: above the atlas's largest size,
/// or under a transform that is not a translation (`transformed`).
fn wants_curves(key: &GlyphKey, transformed: bool) -> bool {
    !key.is_color() && (f32::from_bits(key.ppem) > MAX_ATLAS_PPEM || transformed)
}

/// The map from a glyph's curve pixels (y up, from its pen at `x`, `y`
/// physical) to the target, through `g` when it is transformed.
fn glyph_matrix(key: &GlyphKey, x: f32, y: f32, g: Option<[f32; 6]>) -> [f32; 6] {
    let k = f32::from_bits(key.ppem) / crate::text::CURVE_EM;
    let pen = x + f32::from(key.bin) / 4.0;
    let m = [k, 0.0, 0.0, -k, pen, y];
    g.map_or(m, |g| path::then(m, g))
}

/// An icon's parts as paths and styles, its strokes `stroke_width` wide
/// (view-box units) in `color`.
fn icon_parts(drawing: &crate::svg::Drawing, stroke_width: f32, color: Color) -> Vec<(Path, PathStyle)> {
    let mut out = Vec::new();
    for part in &drawing.parts {
        if part.fill {
            out.push((part.path.clone(), PathStyle::fill(crate::scene::Gradient::Solid(color))));
        }
        if part.stroke {
            let stroke = Stroke::new(stroke_width).cap(drawing.cap).join(drawing.join);
            out.push((
                part.path.clone(),
                PathStyle {
                    paint: crate::scene::Gradient::Solid(color),
                    draw: PathDraw::Stroke(stroke),
                },
            ));
        }
    }
    out
}

/// Add `d` to `ops`, joined to the last when it continues it.
fn push_op(ops: &mut Vec<Op>, d: Op) {
    match (ops.last_mut(), &d) {
        (Some(Op::Quads { extra, instances }), Op::Quads { extra: e, instances: i })
            if extra == e && instances.end == i.start =>
        {
            instances.end = i.end;
        }
        (Some(Op::Path(r)), Op::Path(i)) | (Some(Op::Slug(r)), Op::Slug(i)) if r.end == i.start => {
            r.end = i.end;
        }
        _ => ops.push(d),
    }
}

/// `b` (logical) in physical pixels: x, y, width, height.
fn phys(b: Bounds, s: f32) -> [f32; 4] {
    [b.x * s, b.y * s, b.width * s, b.height * s]
}

/// The radii `r` of box `b` (logical) as used, CSS's overlapping-curves
/// rule applied (`Shape::used`: none below 0, two on a side never past
/// it), in physical pixels: what the shaders' coverage assumes, whoever
/// made the item.
fn radii(r: Corners, b: Bounds, s: f32) -> [f32; 4] {
    let r = crate::shape::Shape::Rounded(r).used(b.width, b.height);
    [r.top_left * s, r.top_right * s, r.bottom_right * s, r.bottom_left * s]
}

fn rgba(c: Color) -> [f32; 4] {
    [
        c.r.clamp(0.0, 1.0),
        c.g.clamp(0.0, 1.0),
        c.b.clamp(0.0, 1.0),
        c.a.clamp(0.0, 1.0),
    ]
}
