use std::rc::Rc;
use std::sync::Arc;

use taffy::Layout;

use crate::color::Color;
use crate::geometry::Bounds;
use crate::motion::Motion;
use crate::scroll::{ScrollChild, ScrollLayout};
use crate::style::{Corners, Direction, Display, Edges, Overflow, Position, ResolvedText};
use crate::text::{InlineSlot, Paragraph, TextSystem};

use super::write::Write;
use super::{
    Changed, Content, Extra, FieldSettled, Frame, FrameNode, LayoutTree, Leaf, Look, Moving, Node, Pending, Scroll,
    UiState, caret_reveal, field_reach, keep_spare, leaf, need, text_width,
};

/// Where a placed node puts its children.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Placed {
    /// The node's index in the frame.
    index: usize,
    /// The children's origin in their paint space: the node's box there.
    origin: (f32, f32),
    /// What takes the children's paint space to the window: the node's,
    /// less its scroll offset.
    offset: (f32, f32),
    /// The clip its children paint inside, its corners, and the node that set it.
    clip: Option<Bounds>,
    clip_radius: Corners,
    clip_curve: f32,
    clipper: Option<usize>,
    /// It lays its children out in block flow, right to left: the content
    /// box's right edge its in-flow children keep their margin to.
    rtl_block: Option<f32>,
    /// It or an ancestor is an atomic inline its paragraph's clamp or cut
    /// hides ([`engine::Node::inline_hidden`](super::engine::Node::inline_hidden)): its children paint nothing
    /// and take no hit.
    hidden: bool,
}

/// How far a node's children reach from its border box's top-left corner.
#[derive(Clone, Copy)]
struct Reach {
    left: f32,
    right: f32,
    down: f32,
}

impl Default for Reach {
    /// No children: nothing reaches anywhere.
    fn default() -> Self {
        Self {
            left: f32::INFINITY,
            right: 0.0,
            down: 0.0,
        }
    }
}

/// Where a node's border box sits in its parent's: Taffy's place for it,
/// mirrored in a right-to-left block container (Taffy lays block children
/// out from the left whatever the `direction`), where an in-flow child keeps
/// its right margin to the content box's right edge, as CSS places it.
fn location(rtl_block: Option<f32>, position: Position, l: &Layout) -> (f32, f32) {
    match rtl_block {
        Some(right) if position == Position::Relative => (right - l.margin.right - l.size.width, l.location.y),
        _ => (l.location.x, l.location.y),
    }
}

/// Where a node is placed from: its parent's placing, or the layer's origin.
#[derive(Clone, Copy)]
struct From<'p> {
    parent: Option<&'p Placed>,
    origin: (f32, f32),
    opened: Option<crate::placement::Placement>,
}

impl LayoutTree {
    /// Write `root`'s subtree (one layer, depth-first) into `frame`, with
    /// the root at `origin`: every node pushed anew (`Write::All`), or in
    /// place, only the nodes that need it (`Write::Changed`). `moved` says
    /// the layer's origin moved: every node of it is written.
    #[expect(clippy::too_many_arguments, reason = "a layer, where it goes and what places it")]
    pub(super) fn place(
        &mut self,
        frame: &mut Frame,
        root: &mut Node,
        origin: (f32, f32),
        opened: Option<crate::placement::Placement>,
        moved: bool,
        state: &UiState,
        motion: &mut Motion,
        text: &mut TextSystem,
        write: Write,
    ) {
        let from = From {
            parent: None,
            origin,
            opened,
        };
        let force = write == Write::All || moved;
        self.place_node(frame, root, from, force, force, state, motion, text, write);
    }

    /// Place `node`: write its frame node when `again` (or it needs it, or
    /// layout moved it), then its children that need it; with `all`, its
    /// whole subtree.
    #[expect(clippy::too_many_arguments, reason = "a node, where it goes and what places it")]
    fn place_node(
        &mut self,
        frame: &mut Frame,
        node: &mut Node,
        from: From<'_>,
        again: bool,
        all: bool,
        state: &UiState,
        motion: &mut Motion,
        text: &mut TextSystem,
        write: Write,
    ) {
        let u = node.user;
        let Some(mut ex) = self.take_extra(u) else { return };
        let moved = node.take_moved();
        let ran = node.run();
        let ran_again = ran != ex.ran;
        let recompute = again || all || ex.own & need::PLACE != 0 || moved || ran_again;
        let mut kids_changed = false;
        if recompute {
            self.visits.placed = self.visits.placed.saturating_add(1);
            let index = match write {
                Write::All => frame.nodes.len(),
                Write::Changed => usize::try_from(ex.index).unwrap_or(usize::MAX),
            };
            let mut tracker = std::mem::take(&mut ex.place);
            let (probe, marks) = (self.probe, Rc::clone(&self.marks));
            let boxes = std::mem::take(&mut ex.spare_boxes);
            let (fresh, kids) = tracker.run(probe, &marks, u, need::PLACE, || {
                self.frame_node(&ex, node, from, index, state, boxes, text)
            });
            ex.place = tracker;
            if ex.measures {
                motion.natural(&ex.path, fresh.bounds.height);
            }
            let fresh_behaviour = std::mem::take(&mut ex.fresh_behaviour);
            let spans = !ex.behaviour.spans.is_empty();
            match write {
                Write::All => {
                    frame.nodes.push(fresh);
                    frame.behaviour.push(Rc::clone(&ex.behaviour));
                    if spans {
                        locate_spans(frame, index, text);
                    }
                }
                Write::Changed => {
                    let (Some(slot), Some(b)) = (frame.nodes.get_mut(index), frame.behaviour.get_mut(index)) else {
                        self.put_extra(u, ex);
                        return;
                    };
                    debug_assert!(slot.path == fresh.path, "a frame node written in place is another node");
                    let mut changed = false;
                    // Its spans lie where they did unless its text or
                    // where it lies changed: a colour changing finds none.
                    let relocate = fresh_behaviour || !lays_text_alike(slot, &fresh);
                    if *slot == fresh {
                        if let Content::Text { boxes, .. } = fresh.content {
                            ex.spare_boxes = boxes;
                        }
                    } else {
                        let repainted = slot.look != fresh.look || slot.content != fresh.content;
                        frame.changes.log.push(Changed {
                            index: u32::try_from(index).unwrap_or(u32::MAX),
                            before: slot.bounds,
                            repainted,
                        });
                        let old = std::mem::replace(slot, fresh);
                        // What it showed is the node's spare: it writes the
                        // next frame node's there.
                        if let Content::Text { runs, boxes, .. } = old.content {
                            ex.spare_boxes = boxes;
                            if matches!(&ex.content, Pending::Text { runs: now, .. }
                                if !Rc::ptr_eq(now, &runs) && now.len() == runs.len())
                            {
                                keep_spare(&mut ex.spare_runs, runs);
                            }
                        }
                        changed = true;
                    }
                    if fresh_behaviour {
                        let handles = b.scroll.is_some() != ex.behaviour.scroll.is_some();
                        *b = Rc::clone(&ex.behaviour);
                        if !changed {
                            frame.changes.log.push(Changed {
                                index: u32::try_from(index).unwrap_or(u32::MAX),
                                before: frame.nodes.get(index).map_or_else(Bounds::default, |n| n.bounds),
                                repainted: false,
                            });
                        }
                        changed = true;
                        if handles {
                            frame.handles = (0..frame.behaviour.len())
                                .filter(|&i| frame.behaviour.get(i).is_some_and(|b| b.scroll.is_some()))
                                .collect();
                        }
                    }
                    if changed && spans && relocate {
                        locate_spans_at(frame, index, text);
                    }
                }
            }
            ex.index = u32::try_from(index).unwrap_or(u32::MAX);
            ex.ran = ran;
            kids_changed = ex.kids != Some(kids);
            ex.kids = Some(kids);
            ex.own &= !need::PLACE;
        }
        let Some(kids) = ex.kids else {
            self.put_extra(u, ex);
            return;
        };
        self.put_extra(u, ex);
        // Out-of-flow boxes below are placed by this node's run, wherever
        // they lie in its subtree: a run places the whole subtree again.
        let all_below = all || (ran_again && !node.hoisted().is_empty());
        let from = From {
            parent: Some(&kids),
            origin: from.origin,
            opened: from.opened,
        };
        for c in &mut node.children {
            let go = all_below
                || kids_changed
                || self.extra(c.user).is_some_and(|e| {
                    (e.own | e.below) & need::PLACE != 0 || (ran_again && (c.moved() || c.run() != e.ran))
                });
            if go {
                self.place_node(frame, c, from, kids_changed, all_below, state, motion, text, write);
            }
        }
        if !node.children.is_empty() {
            let below = node
                .children
                .iter()
                .fold(0, |b, c| b | self.extra(c.user).map_or(0, |e| e.own | e.below));
            if let Some(e) = self.extra_mut(u) {
                e.below = below;
            }
        }
    }

    /// The frame node `ex` is now, at `index` (where it is or goes), and
    /// what it passes its children; runs a scroll handle's and a measure
    /// handler's hooks.
    #[expect(
        clippy::too_many_arguments,
        reason = "a node, where it goes, and what lays out a field's text"
    )]
    fn frame_node(
        &self,
        ex: &Extra,
        node: &Node,
        from: From<'_>,
        index: usize,
        state: &UiState,
        mut boxes: Vec<InlineSlot>,
        text: &mut TextSystem,
    ) -> (FrameNode, Placed) {
        let l = node.layout();
        let parent = from.parent;
        let opened = from.opened;
        let hidden = parent.is_some_and(|p| p.hidden) || node.inline_hidden();
        let (x, y) = location(parent.and_then(|p| p.rtl_block), ex.style.position, l);
        let (base, above, clip, clip_radius, clip_curve, clipper) = parent
            .map_or((from.origin, (0.0, 0.0), None, Corners::default(), 1.0, None), |p| {
                (p.origin, p.offset, p.clip, p.clip_radius, p.clip_curve, p.clipper)
            });
        let shift = ex.style.transform.offset((l.size.width, l.size.height), opened);
        // A moving translation is a transform of the property tree (ADR-0016),
        // in whole device pixels as a scroll offset is (ADR-0020); a still one
        // is part of the node's place.
        let (still, moving) = if ex.translating {
            ((0.0, 0.0), crate::geometry::device_rounded(shift, state.scale))
        } else {
            (shift, (0.0, 0.0))
        };
        let space = Bounds {
            x: base.0 + x + still.0,
            y: base.1 + y + still.1,
            width: l.size.width,
            height: l.size.height,
        };
        let offset = (above.0 + moving.0, above.1 + moving.1);
        let bounds = Bounds {
            x: space.x + offset.0,
            y: space.y + offset.1,
            ..space
        };
        // A detached tree tells the window's code nothing it laid out.
        if let Some(f) = ex.behaviour.handlers.measured.as_ref().filter(|_| !self.detached) {
            // Run once its tree's lists are filled (`join_lists`), which it
            // may read; what it writes is read again by the pass it asks
            // for, not by this one.
            self.measured_due.borrow_mut().push((Rc::clone(f), bounds));
        }
        let padding_box = bounds.inset(l.border.top, l.border.right, l.border.bottom, l.border.left);
        let content_box = padding_box.inset(l.padding.top, l.padding.right, l.padding.bottom, l.padding.left);
        let space_content = space
            .inset(l.border.top, l.border.right, l.border.bottom, l.border.left)
            .inset(l.padding.top, l.padding.right, l.padding.bottom, l.padding.left);
        let rtl = ex.text.direction == Direction::Rtl;
        // A paragraph's children are where its lines put them.
        let rtl_block = (ex.style.display == Display::Block && rtl && node.leaf.is_none())
            .then_some(l.size.width - l.padding.right - l.border.right);

        let scrolls = ex.style.overflow_x == Overflow::Scroll || ex.style.overflow_y == Overflow::Scroll;
        let mut scroll: Option<Scroll> = None;
        if scrolls || ex.behaviour.scroll.is_some() {
            // How far the children reach (left, right, down), for scroll
            // limits, and where they lie for a scroll handle.
            let mut e = Reach::default();
            let mut tracked = Vec::new();
            for c in &node.children {
                let Some(ce) = self.extra(c.user) else { continue };
                let cl = c.layout();
                let (cx, cy) = location(rtl_block, ce.style.position, cl);
                e.left = e.left.min(cx - cl.margin.left);
                e.right = e.right.max(cx + cl.size.width + cl.margin.right);
                e.down = e.down.max(cy + cl.size.height + cl.margin.bottom);
                if ex.behaviour.scroll.is_some() {
                    tracked.push(ScrollChild {
                        key: ce.path.0.last().and_then(|seg| seg.element_key()),
                        bounds: Bounds {
                            x: cx,
                            y: cy,
                            width: cl.size.width,
                            height: cl.size.height,
                        },
                        focus_within: state
                            .focus_at
                            .as_ref()
                            .is_some_and(|(f, _)| f.0.starts_with(&ce.path.0)),
                    });
                }
            }
            scroll = scrolls.then(|| {
                // Offsets run from the inline start: right to left content
                // overflows to the left, and scrolls that way from 0.
                let max = (
                    if rtl {
                        (l.border.left + l.padding.left - e.left).max(0.0)
                    } else {
                        (e.right + l.padding.right + l.border.right - l.size.width).max(0.0)
                    },
                    (e.down + l.padding.bottom + l.border.bottom - l.size.height).max(0.0),
                );
                let id = || (ex.path.clone(), self.owner_of(ex));
                let want = match &ex.behaviour.scroll {
                    Some(h) => h.wanted(),
                    None => state.scroll.get(&id()).copied().unwrap_or_default(),
                };
                let generation = match &ex.behaviour.scroll {
                    Some(h) => h.generation(),
                    None => state.scroll_generation.get(&id()).copied().unwrap_or_default(),
                };
                Scroll {
                    offset: (want.0.clamp(0.0, max.0), want.1.clamp(0.0, max.1)),
                    applied: (0.0, 0.0),
                    max,
                    generation,
                    thumb: if ex.style.scrollbar_hidden {
                        Color::TRANSPARENT
                    } else {
                        ex.text.color.opacity(0.25)
                    },
                }
            });
            if let (Some(h), Some(s)) = (&ex.behaviour.scroll, scroll) {
                let laid = ScrollLayout {
                    path: ex.path.clone(),
                    bounds,
                    padding: Edges {
                        top: l.padding.top,
                        right: l.padding.right,
                        bottom: l.padding.bottom,
                        left: l.padding.left,
                    },
                    offset: s.offset,
                    max: s.max,
                    children: tracked,
                };
                if !self.detached {
                    crate::reactive::untracked(|| h.laid_out(laid));
                }
            }
        }
        // A text input scrolls its own content (its text, not children), as
        // a scroll container does: its offset is the window's state, as far
        // as its text reaches past its content box (`field_reach`), with no
        // scrollbar; paint draws its text under the offset, clipped.
        if let Pending::Input(input) = &ex.content {
            let edit = &input.shown;
            let focused = ex.focus_id.is_some() && ex.focus_id == state.focused;
            let max = field_reach(text, &edit.text, &ex.text, content_box, input.multiline, focused);
            // Its offset: its handle's (one field shows a handle), clamped:
            // one its text shrank under is written back, so a regrowth does
            // not bring the old one back. An edit asked to show its caret
            // since it last took an ask: it shows it as laid out now (its
            // box and value); the window's own tree, not a cold one, takes
            // it. Placing reads the view: a change of it places the node
            // again (not resolves it).
            let (asked, want, applied) = input.field.view.with(|v| (v.reveal, v.offset, v.applied));
            let mut offset = (want.0.clamp(0.0, max.0), want.1.clamp(0.0, max.1));
            let reveal = asked != applied;
            if reveal && !self.detached {
                let t = &ex.text;
                let reveal = (offset, max);
                offset = caret_reveal(
                    text,
                    &edit.text,
                    edit.caret,
                    t,
                    content_box,
                    input.multiline,
                    focused,
                    reveal,
                );
            }
            if (offset != want || reveal) && !self.detached {
                let mut fields = self.fields.borrow_mut();
                #[cfg(test)]
                if fields.len() == fields.capacity() {
                    crate::edit::FIELD_GROWS.with(|n| n.set(n.get().saturating_add(1)));
                }
                fields.push(FieldSettled {
                    field: input.field,
                    offset,
                    applied: asked,
                });
            }
            scroll = Some(Scroll {
                offset,
                applied: (0.0, 0.0),
                max,
                generation: state
                    .scroll_generation
                    .get(&(ex.path.clone(), self.owner_of(ex)))
                    .copied()
                    .unwrap_or_default(),
                thumb: Color::TRANSPARENT,
            });
        }
        let scrolled = scroll.map_or((0.0, 0.0), |s| crate::geometry::device_snapped(s.offset, state.scale));
        let applied = (if rtl { scrolled.0 } else { -scrolled.0 }, -scrolled.1);
        if let Some(s) = &mut scroll {
            s.applied = applied;
        }
        let (child_clip, child_radius, child_curve, child_clipper) = if ex.style.clips() {
            let r = ex.style.shape.used(bounds.width, bounds.height);
            let inner = Corners {
                top_left: (r.top_left - l.border.top.max(l.border.left)).max(0.0),
                top_right: (r.top_right - l.border.top.max(l.border.right)).max(0.0),
                bottom_right: (r.bottom_right - l.border.bottom.max(l.border.right)).max(0.0),
                bottom_left: (r.bottom_left - l.border.bottom.max(l.border.left)).max(0.0),
            };
            (
                Some(clip.map_or(padding_box, |c| c.intersect(&padding_box))),
                inner,
                ex.style.shape.curve(),
                Some(index),
            )
        } else {
            (clip, clip_radius, clip_curve, clipper)
        };

        let content = match &ex.content {
            Pending::None => Content::None,
            Pending::Text { content, runs } => Content::Text {
                content: Arc::clone(content),
                text: ex.text.clone(),
                runs: Rc::clone(runs),
                boxes: match &node.leaf {
                    Some(Leaf::Text { objects, .. }) if !objects.is_empty() => {
                        boxes.clear();
                        boxes.extend(leaf::slots(objects, node.inline_boxes()));
                        boxes
                    }
                    _ => Vec::new(),
                },
            },
            Pending::Icon { icon, stroke_width } => Content::Icon {
                icon: *icon,
                color: ex.text.color,
                stroke_width: *stroke_width,
            },
            Pending::Figures(shapes) => Content::Figures(shapes.clone()),
            Pending::Drawn(d) => Content::Drawn(d.clone()),
            Pending::Image(source) => Content::Image {
                source: source.clone(),
                radius: ex.style.shape.used(bounds.width, bounds.height),
            },
            Pending::Input(input) => Content::Input {
                edit: input.shown.clone(),
                preedit: input.preedit,
                placeholder: input.placeholder.clone(),
                placeholder_color: input.placeholder_color.unwrap_or(ex.text.color.opacity(0.5)),
                selection_color: input.selection_color.unwrap_or(Color::hex(0x0003_66d6).opacity(0.3)),
                text: ex.text.clone(),
                focused: ex.focus_id.is_some() && ex.focus_id == state.focused,
                multiline: input.multiline,
                mode: input.mode,
            },
        };
        let fresh = FrameNode {
            path: ex.path.clone(),
            name: ex.name,
            bounds,
            content_box,
            space,
            space_content,
            offset,
            translating: ex.translating,
            moving: ex.curves.as_ref().map(|curves| Moving {
                curves: std::sync::Arc::clone(curves),
                transform: ex.style.transform,
                above,
                opened,
            }),
            clip,
            clip_radius,
            clip_curve,
            clipper,
            layer: ex.layer,
            parent: parent.map(|p| p.index),
            look: Look {
                background: ex.style.background,
                border: ex.style.border_widths,
                border_color: ex.style.border_color,
                shape: ex.style.shape,
                shadows: ex.style.shadows.clone(),
                ring: ex.style.ring,
                decoration: ex.style.decoration.clone(),
                hit_slop: ex.style.hit_slop,
                opacity: ex.opacity,
                invisible: ex.invisible || hidden || ex.dormant,
                dormant: ex.dormant && !ex.invisible && !hidden,
                effects: ex.style.effects.clone(),
                transform: ex.style.transform.painted(bounds, opened),
                // Its effects alone read the colour (`currentColor`): a box
                // without them keeps none, and a provider's colour
                // changing above it repaints only what reads it.
                text: if ex.effects_read_color() {
                    ex.text.clone()
                } else {
                    ex.text.clone().shape()
                },
            },
            content,
            scroll,
        };
        let kids = Placed {
            index,
            origin: (space.x, space.y),
            offset: (offset.0 + applied.0, offset.1 + applied.1),
            clip: child_clip,
            clip_radius: child_radius,
            clip_curve: child_curve,
            clipper: child_clipper,
            rtl_block,
            hidden,
        };
        (fresh, kids)
    }
}

/// Frame nodes `a` and `b` lay their text out alike, in the same place:
/// what is drawn differs at most in what only paints.
fn lays_text_alike(a: &FrameNode, b: &FrameNode) -> bool {
    a.content_box == b.content_box
        && match (&a.content, &b.content) {
            (
                Content::Text {
                    content: c0,
                    text: t0,
                    runs: r0,
                    boxes: b0,
                },
                Content::Text {
                    content: c1,
                    text: t1,
                    runs: r1,
                    boxes: b1,
                },
            ) => {
                c0 == c1
                    && ResolvedText::shape(t0.clone()) == ResolvedText::shape(t1.clone())
                    && b0 == b1
                    && (Rc::ptr_eq(r0, r1)
                        || (r0.len() == r1.len()
                            && r0
                                .iter()
                                .zip(r1.iter())
                                .all(|(x, y)| Leaf::measuring_run(x) == Leaf::measuring_run(y))))
            }
            _ => false,
        }
}

/// Find where the pointer-taking spans of frame node `i` lie.
fn locate_spans_at(frame: &mut Frame, i: usize, text: &mut TextSystem) {
    let (Some(node), Some(behaviour)) = (frame.nodes.get(i), frame.behaviour.get_mut(i)) else {
        return;
    };
    if behaviour.spans.is_empty() {
        return;
    }
    let Content::Text {
        content,
        text: t,
        runs,
        boxes,
    } = &node.content
    else {
        return;
    };
    let area = node.content_box;
    let p = Paragraph { content, runs, boxes };
    for span in &mut Rc::make_mut(behaviour).spans {
        span.boxes = text
            .selection(p, t, text_width(t, &area), span.range.start, span.range.end)
            .into_iter()
            .map(|b| Bounds {
                x: area.x + b.x,
                y: area.y + b.y,
                ..b
            })
            .collect();
    }
}

/// Find where the pointer-taking spans of the frame node just pushed at `i` lie.
fn locate_spans(frame: &mut Frame, i: usize, text: &mut TextSystem) {
    locate_spans_at(frame, i, text);
}
