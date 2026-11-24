//! From an element tree to a [`Frame`]: every node's box in window pixels,
//! what it paints, and what it does when acted on.
//!
//! Layout runs on a tree kept across frames ([`LayoutTree`], ADR-0014): each
//! pass patches it from the resolved element tree by identity (place, key
//! and kind), compares each node's layout style and content with what it
//! held, and lays out again only what changed, Taffy's algorithms reading
//! the tree through its low-level traits ([`engine`]). Interactive state
//! from the last frame (hover, press, focus, scroll offsets) is applied
//! first: each node's [`Interaction`](crate::Interaction) flags written
//! before its bindings run, and its scroll offsets. [`Deferred`] subtrees are laid
//! out afterwards, each on a layer of its own (a root of its own), placed
//! against their anchor and kept inside the window.

pub mod boxstyle;
pub mod engine;
mod leaf;

mod binding;
mod changes;
mod field;
mod frame;
mod layers;
mod patch;
mod place;
mod state;
mod write;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

pub use boxstyle::{BoxStyle, Len};
pub use engine::{Asks, Schedule, Stats};
pub use leaf::{ImageMeasure, Leaf};

pub(crate) use changes::{Changed, Changes, ReadAt, Since};
pub(crate) use field::{FieldSettled, caret_reveal, field_reach, input_text};
pub(crate) use frame::{Before, SpanTarget};
pub use frame::{Behaviour, Content, Frame, FrameNode, Layer, Look, Moving, Scroll};
pub(crate) use state::{ByScroller, ScrollId};

/// The deepest element tree `gui` takes: Chromium's HTML parser nests no
/// deeper (`kMaximumHTMLParserDOMTreeDepth`). Building, laying out and
/// painting a node recurse into its children, so the threads that run them
/// get a stack sized for it ([`STACK`]).
pub const MAX_TREE_DEPTH: usize = 512;

/// The stack of the UI thread and of the layout workers: room for a tree
/// [`MAX_TREE_DEPTH`] deep in a debug build (the deepest frame measured, a
/// level of build, layout and paint, is under 40 KB there), twice over. It
/// is address space: pages are committed only as the stack reaches them.
pub const STACK: usize = 64 << 20;

use crate::color::Color;
use crate::edit::TextField;
use crate::element::{Deferred, Element, Handler, InputMode, Positioning};
use crate::focus::FocusId;
use crate::geometry::{Bounds, NodePath, PathSeg};
use crate::icon::Icon;
use crate::motion::Motion;
use crate::style::{ContentStyle, Display, Edges, Effects, Overflow, Position, ResolvedText, Ring, Style};
use crate::text::{InlineSlot, Run, TextSystem};

use binding::{DrawBinding, StyleBinding, TextBinding, Tracker};
use place::Placed;

/// A node of the retained tree.
pub type Node = engine::Node<BoxStyle, Leaf>;

/// The interactive state a frame is built with.
#[derive(Clone, Debug, Default)]
pub struct UiState {
    pub hovered: HashSet<NodePath>,
    pub active: HashSet<NodePath>,
    /// The offsets of the scroll containers without a handle, by identity
    /// ([`ScrollId`]).
    pub(crate) scroll: ByScroller<(f32, f32)>,
    /// How many times the UI set each of those offsets itself (its wheel),
    /// not taking in the compositor's scroll (ADR-0015).
    pub(crate) scroll_generation: ByScroller<u64>,
    pub focused: Option<FocusId>,
    pub focus_visible: bool,
    /// Device pixels per logical pixel: scroll offsets move content by
    /// whole device pixels (ADR-0020). Zero leaves them as they are.
    pub scale: f32,
}

impl UiState {
    /// Whether a scroll was written since the last layout pass.
    fn scrolled(&self) -> bool {
        !self.scroll.changed.borrow().is_empty() || !self.scroll_generation.changed.borrow().is_empty()
    }

    /// Scroll container `id` (without a handle) at `offset`: what another
    /// instance left at its path goes, so a path holds one entry.
    pub(crate) fn set_scroll(&mut self, id: ScrollId, offset: (f32, f32)) {
        self.scroll.insert(id, offset);
    }
}

/// What a node draws, as the frame will hold it.
enum Pending {
    None,
    Text { content: Arc<str>, runs: Rc<[Run]> },
    Icon { icon: Icon, stroke_width: f32 },
    Input(Box<PendingInput>),
    Figures(Vec<crate::figure::Figure>),
    Drawn(crate::chunk::Drawing),
    Image(crate::image::ImageSource),
}

struct PendingInput {
    field: TextField,
    /// What it shows, made as it resolved ([`crate::edit::Shown`]).
    shown: crate::edit::Shown,
    /// The byte range of `shown`'s text an input method is composing.
    preedit: Option<(usize, usize)>,
    placeholder: String,
    placeholder_color: Option<Color>,
    selection_color: Option<Color>,
    multiline: bool,
    mode: InputMode,
}

/// What a retained node needs before the frame is right again (ADR-0013's
/// phases on the retained tree): its own, and as a summary of its subtree,
/// so a pass walks only the paths that lead to work.
mod need {
    /// Resolve its style and content again (a binding or a signal its
    /// patch read changed, its hover or focus changed, its motion moves).
    pub(super) const RESOLVE: u8 = 1;
    /// Write its frame node again (a scroll offset or a signal placing it
    /// read changed, or it was resolved again).
    pub(super) const PLACE: u8 = 2;
    /// Visit it each pass (it joins a list, or holds a deferred subtree):
    /// work a clean pass still does.
    pub(super) const VISIT: u8 = 4;
}

/// Node and phase marks that signal writes left for the next pass.
type Marks = Rc<RefCell<Vec<(u32, u8)>>>;

/// What the tree keeps beside each layout node: who it is and what paint
/// and input read of it.
struct Extra {
    path: NodePath,
    kind: u8,
    name: Option<&'static str>,
    /// What placing and paint read of its resolved style (bindings and
    /// motion applied); the layout half is the node's own style.
    style: PaintStyle,
    /// The content it holds or passes on: what was provided to it with
    /// what it provides itself ([`ContentStyle`]).
    text: ResolvedText,
    /// What it provides itself, as last resolved, and what each of its
    /// spans does: a provider above it changing only what paints (a
    /// hover colour) is applied from these without resolving it.
    provides: ContentStyle,
    span_content: Vec<ContentStyle>,
    /// One per span, made for a span that binds a property.
    span_bindings: Vec<Option<StyleBinding>>,
    /// What each span's bindings gave it at the last resolve (`None` for
    /// a span that binds nothing); empty when no span binds.
    span_styles: Vec<Option<Style>>,
    /// The runs its last resolve made, kept for the buffer
    /// ([`trim_scratch`]).
    runs: Vec<Run>,
    /// Runs it showed, given back by the frame node that shared them once
    /// that node was written anew (at most [`SPARE_RUNS`]): what it shows
    /// next is written into one held by nothing else, so a run's colour
    /// changing back and forth allocates nothing. Paint's chunk shares the
    /// runs it last recorded until it records the node again: with runs
    /// written twice before a paint, one spare is still paint's, and the
    /// next is used.
    spare_runs: Vec<Rc<[Run]>>,
    /// The spans its last resolve had: its scratch is cut to the larger of
    /// these and the spans it has now ([`trim_scratch`]), so a paragraph
    /// that alternates between sizes keeps the room of the larger.
    last_spans: usize,
    /// The boxes of its atomic inlines its frame node held before it was
    /// last written: the next frame node's are written into them.
    spare_boxes: Vec<InlineSlot>,
    /// Its enclosing boxes' decorations reach it
    /// ([`Style::takes_decorations`]).
    takes_decorations: bool,
    opacity: f32,
    invisible: bool,
    /// It or an ancestor is dormant ([`crate::Div::dormant`]).
    dormant: bool,
    /// Its translation is moving this pass ([`Motion::sample`]): paint
    /// applies it as a transform, and its subtree's paint space leaves it out.
    translating: bool,
    /// Only its opacity and transform move, and the compositor moves them
    /// by these (ADR-0015).
    curves: Option<std::sync::Arc<crate::motion::Curves>>,
    content: Pending,
    behaviour: Rc<Behaviour>,
    layer: usize,
    /// Boxed: few nodes bind, and a binding keeps a whole style.
    style_binding: Option<Box<StyleBinding>>,
    text_binding: Option<Box<TextBinding>>,
    draw_binding: Option<Box<DrawBinding>>,
    /// For a root: its layout style before layout's overrides.
    base: Option<Box<BoxStyle>>,
    /// The element it was last patched from (shared, so an element a clean
    /// instance gave back again is known by pointer), what it inherited
    /// then, and what it passed on.
    element: Element,
    inherit: Option<Inherit>,
    mine: Inherit,
    /// Its parent's slot ([`NO_SLOT`] for a root).
    parent: u32,
    /// What it needs ([`need`]) and what its subtree needs.
    own: u8,
    below: u8,
    /// Its place in the frame, as last placed.
    index: u32,
    /// The run of its own layout algorithm it was last placed after.
    ran: u64,
    /// What it passed its children when last placed.
    kids: Option<Placed>,
    /// What patching and placing it read (ADR-0013).
    resolve: Tracker,
    place: Tracker,
    /// Its focus handle's id, as the tree indexes it.
    focus_id: Option<FocusId>,
    /// The text field handle it holds the claim of, by its focus id
    /// ([`LayoutTree::claim_fields`]): the claim is the node's, so a node
    /// patched to show another handle gives its old one up.
    claim: Option<FocusId>,
    /// Motion keeps a state for it, and its height is measured for it.
    motion: bool,
    measures: bool,
    /// Its behaviour was made anew since it was last placed.
    fresh_behaviour: bool,
    /// What its children that took no space (`display: none`) read to
    /// decide so: a change resolves this node again, which builds them.
    hidden: Option<crate::reactive::RenderNode>,
    /// Its [`Interaction`](crate::Interaction) flags, made once and bound
    /// to the handle its element gives (ADR-0022): a render that makes a
    /// new handle makes no signal.
    interaction: crate::interact::NodeInteraction,
}

impl Drop for Extra {
    fn drop(&mut self) {
        if let Some(h) = self.hidden.take() {
            h.dispose();
        }
    }
}

impl Extra {
    /// Its background image or mask may paint `currentColor`.
    fn effects_read_color(&self) -> bool {
        self.style.effects.background_image.is_some() || self.style.effects.mask.is_some()
    }

    /// Its frame node shows what of its content only paints (the colour,
    /// underline, line-through): its text, icon or input draws in it, its
    /// effects read `currentColor`, or its scrollbar thumb is the colour faded.
    fn paints_content(&self) -> bool {
        matches!(
            self.content,
            Pending::Text { .. } | Pending::Icon { .. } | Pending::Input(_)
        ) || self.effects_read_color()
            || ((self.style.overflow_x == Overflow::Scroll || self.style.overflow_y == Overflow::Scroll)
                && !self.style.scrollbar_hidden)
    }

    /// Take a new `inherit` that differs from the one it has only in what
    /// paints (see [`Inherit::paints_as`]), from what it and its spans
    /// provide, without resolving it: only its frame node is written again,
    /// and only when it shows what changed.
    fn repaint_content(&mut self, inherit: &Inherit) {
        let enclosing = if self.takes_decorations {
            inherit.text.clone()
        } else {
            inherit.text.clone().undecorated()
        };
        let text = self.text.clone().painted_as(&enclosing.provide(&self.provides));
        let mut changed = text != self.text;
        if let Pending::Text { runs, .. } = &mut self.content {
            let painted = |r: &Run, own| r.text.clone().painted_as(&Run::style_in(&text, own));
            if runs
                .iter()
                .zip(&self.span_content)
                .any(|(r, own)| painted(r, own) != r.text)
            {
                changed = true;
                for (r, own) in writable(runs, &mut self.spare_runs).iter_mut().zip(&self.span_content) {
                    r.text = painted(r, own);
                }
            }
        }
        self.text = text.clone();
        self.mine.text = text;
        self.inherit = Some(inherit.clone());
        if changed && self.paints_content() {
            self.own |= need::PLACE;
        }
    }
}

/// The most runs a node keeps given back ([`Extra::spare_runs`]): one
/// paint's chunk holds, and one for each time they are written again
/// before the next paint.
const SPARE_RUNS: usize = 3;

/// `runs`, made writable: written in place when held by nothing else,
/// else copied into a spare of the same length held by nothing else (runs
/// the frame node gave back), and copied anew only when there is none.
fn writable<'a>(runs: &'a mut Rc<[Run]>, spares: &mut Vec<Rc<[Run]>>) -> &'a mut [Run] {
    if Rc::get_mut(runs).is_none()
        && let Some(at) = spares
            .iter_mut()
            .position(|s| s.len() == runs.len() && Rc::get_mut(s).is_some())
    {
        let mut s = spares.swap_remove(at);
        if let Some(m) = Rc::get_mut(&mut s) {
            m.clone_from_slice(runs);
        }
        *runs = s;
    }
    Rc::make_mut(runs)
}

/// Keep `runs`, given back by a frame node, among `spares`: in room left,
/// else over one paint still holds (of no use until it lets go).
fn keep_spare(spares: &mut Vec<Rc<[Run]>>, runs: Rc<[Run]>) {
    if spares.len() < SPARE_RUNS {
        spares.push(runs);
    } else if let Some(held) = spares.iter_mut().find(|s| Rc::strong_count(s) > 1) {
        *held = runs;
    }
}

/// A node's scratch buffer after a resolve: let go when it holds nothing,
/// cut to `keep` (what it holds, or more) when large (over 64 KiB) and
/// over twice that. A node resolves only on a change, so it is cut then,
/// not after frames; `keep` is the larger of what it holds now and held
/// at the last resolve, so sizes taken by turns do not cut and grow it.
fn trim_scratch<T>(v: &mut Vec<T>, keep: usize) {
    let keep = keep.max(v.len());
    if v.is_empty() {
        *v = Vec::new();
    } else if v.capacity().saturating_mul(std::mem::size_of::<T>()) > 64 * 1024 && v.capacity() > keep.saturating_mul(2)
    {
        v.shrink_to(keep);
    }
}

/// No slot: the parent of a root.
const NO_SLOT: u32 = u32::MAX;

/// The part of a resolved [`Style`] that placing and paint read.
#[derive(Default)]
struct PaintStyle {
    display: Display,
    position: Position,
    overflow_x: Overflow,
    overflow_y: Overflow,
    scrollbar_hidden: bool,
    background: Option<Color>,
    border_color: Option<Color>,
    border_widths: Edges<f32>,
    shape: crate::shape::Shape,
    shadows: crate::style::Shadows,
    ring: Option<Ring>,
    decoration: crate::shape::Decoration,
    hit_slop: Edges<f32>,
    transform: crate::style::Transform,
    effects: Effects,
}

impl PaintStyle {
    fn of(style: Style) -> Self {
        Self {
            display: style.display,
            position: style.position,
            overflow_x: style.overflow_x,
            overflow_y: style.overflow_y,
            scrollbar_hidden: style.scrollbar_hidden,
            background: style.background,
            border_color: style.border_color,
            border_widths: style.border_widths,
            shape: style.shape,
            shadows: style.shadows,
            ring: style.ring,
            decoration: style.decoration,
            hit_slop: style.hit_slop,
            transform: style.transform,
            effects: style.effects,
        }
    }

    /// It cuts its content to its padding box (see [`Style::clips`]).
    fn clips(&self) -> bool {
        self.overflow_x != Overflow::Visible || self.overflow_y != Overflow::Visible
    }
}

/// What a node passes to its children: the content provided to them
/// ([`ContentStyle`], with the direction, which alone flows to every node),
/// whether they are seen, how they are laid out.
#[derive(Clone, PartialEq)]
struct Inherit {
    text: ResolvedText,
    invisible: bool,
    /// It or an ancestor is dormant ([`crate::Div::dormant`]).
    dormant: bool,
    inert: bool,
    /// It or an ancestor lets the pointer through ([`crate::Div::pass_through_all`]).
    passes: bool,
    /// It or an ancestor is disabled.
    disabled: bool,
    /// It lays its children out in block flow.
    block: bool,
}

impl Inherit {
    /// The two differ in what paints alone (a provider's colour): a node
    /// taking `other` for `self` lays out as it did.
    fn paints_as(&self, other: &Self) -> bool {
        self.invisible == other.invisible
            && self.dormant == other.dormant
            && self.inert == other.inert
            && self.passes == other.passes
            && self.disabled == other.disabled
            && self.block == other.block
            && self.text.clone().shape() == other.text.clone().shape()
    }
}

struct LayerTodo {
    deferred: Rc<Deferred>,
    path: NodePath,
    /// The anchor's path.
    anchor: NodePath,
    /// The anchor is inert, and so is the subtree.
    inert: bool,
    /// The anchor is disabled, and so is the subtree.
    disabled: bool,
}

/// A deferred subtree's root, kept across frames by its path.
struct LayerRoot {
    path: NodePath,
    node: Node,
    memo: Option<LayerMemo>,
    /// When it opened, as [`LayoutTree::opened`] counts: layers paint in
    /// opening order (ADR-0024).
    opened: u64,
}

/// The interactive state the last pass read, to mark what changed since.
#[derive(Default)]
struct Seen {
    focused: Option<FocusId>,
    focus_visible: bool,
    scale: f32,
}

impl Seen {
    fn of(state: &UiState) -> Self {
        Self {
            focused: state.focused,
            focus_visible: state.focus_visible,
            scale: state.scale,
        }
    }

    fn same_as(&self, state: &UiState) -> bool {
        !state.scrolled()
            && self.focused == state.focused
            && self.focus_visible == state.focus_visible
            && self.scale == state.scale
    }
}

/// Nodes each pass of the tree visited (ADR-0013: a pass walks only what
/// leads to work): resolved again, frame nodes written, and visited at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Visits {
    /// Nodes patched: their style and content resolved again.
    pub resolved: u64,
    /// Nodes walked by the patch without resolving them again.
    pub walked: u64,
    /// Of those, nodes that took a provider's new colour (what of the
    /// content only paints) without resolving ([`ContentStyle`]).
    pub repainted: u64,
    /// Frame nodes written again.
    pub placed: u64,
    /// Passes that rebuilt the whole frame (nodes came, went or moved
    /// among their siblings).
    pub rebuilt: u64,
}

impl std::fmt::Display for Visits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "resolved {}  walked {} (repainted {})  placed {}  rebuilt {}",
            self.resolved, self.walked, self.repainted, self.placed, self.rebuilt
        )
    }
}

impl Visits {
    pub fn add(&mut self, o: Self) {
        self.resolved = self.resolved.saturating_add(o.resolved);
        self.walked = self.walked.saturating_add(o.walked);
        self.repainted = self.repainted.saturating_add(o.repainted);
        self.placed = self.placed.saturating_add(o.placed);
        self.rebuilt = self.rebuilt.saturating_add(o.rebuilt);
    }
}

/// How a layer was placed, reused while nothing it was placed from changed.
#[derive(Clone, Copy, PartialEq)]
struct LayerMemo {
    /// Where the layer was asked to go: a new request places it again.
    positioning: Positioning,
    anchor: Option<Bounds>,
    part: Option<Bounds>,
    viewport: (f32, f32),
    origin: (f32, f32),
    placed: Option<crate::placement::Placement>,
}

/// The layout tree kept across frames (ADR-0014): the window's root and each
/// layer's, patched every pass from the resolved element tree.
pub struct LayoutTree {
    root: Option<Node>,
    layers: Vec<LayerRoot>,
    /// Boxed: a pass takes each out and puts it back.
    extras: Vec<Option<Box<Extra>>>,
    free: Vec<u32>,
    schedule: Schedule,
    stats: Stats,
    visits: Visits,
    /// The focused node's path, as the last pass found it: a scroll
    /// container tells its handle which child holds focus (a virtual list
    /// keeps that row built, ADR-0021).
    focused: Option<NodePath>,
    /// The layer the focused node is in: focus within a node is focus at
    /// or below it in the same layer, as the frame's parents link them.
    focused_layer: usize,
    /// The root's path, made once: a pass allocates nothing for it.
    root_path: NodePath,
    /// The frame as laid out, kept and written in place (ADR-0013).
    frame: Frame,
    /// What signal writes marked since the last pass.
    marks: Marks,
    /// Runs a phase of a node that has no tracker yet.
    probe: crate::reactive::RenderNode,
    /// Nodes by path and by focus id, to mark what interactive state names.
    by_path: foldhash::HashMap<NodePath, u32>,
    by_focus: foldhash::HashMap<FocusId, u32>,
    /// The field showing each text field handle, by its focus: one at a
    /// time ([`LayoutTree::claim_fields`]).
    by_field: foldhash::HashMap<FocusId, u32>,
    /// Fields resolved since the last claim: whose handle each shows.
    mounting: Vec<(FocusId, u32)>,
    seen: Option<Seen>,
    /// Scroll writes the passes placed (tests: a pass costs what changed).
    #[cfg(test)]
    scroll_marked: u64,
    /// Paths of nodes released this pass that motion kept a state for.
    released: Vec<NodePath>,
    /// This pass added, removed or reordered a node: the frame is rebuilt.
    structure: bool,
    /// The layers' paths, as the last pass placed them, each with when it
    /// opened: what a pass that panics carries to the next.
    layer_paths: Vec<(NodePath, u64)>,
    /// The member nodes ([`crate::Members`]) this pass patched or walked,
    /// in tree order: each joins its lists once placed, when shown.
    joining: Vec<u32>,
    /// The measure hooks ([`crate::Div::on_measured`]) of the nodes placed
    /// since the lists were last filled, with their boxes: run after them.
    measured_due: RefCell<Vec<(Handler<Bounds>, Bounds)>>,
    /// What placing text fields says of their scroll state, for the input
    /// state to take ([`LayoutTree::swap_fields`]).
    fields: RefCell<Vec<FieldSettled>>,
    /// Its lists ([`crate::Members`]), filled each pass.
    lists: Rc<crate::members::Lists>,
    /// A tree laid out once beside the window's own (a cold check): it
    /// writes nothing of the window's (its lists, focus, scroll handles,
    /// measure hooks, placement reports), so laying it out changes nothing.
    detached: bool,
    /// How many layers have opened: the next one's place in opening order.
    opened: u64,
    /// The opening order of a tree laid out beside the window's (a cold
    /// check, [`layout_carrying`]): the window's layers, where they opened.
    /// Which layer opened first is retained state, as a scroll offset is,
    /// so a cold build paints the layers in the order the window does.
    carried: HashMap<NodePath, u64>,
    /// The window size the last pass laid out at.
    size: (f32, f32),
    /// The motion generation the last pass sampled at.
    motion_seen: u64,
}

impl Default for LayoutTree {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LayoutTree {
    fn drop(&mut self) {
        // Bindings' trackers, then the probe.
        self.extras.clear();
        self.probe.dispose();
    }
}

fn set_style(node: &mut Node, style: BoxStyle) {
    if node.style != style {
        node.style = style;
        node.invalidate();
    }
}

impl LayoutTree {
    #[must_use]
    pub fn new() -> Self {
        Self::with_schedule(Schedule::from_env())
    }

    /// A tree laid out as `schedule` says (the determinism checks).
    #[must_use]
    pub fn with_schedule(schedule: Schedule) -> Self {
        if schedule.parallel {
            leaf::warm_pool();
        }
        Self {
            root: None,
            layers: Vec::new(),
            extras: Vec::new(),
            free: Vec::new(),
            schedule,
            stats: Stats::default(),
            visits: Visits::default(),
            focused: None,
            focused_layer: 0,
            root_path: NodePath::new([PathSeg::Index(0)].into()),
            frame: Frame::default(),
            marks: Rc::default(),
            probe: crate::reactive::detached(|| crate::reactive::RenderNode::new(|| {})),
            by_path: foldhash::HashMap::default(),
            by_focus: foldhash::HashMap::default(),
            by_field: foldhash::HashMap::default(),
            mounting: Vec::new(),
            seen: None,
            #[cfg(test)]
            scroll_marked: 0,
            released: Vec::new(),
            structure: true,
            layer_paths: Vec::new(),
            joining: Vec::new(),
            measured_due: RefCell::new(Vec::new()),
            fields: RefCell::default(),
            lists: Rc::default(),
            detached: false,
            opened: 0,
            carried: HashMap::new(),
            size: (-1.0, -1.0),
            motion_seen: 0,
        }
    }

    /// What the last pass did.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The nodes the last pass visited.
    #[must_use]
    pub fn visits(&self) -> Visits {
        self.visits
    }

    /// The frame as last laid out.
    #[must_use]
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// The frame as last laid out, for a test to make stale.
    #[cfg(test)]
    pub(crate) fn frame_mut(&mut self) -> &mut Frame {
        &mut self.frame
    }

    /// The focus handles it finds nodes by.
    #[cfg(test)]
    pub(crate) fn focus_entries(&self) -> usize {
        self.by_focus.len()
    }

    /// The index in the frame of the node holding focus `id`.
    pub(crate) fn focus_index(&self, id: FocusId) -> Option<usize> {
        // A node not shown holds no focus ([`Frame::shown`]).
        let holds = |i: usize| {
            self.frame.shown(i)
                && self
                    .frame
                    .behaviour(i)
                    .and_then(|b| b.focus)
                    .is_some_and(|f| f.id() == id)
        };
        let placed = self
            .by_focus
            .get(&id)
            .and_then(|&u| self.extra(u))
            .and_then(|e| usize::try_from(e.index).ok())
            .filter(|&i| holds(i));
        // A node not placed in this frame yet (or no longer): the frame's own word.
        placed.or_else(|| (0..self.frame.nodes.len()).find(|&i| holds(i)))
    }

    /// Whether a node at `path` is in the tree.
    #[must_use]
    pub fn holds(&self, path: &NodePath) -> bool {
        self.by_path.contains_key(path)
    }

    /// The frame's nodes from an index on as they were before passes since
    /// the last call rebuilt them.
    pub(crate) fn take_before(&mut self) -> Option<Before> {
        self.frame.before.take()
    }

    /// What was written in place since the frame report last read, into
    /// `into` ([`Changes::unread`]): false when it had not read this frame yet.
    pub(crate) fn unread_changes(&mut self, into: &mut Vec<Changed>) -> bool {
        self.frame.changes.unread(into)
    }

    /// What layout read outside any node changed: the next pass writes the
    /// whole frame and places every layer again.
    pub fn invalidate_all(&mut self) {
        self.structure = true;
        for l in &mut self.layers {
            l.memo = None;
        }
    }

    /// The frame, taken: the next pass builds it anew.
    pub fn take_frame(&mut self) -> Frame {
        self.structure = true;
        std::mem::take(&mut self.frame)
    }

    /// Nodes the tree holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.extras.len().saturating_sub(self.free.len())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether a pass now would do anything: something marked a node, or
    /// the interactive state, the size or what moves changed since the last.
    #[must_use]
    pub fn is_clean(&self, root: &Element, width: f32, height: f32, state: &UiState) -> bool {
        let root_same = self
            .root
            .as_ref()
            .and_then(|n| self.extra(n.user))
            .is_some_and(|e| e.element.ptr_eq(root) && (e.own | e.below) & (need::RESOLVE | need::PLACE) == 0);
        root_same
            && self.layers.is_empty()
            && self.marks.borrow().is_empty()
            && self.size == (width, height)
            && self.seen.as_ref().is_some_and(|s| s.same_as(state))
    }

    fn extra(&self, u: u32) -> Option<&Extra> {
        self.extras.get(usize::try_from(u).ok()?)?.as_deref()
    }

    fn extra_mut(&mut self, u: u32) -> Option<&mut Extra> {
        self.extras.get_mut(usize::try_from(u).ok()?)?.as_deref_mut()
    }

    fn alloc(&mut self, extra: Box<Extra>) -> u32 {
        if let Some(u) = self.free.pop()
            && let Some(slot) = usize::try_from(u).ok().and_then(|i| self.extras.get_mut(i))
        {
            *slot = Some(extra);
            return u;
        }
        self.extras.push(Some(extra));
        u32::try_from(self.extras.len().saturating_sub(1)).unwrap_or(u32::MAX)
    }

    fn release(&mut self, node: Node) {
        self.structure = true;
        if let Some(slot) = usize::try_from(node.user).ok().and_then(|i| self.extras.get_mut(i))
            && let Some(ex) = slot.take()
        {
            if self.by_path.get(&ex.path) == Some(&node.user) {
                self.by_path.remove(&ex.path);
            }
            if let Some(f) = ex.focus_id
                && self.by_focus.get(&f) == Some(&node.user)
            {
                self.by_focus.remove(&f);
            }
            if let Some(f) = ex.claim
                && self.by_field.get(&f) == Some(&node.user)
            {
                self.by_field.remove(&f);
            }
            if ex.motion {
                self.released.push(ex.path.clone());
            }
            self.free.push(node.user);
        }
        for c in node.children {
            self.release(c);
        }
    }

    fn take_extra(&mut self, u: u32) -> Option<Box<Extra>> {
        self.extras.get_mut(usize::try_from(u).ok()?)?.take()
    }

    fn put_extra(&mut self, u: u32, extra: Box<Extra>) {
        if let Some(slot) = usize::try_from(u).ok().and_then(|i| self.extras.get_mut(i)) {
            *slot = Some(extra);
        }
    }

    /// The component instance whose output `ex` is: the nearest marked
    /// root above it, itself included, else its own owner, as
    /// [`crate::interact`] names a node ([`ScrollId`]). Its ancestors are
    /// in the tree while it is placed.
    fn owner_of(&self, ex: &Extra) -> Option<crate::reactive::OwnerId> {
        let mut at = Some(ex);
        while let Some(e) = at {
            if let Some(i) = e.behaviour.handlers.instance {
                return Some(i);
            }
            at = self.extra(e.parent);
        }
        ex.behaviour.handlers.owner
    }

    /// Mark `need` on node `u`, and as a need below on its ancestors.
    fn mark(&mut self, u: u32, need: u8) {
        let Some(ex) = self.extra_mut(u) else { return };
        ex.own |= need;
        let mut p = ex.parent;
        while p != NO_SLOT {
            let Some(e) = self.extra_mut(p) else { break };
            if e.below & need == need {
                break;
            }
            e.below |= need;
            p = e.parent;
        }
    }

    /// Mark `need` on the node at `path`, or on its nearest ancestor in the
    /// tree when it is not.
    fn mark_path(&mut self, path: &NodePath, need: u8) {
        if let Some(&u) = self.by_path.get(path) {
            self.mark(u, need);
            return;
        }
        let mut len = path.0.len();
        while len > 1 {
            len = len.saturating_sub(1);
            let up = NodePath::new(path.0.get(..len).map(Into::into).unwrap_or_default());
            if let Some(&u) = self.by_path.get(&up) {
                self.mark(u, need);
                return;
            }
        }
    }

    /// The scroll containers holding node `u` (focus within them moved).
    fn mark_scrollers_above(&mut self, u: u32) {
        let mut p = self.extra(u).map_or(NO_SLOT, |e| e.parent);
        while p != NO_SLOT {
            let Some(e) = self.extra(p) else { break };
            let (scrolls, up) = (e.behaviour.scroll.is_some(), e.parent);
            if scrolls {
                self.mark(p, need::PLACE);
            }
            p = up;
        }
    }

    /// Take in what signal writes marked and what of the interactive state
    /// changed since the last pass, as marks.
    fn take_marks(&mut self, state: &UiState) {
        let marks = std::mem::take(&mut *self.marks.borrow_mut());
        for &(u, need) in &marks {
            self.mark(u, need);
        }
        // Give the buffer back, so marking allocates nothing once it has grown.
        if let Ok(mut m) = self.marks.try_borrow_mut()
            && m.is_empty()
        {
            let mut marks = marks;
            marks.clear();
            *m = marks;
        }
        // The scroll writes since the last pass, taken: this pass places
        // them (a detached tree, the cold check's, leaves them to the
        // window's own).
        let written = if self.detached {
            HashSet::new()
        } else {
            let mut w = state.scroll.take_written();
            w.extend(state.scroll_generation.take_written());
            w
        };
        let Some(mut seen) = self.seen.take() else {
            self.seen = Some(Seen::of(state));
            return;
        };
        // Hover, press and focus reach the nodes' styles through their
        // `Interaction`s' signals, which hit testing and focus write. What
        // focus moving changes here is placed: an input's frame node says
        // whether it holds focus (its caret), and the scroll containers
        // above the nodes it left and came to keep it in view.
        if seen.focused != state.focused || seen.focus_visible != state.focus_visible {
            for f in [seen.focused, state.focused].into_iter().flatten() {
                if let Some(&u) = self.by_focus.get(&f) {
                    self.mark(u, need::PLACE);
                    self.mark_scrollers_above(u);
                }
            }
            seen.focused = state.focused;
            seen.focus_visible = state.focus_visible;
        }
        // The scroll containers written since the last pass, and only
        // those ([`ByScroller`]'s log).
        for p in &written {
            #[cfg(test)]
            {
                self.scroll_marked = self.scroll_marked.saturating_add(1);
            }
            self.mark_path(p, need::PLACE);
        }
        if seen.scale != state.scale {
            // Scroll offsets and moving translations round to the new pixels.
            self.structure = true;
            seen.scale = state.scale;
        }
        self.seen = Some(seen);
    }
}

/// What one patch pass reads and fills.
struct Pass<'s> {
    state: &'s UiState,
    motion: &'s mut Motion,
    todo: &'s mut std::collections::VecDeque<LayerTodo>,
}

/// Lay `root` out on a tree of its own, cold: what the retained tree's
/// layout must equal (the cold check, tests).
pub fn layout(
    root: &Element,
    width: f32,
    height: f32,
    state: &UiState,
    motion: &mut Motion,
    text: &mut TextSystem,
) -> Frame {
    layout_carrying(root, width, height, state, motion, text, None)
}

/// [`layout`], the layers that `window`'s tree holds open painting in the
/// order they opened there (ADR-0024): the cold check's build, which
/// carries the window's retained state and nothing of its history else.
pub(crate) fn layout_carrying(
    root: &Element,
    width: f32,
    height: f32,
    state: &UiState,
    motion: &mut Motion,
    text: &mut TextSystem,
    window: Option<&LayoutTree>,
) -> Frame {
    let mut tree = LayoutTree::new();
    tree.detached = true;
    if let Some(w) = window {
        tree.carried = w.layers.iter().map(|l| (l.path.clone(), l.opened)).collect();
        tree.opened = w.opened;
    }
    tree.layout(root, width, height, state, motion, text);
    tree.take_frame()
}

/// The wrap width a text node is drawn with, given its area.
#[must_use]
pub fn text_width(text: &ResolvedText, area: &Bounds) -> Option<f32> {
    leaf::wrap_width(text, area.width)
}

#[cfg(test)]
mod phases;

#[cfg(test)]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    reason = "tests: seeds, counts and offsets are small"
)]
mod tests;
