//! Recording a [`Frame`] as a [`Scene`]: shadows, rounded backgrounds and
//! borders, focus rings, text, icons, text inputs and scrollbars, each
//! clipped to what its ancestors allow, and the groups effects composite.
//! What the primitives become in pixels is a renderer's ([`crate::cpu`],
//! [`crate::gpu`]).

use std::time::Duration;

use crate::clock;
use crate::color::Color;
use crate::effects;
use crate::geometry::Bounds;
use crate::layout::{Content, Frame, FrameNode, Look, input_text, text_width};
use crate::scene::{Clip, Group, GroupTransform, Item, Scene};
use crate::style::{Corners, ResolvedText};
use crate::text::{Paragraph, Target, TextSystem, physical, physical_pos};

/// The caret shows for this long, then hides for as long.
const BLINK: f32 = 0.53;

/// What recording keeps across frames: the caret's blink timer.
#[derive(Default)]
pub struct Painter {
    blink: Option<clock::TimerId>,
}

pub(crate) fn grow(r: Corners, d: f32) -> Corners {
    Corners {
        top_left: (r.top_left + d).max(0.0),
        top_right: (r.top_right + d).max(0.0),
        bottom_right: (r.bottom_right + d).max(0.0),
        bottom_left: (r.bottom_left + d).max(0.0),
    }
}

/// The clip `node` paints through.
fn clip_of(node: &FrameNode) -> Option<Clip> {
    node.clip.map(|bounds| Clip {
        bounds,
        radius: node.clip_radius,
    })
}

impl Painter {
    /// Record `frame` over `background`, `size` physical pixels at `scale`
    /// physical pixels per logical pixel.
    pub fn record(
        &mut self,
        frame: &Frame,
        text: &mut TextSystem,
        size: (u32, u32),
        scale: f32,
        background: Color,
    ) -> Scene {
        let mut scene = Scene::new(size, scale, Some(background));
        let mut blinking = false;
        let mut start = 0;
        while let Some(first) = frame.nodes.get(start) {
            let end = frame
                .nodes
                .iter()
                .skip(start)
                .position(|n| n.layer != first.layer)
                .map_or(frame.nodes.len(), |n| start.saturating_add(n));
            blinking |= self.record_nodes(frame, start..end, &mut scene.items, size, scale, text, None);
            scrollbars(frame, start..end, &mut scene.items, scale);
            start = end;
        }
        self.schedule_blink(blinking);
        scene
    }

    /// Record `range` of one layer's nodes in order, a node whose effects
    /// need it with its children as one group; returns whether a caret blinks.
    /// Inside a scaled or turned group, `lifted` is the clip from outside it,
    /// which the group applies once transformed: a node clipped by nothing
    /// else paints unclipped.
    #[expect(clippy::too_many_arguments, reason = "the recursion's state, passed down as is")]
    fn record_nodes(
        &mut self,
        frame: &Frame,
        range: std::ops::Range<usize>,
        items: &mut Vec<Item>,
        size: (u32, u32),
        scale: f32,
        text: &mut TextSystem,
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
                blinking |= self.record_group(frame, i..end, items, size, scale, text, lifted);
                i = end;
            } else {
                let mut target = Target {
                    items,
                    scale,
                    size,
                    clip: None,
                };
                blinking |= record_node(node, &mut target, text, lifted);
                i = next;
            }
        }
        blinking
    }

    /// Record the node at `subtree.start` and its children as a group, to be
    /// masked, filtered, transformed and composited as one.
    #[expect(clippy::too_many_arguments, reason = "the recursion's state, passed down as is")]
    fn record_group(
        &mut self,
        frame: &Frame,
        subtree: std::ops::Range<usize>,
        items: &mut Vec<Item>,
        size: (u32, u32),
        scale: f32,
        text: &mut TextSystem,
        lifted: Option<Option<Bounds>>,
    ) -> bool {
        // Fully transparent, the group shows nothing.
        let Some(node) = frame
            .nodes
            .get(subtree.start)
            .filter(|n| !n.look.invisible && n.look.opacity > 0.0)
        else {
            return false;
        };
        let t = node.look.transform;
        let transformed = !t.is_identity();
        let region = if transformed || node.look.opacity < 1.0 {
            effects::subtree_region(frame.nodes.get(subtree.clone()).unwrap_or_default(), scale, size)
        } else {
            effects::group_region(node, scale, size)
        };
        let Some(region) = region else {
            return false;
        };
        // A transformed group is clipped from outside once transformed.
        let inner = if transformed { Some(node.clip) } else { lifted };
        let mut inside = Vec::new();
        let children = subtree.start.saturating_add(1)..subtree.end;
        let blinking = {
            let mut target = Target {
                items: &mut inside,
                scale,
                size,
                clip: None,
            };
            record_node(node, &mut target, text, inner)
        } | self.record_nodes(frame, children, &mut inside, size, scale, text, inner);
        items.push(Item::Group(Box::new(Group {
            region,
            items: inside,
            mask: effects::mask_gradient(node),
            filters: node.look.effects.filters.clone(),
            opacity: node.look.opacity,
            transform: transformed.then_some(GroupTransform {
                origin: (
                    (node.bounds.x + t.origin.0) * scale,
                    (node.bounds.y + t.origin.1) * scale,
                ),
                scale: t.scale,
                rotate: t.rotate,
            }),
            clip: if transformed { clip_of(node) } else { None },
        })));
        blinking
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
}

/// Record one node's box and content; returns whether a caret blinks.
fn record_node(
    node: &FrameNode,
    target: &mut Target<'_>,
    text: &mut TextSystem,
    lifted: Option<Option<Bounds>>,
) -> bool {
    if node.look.invisible
        || node
            .clip
            .is_some_and(|c| c.intersect(&node.bounds.outset(16.0)).is_empty())
    {
        return false;
    }
    target.clip = if lifted == Some(node.clip) { None } else { clip_of(node) };
    record_box(target, node, text);
    record_content(target, node, text)
}

fn record_box(target: &mut Target<'_>, node: &FrameNode, text: &mut TextSystem) {
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
    let clip = target.clip;
    for s in shadows.iter().filter(|s| !s.color.is_transparent()) {
        target.items.push(Item::Shadow {
            bounds: b,
            radius: *radius,
            shadow: *s,
            clip,
        });
    }
    if let Some(ring) = ring.filter(|r| r.width > 0.0 && !r.color.is_transparent()) {
        target.items.push(Item::Band {
            outer: b.outset(ring.width),
            outer_radius: grow(*radius, ring.width),
            inner: b,
            inner_radius: *radius,
            color: ring.color,
            clip,
        });
    }
    // A background clipped to the text is painted with the text.
    if let Some(bg) = background.filter(|c| !c.is_transparent())
        && effects::text_background(node).is_none()
    {
        target.items.push(Item::RoundedRect {
            bounds: b,
            radius: *radius,
            color: bg,
            clip,
        });
    }
    if let Some(img) = effects::box_background(node) {
        effects::record_background(target, node, img, text);
    }
    let has_border = border.top > 0.0 || border.right > 0.0 || border.bottom > 0.0 || border.left > 0.0;
    if has_border && let Some(color) = border_color.filter(|c| !c.is_transparent()) {
        let inner = b.inset(border.top, border.right, border.bottom, border.left);
        let inner_radius = Corners {
            top_left: (radius.top_left - border.top.max(border.left)).max(0.0),
            top_right: (radius.top_right - border.top.max(border.right)).max(0.0),
            bottom_right: (radius.bottom_right - border.bottom.max(border.right)).max(0.0),
            bottom_left: (radius.bottom_left - border.bottom.max(border.left)).max(0.0),
        };
        target.items.push(Item::Band {
            outer: b,
            outer_radius: *radius,
            inner,
            inner_radius,
            color,
            clip,
        });
    }
}

/// Record what the node holds; returns whether a caret is blinking.
fn record_content(target: &mut Target<'_>, node: &FrameNode, text: &mut TextSystem) -> bool {
    let clip = target.clip;
    match &node.content {
        Content::None => false,
        Content::Shapes(shapes) => {
            let b = node.bounds.snapped(target.scale);
            target.items.push(Item::Shapes {
                origin: (b.x, b.y),
                shapes: shapes.clone(),
                clip,
            });
            false
        }
        Content::Image { source, radius } => {
            target.items.push(Item::Image {
                source: source.clone(),
                bounds: node.bounds.snapped(target.scale),
                radius: *radius,
                clip,
            });
            false
        }
        Content::Text { content, text: t, runs } => {
            let area = &node.content_box;
            let p = Paragraph { content, runs };
            if let Some(img) = effects::text_background(node) {
                effects::record_text_through(target, node, img, text, p, t, text_width(t, area));
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
            target.items.push(Item::Icon {
                icon: *icon,
                size: physical(side, scale),
                color: *color,
                stroke_width: *stroke_width,
                x: physical_pos(node.bounds.x + (node.bounds.width - side) / 2.0, scale),
                y: physical_pos(node.bounds.y + (node.bounds.height - side) / 2.0, scale),
                clip,
            });
            false
        }
        Content::Input {
            edit,
            preedit,
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
            if *focused && edit.has_selection() {
                let (from, to) = edit.selection();
                for r in text.selection(&edit.text, &line_t, width, from, to) {
                    let b = Bounds {
                        x: origin.0 + r.x,
                        y: origin.1 + r.y,
                        ..r
                    }
                    .intersect(&area.outset(1.0));
                    target.fill_rect(b, *selection_color);
                }
            }
            if edit.text.is_empty() {
                text.draw(
                    target,
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
                text.draw(target, edit.text.as_str(), &line_t, width, origin, 1.0);
            }
            // What an input method is composing is underlined, as a
            // browser marks a composition, until it is committed.
            if let Some((from, to)) = *preedit {
                for r in text.selection(&edit.text, &line_t, width, from, to) {
                    let b = Bounds {
                        x: origin.0 + r.x,
                        y: origin.1 + r.y + r.height - 1.0,
                        width: r.width,
                        height: 1.0,
                    }
                    .intersect(&area.outset(1.0));
                    target.fill_rect(b, t.color);
                }
            }
            let on = (clock::frame_time() / BLINK).floor().rem_euclid(2.0) < 1.0;
            if *focused && on && !mode.read_only {
                let b = Bounds {
                    x: (origin.0 + caret.x).min(area.right() - 1.0),
                    y: origin.1 + caret.y,
                    width: 1.0,
                    height: caret.height,
                };
                target.fill_rect(b, t.color);
            }
            *focused
        }
    }
}

fn scrollbars(frame: &Frame, range: std::ops::Range<usize>, items: &mut Vec<Item>, scale: f32) {
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
        items.push(Item::RoundedRect {
            bounds: thumb,
            radius: Corners::all(3.0),
            color: scroll.thumb,
            clip: clip_of(node),
        });
    }
}
