//! The CPU renderer: a [`Scene`] painted into a tiny-skia pixmap. It runs
//! headless (scenarios, the verdict, CI with no GPU) and where a window finds
//! no GPU; the GPU renderer ([`crate::gpu`]) is held to its pixels.

use std::collections::HashMap;

use cosmic_text::{CacheKey, SwashContent};
use tiny_skia::{
    BlendMode, FillRule, GradientStop, IntRect, Mask, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Point,
    PremultipliedColorU8, Rect, Shader, SpreadMode, Transform,
};

use crate::cache::FrameCache;
use crate::color::Color;
use crate::convert::{f32_to_i32, f32_to_u32, i32_to_f32, u32_to_f32, unit_to_u8, usize_to_f32};
use crate::geometry::Bounds;
use crate::icon::IconCache;
use crate::inspect::Raster;
use crate::paint::grow;
use crate::scene::{Clip, Gradient, Group, Held, Item, PixelRect, Renderer, Scene};
use crate::shape::{PathCommand, Shape};
use crate::style::{Corners, Filter, Shadow};
use crate::text::{TextSystem, physical, physical_pos};

const CUBIC_ARC: f32 = 0.552_284_8;

/// Colour stops added between two stops, so tiny-skia's sRGB interpolation
/// follows the OKLab one CSS asks for.
const OKLAB_STEPS: u8 = 8;

/// Rasters kept across frames.
#[derive(Default)]
pub struct CpuRenderer {
    glyphs: FrameCache<GlyphKey, Option<GlyphImage>>,
    icons: IconCache,
    shadows: FrameCache<ShadowKey, Option<ShadowImage>>,
    /// Images scaled to the size they are drawn at.
    images: FrameCache<ImageKey, Option<ScaledImage>>,
    /// Window-sized pixmaps groups paint aside into, kept for the next
    /// frame: as many as the last two frames had in use at once.
    layers: Vec<Pixmap>,
    /// Layers in use now (a group inside a group holds two).
    layers_out: usize,
    /// The most in use at once this frame, and in the frame before.
    layers_peak: (usize, usize),
}

impl CpuRenderer {
    /// Paint `scene` into `pixmap`, which is `scene.size`.
    pub fn render(&mut self, scene: &Scene, text: &mut TextSystem, pixmap: &mut Pixmap) {
        if let Some(bg) = scene.background {
            pixmap.fill(bg.to_skia());
        }
        let mut clips = Clips {
            masks: HashMap::new(),
            size: (pixmap.width(), pixmap.height()),
            scale: scene.scale,
        };
        self.items(&scene.items, pixmap, text, &mut clips);
    }

    /// `scene` painted into a new pixmap.
    pub fn capture(&mut self, scene: &Scene, text: &mut TextSystem) -> Option<Pixmap> {
        let mut pixmap = Pixmap::new(scene.size.0, scene.size.1)?;
        self.render(scene, text, &mut pixmap);
        Some(pixmap)
    }

    /// Forget what the last frames did not draw; call once per frame.
    pub fn end_frame(&mut self) {
        self.glyphs.end_frame();
        self.icons.end_frame();
        self.shadows.end_frame();
        self.images.end_frame();
        // Keep the layers the last two frames needed at once, as the caches
        // keep what the last two frames drew: a dialog's nested groups do not
        // hold window-sized pixmaps after it has gone.
        let (now, before) = self.layers_peak;
        self.layers.truncate(now.max(before));
        self.layers_peak = (0, now);
    }

    #[must_use]
    pub fn held(&self) -> Held {
        Held {
            glyphs: Raster::of(self.glyphs.values().flatten().map(|g| &g.pixmap)),
            icons: self.icons.footprint(),
            shadows: Raster::of(
                self.shadows
                    .values()
                    .flatten()
                    .flat_map(|s| s.parts.iter().map(|(_, _, p)| p)),
            ),
            images: Raster::of(self.images.values().flatten().map(|i| &i.pixels)),
            layers: Raster::of(&self.layers),
        }
    }

    fn items(&mut self, items: &[Item], pixmap: &mut Pixmap, text: &mut TextSystem, clips: &mut Clips) {
        for item in items {
            self.item(item, pixmap, text, clips);
        }
    }

    #[expect(clippy::too_many_lines, reason = "one arm per primitive")]
    fn item(&mut self, item: &Item, pixmap: &mut Pixmap, text: &mut TextSystem, clips: &mut Clips) {
        let scale = clips.scale;
        let to_device = Transform::from_scale(scale, scale);
        match item {
            Item::Rect { bounds, color, clip } => {
                if let Some(rect) = Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height) {
                    let mut paint = Paint::default();
                    paint.set_color(color.to_skia());
                    pixmap.fill_rect(rect, &paint, to_device, clips.get(*clip));
                }
            }
            Item::RoundedRect {
                bounds,
                radius,
                color,
                clip,
            } => {
                if let Some(path) = rrect(*bounds, *radius) {
                    pixmap.fill_path(&path, &solid(*color), FillRule::Winding, to_device, clips.get(*clip));
                }
            }
            Item::Band {
                outer,
                outer_radius,
                inner,
                inner_radius,
                color,
                clip,
            } => {
                if let Some(path) = band(*outer, *outer_radius, *inner, *inner_radius) {
                    pixmap.fill_path(&path, &solid(*color), FillRule::EvenOdd, to_device, clips.get(*clip));
                }
            }
            Item::Shadow {
                bounds,
                radius,
                shadow,
                clip,
            } => self.shadow(pixmap, scale, *bounds, *radius, shadow, clips.get(*clip)),
            Item::Gradient {
                bounds,
                radius,
                gradient,
                clip,
            } => {
                if let Some(shader) = shader(gradient)
                    && let Some(path) = rrect(*bounds, *radius)
                {
                    pixmap.fill_path(
                        &path,
                        &shader_paint(shader),
                        FillRule::Winding,
                        to_device,
                        clips.get(*clip),
                    );
                }
            }
            Item::Glyph { key, x, y, color, clip } => {
                let glyph = GlyphKey {
                    glyph: *key,
                    rgba: color.to_rgba8(),
                };
                let image = self
                    .glyphs
                    .get_or_insert_with(glyph, || glyph_image(text, *key, *color));
                if let Some(g) = image {
                    pixmap.draw_pixmap(
                        x.saturating_add(g.left),
                        y.saturating_sub(g.top),
                        g.pixmap.as_ref(),
                        &PixmapPaint::default(),
                        Transform::identity(),
                        clips.get(*clip),
                    );
                }
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
                if let Some(img) = self.icons.get(*icon, *size, *color, *stroke_width) {
                    pixmap.draw_pixmap(
                        *x,
                        *y,
                        img.as_ref(),
                        &PixmapPaint::default(),
                        Transform::identity(),
                        clips.get(*clip),
                    );
                }
            }
            Item::Image {
                source,
                bounds,
                radius,
                clip,
            } => self.image(pixmap, scale, *bounds, source, *radius, clips.get(*clip)),
            Item::Shapes { origin, shapes, clip } => {
                paint_shapes(
                    pixmap,
                    to_device.pre_translate(origin.0, origin.1),
                    shapes,
                    clips.get(*clip),
                );
            }
            Item::TextThrough {
                region,
                mask,
                background,
                fill,
                clip,
            } => self.text_through(pixmap, text, clips, *region, mask, *background, fill.as_ref(), *clip),
            Item::Group(group) => self.group(pixmap, text, clips, group),
        }
    }

    /// A pooled window-sized layer, `region` of it cleared; give it back
    /// with [`Self::give_back`].
    fn layer(&mut self, size: (u32, u32), region: IntRect) -> Option<Pixmap> {
        self.layers.retain(|l| (l.width(), l.height()) == size);
        let mut layer = self.layers.pop().or_else(|| Pixmap::new(size.0, size.1))?;
        self.layers_out = self.layers_out.saturating_add(1);
        self.layers_peak.0 = self.layers_peak.0.max(self.layers_out);
        let paint = Paint {
            blend_mode: BlendMode::Clear,
            ..Paint::default()
        };
        layer.fill_rect(region.to_rect(), &paint, Transform::identity(), None);
        Some(layer)
    }

    fn give_back(&mut self, layer: Pixmap) {
        self.layers.push(layer);
        self.layers_out = self.layers_out.saturating_sub(1);
    }

    /// Paint `group`'s items aside, then mask, filter, transform and
    /// composite them onto `pixmap`.
    fn group(&mut self, pixmap: &mut Pixmap, text: &mut TextSystem, clips: &mut Clips, group: &Group) {
        let size = (pixmap.width(), pixmap.height());
        let Some(region) = int_rect(group.region) else { return };
        let Some(mut layer) = self.layer(size, region) else {
            return;
        };
        self.items(&group.items, &mut layer, text, clips);
        composite(&layer, region, group, clips.scale, pixmap, clips.get(group.clip));
        self.give_back(layer);
    }

    #[expect(clippy::too_many_arguments, reason = "the item's fields")]
    fn text_through(
        &mut self,
        pixmap: &mut Pixmap,
        text: &mut TextSystem,
        clips: &mut Clips,
        region: PixelRect,
        mask: &[Item],
        background: Option<(Bounds, Color)>,
        fill: Option<&(Gradient, Bounds)>,
        clip: Option<Clip>,
    ) {
        let size = (pixmap.width(), pixmap.height());
        let scale = clips.scale;
        let Some(rect) = int_rect(region) else { return };
        let Some(mut layer) = self.layer(size, rect) else {
            return;
        };
        self.items(mask, &mut layer, text, clips);
        let glyphs = layer.clone_rect(rect);
        self.give_back(layer);
        let (Some(glyphs), Some(mut out)) = (glyphs, Pixmap::new(rect.width(), rect.height())) else {
            return;
        };
        let to_fill = into_rect(rect, scale);
        if let Some((b, bg)) = background
            && let Some(r) = Rect::from_xywh(b.x, b.y, b.width, b.height)
        {
            let mut paint = Paint::default();
            paint.set_color(bg.to_skia());
            out.fill_rect(r, &paint, to_fill, None);
        }
        if let Some((g, shown)) = fill
            && let Some(shader) = shader(g)
            && let Some(r) = Rect::from_xywh(shown.x, shown.y, shown.width, shown.height)
        {
            out.fill_rect(r, &shader_paint(shader), to_fill, None);
        }
        out.draw_pixmap(
            0,
            0,
            glyphs.as_ref(),
            &PixmapPaint {
                blend_mode: BlendMode::DestinationIn,
                ..PixmapPaint::default()
            },
            Transform::identity(),
            None,
        );
        pixmap.draw_pixmap(
            rect.x(),
            rect.y(),
            out.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            clips.get(clip),
        );
    }

    fn shadow(&mut self, pixmap: &mut Pixmap, scale: f32, b: Bounds, radius: Corners, s: &Shadow, clip: Option<&Mask>) {
        let color = s.color;
        if color.is_transparent() {
            return;
        }
        let shape = Bounds {
            x: b.x + s.x,
            y: b.y + s.y,
            ..b
        }
        .outset(s.spread);
        if shape.is_empty() {
            return;
        }
        let r = grow(radius, s.spread);
        let pad = f32_to_u32((s.blur * 1.5 * scale).ceil()).saturating_add(1);
        let key = ShadowKey {
            size: (physical(shape.width, scale), physical(shape.height, scale)),
            cutout: [
                physical_pos(s.spread - s.x, scale),
                physical_pos(s.spread - s.y, scale),
                physical_pos(b.width, scale),
                physical_pos(b.height, scale),
            ],
            radius: [r.top_left, r.top_right, r.bottom_right, r.bottom_left].map(|v| f32_to_u32(v * scale * 4.0)),
            blur: f32_to_u32(s.blur * scale * 4.0),
            rgba: color.to_rgba8(),
        };
        let image = self
            .shadows
            .get_or_insert_with(key, || shadow_image(key, radius, r, s.blur, color, scale, pad));
        if let Some(img) = image {
            let pad_i = i32::try_from(pad).unwrap_or(0);
            let x = physical_pos(shape.x, scale).saturating_sub(pad_i);
            let y = physical_pos(shape.y, scale).saturating_sub(pad_i);
            for (dx, dy, part) in &img.parts {
                pixmap.draw_pixmap(
                    x.saturating_add(*dx),
                    y.saturating_add(*dy),
                    part.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    clip,
                );
            }
        }
    }

    /// Paint `source` covering `b` (snapped to device pixels), its corners
    /// rounded by `radius`, from its pixels scaled to that size once.
    fn image(
        &mut self,
        pixmap: &mut Pixmap,
        scale: f32,
        b: Bounds,
        source: &crate::image::ImageSource,
        radius: Corners,
        clip: Option<&Mask>,
    ) {
        let size = (physical(b.width, scale), physical(b.height, scale));
        let key = ImageKey {
            source: std::rc::Rc::as_ptr(&source.0).addr(),
            size,
        };
        let Some(image) = self.images.get_or_insert_with(key, || scaled_image(source, size)) else {
            return;
        };
        let Some(path) = rrect(b, radius) else { return };
        let pattern = tiny_skia::Pattern::new(
            image.pixels.as_ref(),
            SpreadMode::Pad,
            tiny_skia::FilterQuality::Nearest,
            1.0,
            Transform::from_translate(b.x, b.y).pre_scale(1.0 / scale, 1.0 / scale),
        );
        let paint = Paint {
            shader: pattern,
            anti_alias: true,
            ..Paint::default()
        };
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::from_scale(scale, scale),
            clip,
        );
    }
}

/// The CPU renderer painting into one pixmap.
pub struct PixmapRenderer<'a> {
    pub renderer: &'a mut CpuRenderer,
    pub pixmap: &'a mut Pixmap,
}

impl Renderer for PixmapRenderer<'_> {
    fn render(&mut self, scene: &Scene, text: &mut TextSystem) {
        self.renderer.render(scene, text, self.pixmap);
    }

    fn end_frame(&mut self) {
        self.renderer.end_frame();
    }

    fn held(&self) -> Held {
        self.renderer.held()
    }

    fn capture(&mut self, scene: &Scene, text: &mut TextSystem) -> Option<Pixmap> {
        self.renderer.capture(scene, text)
    }
}

fn int_rect(r: PixelRect) -> Option<IntRect> {
    IntRect::from_xywh(r.x, r.y, r.width, r.height)
}

/// Physical-pixel clip masks, made once per distinct clip per frame.
struct Clips {
    masks: HashMap<[i32; 8], Mask>,
    size: (u32, u32),
    scale: f32,
}

impl Clips {
    fn key(&self, c: Clip) -> [i32; 8] {
        let s = self.scale;
        let (b, r) = (c.bounds, c.radius);
        [
            physical_pos(b.x, s),
            physical_pos(b.y, s),
            physical_pos(b.right(), s),
            physical_pos(b.bottom(), s),
            physical_pos(r.top_left, s),
            physical_pos(r.top_right, s),
            physical_pos(r.bottom_right, s),
            physical_pos(r.bottom_left, s),
        ]
    }

    /// The mask `clip` paints through, made when first asked for.
    fn get(&mut self, clip: Option<Clip>) -> Option<&Mask> {
        let c = clip?;
        let key = self.key(c);
        if !self.masks.contains_key(&key) {
            let mut mask = Mask::new(self.size.0, self.size.1)?;
            if let Some(path) = rrect(c.bounds.snapped(self.scale), c.radius) {
                mask.fill_path(
                    &path,
                    FillRule::Winding,
                    !c.radius.is_zero(),
                    Transform::from_scale(self.scale, self.scale),
                );
            }
            self.masks.insert(key, mask);
        }
        self.masks.get(&key)
    }
}

fn push_rrect(pb: &mut PathBuilder, b: Bounds, r: Corners) {
    let max = b.width.min(b.height) / 2.0;
    let (tl, tr, br, bl) = (
        r.top_left.clamp(0.0, max),
        r.top_right.clamp(0.0, max),
        r.bottom_right.clamp(0.0, max),
        r.bottom_left.clamp(0.0, max),
    );
    let (x, y, right, bottom) = (b.x, b.y, b.right(), b.bottom());
    let k = CUBIC_ARC;
    pb.move_to(x + tl, y);
    pb.line_to(right - tr, y);
    if tr > 0.0 {
        pb.cubic_to(right - tr + tr * k, y, right, y + tr - tr * k, right, y + tr);
    }
    pb.line_to(right, bottom - br);
    if br > 0.0 {
        pb.cubic_to(
            right,
            bottom - br + br * k,
            right - br + br * k,
            bottom,
            right - br,
            bottom,
        );
    }
    pb.line_to(x + bl, bottom);
    if bl > 0.0 {
        pb.cubic_to(x + bl - bl * k, bottom, x, bottom - bl + bl * k, x, bottom - bl);
    }
    pb.line_to(x, y + tl);
    if tl > 0.0 {
        pb.cubic_to(x, y + tl - tl * k, x + tl - tl * k, y, x + tl, y);
    }
    pb.close();
}

pub(crate) fn rrect(b: Bounds, r: Corners) -> Option<Path> {
    let mut pb = PathBuilder::new();
    push_rrect(&mut pb, b, r);
    pb.finish()
}

/// The ring between two rounded boxes.
fn band(outer: Bounds, outer_r: Corners, inner: Bounds, inner_r: Corners) -> Option<Path> {
    let mut pb = PathBuilder::new();
    push_rrect(&mut pb, outer, outer_r);
    if !inner.is_empty() {
        push_rrect(&mut pb, inner, inner_r);
    }
    pb.finish()
}

fn solid(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color.to_skia());
    paint.anti_alias = true;
    paint
}

fn shader_paint(shader: Shader<'_>) -> Paint<'_> {
    Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    }
}

/// `g` as a tiny-skia shader, OKLab mixing approximated by stops between stops.
fn shader(g: &Gradient) -> Option<Shader<'static>> {
    match g {
        Gradient::Solid(c) => Some(Shader::SolidColor(c.to_skia())),
        Gradient::Linear { start, end, stops } => {
            let mut skia = Vec::new();
            for pair in stops.windows(2) {
                if let [(o0, c0), (o1, c1)] = pair {
                    for k in 0..OKLAB_STEPS {
                        let t = f32::from(k) / f32::from(OKLAB_STEPS);
                        skia.push(GradientStop::new(
                            (o1 - o0).mul_add(t, *o0),
                            c0.mix_oklab(*c1, t).to_skia(),
                        ));
                    }
                }
            }
            let last = stops.last()?.1;
            skia.push(GradientStop::new(1.0, last.to_skia()));
            tiny_skia::LinearGradient::new(
                Point::from_xy(start.0, start.1),
                Point::from_xy(end.0, end.1),
                skia,
                SpreadMode::Pad,
                Transform::identity(),
            )
        }
    }
}

/// The offset that paints logical pixels into a pixmap whose origin is `rect`'s.
fn into_rect(rect: IntRect, scale: f32) -> Transform {
    Transform::from_row(scale, 0.0, 0.0, scale, -i32_to_f32(rect.x()), -i32_to_f32(rect.y()))
}

/// Mask and filter what `group` painted in `region` of `layer`, and draw it
/// onto `into` scaled and turned as the group asks, through `clip`.
fn composite(layer: &Pixmap, region: IntRect, group: &Group, scale: f32, into: &mut Pixmap, clip: Option<&Mask>) {
    let Some(mut pixels) = layer.clone_rect(region) else {
        return;
    };
    // The whole group, in the logical pixels the mask's shader is placed in.
    let all = Rect::from_xywh(
        i32_to_f32(region.x()) / scale,
        i32_to_f32(region.y()) / scale,
        u32_to_f32(region.width()) / scale,
        u32_to_f32(region.height()) / scale,
    );
    if let Some(g) = &group.mask
        && let Some(shader) = shader(g)
        && let Some(all) = all
    {
        let paint = Paint {
            blend_mode: BlendMode::DestinationIn,
            ..shader_paint(shader)
        };
        pixels.fill_rect(all, &paint, into_rect(region, scale), None);
    }
    for f in &group.filters {
        for px in pixels.pixels_mut() {
            *px = filter(*f, *px);
        }
    }
    let opacity = group.opacity;
    let (transform, paint) = match group.transform {
        None => (
            Transform::identity(),
            PixmapPaint {
                opacity,
                ..PixmapPaint::default()
            },
        ),
        Some(t) => {
            let (ox, oy) = t.origin;
            (
                Transform::from_translate(ox, oy)
                    .pre_rotate(t.rotate)
                    .pre_scale(t.scale, t.scale)
                    .pre_translate(-ox, -oy),
                PixmapPaint {
                    opacity,
                    quality: tiny_skia::FilterQuality::Bilinear,
                    ..PixmapPaint::default()
                },
            )
        }
    };
    into.draw_pixmap(region.x(), region.y(), pixels.as_ref(), &paint, transform, clip);
}

/// One premultiplied pixel through `f`, in sRGB as browsers apply CSS filters.
fn filter(f: Filter, px: PremultipliedColorU8) -> PremultipliedColorU8 {
    let a = f32::from(px.alpha()) / 255.0;
    let [r, g, b] = [px.red(), px.green(), px.blue()].map(|v| f32::from(v) / 255.0);
    let [r, g, b] = match f {
        Filter::Brightness(k) => [r * k, g * k, b * k],
        Filter::Grayscale(amount) => {
            let lum = 0.0722_f32.mul_add(b, 0.2126_f32.mul_add(r, 0.7152 * g));
            [r, g, b].map(|c| (lum - c).mul_add(amount, c))
        }
    };
    let [r, g, b] = [r, g, b].map(|c| unit_to_u8(c.clamp(0.0, a)));
    PremultipliedColorU8::from_rgba(r, g, b, px.alpha()).unwrap_or(px)
}

pub(crate) fn paint_shapes(pixmap: &mut Pixmap, transform: Transform, shapes: &[Shape], clip: Option<&Mask>) {
    for shape in shapes {
        let mut pb = PathBuilder::new();
        for c in &shape.commands {
            match *c {
                PathCommand::MoveTo(x, y) => pb.move_to(x, y),
                PathCommand::LineTo(x, y) => pb.line_to(x, y),
                PathCommand::QuadTo(cx, cy, x, y) => pb.quad_to(cx, cy, x, y),
                PathCommand::CubicTo(a, b2, c2, d, x, y) => pb.cubic_to(a, b2, c2, d, x, y),
                PathCommand::Close => pb.close(),
            }
        }
        let Some(path) = pb.finish() else { continue };
        if let Some(fill) = shape.fill {
            pixmap.fill_path(&path, &solid(fill), FillRule::Winding, transform, clip);
        }
        if let Some((color, width)) = shape.stroke {
            let stroke = tiny_skia::Stroke {
                width,
                line_cap: tiny_skia::LineCap::Round,
                line_join: tiny_skia::LineJoin::Round,
                ..tiny_skia::Stroke::default()
            };
            pixmap.stroke_path(&path, &solid(color), &stroke, transform, clip);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    glyph: CacheKey,
    rgba: [u8; 4],
}

struct GlyphImage {
    pixmap: Pixmap,
    left: i32,
    top: i32,
}

fn glyph_image(text: &mut TextSystem, key: CacheKey, color: Color) -> Option<GlyphImage> {
    let image = text.raster(key)?;
    let (w, h) = (image.placement.width, image.placement.height);
    let mut pixmap = Pixmap::new(w.max(1), h.max(1))?;
    let [r, g, b, a] = color.to_rgba8();
    match image.content {
        SwashContent::Mask => {
            for (px, &coverage) in pixmap.pixels_mut().iter_mut().zip(image.data.iter()) {
                let alpha = mul_u8(coverage, a);
                *px = tiny_skia::ColorU8::from_rgba(r, g, b, alpha).premultiply();
            }
        }
        SwashContent::Color | SwashContent::SubpixelMask => {
            let (rgba, _) = image.data.as_chunks::<4>();
            for (px, &[cr, cg, cb, ca]) in pixmap.pixels_mut().iter_mut().zip(rgba) {
                *px = tiny_skia::ColorU8::from_rgba(cr, cg, cb, mul_u8(ca, a)).premultiply();
            }
        }
    }
    Some(GlyphImage {
        pixmap,
        left: image.placement.left,
        top: image.placement.top,
    })
}

/// `a * b / 255`, rounded.
pub(crate) fn mul_u8(a: u8, b: u8) -> u8 {
    let p = u16::from(a).saturating_mul(u16::from(b));
    u8::try_from(p.saturating_add(127) / 255).unwrap_or(u8::MAX)
}

/// An image's pixels at one physical size, keyed by the decoded image (its
/// address, kept alive by the entry) and that size.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ImageKey {
    source: usize,
    size: (u32, u32),
}

struct ScaledImage {
    /// Holds the decoded image, so its address names no other while cached.
    _source: crate::image::ImageSource,
    pixels: Pixmap,
}

/// `source` covering a `size` physical-pixel box, as `object-fit: cover`
/// centres it, sampled bilinearly once.
fn scaled_image(source: &crate::image::ImageSource, size: (u32, u32)) -> Option<ScaledImage> {
    let (iw, ih) = source.size();
    let (iw, ih) = (u32_to_f32(iw).max(1.0), u32_to_f32(ih).max(1.0));
    let (w, h) = (u32_to_f32(size.0), u32_to_f32(size.1));
    let fit = (w / iw).max(h / ih);
    let mut pixels = Pixmap::new(size.0, size.1)?;
    let pattern = tiny_skia::Pattern::new(
        source.0.as_ref().as_ref(),
        SpreadMode::Pad,
        tiny_skia::FilterQuality::Bilinear,
        1.0,
        Transform::from_translate((w - iw * fit) / 2.0, (h - ih * fit) / 2.0).pre_scale(fit, fit),
    );
    let paint = Paint {
        shader: pattern,
        ..Paint::default()
    };
    pixels.fill_rect(Rect::from_xywh(0.0, 0.0, w, h)?, &paint, Transform::identity(), None);
    Some(ScaledImage {
        _source: source.clone(),
        pixels,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ShadowKey {
    size: (u32, u32),
    /// The casting box inside the shadow image: offset from the shadow's
    /// origin and size, in physical pixels; it is cut out, as CSS draws
    /// shadows only outside the box.
    cutout: [i32; 4],
    radius: [u32; 4],
    blur: u32,
    rgba: [u8; 4],
}

/// A shadow as the parts of its image that hold ink, each at its offset in
/// the whole: the bands around the cut-out box's clear middle, which is
/// neither held nor blended (a card's shadow is a thin ring around it).
struct ShadowImage {
    parts: Vec<(i32, i32, Pixmap)>,
}

impl ShadowImage {
    /// `whole` without `clear`, a rectangle of it no pixel of which is inked.
    fn banded(whole: Pixmap, clear: Option<IntRect>) -> Self {
        let (Ok(w), Ok(h)) = (i32::try_from(whole.width()), i32::try_from(whole.height())) else {
            return Self {
                parts: vec![(0, 0, whole)],
            };
        };
        let Some(c) = clear.filter(|c| c.left() > 0 && c.top() > 0 && c.right() < w && c.bottom() < h) else {
            return Self {
                parts: vec![(0, 0, whole)],
            };
        };
        let bands = [
            (0, 0, w, c.top()),
            (0, c.bottom(), w, h),
            (0, c.top(), c.left(), c.bottom()),
            (c.right(), c.top(), w, c.bottom()),
        ];
        Self {
            parts: bands
                .into_iter()
                .filter_map(|(l, t, r, b)| {
                    let part = whole.clone_rect(IntRect::from_ltrb(l, t, r, b)?)?;
                    Some((l, t, part))
                })
                .collect(),
        }
    }
}

/// A blurred rounded box, `pad` pixels of blur room on each side, with the
/// casting box (radius `own`) cut out.
fn shadow_image(
    key: ShadowKey,
    own: Corners,
    r: Corners,
    blur: f32,
    color: Color,
    scale: f32,
    pad: u32,
) -> Option<ShadowImage> {
    let size = key.size;
    let w = size.0.saturating_add(pad.saturating_mul(2));
    let h = size.1.saturating_add(pad.saturating_mul(2));
    let mut shape = Pixmap::new(w, h)?;
    let p = u32_to_f32(pad);
    let b = Bounds {
        x: p,
        y: p,
        width: u32_to_f32(size.0),
        height: u32_to_f32(size.1),
    };
    let path = rrect(b, r.scaled(scale))?;
    shape.fill_path(
        &path,
        &solid(Color::BLACK),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    let (wu, hu) = (usize::try_from(w).ok()?, usize::try_from(h).ok()?);
    let mut alpha: Vec<f32> = shape.pixels().iter().map(|px| f32::from(px.alpha()) / 255.0).collect();
    let sigma = blur * scale / 2.0;
    if sigma > 0.0 {
        // Three box blurs approximate a Gaussian.
        let radius = f32_to_i32(((12.0 * sigma * sigma / 3.0 + 1.0).sqrt() / 2.0).floor()).max(1);
        let radius = usize::try_from(radius).unwrap_or(1);
        for _ in 0..3 {
            alpha = blur_rows(&alpha, wu, radius);
            alpha = transpose(&alpha, wu, hu);
            alpha = blur_rows(&alpha, hu, radius);
            alpha = transpose(&alpha, hu, wu);
        }
    }
    let [r8, g8, b8, a8] = color.to_rgba8();
    for (px, a) in shape.pixels_mut().iter_mut().zip(alpha) {
        let alpha = unit_to_u8(a * f32::from(a8) / 255.0);
        *px = tiny_skia::ColorU8::from_rgba(r8, g8, b8, alpha).premultiply();
    }
    let [cx, cy, cw, ch] = key.cutout.map(i32_to_f32);
    let cut = Bounds {
        x: cx + p,
        y: cy + p,
        width: cw,
        height: ch,
    };
    let own = own.scaled(scale);
    if let Some(path) = rrect(cut, own) {
        let mut clear = solid(Color::BLACK);
        clear.blend_mode = BlendMode::Clear;
        shape.fill_path(&path, &clear, FillRule::Winding, Transform::identity(), None);
    }
    // Inside the cut-out box, past its corners and the pixel its edge's
    // coverage reaches, nothing is inked.
    let inset = own
        .top_left
        .max(own.top_right)
        .max(own.bottom_right)
        .max(own.bottom_left)
        + 1.0;
    let clear = IntRect::from_ltrb(
        f32_to_i32((cut.x + inset).ceil()),
        f32_to_i32((cut.y + inset).ceil()),
        f32_to_i32((cut.right() - inset).floor()),
        f32_to_i32((cut.bottom() - inset).floor()),
    );
    Some(ShadowImage::banded(shape, clear))
}

fn blur_rows(data: &[f32], width: usize, radius: usize) -> Vec<f32> {
    let norm = 1.0 / usize_to_f32(radius.saturating_mul(2).saturating_add(1));
    let mut out = Vec::with_capacity(data.len());
    for row in data.chunks(width.max(1)) {
        let mut prefix = Vec::with_capacity(row.len().saturating_add(1));
        prefix.push(0.0_f32);
        let mut sum = 0.0;
        for v in row {
            sum += v;
            prefix.push(sum);
        }
        for x in 0..row.len() {
            let lo = x.saturating_sub(radius);
            let hi = x.saturating_add(radius).saturating_add(1).min(row.len());
            let total = prefix.get(hi).copied().unwrap_or(0.0) - prefix.get(lo).copied().unwrap_or(0.0);
            out.push(total * norm);
        }
    }
    out
}

fn transpose(data: &[f32], width: usize, height: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(data.len());
    for x in 0..width {
        for y in 0..height {
            let i = y.saturating_mul(width).saturating_add(x);
            out.push(data.get(i).copied().unwrap_or(0.0));
        }
    }
    out
}
