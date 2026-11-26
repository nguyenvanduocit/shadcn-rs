use std::collections::HashMap;

use taffy::AvailableSpace;

use crate::element::{ElementKey, Positioning};
use crate::geometry::{Bounds, NodePath};
use crate::placement::{EDGE_MARGIN, fitted_height, over, placed, position};
use crate::scroll::ScrollHandle;
use crate::text::TextSystem;

use super::{BoxStyle, Frame, LayerRoot, LayerTodo, LayoutTree, Len, Node, set_style};

/// The waiting layer that opened first: one kept from the last pass, oldest
/// first, else the first new one found.
pub(super) fn take_oldest(
    todo: &mut std::collections::VecDeque<LayerTodo>,
    old: &[LayerRoot],
    carried: &HashMap<NodePath, u64>,
) -> Option<LayerTodo> {
    let opened = |t: &LayerTodo| {
        old.iter()
            .find(|l| l.path == t.path)
            .map(|l| l.opened)
            .or_else(|| carried.get(&t.path).copied())
            .unwrap_or(u64::MAX)
    };
    let (i, _) = todo.iter().enumerate().min_by_key(|(_, t)| opened(t))?;
    todo.remove(i)
}

impl LayoutTree {
    /// The box of the node at `path`, as placed.
    /// Whether the node at `path`, placed in `frame`, is hidden other than
    /// by sleeping, dormant alone, and inert.
    pub(super) fn shown_at(&self, frame: &Frame, path: &NodePath) -> (bool, bool, bool) {
        let at = self
            .by_path
            .get(path)
            .and_then(|&u| self.extra(u))
            .and_then(|e| usize::try_from(e.index).ok())
            .filter(|&i| frame.nodes.get(i).is_some_and(|n| &n.path == path));
        let Some(i) = at else {
            return (false, false, false);
        };
        let inert = frame.behaviour(i).is_some_and(|b| b.inert);
        frame.nodes.get(i).map_or((false, false, inert), |n| {
            (n.look.invisible && !n.look.dormant, n.look.dormant, inert)
        })
    }

    pub(super) fn bounds_at(&self, frame: &Frame, path: &NodePath) -> Option<Bounds> {
        let i = self.by_path.get(path).and_then(|&u| self.extra(u)).map(|e| e.index)?;
        frame
            .nodes
            .get(usize::try_from(i).ok()?)
            .filter(|n| &n.path == path)
            .map(|n| n.bounds)
            .or_else(|| frame_bounds_of(frame, path))
    }

    /// Lay a layer's root out against its anchor; where it goes and how it opened.
    pub(super) fn place_layer(
        &mut self,
        node: &mut Node,
        placement: Positioning,
        anchor: Option<Bounds>,
        part_at: Option<Bounds>,
        viewport: Bounds,
        text: &mut TextSystem,
    ) -> ((f32, f32), Option<crate::placement::Placement>) {
        let (width, height) = (viewport.width, viewport.height);
        let mut base = self
            .extra(node.user)
            .and_then(|e| e.base.as_deref().cloned())
            .unwrap_or_default();
        if placement == Positioning::Fill {
            base.size = taffy::Size {
                width: Len::Px(width),
                height: Len::Px(height),
            };
            base.position = taffy::Position::Relative;
        } else {
            base.max_size = taffy::Size {
                width: Len::Px(width - 2.0 * EDGE_MARGIN),
                height: Len::Px(height - 2.0 * EDGE_MARGIN),
            };
        }
        let mut lay = |tree: &mut Self, node: &mut Node, s: BoxStyle| {
            set_style(node, s);
            tree.stats.add(node.compute_root(
                taffy::Size {
                    width: AvailableSpace::MaxContent,
                    height: AvailableSpace::MaxContent,
                },
                text,
                tree.schedule,
            ));
            let size = node.layout().size;
            (size.width, size.height)
        };
        let mut size = lay(self, node, base.clone());
        // At least as wide as the anchor, and still as wide as its own min
        // width asks: the width laid out so far already holds that.
        if let (Positioning::Anchored { match_width: true, .. }, Some(a)) = (placement, anchor)
            && a.width > size.0
        {
            let mut s = base.clone();
            s.min_size.width = Len::Px(a.width);
            size = lay(self, node, s);
        }
        let origin = match (placement, anchor) {
            (
                Positioning::Over {
                    part,
                    direction,
                    min_height,
                    ..
                },
                Some(a),
            ) => {
                let anchor_part = part_at.unwrap_or(a);
                let part = self.bounds_by_key(node, part).unwrap_or_default();
                let o = over(a, anchor_part, part, size, viewport, direction, min_height);
                let mut s = base;
                s.min_size.width = Len::Px(o.min_width.max(size.0));
                if let Some(h) = o.height {
                    s.max_size.height = Len::Px(h);
                }
                lay(self, node, s);
                // Scrolled once, as it opens: the reader scrolls it after that
                // (by the window's layout: a detached one writes nothing).
                if !self.detached
                    && let Some(handle) = self.first_scroll(node)
                    && handle.layout().is_none()
                {
                    handle.scroll_to((0.0, o.scroll));
                }
                o.origin
            }
            _ => {
                let size = match fitted_height(placement, anchor, size, viewport) {
                    Some(h) => {
                        let mut s = base;
                        s.max_size.height = Len::Px(h);
                        lay(self, node, s)
                    }
                    None => size,
                };
                position(placement, anchor, size, viewport)
            }
        };
        let size = (node.layout().size.width, node.layout().size.height);
        (origin, placed(placement, anchor, size, viewport))
    }

    /// The first scroll handle in `node`'s subtree, in tree order.
    fn first_scroll(&self, node: &Node) -> Option<ScrollHandle> {
        if let Some(h) = self.extra(node.user).and_then(|e| e.behaviour.scroll) {
            return Some(h);
        }
        node.children.iter().find_map(|c| self.first_scroll(c))
    }

    /// The box of the node keyed `key` in `root`'s subtree, from the root's
    /// top-left corner, as laid out (not yet placed).
    fn bounds_by_key(&self, root: &Node, key: ElementKey) -> Option<Bounds> {
        fn find(tree: &LayoutTree, n: &Node, key: ElementKey, at: (f32, f32)) -> Option<Bounds> {
            for c in &n.children {
                let l = c.layout();
                let here = (at.0 + l.location.x, at.1 + l.location.y);
                if tree
                    .extra(c.user)
                    .and_then(|e| e.path.0.last())
                    .and_then(|s| s.element_key())
                    == Some(key)
                {
                    return Some(Bounds {
                        x: here.0,
                        y: here.1,
                        width: l.size.width,
                        height: l.size.height,
                    });
                }
                if let Some(b) = find(tree, c, key, here) {
                    return Some(b);
                }
            }
            None
        }
        find(self, root, key, (0.0, 0.0))
    }
}

/// The box of the node at `path`, once placed.
fn frame_bounds_of(frame: &Frame, path: &NodePath) -> Option<Bounds> {
    frame.nodes.iter().find(|n| &n.path == path).map(|n| n.bounds)
}

/// The box of the node keyed `key` inside the node at `anchor`, once placed.
pub(super) fn frame_bounds_by_key(frame: &Frame, anchor: &NodePath, key: ElementKey) -> Option<Bounds> {
    frame
        .nodes
        .iter()
        .find(|n| n.path.0.starts_with(&anchor.0) && n.path.0.last().and_then(|s| s.element_key()) == Some(key))
        .map(|n| n.bounds)
}
