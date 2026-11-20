//! Painting a node's [`Effects`]: a gradient background, text that shows the
//! background through its glyphs, and a mask and filters over everything the
//! node and its children paint (a group, painted aside and composited).

use tiny_skia::{
    BlendMode, FillRule, GradientStop, IntRect, Paint, Pixmap, PixmapPaint, Point, PremultipliedColorU8, Rect, Shader,
    SpreadMode, Transform,
};

use crate::color::Color;
use crate::convert::{f32_to_i32, i32_to_f32, u32_to_f32, unit_to_u8};
use crate::geometry::Bounds;
use crate::layout::FrameNode;
use crate::style::{Axis, BackgroundClip, BackgroundImage, Direction, Filter, LinearGradient, MaskImage, ResolvedText};
use crate::text::{Paragraph, Target, TextSystem};

/// Colour stops added between two stops, so tiny-skia's sRGB interpolation
/// follows the OKLab one CSS asks for.
const OKLAB_STEPS: u8 = 8;

/// Pixels per `ch` for `t`: the advance of its `0`.
fn ch(text: &mut TextSystem, t: &ResolvedText) -> f32 {
    text.measure("0", t, None).0
}

/// `gradient` painting `tile`, its `currentColor` being `current`.
fn shader(g: &LinearGradient, tile: Bounds, current: Color, ch: f32) -> Option<Shader<'static>> {
    let (sin, cos) = g.angle.to_radians().sin_cos();
    let dir = (sin, -cos);
    let length = (tile.width * sin).abs() + (tile.height * cos).abs();
    // A stop never sits before the one ahead of it (CSS fixes it up the same way).
    let mut at = f32::NEG_INFINITY;
    let stops: Vec<(f32, Color)> = g
        .stops
        .iter()
        .map(|s| {
            at = at.max(s.at.resolve(length, ch));
            (at, s.ink.resolve(current))
        })
        .collect();
    let (&(first, _), &(last, last_color)) = (stops.first()?, stops.last()?);
    if last - first < 1e-3 {
        return Some(Shader::SolidColor(last_color.to_skia()));
    }
    let line_start = (
        dir.0.mul_add(-length / 2.0, tile.x + tile.width / 2.0),
        dir.1.mul_add(-length / 2.0, tile.y + tile.height / 2.0),
    );
    let point = |p: f32| Point::from_xy(dir.0.mul_add(p, line_start.0), dir.1.mul_add(p, line_start.1));
    let offset = |p: f32| (p - first) / (last - first);
    let mut skia = Vec::new();
    for pair in stops.windows(2) {
        if let [(p0, c0), (p1, c1)] = pair {
            for k in 0..OKLAB_STEPS {
                let t = f32::from(k) / f32::from(OKLAB_STEPS);
                skia.push(GradientStop::new(
                    offset((p1 - p0).mul_add(t, *p0)),
                    c0.mix_oklab(*c1, t).to_skia(),
                ));
            }
        }
    }
    skia.push(GradientStop::new(1.0, last_color.to_skia()));
    tiny_skia::LinearGradient::new(point(first), point(last), skia, SpreadMode::Pad, Transform::identity())
}

/// Where the background tile sits in `node`'s padding box.
fn tile(img: &BackgroundImage, node: &FrameNode, ch: f32) -> Bounds {
    let b = node.look.border;
    let area = node.bounds.inset(b.top, b.right, b.bottom, b.left);
    let [w, h] = img.size;
    let [x, y] = img.position;
    let (width, height) = (w.resolve(area.width, ch), h.resolve(area.height, ch));
    Bounds {
        x: area.x + x.resolve(area.width - width, ch),
        y: area.y + y.resolve(area.height - height, ch),
        width,
        height,
    }
}

fn shader_paint(shader: Shader<'_>) -> Paint<'_> {
    Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    }
}

/// The background image of `node` when it paints its box (not its text).
pub(crate) fn box_background(node: &FrameNode) -> Option<&BackgroundImage> {
    let img = node.look.effects.background_image.as_ref()?;
    (img.clip == BackgroundClip::BorderBox).then_some(img)
}

/// The background image of `node` when it shows through the node's text.
pub(crate) fn text_background(node: &FrameNode) -> Option<&BackgroundImage> {
    let img = node.look.effects.background_image.as_ref()?;
    (img.clip == BackgroundClip::Text).then_some(img)
}

/// Paint `img` over `node`'s border box, inside its rounded corners.
pub(crate) fn paint_background(
    target: &mut Target<'_>,
    node: &FrameNode,
    img: &BackgroundImage,
    text: &mut TextSystem,
) {
    let ch = ch(text, &node.look.text);
    let tile = tile(img, node, ch);
    let Some(shader) = shader(&img.gradient, tile, node.look.text.color, ch) else {
        return;
    };
    let scale = target.scale;
    let Some(path) = crate::paint::rrect(node.bounds.snapped(scale).intersect(&tile), node.look.radius) else {
        return;
    };
    target.pixmap.fill_path(
        &path,
        &shader_paint(shader),
        FillRule::Winding,
        Transform::from_scale(scale, scale),
        target.clip,
    );
}

/// `rect` in physical pixels, grown to whole pixels and kept inside `size`.
fn physical_rect(rect: Bounds, scale: f32, size: (u32, u32)) -> Option<IntRect> {
    let (w, h) = (
        i32::try_from(size.0).unwrap_or(i32::MAX),
        i32::try_from(size.1).unwrap_or(i32::MAX),
    );
    IntRect::from_ltrb(
        f32_to_i32((rect.x * scale).floor()).clamp(0, w),
        f32_to_i32((rect.y * scale).floor()).clamp(0, h),
        f32_to_i32((rect.right() * scale).ceil()).clamp(0, w),
        f32_to_i32((rect.bottom() * scale).ceil()).clamp(0, h),
    )
}

/// The offset that paints logical pixels into a pixmap whose origin is `rect`'s.
fn into_rect(rect: IntRect, scale: f32) -> Transform {
    Transform::from_row(scale, 0.0, 0.0, scale, -i32_to_f32(rect.x()), -i32_to_f32(rect.y()))
}

/// Paint `node`'s text with `img` showing through its glyphs, in place of
/// the text's own colour.
pub(crate) fn paint_text_through(
    target: &mut Target<'_>,
    node: &FrameNode,
    img: &BackgroundImage,
    text: &mut TextSystem,
    p: Paragraph<'_>,
    t: &ResolvedText,
    width: Option<f32>,
) {
    let area = node.content_box;
    let scale = target.scale;
    let size = (target.pixmap.width(), target.pixmap.height());
    let Some(rect) = physical_rect(node.bounds.outset(t.line_height), scale, size) else {
        return;
    };
    let (Some(mut glyphs), Some(mut fill)) = (
        Pixmap::new(rect.width(), rect.height()),
        Pixmap::new(rect.width(), rect.height()),
    ) else {
        return;
    };
    // Glyph coverage only: the text's colour shows nowhere.
    let shape = ResolvedText {
        color: Color::WHITE,
        ..*t
    };
    let (dx, dy) = (i32_to_f32(rect.x()) / scale, i32_to_f32(rect.y()) / scale);
    text.draw(
        &mut Target {
            pixmap: &mut glyphs,
            scale,
            clip: None,
        },
        p,
        &shape,
        width,
        (area.x - dx, area.y - dy),
        1.0,
    );
    let ch = ch(text, t);
    let tile = tile(img, node, ch);
    let to_fill = into_rect(rect, scale);
    if let Some(bg) = node.look.background.filter(|c| !c.is_transparent())
        && let Some(r) = Rect::from_xywh(node.bounds.x, node.bounds.y, node.bounds.width, node.bounds.height)
    {
        let mut paint = Paint::default();
        paint.set_color(bg.to_skia());
        fill.fill_rect(r, &paint, to_fill, None);
    }
    let shown = node.bounds.intersect(&tile);
    if let Some(shader) = shader(&img.gradient, tile, t.color, ch)
        && let Some(r) = Rect::from_xywh(shown.x, shown.y, shown.width, shown.height)
    {
        fill.fill_rect(r, &shader_paint(shader), to_fill, None);
    }
    fill.draw_pixmap(
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
    target.pixmap.draw_pixmap(
        rect.x(),
        rect.y(),
        fill.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        target.clip,
    );
}

/// The mask over `node`'s painting this frame, a scroll fade resolved
/// against where the node is scrolled to.
fn mask(node: &FrameNode) -> Option<LinearGradient> {
    match node.look.effects.mask.as_ref()? {
        MaskImage::Linear(g) => Some(g.clone()),
        MaskImage::ScrollFade(fade) => {
            let scroll = node.scroll?;
            match fade.axis {
                // `scroll-fade-x` under `dir="rtl"` runs its gradient `to left`:
                // the start fade sits on the right, where right to left
                // content starts.
                Axis::Horizontal => fade
                    .gradient(node.bounds.width, scroll.offset.0, scroll.max.0)
                    .map(|g| match node.look.text.direction {
                        Direction::Ltr => g,
                        Direction::Rtl => LinearGradient { angle: 270.0, ..g },
                    }),
                Axis::Vertical => fade.gradient(node.bounds.height, scroll.offset.1, scroll.max.1),
            }
        }
    }
}

/// Whether `node` and its children paint aside first, to be masked,
/// filtered, scaled or turned as one.
pub(crate) fn grouped(node: &FrameNode) -> bool {
    !node.look.effects.filters.is_empty()
        || mask(node).is_some()
        || !node.look.transform.is_identity()
        || node.look.opacity < 1.0
}

/// The physical region a group of `node` covers: its border box under a mask
/// (which clips to it), else the box with its ring and shadows.
pub(crate) fn group_region(node: &FrameNode, scale: f32, size: (u32, u32)) -> Option<IntRect> {
    let reach = if node.look.effects.mask.is_some() {
        0.0
    } else {
        reach(node)
    };
    physical_rect(node.bounds.outset(reach), scale, size)
}

/// How far past its border box `node` paints: its ring and shadows.
fn reach(node: &FrameNode) -> f32 {
    let look = &node.look;
    let ring = look.ring.map_or(0.0, |r| r.width);
    look.shadows
        .iter()
        .map(|s| s.blur.mul_add(1.5, s.spread) + s.x.abs().max(s.y.abs()))
        .fold(ring, f32::max)
}

/// The physical region the nodes of a subtree paint, `nodes[0]` its root:
/// every box with its ring and shadows, inside the clips set within it.
pub(crate) fn subtree_region(nodes: &[FrameNode], scale: f32, size: (u32, u32)) -> Option<IntRect> {
    let outer = nodes.first()?.clip;
    let painted = nodes.iter().filter(|n| !n.look.invisible).map(|n| {
        let b = n.bounds.outset(reach(n));
        match n.clip.filter(|_| n.clip != outer) {
            Some(c) => c.intersect(&b),
            None => b,
        }
    });
    let union = painted.filter(|b| !b.is_empty()).reduce(|a, b| {
        let (x, y) = (a.x.min(b.x), a.y.min(b.y));
        Bounds {
            x,
            y,
            width: a.right().max(b.right()) - x,
            height: a.bottom().max(b.bottom()) - y,
        }
    })?;
    physical_rect(union, scale, size)
}

/// Clear `region` of `layer`, ready for a group to paint into.
pub(crate) fn clear(layer: &mut Pixmap, region: IntRect) {
    let rect = region.to_rect();
    let paint = Paint {
        blend_mode: BlendMode::Clear,
        ..Paint::default()
    };
    layer.fill_rect(rect, &paint, Transform::identity(), None);
}

/// Mask and filter what `node`'s group painted in `region` of `layer`, and
/// draw it onto `into` scaled and turned as the node is, through `clip`.
pub(crate) fn composite(
    layer: &Pixmap,
    region: IntRect,
    node: &FrameNode,
    scale: f32,
    into: &mut Pixmap,
    clip: Option<&tiny_skia::Mask>,
) {
    let Some(mut group) = layer.clone_rect(region) else {
        return;
    };
    // The whole group, in the logical pixels the mask's shader is placed in.
    let all = Rect::from_xywh(
        i32_to_f32(region.x()) / scale,
        i32_to_f32(region.y()) / scale,
        u32_to_f32(region.width()) / scale,
        u32_to_f32(region.height()) / scale,
    );
    if let Some(g) = mask(node)
        && let Some(shader) = shader(&g, node.bounds, node.look.text.color, 0.0)
        && let Some(all) = all
    {
        let paint = Paint {
            blend_mode: BlendMode::DestinationIn,
            ..shader_paint(shader)
        };
        group.fill_rect(all, &paint, into_rect(region, scale), None);
    }
    for f in &node.look.effects.filters {
        for px in group.pixels_mut() {
            *px = filter(*f, *px);
        }
    }
    let t = node.look.transform;
    let opacity = node.look.opacity;
    let (transform, paint) = if t.is_identity() {
        (
            Transform::identity(),
            PixmapPaint {
                opacity,
                ..PixmapPaint::default()
            },
        )
    } else {
        let (ox, oy) = (
            (node.bounds.x + t.origin.0) * scale,
            (node.bounds.y + t.origin.1) * scale,
        );
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
    };
    into.draw_pixmap(region.x(), region.y(), group.as_ref(), &paint, transform, clip);
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
