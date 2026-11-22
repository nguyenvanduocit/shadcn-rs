//! A node's [`Effects`] as the scene records them: a gradient background,
//! text that shows the background through its glyphs, and the group a
//! mask, filters, opacity or a transform make of a node and its children.
//!
//! [`Effects`]: crate::style::Effects

use crate::color::Color;
use crate::convert::f32_to_i32;
use crate::geometry::Bounds;
use crate::layout::FrameNode;
use crate::scene::{Gradient, Item, PixelRect};
use crate::style::{Axis, BackgroundClip, BackgroundImage, Direction, LinearGradient, MaskImage, ResolvedText};
use crate::text::{Paragraph, Target, TextSystem};

/// Pixels per `ch` for `t`: the advance of its `0`.
fn ch(text: &mut TextSystem, t: &ResolvedText) -> f32 {
    text.measure("0", t, None).0
}

/// `g` placed over `tile`, its `currentColor` being `current`.
pub(crate) fn gradient(g: &LinearGradient, tile: Bounds, current: Color, ch: f32) -> Option<Gradient> {
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
            (at, s.color.resolve(current))
        })
        .collect();
    let (&(first, _), &(last, last_color)) = (stops.first()?, stops.last()?);
    if last - first < 1e-3 {
        return Some(Gradient::Solid(last_color));
    }
    let line_start = (
        dir.0.mul_add(-length / 2.0, tile.x + tile.width / 2.0),
        dir.1.mul_add(-length / 2.0, tile.y + tile.height / 2.0),
    );
    let point = |p: f32| (dir.0.mul_add(p, line_start.0), dir.1.mul_add(p, line_start.1));
    Some(Gradient::Linear {
        start: point(first),
        end: point(last),
        stops: stops.iter().map(|&(p, c)| ((p - first) / (last - first), c)).collect(),
    })
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

/// Record `img` over `node`'s border box, inside its rounded corners.
pub(crate) fn record_background(
    target: &mut Target<'_>,
    node: &FrameNode,
    img: &BackgroundImage,
    text: &mut TextSystem,
) {
    let ch = ch(text, &node.look.text);
    let tile = tile(img, node, ch);
    let Some(gradient) = gradient(&img.gradient, tile, node.look.text.color, ch) else {
        return;
    };
    let b = target.snapped(node.bounds);
    target.items.push(Item::Gradient {
        bounds: b.intersect(&tile),
        radius: node.look.shape.used(b.width, b.height),
        gradient,
    });
}

/// `rect` in physical pixels, grown to whole pixels and kept inside `size`.
pub(crate) fn physical_rect(rect: Bounds, scale: f32, size: (u32, u32)) -> Option<PixelRect> {
    let (w, h) = (
        i32::try_from(size.0).unwrap_or(i32::MAX),
        i32::try_from(size.1).unwrap_or(i32::MAX),
    );
    let (l, t) = (
        f32_to_i32((rect.x * scale).floor()).clamp(0, w),
        f32_to_i32((rect.y * scale).floor()).clamp(0, h),
    );
    let (r, b) = (
        f32_to_i32((rect.right() * scale).ceil()).clamp(0, w),
        f32_to_i32((rect.bottom() * scale).ceil()).clamp(0, h),
    );
    let width = u32::try_from(r.checked_sub(l)?).ok().filter(|&w| w > 0)?;
    let height = u32::try_from(b.checked_sub(t)?).ok().filter(|&h| h > 0)?;
    Some(PixelRect {
        x: l,
        y: t,
        width,
        height,
    })
}

/// The whole physical pixels `rect` touches, unbounded.
fn pixel_cover(rect: Bounds, scale: f32) -> Option<PixelRect> {
    let (l, t) = (
        f32_to_i32((rect.x * scale).floor()),
        f32_to_i32((rect.y * scale).floor()),
    );
    let (r, b) = (
        f32_to_i32((rect.right() * scale).ceil()),
        f32_to_i32((rect.bottom() * scale).ceil()),
    );
    let width = u32::try_from(r.checked_sub(l)?).ok().filter(|&w| w > 0)?;
    let height = u32::try_from(b.checked_sub(t)?).ok().filter(|&h| h > 0)?;
    Some(PixelRect {
        x: l,
        y: t,
        width,
        height,
    })
}

/// Record `node`'s text with `img` showing through its glyphs, in place of
/// the text's own colour.
pub(crate) fn record_text_through(
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
    // In the chunk's paint space, which the window's edges do not bound.
    let Some(region) = pixel_cover(node.bounds.outset(t.line_height), scale) else {
        return;
    };
    // Glyph coverage only: the text's colour shows nowhere.
    let shape = ResolvedText {
        color: Color::WHITE,
        ..*t
    };
    let mut mask = Vec::new();
    text.draw(
        &mut Target {
            items: &mut mask,
            scale,
            size: target.size,
            cull: None,
            snap: target.snap,
        },
        p,
        &shape,
        width,
        (area.x, area.y),
        1.0,
    );
    let ch = ch(text, t);
    let tile = tile(img, node, ch);
    let shown = node.bounds.intersect(&tile);
    target.items.push(Item::TextThrough {
        region,
        mask,
        background: node
            .look
            .background
            .filter(|c| !c.is_transparent())
            .map(|c| (node.bounds, c)),
        fill: gradient(&img.gradient, tile, t.color, ch).map(|g| (g, shown)),
    });
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
pub(crate) fn group_region(node: &FrameNode, scale: f32, size: (u32, u32)) -> Option<PixelRect> {
    let reach = if node.look.effects.mask.is_some() {
        0.0
    } else {
        reach(node)
    };
    physical_rect(node.bounds.outset(reach), scale, size)
}

/// How far past its border box `node` paints: its ring (at its offset)
/// and shadows, its inset layer's shadows included.
fn reach(node: &FrameNode) -> f32 {
    let look = &node.look;
    let ring = look.ring.map_or(0.0, |r| (r.offset + r.width).max(0.0));
    let shadow = |s: &crate::style::Shadow| s.blur.mul_add(1.5, s.spread) + s.x.abs().max(s.y.abs());
    let inset = look.decoration.inset.as_ref().map_or(0.0, |l| {
        let least = l.inset.top.min(l.inset.right).min(l.inset.bottom).min(l.inset.left);
        l.shadows.iter().map(shadow).fold(0.0, f32::max) - least
    });
    look.shadows.iter().map(shadow).fold(ring.max(inset), f32::max)
}

/// The physical region the nodes of a subtree paint, `nodes[0]` its root:
/// every box with its ring and shadows, inside the clips set within it.
pub(crate) fn subtree_region(nodes: &[FrameNode], scale: f32, size: (u32, u32)) -> Option<PixelRect> {
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

/// The mask gradient over `node`'s group, placed over its border box.
pub(crate) fn mask_gradient(node: &FrameNode) -> Option<Gradient> {
    gradient(&mask(node)?, node.bounds, node.look.text.color, 0.0)
}
