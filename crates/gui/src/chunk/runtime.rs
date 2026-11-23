//! Data chunks on the render thread (ADR-0018): every kind registered, each
//! chunk's GPU state, and the frame's compute passes, coverage passes and
//! main-pass draws.
//!
//! A chunk is brought up to date only when what decides its pixels changed
//! (its parameters, its data's versions, its size and the fraction of a
//! pixel it starts at); then its kind uploads, records its dispatches and
//! its draws. Otherwise the draws it recorded are replayed and its coverage
//! texture composited as it is. Kinds record commands; the renderer runs
//! them: dispatches in one compute pass a chunk before the frame's passes,
//! coverage draws in one render pass a chunk into its own texture, main
//! draws where the chunk stands in its pass, so a kind never opens a pass
//! or picks a target. Bind groups are made when what they hold changed.
//! Compute and coverage passes are timed with timestamp queries where the
//! adapter has them.

use std::any::Any;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Mutex;

use super::store::{BufferId, ResidentPage, Store, TextureId};
pub(crate) use super::timing::Stamped;
use super::timing::Timing;
use super::{ChunkKind, DataItem, GridSnapshot, KindId, Source};
use crate::chunk::series::SeriesSnapshot;
use crate::color::Color;

/// A chunk not drawn for this many frames lets go of its GPU state.
const SLOT_FRAMES: u64 = 240;
/// Bytes of one chunk's uniform block (`ChunkItem` in `chunk.wgsl`).
const ITEM_BYTES: usize = 96;
/// The format coverage is drawn in: areas add in red, lines take the greatest in alpha.
const COVERAGE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// A compute pipeline a kind built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ComputeId(u32);

/// A draw pipeline a kind built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DrawId(u32);

/// What a pipeline binding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindKind {
    Uniform,
    /// Storage the pipeline reads.
    Storage,
    /// Storage a compute pipeline writes.
    StorageWrite,
    /// A float texture read with `textureLoad`.
    Texture,
}

/// What a binding holds in a dispatch or draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bind {
    Buffer(BufferId),
    /// `size` bytes of a buffer from `offset` (a uniform block).
    Range(BufferId, u64, u64),
    /// A series page, by its [`super::store::ResidentPage::seq`].
    Page(u64),
    Texture(TextureId),
    /// The chunk's own coverage texture.
    Coverage,
}

/// Where a draw pipeline draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The main pass: premultiplied colour over what is there.
    Main,
    /// The chunk's coverage, areas: added into red.
    Area,
    /// The chunk's coverage, lines: the greatest into alpha.
    Line,
}

/// What changed of a chunk's data since it was last brought up to date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataOp {
    /// Source `source` is new or every point of it was replaced.
    Replaced { source: usize },
    /// Points from `from` to `to` (places among all the series was ever
    /// given) were appended.
    Appended { source: usize, from: u64, to: u64 },
    /// Points before `until` were let go.
    Dropped { source: usize, until: u64 },
}

/// Where a chunk draws this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameInfo {
    /// Its texture: its region's device pixels, floored.
    pub size: (u32, u32),
    /// Where its data maps in the texture's pixels: x, y, width, height.
    pub bounds: [f32; 4],
    pub scale: f32,
}

/// A series source as uploaded: its snapshot and its pages on the GPU.
#[derive(Clone, Copy, Debug)]
pub struct SeriesPages<'a> {
    pub snapshot: &'a SeriesSnapshot,
    pub pages: &'a [ResidentPage],
}

struct ComputePipe {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

struct DrawPipe {
    pipeline: wgpu::RenderPipeline,
    layout: Option<wgpu::BindGroupLayout>,
    target: Target,
}

#[derive(Default)]
pub(crate) struct Pipes {
    compute: Vec<ComputePipe>,
    draw: Vec<DrawPipe>,
}

/// Builds a kind's pipelines when a renderer is made.
pub struct PipelineBuilder<'a> {
    device: &'a wgpu::Device,
    format: wgpu::TextureFormat,
    frame: &'a wgpu::BindGroupLayout,
    pipes: &'a mut Pipes,
}

fn layout_entries(binds: &[BindKind], visibility: wgpu::ShaderStages) -> Vec<wgpu::BindGroupLayoutEntry> {
    binds
        .iter()
        .enumerate()
        .map(|(i, b)| wgpu::BindGroupLayoutEntry {
            binding: u32::try_from(i).unwrap_or(0),
            visibility,
            ty: match b {
                BindKind::Uniform => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                BindKind::Storage => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                BindKind::StorageWrite => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                BindKind::Texture => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            },
            count: None,
        })
        .collect()
}

impl PipelineBuilder<'_> {
    /// A compute pipeline from WGSL `source` (its bindings group 0, in
    /// the order of `binds`) and its entry point.
    /// The device's limits: what a kind may bind.
    pub fn limits(&self) -> wgpu::Limits {
        self.device.limits()
    }

    pub fn compute(&mut self, label: &str, source: &str, entry: &str, binds: &[BindKind]) -> ComputeId {
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &layout_entries(binds, wgpu::ShaderStages::COMPUTE),
        });
        let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: Some(&pl),
            module: &module,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        self.pipes.compute.push(ComputePipe { pipeline, layout });
        ComputeId(u32::try_from(self.pipes.compute.len().saturating_sub(1)).unwrap_or(0))
    }

    /// A draw pipeline from WGSL `source`, after `common.wgsl` and
    /// `chunk.wgsl` (group 0, the chunk's frame); its own bindings are
    /// group 1, in the order of `binds`. It draws triangle strips.
    pub fn draw(
        &mut self,
        label: &str,
        source: &str,
        vertex: &str,
        fragment: &str,
        binds: &[BindKind],
        target: Target,
    ) -> DrawId {
        let full = format!(
            "{}\n{}\n{}",
            include_str!("../gpu/common.wgsl"),
            include_str!("chunk.wgsl"),
            source
        );
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(full.into()),
        });
        let layout = (!binds.is_empty()).then(|| {
            self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries: &layout_entries(binds, wgpu::ShaderStages::VERTEX_FRAGMENT),
            })
        });
        let groups: Vec<Option<&wgpu::BindGroupLayout>> = std::iter::once(Some(self.frame))
            .chain(layout.as_ref().map(Some))
            .collect();
        let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &groups,
            immediate_size: 0,
        });
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let max = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Max,
        };
        let (format, blend, write_mask) = match target {
            Target::Main => (
                self.format,
                wgpu::BlendState {
                    color: premultiplied,
                    alpha: premultiplied,
                },
                wgpu::ColorWrites::ALL,
            ),
            Target::Area => (
                COVERAGE,
                wgpu::BlendState { color: add, alpha: max },
                wgpu::ColorWrites::RED,
            ),
            Target::Line => (
                COVERAGE,
                wgpu::BlendState { color: add, alpha: max },
                wgpu::ColorWrites::ALPHA,
            ),
        };
        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some(vertex),
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
                entry_point: Some(fragment),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(blend),
                    write_mask,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipes.draw.push(DrawPipe {
            pipeline,
            layout,
            target,
        });
        DrawId(u32::try_from(self.pipes.draw.len().saturating_sub(1)).unwrap_or(0))
    }
}

/// A kind with its types forgotten, as the runtime holds it.
trait Erased: Send {
    fn state(&self) -> Box<dyn Any + Send>;
    #[expect(clippy::too_many_arguments, reason = "one chunk brought up to date")]
    fn run(
        &self,
        up: &mut ChunkUpload<'_>,
        state: &mut dyn Any,
        params: &[u32],
        ops: &[DataOp],
        frame: &FrameInfo,
        compute: &mut ChunkCompute<'_>,
        draw: &mut ChunkDraw<'_>,
    );
}

impl<K: ChunkKind> Erased for K {
    fn state(&self) -> Box<dyn Any + Send> {
        Box::new(K::State::default())
    }

    fn run(
        &self,
        up: &mut ChunkUpload<'_>,
        state: &mut dyn Any,
        params: &[u32],
        ops: &[DataOp],
        frame: &FrameInfo,
        compute: &mut ChunkCompute<'_>,
        draw: &mut ChunkDraw<'_>,
    ) {
        let Some(state) = state.downcast_mut::<K::State>() else {
            return;
        };
        let Ok(params) = bytemuck::try_pod_read_unaligned::<K::Params>(bytemuck::cast_slice(params)) else {
            return;
        };
        self.update(up, state, &params, ops);
        self.compute(compute, state, &params, frame);
        self.draw(draw, state, &params, frame);
    }
}

type Maker = fn(&mut PipelineBuilder<'_>) -> (KindId, Box<dyn Erased>);

/// Kinds registered beyond `gui`'s own, for every renderer made after.
static REGISTERED: Mutex<Vec<Maker>> = Mutex::new(Vec::new());

fn make<K: ChunkKind>(ctx: &mut PipelineBuilder<'_>) -> (KindId, Box<dyn Erased>) {
    (KindId::of::<K>(), Box::new(K::pipelines(ctx)))
}

/// Register kind `K`: every renderer made from now on builds its pipelines.
/// Register before [`crate::run`].
pub fn register<K: ChunkKind>() {
    let mut r = REGISTERED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let m: Maker = make::<K>;
    if !r.iter().any(|x| std::ptr::fn_addr_eq(*x, m)) {
        r.push(m);
    }
}

/// What a kind may do to bring a chunk up to date: buffers and textures
/// (each at most a page), writes into them, its sources as uploaded.
pub struct ChunkUpload<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    store: &'a mut Store,
    sources: &'a [Source],
    pages: &'a [Vec<ResidentPage>],
    owned: &'a mut Owned,
    frame: FrameInfo,
}

#[derive(Default)]
struct Owned {
    buffers: Vec<BufferId>,
    textures: Vec<TextureId>,
}

impl ChunkUpload<'_> {
    /// Where the chunk draws this frame.
    #[must_use]
    pub fn frame(&self) -> FrameInfo {
        self.frame
    }

    /// A buffer of at least `bytes` in `slot` (at most [`super::PAGE_BYTES`];
    /// `None` past it): the one there when large enough, else a new one
    /// whose contents are not kept. Storage, uniform and indirect alike.
    pub fn buffer(&mut self, slot: &mut Option<BufferId>, bytes: u64) -> Option<BufferId> {
        let usage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::UNIFORM
            | wgpu::BufferUsages::INDIRECT
            | wgpu::BufferUsages::COPY_SRC;
        let had = *slot;
        let id = self.store.ensure_buffer(self.device, slot, bytes, usage)?;
        if had != Some(id) {
            self.owned.buffers.push(id);
        }
        Some(id)
    }

    /// Write `data` at `offset` of `buffer`.
    pub fn write(&mut self, buffer: BufferId, offset: u64, data: &[u8]) {
        self.store.write(self.device, self.queue, buffer, offset, data);
    }

    /// A texture of `size` and `format` in `slot`.
    pub fn texture(
        &mut self,
        slot: &mut Option<TextureId>,
        size: (u32, u32),
        format: wgpu::TextureFormat,
    ) -> Option<TextureId> {
        let had = *slot;
        let id = self
            .store
            .ensure_texture(self.device, slot, size, format, wgpu::TextureUsages::empty())?;
        if had != Some(id) {
            self.owned.textures.push(id);
        }
        Some(id)
    }

    /// Write a whole texture, `bytes_per_row` a row.
    pub fn write_texture(&mut self, texture: TextureId, data: &[u8], bytes_per_row: u32) {
        self.store.write_texture(self.queue, texture, data, bytes_per_row);
    }

    /// Source `i` when it is a series: its snapshot and its pages on the GPU.
    #[must_use]
    pub fn series(&self, i: usize) -> Option<SeriesPages<'_>> {
        match self.sources.get(i)? {
            Source::Series(s) => Some(SeriesPages {
                snapshot: s,
                pages: self.pages.get(i).map_or(&[], Vec::as_slice),
            }),
            Source::Grid(_) | Source::Bytes(_) => None,
        }
    }

    /// Source `i` when it is a grid.
    #[must_use]
    pub fn grid(&self, i: usize) -> Option<&GridSnapshot> {
        match self.sources.get(i)? {
            Source::Grid(g) => Some(g),
            Source::Series(_) | Source::Bytes(_) => None,
        }
    }

    /// Source `i` when it is bytes.
    #[must_use]
    pub fn bytes(&self, i: usize) -> Option<&[u8]> {
        match self.sources.get(i)? {
            Source::Bytes(b) => Some(b.data()),
            Source::Series(_) | Source::Grid(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
enum ComputeCmd {
    Dispatch {
        pipe: ComputeId,
        binds: Vec<Bind>,
        groups: [u32; 3],
    },
    Indirect {
        pipe: ComputeId,
        binds: Vec<Bind>,
        args: BufferId,
        offset: u64,
    },
    /// Read `size` bytes of a buffer from `offset` back, after the pass.
    Read {
        buffer: BufferId,
        offset: u64,
        size: u64,
        tag: u32,
    },
}

/// A chunk's compute passes: dispatches of its kind's pipelines on what
/// the store issued, run in order before the frame's passes.
pub struct ChunkCompute<'a> {
    cmds: &'a mut Vec<ComputeCmd>,
    /// Whether its item takes readbacks.
    reads: bool,
}

impl ChunkCompute<'_> {
    /// Read `size` bytes of `buffer` from `offset` (a multiple of four)
    /// back to the UI thread once the chunk's dispatches ran, tagged `tag`:
    /// they arrive through the item's [`super::Readback`] a frame or two
    /// later, never waited for. Nothing is read when the item takes no
    /// readbacks ([`super::DataItem::reading_back`]).
    pub fn read_back(&mut self, buffer: BufferId, offset: u64, size: u64, tag: u32) {
        if self.reads && size > 0 {
            self.cmds.push(ComputeCmd::Read {
                buffer,
                offset,
                size,
                tag,
            });
        }
    }

    pub fn dispatch(&mut self, pipe: ComputeId, binds: &[Bind], groups: [u32; 3]) {
        if groups.iter().all(|g| *g > 0) {
            self.cmds.push(ComputeCmd::Dispatch {
                pipe,
                binds: binds.to_vec(),
                groups,
            });
        }
    }

    /// A dispatch whose group counts a buffer holds (three `u32`s at `offset`).
    pub fn dispatch_indirect(&mut self, pipe: ComputeId, binds: &[Bind], args: BufferId, offset: u64) {
        self.cmds.push(ComputeCmd::Indirect {
            pipe,
            binds: binds.to_vec(),
            args,
            offset,
        });
    }
}

#[derive(Clone, Debug)]
enum DrawCmd {
    Draw {
        pipe: DrawId,
        binds: Vec<Bind>,
        vertices: Range<u32>,
        instances: Range<u32>,
    },
    Indirect {
        pipe: DrawId,
        binds: Vec<Bind>,
        args: BufferId,
        offset: u64,
    },
    Composite,
}

/// A chunk's draws: into its coverage texture (pipelines whose target is
/// [`Target::Area`] or [`Target::Line`]), and in the main pass where it
/// stands ([`Target::Main`], and [`Self::composite`]).
pub struct ChunkDraw<'a> {
    pipes: &'a Pipes,
    coverage: &'a mut Vec<DrawCmd>,
    main: &'a mut Vec<DrawCmd>,
    colors: &'a mut [[f32; 4]; 3],
}

impl ChunkDraw<'_> {
    fn target(&self, pipe: DrawId) -> Option<Target> {
        self.pipes.draw.get(usize::try_from(pipe.0).ok()?).map(|p| p.target)
    }

    fn list(&mut self, pipe: DrawId) -> Option<&mut Vec<DrawCmd>> {
        match self.target(pipe)? {
            Target::Main => Some(self.main),
            Target::Area | Target::Line => Some(self.coverage),
        }
    }

    /// `instances` of `vertices` (a triangle strip), where the pipeline draws.
    pub fn draw(&mut self, pipe: DrawId, binds: &[Bind], vertices: Range<u32>, instances: Range<u32>) {
        if instances.is_empty() || vertices.is_empty() {
            return;
        }
        if let Some(l) = self.list(pipe) {
            l.push(DrawCmd::Draw {
                pipe,
                binds: binds.to_vec(),
                vertices,
                instances,
            });
        }
    }

    /// A draw whose counts a buffer holds (four `u32`s at `offset`), wgpu's
    /// indirect validation on.
    pub fn draw_indirect(&mut self, pipe: DrawId, binds: &[Bind], args: BufferId, offset: u64) {
        if let Some(l) = self.list(pipe) {
            l.push(DrawCmd::Indirect {
                pipe,
                binds: binds.to_vec(),
                args,
                offset,
            });
        }
    }

    /// The coverage texture onto the main pass: areas in a colour that
    /// goes from `area_top` to `area_bottom` down the data's bounds, lines
    /// in `line`, over them.
    pub fn composite(&mut self, area_top: Color, area_bottom: Color, line: Color) {
        self.colors(area_top, area_bottom, line);
        self.main.push(DrawCmd::Composite);
    }

    /// The colours the chunk's main draws read (`chunk_item` in
    /// `chunk.wgsl`): an area's at the top and the bottom, a line's.
    pub fn colors(&mut self, area_top: Color, area_bottom: Color, line: Color) {
        let c = |c: Color| [c.r, c.g, c.b, c.a];
        *self.colors = [c(area_top), c(area_bottom), c(line)];
    }
}

/// A chunk's GPU state across frames.
struct Slot {
    kind: KindId,
    state: Box<dyn Any + Send>,
    /// What its pixels were last made from.
    sig: u64,
    stamps: Vec<(u64, u64, u64, u64, u64)>,
    coverage_tex: Option<TextureId>,
    coverage: Vec<DrawCmd>,
    main: Vec<DrawCmd>,
    colors: [[f32; 4]; 3],
    owned: Owned,
    frame: FrameInfo,
    seen: u64,
}

/// One chunk drawn this frame: its main draws, bind groups resolved.
struct Prepared {
    key: u64,
    uniform: u32,
    cmds: Vec<(DrawCmd, Option<wgpu::BindGroup>)>,
}

/// A coverage pass to draw before the frame.
struct CoveragePass {
    key: u64,
    uniform: u32,
    globals: u32,
}

type GroupKey = (u8, u32, Vec<(u64, u64, u64, u64)>);

pub(crate) struct Runtime {
    kinds: foldhash::HashMap<KindId, Box<dyn Erased>>,
    pipes: Pipes,
    frame_layout: wgpu::BindGroupLayout,
    composite: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    pub(crate) store: Store,
    slots: foldhash::HashMap<u64, Slot>,
    prepared: foldhash::HashMap<u64, u32>,
    draws: Vec<Prepared>,
    computes: Vec<(u64, Vec<ComputeCmd>, Option<super::ReadbackTo>)>,
    /// Reads of user chunks on their way back ([`super::readback`]).
    belt: super::readback::Belt,
    coverages: Vec<CoveragePass>,
    /// This frame's uniform blocks, `stride` bytes apart.
    uniforms: Vec<u8>,
    uniform_buf: Option<wgpu::Buffer>,
    cov_globals: Vec<u8>,
    cov_globals_buf: Option<wgpu::Buffer>,
    stride: usize,
    groups: foldhash::HashMap<GroupKey, (wgpu::BindGroup, u64)>,
    main_group: Option<((u64, u64, u64), wgpu::BindGroup)>,
    cov_group: Option<((u64, u64), wgpu::BindGroup)>,
    buffers_made: u64,
    frame: u64,
    /// Every pass's timestamps ([`super::timing`]).
    pub(crate) timing: Option<Timing>,
    /// Checking each frame's chunks against ones built cold
    /// (`GUI_COLD_CHECK`): the scene's chunks drawn this frame, and the
    /// runtime they are built cold in.
    cold_check: bool,
    noted: Vec<(std::sync::Arc<DataItem>, f32, (f32, f32))>,
    cold: Option<Box<Runtime>>,
    /// Chunks checked against cold ones so far.
    pub(crate) checked: usize,
    format: wgpu::TextureFormat,
    alignment: u32,
    /// Chunks brought up to date in the last frame.
    pub(crate) updated: usize,
}

impl Runtime {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        uniform_alignment: u32,
    ) -> Self {
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vf,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(16),
            },
            count: None,
        };
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gui chunk frame"),
            entries: &[
                uniform(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: vf,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(u64::try_from(ITEM_BYTES).unwrap_or(96)),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: vf,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let mut own = Pipes::default();
        let composite_id = PipelineBuilder {
            device,
            format,
            frame: &frame_layout,
            pipes: &mut own,
        }
        .draw(
            "gui chunk composite",
            include_str!("composite.wgsl"),
            "vs_composite",
            "fs_composite",
            &[BindKind::Texture],
            Target::Main,
        );
        let DrawPipe {
            pipeline: composite,
            layout: composite_layout,
            ..
        } = own.draw.swap_remove(usize::try_from(composite_id.0).unwrap_or(0));
        let composite_layout = composite_layout.unwrap_or_else(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gui chunk composite"),
                entries: &layout_entries(&[BindKind::Texture], vf),
            })
        });
        let mut pipes = Pipes::default();
        let mut builder = PipelineBuilder {
            device,
            format,
            frame: &frame_layout,
            pipes: &mut pipes,
        };
        let mut kinds: foldhash::HashMap<KindId, Box<dyn Erased>> = foldhash::HashMap::default();
        let own_kinds: [Maker; 7] = [
            make::<crate::canvas::tile::Tiles>,
            make::<crate::canvas::boxes::Boxes>,
            make::<super::kinds::Lines>,
            make::<super::kinds::Bars>,
            make::<super::kinds::Candles>,
            make::<super::kinds::Scatter>,
            make::<super::kinds::Heatmap>,
        ];
        let registered: Vec<Maker> = REGISTERED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for m in own_kinds.into_iter().chain(registered) {
            let (id, k) = m(&mut builder);
            kinds.insert(id, k);
        }
        let timing = Timing::new(device, queue);
        let cold_check = cfg!(debug_assertions) && std::env::var_os("GUI_COLD_CHECK").is_some();
        Self {
            kinds,
            pipes,
            frame_layout,
            composite,
            composite_layout,
            store: Store::new(),
            slots: foldhash::HashMap::default(),
            prepared: foldhash::HashMap::default(),
            draws: Vec::new(),
            computes: Vec::new(),
            belt: super::readback::Belt::default(),
            coverages: Vec::new(),
            uniforms: Vec::new(),
            uniform_buf: None,
            cov_globals: Vec::new(),
            cov_globals_buf: None,
            stride: usize::try_from(uniform_alignment).unwrap_or(256).max(ITEM_BYTES),
            groups: foldhash::HashMap::default(),
            main_group: None,
            cov_group: None,
            buffers_made: 0,
            frame: 0,
            timing,
            cold_check,
            noted: Vec::new(),
            cold: None,
            checked: 0,
            format,
            alignment: uniform_alignment,
            updated: 0,
        }
    }

    /// A new frame's draws (its passes are planned anew).
    pub(crate) fn begin(&mut self) {
        self.prepared.clear();
        self.draws.clear();
        self.uniforms.clear();
        self.noted.clear();
    }

    /// Check every frame's chunks against ones built cold from now on (or not).
    pub(crate) fn set_cold_check(&mut self, on: bool) {
        self.cold_check = on;
    }

    /// Scene chunk `item` is drawn this frame at `scale` and `off`: checked
    /// cold after it, when checking.
    pub(crate) fn note(&mut self, item: &std::sync::Arc<DataItem>, scale: f32, off: (f32, f32)) {
        if self.cold_check {
            self.noted.push((std::sync::Arc::clone(item), scale, off));
        }
    }

    /// Every scene chunk drawn this frame, drawn alone from what this
    /// runtime kept and again by a runtime that never saw it: the pairs of
    /// textures with their sizes, to be compared (ADR-0023: incremental
    /// equals cold).
    pub(crate) fn cold_pairs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Vec<(u64, wgpu::Texture, wgpu::Texture, (u32, u32))> {
        let noted = std::mem::take(&mut self.noted);
        if noted.is_empty() {
            return Vec::new();
        }
        let (format, alignment) = (self.format, self.alignment);
        let mut cold = self
            .cold
            .take()
            .unwrap_or_else(|| Box::new(Self::new(device, queue, format, alignment)));
        let mut out = Vec::new();
        for (item, scale, off) in &noted {
            cold.forget();
            if let (Some((warm, size)), Some((built, _))) = (
                self.alone(device, queue, item, *scale, *off),
                cold.alone(device, queue, item, *scale, *off),
            ) {
                out.push((item.key, warm, built, size));
                self.checked = self.checked.saturating_add(1);
            }
        }
        self.cold = Some(cold);
        out
    }

    /// Forget every chunk and every byte uploaded: what comes next is built cold.
    fn forget(&mut self) {
        self.slots.clear();
        self.store = Store::new();
        self.groups.clear();
        self.prepared.clear();
        self.draws.clear();
        self.computes.clear();
        self.coverages.clear();
        self.cov_globals.clear();
        self.uniforms.clear();
        self.main_group = None;
        self.cov_group = None;
    }

    /// Chunk `item` drawn alone into a texture of its own size, from the
    /// state this runtime keeps (brought up to date first if it is not).
    fn alone(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        item: &DataItem,
        scale: f32,
        off: (f32, f32),
    ) -> Option<(wgpu::Texture, (u32, u32))> {
        self.begin();
        let d = self.prepare(device, queue, item, scale, off, 0, 1.0)?;
        let size = self.slots.get(&item.key)?.frame.size;
        let r = item.region;
        let origin = (r.x.mul_add(scale, off.0).floor(), r.y.mul_add(scale, off.1).floor());
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gui cold check globals"),
            size: 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let g = [
            crate::convert::u32_to_f32(size.0),
            crate::convert::u32_to_f32(size.1),
            origin.0,
            origin.1,
        ];
        queue.write_buffer(&globals, 0, bytemuck::cast_slice(&g));
        let clips = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gui cold check clips"),
            size: 64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui cold check"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gui cold check"),
        });
        // A generation no frame's globals or clips have: the frame bind
        // group is made for these and again for the next frame.
        self.encode(device, queue, &mut enc, (&globals, u64::MAX), (&clips, u64::MAX));
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gui cold check"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.replay(&mut rp, d, 0);
        }
        self.finish(&mut enc);
        let uploads = self.store.take_uploads();
        queue.submit(uploads.into_iter().chain([enc.finish()]));
        self.after_submit(queue);
        self.main_group = None;
        Some((texture, size))
    }

    fn uniform(
        &mut self,
        origin: [f32; 2],
        frame: &FrameInfo,
        clip: u32,
        alpha: f32,
        colors: [[f32; 4]; 3],
        coverage: bool,
    ) -> u32 {
        let at = self.uniforms.len();
        let mut words: Vec<u32> = Vec::with_capacity(ITEM_BYTES / 4);
        words.extend(origin.map(f32::to_bits));
        words.push(crate::convert::u32_to_f32(frame.size.0).to_bits());
        words.push(crate::convert::u32_to_f32(frame.size.1).to_bits());
        words.extend(frame.bounds.map(f32::to_bits));
        words.push(clip);
        words.push(alpha.to_bits());
        words.push(frame.scale.to_bits());
        words.push(u32::from(coverage));
        for c in colors {
            words.extend(c.map(f32::to_bits));
        }
        self.uniforms.extend_from_slice(bytemuck::cast_slice(&words));
        self.uniforms.resize(at.saturating_add(self.stride), 0);
        u32::try_from(at).unwrap_or(0)
    }

    /// Chunk `item` drawn with its paint space at `off` (physical window
    /// pixels), under device clip `clip`, faded by `alpha`: brought up to
    /// date when it changed; the draw to replay where it stands.
    #[expect(clippy::too_many_arguments, reason = "a chunk and where it is drawn")]
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        item: &DataItem,
        scale: f32,
        off: (f32, f32),
        clip: u32,
        alpha: f32,
    ) -> Option<u32> {
        if let Some(d) = self.prepared.get(&item.key) {
            return Some(*d);
        }
        let r = item.region;
        let (x0, y0) = (r.x.mul_add(scale, off.0), r.y.mul_add(scale, off.1));
        let (x1, y1) = (r.right().mul_add(scale, off.0), r.bottom().mul_add(scale, off.1));
        if !(x1 > x0 && y1 > y0) || !(x0.is_finite() && y0.is_finite()) {
            return None;
        }
        let (ox, oy) = (x0.floor(), y0.floor());
        let max = device.limits().max_texture_dimension_2d;
        let size = (
            crate::convert::f32_to_u32((x1 - ox).ceil()).clamp(1, max),
            crate::convert::f32_to_u32((y1 - oy).ceil()).clamp(1, max),
        );
        let b = item.bounds;
        let frame = FrameInfo {
            size,
            bounds: [
                b.x.mul_add(scale, off.0) - ox,
                b.y.mul_add(scale, off.1) - oy,
                b.width * scale,
                b.height * scale,
            ],
            scale,
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        item.kind.hash(&mut h);
        item.params.hash(&mut h);
        item.readback.as_ref().map(super::ReadbackTo::id).hash(&mut h);
        for s in &item.sources {
            s.stamp().hash(&mut h);
        }
        size.hash(&mut h);
        frame.bounds.map(f32::to_bits).hash(&mut h);
        let sig = h.finish();
        let frame_no = self.frame;
        let fresh = !self.slots.get(&item.key).is_some_and(|s| s.kind == item.kind);
        if fresh {
            let state = self.kinds.get(&item.kind)?.state();
            if let Some(old) = self.slots.remove(&item.key) {
                self.release(old);
            }
            self.slots.insert(
                item.key,
                Slot {
                    kind: item.kind,
                    state,
                    sig: 0,
                    stamps: Vec::new(),
                    coverage_tex: None,
                    coverage: Vec::new(),
                    main: Vec::new(),
                    colors: [[0.0; 4]; 3],
                    owned: Owned::default(),
                    frame,
                    seen: frame_no,
                },
            );
        }
        let mut slot = self.slots.remove(&item.key)?;
        slot.seen = frame_no;
        if slot.sig != sig || fresh {
            self.bring_up(device, queue, item, &mut slot, &frame);
            slot.sig = sig;
        }
        let colors = slot.colors;
        let uniform = self.uniform([ox, oy], &frame, clip, alpha, colors, false);
        let cmds = slot.main.iter().cloned().map(|c| (c, None)).collect();
        self.slots.insert(item.key, slot);
        let d = u32::try_from(self.draws.len()).ok()?;
        self.draws.push(Prepared {
            key: item.key,
            uniform,
            cmds,
        });
        self.prepared.insert(item.key, d);
        Some(d)
    }

    /// Upload, compute and draw `item` anew.
    fn bring_up(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        item: &DataItem,
        slot: &mut Slot,
        frame: &FrameInfo,
    ) {
        self.updated = self.updated.saturating_add(1);
        let mut pages = Vec::with_capacity(item.sources.len());
        let mut ops = Vec::new();
        let mut stamps = Vec::with_capacity(item.sources.len());
        for (i, s) in item.sources.iter().enumerate() {
            match s {
                Source::Series(snap) => {
                    pages.push(self.store.sync_series(device, queue, snap));
                    let now = (
                        snap.id(),
                        snap.generation(),
                        snap.start(),
                        snap.start().saturating_add(u64::from(snap.len())),
                        snap.version(),
                    );
                    match slot.stamps.get(i) {
                        Some(was) if was.0 == now.0 && was.1 == now.1 => {
                            if now.2 > was.2 {
                                ops.push(DataOp::Dropped {
                                    source: i,
                                    until: now.2,
                                });
                            }
                            if now.3 > was.3 {
                                ops.push(DataOp::Appended {
                                    source: i,
                                    from: was.3,
                                    to: now.3,
                                });
                            }
                        }
                        _ => ops.push(DataOp::Replaced { source: i }),
                    }
                    stamps.push(now);
                }
                Source::Grid(_) | Source::Bytes(_) => {
                    pages.push(Vec::new());
                    let (id, version) = s.stamp();
                    let now = (id, 0, 0, 0, version);
                    if slot.stamps.get(i) != Some(&now) {
                        ops.push(DataOp::Replaced { source: i });
                    }
                    stamps.push(now);
                }
            }
        }
        slot.stamps = stamps;
        let Some(kind) = self.kinds.get(&item.kind) else { return };
        let mut compute = Vec::new();
        let mut coverage = Vec::new();
        let mut main = Vec::new();
        let mut colors = [[0.0; 4]; 3];
        {
            let mut up = ChunkUpload {
                device,
                queue,
                store: &mut self.store,
                sources: &item.sources,
                pages: &pages,
                owned: &mut slot.owned,
                frame: *frame,
            };
            let mut c = ChunkCompute {
                cmds: &mut compute,
                reads: item.readback.is_some(),
            };
            let mut d = ChunkDraw {
                pipes: &self.pipes,
                coverage: &mut coverage,
                main: &mut main,
                colors: &mut colors,
            };
            kind.run(&mut up, slot.state.as_mut(), &item.params, &ops, frame, &mut c, &mut d);
        }
        if !coverage.is_empty() {
            let had = slot.coverage_tex;
            let t = self.store.ensure_texture(
                device,
                &mut slot.coverage_tex,
                frame.size,
                COVERAGE,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            );
            if had != t
                && let Some(t) = t
            {
                slot.owned.textures.push(t);
            }
        }
        slot.colors = colors;
        slot.frame = *frame;
        slot.main = main;
        if !compute.is_empty() {
            self.computes.push((item.key, compute, item.readback.clone()));
        }
        if !coverage.is_empty() {
            let uniform = self.uniform([0.0, 0.0], frame, 0, 1.0, colors, true);
            let globals = u32::try_from(self.cov_globals.len()).unwrap_or(0);
            let mut block = vec![0_u8; self.stride];
            let size = [
                crate::convert::u32_to_f32(frame.size.0),
                crate::convert::u32_to_f32(frame.size.1),
                0.0,
                0.0,
            ];
            for (d, s) in block.iter_mut().zip(bytemuck::cast_slice::<f32, u8>(&size)) {
                *d = *s;
            }
            self.cov_globals.extend(block);
            self.coverages.push(CoveragePass {
                key: item.key,
                uniform,
                globals,
            });
        }
        slot.coverage = coverage;
    }

    fn release(&mut self, slot: Slot) {
        for b in slot.owned.buffers {
            self.store.release_buffer(b);
        }
        for t in slot.owned.textures {
            self.store.release_texture(t);
        }
    }

    /// The bind group of `binds` for layout `layout` (pipeline `tag`, `id`).
    fn group(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        tag: (u8, u32),
        binds: &[Bind],
        coverage: Option<TextureId>,
    ) -> Option<wgpu::BindGroup> {
        let mut key_parts = Vec::with_capacity(binds.len());
        for b in binds {
            key_parts.push(match *b {
                Bind::Buffer(id) => (0, u64::from(id.0), self.store.buffer(id)?.generation, 0),
                Bind::Range(id, o, s) => (1, u64::from(id.0), self.store.buffer(id)?.generation, o ^ (s << 40)),
                Bind::Page(seq) => (2, seq, self.store.page(seq)?.generation, 0),
                Bind::Texture(id) => (3, u64::from(id.0), self.store.texture(id)?.generation, 0),
                Bind::Coverage => {
                    let id = coverage?;
                    (4, u64::from(id.0), self.store.texture(id)?.generation, 0)
                }
            });
        }
        let key: GroupKey = (tag.0, tag.1, key_parts);
        let frame = self.frame;
        if let Some((g, used)) = self.groups.get_mut(&key) {
            *used = frame;
            return Some(g.clone());
        }
        let mut entries = Vec::with_capacity(binds.len());
        for (i, b) in binds.iter().enumerate() {
            let binding = u32::try_from(i).ok()?;
            let resource = match *b {
                Bind::Buffer(id) => self.store.buffer(id)?.buffer.as_entire_binding(),
                Bind::Range(id, offset, size) => wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.store.buffer(id)?.buffer,
                    offset,
                    size: wgpu::BufferSize::new(size),
                }),
                Bind::Page(seq) => self.store.page(seq)?.buffer.as_entire_binding(),
                Bind::Texture(id) => wgpu::BindingResource::TextureView(&self.store.texture(id)?.view),
                Bind::Coverage => wgpu::BindingResource::TextureView(&self.store.texture(coverage?)?.view),
            };
            entries.push(wgpu::BindGroupEntry { binding, resource });
        }
        let g = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gui chunk"),
            layout,
            entries: &entries,
        });
        self.buffers_made = self.buffers_made.saturating_add(1);
        self.groups.insert(key, (g.clone(), frame));
        Some(g)
    }

    fn draw_group(
        &mut self,
        device: &wgpu::Device,
        cmd: &DrawCmd,
        coverage: Option<TextureId>,
    ) -> Option<wgpu::BindGroup> {
        match cmd {
            DrawCmd::Draw { pipe, binds, .. } | DrawCmd::Indirect { pipe, binds, .. } => {
                let layout = self.pipes.draw.get(usize::try_from(pipe.0).ok()?)?.layout.clone()?;
                self.group(device, &layout, (1, pipe.0), binds, coverage)
            }
            DrawCmd::Composite => {
                let layout = self.composite_layout.clone();
                self.group(device, &layout, (2, 0), &[Bind::Coverage], coverage)
            }
        }
    }

    /// The bind groups this frame's draws need; before [`Self::encode`].
    fn resolve(&mut self, device: &wgpu::Device) {
        let mut draws = std::mem::take(&mut self.draws);
        for d in &mut draws {
            let coverage = self.slots.get(&d.key).and_then(|s| s.coverage_tex);
            for (cmd, group) in &mut d.cmds {
                *group = self.draw_group(device, cmd, coverage);
            }
        }
        self.draws = draws;
    }

    /// Run this frame's compute and coverage passes into `encoder`, and
    /// ready its main draws; `globals` is the frame's pass globals and
    /// `clips` its clip table (each with its generation).
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        globals: (&wgpu::Buffer, u64),
        clips: (&wgpu::Buffer, u64),
    ) {
        if let Some(t) = self.timing.as_mut() {
            t.read(device);
        }
        self.belt.deliver(device, false);
        // The uniform blocks.
        let need = u64::try_from(self.uniforms.len()).unwrap_or(0).max(256);
        if self.uniform_buf.as_ref().is_none_or(|b| b.size() < need) {
            self.uniform_buf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gui chunk items"),
                size: need.next_power_of_two(),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.main_group = None;
            self.cov_group = None;
        }
        let cov_need = u64::try_from(self.cov_globals.len()).unwrap_or(0).max(256);
        if self.cov_globals_buf.as_ref().is_none_or(|b| b.size() < cov_need) {
            self.cov_globals_buf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gui chunk coverage globals"),
                size: cov_need.next_power_of_two(),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.cov_group = None;
        }
        let (Some(ubuf), Some(cbuf)) = (self.uniform_buf.clone(), self.cov_globals_buf.clone()) else {
            return;
        };
        // Through the staging belt, not the queue's own staging (a new
        // buffer a write): submitted ahead of the frame's commands.
        let uniforms = std::mem::take(&mut self.uniforms);
        self.store.stage(device, queue, &ubuf, 0, &uniforms);
        self.uniforms = uniforms;
        let cov = std::mem::take(&mut self.cov_globals);
        self.store.stage(device, queue, &cbuf, 0, &cov);
        self.cov_globals = cov;
        fn item_binding(b: &wgpu::Buffer) -> wgpu::BindingResource<'_> {
            wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: b,
                offset: 0,
                size: wgpu::BufferSize::new(u64::try_from(ITEM_BYTES).unwrap_or(96)),
            })
        }
        let gkey = (globals.1, clips.1, ubuf.size());
        if self.main_group.as_ref().is_none_or(|(k, _)| *k != gkey) {
            let g = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gui chunk frame"),
                layout: &self.frame_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: globals.0,
                            offset: 0,
                            size: wgpu::BufferSize::new(16),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: item_binding(&ubuf),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: clips.0.as_entire_binding(),
                    },
                ],
            });
            self.main_group = Some((gkey, g));
        }
        let ckey = (clips.1, ubuf.size() ^ (cbuf.size() << 32));
        if self.cov_group.as_ref().is_none_or(|(k, _)| *k != ckey) {
            let g = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gui chunk coverage frame"),
                layout: &self.frame_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &cbuf,
                            offset: 0,
                            size: wgpu::BufferSize::new(16),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: item_binding(&ubuf),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: clips.0.as_entire_binding(),
                    },
                ],
            });
            self.cov_group = Some((ckey, g));
        }
        self.resolve(device);
        // Compute, chunk by chunk in the order drawn.
        let computes = std::mem::take(&mut self.computes);
        for (_, cmds, to) in &computes {
            let mut groups = Vec::with_capacity(cmds.len());
            for cmd in cmds {
                let (ComputeCmd::Dispatch { pipe, binds, .. } | ComputeCmd::Indirect { pipe, binds, .. }) = cmd else {
                    groups.push(None);
                    continue;
                };
                let layout = self
                    .pipes
                    .compute
                    .get(usize::try_from(pipe.0).unwrap_or(usize::MAX))
                    .map(|p| p.layout.clone());
                groups.push(layout.and_then(|l| self.group(device, &l, (0, pipe.0), binds, None)));
            }
            let stamp = self.stamp(Stamped::Compute);
            let stamp = stamp.and_then(|at| Some((self.query_set()?, at)));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gui chunk compute"),
                timestamp_writes: stamp.map(|(set, at)| wgpu::ComputePassTimestampWrites {
                    query_set: set,
                    beginning_of_pass_write_index: Some(at),
                    end_of_pass_write_index: Some(at.saturating_add(1)),
                }),
            });
            for (cmd, group) in cmds.iter().zip(&groups) {
                let Some(group) = group else { continue };
                match cmd {
                    ComputeCmd::Dispatch { pipe, groups, .. } => {
                        let Some(p) = self.pipes.compute.get(usize::try_from(pipe.0).unwrap_or(usize::MAX)) else {
                            continue;
                        };
                        pass.set_pipeline(&p.pipeline);
                        pass.set_bind_group(0, group, &[]);
                        pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
                    }
                    ComputeCmd::Indirect { pipe, args, offset, .. } => {
                        let (Some(p), Some(b)) = (
                            self.pipes.compute.get(usize::try_from(pipe.0).unwrap_or(usize::MAX)),
                            self.store.buffer(*args),
                        ) else {
                            continue;
                        };
                        pass.set_pipeline(&p.pipeline);
                        pass.set_bind_group(0, group, &[]);
                        pass.dispatch_workgroups_indirect(&b.buffer, *offset);
                    }
                    ComputeCmd::Read { .. } => {}
                }
            }
            drop(pass);
            // What the dispatches wrote, read back after them.
            if let Some(to) = to {
                for cmd in cmds {
                    if let ComputeCmd::Read {
                        buffer,
                        offset,
                        size,
                        tag,
                    } = cmd
                        && let Some(b) = self.store.buffer(*buffer)
                    {
                        self.belt.copy(device, encoder, &b.buffer, *offset, *size, *tag, to);
                    }
                }
            }
        }
        // Coverage, one pass a chunk into its own texture.
        let coverages = std::mem::take(&mut self.coverages);
        for c in &coverages {
            let Some(slot) = self.slots.get(&c.key) else { continue };
            let Some(tex) = slot.coverage_tex else { continue };
            let cmds = slot.coverage.clone();
            let mut groups = Vec::with_capacity(cmds.len());
            for cmd in &cmds {
                groups.push(self.draw_group(device, cmd, Some(tex)));
            }
            let at = self.stamp(Stamped::Coverage);
            let (Some(view), Some((_, frame))) =
                (self.store.texture(tex).map(|t| t.view.clone()), self.cov_group.as_ref())
            else {
                continue;
            };
            let stamp = at.and_then(|at| Some((self.query_set()?, at)));
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gui chunk coverage"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes: stamp.map(|(set, at)| wgpu::RenderPassTimestampWrites {
                    query_set: set,
                    beginning_of_pass_write_index: Some(at),
                    end_of_pass_write_index: Some(at.saturating_add(1)),
                }),
                ..Default::default()
            });
            rp.set_bind_group(0, frame, &[c.globals, c.uniform]);
            for (cmd, group) in cmds.iter().zip(&groups) {
                self.issue(&mut rp, cmd, group.as_ref());
            }
        }
        self.cov_globals.clear();
    }

    /// A pair of queries for a pass of kind `tag` ([`Timing::stamp`]).
    pub(crate) fn stamp(&mut self, tag: Stamped) -> Option<u32> {
        self.timing.as_mut()?.stamp(tag)
    }

    /// The set [`Self::stamp`]'s queries are in.
    pub(crate) fn query_set(&self) -> Option<&wgpu::QuerySet> {
        self.timing.as_ref()?.query_set()
    }

    /// After the frame's passes ([`Timing::finish`]).
    pub(crate) fn finish(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(t) = self.timing.as_mut() {
            t.finish(encoder);
        }
    }

    /// After the frame's submit ([`Timing::after_submit`]).
    pub(crate) fn after_submit(&mut self, queue: &wgpu::Queue) {
        if let Some(t) = self.timing.as_mut() {
            t.after_submit(queue);
        }
        self.belt.after_submit();
    }

    /// Whether reads of chunks are on their way back.
    pub(crate) fn reads_out(&self) -> bool {
        self.belt.busy()
    }

    /// Send the UI thread what came back of chunks' reads (`wait`: wait
    /// for the GPU first, as a test does); whether reads are still out.
    pub(crate) fn deliver_reads(&mut self, device: &wgpu::Device, wait: bool) -> bool {
        self.belt.deliver(device, wait);
        self.belt.busy()
    }

    /// The last frame's GPU time read back, and by kind of pass.
    pub(crate) fn gpu_times(&self) -> (Option<std::time::Duration>, Option<crate::scene::GpuPasses>) {
        self.timing.as_ref().map_or((None, None), |t| (t.frame, t.passes))
    }

    fn issue(&self, rp: &mut wgpu::RenderPass<'_>, cmd: &DrawCmd, group: Option<&wgpu::BindGroup>) {
        match cmd {
            DrawCmd::Draw {
                pipe,
                vertices,
                instances,
                ..
            } => {
                let Some(p) = self.pipes.draw.get(usize::try_from(pipe.0).unwrap_or(usize::MAX)) else {
                    return;
                };
                if p.layout.is_some() && group.is_none() {
                    return;
                }
                rp.set_pipeline(&p.pipeline);
                if let Some(g) = group {
                    rp.set_bind_group(1, g, &[]);
                }
                rp.draw(vertices.clone(), instances.clone());
            }
            DrawCmd::Indirect { pipe, args, offset, .. } => {
                let (Some(p), Some(b)) = (
                    self.pipes.draw.get(usize::try_from(pipe.0).unwrap_or(usize::MAX)),
                    self.store.buffer(*args),
                ) else {
                    return;
                };
                if p.layout.is_some() && group.is_none() {
                    return;
                }
                rp.set_pipeline(&p.pipeline);
                if let Some(g) = group {
                    rp.set_bind_group(1, g, &[]);
                }
                rp.draw_indirect(&b.buffer, *offset);
            }
            DrawCmd::Composite => {
                let Some(g) = group else { return };
                rp.set_pipeline(&self.composite);
                rp.set_bind_group(1, g, &[]);
                rp.draw(0..4, 0..1);
            }
        }
    }

    /// Draw `draw` where it stands in a pass whose globals are at `offset`.
    pub(crate) fn replay(&self, rp: &mut wgpu::RenderPass<'_>, draw: u32, offset: u32) {
        let (Some(d), Some((_, frame))) = (
            self.draws.get(usize::try_from(draw).unwrap_or(usize::MAX)),
            self.main_group.as_ref(),
        ) else {
            return;
        };
        rp.set_bind_group(0, frame, &[offset, d.uniform]);
        for (cmd, group) in &d.cmds {
            self.issue(rp, cmd, group.as_ref());
        }
    }

    /// The frame is done: let go of chunks and bind groups not drawn for a while.
    /// The state of the chunk drawn under `key`, as kind state `T`.
    #[cfg(test)]
    pub(crate) fn state_of<T: 'static>(&self, key: u64) -> Option<&T> {
        self.slots.get(&key)?.state.downcast_ref()
    }

    /// Buffer `id`'s bytes, read back (waits for the GPU).
    #[cfg(test)]
    pub(crate) fn read_back(&self, device: &wgpu::Device, queue: &wgpu::Queue, id: BufferId) -> Vec<u8> {
        let Some(b) = self.store.buffer(id) else {
            return Vec::new();
        };
        let size = b.buffer.size();
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gui chunk test read"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        enc.copy_buffer_to_buffer(&b.buffer, 0, &read, 0, size);
        queue.submit([enc.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        drop(device.poll(wgpu::PollType::wait_indefinitely()));
        read.slice(..)
            .get_mapped_range()
            .map(|v| v.to_vec())
            .unwrap_or_default()
    }

    pub(crate) fn end_frame(&mut self) {
        self.frame = self.frame.saturating_add(1);
        let frame = self.frame;
        let stale: Vec<u64> = self
            .slots
            .iter()
            .filter(|(_, s)| frame.saturating_sub(s.seen) >= SLOT_FRAMES)
            .map(|(k, _)| *k)
            .collect();
        for k in stale {
            if let Some(s) = self.slots.remove(&k) {
                self.release(s);
            }
        }
        self.groups
            .retain(|_, (_, used)| frame.saturating_sub(*used) < SLOT_FRAMES);
        self.store.end_frame();
        self.updated = 0;
    }

    /// Chunks it keeps, and the bytes their GPU data takes.
    pub(crate) fn held(&self) -> (usize, u64) {
        (self.slots.len(), self.store.bytes())
    }
}
