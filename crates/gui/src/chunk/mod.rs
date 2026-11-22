//! GPU data chunks (ADR-0018): content whose data lives on the GPU and is
//! reduced there, by compute, to what the screen can show: charts of
//! millions of points, heatmaps, user kinds.
//!
//! In a scene a data chunk is one [`crate::scene::Item::Data`]: a
//! [`DataItem`] naming its [`ChunkKind`], small fixed parameters (`Pod`) and
//! the data it reads ([`Source`]: a [`series::SeriesSnapshot`], a
//! [`GridSnapshot`], [`Bytes`] it lays out itself). It takes one place in its paint chunk's order and paints
//! under that chunk's transform, clip and effect, like every primitive
//! (ADR-0016), so rounded clips, scrolling, fades and damage apply
//! unchanged. A change of its data or its view records only its own chunk
//! (`NeedsChunk`: no layout of anything else, no other chunk), and its
//! damage is its region.
//!
//! On the render thread ([`crate::gpu`]) a kind's pipelines are built when
//! the renderer is made ([`ChunkKind::pipelines`]); each frame a chunk whose
//! data, view or size changed is brought up to date ([`ChunkKind::update`]:
//! the new data is uploaded, only the new data), reduced
//! ([`ChunkKind::compute`]) and drawn ([`ChunkKind::draw`]): into a coverage
//! texture of its own (lines and areas, so overlaps never darken) and in
//! the main pass, in order. A kind cannot open a pass or change a target:
//! [`ChunkCompute`] and [`ChunkDraw`] take only dispatches and draws on
//! buffers the [`ChunkUpload`] store issued, so painting order, clips and
//! damage stay the renderer's. Data buffers are pages of at most
//! [`PAGE_BYTES`].
//!
//! `gui`'s own marks ([`plot`]: lines, areas, bars, candles, scatter,
//! heatmaps) are kinds like any other.

pub mod plot;
mod readback;
pub(crate) mod runtime;
pub mod series;
pub(crate) mod store;
mod timing;

mod kinds;
#[cfg(test)]
mod tests;

use std::any::TypeId;
use std::sync::Arc;

pub use self::kinds::{
    BarParams, Bars, CURVE_MAX, CandleParams, Candles, Heatmap, HeatmapParams, LineCurve, LineDash, LineJoin,
    LineParams, Lines, MAX_COLUMNS, MarkView, Scatter, ScatterParams, Words,
};
pub use self::readback::{Readback, ReadbackTo, Readout, readback};
pub use self::runtime::{
    Bind, BindKind, ChunkCompute, ChunkDraw, ChunkUpload, ComputeId, DataOp, DrawId, FrameInfo, PipelineBuilder,
    SeriesPages, Target, register,
};
pub use self::store::{BufferId, TextureId};

use crate::geometry::Bounds;

/// The most bytes one GPU buffer of chunk data holds (wgpu's default
/// storage binding size); larger data is split into pages.
pub const PAGE_BYTES: u64 = 128 << 20;

/// Which [`ChunkKind`] draws an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KindId(TypeId);

impl KindId {
    #[must_use]
    pub fn of<K: ChunkKind>() -> Self {
        Self(TypeId::of::<K>())
    }
}

/// A grid of values (a heatmap's), row by row from the top, as one frame saw it.
#[derive(Clone, Debug)]
pub struct GridSnapshot(Arc<GridData>);

#[derive(Debug)]
pub struct GridData {
    pub id: u64,
    pub version: u64,
    pub width: u32,
    pub height: u32,
    pub values: Vec<f32>,
}

impl GridSnapshot {
    /// `values`, `width` a row, as a grid of its own.
    #[must_use]
    pub fn new(width: u32, height: u32, values: Vec<f32>) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(Arc::new(GridData {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            version: 0,
            width,
            height,
            values,
        }))
    }

    #[must_use]
    pub fn data(&self) -> &GridData {
        &self.0
    }
}

impl PartialEq for GridSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id && self.0.version == other.0.version
    }
}

/// Bytes a kind reads as it lays them out (instances it draws, say),
/// shared and never changed: a new value is new bytes.
#[derive(Clone, Debug)]
pub struct Bytes(Arc<(u64, Vec<u8>)>);

impl Bytes {
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(Arc::new((
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            data,
        )))
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.0.1
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        self.0.0 == other.0.0
    }
}

/// What a data chunk reads.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Series(series::SeriesSnapshot),
    Grid(GridSnapshot),
    Bytes(Bytes),
}

impl Source {
    /// Its identity and version: what a renderer compares to know it changed.
    #[must_use]
    pub fn stamp(&self) -> (u64, u64) {
        match self {
            Self::Series(s) => (s.id(), s.version()),
            Self::Grid(g) => (g.0.id, g.0.version),
            Self::Bytes(b) => (b.0.0, 0),
        }
    }
}

/// One data chunk as a scene item: its kind, parameters and data, where
/// its data maps (`bounds`) and what it may paint (`region`), both logical
/// and in its paint chunk's space. `key` is who it is across frames (its
/// node and place), what the renderer keeps its GPU state under.
#[derive(Clone, Debug, PartialEq)]
pub struct DataItem {
    pub kind: KindId,
    pub key: u64,
    pub bounds: Bounds,
    pub region: Bounds,
    /// The kind's [`ChunkKind::Params`], as words.
    pub params: Vec<u32>,
    pub sources: Vec<Source>,
    /// Where what its compute reads back goes ([`ChunkCompute::read_back`]).
    pub readback: Option<ReadbackTo>,
}

impl DataItem {
    /// An item of kind `K`.
    #[must_use]
    pub fn new<K: ChunkKind>(
        key: u64,
        bounds: Bounds,
        region: Bounds,
        params: &K::Params,
        sources: Vec<Source>,
    ) -> Self {
        Self {
            kind: KindId::of::<K>(),
            key,
            bounds,
            region,
            params: bytemuck::cast_slice::<u8, u32>(bytemuck::bytes_of(params)).to_vec(),
            sources,
            readback: None,
        }
    }

    /// Its compute's reads ([`ChunkCompute::read_back`]) sent to `to`.
    #[must_use]
    pub fn reading_back(mut self, to: ReadbackTo) -> Self {
        self.readback = Some(to);
        self
    }

    /// Its parameters as kind `K`'s.
    #[must_use]
    pub fn params_as<K: ChunkKind>(&self) -> Option<K::Params> {
        bytemuck::try_pod_read_unaligned(bytemuck::cast_slice(&self.params)).ok()
    }
}

/// What a node of [`crate::drawing`] draws, made from what its closure
/// read (signals: series, views) and laid into its box when painted. Equal
/// drawings record nothing again; a new one records the node's chunk alone.
pub trait Picture: std::fmt::Debug + PartialEq + Send + Sync + 'static {
    /// Its items, in paint space, for a node whose content box is `area`
    /// at `scale` device pixels per logical; `key` is the node's identity
    /// across frames, for [`DataItem::key`].
    fn draw(&self, area: Bounds, scale: f32, key: u64, out: &mut Vec<crate::scene::Item>);

    /// Shape the text it shows that is not shaped yet (a canvas' labels),
    /// before [`Self::draw`], with the paint's text system.
    fn shape(&self, _text: &mut crate::text::TextSystem) {}
}

/// A [`Picture`] of any type.
#[derive(Clone)]
pub struct Drawing(Arc<dyn Erased>);

impl Drawing {
    #[must_use]
    pub fn new(d: impl Picture) -> Self {
        Self(Arc::new(d))
    }

    pub(crate) fn draw(&self, area: Bounds, scale: f32, key: u64, out: &mut Vec<crate::scene::Item>) {
        self.0.draw_erased(area, scale, key, out);
    }

    pub(crate) fn shape(&self, text: &mut crate::text::TextSystem) {
        self.0.shape_erased(text);
    }

    /// It as a `D`, if it is one.
    #[must_use]
    pub fn get<D: Picture>(&self) -> Option<&D> {
        self.0.any().downcast_ref()
    }
}

impl PartialEq for Drawing {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.same(other.0.any())
    }
}

impl std::fmt::Debug for Drawing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.debug(f)
    }
}

trait Erased: Send + Sync {
    fn draw_erased(&self, area: Bounds, scale: f32, key: u64, out: &mut Vec<crate::scene::Item>);
    fn shape_erased(&self, text: &mut crate::text::TextSystem);
    fn same(&self, other: &dyn std::any::Any) -> bool;
    fn any(&self) -> &dyn std::any::Any;
    fn debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result;
}

impl<D: Picture> Erased for D {
    fn draw_erased(&self, area: Bounds, scale: f32, key: u64, out: &mut Vec<crate::scene::Item>) {
        self.draw(area, scale, key, out);
    }

    fn shape_erased(&self, text: &mut crate::text::TextSystem) {
        self.shape(text);
    }

    fn same(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<D>().is_some_and(|o| o == self)
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }

    fn debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

/// A kind of data chunk (ADR-0018). Its pipelines are built once, when a
/// renderer is made; its per-chunk GPU state is [`Self::State`], made when a
/// chunk of the kind is first drawn and dropped when it goes.
pub trait ChunkKind: Send + Sized + 'static {
    /// What a chunk asks of the kind: small and fixed (a colour, a width).
    type Params: bytemuck::Pod + PartialEq + Send + Sync;
    /// What the kind keeps for each chunk on the GPU (its buffers).
    type State: Default + Send + 'static;

    /// Build every pipeline the kind draws with.
    fn pipelines(ctx: &mut PipelineBuilder<'_>) -> Self;

    /// The chunk's data, view or size changed (`ops` says what of its
    /// data): bring its buffers up to date.
    fn update(&self, ctx: &mut ChunkUpload<'_>, state: &mut Self::State, params: &Self::Params, ops: &[DataOp]);

    /// Its compute passes, after [`Self::update`].
    fn compute(&self, pass: &mut ChunkCompute<'_>, state: &Self::State, params: &Self::Params, frame: &FrameInfo);

    /// Its draws: into its coverage texture, and in the main pass in order.
    fn draw(&self, pass: &mut ChunkDraw<'_>, state: &Self::State, params: &Self::Params, frame: &FrameInfo);
}
