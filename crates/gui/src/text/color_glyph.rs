//! Colour glyphs (emoji) rasterized into premultiplied RGBA bitmaps for an RGBA atlas page.
//!
//! ADR-0019: COLR fonts are painted through skrifa's paint-graph traversal onto a `vello_cpu`
//! canvas (which supplies analytic antialiasing, two-point conical and sweep gradients, extend
//! modes and every composite mode). `sbix` and `CBDT` bitmaps are decoded and scaled to the
//! requested size on the CPU. Both produce premultiplied RGBA with no gamma table and no LCD
//! coverage: a colour glyph is an image, not a coverage mask.
//!
//! Shaping happens elsewhere; this module only turns one glyph id at one size into pixels.

use std::collections::HashMap;
use std::io::Cursor;

use skrifa::bitmap::{BitmapData, BitmapFormat, BitmapGlyph, BitmapStrikes, Origin};
use skrifa::color::{Brush, ColorGlyph, ColorGlyphFormat, ColorPainter, ColorStop, CompositeMode, Transform};
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, OutlineGlyphCollection, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::raw::types::BoundingBox;
use skrifa::string::StringId;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use vello_cpu::color::{AlphaColor, DynamicColor, Srgb};
use vello_cpu::kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use vello_cpu::peniko::{
    self, BlendMode, ColorStops, Compose, Gradient, LinearGradientPosition, Mix, RadialGradientPosition,
    SweepGradientPosition,
};
use vello_cpu::{PixmapMut, RasterizerSettings, RenderContext, RenderMode, Resources};

/// A rasterized colour glyph, positioned relative to the pen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorImage {
    /// Pixel offset of the bitmap's left edge from the pen position, and of its top edge ABOVE the baseline (positive up), in device pixels.
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA, row-major, width*height*4 bytes.
    pub rgba: Vec<u8>,
}

impl ColorImage {
    /// A colour glyph that exists but paints nothing (a space in an emoji font, say). It is still
    /// a colour glyph, so the caller must not fall back to the outline.
    fn empty() -> Self {
        Self {
            left: 0,
            top: 0,
            width: 0,
            height: 0,
            rgba: Vec::new(),
        }
    }
}

/// The colour glyph `glyph` of `font` at `ppem` device pixels per em, with `coords` (normalized variation coordinates) and a horizontal subpixel offset `x_offset` in [0,1) device px, or None if the glyph has no colour form (COLRv0/v1, sbix, CBDT/CBLC) — the caller then draws its outline.
/// `foreground` is the text colour (straight RGBA) for COLR palette index 0xFFFF.
pub fn render(
    font: &FontRef<'_>,
    glyph: GlyphId,
    ppem: f32,
    coords: &[NormalizedCoord],
    x_offset: f32,
    foreground: [u8; 4],
) -> Option<ColorImage> {
    if !ppem.is_finite() || ppem <= 0.0 {
        return None;
    }
    let x_offset = if x_offset.is_finite() { x_offset } else { 0.0 };
    match color_form(font, glyph)? {
        ColorForm::Colr(color_glyph) => render_colr(font, &color_glyph, ppem, coords, x_offset, foreground),
        ColorForm::Bitmap(strikes) => render_bitmap(font, &strikes, glyph, ppem, x_offset),
    }
}

/// Whether `render` would draw this glyph in colour (cheap; used to route it to the colour atlas).
pub fn has_color(font: &FontRef<'_>, glyph: GlyphId) -> bool {
    color_form(font, glyph).is_some()
}

/// Whether the glyph's COLR paint uses the foreground colour (so the caller keys the cache on it).
pub fn uses_foreground(font: &FontRef<'_>, glyph: GlyphId) -> bool {
    let Some(ColorForm::Colr(color_glyph)) = color_form(font, glyph) else {
        return false;
    };
    let mut probe = ForegroundProbe { found: false };
    // A malformed graph stops the walk early; whatever was seen before that still counts, and
    // `render` refuses such a glyph anyway.
    match color_glyph.paint(LocationRef::default(), &mut probe) {
        Ok(()) | Err(_) => probe.found,
    }
}

/// The palette index COLR uses for "the current text colour".
const FOREGROUND_INDEX: u16 = 0xFFFF;

/// The largest bitmap edge we rasterize. `vello_cpu` addresses its canvas with `u16`, and a
/// colour glyph this large is better drawn as paths by the GPU than cached in an atlas.
const MAX_EDGE: i32 = 4096;

/// Slack used when rounding bounds out to whole pixels, so that float noise on an edge that lies
/// exactly on a pixel boundary does not add an empty row or column.
const EDGE_EPSILON: f64 = 1e-3;

/// The colour representation chosen for one glyph.
enum ColorForm<'a> {
    Colr(ColorGlyph<'a>),
    Bitmap(BitmapStrikes<'a>),
}

/// Picks the glyph's colour form in the order Chrome's fontations backend uses: COLRv1, then
/// embedded colour bitmaps (sbix, CBDT), then COLRv0. COLRv1 wins because it is resolution
/// independent; COLRv0 loses to bitmaps because fonts that carry both keep v0 as a fallback.
fn color_form<'a>(font: &FontRef<'a>, glyph: GlyphId) -> Option<ColorForm<'a>> {
    let colr = font.color_glyphs();
    if let Some(v1) = colr.get_with_format(glyph, ColorGlyphFormat::ColrV1) {
        return Some(ColorForm::Colr(v1));
    }
    if let Some(strikes) = color_strikes(font)
        && strikes
            .iter()
            .any(|strike| strike.get(glyph).is_some_and(|bitmap| is_color_bitmap(&bitmap)))
    {
        return Some(ColorForm::Bitmap(strikes));
    }
    colr.get_with_format(glyph, ColorGlyphFormat::ColrV0)
        .map(ColorForm::Colr)
}

/// The font's colour bitmap strikes. EBDT is left out on purpose: it holds monochrome or grey
/// masks, which are outline substitutes rather than colour glyphs.
fn color_strikes<'a>(font: &FontRef<'a>) -> Option<BitmapStrikes<'a>> {
    BitmapStrikes::with_format(font, BitmapFormat::Sbix)
        .or_else(|| BitmapStrikes::with_format(font, BitmapFormat::Cbdt))
}

/// A bitmap is a colour glyph when it carries colour pixels. CBDT may also store grey masks.
fn is_color_bitmap(bitmap: &BitmapGlyph<'_>) -> bool {
    matches!(bitmap.data, BitmapData::Png(_) | BitmapData::Bgra(_))
}

// ---------------------------------------------------------------------------------------------
// COLR
// ---------------------------------------------------------------------------------------------

/// Paints a COLRv0 or COLRv1 glyph. The canvas is the glyph's clip box when the font has one,
/// otherwise the union of everything the paint graph draws, both rounded out to whole pixels.
fn render_colr(
    font: &FontRef<'_>,
    color_glyph: &ColorGlyph<'_>,
    ppem: f32,
    coords: &[NormalizedCoord],
    x_offset: f32,
    foreground: [u8; 4],
) -> Option<ColorImage> {
    let location = LocationRef::new(coords);
    let upem = font.head().ok()?.units_per_em();
    if upem == 0 {
        return None;
    }
    let scale = f64::from(ppem) / f64::from(upem);
    // Font units are y-up with the pen at the origin; the pen space is y-down pixels with the
    // pen at (x_offset, 0) on the baseline.
    let font_to_pen = Affine::new([scale, 0.0, 0.0, -scale, f64::from(x_offset), 0.0]);
    let outlines = font.outline_glyphs();
    let mut paths = PathCache::default();

    let bounds = match color_glyph.bounding_box(location, Size::unscaled()) {
        Some(clip) => Some(font_to_pen.transform_rect_bbox(rect_from_box(clip))),
        None => {
            let mut probe = BoundsProbe {
                outlines: &outlines,
                location,
                paths: &mut paths,
                transforms: vec![font_to_pen],
                clips: Vec::new(),
                bounds: None,
            };
            color_glyph.paint(location, &mut probe).ok()?;
            probe.bounds
        }
    };
    let Some(bounds) = bounds.filter(|rect| rect.area() > 0.0 && rect.is_finite()) else {
        return Some(ColorImage::empty());
    };

    let left = floor_px(bounds.x0 + EDGE_EPSILON);
    let top_down = floor_px(bounds.y0 + EDGE_EPSILON);
    let right = ceil_px(bounds.x1 - EDGE_EPSILON);
    let bottom = ceil_px(bounds.y1 - EDGE_EPSILON);
    let (Some(width), Some(height)) = (canvas_edge(left, right), canvas_edge(top_down, bottom)) else {
        return Some(ColorImage::empty());
    };

    let to_canvas = font_to_pen.then_translate(Vec2::new(-f64::from(left), -f64::from(top_down)));
    let mut ctx = RenderContext::new(width, height);
    let mut painter = ColrPainter {
        ctx: &mut ctx,
        outlines: &outlines,
        location,
        paths: &mut paths,
        palette: palette(font),
        foreground: AlphaColor::from_rgba8(foreground[0], foreground[1], foreground[2], foreground[3]),
        transforms: vec![to_canvas],
        stack: Vec::new(),
        canvas: Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
    };
    let painted = color_glyph.paint(location, &mut painter);
    // skrifa leaves pushes unbalanced when it aborts a malformed graph; the canvas refuses to
    // render with open layers, so close everything first.
    painter.unwind();
    painted.ok()?;

    let mut rgba = vec![0_u8; usize::from(width).checked_mul(usize::from(height))?.checked_mul(4)?];
    let target = PixmapMut::new(width, height, &mut rgba)?;
    let settings = RasterizerSettings {
        render_mode: RenderMode::OptimizeQuality,
        ..RasterizerSettings::default()
    };
    ctx.render_with(target, &mut Resources::new(), settings);

    Some(ColorImage {
        left,
        top: top_down.checked_neg()?,
        width: u32::from(width),
        height: u32::from(height),
        rgba,
    })
}

/// Palette 0, the default palette every COLR renderer uses unless the user picks another.
fn palette(font: &FontRef<'_>) -> Vec<skrifa::color::Color> {
    font.color_palettes()
        .get(0)
        .map(|palette| palette.colors().to_vec())
        .unwrap_or_default()
}

/// Width or height of the canvas between two pixel edges, if it is drawable.
fn canvas_edge(start: i32, end: i32) -> Option<u16> {
    let len = end.checked_sub(start)?;
    if len <= 0 || len > MAX_EDGE {
        return None;
    }
    u16::try_from(len).ok()
}

/// Outlines in font units, built once per render even when the paint graph clips to the same
/// glyph many times (Noto's flags reuse a handful of shapes thousands of times).
#[derive(Default)]
struct PathCache {
    paths: HashMap<GlyphId, Option<BezPath>>,
}

impl PathCache {
    fn get(
        &mut self,
        outlines: &OutlineGlyphCollection<'_>,
        location: LocationRef<'_>,
        glyph: GlyphId,
    ) -> Option<&BezPath> {
        self.paths
            .entry(glyph)
            .or_insert_with(|| {
                let outline = outlines.get(glyph)?;
                let mut pen = PathPen(BezPath::new());
                outline
                    .draw(DrawSettings::unhinted(Size::unscaled(), location), &mut pen)
                    .ok()?;
                Some(pen.0)
            })
            .as_ref()
    }
}

/// Collects a skrifa outline into a kurbo path.
struct PathPen(BezPath);

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0
            .quad_to((f64::from(cx0), f64::from(cy0)), (f64::from(x), f64::from(y)));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(
            (f64::from(cx0), f64::from(cy0)),
            (f64::from(cx1), f64::from(cy1)),
            (f64::from(x), f64::from(y)),
        );
    }

    fn close(&mut self) {
        self.0.close_path();
    }
}

/// What the painter has open on the canvas, so pops match pushes even for malformed fonts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    Clip,
    Layer,
}

/// Draws the paint graph onto a `vello_cpu` canvas.
///
/// Clips are non-isolated clip paths (cheap, exact coverage); only `PaintComposite` opens a
/// layer, because only it needs an isolated group to blend with a mode.
struct ColrPainter<'c, 'f> {
    ctx: &'c mut RenderContext,
    outlines: &'c OutlineGlyphCollection<'f>,
    location: LocationRef<'c>,
    paths: &'c mut PathCache,
    palette: Vec<skrifa::color::Color>,
    foreground: AlphaColor<Srgb>,
    transforms: Vec<Affine>,
    stack: Vec<Open>,
    canvas: Rect,
}

impl ColrPainter<'_, '_> {
    fn current(&self) -> Affine {
        self.transforms.last().copied().unwrap_or(Affine::IDENTITY)
    }

    fn pop_open(&mut self, expected: Open) -> bool {
        if self.stack.last() == Some(&expected) {
            self.stack.pop();
            true
        } else {
            false
        }
    }

    fn unwind(&mut self) {
        while let Some(open) = self.stack.pop() {
            match open {
                Open::Clip => self.ctx.pop_clip(),
                Open::Layer => self.ctx.pop_layer(),
            }
        }
    }

    fn color(&self, palette_index: u16, alpha: f32) -> AlphaColor<Srgb> {
        let base = if palette_index == FOREGROUND_INDEX {
            self.foreground
        } else {
            // An index past the palette is a font bug; FreeType and Skia paint it black.
            self.palette
                .get(usize::from(palette_index))
                .map_or(AlphaColor::BLACK, |c| {
                    AlphaColor::from_rgba8(c.red, c.green, c.blue, c.alpha)
                })
        };
        base.multiply_alpha(alpha)
    }

    /// Converts a COLR colour line to peniko's, which wants stops at exactly 0 and 1. COLR
    /// keeps only the last of several stops at offset 1, so the earlier ones are dropped.
    fn stops(&self, stops: &[ColorStop]) -> ColorStops {
        let mut out: Vec<peniko::ColorStop> = stops
            .iter()
            .map(|stop| peniko::ColorStop {
                offset: stop.offset,
                color: DynamicColor::from_alpha_color(self.color(stop.palette_index, stop.alpha)),
            })
            .collect();
        if let Some(first) = out.first().copied()
            && first.offset > 0.0
        {
            out.insert(0, peniko::ColorStop { offset: 0.0, ..first });
        }
        if let Some(last) = out.last().copied()
            && last.offset < 1.0
        {
            out.push(peniko::ColorStop { offset: 1.0, ..last });
        }
        while out.len() > 2 {
            let Some(before_last) = out.len().checked_sub(2).and_then(|i| out.get(i)) else {
                break;
            };
            if (before_last.offset - 1.0).abs() < 1e-6 {
                let index = out.len().saturating_sub(2);
                out.remove(index);
            } else {
                break;
            }
        }
        ColorStops(out.into_iter().collect())
    }

    /// Sets the canvas paint for `brush`. `paint_transform` maps the brush's font-unit space
    /// into the space of the path about to be filled. Returns false when nothing should draw.
    fn set_brush(&mut self, brush: &Brush<'_>, paint_transform: Affine) -> bool {
        self.ctx.set_paint_transform(paint_transform);
        match *brush {
            Brush::Solid { palette_index, alpha } => {
                self.ctx.set_paint(self.color(palette_index, alpha));
            }
            Brush::LinearGradient {
                p0,
                p1,
                color_stops,
                extend,
            } => {
                let stops = self.stops(color_stops);
                if !self.set_gradient_or_solid(
                    stops,
                    extend,
                    LinearGradientPosition {
                        start: point(p0),
                        end: point(p1),
                    }
                    .into(),
                ) {
                    return false;
                }
            }
            Brush::RadialGradient {
                c0,
                r0,
                c1,
                r1,
                color_stops,
                extend,
            } => {
                let stops = self.stops(color_stops);
                if r0 < 0.0 || r1 < 0.0 {
                    return false;
                }
                let kind = RadialGradientPosition {
                    start_center: point(c0),
                    start_radius: r0,
                    end_center: point(c1),
                    end_radius: r1,
                };
                if !self.set_gradient_or_solid(stops, extend, kind.into()) {
                    return false;
                }
            }
            Brush::SweepGradient {
                c0,
                start_angle,
                end_angle,
                color_stops,
                extend,
            } => {
                let stops = self.stops(color_stops);
                // skrifa hands over clockwise angles in degrees with start < end; peniko measures
                // clockwise in a y-down space, so the gradient is built mirrored in y and the
                // mirror is undone by the paint transform. A zero sector (only possible with pad)
                // is widened a hair so the shader has a direction to work with.
                let end_angle = if end_angle <= start_angle {
                    start_angle + 0.01
                } else {
                    end_angle
                };
                let kind = SweepGradientPosition {
                    center: Point::new(f64::from(c0.x), -f64::from(c0.y)),
                    start_angle: start_angle.to_radians(),
                    end_angle: end_angle.to_radians(),
                };
                self.ctx
                    .set_paint_transform(paint_transform.pre_scale_non_uniform(1.0, -1.0));
                if !self.set_gradient_or_solid(stops, extend, kind.into()) {
                    return false;
                }
            }
        }
        true
    }

    fn set_gradient_or_solid(
        &mut self,
        stops: ColorStops,
        extend: skrifa::color::Extend,
        kind: peniko::GradientKind,
    ) -> bool {
        match stops.0.as_slice() {
            [] => false,
            [only] => {
                self.ctx.set_paint(only.color.to_alpha_color::<Srgb>());
                true
            }
            _ => {
                let extend = match extend {
                    skrifa::color::Extend::Repeat => peniko::Extend::Repeat,
                    skrifa::color::Extend::Reflect => peniko::Extend::Reflect,
                    skrifa::color::Extend::Pad | skrifa::color::Extend::Unknown => peniko::Extend::Pad,
                };
                self.ctx.set_paint(Gradient {
                    kind,
                    stops,
                    extend,
                    ..Gradient::default()
                });
                true
            }
        }
    }
}

impl ColorPainter for ColrPainter<'_, '_> {
    fn push_transform(&mut self, transform: Transform) {
        let next = compose(self.current(), affine(transform));
        self.transforms.push(next);
    }

    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        // A glyph without an outline clips everything away, as an empty path does.
        self.ctx.set_transform(self.current());
        let path = self.paths.get(self.outlines, self.location, glyph_id);
        self.ctx.push_clip_path(path.unwrap_or(&BezPath::new()));
        self.stack.push(Open::Clip);
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        self.ctx.set_transform(self.current());
        self.ctx.push_clip_path(&rect_from_box(clip_box).to_path(0.1));
        self.stack.push(Open::Clip);
    }

    fn pop_clip(&mut self) {
        if self.pop_open(Open::Clip) {
            self.ctx.pop_clip();
        }
    }

    fn fill(&mut self, brush: Brush<'_>) {
        // Fill the whole canvas; the open clips decide what shows. The path is in canvas pixels,
        // so the brush's own space is reached through the paint transform alone.
        self.ctx.set_transform(Affine::IDENTITY);
        if self.set_brush(&brush, self.current()) {
            self.ctx.fill_rect(&self.canvas);
        }
    }

    fn fill_glyph(&mut self, glyph_id: GlyphId, brush_transform: Option<Transform>, brush: Brush<'_>) {
        // The common `PaintGlyph(fill)` case fills the outline directly instead of clipping a
        // canvas-sized rectangle, which is both faster and gives the edge single-pass coverage.
        self.ctx.set_transform(self.current());
        if self.set_brush(&brush, brush_transform.map_or(Affine::IDENTITY, affine))
            && let Some(path) = self.paths.get(self.outlines, self.location, glyph_id)
        {
            self.ctx.fill_path(path);
        }
    }

    fn push_layer(&mut self, composite_mode: CompositeMode) {
        self.ctx.push_blend_layer(blend_mode(composite_mode));
        self.stack.push(Open::Layer);
    }

    fn pop_layer(&mut self) {
        if self.pop_open(Open::Layer) {
            self.ctx.pop_layer();
        }
    }
}

/// Measures what a paint graph draws, in pen pixels, for glyphs without a clip box: every fill
/// contributes its outline (or the clip it fills) intersected with the clips open around it.
struct BoundsProbe<'p, 'f> {
    outlines: &'p OutlineGlyphCollection<'f>,
    location: LocationRef<'p>,
    paths: &'p mut PathCache,
    transforms: Vec<Affine>,
    clips: Vec<Rect>,
    bounds: Option<Rect>,
}

impl BoundsProbe<'_, '_> {
    fn current(&self) -> Affine {
        self.transforms.last().copied().unwrap_or(Affine::IDENTITY)
    }

    fn glyph_rect(&mut self, glyph_id: GlyphId) -> Rect {
        let current = self.current();
        self.paths
            .get(self.outlines, self.location, glyph_id)
            .filter(|path| !path.is_empty())
            .map_or(Rect::ZERO, |path| current.transform_rect_bbox(path.bounding_box()))
    }

    fn clip_to(&self, rect: Rect) -> Rect {
        self.clips.last().map_or(rect, |clip| clip.intersect(rect))
    }

    fn add(&mut self, rect: Rect) {
        if rect.area() > 0.0 {
            self.bounds = Some(self.bounds.map_or(rect, |bounds| bounds.union(rect)));
        }
    }
}

impl ColorPainter for BoundsProbe<'_, '_> {
    fn push_transform(&mut self, transform: Transform) {
        let next = compose(self.current(), affine(transform));
        self.transforms.push(next);
    }

    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let rect = self.glyph_rect(glyph_id);
        let clipped = self.clip_to(rect);
        self.clips.push(clipped);
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        let rect = self.current().transform_rect_bbox(rect_from_box(clip_box));
        let clipped = self.clip_to(rect);
        self.clips.push(clipped);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn fill(&mut self, _brush: Brush<'_>) {
        // An unclipped fill would cover the plane; COLR always clips fills (a PaintGlyph or a
        // clip box surrounds them), so only the clipped case adds bounds.
        if let Some(clip) = self.clips.last().copied() {
            self.add(clip);
        }
    }

    fn fill_glyph(&mut self, glyph_id: GlyphId, _brush_transform: Option<Transform>, _brush: Brush<'_>) {
        let rect = self.glyph_rect(glyph_id);
        let clipped = self.clip_to(rect);
        self.add(clipped);
    }

    fn push_layer(&mut self, _composite_mode: CompositeMode) {}
}

/// Records whether any brush reads the text colour.
struct ForegroundProbe {
    found: bool,
}

impl ForegroundProbe {
    fn see(&mut self, brush: &Brush<'_>) {
        let uses = match brush {
            Brush::Solid { palette_index, .. } => *palette_index == FOREGROUND_INDEX,
            Brush::LinearGradient { color_stops, .. }
            | Brush::RadialGradient { color_stops, .. }
            | Brush::SweepGradient { color_stops, .. } => {
                color_stops.iter().any(|stop| stop.palette_index == FOREGROUND_INDEX)
            }
        };
        self.found |= uses;
    }
}

impl ColorPainter for ForegroundProbe {
    fn push_transform(&mut self, _transform: Transform) {}
    fn pop_transform(&mut self) {}
    fn push_clip_glyph(&mut self, _glyph_id: GlyphId) {}
    fn push_clip_box(&mut self, _clip_box: BoundingBox<f32>) {}
    fn pop_clip(&mut self) {}

    fn fill(&mut self, brush: Brush<'_>) {
        self.see(&brush);
    }

    fn fill_glyph(&mut self, _glyph_id: GlyphId, _brush_transform: Option<Transform>, brush: Brush<'_>) {
        self.see(&brush);
    }

    fn push_layer(&mut self, _composite_mode: CompositeMode) {}
}

fn affine(t: Transform) -> Affine {
    Affine::new([
        f64::from(t.xx),
        f64::from(t.yx),
        f64::from(t.xy),
        f64::from(t.yy),
        f64::from(t.dx),
        f64::from(t.dy),
    ])
}

/// `outer * inner`: applies `inner` first. Spelled out because `Affine`'s `Mul` is an
/// operator the arithmetic lint cannot see through.
fn compose(outer: Affine, inner: Affine) -> Affine {
    let [a0, a1, a2, a3, a4, a5] = outer.as_coeffs();
    let [b0, b1, b2, b3, b4, b5] = inner.as_coeffs();
    Affine::new([
        a0 * b0 + a2 * b1,
        a1 * b0 + a3 * b1,
        a0 * b2 + a2 * b3,
        a1 * b2 + a3 * b3,
        a0 * b4 + a2 * b5 + a4,
        a1 * b4 + a3 * b5 + a5,
    ])
}

fn point(p: skrifa::raw::types::Point<f32>) -> Point {
    Point::new(f64::from(p.x), f64::from(p.y))
}

fn rect_from_box(b: BoundingBox<f32>) -> Rect {
    Rect::new(
        f64::from(b.x_min),
        f64::from(b.y_min),
        f64::from(b.x_max),
        f64::from(b.y_max),
    )
}

/// COLR composite modes map one to one onto Porter-Duff composes and separable/non-separable
/// mixes (the COLR spec defines them as the W3C compositing modes).
fn blend_mode(mode: CompositeMode) -> BlendMode {
    let (mix, compose) = match mode {
        CompositeMode::Clear => (Mix::Normal, Compose::Clear),
        CompositeMode::Src => (Mix::Normal, Compose::Copy),
        CompositeMode::Dest => (Mix::Normal, Compose::Dest),
        CompositeMode::SrcOver | CompositeMode::Unknown => (Mix::Normal, Compose::SrcOver),
        CompositeMode::DestOver => (Mix::Normal, Compose::DestOver),
        CompositeMode::SrcIn => (Mix::Normal, Compose::SrcIn),
        CompositeMode::DestIn => (Mix::Normal, Compose::DestIn),
        CompositeMode::SrcOut => (Mix::Normal, Compose::SrcOut),
        CompositeMode::DestOut => (Mix::Normal, Compose::DestOut),
        CompositeMode::SrcAtop => (Mix::Normal, Compose::SrcAtop),
        CompositeMode::DestAtop => (Mix::Normal, Compose::DestAtop),
        CompositeMode::Xor => (Mix::Normal, Compose::Xor),
        CompositeMode::Plus => (Mix::Normal, Compose::Plus),
        CompositeMode::Screen => (Mix::Screen, Compose::SrcOver),
        CompositeMode::Overlay => (Mix::Overlay, Compose::SrcOver),
        CompositeMode::Darken => (Mix::Darken, Compose::SrcOver),
        CompositeMode::Lighten => (Mix::Lighten, Compose::SrcOver),
        CompositeMode::ColorDodge => (Mix::ColorDodge, Compose::SrcOver),
        CompositeMode::ColorBurn => (Mix::ColorBurn, Compose::SrcOver),
        CompositeMode::HardLight => (Mix::HardLight, Compose::SrcOver),
        CompositeMode::SoftLight => (Mix::SoftLight, Compose::SrcOver),
        CompositeMode::Difference => (Mix::Difference, Compose::SrcOver),
        CompositeMode::Exclusion => (Mix::Exclusion, Compose::SrcOver),
        CompositeMode::Multiply => (Mix::Multiply, Compose::SrcOver),
        CompositeMode::HslHue => (Mix::Hue, Compose::SrcOver),
        CompositeMode::HslSaturation => (Mix::Saturation, Compose::SrcOver),
        CompositeMode::HslColor => (Mix::Color, Compose::SrcOver),
        CompositeMode::HslLuminosity => (Mix::Luminosity, Compose::SrcOver),
    };
    BlendMode::new(mix, compose)
}

// ---------------------------------------------------------------------------------------------
// sbix and CBDT
// ---------------------------------------------------------------------------------------------

/// Draws an embedded colour bitmap scaled to `ppem`.
///
/// Placement follows skrifa's `BitmapGlyph` contract (the one Skia's fontations backend uses):
/// outer bearings in font units, inner bearings in strike pixels, and the image's origin at its
/// bottom-left (sbix) or top-left (CBDT). Apple Color Emoji is the exception CoreText makes: its
/// image is dropped by an eighth of an em with no outer bearing, which is how Chrome on macOS
/// shows it.
///
/// The image's top edge is snapped to a whole pixel. Text is positioned on whole-pixel
/// baselines with horizontal subpixel offsets only, and a half-pixel vertical phase (an eighth
/// of 20 px is 2.5) would blur every emoji row for no gain. The horizontal position keeps its
/// subpixel phase so that colour glyphs advance with the text around them.
fn render_bitmap(
    font: &FontRef<'_>,
    strikes: &BitmapStrikes<'_>,
    glyph: GlyphId,
    ppem: f32,
    x_offset: f32,
) -> Option<ColorImage> {
    let bitmap = pick_strike(strikes, glyph, ppem)?;
    let image = decode_bitmap(&bitmap)?;
    if image.width == 0 || image.height == 0 {
        return Some(ColorImage::empty());
    }
    let upem = f32::from(font.head().ok()?.units_per_em());
    if upem <= 0.0 || bitmap.ppem_x <= 0.0 || bitmap.ppem_y <= 0.0 {
        return None;
    }
    let scale_x = ppem / bitmap.ppem_x;
    let scale_y = ppem / bitmap.ppem_y;
    let (outer_x, outer_y) = if strikes.format() == Some(BitmapFormat::Sbix) && is_apple_color_emoji(font) {
        (0.0, ppem / 8.0)
    } else {
        (bitmap.bearing_x * ppem / upem, -bitmap.bearing_y * ppem / upem)
    };
    let src_w = f32_from_u32(image.width);
    let src_h = f32_from_u32(image.height);
    let origin_shift = match bitmap.placement_origin {
        Origin::TopLeft => 0.0,
        Origin::BottomLeft => -src_h,
    };
    // Edges of the scaled image in pen pixels, y down.
    let x0 = x_offset + outer_x + scale_x * bitmap.inner_bearing_x;
    let y0 = (outer_y + scale_y * (origin_shift - bitmap.inner_bearing_y)).round();
    let x1 = x0 + scale_x * src_w;
    let y1 = y0 + scale_y * src_h;

    let left = floor_px(f64::from(x0) + EDGE_EPSILON);
    let right = ceil_px(f64::from(x1) - EDGE_EPSILON);
    let top_down = floor_px(f64::from(y0) + EDGE_EPSILON);
    let bottom = ceil_px(f64::from(y1) - EDGE_EPSILON);
    let (Some(width), Some(height)) = (canvas_edge(left, right), canvas_edge(top_down, bottom)) else {
        return Some(ColorImage::empty());
    };

    let columns = axis_weights(left, usize::from(width), x0, scale_x, image.width);
    let rows = axis_weights(top_down, usize::from(height), y0, scale_y, image.height);
    let rgba = resample(&image, &columns, &rows, usize::from(width))?;
    Some(ColorImage {
        left,
        top: top_down.checked_neg()?,
        width: u32::from(width),
        height: u32::from(height),
        rgba,
    })
}

/// The smallest strike at least as large as `ppem` that has the glyph, else the largest one.
/// Scaling down from the next size up keeps all of the artwork's detail; scaling up loses none
/// that a smaller strike would have had.
fn pick_strike<'a>(strikes: &BitmapStrikes<'a>, glyph: GlyphId, ppem: f32) -> Option<BitmapGlyph<'a>> {
    let mut above: Option<(f32, BitmapGlyph<'a>)> = None;
    let mut largest: Option<(f32, BitmapGlyph<'a>)> = None;
    for strike in strikes.iter() {
        let size = strike.ppem();
        let Some(bitmap) = strike.get(glyph).filter(is_color_bitmap) else {
            continue;
        };
        if size >= ppem && above.as_ref().is_none_or(|(best, _)| size < *best) {
            above = Some((size, bitmap.clone()));
        }
        if largest.as_ref().is_none_or(|(best, _)| size > *best) {
            largest = Some((size, bitmap));
        }
    }
    above.or(largest).map(|(_, bitmap)| bitmap)
}

/// Apple Color Emoji, the font CoreText places with its fixed downward shift. Both faces of the
/// system collection (`AppleColorEmoji` and the UI variant `.AppleColorEmojiUI`) qualify.
fn is_apple_color_emoji(font: &FontRef<'_>) -> bool {
    font.localized_strings(StringId::POSTSCRIPT_NAME).any(|name| {
        let name: String = name.chars().collect();
        name.trim_start_matches('.').starts_with("AppleColorEmoji")
    })
}

/// A decoded bitmap: premultiplied RGBA as floats, one entry per pixel, row-major.
struct SourceImage {
    width: u32,
    height: u32,
    pixels: Vec<[f32; 4]>,
}

fn decode_bitmap(bitmap: &BitmapGlyph<'_>) -> Option<SourceImage> {
    match bitmap.data {
        BitmapData::Png(data) => decode_png(data),
        BitmapData::Bgra(data) => {
            // CBDT's raw 32-bit format is already premultiplied BGRA.
            let pixels: Vec<[f32; 4]> = data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&[b, g, r, a]| [f32::from(r), f32::from(g), f32::from(b), f32::from(a)])
                .collect();
            let count = usize::try_from(bitmap.width)
                .ok()?
                .checked_mul(usize::try_from(bitmap.height).ok()?)?;
            (pixels.len() >= count).then(|| SourceImage {
                width: bitmap.width,
                height: bitmap.height,
                pixels: pixels.into_iter().take(count).collect(),
            })
        }
        BitmapData::Mask(_) => None,
    }
}

/// Decodes PNG strike data to premultiplied RGBA. Strikes are sRGB artwork; no colour
/// management or gamma chunk handling is applied, as browsers do not apply it to emoji either.
fn decode_png(data: &[u8]) -> Option<SourceImage> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0_u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    let width = usize::try_from(info.width).ok()?;
    let row_bytes = width.checked_mul(channels)?;
    let mut pixels = Vec::with_capacity(width.checked_mul(usize::try_from(info.height).ok()?)?);
    for row in buf.chunks(info.line_size).take(usize::try_from(info.height).ok()?) {
        for px in row.get(..row_bytes)?.chunks_exact(channels) {
            let [r, g, b, a] = match *px {
                [l] => [l, l, l, u8::MAX],
                [l, a] => [l, l, l, a],
                [r, g, b] => [r, g, b, u8::MAX],
                [r, g, b, a] => [r, g, b, a],
                _ => return None,
            };
            let alpha = f32::from(a) / 255.0;
            pixels.push([
                f32::from(r) * alpha,
                f32::from(g) * alpha,
                f32::from(b) * alpha,
                f32::from(a),
            ]);
        }
    }
    Some(SourceImage {
        width: info.width,
        height: info.height,
        pixels,
    })
}

/// For each destination pixel along one axis, the source pixels it draws from and their weights.
///
/// `dst_start` is the absolute pixel coordinate of the first destination pixel, `origin` the
/// absolute coordinate of the source image's leading edge, `scale` destination pixels per
/// source pixel. Weights do not renormalize at the image edge: the area outside the image is
/// transparent, which is what antialiases the image's border at a fractional position.
///
/// Downscaling uses an exact box (area-average) filter: each destination pixel is the mean of
/// the source area it covers. Strike selection picks the next size up, so the reduction is at
/// most the gap between strikes (under 2:1 for Apple Color Emoji and for Noto at text sizes),
/// where a box filter is the alias-free reduction Skia's mipmapped sampling approximates, without
/// the extra softening of blending in a half-resolution mip level, and it reproduces the strike
/// exactly at 1:1. Upscaling (above the largest strike) uses a tent (bilinear) filter, as a box
/// would show the source pixels as blocks.
fn axis_weights(dst_start: i32, dst_len: usize, origin: f32, scale: f32, src_len: u32) -> Vec<Vec<(usize, f32)>> {
    let src_len_f = f32_from_u32(src_len);
    let mut out = Vec::with_capacity(dst_len);
    let mut position = f32_from_i32(dst_start);
    for _ in 0..dst_len {
        let mut taps = Vec::new();
        if scale <= 1.0 {
            let a = ((position - origin) / scale).max(0.0);
            let b = ((position + 1.0 - origin) / scale).min(src_len_f);
            let mut i = a.floor();
            while i < b {
                let overlap = b.min(i + 1.0) - a.max(i);
                if overlap > 0.0
                    && let Some(index) = index_from_f32(i)
                {
                    taps.push((index, overlap * scale));
                }
                i += 1.0;
            }
        } else {
            let center = (position + 0.5 - origin) / scale - 0.5;
            let base = center.floor();
            for i in [base, base + 1.0] {
                let weight = 1.0 - (center - i).abs();
                if weight > 0.0
                    && i >= 0.0
                    && i < src_len_f
                    && let Some(index) = index_from_f32(i)
                {
                    taps.push((index, weight));
                }
            }
        }
        out.push(taps);
        position += 1.0;
    }
    out
}

/// Separable resampling of premultiplied pixels: rows first, then columns.
fn resample(
    image: &SourceImage,
    columns: &[Vec<(usize, f32)>],
    rows: &[Vec<(usize, f32)>],
    dst_width: usize,
) -> Option<Vec<u8>> {
    let src_width = usize::try_from(image.width).ok()?;
    let horizontal: Vec<Vec<[f32; 4]>> = image
        .pixels
        .chunks_exact(src_width)
        .map(|row| columns.iter().map(|taps| weighted_sum(taps, |i| row.get(i))).collect())
        .collect();
    let mut out = Vec::with_capacity(dst_width.checked_mul(rows.len())?.checked_mul(4)?);
    for taps in rows {
        for x in 0..dst_width {
            let [r, g, b, a] = weighted_sum(taps, |y| horizontal.get(y).and_then(|row| row.get(x)));
            let alpha = channel(a);
            out.extend_from_slice(&[
                channel(r).min(alpha),
                channel(g).min(alpha),
                channel(b).min(alpha),
                alpha,
            ]);
        }
    }
    Some(out)
}

fn weighted_sum<'p>(taps: &[(usize, f32)], pixel: impl Fn(usize) -> Option<&'p [f32; 4]>) -> [f32; 4] {
    let mut sum = [0.0_f32; 4];
    for &(index, weight) in taps {
        if let Some(px) = pixel(index) {
            for (acc, value) in sum.iter_mut().zip(px) {
                *acc += value * weight;
            }
        }
    }
    sum
}

/// Rounds an accumulated channel to a byte.
#[expect(clippy::as_conversions, reason = "saturating float to pixel")]
fn channel(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

#[expect(clippy::as_conversions, reason = "saturating float to pixel")]
fn floor_px(value: f64) -> i32 {
    value.floor() as i32
}

#[expect(clippy::as_conversions, reason = "saturating float to pixel")]
fn ceil_px(value: f64) -> i32 {
    value.ceil() as i32
}

/// A non-negative whole float (a source pixel index) as an index.
#[expect(clippy::as_conversions, reason = "saturating float to pixel")]
fn index_from_f32(value: f32) -> Option<usize> {
    (value >= 0.0 && value.is_finite()).then_some(value as usize)
}

/// Image dimensions and pixel coordinates are far below 2^24, where `f32` stops being exact.
#[expect(
    clippy::as_conversions,
    reason = "pixel coordinates are small enough to be exact in f32"
)]
fn f32_from_u32(value: u32) -> f32 {
    value as f32
}

#[expect(
    clippy::as_conversions,
    reason = "pixel coordinates are small enough to be exact in f32"
)]
fn f32_from_i32(value: i32) -> f32 {
    value as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTO_COLRV1: &[u8] = include_bytes!("../../assets/test-fonts/NotoColorEmoji-COLRv1-Subset.ttf");
    const NOTO_CBDT: &[u8] = include_bytes!("../../assets/test-fonts/NotoColorEmoji-CBTF-Subset.ttf");
    const TWEMOJI_COLRV0: &[u8] = include_bytes!("../../assets/test-fonts/Twemoji.Mozilla-Subset.ttf");
    const COLR_TEST_GLYPHS: &[u8] = include_bytes!("../../assets/test-fonts/test_glyphs-glyf_colr_1.ttf");
    const APPLE_EMOJI: &str = "/System/Library/Fonts/Apple Color Emoji.ttc";
    const ARIAL: &str = "/System/Library/Fonts/Supplemental/Arial.ttf";
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    fn gid(font: &FontRef<'_>, ch: char) -> GlyphId {
        font.charmap().map(ch).unwrap_or_else(|| panic!("no glyph for {ch:?}"))
    }

    fn pixels(image: &ColorImage) -> &[[u8; 4]] {
        image.rgba.as_chunks::<4>().0
    }

    fn pixel(image: &ColorImage, x: u32, y: u32) -> [u8; 4] {
        let row = usize::try_from(y).unwrap();
        let column = usize::try_from(x).unwrap();
        let width = usize::try_from(image.width).unwrap();
        pixels(image).chunks_exact(width).nth(row).unwrap()[column]
    }

    fn count(n: usize) -> f64 {
        f64::from(u32::try_from(n).unwrap())
    }

    /// Opaque pixels whose channels spread by more than `spread`, i.e. clearly coloured.
    fn colourful_pixels(image: &ColorImage, spread: u8) -> usize {
        pixels(image)
            .iter()
            .filter(|px| {
                let (max, min) = (px[..3].iter().max().unwrap(), px[..3].iter().min().unwrap());
                px[3] > 200 && max.abs_diff(*min) > spread
            })
            .count()
    }

    fn assert_plausible_emoji(image: &ColorImage, ppem: f64) {
        let (w, h) = (f64::from(image.width), f64::from(image.height));
        assert_eq!(count(image.rgba.len()), w * h * 4.0);
        assert!(w >= ppem * 0.8 && w <= ppem * 1.5, "width {w} at {ppem}");
        assert!(h >= ppem * 0.8 && h <= ppem * 1.5, "height {h} at {ppem}");
        let opaque = pixels(image).iter().filter(|px| px[3] > 200).count();
        assert!(count(opaque) > w * h * 0.3, "too few opaque pixels: {opaque}");
        assert!(count(colourful_pixels(image, 60)) > w * h * 0.1, "image is grey");
        for px in pixels(image) {
            assert!(
                px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3],
                "not premultiplied: {px:?}"
            );
        }
    }

    /// Writes the image over white (and a 4x nearest-neighbour enlargement) into the temp dir, for eyeballing.
    fn write_png(name: &str, image: &ColorImage) {
        if image.width == 0 {
            return;
        }
        let dir = std::env::temp_dir().join("gui-color-glyph");
        std::fs::create_dir_all(&dir).unwrap();
        let zoom = 4_u32;
        let (w, h) = (
            image.width.checked_mul(zoom).unwrap(),
            image.height.checked_mul(zoom).unwrap(),
        );
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let [r, g, b, a] = pixel(image, x.checked_div(zoom).unwrap(), y.checked_div(zoom).unwrap());
                let over = |c: u8| c.saturating_add(u8::MAX.saturating_sub(a));
                data.extend_from_slice(&[over(r), over(g), over(b), u8::MAX]);
            }
        }
        let file = std::fs::File::create(dir.join(format!("{name}.png"))).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&data).unwrap();
    }

    #[test]
    fn colrv1_emoji_at_text_sizes() {
        let font = FontRef::new(NOTO_COLRV1).unwrap();
        let grin = gid(&font, '\u{1F600}');
        assert!(has_color(&font, grin));
        assert!(!uses_foreground(&font, grin));
        for ppem in [16.0, 32.0, 64.0] {
            let image = render(&font, grin, ppem, &[], 0.0, BLACK).unwrap();
            assert_plausible_emoji(&image, f64::from(ppem));
            // Noto's emoji sit on the baseline and descend a little below it.
            assert!(
                image.top > 0 && image.top <= floor_px(f64::from(ppem) * 1.1),
                "top {}",
                image.top
            );
            write_png(&format!("colrv1_1F600_{ppem}"), &image);
        }
        for (ch, name) in [('\u{1F308}', "rainbow"), ('\u{1F525}', "fire"), ('\u{1F30D}', "globe")] {
            let image = render(&font, gid(&font, ch), 48.0, &[], 0.0, BLACK).unwrap();
            assert_plausible_emoji(&image, 48.0);
            write_png(&format!("colrv1_{name}_48"), &image);
        }
    }

    #[test]
    fn subpixel_offset_moves_colrv1_content() {
        let font = FontRef::new(NOTO_COLRV1).unwrap();
        let grin = gid(&font, '\u{1F600}');
        let a = render(&font, grin, 32.0, &[], 0.0, BLACK).unwrap();
        let b = render(&font, grin, 32.0, &[], 0.5, BLACK).unwrap();
        assert!(a.left <= b.left && b.left <= a.left.checked_add(1).unwrap());
        assert_ne!(a.rgba, b.rgba);
    }

    /// `sweep_0_360_pad_narrow`: a circle centred at (500, 600) in a 1000-unit em, filled with a
    /// sweep from 0 to 360 degrees counter-clockwise whose stops are linen at 90 degrees, blue at
    /// 150, red at 210 and dark slate grey at 270, padded outside that range.
    #[test]
    fn sweep_gradient_pixels() {
        let font = FontRef::new(COLR_TEST_GLYPHS).unwrap();
        let glyph = gid(&font, '\u{F0200}');
        let ppem = 100.0;
        let image = render(&font, glyph, ppem, &[], 0.0, BLACK).unwrap();
        write_png("sweep_0_360_pad", &image);
        // The clip box is the whole em: (0, 0)..(1000, 1000) font units.
        assert_eq!((image.left, image.top, image.width, image.height), (0, 100, 100, 100));
        let sample = |degrees: f64| {
            let (sin, cos) = degrees.to_radians().sin_cos();
            // 250 units from the centre, then font units to image pixels.
            let fx = 500.0 + 250.0 * cos;
            let fy = 600.0 + 250.0 * sin;
            let px = u32::try_from(floor_px(fx * 0.1)).unwrap();
            let py = u32::try_from(floor_px(100.0 - fy * 0.1)).unwrap();
            pixel(&image, px, py)
        };
        let close = |got: [u8; 4], want: [u8; 4], tol: u8| got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= tol);
        // Padded with the first stop before 90 degrees, the last after 270.
        assert!(
            close(sample(45.0), [0xFA, 0xF0, 0xE6, 255], 3),
            "45: {:?}",
            sample(45.0)
        );
        assert!(
            close(sample(315.0), [0x2F, 0x4F, 0x4F, 255], 3),
            "315: {:?}",
            sample(315.0)
        );
        assert!(close(sample(150.0), [0, 0, 255, 255], 12), "150: {:?}", sample(150.0));
        assert!(close(sample(210.0), [255, 0, 0, 255], 12), "210: {:?}", sample(210.0));
        // Halfway between blue and red.
        assert!(close(sample(180.0), [128, 0, 128, 255], 12), "180: {:?}", sample(180.0));
        // Counter-clockwise: 120 degrees lies between linen and blue, so it has blue and red but
        // more blue than red; 240 the mirror image towards slate.
        let at_120 = sample(120.0);
        assert!(at_120[2] > at_120[1] && at_120[0] > 100, "120: {at_120:?}");
        let at_240 = sample(240.0);
        assert!(at_240[0] > at_240[2] && at_240[1] > 20, "240: {at_240:?}");
        // Outside the circle is transparent.
        assert_eq!(pixel(&image, 1, 1)[3], 0);
    }

    #[test]
    fn colrv1_paint_kinds_render() {
        let font = FontRef::new(COLR_TEST_GLYPHS).unwrap();
        for (cp, name) in [
            (0xF0100, "linear_repeat"),
            (0xF020C, "sweep_reflect"),
            (0xF0218, "sweep_repeat"),
            (0xF0504, "radial_repeat"),
            (0xF0506, "radial_two_point_pad"),
            (0xF0300, "scale"),
            (0xF0A05, "composite_src_in"),
            (0xF0A0D, "composite_screen"),
            (0xF0A18, "composite_hue"),
        ] {
            let glyph = gid(&font, char::from_u32(cp).unwrap());
            let image = render(&font, glyph, 64.0, &[], 0.0, BLACK).unwrap();
            assert!(image.width > 0 && image.height > 0, "{name}");
            assert!(pixels(&image).iter().any(|px| px[3] > 0), "{name} drew nothing");
            write_png(&format!("test_glyph_{name}"), &image);
        }
    }

    #[test]
    fn foreground_colour_is_used_and_reported() {
        let font = FontRef::new(COLR_TEST_GLYPHS).unwrap();
        let glyph = gid(&font, '\u{F0B04}');
        assert!(uses_foreground(&font, glyph));
        let green = render(&font, glyph, 64.0, &[], 0.0, [0, 200, 0, 255]).unwrap();
        let blue = render(&font, glyph, 64.0, &[], 0.0, [0, 0, 200, 255]).unwrap();
        assert_ne!(green.rgba, blue.rgba);
        write_png("foreground_sweep_green", &green);
        assert!(!uses_foreground(&font, gid(&font, '\u{F0200}')));
    }

    #[test]
    fn colrv0_layers() {
        let font = FontRef::new(TWEMOJI_COLRV0).unwrap();
        let eyes = gid(&font, '\u{1F440}');
        assert!(has_color(&font, eyes));
        let image = render(&font, eyes, 32.0, &[], 0.0, BLACK).unwrap();
        write_png("colrv0_eyes_32", &image);
        assert!(image.width >= 20 && image.height >= 10);
        assert!(pixels(&image).iter().any(|px| px[3] == 255));
    }

    #[test]
    fn cbdt_bitmap() {
        let font = FontRef::new(NOTO_CBDT).unwrap();
        let eyes = gid(&font, '\u{1F440}');
        assert!(has_color(&font, eyes));
        assert!(!uses_foreground(&font, eyes));
        for ppem in [16.0, 32.0, 64.0] {
            let image = render(&font, eyes, ppem, &[], 0.0, BLACK).unwrap();
            let (width, ppem) = (f64::from(image.width), f64::from(ppem));
            assert!(width <= ppem * 1.4 && width >= ppem * 0.9, "{width}");
            assert!(image.top > 0 && image.top <= ceil_px(ppem), "top {}", image.top);
            write_png(&format!("cbdt_eyes_{ppem}"), &image);
        }
    }

    #[test]
    fn sbix_apple_color_emoji() {
        let Ok(data) = std::fs::read(APPLE_EMOJI) else {
            eprintln!("skipped: {APPLE_EMOJI} is not present");
            return;
        };
        let font = FontRef::from_index(&data, 0).unwrap();
        let grin = gid(&font, '\u{1F600}');
        assert!(has_color(&font, grin));
        assert!(!uses_foreground(&font, grin));
        for ppem in [16.0, 20.0, 32.0, 33.0, 64.0, 200.0] {
            let image = render(&font, grin, ppem, &[], 0.0, BLACK).unwrap();
            assert_plausible_emoji(&image, f64::from(ppem));
            // CoreText drops the em square by an eighth: it spans 7/8 em above to 1/8 below.
            let expected_top = floor_px((f64::from(ppem) * 7.0 / 8.0).round());
            assert!(image.top.abs_diff(expected_top) <= 1, "top {} at {ppem}", image.top);
            assert!(image.left.unsigned_abs() <= 1, "left {} at {ppem}", image.left);
            write_png(&format!("sbix_1F600_{ppem}"), &image);
        }
        // At a strike's own size with no offset, the strike is reproduced exactly.
        let exact = render(&font, grin, 32.0, &[], 0.0, BLACK).unwrap();
        assert_eq!(exact.width, 32);
        for (cp, name) in [(0x1F308, "rainbow"), (0x1F525, "fire"), (0x1F44D, "thumbs")] {
            let image = render(&font, gid(&font, char::from_u32(cp).unwrap()), 24.0, &[], 0.0, BLACK).unwrap();
            write_png(&format!("sbix_{name}_24"), &image);
        }
    }

    #[test]
    fn plain_outline_glyph_has_no_colour() {
        let Ok(data) = std::fs::read(ARIAL) else {
            eprintln!("skipped: {ARIAL} is not present");
            return;
        };
        let font = FontRef::new(&data).unwrap();
        let a = gid(&font, 'A');
        assert!(!has_color(&font, a));
        assert!(!uses_foreground(&font, a));
        assert!(render(&font, a, 16.0, &[], 0.0, BLACK).is_none());
    }

    #[test]
    fn box_filter_is_exact_at_one_to_one_and_averages_at_half() {
        let image = SourceImage {
            width: 2,
            height: 1,
            pixels: vec![[255.0, 0.0, 0.0, 255.0], [0.0, 0.0, 255.0, 255.0]],
        };
        let rows = axis_weights(0, 1, 0.0, 1.0, 1);
        let same = axis_weights(0, 2, 0.0, 1.0, 2);
        assert_eq!(
            resample(&image, &same, &rows, 2).unwrap(),
            vec![255, 0, 0, 255, 0, 0, 255, 255]
        );
        let half = axis_weights(0, 1, 0.0, 0.5, 2);
        assert_eq!(resample(&image, &half, &rows, 1).unwrap(), vec![128, 0, 128, 255]);
        // Half a pixel to the right: the left edge pixel is half covered.
        let shifted = axis_weights(0, 3, 0.5, 1.0, 2);
        assert_eq!(
            resample(&image, &shifted, &rows, 3).unwrap(),
            vec![128, 0, 0, 128, 128, 0, 128, 255, 0, 0, 128, 128]
        );
    }
}
