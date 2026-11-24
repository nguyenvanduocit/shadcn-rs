use taffy::AvailableSpace;

use crate::color::Color;
use crate::element::{Element, Positioning};
use crate::geometry::{Bounds, NodePath};
use crate::motion::Motion;
use crate::style::{Display, ResolvedText};
use crate::text::TextSystem;

use super::layers::{frame_bounds_by_key, take_oldest};
use super::{
    Before, Frame, Inherit, Layer, LayerMemo, LayerRoot, LayerTodo, LayoutTree, Len, NO_SLOT, Pass, Stats, UiState,
    Visits, engine, leaf, need, set_style,
};

/// How a pass writes a layer's part of the frame: every node pushed anew
/// (nodes came, went or moved), or in place, only nodes that changed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Write {
    All,
    Changed,
}

impl LayoutTree {
    /// Lay `root` out in a `width`×`height` window: patch the tree from it
    /// along the paths that lead to what changed, lay out what changed, and
    /// write the frame's nodes that changed (all of them when nodes came,
    /// went or moved).
    ///
    /// A pass that panics (a binding's closure, a measure) leaves the tree
    /// empty, as new: what it held is dropped with every tracker in it, so
    /// none stays subscribed, and the next pass builds the tree anew. Half
    /// patched, the tree would hold nodes its root no longer reaches.
    pub fn layout(
        &mut self,
        root: &Element,
        width: f32,
        height: f32,
        state: &UiState,
        motion: &mut Motion,
        text: &mut TextSystem,
    ) -> &Frame {
        let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.pass(root, width, height, state, motion, text);
        }));
        if let Err(panic) = pass {
            self.forget(motion);
            std::panic::resume_unwind(panic);
        }
        &self.frame
    }

    /// Drop every node it holds, and what it knows of them, as a new tree
    /// holds none. Every node left the tree, as `motion` is told (an exit
    /// starts, a stage is reported); one the next pass builds again enters
    /// anew. Its layers keep the order they last opened in, carried as a
    /// cold build carries the window's: the next pass opens them so.
    fn forget(&mut self, motion: &mut Motion) {
        let gone: Vec<NodePath> = motion.paths().cloned().chain(self.released.drain(..)).collect();
        motion.left(&gone, &|_| false);
        self.carried.clear();
        self.carried.extend(self.layer_paths.drain(..));
        self.root = None;
        self.layers.clear();
        self.extras.clear();
        self.free.clear();
        self.by_path.clear();
        self.by_focus.clear();
        self.by_field.clear();
        self.mounting.clear();
        self.joining.clear();
        if let Ok(mut m) = self.marks.try_borrow_mut() {
            m.clear();
        }
        if let Ok(mut m) = self.measured_due.try_borrow_mut() {
            m.clear();
        }
        if let Ok(mut f) = self.fields.try_borrow_mut() {
            f.clear();
        }
        self.focused = None;
        self.focused_layer = 0;
        self.motion_seen = 0;
        self.frame = Frame::default();
        self.seen = None;
        self.structure = true;
        self.size = (-1.0, -1.0);
        leaf::end_pass();
    }

    fn pass(
        &mut self,
        root: &Element,
        width: f32,
        height: f32,
        state: &UiState,
        motion: &mut Motion,
        text: &mut TextSystem,
    ) {
        let viewport = Bounds {
            x: 0.0,
            y: 0.0,
            width,
            height,
        };
        self.lists.begin_pass();
        self.stats = Stats::default();
        self.visits = Visits::default();
        self.take_marks(state);
        if motion.generation() != self.motion_seen {
            // What runs changed for every node at once: each is sampled again.
            self.motion_seen = motion.generation();
            let paths: Vec<NodePath> = motion.paths().cloned().collect();
            for p in &paths {
                self.mark_path(p, need::RESOLVE);
            }
        }
        let found = state
            .focused
            .and_then(|f| self.by_focus.get(&f))
            .and_then(|&u| self.extra(u));
        let (layer, path) = found.map_or((0, None), |e| (e.layer, Some(e.path.clone())));
        self.focused_layer = layer;
        self.focused = path;
        let mut frame = std::mem::take(&mut self.frame);
        frame.viewport = viewport;
        // A pass that rebuilds starts from the frame as it was, for what the
        // frame report says changed.
        let mut rebuilt_from: Option<usize> = None;
        let mut todo: std::collections::VecDeque<LayerTodo> = std::collections::VecDeque::new();
        let inherit = Inherit {
            text: ResolvedText::root(Color::BLACK),
            invisible: false,
            dormant: false,
            inert: false,
            passes: false,
            disabled: false,
            block: true,
        };
        let path = self.root_path.clone();
        let resized = self.size != (width, height);
        self.size = (width, height);
        let old = self.root.take();
        let structure_before = std::mem::replace(&mut self.structure, false);
        self.root = self.renew(
            &mut Pass {
                state,
                motion: &mut *motion,
                todo: &mut todo,
            },
            old,
            root,
            &path,
            &inherit,
            0,
            NO_SLOT,
        );
        let mut write = if std::mem::take(&mut self.structure) || structure_before || frame.nodes.is_empty() {
            Write::All
        } else {
            Write::Changed
        };
        // The root fills the window.
        let mut root_inherit = inherit;
        if let Some(mut node) = self.root.take() {
            let base = self.extra(node.user).and_then(|e| e.base.as_deref());
            let window = taffy::Size {
                width: Len::Px(width),
                height: Len::Px(height),
            };
            // Resolved again, it holds its own size, not the window's.
            let stale = resized
                || node.style.size != window
                || base.is_some_and(|b| {
                    let mut s = node.style.clone();
                    s.size = b.size;
                    *b != s
                });
            if stale || write == Write::All {
                let mut s = base.cloned().unwrap_or_default();
                s.size = window;
                set_style(&mut node, s);
            }
            self.stats.add(node.compute_root(
                taffy::Size {
                    width: AvailableSpace::Definite(width),
                    height: AvailableSpace::Definite(height),
                },
                text,
                self.schedule,
            ));
            if let Some(e) = self.extra(node.user) {
                root_inherit = Inherit {
                    text: e.text.clone(),
                    invisible: e.invisible,
                    dormant: e.dormant,
                    inert: false,
                    passes: false,
                    disabled: false,
                    block: e.style.display == Display::Block,
                };
            }
            if write == Write::All {
                rebuilt_from = Some(0);
                Self::rebuild_from(&mut frame, 0);
            }
            self.place(
                &mut frame,
                &mut node,
                (0.0, 0.0),
                None,
                false,
                state,
                motion,
                text,
                write,
            );
            self.join_lists(&frame);
            self.root = Some(node);
        } else if !frame.nodes.is_empty() {
            rebuilt_from = Some(0);
            Self::rebuild_from(&mut frame, 0);
            write = Write::All;
        }

        let mut old_layers: Vec<LayerRoot> = std::mem::take(&mut self.layers);
        let mut layer = 0_usize;
        // Layers paint in the order they opened, as portals appended to
        // `<body>` do (ADR-0024): of those waiting, the one open longest
        // goes next, then the new ones in the order they were found. A
        // nested layer opens after the layer it is in, so it comes later.
        while let Some(t) = take_oldest(&mut todo, &old_layers, &self.carried) {
            layer = layer.saturating_add(1);
            let anchor = t
                .deferred
                .anchor_part
                .and_then(|k| frame_bounds_by_key(&frame, &t.anchor, k))
                .or_else(|| self.bounds_at(&frame, &t.anchor));
            // A layer inherits its look from the window's root, not its
            // anchor, as content portalled to `<body>` does: a menu opened
            // from a bold button is not bold. Whether it shows is its
            // owner's, all of it (hidden, dormant, inert): a dialog opened
            // from a slide that goes to sleep sleeps with it.
            let (hidden, dormant, inert) = self.shown_at(&frame, &t.anchor);
            let inherit = Inherit {
                inert: t.inert || inert,
                disabled: t.disabled,
                invisible: hidden,
                dormant,
                ..root_inherit.clone()
            };
            let found = old_layers.iter().position(|l| l.path == t.path);
            let (old, memo, opened_at) = match found {
                Some(i) => {
                    let l = old_layers.swap_remove(i);
                    (Some(l.node), l.memo, l.opened)
                }
                None => match self.carried.get(&t.path) {
                    Some(&at) => (None, None, at),
                    None => {
                        self.opened = self.opened.wrapping_add(1);
                        (None, None, self.opened)
                    }
                },
            };
            let mut pass = Pass {
                state,
                motion: &mut *motion,
                todo: &mut todo,
            };
            let had = old.is_some();
            let Some((mut node, change)) =
                self.renew_layer(&mut pass, old, &t.deferred.child, &t.path, &inherit, layer, NO_SLOT)
            else {
                continue;
            };
            let placement = t.deferred.positioning;
            let part_at = match placement {
                Positioning::Over { anchor_part, .. } => frame_bounds_by_key(&frame, &t.anchor, anchor_part),
                _ => None,
            };
            let reuse = memo.filter(|m| {
                had && change == engine::Change::None
                    && m.positioning == placement
                    && m.anchor == anchor
                    && m.part == part_at
                    && m.viewport == (width, height)
            });
            let (origin, opened) = match reuse {
                Some(m) => (m.origin, m.placed),
                None => self.place_layer(&mut node, placement, anchor, part_at, viewport, text),
            };
            // A layer that is new, or at another place in the order, or
            // whose nodes came or went, is written anew from its start on.
            let moved_in_order = self.layer_paths.get(layer.saturating_sub(1)).map(|(p, _)| p) != Some(&t.path);
            if write == Write::Changed && (std::mem::take(&mut self.structure) || moved_in_order || !had) {
                write = Write::All;
                let start = frame
                    .layers
                    .get(layer.saturating_sub(1))
                    .map_or(frame.nodes.len(), |l| l.root);
                rebuilt_from = Some(rebuilt_from.map_or(start, |r| r.min(start)));
                Self::rebuild_from(&mut frame, start);
            }
            self.structure = false;
            let at = Layer {
                root: frame.nodes.len(),
                anchor,
                tab_after: t.deferred.tab_after,
            };
            let moved = memo.is_none_or(|m| m.origin != origin || m.placed != opened);
            match write {
                Write::All => {
                    frame.layers.truncate(layer.saturating_sub(1));
                    frame.layers.push(at);
                }
                Write::Changed => {
                    if let Some(l) = frame.layers.get_mut(layer.saturating_sub(1)) {
                        l.anchor = anchor;
                        l.tab_after = t.deferred.tab_after;
                    }
                }
            }
            self.place(&mut frame, &mut node, origin, opened, moved, state, motion, text, write);
            self.join_lists(&frame);
            // What a look in the layer reads: how it was placed.
            if let Some(report) = t.deferred.placed.filter(|_| !self.detached) {
                crate::reactive::untracked(|| report.set(opened));
            }
            self.layers.push(LayerRoot {
                path: t.path,
                opened: opened_at,
                node,
                memo: Some(LayerMemo {
                    positioning: placement,
                    anchor,
                    part: part_at,
                    viewport: (width, height),
                    origin,
                    placed: opened,
                }),
            });
        }
        for l in old_layers {
            self.release(l.node);
        }
        // Every tree is reconciled and every leaving node gone: the fields
        // that came claim their handles.
        self.claim_fields();
        // Layers that closed: the frame ends where the last open one does.
        if frame.layers.len() > self.layers.len() || std::mem::take(&mut self.structure) {
            let end = frame
                .layers
                .get(self.layers.len())
                .map_or(frame.nodes.len(), |l| l.root);
            if write == Write::Changed || end < frame.nodes.len() {
                rebuilt_from = Some(rebuilt_from.map_or(end, |r| r.min(end)));
                Self::rebuild_from(&mut frame, end);
                frame.layers.truncate(self.layers.len());
            }
        }
        if let Some(start) = rebuilt_from {
            self.visits.rebuilt = self.visits.rebuilt.saturating_add(1);
            frame.changes.rebuilt(start);
            frame.handles = (0..frame.behaviour.len())
                .filter(|&i| frame.behaviour.get(i).is_some_and(|b| b.scroll.is_some()))
                .collect();
        }
        self.layer_paths.clear();
        self.layer_paths
            .extend(self.layers.iter().map(|l| (l.path.clone(), l.opened)));
        // What a cold build or a recovered pass carried is the layers' own now.
        self.carried.clear();
        frame.changes.trim(frame.nodes.len());
        // What is shown now, before anything after layout (a list's
        // watcher, a hook) asks for focus.
        if !self.detached {
            crate::focus::publish(&frame);
            // Every member has joined: lists watched see them whole.
            crate::reactive::untracked(|| self.lists.end_pass());
        }
        leaf::end_pass();
        motion.left(&self.released, &|p| self.by_path.contains_key(p));
        self.released.clear();
        self.frame = frame;
    }

    /// Drop the frame's nodes from `start` on, keeping them as they were
    /// when the frame was last painted, for the frame report's diff: as they
    /// are now, with what earlier passes wrote into them in place undone
    /// (each node's first box before, and whether one repainted it).
    pub(super) fn rebuild_from(frame: &mut Frame, start: usize) {
        let start = start.min(frame.nodes.len());
        let mut was = frame.nodes.split_off(start);
        frame.behaviour.truncate(start);
        frame.layers.retain(|l| l.root < start);
        // Of what is dropped, the nodes not kept yet: from `at` on they are.
        let fresh = match &frame.before {
            Some(b) if b.at <= start => return,
            Some(b) => b.at.saturating_sub(start).min(was.len()),
            None => was.len(),
        };
        was.truncate(fresh);
        let mut repainted = Vec::new();
        // Only boxes are undone: a repaint mark stands for look and content.
        frame.changes.unread_back(start..start.saturating_add(fresh), |k, c| {
            if let Some(n) = was.get_mut(k) {
                n.bounds = c.before;
            }
            if c.repainted {
                repainted.push(k);
            }
        });
        repainted.sort_unstable();
        repainted.dedup();
        match &mut frame.before {
            Some(b) => {
                was.append(&mut b.nodes);
                repainted.extend(b.repainted.iter().map(|k| k.saturating_add(fresh)));
                *b = Before {
                    at: start,
                    nodes: was,
                    repainted,
                };
            }
            None => {
                frame.before = Some(Before {
                    at: start,
                    nodes: was,
                    repainted,
                });
            }
        }
    }

    /// Add the member nodes patched or walked since the last call to their
    /// lists, in tree order, once placed in `frame`: those navigable
    /// ([`Frame::navigable`]: shown, or dormant alone) only, so a list's
    /// handlers (arrow keys, typeahead) never reach a member hidden for
    /// any other reason.
    fn join_lists(&mut self, frame: &Frame) {
        let joining = std::mem::take(&mut self.joining);
        // A detached tree leaves the window's lists as its own tree filled them.
        for &u in joining.iter().filter(|_| !self.detached) {
            let Some(e) = self.extra(u) else { continue };
            if !usize::try_from(e.index).is_ok_and(|i| frame.navigable(i)) {
                continue;
            }
            for join in e.element.common().map_or(&[][..], |c| &c.joins[..]) {
                join(&self.lists);
            }
        }
        self.joining = joining;
        self.joining.clear();
        let due = std::mem::take(&mut *self.measured_due.borrow_mut());
        for (f, bounds) in &due {
            crate::reactive::untracked(|| f(bounds));
        }
        let mut due = due;
        due.clear();
        *self.measured_due.borrow_mut() = due;
    }
}
