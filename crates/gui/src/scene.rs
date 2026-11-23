//! What a frame paints (ADR-0016): paint chunks in painting order, each a
//! run of primitives recorded in its node's paint space with the property
//! tree state it paints under, and the property trees (transforms, clips,
//! effects) that state names. The painter ([`crate::paint`]) keeps each
//! node's chunk across frames and records it again only when what it paints
//! changed; a scroll, a moving translation or a fade changes a property tree
//! entry and nothing else. The renderer ([`crate::gpu`]) keeps each chunk's
//! instances on the GPU across frames, redraws only what changed (damage),
//! and decides per effect whether it needs a layer of its own. As GPUI
//! splits its scene from its renderers, every decision of what to paint is
//! the scene's and only how is the renderer's.
//!
//! Positions are logical pixels unless a field says physical; a renderer
//! multiplies by [`Scene::scale`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::color::Color;
use crate::figure::{Path, PathStyle};
use crate::geometry::Bounds;
use crate::icon::Icon;
use crate::image::ImageSource;
use crate::image::Pixels;
use crate::inspect::Raster;
use crate::style::{Corners, Shadow};
use crate::text::Glyphs;

/// One frame: chunks painted in order over `background`, under the states
/// `properties` holds.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    /// Physical pixels per logical pixel.
    pub scale: f32,
    /// The size painted, in physical pixels.
    pub size: (u32, u32),
    /// What the frame is painted over; `None` paints over what is there
    /// (the overlay and the HUD over a frame).
    pub background: Option<Color>,
    pub properties: Properties,
    pub chunks: Vec<Chunk>,
    /// What the compositor moves by itself: scroll containers, the boxes a
    /// wheel is hit-tested against, nodes whose opacity and transform move
    /// (ADR-0015).
    pub compositing: crate::compositor::Compositing,
}

impl Scene {
    #[must_use]
    pub fn new(size: (u32, u32), scale: f32, background: Option<Color>) -> Self {
        Self {
            scale,
            size,
            background,
            properties: Properties::default(),
            chunks: Vec::new(),
            compositing: crate::compositor::Compositing::default(),
        }
    }

    /// The logical size painted.
    #[must_use]
    pub fn logical_size(&self) -> (f32, f32) {
        (
            crate::convert::u32_to_f32(self.size.0) / self.scale,
            crate::convert::u32_to_f32(self.size.1) / self.scale,
        )
    }

    /// A chunk of `items` under the root state, recorded now (overlays,
    /// tests): its id is fresh.
    pub fn push(&mut self, items: Vec<Item>) {
        self.push_under(State::default(), items);
    }

    /// A chunk of `items` under `state`, recorded now, with a fresh id.
    pub fn push_under(&mut self, state: State, items: Vec<Item>) {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = ChunkId(NEXT.fetch_add(1, Ordering::Relaxed) | (1 << 63));
        self.chunks.push(Chunk {
            id,
            state,
            display: Arc::new(Display::new(items)),
        });
    }

    /// A transform node under the root moving its space by `offset`
    /// (logical) and then by `matrix`; its id.
    pub fn add_transform(&mut self, offset: (f32, f32), matrix: Option<GroupTransform>) -> u32 {
        push_id(
            &mut self.properties.transforms,
            TransformNode {
                parent: 0,
                offset,
                matrix,
            },
        )
    }

    /// A clip node; its id.
    pub fn add_clip(&mut self, clip: Clip, transform: u32) -> u32 {
        push_id(
            &mut self.properties.clips,
            ClipNode {
                clip,
                transform,
                parent: 0,
            },
        )
    }

    /// An effect node; its id.
    pub fn add_effect(&mut self, effect: EffectNode) -> u32 {
        push_id(&mut self.properties.effects, effect)
    }

    /// Every item of every chunk, in painting order (tests).
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.chunks.iter().flat_map(|c| c.display.items.iter())
    }
}

fn push_id<T>(v: &mut Vec<T>, t: T) -> u32 {
    v.push(t);
    u32::try_from(v.len().saturating_sub(1)).unwrap_or(u32::MAX)
}

/// Who a chunk is across frames: its node (and which of its chunks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkId(pub u64);

/// The property tree state a chunk paints under (Chromium's
/// `PropertyTreeState`): ids into [`Properties`], 0 the root of each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct State {
    pub transform: u32,
    pub clip: u32,
    pub effect: u32,
}

/// One node's paint, kept across frames while what it paints is the same.
#[derive(Clone, Debug)]
pub struct Chunk {
    pub id: ChunkId,
    pub state: State,
    pub display: Arc<Display>,
}

/// What a chunk paints: its items in its paint space. A recording has a
/// generation of its own; a chunk replayed as it was keeps it.
///
/// The UI thread records it and the render thread draws it, behind one
/// `Arc`. Once every holder but the painter let go of a display it
/// replaced (the render thread drew what replaced it), the painter records
/// the next chunk into it ([`Self::renew`]): its `Arc` and its items'
/// buffer are reused, so a repaint allocates nothing for them.
#[derive(Debug)]
pub struct Display {
    pub generation: u64,
    pub items: Vec<Item>,
}

/// The generations recordings are numbered by.
static GENERATIONS: AtomicU64 = AtomicU64::new(1);

impl Display {
    /// `items`, recorded now.
    #[must_use]
    pub fn new(items: Vec<Item>) -> Self {
        Self {
            generation: GENERATIONS.fetch_add(1, Ordering::Relaxed),
            items,
        }
    }

    /// Empty, under a generation of its own, for a recording made now.
    pub fn renew(&mut self) {
        self.items.clear();
        self.generation = GENERATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

/// The property trees (ADR-0016), indexed by id; entry 0 of each is the
/// root. Ids are kept for a node across frames, so a renderer finds what
/// changed by value.
#[derive(Clone, Debug, PartialEq)]
pub struct Properties {
    pub transforms: Vec<TransformNode>,
    pub clips: Vec<ClipNode>,
    pub effects: Vec<EffectNode>,
}

impl Default for Properties {
    fn default() -> Self {
        Self {
            transforms: vec![TransformNode::default()],
            clips: vec![ClipNode::default()],
            effects: vec![EffectNode::default()],
        }
    }
}

impl Properties {
    #[must_use]
    pub fn transform(&self, id: u32) -> &TransformNode {
        static ROOT: TransformNode = TransformNode {
            parent: 0,
            offset: (0.0, 0.0),
            matrix: None,
        };
        usize::try_from(id)
            .ok()
            .and_then(|i| self.transforms.get(i))
            .unwrap_or(&ROOT)
    }

    /// Clip `id`'s axis-aligned bounds as it clips now: its rectangle
    /// within each it lies in ([`ClipNode::parent`]), each where its own
    /// transform has it, its own corners and transform. What culls and
    /// rejects early; the GPU's clip table keeps each clip's own rounded
    /// rectangle and its parent, and the shaders walk the chain.
    #[must_use]
    pub fn clip(&self, id: u32) -> Option<ClipNode> {
        let get = |id: u32| {
            (id != 0)
                .then(|| usize::try_from(id).ok().and_then(|i| self.clips.get(i)))
                .flatten()
        };
        let mut c = *get(id)?;
        let mut up = c.parent;
        for _ in 0..64 {
            let Some(p) = get(up) else { break };
            c.clip.bounds = c.clip.bounds.intersect(&p.clip.bounds);
            up = p.parent;
        }
        Some(c)
    }

    #[must_use]
    pub fn effect(&self, id: u32) -> Option<&EffectNode> {
        (id != 0)
            .then(|| usize::try_from(id).ok().and_then(|i| self.effects.get(i)))
            .flatten()
    }

    /// The scale-and-turn transforms from `id` up to (not including)
    /// `until`, innermost first.
    pub fn matrices(&self, id: u32, until: u32) -> impl Iterator<Item = GroupTransform> + '_ {
        let mut at = id;
        let mut steps = 0_usize;
        std::iter::from_fn(move || {
            while at != 0 && at != until && steps < 4096 {
                steps = steps.saturating_add(1);
                let t = self.transform(at);
                at = t.parent;
                if let Some(m) = t.matrix {
                    return Some(m);
                }
            }
            None
        })
    }

    /// The effect `id` and its ancestors, innermost first, up to (not
    /// including) `until`.
    pub fn effect_chain(&self, id: u32, until: u32) -> impl Iterator<Item = (u32, &EffectNode)> + '_ {
        let mut at = id;
        let mut steps = 0_usize;
        std::iter::from_fn(move || {
            if at == 0 || at == until || steps >= 4096 {
                return None;
            }
            steps = steps.saturating_add(1);
            let e = self.effect(at)?;
            let here = at;
            at = e.parent;
            Some((here, e))
        })
    }
}

/// A transform node: what takes a paint space to its parent's, and so to
/// the window. Scroll offsets and moving translations are `offset`s (the
/// whole of it from the space to the window, not from the parent); a scale
/// or a turn is `matrix`, applied about its origin after the offset.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TransformNode {
    pub parent: u32,
    /// Logical pixels from the space to the window, before any matrix.
    pub offset: (f32, f32),
    pub matrix: Option<GroupTransform>,
}

/// A clip node: a rounded rectangle in the window (as laid out, before any
/// scale or turn), and the transform whose scales and turns it lies under.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClipNode {
    pub clip: Clip,
    pub transform: u32,
    /// The clip it lies within (0 for none), each moved by its own
    /// transform: a text input's own clip within the container's that
    /// clips it, which stays with the container as what is between them
    /// scrolls or moves ([`Properties::clip`] intersects them).
    pub parent: u32,
}

/// An effect node (CSS's stacking context with `opacity`, `filter`,
/// `mask-image` or a scale or turn): what its subtree is faded, masked and
/// filtered by as one. The renderer decides whether that needs a layer of
/// its own (ADR-0016's per-case rule).
#[derive(Clone, Debug, PartialEq)]
pub struct EffectNode {
    pub parent: u32,
    pub opacity: f32,
    pub filters: crate::style::Filters,
    /// A gradient whose alpha scales the group's, before filters (window,
    /// logical).
    pub mask: Option<Gradient>,
    /// The transform node its subtree paints under: its own when it moves
    /// or scales or turns, else the one it inherits.
    pub transform: u32,
    /// That node is its own scale or turn.
    pub transformed: bool,
    /// The physical window pixels its subtree may paint, as untransformed.
    pub region: Option<PixelRect>,
}

impl Default for EffectNode {
    fn default() -> Self {
        Self {
            parent: 0,
            opacity: 1.0,
            filters: crate::style::Filters::new(),
            mask: None,
            transform: 0,
            transformed: false,
            region: None,
        }
    }
}

/// A rounded rectangle an item paints inside: an ancestor's clip, its
/// bounds as laid out (snapped to device pixels when painted), its inner
/// corners and their curve (the ancestor's shape's, CSS's
/// `superellipse(k)`; 1 is round).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip {
    pub bounds: Bounds,
    pub radius: Corners,
    pub curve: f32,
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            bounds: Bounds::default(),
            radius: Corners::default(),
            curve: crate::shape::ROUND,
        }
    }
}

/// One primitive, in its chunk's paint space.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A rectangle, edges anti-aliased.
    Rect { bounds: Bounds, color: Color },
    /// A rounded rectangle.
    RoundedRect {
        bounds: Bounds,
        radius: Corners,
        color: Color,
    },
    /// What lies between two rounded rectangles: a border, a ring.
    Band {
        outer: Bounds,
        outer_radius: Corners,
        /// Empty paints the whole outer rectangle.
        inner: Bounds,
        inner_radius: Corners,
        color: Color,
    },
    /// A box whose corners curve as CSS's `superellipse(k)` (a
    /// [`crate::shape::Shape::Superellipse`]), filled, or with `inner`
    /// (same curve) cut out: a border, a ring. Radii are used values.
    Superellipse {
        outer: Bounds,
        outer_radius: Corners,
        /// Empty fills the whole outer box.
        inner: Bounds,
        inner_radius: Corners,
        k: f32,
        color: Color,
    },
    /// A box shadow cast by `bounds` (snapped), drawn only outside it, as
    /// CSS does; the box's corners curve as `curve` (CSS's
    /// `superellipse(k)`, 1 round), and so do the shadow's.
    Shadow {
        bounds: Bounds,
        radius: Corners,
        curve: f32,
        shadow: Shadow,
    },
    /// `gradient` filling a rounded rectangle.
    Gradient {
        bounds: Bounds,
        radius: Corners,
        gradient: Gradient,
    },
    /// One glyph of a font at physical position `x`, `y` (its pen, on the
    /// baseline), in `color`.
    Glyph {
        key: crate::text::GlyphKey,
        x: i32,
        y: i32,
        color: Color,
    },
    /// A Lucide icon `size` physical pixels square, its top left at `x`, `y` physical.
    Icon {
        icon: Icon,
        size: u32,
        color: Color,
        stroke_width: f32,
        x: i32,
        y: i32,
    },
    /// An image covering `bounds` (snapped), centred, inside corners of
    /// radii `radius` curving as `curve`.
    Image {
        source: ImageSource,
        bounds: Bounds,
        radius: Corners,
        curve: f32,
    },
    /// What lies below `bounds` in its target (the canvas, or the layer of
    /// the nearest effect drawn aside: Filter Effects 2's backdrop root),
    /// blurred and saturated, shown inside the box's corners (radii
    /// `radius`, curve `curve`): CSS's `backdrop-filter`. The blur reads
    /// only the box, its edges mirrored, as Chrome's does.
    Backdrop {
        bounds: Bounds,
        radius: Corners,
        curve: f32,
        backdrop: crate::shape::Backdrop,
    },
    /// What lies between two boxes of one curve (a border), in `color`
    /// blended with what lies below by `blend` (CSS's `mix-blend-mode`,
    /// separable, in the target's encoded values as browsers blend). A
    /// [`crate::shape::Blend::Normal`] one is a [`Item::Band`].
    Blended {
        outer: Bounds,
        outer_radius: Corners,
        /// Empty paints the whole outer box.
        inner: Bounds,
        inner_radius: Corners,
        curve: f32,
        color: Color,
        blend: crate::shape::Blend,
    },
    /// A vector path painted with `style`, its coordinates (logical) from
    /// `origin`: rasterized on the GPU (ADR-0017).
    Path {
        origin: (f32, f32),
        path: Path,
        style: PathStyle,
    },
    /// Text showing `fill` through it in place of its colour
    /// (`background-clip: text`): `mask` holds the text's items (glyphs,
    /// decorations), whose alpha alone counts, and `region` (physical)
    /// bounds what they paint.
    TextThrough {
        region: PixelRect,
        mask: Vec<Item>,
        /// The node's own background, beneath the gradient.
        background: Option<(Bounds, Color)>,
        /// The gradient and the rectangle it shows in.
        fill: Option<(Gradient, Bounds)>,
    },
    /// A GPU data chunk (ADR-0018): a chart's mark, a heatmap, a user's
    /// kind, drawn by its kind in this place of the order.
    Data(Arc<crate::chunk::DataItem>),
    /// A canvas (ADR-0017): its world through its camera, drawn by the
    /// renderer straight or from tiles, cut to its box.
    World(Arc<crate::canvas::WorldView>),
}

impl Item {
    /// The physical pixels it may paint in its space, its reach (a
    /// shadow's blur and offset, an anti-aliased edge) included.
    #[must_use]
    pub fn reach(&self, scale: f32) -> Option<[f32; 4]> {
        let phys = |b: Bounds, d: f32| -> [f32; 4] {
            [
                b.x.mul_add(scale, -d),
                b.y.mul_add(scale, -d),
                b.right().mul_add(scale, d),
                b.bottom().mul_add(scale, d),
            ]
        };
        let r = match self {
            Self::Rect { bounds, .. } | Self::RoundedRect { bounds, .. } | Self::Gradient { bounds, .. } => {
                phys(*bounds, 1.0)
            }
            Self::Image { bounds, .. } | Self::Backdrop { bounds, .. } => phys(*bounds, 1.0),
            Self::Band { outer, .. } | Self::Superellipse { outer, .. } | Self::Blended { outer, .. } => {
                phys(*outer, 1.0)
            }
            Self::Shadow { bounds, shadow, .. } => {
                let shape = bounds.translated(shadow.x, shadow.y).outset(shadow.spread);
                if shape.is_empty() {
                    return None;
                }
                let sigma = shadow.blur * scale / 2.0;
                phys(shape, sigma.mul_add(3.0, 1.0))
            }
            Self::Glyph { key, x, y, .. } => {
                // A glyph's ink lies within an em of its pen, either way.
                let em = f32::from_bits(key.ppem).max(1.0) * 1.5;
                let (x, y) = (crate::convert::i32_to_f32(*x), crate::convert::i32_to_f32(*y));
                [x - em, y - em, x + em, y + em]
            }
            Self::Icon { size, x, y, .. } => {
                let (x, y) = (crate::convert::i32_to_f32(*x), crate::convert::i32_to_f32(*y));
                let s = crate::convert::u32_to_f32(*size);
                [x - 1.0, y - 1.0, x + s + 1.0, y + s + 1.0]
            }
            Self::Path { origin, path, style } => {
                let [x0, y0, x1, y1] = path.bounds();
                // A stroke reaches half its width out, a mitred join as far
                // as its limit lets it.
                let w = match &style.draw {
                    crate::figure::Draw::Stroke(s) => s.width * 0.5 * s.miter_limit.max(1.0),
                    crate::figure::Draw::Fill => 0.0,
                };
                let b = Bounds {
                    x: x0 + origin.0 - w,
                    y: y0 + origin.1 - w,
                    width: (x1 - x0) + 2.0 * w,
                    height: (y1 - y0) + 2.0 * w,
                };
                phys(b, 1.0)
            }
            Self::Data(d) => phys(d.region, 1.0),
            Self::World(v) => phys(v.bounds, 1.0),
            Self::TextThrough { region, .. } => {
                let x = crate::convert::i32_to_f32(region.x);
                let y = crate::convert::i32_to_f32(region.y);
                [
                    x,
                    y,
                    x + crate::convert::u32_to_f32(region.width),
                    y + crate::convert::u32_to_f32(region.height),
                ]
            }
        };
        (r[2] > r[0] && r[3] > r[1]).then_some(r)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupTransform {
    pub origin: (f32, f32),
    pub scale: f32,
    /// Degrees, clockwise.
    pub rotate: f32,
}

impl GroupTransform {
    /// Whether it keeps the pixel grid (ADR-0020): no scale and no turn
    /// (a whole number of turns). Only under such a transform are edges
    /// snapped to device pixels; under any other, geometry stays fractional.
    #[must_use]
    pub fn preserves_grid(&self) -> bool {
        (self.scale - 1.0).abs() < 1e-4 && self.rotate.rem_euclid(360.0) < 1e-4
    }

    /// The transform as a matrix `[a, b, c, d, e, f]`: `x' = a·x + c·y + e`,
    /// `y' = b·x + d·y + f` (physical pixels).
    #[must_use]
    pub fn matrix(&self) -> [f32; 6] {
        let (sin, cos) = self.rotate.to_radians().sin_cos();
        let k = self.scale;
        let (a, b, c, d) = (cos * k, sin * k, -sin * k, cos * k);
        let (ox, oy) = self.origin;
        [a, b, c, d, ox - (a * ox + c * oy), oy - (b * ox + d * oy)]
    }
}

/// A rectangle of whole physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A linear gradient placed in logical pixels: colours `stops` along the
/// line from `start` to `end` (offsets `0.0..=1.0` along it), mixed in OKLab
/// between stops as CSS mixes `bg-linear-*`, padded past both ends.
#[derive(Clone, Debug, PartialEq)]
pub enum Gradient {
    Solid(Color),
    Linear {
        start: (f32, f32),
        end: (f32, f32),
        stops: Vec<(f32, Color)>,
    },
}

/// What a renderer holds across frames: glyphs and icons (the atlas pages
/// they fill), images, the paths it binned (their buffers on the GPU), group
/// layers (the textures) and the retained chunks' instances.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub glyphs: Raster,
    pub icons: Raster,
    pub images: Raster,
    pub paths: Raster,
    pub layers: Raster,
    /// Chunks whose instances live on the GPU, and the bytes they take.
    pub chunks: Raster,
}

/// What a renderer did for the last frame (ADR-0016's counters, shown in
/// [`crate::inspect::FrameReport`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drawn {
    /// Work went to the GPU: a frame with no damage submits nothing.
    pub submitted: bool,
    /// Chunks whose instances were written again (new, recorded again, or
    /// moved to another state or atlas page).
    pub encoded: usize,
    /// Bytes written to GPU buffers.
    pub uploaded: u64,
    /// Physical pixels redrawn, and the rectangles they were merged into.
    pub damage: u64,
    pub damage_rects: usize,
    /// Layers drawn this frame, and layers composited as they were kept.
    pub layers: usize,
    pub layers_kept: usize,
    /// Effects whose opacity was multiplied into their instances, no layer needed.
    pub folded: usize,
    /// Chunks whose route and signature were decided again, and whether
    /// the frame was planned whole (not from what changed).
    pub signed: usize,
    pub planned_whole: bool,
    /// Clip table entries written: a clip that moved writes its own, not
    /// those of the clips within it (the shaders walk each chain).
    pub clips_written: usize,
    /// The longest effective clip chain among the clips decided this frame
    /// (the clips the shaders walk for it, [`crate::clip::needed`]).
    pub clip_chain: usize,
    /// The GPU time of the last frame whose timestamps came back (a frame
    /// or two behind; `None` without timestamp queries), its first pass's
    /// start to its last pass's end, and what each kind of pass took.
    pub gpu: Option<std::time::Duration>,
    pub gpu_passes: Option<GpuPasses>,
    /// How long a headless frame waited for the GPU to finish it (part of
    /// its paint time: what is left is the CPU's).
    pub gpu_wait: std::time::Duration,
    /// Canvas raster this frame (ADR-0017): the cost spent rasterizing tile
    /// steps ([`crate::canvas`]'s budget units), tiles finished, and how
    /// long a headless frame waited for the raster workers' plans (not the
    /// render thread's work: what is left of the frame is).
    pub raster: u32,
    pub tiles_done: usize,
    pub raster_wait: std::time::Duration,
}

/// A frame's GPU time by kind of pass, from timestamp queries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuPasses {
    /// The path rasterizer's compute passes.
    pub paths: std::time::Duration,
    /// Data chunks' compute passes (M4, culling).
    pub compute: std::time::Duration,
    /// Data chunks' coverage passes (lines and areas).
    pub coverage: std::time::Duration,
    /// The frame's render passes: the canvas, layers, tiles, the blit.
    pub main: std::time::Duration,
}

impl GpuPasses {
    /// The data chunks' own passes.
    #[must_use]
    pub fn chunks(&self) -> std::time::Duration {
        self.compute.saturating_add(self.coverage)
    }
}

/// What changed in a scene since a renderer last drew it (ADR-0016): the
/// render side's work is what this names, never a pass over the whole.
/// The compositor knows it from the commits it took in and the entries
/// its scroll and motion wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    /// Anything may have (the first frame, an order or a size changed): the
    /// renderer plans the scene whole.
    pub all: bool,
    /// Chunks recorded again, by index in [`Scene::chunks`].
    pub chunks: Vec<usize>,
    /// Transform entries that changed, with every transform under one
    /// whose matrix changed (it moves what they draw).
    pub transforms: Vec<u32>,
    pub clips: Vec<u32>,
    /// Effect entries that changed, with every effect under one (its
    /// opacity is folded into theirs).
    pub effects: Vec<u32>,
}

impl Changed {
    /// Anything may have changed.
    #[must_use]
    pub fn all() -> Self {
        Self {
            all: true,
            ..Self::default()
        }
    }

    /// Nothing did.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.all
            && self.chunks.is_empty()
            && self.transforms.is_empty()
            && self.clips.is_empty()
            && self.effects.is_empty()
    }

    /// Forget it all (kept for its allocations).
    pub fn clear(&mut self) {
        self.all = false;
        self.chunks.clear();
        self.transforms.clear();
        self.clips.clear();
        self.effects.clear();
    }
}

/// A set of ids met this frame, kept across frames for its memory.
#[derive(Default)]
pub(crate) struct Marks {
    stamp: Vec<u32>,
    now: u32,
}

impl Marks {
    pub(crate) fn begin(&mut self, n: usize) {
        if self.stamp.len() < n {
            self.stamp.resize(n, 0);
        }
        self.now = self.now.wrapping_add(1);
        if self.now == 0 {
            self.stamp.fill(0);
            self.now = 1;
        }
    }

    /// Mark `id`; whether it was not yet.
    pub(crate) fn mark(&mut self, id: u32) -> bool {
        let Some(s) = usize::try_from(id).ok().and_then(|i| self.stamp.get_mut(i)) else {
            return false;
        };
        let fresh = *s != self.now;
        *s = self.now;
        fresh
    }

    pub(crate) fn has(&self, id: u32) -> bool {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.stamp.get(i))
            .is_some_and(|s| *s == self.now)
    }
}

/// Turns scenes into pixels.
pub trait Renderer {
    /// Bring the frame's canvas up to `scene`, of which `changed` changed
    /// since the last: only that is planned and drawn again; an unchanged
    /// scene draws and submits nothing.
    fn render(&mut self, scene: &Scene, changed: &Changed, text: &mut Glyphs);
    /// Paint `scene` over this frame as shown, for this frame only (the
    /// overlay and the HUD).
    fn overlay(&mut self, scene: &Scene, text: &mut Glyphs);
    /// The frame is painted: show it, and forget what the last frames did not use.
    fn end_frame(&mut self);
    /// What is held across frames.
    fn held(&self) -> Held;
    /// What the last frame did.
    fn drawn(&self) -> Drawn;
    /// `scene`, and `overlay` over it, painted whole aside and read back, for
    /// a screenshot.
    fn capture(&mut self, scene: &Scene, overlay: Option<&Scene>, text: &mut Glyphs) -> Option<Pixels>;
    /// How long rendering waited for the display since last asked.
    fn waited(&mut self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}
