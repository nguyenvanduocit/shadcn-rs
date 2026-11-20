//! What a frame paints, as a list of primitives in painting order: the
//! painter records a [`Scene`] from the laid-out frame, and a renderer turns
//! it into pixels — on the GPU in a window ([`crate::gpu`]), on the CPU
//! headless or where no GPU answers ([`crate::cpu`]). As GPUI splits its
//! scene from its Metal and wgpu renderers, the two renderers share every
//! decision of what to paint and differ only in how.
//!
//! Positions are logical pixels unless a field says physical; a renderer
//! multiplies by [`Scene::scale`].

use crate::color::Color;
use crate::geometry::Bounds;
use crate::icon::Icon;
use crate::image::ImageSource;
use crate::inspect::Raster;
use crate::shape::Shape;
use crate::style::{Corners, Filter, Shadow};
use crate::text::TextSystem;

/// One frame's primitives, painted in order over `background`.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    /// Physical pixels per logical pixel.
    pub scale: f32,
    /// The size painted, in physical pixels.
    pub size: (u32, u32),
    /// What the frame is painted over; `None` paints over what is there
    /// (the overlay and the HUD over a frame).
    pub background: Option<Color>,
    pub items: Vec<Item>,
}

impl Scene {
    #[must_use]
    pub fn new(size: (u32, u32), scale: f32, background: Option<Color>) -> Self {
        Self {
            scale,
            size,
            background,
            items: Vec::new(),
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

    /// `other`'s items painted after this scene's.
    pub fn append(&mut self, other: Self) {
        self.items.extend(other.items);
    }
}

/// A rounded rectangle an item paints inside: an ancestor's clip, its
/// bounds as laid out (snapped to device pixels when painted) and its
/// inner corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip {
    pub bounds: Bounds,
    pub radius: Corners,
}

/// One primitive, clipped by `clip` where it has one.
#[derive(Clone, Debug)]
pub enum Item {
    /// A rectangle, edges anti-aliased.
    Rect {
        bounds: Bounds,
        color: Color,
        clip: Option<Clip>,
    },
    /// A rounded rectangle.
    RoundedRect {
        bounds: Bounds,
        radius: Corners,
        color: Color,
        clip: Option<Clip>,
    },
    /// What lies between two rounded rectangles: a border, a ring.
    Band {
        outer: Bounds,
        outer_radius: Corners,
        /// Empty paints the whole outer rectangle.
        inner: Bounds,
        inner_radius: Corners,
        color: Color,
        clip: Option<Clip>,
    },
    /// A box shadow cast by `bounds` (snapped), drawn only outside it, as CSS does.
    Shadow {
        bounds: Bounds,
        radius: Corners,
        shadow: Shadow,
        clip: Option<Clip>,
    },
    /// `gradient` filling a rounded rectangle.
    Gradient {
        bounds: Bounds,
        radius: Corners,
        gradient: Gradient,
        clip: Option<Clip>,
    },
    /// One glyph of a font at physical position `x`, `y` (its pen, on the
    /// baseline), in `color`.
    Glyph {
        key: cosmic_text::CacheKey,
        x: i32,
        y: i32,
        color: Color,
        clip: Option<Clip>,
    },
    /// A Lucide icon `size` physical pixels square, its top left at `x`, `y` physical.
    Icon {
        icon: Icon,
        size: u32,
        color: Color,
        stroke_width: f32,
        x: i32,
        y: i32,
        clip: Option<Clip>,
    },
    /// An image covering `bounds` (snapped), centred, inside rounded corners.
    Image {
        source: ImageSource,
        bounds: Bounds,
        radius: Corners,
        clip: Option<Clip>,
    },
    /// Vector shapes, their coordinates from `origin` (snapped).
    Shapes {
        origin: (f32, f32),
        shapes: Vec<Shape>,
        clip: Option<Clip>,
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
        clip: Option<Clip>,
    },
    /// Items painted aside on a transparent layer, then masked, filtered,
    /// faded, scaled and turned as one and painted here.
    Group(Box<Group>),
}

/// A group of items composited as one (CSS's stacking context with
/// `opacity`, `filter`, `mask-image` or `transform`).
#[derive(Clone, Debug)]
pub struct Group {
    /// The physical pixels the group's items may paint, as untransformed.
    pub region: PixelRect,
    pub items: Vec<Item>,
    /// A gradient whose alpha scales the group's, before filters.
    pub mask: Option<Gradient>,
    pub filters: Vec<Filter>,
    pub opacity: f32,
    /// Scale then turn about `origin` (physical pixels).
    pub transform: Option<GroupTransform>,
    /// The clip from outside, applied once transformed.
    pub clip: Option<Clip>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupTransform {
    pub origin: (f32, f32),
    pub scale: f32,
    /// Degrees, clockwise.
    pub rotate: f32,
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

/// What a renderer holds across frames: glyphs, icons, shadows, scaled
/// images and group layers (on the GPU, the atlas pages and textures each
/// fills).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub glyphs: Raster,
    pub icons: Raster,
    pub shadows: Raster,
    pub images: Raster,
    pub layers: Raster,
}

/// Turns scenes into pixels: the GPU in a window, the CPU into a pixmap.
pub trait Renderer {
    /// Paint `scene` over what this frame painted so far (or over its
    /// background, when it has one).
    fn render(&mut self, scene: &Scene, text: &mut TextSystem);
    /// The frame is painted: show it, and forget what the last frames did not use.
    fn end_frame(&mut self);
    /// What is held across frames.
    fn held(&self) -> Held;
    /// `scene` painted aside and read back, for a screenshot.
    fn capture(&mut self, scene: &Scene, text: &mut TextSystem) -> Option<tiny_skia::Pixmap>;
    /// How long rendering waited for the display since last asked.
    fn waited(&mut self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}
