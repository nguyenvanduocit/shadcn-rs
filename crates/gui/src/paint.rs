//! Painting a [`Frame`] into a pixmap with tiny-skia: shadows, rounded
//! backgrounds and borders, focus rings, text, icons, text inputs and
//! scrollbars, each clipped to what its ancestors allow.

use std::collections::HashMap;
use std::time::Duration;

use tiny_skia::{FillRule, IntRect, Mask, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Transform};

use crate::cache::FrameCache;
use crate::clock;
use crate::color::Color;
use crate::convert::{f32_to_i32, f32_to_u32, u32_to_f32, usize_to_f32};
use crate::effects;
use crate::geometry::Bounds;
use crate::icon::IconCache;
use crate::inspect::Raster;
use crate::layout::{Content, Frame, FrameNode, Look, input_text, text_width};
use crate::style::{Corners, ResolvedText, Shadow};
use crate::text::{Paragraph, Target, TextSystem, physical, physical_pos};

/// The caret shows for this long, then hides for as long.
const BLINK: f32 = 0.53;
const CUBIC_ARC: f32 = 0.552_284_8;

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

/// Caches that live across frames.
#[derive(Default)]
pub struct Painter {
    icons: IconCache,
    shadows: FrameCache<ShadowKey, Option<ShadowImage>>,
    /// Images scaled to the size they are drawn at.
    images: FrameCache<ImageKey, Option<ScaledImage>>,
    blink: Option<clock::TimerId>,
    /// Window-sized pixmaps groups paint aside into (see [`effects`]), kept
    /// for the next frame.
    layers: Vec<Pixmap>,
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

fn grow(r: Corners, d: f32) -> Corners {
    Corners {
        top_left: (r.top_left + d).max(0.0),
        top_right: (r.top_right + d).max(0.0),
        bottom_right: (r.bottom_right + d).max(0.0),
        bottom_left: (r.bottom_left + d).max(0.0),
    }
}

fn solid(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color.to_skia());
    paint.anti_alias = true;
    paint
}

/// Physical-pixel clip masks, made once per distinct clip per frame.
struct Clips {
    masks: HashMap<[i32; 8], Mask>,
    size: (u32, u32),
    scale: f32,
}

impl Clips {
    fn key(&self, b: Bounds, r: Corners) -> [i32; 8] {
        let s = self.scale;
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

    /// Make the mask `node` paints through, if it is not made yet.
    fn prepare(&mut self, node: &FrameNode) {
        let Some(b) = node.clip else { return };
        let key = self.key(b, node.clip_radius);
        if self.masks.contains_key(&key) {
            return;
        }
        let Some(mut mask) = Mask::new(self.size.0, self.size.1) else {
            return;
        };
        if let Some(path) = rrect(b.snapped(self.scale), node.clip_radius) {
            let rounded = !node.clip_radius.is_zero();
            mask.fill_path(
                &path,
                FillRule::Winding,
                rounded,
                Transform::from_scale(self.scale, self.scale),
            );
        }
        self.masks.insert(key, mask);
    }

    /// The mask `node` paints through, once prepared.
    fn get(&self, node: &FrameNode) -> Option<&Mask> {
        self.masks.get(&self.key(node.clip?, node.clip_radius))
    }
}

impl Painter {
    /// Paint `frame` over `background` at `scale` physical pixels per logical pixel.
    pub fn paint(&mut self, frame: &Frame, text: &mut TextSystem, pixmap: &mut Pixmap, scale: f32, background: Color) {
        pixmap.fill(background.to_skia());
        let mut clips = Clips {
            masks: HashMap::new(),
            size: (pixmap.width(), pixmap.height()),
            scale,
        };
        let mut blinking = false;
        let mut start = 0;
        while let Some(first) = frame.nodes.get(start) {
            let end = frame
                .nodes
                .iter()
                .skip(start)
                .position(|n| n.layer != first.layer)
                .map_or(frame.nodes.len(), |n| start.saturating_add(n));
            blinking |= self.paint_nodes(frame, start..end, pixmap, text, &mut clips, None);
            self.scrollbars(frame, start..end, pixmap, &mut clips);
            start = end;
        }
        self.schedule_blink(blinking);
        self.icons.end_frame();
        self.shadows.end_frame();
        self.images.end_frame();
    }

    /// The icon, shadow, scaled image and group-layer images held.
    pub(crate) fn footprint(&self) -> (Raster, Raster, Raster, Raster) {
        (
            self.icons.footprint(),
            Raster::of(
                self.shadows
                    .values()
                    .flatten()
                    .flat_map(|s| s.parts.iter().map(|(_, _, p)| p)),
            ),
            Raster::of(self.images.values().flatten().map(|i| &i.pixels)),
            Raster::of(&self.layers),
        )
    }

    /// Paint `range` of one layer's nodes in order, a node whose effects
    /// need it with its children as one group; returns whether a caret blinks.
    /// Inside a scaled or turned group, `lifted` is the clip from outside it,
    /// which the group applies once transformed: a node clipped by nothing
    /// else paints unclipped.
    fn paint_nodes(
        &mut self,
        frame: &Frame,
        range: std::ops::Range<usize>,
        pixmap: &mut Pixmap,
        text: &mut TextSystem,
        clips: &mut Clips,
        lifted: Option<Option<Bounds>>,
    ) -> bool {
        let mut blinking = false;
        let mut i = range.start;
        while let Some(node) = frame.nodes.get(i).filter(|_| range.contains(&i)) {
            let next = i.saturating_add(1);
            if effects::grouped(node) {
                let end = (next..range.end)
                    .find(|&k| !frame.ancestors(k).any(|a| a == i))
                    .unwrap_or(range.end);
                blinking |= self.paint_group(frame, i..end, pixmap, text, clips, lifted);
                i = end;
            } else {
                blinking |= self.paint_node(node, pixmap, text, clips, lifted);
                i = next;
            }
        }
        blinking
    }

    /// Paint the node at `subtree.start` and its children aside, then mask,
    /// filter, transform and composite them onto `pixmap`.
    fn paint_group(
        &mut self,
        frame: &Frame,
        subtree: std::ops::Range<usize>,
        pixmap: &mut Pixmap,
        text: &mut TextSystem,
        clips: &mut Clips,
        lifted: Option<Option<Bounds>>,
    ) -> bool {
        let size = (pixmap.width(), pixmap.height());
        // Fully transparent, the group shows nothing.
        let Some(node) = frame
            .nodes
            .get(subtree.start)
            .filter(|n| !n.look.invisible && n.look.opacity > 0.0)
        else {
            return false;
        };
        let transformed = !node.look.transform.is_identity();
        let region = if transformed || node.look.opacity < 1.0 {
            effects::subtree_region(frame.nodes.get(subtree.clone()).unwrap_or_default(), clips.scale, size)
        } else {
            effects::group_region(node, clips.scale, size)
        };
        let Some(region) = region else {
            return false;
        };
        // A transformed group is clipped from outside once transformed.
        let inner = if transformed {
            clips.prepare(node);
            Some(node.clip)
        } else {
            lifted
        };
        self.layers.retain(|l| (l.width(), l.height()) == size);
        let Some(mut layer) = self.layers.pop().or_else(|| Pixmap::new(size.0, size.1)) else {
            return false;
        };
        effects::clear(&mut layer, region);
        let children = subtree.start.saturating_add(1)..subtree.end;
        let blinking = self.paint_node(node, &mut layer, text, clips, inner)
            | self.paint_nodes(frame, children, &mut layer, text, clips, inner);
        let outer = if transformed { clips.get(node) } else { None };
        effects::composite(&layer, region, node, clips.scale, pixmap, outer);
        self.layers.push(layer);
        blinking
    }

    /// Paint one node's box and content; returns whether a caret blinks.
    fn paint_node(
        &mut self,
        node: &FrameNode,
        pixmap: &mut Pixmap,
        text: &mut TextSystem,
        clips: &mut Clips,
        lifted: Option<Option<Bounds>>,
    ) -> bool {
        if node.look.invisible
            || node
                .clip
                .is_some_and(|c| c.intersect(&node.bounds.outset(16.0)).is_empty())
        {
            return false;
        }
        clips.prepare(node);
        let mut target = Target {
            pixmap,
            scale: clips.scale,
            clip: if lifted == Some(node.clip) {
                None
            } else {
                clips.get(node)
            },
        };
        self.paint_box(&mut target, node, text);
        self.paint_content(&mut target, node, text)
    }

    fn schedule_blink(&mut self, blinking: bool) {
        if let Some(t) = self.blink.take() {
            clock::cancel(t);
        }
        if blinking {
            let t = clock::frame_time();
            let next = (t / BLINK).floor().mul_add(BLINK, BLINK) - t;
            self.blink = Some(clock::after(Duration::from_secs_f32(next.max(0.01)), || {}));
        }
    }

    fn paint_box(&mut self, target: &mut Target<'_>, node: &FrameNode, text: &mut TextSystem) {
        let Look {
            background,
            border,
            border_color,
            radius,
            shadows,
            ring,
            ..
        } = &node.look;
        let b = node.bounds.snapped(target.scale);
        let transform = Transform::from_scale(target.scale, target.scale);
        for s in shadows {
            self.paint_shadow(target, b, *radius, s);
        }
        if let Some(ring) = ring.filter(|r| r.width > 0.0 && !r.color.is_transparent())
            && let Some(path) = band(b.outset(ring.width), grow(*radius, ring.width), b, *radius)
        {
            target
                .pixmap
                .fill_path(&path, &solid(ring.color), FillRule::EvenOdd, transform, target.clip);
        }
        // A background clipped to the text is painted with the text.
        if let Some(bg) = background.filter(|c| !c.is_transparent())
            && effects::text_background(node).is_none()
            && let Some(path) = rrect(b, *radius)
        {
            target
                .pixmap
                .fill_path(&path, &solid(bg), FillRule::Winding, transform, target.clip);
        }
        if let Some(img) = effects::box_background(node) {
            effects::paint_background(target, node, img, text);
        }
        let has_border = border.top > 0.0 || border.right > 0.0 || border.bottom > 0.0 || border.left > 0.0;
        if has_border && let Some(color) = border_color.filter(|c| !c.is_transparent()) {
            let inner = b.inset(border.top, border.right, border.bottom, border.left);
            let inner_r = Corners {
                top_left: (radius.top_left - border.top.max(border.left)).max(0.0),
                top_right: (radius.top_right - border.top.max(border.right)).max(0.0),
                bottom_right: (radius.bottom_right - border.bottom.max(border.right)).max(0.0),
                bottom_left: (radius.bottom_left - border.bottom.max(border.left)).max(0.0),
            };
            if let Some(path) = band(b, *radius, inner, inner_r) {
                target
                    .pixmap
                    .fill_path(&path, &solid(color), FillRule::EvenOdd, transform, target.clip);
            }
        }
    }

    fn paint_shadow(&mut self, target: &mut Target<'_>, b: Bounds, radius: Corners, s: &Shadow) {
        let color = s.color;
        if color.is_transparent() {
            return;
        }
        let scale = target.scale;
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
                target.pixmap.draw_pixmap(
                    x.saturating_add(*dx),
                    y.saturating_add(*dy),
                    part.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    target.clip,
                );
            }
        }
    }

    /// Paint what the node holds; returns whether a caret is blinking.
    fn paint_content(&mut self, target: &mut Target<'_>, node: &FrameNode, text: &mut TextSystem) -> bool {
        // At full opacity: its own, below 1, is its group's to composite at.
        match &node.content {
            Content::None => false,
            Content::Shapes(shapes) => {
                paint_shapes(target, node.bounds.snapped(target.scale), shapes);
                false
            }
            Content::Image { source, radius } => {
                self.paint_image(target, node.bounds.snapped(target.scale), source, *radius);
                false
            }
            Content::Text { content, text: t, runs } => {
                let area = &node.content_box;
                let p = Paragraph { content, runs };
                if let Some(img) = effects::text_background(node) {
                    effects::paint_text_through(target, node, img, text, p, t, text_width(t, area));
                } else {
                    text.draw(target, p, t, text_width(t, area), (area.x, area.y), 1.0);
                }
                false
            }
            Content::Icon {
                icon,
                color,
                stroke_width,
            } => {
                let scale = target.scale;
                let side = node.bounds.width.min(node.bounds.height);
                let px = physical(side, scale);
                if let Some(img) = self.icons.get(*icon, px, *color, *stroke_width) {
                    let x = physical_pos(node.bounds.x + (node.bounds.width - side) / 2.0, scale);
                    let y = physical_pos(node.bounds.y + (node.bounds.height - side) / 2.0, scale);
                    target.pixmap.draw_pixmap(
                        x,
                        y,
                        img.as_ref(),
                        &PixmapPaint::default(),
                        Transform::identity(),
                        target.clip,
                    );
                }
                false
            }
            Content::Input {
                edit,
                placeholder,
                placeholder_color,
                selection_color,
                text: t,
                focused,
                multiline,
                mode,
            } => {
                let area = &node.content_box;
                let it = input_text(text, &edit.text, t, *area, *multiline, edit.caret);
                let (line_t, width, origin) = (it.t, it.width, it.origin);
                let caret = text.caret(&edit.text, &line_t, width, edit.caret);
                let mut clip_target = Target {
                    pixmap: target.pixmap,
                    scale: target.scale,
                    clip: target.clip,
                };
                if *focused && edit.has_selection() {
                    let (from, to) = edit.selection();
                    for r in text.selection(&edit.text, &line_t, width, from, to) {
                        let b = Bounds {
                            x: origin.0 + r.x,
                            y: origin.1 + r.y,
                            ..r
                        }
                        .intersect(&area.outset(1.0));
                        clip_target.fill_rect(b, *selection_color);
                    }
                }
                if edit.text.is_empty() {
                    text.draw(
                        &mut clip_target,
                        placeholder.as_str(),
                        &ResolvedText {
                            color: *placeholder_color,
                            ..line_t
                        },
                        width,
                        origin,
                        1.0,
                    );
                } else {
                    text.draw(&mut clip_target, edit.text.as_str(), &line_t, width, origin, 1.0);
                }
                let on = (clock::frame_time() / BLINK).floor().rem_euclid(2.0) < 1.0;
                if *focused && on && !mode.read_only {
                    let b = Bounds {
                        x: (origin.0 + caret.x).min(area.right() - 1.0),
                        y: origin.1 + caret.y,
                        width: 1.0,
                        height: caret.height,
                    };
                    clip_target.fill_rect(b, t.color);
                }
                *focused
            }
        }
    }

    fn scrollbars(&mut self, frame: &Frame, range: std::ops::Range<usize>, pixmap: &mut Pixmap, clips: &mut Clips) {
        let scale = clips.scale;
        for node in frame.nodes.get(range).unwrap_or_default() {
            let Some(scroll) = node.scroll.filter(|s| s.max.1 > 0.0 && !s.thumb.is_transparent()) else {
                continue;
            };
            let b = node.bounds.snapped(scale);
            let content_h = b.height + scroll.max.1;
            let thumb_h = (b.height * b.height / content_h).max(20.0);
            let travel = (b.height - thumb_h - 4.0).max(0.0);
            let thumb = Bounds {
                x: b.right() - 8.0,
                y: b.y + 2.0 + travel * (scroll.offset.1 / scroll.max.1),
                width: 6.0,
                height: thumb_h,
            };
            clips.prepare(node);
            if let Some(path) = rrect(thumb, Corners::all(3.0)) {
                pixmap.fill_path(
                    &path,
                    &solid(scroll.thumb),
                    FillRule::Winding,
                    Transform::from_scale(scale, scale),
                    clips.get(node),
                );
            }
        }
    }
}

fn paint_shapes(target: &mut Target<'_>, b: Bounds, shapes: &[crate::shape::Shape]) {
    use crate::shape::PathCommand;
    let transform = Transform::from_scale(target.scale, target.scale).pre_translate(b.x, b.y);
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
            target
                .pixmap
                .fill_path(&path, &solid(fill), FillRule::Winding, transform, target.clip);
        }
        if let Some((color, width)) = shape.stroke {
            let stroke = tiny_skia::Stroke {
                width,
                line_cap: tiny_skia::LineCap::Round,
                line_join: tiny_skia::LineJoin::Round,
                ..tiny_skia::Stroke::default()
            };
            target
                .pixmap
                .stroke_path(&path, &solid(color), &stroke, transform, target.clip);
        }
    }
}

/// Draw `source` scaled to cover `b`, centred, inside its rounded corners.
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
        tiny_skia::SpreadMode::Pad,
        tiny_skia::FilterQuality::Bilinear,
        1.0,
        Transform::from_translate((w - iw * fit) / 2.0, (h - ih * fit) / 2.0).pre_scale(fit, fit),
    );
    let paint = Paint {
        shader: pattern,
        ..Paint::default()
    };
    pixels.fill_rect(
        tiny_skia::Rect::from_xywh(0.0, 0.0, w, h)?,
        &paint,
        Transform::identity(),
        None,
    );
    Some(ScaledImage {
        _source: source.clone(),
        pixels,
    })
}

impl Painter {
    /// Paint `source` covering `b` (snapped to device pixels), its corners
    /// rounded by `radius`, from its pixels scaled to that size once.
    fn paint_image(&mut self, target: &mut Target<'_>, b: Bounds, source: &crate::image::ImageSource, radius: Corners) {
        let scale = target.scale;
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
            tiny_skia::SpreadMode::Pad,
            tiny_skia::FilterQuality::Nearest,
            1.0,
            Transform::from_translate(b.x, b.y).pre_scale(1.0 / scale, 1.0 / scale),
        );
        let paint = Paint {
            shader: pattern,
            anti_alias: true,
            ..Paint::default()
        };
        target.pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::from_scale(scale, scale),
            target.clip,
        );
    }
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
        let alpha = crate::convert::unit_to_u8(a * f32::from(a8) / 255.0);
        *px = tiny_skia::ColorU8::from_rgba(r8, g8, b8, alpha).premultiply();
    }
    let [cx, cy, cw, ch] = key.cutout.map(crate::convert::i32_to_f32);
    let cut = Bounds {
        x: cx + p,
        y: cy + p,
        width: cw,
        height: ch,
    };
    let own = own.scaled(scale);
    if let Some(path) = rrect(cut, own) {
        let mut clear = solid(Color::BLACK);
        clear.blend_mode = tiny_skia::BlendMode::Clear;
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
