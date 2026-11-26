//! What a screen reader is told about a frame (ADR-0009, ADR-0021): an
//! AccessKit tree of the laid out nodes, each with the role, name and state
//! its [`Semantics`] or its content give it, the window as its root and
//! each layer a child of it.
//!
//! **A node's semantics are a phase of it.** [`Accessibility`] keeps what
//! each node's AccessKit node was built from the last time it was sent (its
//! role, name, state, box, text, and its child list) and, for a new frame,
//! builds and sends only the nodes whose inputs differ: new or changed
//! nodes, and parents whose child list changed. A node gone from the frame
//! is not sent; its parent, sent with the list it has now, removes it with
//! its subtree. A screen reader connecting gets the whole tree once
//! ([`Accessibility::reset`]). Nothing runs while no assistive technology
//! listens: the window asks for an update only then.
//!
//! **Text fields** have a `TextRun` child per line of their text (per part
//! of a line in one direction, at most 256 characters), with each
//! character's length, position and width and where words start, and the
//! field the selection over them, so a screen reader moves by character,
//! word and line and follows the caret. `SetTextSelection` comes back as
//! input ([`selection_in`]).
//!
//! **Rows** (a node with a row index, as [`crate::list::virtual_list`] marks
//! its rows) take the role their list's role implies and their place in the
//! whole list: position and size of set, row index; the list carries the
//! row count.
//!
//! **Scrolling moves one node.** A scroll container's node carries the
//! translation to its content (AccessKit's `transform`), and the nodes
//! inside it have their boxes relative to that content, as Chromium's
//! accessibility tree keeps a scroll offset apart from the boxes under it:
//! a scroll sends the container alone.
//!
//! **Layers** (ADR-0024): a modal layer's region is marked modal, and
//! while it is open every subtree below it (the page, the layers under it)
//! is hidden, in the same update that moves focus into it; closing shows
//! them again in the update that gives focus back. An inert subtree is
//! left out.
//!
//! **Relations** (`labelled_by`, `described_by`, `controls`, `owns`,
//! `active_descendant`) name their targets by [`NodeRef`], never by a
//! string id, and are told only towards targets the tree holds: a node
//! is sent again when one of its targets comes or goes.
//!
//! Ids are hashes of node paths, a text run's of its field's id and its
//! index; debug builds check that no two nodes of a frame share one.

use std::borrow::Cow;
use std::hash::{BuildHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use accesskit::{
    Action, Affine, Node, NodeId, Rect, Role, TextDirection, TextPosition, TextSelection, Toggled, TreeId, TreeInfo,
    TreeUpdate,
};

use crate::element::{InputType, Relations, Semantics};
use crate::focus::FocusId;
use crate::geometry::Bounds;
use crate::layout::{Behaviour, Changed, Content, Frame, FrameNode, ReadAt, Since, input_text};
use crate::style::ResolvedText;
use crate::text::{TextRun, TextSystem};

/// Accessibility's reader of laid-out geometry ([`crate::layout::Sight`]).
#[derive(Clone, Copy)]
pub(crate) struct Sight(());

/// Accessibility's [`Sight`].
const SIGHT: Sight = Sight(());

/// The window, the root every layer hangs from.
const WINDOW: NodeId = NodeId(0);

/// The node refs made, numbered.
static NEXT_REF: crate::engine::IdKind = crate::engine::IdKind::new("node ref");

/// A node's name for other nodes' relations to it: made once (a
/// component's setup), given to the node with `.node_ref`, and to others'
/// `.labelled_by`, `.controls`… A relation to a node not in the tree is
/// not told.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeRef(u64);

impl Default for NodeRef {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeRef {
    #[must_use]
    pub fn new() -> Self {
        Self(crate::engine::unique_id(&NEXT_REF))
    }
}

/// A node's relations as told: towards the nodes the tree holds.
#[derive(Clone, Debug, Default, PartialEq)]
struct Told {
    labelled_by: Vec<NodeId>,
    described_by: Vec<NodeId>,
    controls: Vec<NodeId>,
    owns: Vec<NodeId>,
    active_descendant: Option<NodeId>,
}

impl Told {
    fn of(r: &Relations, refs: &foldhash::HashMap<NodeRef, NodeId>) -> Self {
        let ids = |v: &[NodeRef]| v.iter().filter_map(|n| refs.get(n).copied()).collect();
        Self {
            labelled_by: ids(&r.labelled_by),
            described_by: ids(&r.described_by),
            controls: ids(&r.controls),
            owns: ids(&r.owns),
            active_descendant: r.active_descendant.and_then(|n| refs.get(&n).copied()),
        }
    }

    fn apply(&self, node: &mut Node) {
        if !self.labelled_by.is_empty() {
            node.set_labelled_by(self.labelled_by.clone());
        }
        if !self.described_by.is_empty() {
            node.set_described_by(self.described_by.clone());
        }
        if !self.controls.is_empty() {
            node.set_controls(self.controls.clone());
        }
        if !self.owns.is_empty() {
            node.set_owns(self.owns.clone());
        }
        if let Some(a) = self.active_descendant {
            node.set_active_descendant(a);
        }
    }
}

/// The hasher ids are made with: fast and the same in every run.
fn hasher() -> impl Hasher {
    foldhash::fast::FixedState::with_seed(0xa11e_5021).build_hasher()
}

/// A node's identity across frames: its published [`crate::geometry::NodeId`]
/// (ADR-0026), kept through a keyed move and a frame that rebuilds
/// everything, never another node's.
pub(crate) fn id_of(id: crate::geometry::NodeId) -> NodeId {
    NodeId(id.published())
}

/// The id of text run `index` of the field `parent`: in the synthetic
/// domain (the top bits 2), so it never names a node.
fn run_id(parent: NodeId, index: usize) -> NodeId {
    let mut h = hasher();
    (parent.0, index, "run").hash(&mut h);
    NodeId((h.finish() & ((1 << 62) - 1)) | (2 << 62))
}

/// The node an action from assistive technology names, in `frame`: none
/// when the tree leaves it out (it or an ancestor hidden, inert or not
/// rendered), so an action queued for a node since dropped does nothing.
#[must_use]
pub fn target(frame: &Frame, id: NodeId) -> Option<usize> {
    let i = frame.nodes.iter().position(|n| id_of(n.id) == id)?;
    // Shown ([`Frame::shown`]), and out of the tree by none of its
    // ancestors' semantics.
    let in_tree = |k: usize| frame.shown(k) && frame.behaviour(k).is_some_and(|b| !b.semantics.hidden);
    let mut at = Some(i);
    while let Some(k) = at {
        if !in_tree(k) {
            return None;
        }
        at = frame.nodes.get(k).and_then(|n| n.parent);
    }
    Some(i)
}

/// The whole tree for `frame`, laid out at `scale` physical pixels per
/// logical one, with focus on the node `focused` names.
#[must_use]
pub fn tree(frame: &Frame, focused: Option<FocusId>, scale: f32, title: &str, text: &mut TextSystem) -> TreeUpdate {
    Accessibility::default().update(frame, focused, scale, title, text)
}

/// What the last update cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccessStats {
    /// Nodes sent (text runs included).
    pub sent: usize,
    /// Frame nodes read.
    pub visited: usize,
    /// Nodes in the tree.
    pub nodes: usize,
    /// Whether it was the whole tree.
    pub full: bool,
    /// Text runs compared with those sent before, to send only what changed.
    pub compared: usize,
    /// Text runs built: only for a field whose text, box or look changed,
    /// never for its caret or selection alone.
    pub runs_built: usize,
    pub took: Duration,
}

/// How much of a kept node an update changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Dirty {
    /// Nothing of its own (its child list, perhaps).
    No,
    /// The node, not a field's runs: its caret or selection moved, or it
    /// scrolled (its runs are in its content's space).
    Node,
    /// A field's text, box or look: its runs are built again.
    Runs,
}

/// What a field's text runs are built from (its caret and selection
/// apart: they move over the runs, which stay).
#[derive(Clone, Debug, PartialEq)]
struct Field {
    area: Bounds,
    text: ResolvedText,
    multiline: bool,
    /// Being edited: its text is never cut short.
    focused: bool,
}

/// Everything a node's AccessKit node is built from (its children aside).
/// Borrowed from the frame while compared; owned once kept. Boxes are in
/// the space of the nearest scroll container's content around it.
#[derive(Clone, Debug, PartialEq)]
struct Inputs<'f> {
    role: Role,
    bounds: Bounds,
    /// A scroll container's move from its parent's space to its content's.
    transform: Option<(f32, f32)>,
    label: Option<Cow<'f, str>>,
    value: Option<Cow<'f, str>>,
    placeholder: Option<Cow<'f, str>>,
    field: Option<Field>,
    /// A field's selection, (anchor, caret), in what it shows.
    selection: Option<(usize, usize)>,
    read_only: bool,
    focusable: bool,
    clickable: bool,
    scrolls_into_view: bool,
    disabled: bool,
    checked: Option<Toggled>,
    selected: Option<bool>,
    expanded: Option<bool>,
    orientation: Option<crate::style::Axis>,
    /// Its value in a range, and whether its keys step it.
    range: Option<Cow<'f, crate::element::ValueRange>>,
    row_count: Option<usize>,
    /// Its index among its list's rows.
    row_index: Option<usize>,
    /// Its place in its set, from 1, and the set's size when known.
    set: Option<(usize, Option<usize>)>,
    /// It is a modal layer's region (`aria-modal`).
    modal: bool,
    /// A root below the top modal layer: hidden with its subtree.
    hidden: bool,
    relations: Cow<'f, Relations>,
    announce: Cow<'f, crate::element::Announce>,
}

fn owned(c: Option<Cow<'_, str>>) -> Option<Cow<'static, str>> {
    c.map(|c| Cow::Owned(c.into_owned()))
}

impl Inputs<'_> {
    fn into_owned(self) -> Inputs<'static> {
        Inputs {
            label: owned(self.label),
            value: owned(self.value),
            placeholder: owned(self.placeholder),
            role: self.role,
            bounds: self.bounds,
            transform: self.transform,
            field: self.field,
            selection: self.selection,
            read_only: self.read_only,
            focusable: self.focusable,
            clickable: self.clickable,
            scrolls_into_view: self.scrolls_into_view,
            disabled: self.disabled,
            checked: self.checked,
            selected: self.selected,
            expanded: self.expanded,
            orientation: self.orientation,
            range: self.range.map(|r| Cow::Owned(r.into_owned())),
            row_count: self.row_count,
            row_index: self.row_index,
            set: self.set,
            modal: self.modal,
            hidden: self.hidden,
            relations: Cow::Owned(self.relations.into_owned()),
            announce: Cow::Owned(self.announce.into_owned()),
        }
    }
}

/// A node as last sent.
struct Kept {
    inputs: Inputs<'static>,
    /// Its children in the frame (its text runs apart).
    children: Vec<NodeId>,
    /// A field's text runs.
    runs: Vec<(NodeId, Node)>,
    /// Its relations as last told.
    told: Told,
    /// The update that last saw it.
    stamp: u64,
}

/// The window's node as last sent.
#[derive(PartialEq)]
struct WindowKept {
    title: String,
    scale: f32,
    viewport: Bounds,
    children: Vec<NodeId>,
}

/// A node of the frame as last read, once it is known to be shown: what
/// its children's inputs are read under.
#[derive(Clone, Copy, PartialEq)]
struct Slot {
    id: NodeId,
    /// The role and row count of the nearest list around it (itself included).
    set: Option<(Role, usize)>,
    /// It is inside a scroll container (or is one).
    scrolls: bool,
    /// It or a node around it is unavailable ([`crate::Div::unavailable`]):
    /// read as disabled, as Chrome reads what an `aria-disabled` element
    /// holds, though it keeps its focus and its own actions.
    unavailable: bool,
    /// Where the space its children's boxes are in starts, in window pixels:
    /// a scroll container's content, moving with it.
    space: (f32, f32),
}

/// What reading one frame node gave.
struct Read {
    slot: Option<Slot>,
    /// How its inputs differ from those last sent (all, when it is new).
    dirty: Dirty,
    focus: Option<FocusId>,
    /// Its name for relations.
    node: Option<NodeRef>,
}

/// The tree a screen reader holds, kept so each frame reads and sends only
/// what changed (ADR-0021).
///
/// It keeps, per frame node, what the last update read (whether it is
/// shown, the space and list its children are read in, its focus) and the
/// frame's child lists. An update after one that read the same frame
/// structure reads only the nodes the frame's change log names (written in
/// place by layout), their subtrees where what their children are read
/// under changed, and the tabs above them (a tab is named by the text
/// inside it); it sends those whose inputs changed and the parents whose
/// list of shown children changed. A frame rebuilt (nodes came, went or
/// moved) is read whole, still sending only what differs.
#[derive(Default)]
pub struct Accessibility {
    kept: foldhash::HashMap<NodeId, Kept>,
    window: Option<WindowKept>,
    stamp: u64,
    /// Where it read the frame's change log up to.
    read: ReadAt,
    /// Per frame node, as last read.
    slots: Vec<Option<Slot>>,
    focus_at: Vec<Option<FocusId>>,
    /// Per frame node, its children in the frame, and the nodes without a parent.
    kids: Vec<Vec<u32>>,
    roots: Vec<u32>,
    /// The node each focus handle is on.
    focus: foldhash::HashMap<FocusId, NodeId>,
    /// Per frame node, its name for relations, as last read.
    ref_at: Vec<Option<NodeRef>>,
    /// The node each [`NodeRef`] names.
    refs: foldhash::HashMap<NodeRef, NodeId>,
    /// The nodes that have relations.
    relating: foldhash::HashSet<NodeId>,
    /// The layers of the frame last read, and its top modal layer's floor
    /// then (everything before it is hidden).
    layers: crate::layer::Stack,
    floor: usize,
    /// Text runs kept, over every field.
    runs: usize,
    /// Scratch kept across updates.
    seen: Vec<u64>,
    list: Vec<NodeId>,
    stats: AccessStats,
}

impl Accessibility {
    /// Forget what was sent: the next update is the whole tree (a screen
    /// reader connected again, or went away).
    pub fn reset(&mut self) {
        self.kept.clear();
        self.window = None;
        self.read = ReadAt::default();
    }

    /// What the last update cost.
    #[must_use]
    pub fn stats(&self) -> AccessStats {
        self.stats
    }

    /// The update that brings the tree last sent to `frame` (the whole
    /// tree when none was sent), laid out at `scale`, focus on `focused`.
    pub fn update(
        &mut self,
        frame: &Frame,
        focused: Option<FocusId>,
        scale: f32,
        title: &str,
        text: &mut TextSystem,
    ) -> TreeUpdate {
        let started = Instant::now();
        let full = self.window.is_none();
        self.stamp = self.stamp.wrapping_add(1);
        let log = frame.changes.since(&mut self.read);
        // A modal layer came or went: what is hidden changed everywhere.
        self.layers.refresh(frame);
        let refloor = self.layers.floor != self.floor;
        self.floor = self.layers.floor;
        // What to send: frame indices, and how their inputs changed (not
        // at all: only their child list).
        let mut emit: Vec<(usize, Dirty)> = Vec::new();
        let mut window_moved = false;
        let visited = match log {
            Since::Logged(log) if !full && !refloor && self.slots.len() == frame.nodes.len() => {
                self.patch(frame, log, &mut emit, &mut window_moved)
            }
            Since::Logged(_) | Since::Rebuilt | Since::FellBehind => {
                window_moved = true;
                self.walk(frame, &mut emit)
            }
        };
        emit.sort_unstable_by_key(|&(i, _)| i);
        emit.dedup_by(|b, a| {
            if a.0 == b.0 {
                a.1 = a.1.max(b.1);
                true
            } else {
                false
            }
        });
        let mut nodes = Vec::with_capacity(emit.len());
        let (mut compared, mut runs_built) = (0_usize, 0_usize);
        for &(i, dirty) in &emit {
            let Some(slot) = self.slots.get(i).copied().flatten() else {
                continue;
            };
            let Some(k) = self.kept.get_mut(&slot.id) else { continue };
            // A caret moved or a field scrolled keeps the runs sent.
            let dirty = dirty == Dirty::Runs;
            if dirty && let Some(field) = &k.inputs.field {
                let value = k.inputs.value.as_deref().unwrap_or("");
                let runs = run_nodes(slot.id, value, field, text);
                runs_built = runs_built.saturating_add(runs.len());
                // A run's id is its index ([`run_id`]): each compared with
                // the one sent at its index alone.
                for (i, (rid, run)) in runs.iter().enumerate() {
                    compared = compared.saturating_add(1);
                    if k.runs.get(i).is_none_or(|(kid, kn)| kid != rid || kn != run) {
                        nodes.push((*rid, run.clone()));
                    }
                }
                self.runs = self.runs.saturating_sub(k.runs.len()).saturating_add(runs.len());
                k.runs = runs;
            } else if dirty && !k.runs.is_empty() {
                self.runs = self.runs.saturating_sub(k.runs.len());
                k.runs.clear();
            }
            k.told = Told::of(&k.inputs.relations, &self.refs);
            nodes.push((slot.id, build(k)));
        }
        // A node whose relation's target came or went is told again.
        if !self.relating.is_empty() {
            let relating: Vec<NodeId> = self.relating.iter().copied().collect();
            for id in relating {
                let Some(k) = self.kept.get_mut(&id).filter(|k| !k.inputs.relations.is_empty()) else {
                    self.relating.remove(&id);
                    continue;
                };
                let told = Told::of(&k.inputs.relations, &self.refs);
                if told != k.told && !nodes.iter().any(|(n, _)| *n == id) {
                    k.told = told;
                    nodes.push((id, build(k)));
                }
            }
        }

        let children = if window_moved {
            self.roots
                .iter()
                .filter_map(|&r| {
                    self.slots
                        .get(usize::try_from(r).unwrap_or(usize::MAX))
                        .copied()
                        .flatten()
                        .map(|s| s.id)
                })
                .collect()
        } else {
            self.window.as_ref().map(|w| w.children.clone()).unwrap_or_default()
        };
        let window = WindowKept {
            title: title.to_owned(),
            scale,
            viewport: frame.viewport(SIGHT),
            children,
        };
        if self.window.as_ref() != Some(&window) {
            let mut node = Node::new(Role::Window);
            node.set_label(title);
            node.set_transform(Affine::scale(f64::from(scale)));
            node.set_bounds(rect(frame.viewport(SIGHT)));
            node.set_children(window.children.clone());
            nodes.push((WINDOW, node));
            self.window = Some(window);
        }
        let tree = full.then(|| {
            let mut info = TreeInfo::new(WINDOW);
            info.toolkit_name = Some("gui".to_owned());
            info
        });
        let focus = focused.and_then(|f| self.focus.get(&f).copied()).unwrap_or(WINDOW);
        self.stats = AccessStats {
            sent: nodes.len(),
            visited,
            nodes: self.kept.len().saturating_add(1).saturating_add(self.runs),
            full,
            compared,
            runs_built,
            took: started.elapsed(),
        };
        TreeUpdate {
            nodes,
            tree,
            tree_id: TreeId::ROOT,
            focus,
        }
    }

    /// Read every node of `frame`; how many were read.
    fn walk(&mut self, frame: &Frame, emit: &mut Vec<(usize, Dirty)>) -> usize {
        let stamp = self.stamp;
        let count = frame.nodes.len();
        self.slots.clear();
        self.focus_at.clear();
        self.focus.clear();
        self.ref_at.clear();
        self.refs.clear();
        self.roots.clear();
        if self.kids.len() < count {
            self.kids.resize_with(count, Vec::new);
        }
        self.kids.truncate(count);
        for k in &mut self.kids {
            k.clear();
        }
        let mut seen = 0_usize;
        for (i, n) in frame.nodes.iter().enumerate() {
            let at = u32::try_from(i).unwrap_or(u32::MAX);
            match n.parent.and_then(|p| self.kids.get_mut(p)) {
                Some(k) => k.push(at),
                None => self.roots.push(at),
            }
            let r = self.read(frame, i, stamp);
            if let Some(slot) = r.slot {
                seen = seen.saturating_add(1);
                if let Some(f) = r.focus {
                    self.focus.insert(f, slot.id);
                }
                if let Some(n) = r.node {
                    self.refs.insert(n, slot.id);
                }
                if r.dirty != Dirty::No {
                    emit.push((i, r.dirty));
                }
            }
            self.focus_at.push(r.focus.filter(|_| r.slot.is_some()));
            self.ref_at.push(r.node.filter(|_| r.slot.is_some()));
            self.slots.push(r.slot);
        }
        if seen != self.kept.len() {
            self.kept.retain(|_, k| k.stamp == stamp);
        }
        self.runs = self.kept.values().map(|k| k.runs.len()).sum();
        #[cfg(debug_assertions)]
        for k in self.kept.values() {
            for (rid, _) in &k.runs {
                assert!(
                    !self.kept.contains_key(rid),
                    "a text run's accessibility id {rid:?} is a node's"
                );
            }
        }
        for i in 0..count {
            if self.relist(i) {
                emit.push((i, Dirty::No));
            }
        }
        count
    }

    /// Read the nodes `log` names (and what depends on them) again; how
    /// many were read.
    fn patch(&mut self, frame: &Frame, log: &[Changed], emit: &mut Vec<(usize, Dirty)>, window: &mut bool) -> usize {
        let stamp = self.stamp;
        let count = frame.nodes.len();
        self.seen.resize(count, 0);
        let mut todo: Vec<usize> = Vec::with_capacity(log.len());
        for c in log {
            let i = usize::try_from(c.index).unwrap_or(usize::MAX);
            if i >= count {
                continue;
            }
            todo.push(i);
            // A tab without a label of its own is named by the text inside it.
            let mut up = frame.nodes.get(i).and_then(|n| n.parent);
            while let Some(p) = up {
                let named_within = self
                    .slots
                    .get(p)
                    .copied()
                    .flatten()
                    .and_then(|s| self.kept.get(&s.id))
                    .is_some_and(|k| k.inputs.role == Role::Tab)
                    && frame.behaviour(p).is_some_and(|b| b.semantics.label.is_none());
                if named_within {
                    todo.push(p);
                }
                up = frame.nodes.get(p).and_then(|n| n.parent);
            }
        }
        todo.sort_unstable();
        todo.dedup();
        let mut relist: Vec<usize> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut visited = 0_usize;
        for i in todo {
            stack.push(i);
            while let Some(j) = stack.pop() {
                if self.seen.get(j).is_none_or(|&s| s == stamp) {
                    continue;
                }
                if let Some(s) = self.seen.get_mut(j) {
                    *s = stamp;
                }
                visited = visited.saturating_add(1);
                let old = self.slots.get(j).copied().flatten();
                let r = self.read(frame, j, stamp);
                let was = self.focus_at.get(j).copied().flatten();
                if let (Some(f), Some(o)) = (was, old)
                    && self.focus.get(&f) == Some(&o.id)
                {
                    self.focus.remove(&f);
                }
                let now = r.focus.filter(|_| r.slot.is_some());
                if let (Some(f), Some(slot)) = (now, r.slot) {
                    self.focus.insert(f, slot.id);
                }
                if let Some(at) = self.focus_at.get_mut(j) {
                    *at = now;
                }
                let was_ref = self.ref_at.get(j).copied().flatten();
                if let (Some(n), Some(o)) = (was_ref, old)
                    && self.refs.get(&n) == Some(&o.id)
                {
                    self.refs.remove(&n);
                }
                let now_ref = r.node.filter(|_| r.slot.is_some());
                if let (Some(n), Some(slot)) = (now_ref, r.slot) {
                    self.refs.insert(n, slot.id);
                }
                if let Some(at) = self.ref_at.get_mut(j) {
                    *at = now_ref;
                }
                let parent = frame.nodes.get(j).and_then(|n| n.parent);
                match (old, r.slot) {
                    (Some(o), None) => {
                        if let Some(k) = self.kept.remove(&o.id) {
                            self.runs = self.runs.saturating_sub(k.runs.len());
                        }
                        match parent {
                            Some(p) => relist.push(p),
                            None => *window = true,
                        }
                    }
                    (None, Some(_)) => {
                        emit.push((j, Dirty::Runs));
                        relist.push(j);
                        match parent {
                            Some(p) => relist.push(p),
                            None => *window = true,
                        }
                    }
                    (Some(_), Some(_)) if r.dirty != Dirty::No => emit.push((j, r.dirty)),
                    _ => {}
                }
                if let Some(s) = self.slots.get_mut(j) {
                    *s = r.slot;
                }
                // What its children are read under changed: read them too.
                if old != r.slot
                    && let Some(kids) = self.kids.get(j)
                {
                    stack.extend(kids.iter().rev().map(|&k| usize::try_from(k).unwrap_or(usize::MAX)));
                }
            }
        }
        relist.sort_unstable();
        relist.dedup();
        for p in relist {
            if self.relist(p) {
                emit.push((p, Dirty::No));
            }
        }
        visited
    }

    /// Read frame node `i`, its parent read already.
    fn read(&mut self, frame: &Frame, i: usize, stamp: u64) -> Read {
        const NONE: Read = Read {
            slot: None,
            dirty: Dirty::No,
            focus: None,
            node: None,
        };
        let Some(n) = frame.nodes.get(i) else { return NONE };
        let parent = match n.parent {
            Some(p) => match self.slots.get(p).copied().flatten() {
                Some(s) => Some(s),
                None => return NONE,
            },
            None => None,
        };
        let Some(b) = frame.behaviour(i).filter(|b| frame.shown(i) && !b.semantics.hidden) else {
            return NONE;
        };
        let set = parent.and_then(|s| s.set);
        let scrolls = parent.is_some_and(|s| s.scrolls);
        let unavailable = b.semantics.unavailable || parent.is_some_and(|s| s.unavailable);
        let outer = parent.map_or((0.0, 0.0), |s| s.space);
        let space = n.scroll(SIGHT).map_or(outer, |sc| {
            let rtl = n.look.text.direction == crate::style::Direction::Rtl;
            let x = if rtl {
                n.bounds(SIGHT).x + sc.offset.0
            } else {
                n.bounds(SIGHT).x - sc.offset.0
            };
            (x, n.bounds(SIGHT).y - sc.offset.1)
        });
        let hidden = n.parent.is_none() && i < self.floor;
        let Some(mut inputs) = inputs_of(frame, i, b, set, scrolls, unavailable, outer, space, hidden) else {
            return NONE;
        };
        let id = id_of(n.id);
        if !inputs.relations.is_empty() {
            self.relating.insert(id);
        }
        let dirty = match self.kept.get_mut(&id) {
            Some(k) => {
                debug_assert!(
                    k.stamp != stamp,
                    "two nodes share the accessibility id {id:?} ({})",
                    frame.path_of(i)
                );
                k.stamp = stamp;
                // Compared with the selection kept: a caret moved alone
                // moves over the runs kept, and its text is not taken again.
                let selection = std::mem::replace(&mut inputs.selection, k.inputs.selection);
                if k.inputs == inputs {
                    if k.inputs.selection == selection {
                        Dirty::No
                    } else {
                        k.inputs.selection = selection;
                        Dirty::Node
                    }
                } else {
                    let runs = k.inputs.field != inputs.field || k.inputs.value != inputs.value;
                    inputs.selection = selection;
                    k.inputs = inputs.into_owned();
                    if runs { Dirty::Runs } else { Dirty::Node }
                }
            }
            None => {
                self.kept.insert(
                    id,
                    Kept {
                        inputs: inputs.into_owned(),
                        children: Vec::new(),
                        runs: Vec::new(),
                        told: Told::default(),
                        stamp,
                    },
                );
                Dirty::Runs
            }
        };
        let s = &b.semantics;
        let role = self.kept.get(&id).map_or(Role::Unknown, |k| k.inputs.role);
        Read {
            slot: Some(Slot {
                id,
                set: s.row_count.map(|rows| (role, rows)).or(set),
                scrolls: scrolls || n.scroll(SIGHT).is_some(),
                unavailable,
                space,
            }),
            dirty,
            focus: b.focus.map(|h| h.id()),
            node: s.node,
        }
    }

    /// Bring the kept child list of node `i` (if shown) to its shown
    /// children; whether it changed.
    fn relist(&mut self, i: usize) -> bool {
        let Some(slot) = self.slots.get(i).copied().flatten() else {
            return false;
        };
        self.list.clear();
        if let Some(kids) = self.kids.get(i) {
            self.list.extend(kids.iter().filter_map(|&c| {
                self.slots
                    .get(usize::try_from(c).unwrap_or(usize::MAX))
                    .copied()
                    .flatten()
                    .map(|s| s.id)
            }));
        }
        let Some(k) = self.kept.get_mut(&slot.id) else {
            return false;
        };
        if k.children == self.list {
            false
        } else {
            k.children.clone_from(&self.list);
            true
        }
    }

    /// Whether this holds the tree `other` holds (the debug check that an
    /// update read incrementally equals one read whole); what differs.
    pub(crate) fn differs_from(&self, other: &Self) -> Option<String> {
        if self.window != other.window {
            return Some("the window node".to_owned());
        }
        if self.kept.len() != other.kept.len() {
            return Some(format!("{} nodes kept against {}", self.kept.len(), other.kept.len()));
        }
        for (id, k) in &self.kept {
            let Some(o) = other.kept.get(id) else {
                return Some(format!("node {id:?} is not in the tree read whole"));
            };
            if k.inputs != o.inputs {
                return Some(format!("node {id:?}: {:?} against {:?}", k.inputs, o.inputs));
            }
            if k.children != o.children {
                return Some(format!(
                    "node {id:?}'s children: {:?} against {:?}",
                    k.children, o.children
                ));
            }
            if k.runs != o.runs {
                return Some(format!("node {id:?}'s text runs"));
            }
        }
        if self.focus != other.focus {
            return Some("the focus map".to_owned());
        }
        if self.refs != other.refs {
            return Some("the node refs".to_owned());
        }
        None
    }
}

/// What node `i` is built from; `None` leaves it out (an icon or a drawing
/// with no name says nothing).
#[expect(clippy::too_many_arguments, reason = "a node and what it is read under")]
fn inputs_of<'f>(
    frame: &'f Frame,
    i: usize,
    b: &'f Behaviour,
    set: Option<(Role, usize)>,
    scrolls: bool,
    unavailable: bool,
    outer: (f32, f32),
    space: (f32, f32),
    hidden: bool,
) -> Option<Inputs<'f>> {
    let n = frame.nodes.get(i)?;
    let s = &b.semantics;
    let role = s
        .role
        .or_else(|| s.row_index.map(|_| row_role(set.map(|(r, _)| r))))
        .or_else(|| implied_role(n, b, s))?;
    let label = match &s.label {
        Some(label) => Some(Cow::Borrowed(label.as_str())),
        // AccessKit names a button from the text inside it, but not a tab.
        None if role == Role::Tab => Some(Cow::Owned(text_within(frame, i))),
        None => None,
    };
    let mut inputs = Inputs {
        role,
        bounds: local(n.bounds(SIGHT), space),
        transform: (space != outer).then_some((space.0 - outer.0, space.1 - outer.1)),
        label,
        value: None,
        placeholder: None,
        field: None,
        selection: None,
        read_only: false,
        focusable: b.focus.is_some(),
        clickable: b.handlers.click.is_some() && b.takes_clicks(),
        scrolls_into_view: scrolls,
        // Disabled is inherited in the frame already; unavailable is read
        // down from the slot around it.
        disabled: b.disabled || unavailable,
        checked: s.checked,
        selected: s.selected,
        expanded: s.expanded,
        orientation: s.orientation,
        range: s.range.as_ref().map(Cow::Borrowed),
        row_count: s.row_count,
        row_index: s.row_index,
        set: s
            .set
            .map(|(at, of)| (at, Some(of)))
            .or_else(|| s.row_index.map(|r| (r.saturating_add(1), set.map(|(_, rows)| rows)))),
        modal: b.handlers.layer.as_ref().is_some_and(|l| l.is_modal()),
        hidden,
        relations: Cow::Borrowed(&s.relations),
        announce: Cow::Borrowed(&s.announce),
    };
    match &n.content {
        // Its atomic inlines are nodes of their own: their places are not read.
        Content::Text { content, .. } => inputs.value = Some(crate::text::readable(content)),
        Content::Input {
            edit,
            placeholder,
            mode,
            text,
            multiline,
            focused,
            ..
        } => {
            inputs.value = Some(Cow::Borrowed(&*edit.text));
            inputs.placeholder = (!placeholder.is_empty()).then_some(Cow::Borrowed(placeholder.as_str()));
            inputs.read_only = mode.read_only;
            inputs.selection = Some((edit.anchor, edit.caret));
            // Its runs are in its content's space as laid out unscrolled:
            // the node's transform carries its scroll, so a scroll alone
            // moves the node and none of its runs.
            let unscrolled = n
                .scroll(SIGHT)
                .map_or(space, |_| (n.bounds(SIGHT).x, n.bounds(SIGHT).y));
            inputs.field = Some(Field {
                area: local(n.content_box(SIGHT), unscrolled),
                text: text.clone(),
                multiline: *multiline,
                focused: *focused,
            });
        }
        _ => {}
    }
    Some(inputs)
}

/// The AccessKit node kept as `k`: its inputs, its text selection, its
/// children and text runs, its relations as told.
fn build(k: &Kept) -> Node {
    let mut node = node_of(&k.inputs);
    if let Some(selection) = k.inputs.selection
        && let Some(sel) = selection_of(&k.runs, selection)
    {
        node.set_text_selection(sel);
    }
    let mut all = k.children.clone();
    all.extend(k.runs.iter().map(|(rid, _)| *rid));
    node.set_children(all);
    k.told.apply(&mut node);
    node
}

/// The AccessKit node `inputs` describe, children aside.
fn node_of(inputs: &Inputs<'_>) -> Node {
    let mut node = Node::new(inputs.role);
    if inputs.modal {
        node.set_modal();
    }
    if inputs.hidden {
        node.set_hidden();
    }
    node.set_bounds(rect(inputs.bounds));
    if let Some((x, y)) = inputs.transform {
        node.set_transform(Affine::translate((f64::from(x), f64::from(y))));
    }
    if let Some(label) = &inputs.label {
        node.set_label(&**label);
    }
    if let Some(value) = &inputs.value {
        node.set_value(&**value);
    }
    if let Some(p) = &inputs.placeholder {
        node.set_placeholder(&**p);
    }
    if inputs.field.is_some() {
        node.add_action(Action::SetTextSelection);
        if inputs.read_only {
            node.set_read_only();
        } else {
            node.add_action(Action::SetValue);
        }
    }
    if inputs.focusable {
        node.add_action(Action::Focus);
    }
    if inputs.clickable {
        node.add_action(Action::Click);
    }
    if inputs.scrolls_into_view {
        node.add_action(Action::ScrollIntoView);
    }
    if inputs.disabled {
        node.set_disabled();
    }
    if let Some(checked) = inputs.checked {
        node.set_toggled(checked);
    }
    if let Some(selected) = inputs.selected {
        node.set_selected(selected);
    }
    if let Some(expanded) = inputs.expanded {
        node.set_expanded(expanded);
    }
    if let Some(o) = inputs.orientation {
        node.set_orientation(match o {
            crate::style::Axis::Horizontal => accesskit::Orientation::Horizontal,
            crate::style::Axis::Vertical => accesskit::Orientation::Vertical,
        });
    }
    if let Some(range) = &inputs.range {
        if let Some(now) = range.now {
            node.set_numeric_value(now);
        }
        // An end that is not finite is no end: it is left out (no `aria-valuemin`).
        if range.min.is_finite() {
            node.set_min_numeric_value(range.min);
        }
        if range.max.is_finite() {
            node.set_max_numeric_value(range.max);
        }
        if let Some(text) = &range.text {
            node.set_value(text.as_str());
        }
        if inputs.focusable {
            node.add_action(Action::Increment);
            node.add_action(Action::Decrement);
        }
    }
    if let Some(rows) = inputs.row_count {
        node.set_row_count(rows);
    }
    if let Some(row) = inputs.row_index {
        node.set_row_index(row);
    }
    if let Some((at, of)) = inputs.set {
        node.set_position_in_set(at);
        if let Some(of) = of {
            node.set_size_of_set(of);
        }
    }
    let a = &*inputs.announce;
    if let Some(level) = a.level {
        // AccessKit counts levels from 0, ARIA from 1.
        node.set_level(level.saturating_sub(1));
    }
    if let Some(current) = a.current {
        node.set_aria_current(current);
    }
    if let Some(d) = &a.role_description {
        node.set_role_description(d.as_str());
    }
    if let Some(d) = &a.description {
        node.set_description(d.as_str());
    }
    if let Some(v) = &a.value {
        node.set_value(v.as_str());
    }
    if let Some(live) = a.live {
        node.set_live(live);
    }
    if a.multiselectable {
        node.set_multiselectable();
    }
    if a.required {
        node.set_required();
    }
    if a.invalid {
        node.set_invalid(accesskit::Invalid::True);
    }
    if a.read_only {
        node.set_read_only();
    }
    if let Some(popup) = a.has_popup {
        node.set_has_popup(popup);
    }
    if let Some(complete) = a.auto_complete {
        node.set_auto_complete(complete);
    }
    node
}

/// A field's text runs as laid out where it paints them, in the space its area is in.
fn field_runs(value: &str, field: &Field, text: &mut TextSystem) -> Vec<TextRun> {
    // In the field's content space: its node's transform scrolls it, as
    // a scroll container's moves its children.
    let it = input_text(
        text,
        value,
        &field.text,
        field.area,
        field.multiline,
        field.focused,
        (0.0, 0.0),
    );
    let mut runs = text.text_runs(value, &it.t, it.width);
    for r in &mut runs {
        r.bounds.x += it.origin.0;
        r.bounds.y += it.origin.1;
    }
    runs
}

/// The `TextRun` nodes of the field `parent` showing `value`.
fn run_nodes(parent: NodeId, value: &str, field: &Field, text: &mut TextSystem) -> Vec<(NodeId, Node)> {
    let runs = field_runs(value, field, text);
    // Runs on one visual line are linked on it.
    let lines: Vec<usize> = runs.iter().map(|r| r.line).collect();
    runs.into_iter()
        .enumerate()
        .map(|(i, r)| {
            let mut node = Node::new(Role::TextRun);
            if let Some(before) = i.checked_sub(1).filter(|&b| lines.get(b) == Some(&r.line)) {
                node.set_previous_on_line(run_id(parent, before));
            }
            if let Some(after) = i.checked_add(1).filter(|&a| lines.get(a) == Some(&r.line)) {
                node.set_next_on_line(run_id(parent, after));
            }
            node.set_bounds(rect(r.bounds));
            node.set_value(value.get(r.range.clone()).unwrap_or(""));
            node.set_text_direction(if r.rtl {
                TextDirection::RightToLeft
            } else {
                TextDirection::LeftToRight
            });
            node.set_character_lengths(r.lengths);
            node.set_character_positions(r.positions);
            node.set_character_widths(r.widths);
            node.set_word_starts(r.word_starts);
            (run_id(parent, i), node)
        })
        .collect()
}

/// Where byte `at` of a field's text lies in its runs.
fn position_of(runs: &[(NodeId, Node)], at: usize) -> Option<TextPosition> {
    let mut start = 0_usize;
    let last = runs.len().checked_sub(1)?;
    for (i, (id, node)) in runs.iter().enumerate() {
        let len = node.value().map_or(0, str::len);
        let end = start.saturating_add(len);
        if at < end || i == last {
            let mut index = 0_usize;
            let mut b = start;
            for l in node.character_lengths() {
                let next = b.saturating_add(usize::from(*l));
                if at < next {
                    break;
                }
                b = next;
                index = index.saturating_add(1);
            }
            return Some(TextPosition {
                node: *id,
                character_index: index,
            });
        }
        start = end;
    }
    None
}

/// A field's selection, (anchor, caret), over its runs.
fn selection_of(runs: &[(NodeId, Node)], (anchor, caret): (usize, usize)) -> Option<TextSelection> {
    Some(TextSelection {
        anchor: position_of(runs, anchor)?,
        focus: position_of(runs, caret)?,
    })
}

/// The byte range of the text shown by the input at `i` a selection over
/// its runs names: (anchor, caret), in what the input shows.
pub(crate) fn selection_in(
    frame: &Frame,
    i: usize,
    selection: &TextSelection,
    text: &mut TextSystem,
) -> Option<(usize, usize)> {
    let n = frame.nodes.get(i)?;
    let Content::Input {
        edit,
        text: t,
        multiline,
        focused,
        ..
    } = &n.content
    else {
        return None;
    };
    let field = Field {
        area: n.content_box(SIGHT),
        text: t.clone(),
        multiline: *multiline,
        focused: *focused,
    };
    let parent = id_of(n.id);
    let runs = field_runs(&edit.text, &field, text);
    let byte = |p: &TextPosition| {
        runs.iter()
            .enumerate()
            .find(|(k, _)| run_id(parent, *k) == p.node)
            .map(|(_, r)| r.byte_of(p.character_index).min(r.range.end))
    };
    Some((byte(&selection.anchor)?, byte(&selection.focus)?))
}

/// Whether a node of `role` is a row of a list (it carries its index there).
#[must_use]
pub fn is_row_role(role: Role) -> bool {
    matches!(
        role,
        Role::Row
            | Role::ListItem
            | Role::ListBoxOption
            | Role::TreeItem
            | Role::MenuItem
            | Role::MenuItemCheckBox
            | Role::MenuItemRadio
    )
}

/// The role a row takes in a list of `list`'s role, as ARIA pairs them.
fn row_role(list: Option<Role>) -> Role {
    match list {
        Some(Role::ListBox) => Role::ListBoxOption,
        Some(Role::Table | Role::Grid | Role::TreeGrid | Role::RowGroup) => Role::Row,
        Some(Role::Tree) => Role::TreeItem,
        Some(Role::Menu | Role::MenuBar) => Role::MenuItem,
        _ => Role::ListItem,
    }
}

/// The text of the nodes inside the node at `i`, in reading order: they
/// follow it in the frame until the first that is not inside it.
fn text_within(frame: &Frame, i: usize) -> String {
    let inside = |j: &usize| frame.ancestors(*j).any(|a| a == i);
    let words: Vec<Cow<'_, str>> = (i.saturating_add(1)..frame.nodes.len())
        .take_while(inside)
        .filter_map(|j| match &frame.nodes.get(j)?.content {
            Content::Text { content, .. } => Some(crate::text::readable(content)),
            _ => None,
        })
        .collect();
    words.join(" ")
}

/// The role a node's content implies when it sets none: text is a label, a
/// text box an input of its kind, a box that takes clicks a button.
fn implied_role(n: &FrameNode, b: &Behaviour, s: &Semantics) -> Option<Role> {
    match &n.content {
        Content::Text { .. } => Some(Role::Label),
        Content::Input { mode, multiline, .. } => Some(match (mode.input_type, multiline) {
            (InputType::Password, _) => Role::PasswordInput,
            (InputType::Text, true) => Role::MultilineTextInput,
            (InputType::Text, false) => Role::TextInput,
        }),
        Content::Image { .. } => Some(Role::Image),
        Content::Icon { .. } | Content::Figures(_) | Content::Drawn(_) => s.label.as_ref().map(|_| Role::Image),
        Content::None if b.handlers.click.is_some() => Some(Role::Button),
        Content::None => Some(Role::GenericContainer),
    }
}

/// `b` (window pixels) in a space starting at `space`, on 1/64 px so a
/// box that did not move does not change.
fn local(b: Bounds, space: (f32, f32)) -> Bounds {
    let q = |v: f32| (v * 64.0).round() / 64.0;
    Bounds {
        x: q(b.x - space.0),
        y: q(b.y - space.1),
        width: q(b.width),
        height: q(b.height),
    }
}

fn rect(b: Bounds) -> Rect {
    Rect {
        x0: f64::from(b.x),
        y0: f64::from(b.y),
        x1: f64::from(b.right()),
        y1: f64::from(b.bottom()),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod bound;
