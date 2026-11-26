use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use crate::element::{Common, Element};
use crate::focus::FocusHandle;
use crate::geometry::{NodePath, PathSeg};
use crate::style::{ContentStyle, Cursor, Display, Length, ResolvedText, Style};
use crate::text::Run;

use super::attrs::AttrBinding;
use super::binding::{StyleBinding, Tracked, Tracker};
use super::{
    Behaviour, BoxStyle, Extra, ImageMeasure, Inherit, LayerTodo, LayoutTree, Leaf, Len, NO_SLOT, Node, PaintStyle,
    Pass, Pending, PendingInput, SpanTarget, UiState, engine, leaf, need, trim_scratch, writable,
};

fn kind_of(el: &Element) -> u8 {
    match el {
        Element::Div(_) => 1,
        Element::Text(_) => 2,
        Element::Icon(_) => 3,
        Element::Input(_) => 4,
        Element::Figures(_) => 5,
        Element::Image(_) => 6,
        Element::Deferred(_) => 7,
        Element::Component(_) => 8,
        Element::Drawn(_) => 9,
        Element::Empty => 0,
    }
}

/// The focus handle of a node: its own, or one from its path when it is
/// focusable by place.
fn focus_of(common: &Common, path: &NodePath) -> Option<FocusHandle> {
    common
        .focus
        .or_else(|| common.focusable.then(|| FocusHandle::for_path(path)))
}

/// What resolving a node's own style and content found.
struct Resolved {
    style_changed: bool,
    leaf_changed: bool,
    mine: Inherit,
}

impl LayoutTree {
    /// Patch `old` into what `el` (at `path`) now is, or build it anew; `None`
    /// when `el` takes no space.
    #[expect(clippy::too_many_arguments, reason = "a node, where it is and what it inherits")]
    pub(super) fn renew(
        &mut self,
        pass: &mut Pass<'_>,
        old: Option<Node>,
        el: &Element,
        path: &NodePath,
        inherit: &Inherit,
        layer: usize,
        parent: u32,
    ) -> Option<Node> {
        self.renew_layer(pass, old, el, path, inherit, layer, parent)
            .map(|(n, _)| n)
    }

    #[expect(clippy::too_many_arguments, reason = "a node, where it is and what it inherits")]
    pub(super) fn renew_layer(
        &mut self,
        pass: &mut Pass<'_>,
        old: Option<Node>,
        el: &Element,
        path: &NodePath,
        inherit: &Inherit,
        layer: usize,
        parent: u32,
    ) -> Option<(Node, engine::Change)> {
        let same = old
            .as_ref()
            .is_some_and(|n| self.extra(n.user).is_some_and(|e| e.kind == kind_of(el)));
        if same && let Some(mut n) = old {
            return match self.patch(pass, &mut n, el, inherit, layer, false) {
                Some(change) => Some((n, change)),
                None => {
                    self.release(n);
                    None
                }
            };
        }
        if let Some(n) = old {
            self.release(n);
        }
        let mut n = self.build(pass, el, path.clone(), inherit, layer, parent)?;
        if let Some(e) = self.extra_mut(n.user) {
            e.base = Some(Box::new(n.style.clone()));
        }
        n.invalidate();
        Some((n, engine::Change::Outer))
    }

    /// A new node for `el` at `path`; `None` when it takes no space.
    fn build(
        &mut self,
        pass: &mut Pass<'_>,
        el: &Element,
        path: NodePath,
        inherit: &Inherit,
        layer: usize,
        parent: u32,
    ) -> Option<Node> {
        el.common()?;
        self.structure = true;
        let u = self.alloc(Box::new(Extra {
            path: path.clone(),
            kind: kind_of(el),
            name: None,
            style: PaintStyle::default(),
            text: inherit.text.clone(),
            provides: ContentStyle::default(),
            span_content: Vec::new(),
            runs: Vec::new(),
            spare_runs: Vec::new(),
            last_spans: 0,
            spare_boxes: Vec::new(),
            takes_decorations: true,
            span_bindings: Vec::new(),
            span_styles: Vec::new(),
            opacity: 1.0,
            invisible: false,
            dormant: false,
            translating: false,
            curves: None,
            content: Pending::None,
            behaviour: Rc::default(),
            layer,
            style_binding: None,
            text_binding: None,
            draw_binding: None,
            attr_binding: None,
            base: None,
            element: Element::Empty,
            inherit: None,
            mine: inherit.clone(),
            parent,
            own: need::RESOLVE,
            below: 0,
            index: u32::MAX,
            ran: u64::MAX,
            kids: None,
            resolve: Tracker::default(),
            place: Tracker::default(),
            focus_id: None,
            claim: None,
            motion: false,
            measures: false,
            fresh_behaviour: true,
            hidden: None,
            hidden_waits: Vec::new(),
            hidden_mark: None,
            interaction: crate::engine::Ends::default(),
        }));
        self.by_path.insert(path, u);
        let mut node = Node::new(BoxStyle::default(), None);
        node.user = u;
        if self.patch(pass, &mut node, el, inherit, layer, true).is_none() {
            self.hide(parent, node);
            return None;
        }
        Some(node)
    }

    /// Bring `node` up to `el`; how far its change reaches, or `None` when
    /// `el` now takes no space. A node whose element is the one it was
    /// patched from, with nothing marked on it, is not resolved again; its
    /// subtree is walked only where something below is marked, and skipped
    /// when nothing is.
    fn patch(
        &mut self,
        pass: &mut Pass<'_>,
        node: &mut Node,
        el: &Element,
        inherit: &Inherit,
        layer: usize,
        force: bool,
    ) -> Option<engine::Change> {
        let common = el.common()?;
        let u = node.user;
        let (same, own, below) = self.extra(u).map_or((false, need::RESOLVE, 0), |ex| {
            (
                el.ptr_eq(&ex.element) && ex.inherit.as_ref() == Some(inherit) && ex.layer == layer,
                ex.own,
                ex.below,
            )
        });
        if !force && same && own & need::RESOLVE == 0 {
            if (own | below) & (need::RESOLVE | need::VISIT) == 0 {
                return Some(engine::Change::None);
            }
            return self.walk(pass, node, el, common);
        }
        // A provider above changed only what paints (its colour on hover):
        // the node takes it from what it provides itself, unresolved, and
        // passes it on to its children the same way. Only the text, icons
        // and inputs that show it are written again; nothing lays out.
        if !force
            && own & need::RESOLVE == 0
            && let Some(ex) = self.extra_mut(u)
            && el.ptr_eq(&ex.element)
            && ex.layer == layer
            && ex.inherit.as_ref().is_some_and(|i| i.paints_as(inherit))
        {
            ex.repaint_content(inherit);
            self.visits.repainted = self.visits.repainted.saturating_add(1);
            return self.walk(pass, node, el, common);
        }
        self.visits.resolved = self.visits.resolved.saturating_add(1);
        let mut ex = self.take_extra(u)?;
        let mut tracker = std::mem::take(&mut ex.resolve);
        let (probe, marks) = (*self.probe, Rc::clone(&self.marks));
        let resolved = tracker.run(probe, &marks, u, need::RESOLVE, || {
            self.resolve_own(pass, node, el, common, inherit, layer, &mut ex)
        });
        ex.resolve = tracker;
        let Some(r) = resolved else {
            self.put_extra(u, ex);
            return None;
        };
        // What it passes on changed: a change of what paints alone reaches
        // its children unresolved (above), any other resolves them.
        let mine_changed = ex.inherit.is_none() || !ex.mine.paints_as(&r.mine);
        ex.mine = r.mine;
        ex.element = el.clone();
        ex.inherit = Some(inherit.clone());
        let visit = !common.joins.is_empty()
            || el
                .kids()
                .is_some_and(|k| k.iter().any(|c| matches!(c, Element::Deferred(_))));
        ex.own = (ex.own & !(need::RESOLVE | need::VISIT)) | need::PLACE;
        if visit {
            ex.own |= need::VISIT;
        }
        if ex.motion && pass.motion.busy(&ex.path) {
            // What moves is sampled again next pass.
            ex.own |= need::RESOLVE;
        }
        if let Some(f) = ex.focus_id {
            self.by_focus.insert(f, u);
        }
        if let Some(h) = ex.hidden.as_deref() {
            // Its hidden children are built again below, and say again what they read.
            h.track(|| ());
        }
        let stale = ex.hidden_waits.len();
        let path = ex.path.clone();
        let mine = ex.mine.clone();
        self.put_extra(u, ex);

        let (structure, children) = match el.kids() {
            Some(kids) => self.patch_children(pass, node, kids, &path, &mine, layer, force || mine_changed),
            None => {
                let gone = !node.children.is_empty();
                for c in std::mem::take(&mut node.children) {
                    self.release(c);
                }
                (gone, engine::Change::None)
            }
        };
        // Its hidden children registered again as they were built: the
        // registrations of the last build go.
        if stale > 0
            && let Some(ex) = self.extra_mut(u)
        {
            ex.hidden_waits.drain(..stale.min(ex.hidden_waits.len()));
        }
        let objects = self.lay_objects(node, el);
        self.sum_below(node);
        Some(node.settle(r.style_changed, r.leaf_changed || structure || objects, children))
    }

    /// Set which of a paragraph's atomic inlines took space (are its
    /// node's children) in its leaf; whether that changed.
    fn lay_objects(&self, node: &mut Node, el: &Element) -> bool {
        let Element::Text(t) = el else { return false };
        let Some(Leaf::Text { objects, .. }) = &mut node.leaf else {
            return false;
        };
        // Compared where they lie; written into the vector it had.
        if objects.len() == t.kind.objects.len() && self.laid(&node.children, t).eq(objects.iter().copied()) {
            return false;
        }
        objects.clear();
        objects.extend(self.laid(&node.children, t));
        true
    }

    /// Each of paragraph `t`'s atomic inlines, and whether it took space.
    /// `children` are those that did, in order: an inline took space when
    /// it is the next child.
    fn laid<'a>(
        &'a self,
        children: &'a [Node],
        t: &'a crate::element::Text,
    ) -> impl Iterator<Item = leaf::Object> + 'a {
        let mut next = children
            .iter()
            .map(|c| self.extra(c.user).and_then(|e| e.path.0.last().copied()))
            .peekable();
        t.kind
            .children
            .iter()
            .zip(&t.kind.objects)
            .enumerate()
            .map(move |(i, (kid, &at))| {
                let seg = kid.seg(i);
                let laid = next.next_if(|s| *s == Some(seg)).is_some();
                leaf::Object { at, laid }
            })
    }

    /// Let go of `node`, a child of `parent` that now takes no space, keeping
    /// what decided that (its patch's reads and every binding's, of every
    /// kind: [`Extra::trackers`]) as `parent`'s: a change there resolves
    /// `parent` again, which builds it.
    fn hide(&mut self, parent: u32, node: Node) {
        let reads: Vec<crate::reactive::RenderNode> =
            self.extra(node.user).map_or_else(Vec::new, |e| e.trackers().collect());
        if !reads.is_empty() && parent != NO_SLOT {
            let marks = Rc::clone(&self.marks);
            if let Some(p) = self.extra_mut(parent) {
                let hidden = **p.hidden.get_or_insert_with(|| {
                    super::binding::Node::new(crate::reactive::detached(|| {
                        crate::reactive::RenderNode::new(move || {
                            marks.try_with_mut(|m| m.push((parent, need::RESOLVE)));
                            crate::component::frame_due();
                        })
                    }))
                });
                for r in reads {
                    hidden.take_deps(r);
                }
            }
        }
        // Its sources' registrations wake the parent in its place.
        // Into the parent's list, by its mark: both kept from the last time.
        if parent != NO_SLOT
            && let Some(mut waits) = self.extra_mut(parent).map(|p| std::mem::take(&mut p.hidden_waits))
        {
            let from = waits.len();
            if let Some(e) = self.extra_mut(node.user) {
                e.take_waits(&mut waits);
            }
            // Not what reads a handle bound inside it: a hidden node is never
            // hovered nor focused, and built again it registers anew. Handed
            // on, its unbinding would wake the parent, which builds it hidden
            // again, which unbinds it.
            let mut k = from;
            while let Some(w) = waits.get(k) {
                if w.handle().is_some_and(|h| self.bound_inside(&node, h)) {
                    drop(waits.swap_remove(k));
                } else {
                    k = k.saturating_add(1);
                }
            }
            let marks = Rc::clone(&self.marks);
            if let Some(p) = self.extra_mut(parent) {
                if let Some(moved) = waits.get(from..)
                    && !moved.is_empty()
                {
                    let mark = p
                        .hidden_mark
                        .get_or_insert_with(|| super::binding::resolve_mark(&marks, parent));
                    for w in moved {
                        w.repoint(mark);
                    }
                }
                p.hidden_waits = waits;
            }
        }
        self.release(node);
    }

    /// Whether the handle numbered `handle` is bound to `node` or a node
    /// below it.
    fn bound_inside(&self, node: &Node, handle: u64) -> bool {
        self.extra(node.user)
            .and_then(|e| e.interaction.handle())
            .is_some_and(|h| h.id() == handle)
            || node.children.iter().any(|c| self.bound_inside(c, handle))
    }

    /// Recount what `node`'s subtree needs from its children.
    fn sum_below(&mut self, node: &Node) {
        let below = node
            .children
            .iter()
            .fold(0, |b, c| b | self.extra(c.user).map_or(0, |e| e.own | e.below));
        if let Some(e) = self.extra_mut(node.user) {
            e.below = below;
        }
    }

    /// Walk a node that needs nothing itself to the children that need
    /// something, running what it does each pass (joining its lists,
    /// handing on its deferred subtrees).
    fn walk(&mut self, pass: &mut Pass<'_>, node: &mut Node, el: &Element, common: &Common) -> Option<engine::Change> {
        self.visits.walked = self.visits.walked.saturating_add(1);
        if !common.joins.is_empty() {
            self.joining.push(node.user);
        }
        let Some(all) = el.kids() else {
            return Some(engine::Change::None);
        };
        let Some((path, mine, layer)) = self.extra(node.user).map(|e| (e.path.clone(), e.mine.clone(), e.layer)) else {
            return Some(engine::Change::None);
        };
        let mut at = 0_usize;
        let mut reach = engine::Change::None;
        let mut gone: Vec<usize> = Vec::new();
        for (i, kid) in all.iter().enumerate() {
            if let Element::Deferred(d) = kid {
                let seg = kid.seg(i);
                pass.todo.push_back(LayerTodo {
                    deferred: Rc::clone(d),
                    path: path.child(seg),
                    anchor: path.clone(),
                    inert: mine.inert,
                    disabled: mine.disabled,
                });
                continue;
            }
            if kid.common().is_none() {
                continue;
            }
            // The children are the kids that took space, in order: a kid
            // that is not the next child's element took none.
            let Some(c) = node.children.get_mut(at) else { break };
            let ours = self.extra(c.user).is_some_and(|e| e.element.ptr_eq(kid));
            if !ours {
                continue;
            }
            match self.patch(pass, c, kid, &mine, layer, false) {
                Some(change) => reach = reach.max(change),
                None => gone.push(at),
            }
            at = at.saturating_add(1);
        }
        let structure = !gone.is_empty();
        for &g in gone.iter().rev() {
            if g < node.children.len() {
                let n = node.children.remove(g);
                self.hide(node.user, n);
            }
        }
        let structure = self.lay_objects(node, el) || structure;
        self.sum_below(node);
        if !structure && reach == engine::Change::None {
            return Some(engine::Change::None);
        }
        Some(node.settle(false, structure, reach))
    }

    /// Resolve `el`'s own style and content into `node` and `ex`: what
    /// changed, or `None` when it now takes no space.
    #[expect(clippy::too_many_arguments, reason = "a node, its element and what it inherits")]
    fn resolve_own(
        &mut self,
        pass: &mut Pass<'_>,
        node: &mut Node,
        el: &Element,
        common: &Common,
        inherit: &Inherit,
        layer: usize,
        ex: &mut Extra,
    ) -> Option<Resolved> {
        let u = node.user;
        // The one order a node resolves in, so each binding reads the
        // node's flags bound and as they are (never an unbound handle,
        // whose late bind would be a write from the frame):
        //   1. bind its handle and write the flags that come from no
        //      attribute, `disabled` as it last resolved;
        //   2. run its attribute phase;
        //   3. write the flags that come from attributes (`disabled`), if
        //      the phase changed them;
        //   4. run its style phase ([`Self::resolve_style`]).
        let (fixed, bound) = common.attrs_to_resolve();
        let was_disabled = inherit.disabled || ex.attr_binding.as_deref().map_or(fixed, AttrBinding::attrs).disabled;
        self.bind_flags(pass.state, el, common, was_disabled, ex, layer);
        // Its attributes (ADR-0026's accessibility binding phase): the
        // fixed ones, each bound one written over them as its closure last
        // gave, run again here only when what it read changed.
        let attrs_changed = if bound.is_empty() {
            ex.attr_binding = None;
            false
        } else {
            let same = el.ptr_eq(&ex.element);
            // A cold build writes what the live node holds over a run
            // that panicked.
            let held = self.held.get(&ex.path).map_or(&[][..], |h| h.attrs.as_slice());
            ex.attr_binding
                .get_or_insert_with(|| Box::new(AttrBinding::new(&self.marks, u)))
                .resolve(fixed, bound, same, held)
        };
        let attrs = ex.attr_binding.as_deref().map_or(fixed, AttrBinding::attrs);
        let (own_disabled, own_inert, own_dormant, own_passes, tab_reversed) = (
            attrs.disabled,
            attrs.inert,
            attrs.dormant,
            attrs.pass_through_all,
            attrs.tab_reversed,
        );
        let disabled = inherit.disabled || own_disabled;
        if disabled != was_disabled {
            self.bind_flags(pass.state, el, common, disabled, ex, layer);
        }
        let mut style = self.resolve_style(common, ex, u);
        if style.display == Display::None {
            return None;
        }
        if !common.joins.is_empty() {
            self.joining.push(u);
        }
        let sampled = pass.motion.sample(&ex.path, &mut style);
        ex.translating = sampled.translating;
        ex.curves = sampled.curves;
        ex.motion |= sampled.kept;
        ex.measures = sampled.measures;
        // The content provided to it with what it provides itself, in its
        // direction (which alone flows to every node).
        let text = ResolvedText {
            direction: style.direction.unwrap_or(inherit.text.direction),
            ..style.content_in(&inherit.text)
        };
        ex.provides = style.content;
        ex.takes_decorations = style.takes_decorations();
        let invisible = inherit.invisible || style.invisible;
        let dormant = inherit.dormant || own_dormant;
        if matches!(el, Element::Input(_)) && style.cursor.is_none() {
            style.cursor = Some(Cursor::Text);
        }
        let mut b = style.box_style(text.direction);
        if matches!(el, Element::Icon(_)) {
            let size = Len::Px(text.icon_size);
            if style.width == Length::Auto {
                b.size.width = size;
            }
            if style.height == Length::Auto {
                b.size.height = size;
            }
        }
        if style.inline && inherit.block && style.justify_self.is_none() {
            b.justify_self = Some(taffy::AlignItems::START);
        }
        let style_changed = match &mut ex.base {
            Some(base) => {
                let changed = **base != b;
                if changed {
                    (**base).clone_from(&b);
                    node.style = b;
                }
                changed
            }
            None => {
                let changed = node.style != b;
                if changed {
                    node.style = b;
                }
                changed
            }
        };

        // What it holds. A span's bound properties replay here, under
        // the paragraph's own trackers: a signal they read resolves the
        // paragraph, not the instance that built it.
        // Into the buffer it kept from its last resolve: a span's colour
        // changing allocates nothing for it.
        let mut bound_spans = std::mem::take(&mut ex.span_styles);
        bound_spans.clear();
        match el {
            Element::Text(n) if n.kind.spans.iter().any(|sp| !sp.bindings.is_empty()) => {
                ex.span_bindings.resize_with(n.kind.spans.len(), || None);
                // A cold build reuses each span's held writes, the rest run.
                let held = self.held.get(&ex.path).map(|h| h.spans.as_slice()).unwrap_or_default();
                let styles = n
                    .kind
                    .spans
                    .iter()
                    .zip(ex.span_bindings.iter_mut())
                    .enumerate()
                    .map(|(k, (sp, b))| {
                        // A span that no longer binds drops its trackers:
                        // what its old closures read resolves nothing.
                        let Some(ops) = sp.bindings.ops() else {
                            *b = None;
                            return None;
                        };
                        let held = held
                            .iter()
                            .find(|(at, _)| *at == k)
                            .map_or(&[][..], |(_, h)| h.as_slice());
                        Some(
                            b.get_or_insert_with(|| StyleBinding::new(&self.marks, u))
                                .get(ops, &sp.style, held),
                        )
                    });
                bound_spans.extend(styles);
            }
            _ => ex.span_bindings.clear(),
        }
        ex.span_content.clear();
        // Into the buffer it kept from its last resolve: runs resolved
        // again equal are compared there, and nothing is allocated.
        let mut runs = std::mem::take(&mut ex.runs);
        runs.clear();
        if let Element::Text(n) = el {
            runs.extend(n.kind.spans.iter().enumerate().map(|(k, sp)| {
                let own = match bound_spans.get(k).and_then(Option::as_ref) {
                    Some(st) => st,
                    None => &sp.style,
                };
                ex.span_content.push(own.content);
                Run {
                    range: sp.range.clone(),
                    text: Run::style_in(&text, &own.content),
                    background: own.background,
                    radius: own.shape.radii().top_left.max(0.0),
                    padding: own.padding,
                }
            }));
        }
        // The spans that act on the pointer, each with its cursor (a bound
        // one as its binding gave it).
        fn acting<'a>(
            spans: &'a [crate::element::SpanSpec],
            bound: &'a [Option<Style>],
        ) -> impl Iterator<Item = (usize, Option<Cursor>)> + 'a {
            spans.iter().enumerate().filter_map(|(k, sp)| {
                let cursor = bound
                    .get(k)
                    .and_then(Option::as_ref)
                    .map_or(sp.style.cursor, |st| st.cursor);
                (sp.click.is_some() || cursor.is_some()).then_some((k, cursor))
            })
        }
        // Resolved again from the element it was built from (a binding
        // changed), it acts as it did unless its cursor or a bound span's
        // changed, or another span acts: the behaviour stands, and nothing
        // is allocated for it. Spans are told apart by which they are and
        // all they cover (two may start at the same byte). A bound
        // attribute that changed makes it anew.
        let same = el.ptr_eq(&ex.element)
            && !attrs_changed
            && ex.behaviour.cursor == style.cursor
            && match el {
                Element::Text(n) if !bound_spans.is_empty() => acting(&n.kind.spans, &bound_spans)
                    .map(|(k, cursor)| {
                        let sp = n.kind.spans.get(k);
                        (k, sp.map(|sp| &sp.range), cursor, sp.and_then(|sp| sp.key))
                    })
                    .eq(ex
                        .behaviour
                        .spans
                        .iter()
                        .map(|t| (t.index, Some(&t.range), t.cursor, t.key))),
                _ => true,
            };
        let spans: Vec<SpanTarget> = match el {
            Element::Text(_) if same => Vec::new(),
            Element::Text(n) => acting(&n.kind.spans, &bound_spans)
                .filter_map(|(k, cursor)| {
                    let sp = n.kind.spans.get(k)?;
                    Some(SpanTarget {
                        index: k,
                        key: sp.key,
                        range: sp.range.clone(),
                        click: sp.click.clone(),
                        cursor,
                        boxes: Vec::new(),
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        ex.span_styles = bound_spans;
        let (content, leaf) = match el {
            Element::Text(n) => {
                // Its content no longer bound, the binding goes: what it
                // read resolves nothing, and the text it gave is let go.
                // A binding whose first run panicked gives nothing yet:
                // its text is empty ([`Tracked`]).
                let held = self.held.get(&ex.path).and_then(|h| h.text.clone());
                let bound = match n.kind.bound.as_ref() {
                    // A cold build reuses the text the live node holds.
                    Some(_) if let Some(t) = held => Some(t.unwrap_or_else(|| Arc::from(""))),
                    Some(f) => Some(
                        ex.text_binding
                            .get_or_insert_with(|| Box::new(Tracked::new(&self.marks, u)))
                            .get(f, || Arc::from(f()))
                            .unwrap_or_else(|| Arc::from("")),
                    ),
                    None => {
                        ex.text_binding = None;
                        None
                    }
                };
                let source = bound.as_deref().unwrap_or(&n.kind.content);
                // Resolved again with what it showed (a binding of its
                // style changed): what it held stays, copied nowhere; runs
                // as many as it showed are written into those (or the
                // spare the frame node gave back), not allocated anew.
                let (content, runs_shown) = match &mut ex.content {
                    Pending::Text { content, runs: shown } => (
                        Some(&*content).filter(|c| ***c == *source).map(Arc::clone),
                        if **shown == *runs {
                            Some(Rc::clone(shown))
                        } else if shown.len() == runs.len() {
                            writable(shown, &mut ex.spare_runs).clone_from_slice(&runs);
                            Some(Rc::clone(shown))
                        } else {
                            None
                        },
                    ),
                    _ => (None, None),
                };
                let content = content.or(bound).unwrap_or_else(|| Arc::from(n.kind.content.as_str()));
                let runs_shown = runs_shown.unwrap_or_else(|| Rc::from(runs.as_slice()));
                // A spare of another length is of no use.
                ex.spare_runs.retain(|s| s.len() == runs.len());
                // What it is measured by, shared with its last leaf while
                // equal.
                let measured = match &node.leaf {
                    Some(Leaf::Text { runs: old, .. })
                        if old.len() == runs.len()
                            && old.iter().zip(&runs).all(|(o, r)| *o == Leaf::measuring_run(r)) =>
                    {
                        Arc::clone(old)
                    }
                    _ => runs.iter().map(Leaf::measuring_run).collect(),
                };
                // Which inlines took space is known once its children are
                // patched (`lay_objects`): until then the leaf keeps what it
                // had, moved over below if the rest changed, not copied.
                let objects = Vec::new();
                (
                    Pending::Text {
                        content: Arc::clone(&content),
                        runs: runs_shown,
                    },
                    Some(Leaf::Text {
                        content,
                        text: text.clone(),
                        runs: measured,
                        objects,
                    }),
                )
            }
            Element::Icon(n) => (
                Pending::Icon {
                    icon: n.kind.icon,
                    stroke_width: style.stroke_width,
                },
                None,
            ),
            Element::Input(n) => {
                // Its value read (not its scroll, which placing reads), and
                // what it shows made once, shared with what places it: the
                // text it showed before kept when the same (a caret moved).
                let last = match &ex.content {
                    Pending::Input(p) => Some(&p.shown),
                    _ => None,
                };
                let (shown, preedit) = n.kind.field.edit.with(|e| n.kind.mode.shown(e, last));
                let content = Arc::clone(&shown.text);
                (
                    Pending::Input(Box::new(PendingInput {
                        field: n.kind.field,
                        shown,
                        preedit,
                        placeholder: n.kind.placeholder.clone(),
                        placeholder_color: style.placeholder_color,
                        selection_color: style.selection_color,
                        multiline: n.kind.multiline,
                        mode: n.kind.mode,
                    })),
                    Some(Leaf::Input {
                        content,
                        text: text.clone(),
                        multiline: n.kind.multiline,
                    }),
                )
            }
            Element::Figures(n) => (Pending::Figures(n.kind.clone()), None),
            Element::Drawn(n) => {
                let f = &n.kind.make;
                // A cold build reuses the drawing the live node holds.
                let d = match self.held.get(&ex.path).and_then(|h| h.draw.clone()) {
                    Some(held) => held,
                    None => ex
                        .draw_binding
                        .get_or_insert_with(|| Box::new(Tracked::new(&self.marks, u)))
                        .get(f, || f()),
                };
                // A `make` whose first run panicked has drawn nothing yet.
                (d.map_or(Pending::None, Pending::Drawn), None)
            }
            Element::Image(n) => {
                let (w, h) = n.kind.size();
                let percent = |l: Length| l.relative();
                (
                    Pending::Image(n.kind.clone()),
                    Some(Leaf::Image(ImageMeasure {
                        natural: (crate::convert::u32_to_f32(w), crate::convert::u32_to_f32(h)),
                        sized: (style.width != Length::Auto, style.height != Length::Auto),
                        compressible: percent(style.width) || percent(style.max_width),
                    })),
                )
            }
            _ => (Pending::None, None),
        };
        let keep = runs.len().max(ex.last_spans);
        ex.last_spans = runs.len();
        trim_scratch(&mut runs, keep);
        trim_scratch(&mut ex.span_content, keep);
        trim_scratch(&mut ex.span_styles, keep);
        trim_scratch(&mut ex.span_bindings, keep);
        ex.runs = runs;
        if ex.runs.is_empty() {
            ex.spare_runs = Vec::new();
        }
        let mut leaf = leaf.map(Leaf::measuring);
        let leaf_changed = !Leaf::same_but_objects(node.leaf.as_ref(), leaf.as_ref());
        if leaf_changed {
            if let (Some(Leaf::Text { objects: old, .. }), Some(Leaf::Text { objects, .. })) =
                (&mut node.leaf, &mut leaf)
            {
                std::mem::swap(old, objects);
            }
            node.leaf = leaf;
        }
        let field = match &content {
            Pending::Input(i) => Some(i.field),
            _ => None,
        };
        let inert = inherit.inert || own_inert;
        let passes = inherit.passes || own_passes;
        let pass_through = common.pass_through || inert || passes;
        // An inert node keeps its focus id: it holds no focus as one not
        // shown ([`Frame::shown`]), and focus asked for it is refused.
        let focus = focus_of(common, &ex.path).filter(|_| !disabled);
        let focus_id = focus.map(|h| h.id());
        // Patched to another focus handle (a field shown with another
        // `TextField`): the old one is no longer found here.
        if let Some(old) = ex.focus_id.filter(|&f| Some(f) != focus_id)
            && self.by_focus.get(&old) == Some(&u)
        {
            self.by_focus.remove(&old);
        }
        ex.focus_id = focus_id;
        if !same
            || ex.behaviour.inert != inert
            || ex.behaviour.disabled != disabled
            || ex.behaviour.pass_through != pass_through
        {
            ex.behaviour = Rc::new(Behaviour {
                handlers: common.handlers.clone(),
                focus,
                disabled,
                pass_through,
                cursor: style.cursor,
                field,
                spans,
                scroll: common.scroll,
                scroll_built: common.scroll_built.clone(),
                semantics: ex
                    .attr_binding
                    .as_deref()
                    .map_or(common.attrs_to_resolve().0, AttrBinding::attrs)
                    .told(),
                interaction: ex.interaction.flags(),
                inert,
                tab_stop: common.tab_stop.clone(),
                tab_reversed,
            });
            ex.fresh_behaviour = true;
        }
        ex.name = common.name;
        ex.text = text.clone();
        ex.opacity = style.opacity;
        ex.invisible = invisible;
        ex.dormant = dormant;
        ex.content = content;
        let shows = match &ex.content {
            Pending::Input(p) => Some(p.field.focus.id()),
            _ => None,
        };
        if ex.claim != shows {
            // Patched to show another handle (or none): it gives up the
            // old claim, and claims the new one once the frame is whole.
            if let Some(old) = ex.claim.take()
                && self.by_field.get(&old) == Some(&u)
            {
                self.by_field.remove(&old);
            }
            if let Some(f) = shows {
                self.mounting.push((f, u));
            }
        }
        ex.layer = layer;
        let mine = Inherit {
            text,
            invisible,
            dormant,
            inert,
            passes,
            disabled,
            block: style.display == Display::Block,
        };
        ex.style = PaintStyle::of(style);
        Some(Resolved {
            style_changed,
            leaf_changed,
            mine,
        })
    }

    /// Patch `node`'s children to `kids`, matched by place or key and kind;
    /// whether the list of children changed, and the furthest a child's
    /// change reaches. With `force`, every child is resolved again (what
    /// it inherits changed).
    #[expect(clippy::too_many_arguments, reason = "a node, its kids and what they inherit")]
    fn patch_children(
        &mut self,
        pass: &mut Pass<'_>,
        node: &mut Node,
        kids: &[Element],
        path: &NodePath,
        inherit: &Inherit,
        layer: usize,
        force: bool,
    ) -> (bool, engine::Change) {
        let parent = node.user;
        // Most passes find the same children in the same places: patched
        // where they lie, nothing moved.
        let same = {
            let mut olds = node.children.iter();
            kids.iter()
                .enumerate()
                .filter(|(_, c)| !matches!(c, Element::Deferred(_)) && c.common().is_some())
                .all(|(i, c)| {
                    let seg = c.seg(i);
                    olds.next()
                        .and_then(|n| self.extra(n.user))
                        .is_some_and(|e| e.kind == kind_of(c) && e.path.0.last() == Some(&seg))
                })
                && olds.next().is_none()
        };
        if same {
            let mut reach = engine::Change::None;
            let mut gone = false;
            let mut at = 0_usize;
            let mut keep = Vec::new();
            for (i, child) in kids.iter().enumerate() {
                let seg = child.seg(i);
                if let Element::Deferred(d) = child {
                    pass.todo.push_back(LayerTodo {
                        deferred: Rc::clone(d),
                        path: path.child(seg),
                        anchor: path.clone(),
                        inert: inherit.inert,
                        disabled: inherit.disabled,
                    });
                    continue;
                }
                if child.common().is_none() {
                    continue;
                }
                let Some(n) = node.children.get_mut(at) else { break };
                at = at.saturating_add(1);
                match self.patch(pass, n, child, inherit, layer, force) {
                    Some(change) => {
                        reach = reach.max(change);
                        if gone {
                            keep.push(true);
                        }
                    }
                    None => {
                        if !gone {
                            keep.resize(at.saturating_sub(1), true);
                        }
                        gone = true;
                        keep.push(false);
                    }
                }
            }
            if gone {
                let mut flags = keep.into_iter();
                let (kept, dropped): (Vec<Node>, Vec<Node>) = std::mem::take(&mut node.children)
                    .into_iter()
                    .partition(|_| flags.next().unwrap_or(true));
                node.children = kept;
                for n in dropped {
                    self.hide(parent, n);
                }
                self.structure = true;
            }
            return (gone, reach);
        }
        let mut old: Vec<Option<Node>> = std::mem::take(&mut node.children).into_iter().map(Some).collect();
        // Children are found by place first (a list that did not reorder
        // matches child for child), by key and kind only past a mismatch.
        let mut index: Option<HashMap<(PathSeg, u8), usize>> = None;
        let before = old.len();
        let mut structure = false;
        let mut last_taken: Option<usize> = None;
        let mut reach = engine::Change::None;
        let mut out = Vec::with_capacity(kids.len());
        for (i, child) in kids.iter().enumerate() {
            let seg = child.seg(i);
            if let Element::Deferred(d) = child {
                pass.todo.push_back(LayerTodo {
                    deferred: Rc::clone(d),
                    path: path.child(seg),
                    anchor: path.clone(),
                    inert: inherit.inert,
                    disabled: inherit.disabled,
                });
                continue;
            }
            if child.common().is_none() {
                continue;
            }
            let kind = kind_of(child);
            let here = old.get(i).and_then(Option::as_ref).and_then(|n| self.extra(n.user));
            let at = if here.is_some_and(|e| e.kind == kind && e.path.0.last() == Some(&seg)) {
                Some(i)
            } else {
                index
                    .get_or_insert_with(|| {
                        old.iter()
                            .enumerate()
                            .filter_map(|(i, n)| {
                                let e = self.extra(n.as_ref()?.user)?;
                                Some(((*e.path.0.last()?, e.kind), i))
                            })
                            .collect()
                    })
                    .get(&(seg, kind))
                    .copied()
            };
            let found = at.and_then(|at| Some((at, old.get_mut(at)?.take()?)));
            match found {
                Some((at, mut n)) => {
                    if last_taken.is_some_and(|l| at < l) {
                        structure = true;
                    }
                    last_taken = Some(at);
                    match self.patch(pass, &mut n, child, inherit, layer, force) {
                        Some(change) => {
                            reach = reach.max(change);
                            out.push(n);
                        }
                        None => {
                            structure = true;
                            self.hide(parent, n);
                        }
                    }
                }
                None => {
                    if let Some(n) = self.build(pass, child, path.child(seg), inherit, layer, parent) {
                        structure = true;
                        out.push(n);
                    }
                }
            }
        }
        for n in old.into_iter().flatten() {
            structure = true;
            self.release(n);
        }
        structure |= out.len() != before;
        node.children = out;
        if structure {
            self.structure = true;
        }
        (structure, reach)
    }

    /// Its style phase: its style with each bound write over it, run after
    /// its flags are bound and written ([`Self::resolve_own`]'s order).
    fn resolve_style(&self, common: &Common, ex: &mut Extra, u: u32) -> Style {
        match common.bindings.ops() {
            Some(ops) => {
                let base = &common.style;
                // A cold build writes each bound write the live node holds
                // over a panic as it last gave, never running it again;
                // every other write runs, so it is checked (ADR-0015, rule
                // 12: a hold per setter).
                let held = self.held.get(&ex.path).map_or(&[][..], |h| h.style.as_slice());
                ex.style_binding
                    .get_or_insert_with(|| Box::new(StyleBinding::new(&self.marks, u)))
                    .get(ops, base, held)
            }
            // Nothing bound any more: its trackers go, so what its old
            // closures read resolves nothing (as a span's do).
            None => {
                ex.style_binding = None;
                common.style.clone()
            }
        }
    }

    /// Bind the node's flags to the handle its element gives and write
    /// them as the window's input state has them, `disabled` as given:
    /// a handle a render made reads the node as it is. Hit testing and
    /// focus write them from then on. [`Self::resolve_own`] holds the
    /// order its bindings see them in.
    fn bind_flags(&self, state: &UiState, el: &Element, common: &Common, disabled: bool, ex: &mut Extra, layer: usize) {
        let path = &ex.path;
        let enabled = !disabled;
        let focused = focus_of(common, path).is_some_and(|h| Some(h.id()) == state.focused);
        match common.interaction {
            // A detached tree binds no handle a live node holds: its
            // bindings read the flags the window's own tree bound and
            // wrote, which it leaves alone. A handle its own renders made
            // (no live node has it) it binds, until it goes, and writes as
            // any node writes its own: step 3 of the order writes the
            // `disabled` its attributes gave to the handle step 1 bound.
            Some(handle) if self.detached && handle.is_bound() && ex.interaction.handle() != Some(handle) => {}
            Some(handle) => {
                let i = ex.interaction.bind(handle);
                let (focused, focus_visible, focus_within) = crate::interact::focus_flags(
                    path,
                    layer,
                    focused,
                    matches!(el, Element::Input(_)),
                    state.focus_at.as_ref().map(|(f, l)| (f, *l)),
                    state.focus_visible,
                );
                i.write(crate::interact::Flags {
                    hovered: enabled && state.hovered.contains(path),
                    pressed: enabled && state.active.contains(path),
                    focused,
                    focus_visible,
                    focus_within,
                    disabled: !enabled,
                });
            }
            None => ex.interaction.clear(),
        }
    }
}
