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

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use taffy::{AvailableSpace, Layout};

pub use boxstyle::{BoxStyle, Len};
pub use engine::{Schedule, Stats};
pub use leaf::{ImageMeasure, Leaf};

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
use crate::element::{Common, Deferred, Element, ElementKey, Handler, Handlers, InputMode, Positioning, Semantics};
use crate::events::ClickEvent;
use crate::focus::{FocusHandle, FocusId};
use crate::geometry::{Bounds, NodePath, PathSeg};
use crate::icon::Icon;
use crate::motion::Motion;
use crate::placement::{EDGE_MARGIN, fitted_height, over, placed, position};
use crate::scroll::{ScrollChild, ScrollHandle, ScrollLayout};
use crate::style::{
    ContentStyle, Corners, Cursor, Direction, Display, Edges, Effects, Length, Overflow, Painted, Position,
    ResolvedText, Ring, Style, TextAlign,
};
use crate::text::{InlineSlot, Paragraph, Run, TextSystem};

/// A node of the retained tree.
pub type Node = engine::Node<BoxStyle, Leaf>;

/// How a node's box is painted.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub background: Option<Color>,
    pub border: Edges<f32>,
    pub border_color: Option<Color>,
    /// Its shape as asked; paint resolves its radii against the box it
    /// paints (`Shape::used`).
    pub shape: crate::shape::Shape,
    pub shadows: crate::style::Shadows,
    pub ring: Option<Ring>,
    pub decoration: crate::shape::Decoration,
    /// How far past its border box it takes the pointer.
    pub hit_slop: Edges<f32>,
    /// Its own opacity (CSS `opacity`): below 1 it and its children paint
    /// aside as one group, composited at it, so an ancestor's is not theirs.
    pub opacity: f32,
    /// It paints nothing and takes no pointer: it or an ancestor is
    /// `invisible` or dormant, or an inline its paragraph's clamp hides.
    pub invisible: bool,
    /// Of those, dormancy alone ([`crate::Div::dormant`]) hides it: it is
    /// still a member of its lists ([`Frame::navigable`]).
    pub dormant: bool,
    pub effects: Effects,
    /// The scale and turn its painting takes, with its children's.
    pub transform: Painted,
    /// The content provided to the node that its effects resolve against
    /// (`currentColor`, the `ch` unit); without effects, its shape alone
    /// (no colour).
    pub text: ResolvedText,
}

/// What a node draws inside its box.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    None,
    Text {
        /// Shared with the layout tree (and the leaf the layout pool
        /// measures), as `runs` is, so writing the node anew copies neither.
        content: Arc<str>,
        text: ResolvedText,
        /// Parts of `content` styled on their own; empty when it has one style.
        runs: Rc<[Run]>,
        /// The boxes of its atomic inlines ([`crate::Text::child`]) in its
        /// lines, each on a [`crate::text::OBJECT`] of `content`: its
        /// children, which paint themselves. [`crate::text::readable`]
        /// gives `content` without them.
        boxes: Vec<InlineSlot>,
    },
    Icon {
        icon: Icon,
        color: Color,
        stroke_width: f32,
    },
    Figures(Vec<crate::figure::Figure>),
    /// A picture laid into the content box when painted.
    Drawn(crate::chunk::Drawing),
    Image {
        source: crate::image::ImageSource,
        radius: Corners,
    },
    Input {
        /// What the input shows: its value with any composition in it,
        /// its text shared with its leaf ([`crate::edit::Shown`]).
        edit: crate::edit::Shown,
        /// The byte range of `edit.text` an input method is composing.
        preedit: Option<(usize, usize)>,
        placeholder: String,
        placeholder_color: Color,
        selection_color: Color,
        text: ResolvedText,
        focused: bool,
        multiline: bool,
        mode: InputMode,
    },
}

/// Scroll state of a scroll container.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scroll {
    pub offset: (f32, f32),
    /// What the offset moves its children by, in whole device pixels
    /// (ADR-0020): their paint space is the container's, this applied as a
    /// transform (ADR-0016), so scrolling records nothing again.
    pub applied: (f32, f32),
    /// The largest offset on each axis.
    pub max: (f32, f32),
    /// Colour of the scrollbar thumb: the container's text colour, faded.
    pub thumb: Color,
    /// How many times the offset was set other than by the compositor's
    /// scroll: what it scrolled before is then dropped (ADR-0015).
    pub generation: u64,
}

/// A node whose opacity and transform the compositor moves (ADR-0015):
/// what it samples, from what transform, and what places it (the offset of
/// the space above it, the side its layer opened on).
#[derive(Clone, Debug)]
pub struct Moving {
    pub curves: std::sync::Arc<crate::motion::Curves>,
    pub transform: crate::style::Transform,
    pub above: (f32, f32),
    pub(crate) opened: Option<crate::placement::Placement>,
}

impl PartialEq for Moving {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.curves, &other.curves)
            && self.transform == other.transform
            && self.above == other.above
            && self.opened == other.opened
    }
}

/// One laid-out node.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameNode {
    pub path: NodePath,
    pub name: Option<&'static str>,
    /// The border box in window pixels: `space` moved by `offset`.
    pub bounds: Bounds,
    /// The box inside padding and border, where text is laid out.
    pub content_box: Bounds,
    /// The border box in the node's paint space (ADR-0016): where its paint
    /// chunk records it, without the scroll offsets and moving translations
    /// above it, which are transforms of the property tree.
    pub space: Bounds,
    /// The content box in the paint space.
    pub space_content: Bounds,
    /// What takes the paint space to the window: `bounds` is `space` moved
    /// by it.
    pub offset: (f32, f32),
    /// Its translation is moving (a transition or animation): it is in
    /// `offset`, not in `space`.
    pub translating: bool,
    /// The compositor moves its opacity and transform (ADR-0015).
    pub moving: Option<Moving>,
    /// What this node may paint into, when an ancestor clips.
    pub clip: Option<Bounds>,
    /// The corner radii of that clip (the clipping ancestor's inner corners).
    pub clip_radius: Corners,
    /// Those corners' curve (the clipping ancestor's [`crate::Shape::curve`]).
    pub clip_curve: f32,
    /// The node whose padding box set `clip`: the clip property's owner.
    pub clipper: Option<usize>,
    /// 0 for the main tree; each deferred subtree is one higher than the last.
    pub layer: usize,
    /// The parent within the same layer.
    pub parent: Option<usize>,
    pub look: Look,
    pub content: Content,
    pub scroll: Option<Scroll>,
}

/// What a node does, kept beside [`FrameNode`] because closures cannot be compared.
#[derive(Clone, Default)]
pub struct Behaviour {
    pub(crate) handlers: Handlers,
    pub(crate) focus: Option<FocusHandle>,
    /// It or an ancestor (or the anchor of its deferred subtree) is
    /// disabled ([`crate::Div::disabled`]), as `inert` is inherited and as
    /// a disabled `<fieldset>` disables what it holds: no focus, keys or
    /// activation, read as disabled; its own handlers do not run.
    pub(crate) disabled: bool,
    pub(crate) pass_through: bool,
    pub(crate) cursor: Option<Cursor>,
    pub(crate) field: Option<TextField>,
    /// The spans of a paragraph that act on the pointer (links).
    pub(crate) spans: Vec<SpanTarget>,
    /// Holds this scroll container's offset.
    pub(crate) scroll: Option<ScrollHandle>,
    /// What the render built its children from, for the handle's hook.
    pub(crate) scroll_built: Option<Rc<dyn std::any::Any>>,
    pub(crate) semantics: Semantics,
    /// Where hit testing and focus write what they do to the node.
    pub(crate) interaction: Option<crate::interact::NodeFlags>,
    /// It or an ancestor (or the anchor of its deferred subtree) is inert
    /// ([`crate::Div::inert`]): no hit (`pass_through`), no focus, no layer,
    /// no screen reader.
    pub(crate) inert: bool,
    /// Whether it is a Tab stop while it does not hold focus
    /// ([`crate::Div::tab_stop`]), read when Tab looks.
    pub(crate) tab_stop: crate::element::TabStop,
    /// Tab visits its children last first
    /// ([`crate::Div::tab_children_reversed`]).
    pub(crate) tab_reversed: bool,
}

/// A span of a text node that takes clicks or sets the cursor.
#[derive(Clone)]
pub(crate) struct SpanTarget {
    /// Which of its paragraph's spans it is.
    pub(crate) index: usize,
    pub(crate) range: std::ops::Range<usize>,
    pub(crate) click: Option<Handler<ClickEvent>>,
    pub(crate) cursor: Option<Cursor>,
    /// Where its text lies in the window, one box per line it takes.
    pub(crate) boxes: Vec<Bounds>,
}

impl Behaviour {
    /// Whether a click may run on it or pass through it to its ancestors:
    /// not when it is disabled or unavailable ([`crate::Div::unavailable`]).
    pub(crate) fn takes_clicks(&self) -> bool {
        !self.disabled && !self.semantics.unavailable
    }

    /// The span at window point `(x, y)`.
    pub(crate) fn span_at(&self, x: f32, y: f32) -> Option<&SpanTarget> {
        self.spans.iter().find(|s| s.boxes.iter().any(|b| b.contains(x, y)))
    }
}

/// A layer: its root node and the box it was placed against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub root: usize,
    pub anchor: Option<Bounds>,
    /// The keyed node inside its owner its Tab stops follow
    /// ([`crate::Deferred::tab_after`]).
    pub tab_after: Option<ElementKey>,
}

/// A whole laid-out frame, in paint order: the main tree depth-first, then
/// each layer. Kept across passes by the layout tree, which writes in place
/// the nodes that changed and says which in [`Frame::changes`] (ADR-0013).
#[derive(Clone, Default)]
pub struct Frame {
    pub nodes: Vec<FrameNode>,
    /// Shared with the layout tree, which keeps each node's.
    pub(crate) behaviour: Vec<Rc<Behaviour>>,
    pub layers: Vec<Layer>,
    pub viewport: Bounds,
    /// What changed since each reader last looked.
    pub(crate) changes: Changes,
    /// The nodes from an index on as they were when the frame was last
    /// painted, kept once a pass rebuilt them (for the frame report's diff).
    pub(crate) before: Option<Before>,
    /// The nodes that hold a scroll handle.
    pub(crate) handles: Vec<usize>,
}

/// A frame's nodes from `at` on as they were when it was last painted,
/// kept once a pass rebuilt them: what the frame report compares them with.
#[derive(Clone, Debug, Default)]
pub(crate) struct Before {
    pub(crate) at: usize,
    pub(crate) nodes: Vec<FrameNode>,
    /// Of `nodes`, those a pass repainted in place before one rebuilt them
    /// (their look then is gone, only that it changed is kept).
    pub(crate) repainted: Vec<usize>,
}

impl Before {
    /// How the nodes from `at` on in `frame` differ from these, in tree
    /// order like [`crate::inspect::diff`]'s: a node repainted in place
    /// before the rebuild shows as repainted even when it now looks the
    /// same. Linear: one pass over the frame, merged with the diff's.
    pub(crate) fn diff(&self, frame: &Frame) -> crate::inspect::LayoutDiff {
        use crate::inspect::{Change, NodeChange, What};
        let now = frame.nodes.get(self.at..).unwrap_or_default();
        let mut d = crate::inspect::diff(&self.nodes, now);
        if self.repainted.is_empty() {
            return d;
        }
        let marked: HashMap<&NodePath, Bounds> = self
            .repainted
            .iter()
            .filter_map(|&k| self.nodes.get(k))
            .map(|n| (&n.path, n.bounds))
            .collect();
        // The diff's nodes that stayed or came are in `now`'s order, those
        // that went after them: walk `now`, taking each diffed node in turn.
        let found = std::mem::take(&mut d.changes);
        let mut found = found.into_iter().peekable();
        let mut changes = Vec::with_capacity(found.len().saturating_add(marked.len()));
        for n in now {
            let next = found.next_if(|c| c.path == n.path && !matches!(c.change, Change::Removed { .. }));
            match next {
                Some(mut c) => {
                    if let Change::Updated { repainted, .. } = &mut c.change {
                        *repainted |= marked.contains_key(&n.path);
                    }
                    changes.push(c);
                }
                None => {
                    if let Some(&before) = marked.get(&n.path) {
                        changes.push(NodeChange {
                            path: n.path.clone(),
                            what: What::of(n),
                            change: Change::Updated {
                                before,
                                after: n.bounds,
                                repainted: true,
                            },
                        });
                    }
                }
            }
        }
        changes.extend(found);
        d.changes = changes;
        d
    }
}

/// One frame node a pass wrote in place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Changed {
    pub(crate) index: u32,
    /// Its box before.
    pub(crate) before: Bounds,
    /// Its look or content changed.
    pub(crate) repainted: bool,
}

/// What the passes changed in a [`Frame`], for its readers (paint,
/// accessibility, the frame report's diff), each of which keeps where it read up to
/// ([`ReadAt`]): the nodes written in place, in order, since the frame was
/// last rebuilt (nodes came, went or moved, and every index may be another).
#[derive(Clone, Debug, Default)]
pub(crate) struct Changes {
    /// Which rebuild of which frame it describes, unique in the thread: a
    /// new one at each rebuild. 0 for a frame never built.
    epoch: u64,
    pub(crate) log: Vec<Changed>,
    /// How many entries were logged before `log`'s first since the rebuild,
    /// and dropped: a reader that had not read them reads everything.
    dropped: usize,
    /// Where the reader that must see every change (the frame report's
    /// diff, read every frame) read up
    /// to, since the rebuild: `None` until it first reads.
    unread_from: Option<usize>,
    /// The entries from `unread_from` on that a trim or a rebuild dropped
    /// before that reader read them, one per node (its box before the
    /// first, whether any repainted it), so it never falls behind. Each
    /// with its place when last sorted, which keeps a node's first first.
    unread: Vec<(usize, Changed)>,
}

/// Where a reader of a frame's [`Changes`] read up to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReadAt {
    epoch: u64,
    /// How many entries since the rebuild it read.
    at: usize,
    /// It has read a frame.
    read: bool,
}

/// What a reader of a frame's [`Changes`] reads since it last read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Since<'a> {
    /// The nodes written in place since, in order.
    Logged(&'a [Changed]),
    /// The frame was rebuilt since (or never read): what the reader held of
    /// it by index means nothing, and it reads the whole frame.
    Rebuilt,
    /// The same frame, but a trim dropped entries the reader had not read:
    /// it reads the whole frame.
    FellBehind,
}

impl Changes {
    /// The frame was rebuilt from node `start` on: what readers held of it
    /// by index means nothing. The nodes before `start` are as they were, so
    /// what was written into them is kept for the reader that must see
    /// every change; what was written into the others is in the nodes kept
    /// as they were ([`Frame::before`], undone there by the rebuild).
    pub(crate) fn rebuilt(&mut self, start: usize) {
        crate::engine::engine_local! {
            [Fork] static EPOCHS: Cell<u64> = const { Cell::new(0) };
        }
        self.epoch = EPOCHS.with(|e| {
            let next = e.get().wrapping_add(1).max(1);
            e.set(next);
            next
        });
        let end = self.end();
        self.keep_unread(end, |c| usize::try_from(c.index).is_ok_and(|i| i < start));
        self.log.clear();
        self.dropped = 0;
        if self.unread_from.is_some() {
            self.unread_from = Some(0);
        }
    }

    /// How many entries were logged since the rebuild, dropped or not: a
    /// count that only grows within an epoch, so `(epoch, end)` names one
    /// state of the frame (the log's length alone repeats after a trim).
    pub(crate) fn end(&self) -> usize {
        self.dropped.saturating_add(self.log.len())
    }

    /// Keep the log no longer than the frame it describes, twice over: past
    /// that its older half is dropped. A reader that had not read what was
    /// dropped reads the whole frame ([`Since::FellBehind`]): one that reads
    /// each frame does only if one frame's passes logged that much (paint
    /// then records the frame anew). It holds room for that many at once,
    /// so logging a change (a hover's repaint) allocates nothing while the
    /// frame keeps its size.
    pub(crate) fn trim(&mut self, nodes: usize) {
        let most = nodes.saturating_mul(2).saturating_add(64);
        if self.log.len() > most {
            let cut = self.log.len().saturating_sub(most / 2);
            let dropped = self.dropped.saturating_add(cut);
            self.keep_unread(dropped, |_| true);
            self.log.drain(..cut);
            self.dropped = dropped;
        }
        if self.log.capacity() < most {
            self.log
                .reserve_exact(most.saturating_add(1).saturating_sub(self.log.len()));
        }
    }

    /// Fold into `unread` the entries before `upto` (counted since the
    /// rebuild) that the reader of every change had not read and `keep`
    /// holds, one per node.
    fn keep_unread(&mut self, upto: usize, keep: impl Fn(&Changed) -> bool) {
        let Some(from) = self.unread_from else {
            return;
        };
        let lo = from.saturating_sub(self.dropped).min(self.log.len());
        let hi = upto.saturating_sub(self.dropped).min(self.log.len());
        let Some(fresh) = self.log.get(lo..hi).filter(|f| !f.is_empty()) else {
            self.unread.retain(|(_, c)| keep(c));
            return;
        };
        self.unread.extend(fresh.iter().map(|&c| (0, c)));
        self.unread.retain(|(_, c)| keep(c));
        // By node, each node's in the order written (its place breaks the
        // tie, so a sort that allocates nothing keeps that order): its first
        // holds its box before.
        for (k, (at, _)) in self.unread.iter_mut().enumerate() {
            *at = k;
        }
        self.unread.sort_unstable_by_key(|&(at, c)| (c.index, at));
        self.unread.dedup_by(|(_, later), (_, first)| {
            let same = later.index == first.index;
            if same {
                first.repainted |= later.repainted;
            }
            same
        });
    }

    /// The writes into nodes in `range` the reader of every change has not
    /// read, latest first (a node's first write comes last), each with its
    /// node's offset from the range's start. Nothing before it first read.
    fn unread_back(&self, range: std::ops::Range<usize>, mut f: impl FnMut(usize, &Changed)) {
        let Some(from) = self.unread_from else {
            return;
        };
        let from = from.saturating_sub(self.dropped).min(self.log.len());
        let log = self.log.get(from..).unwrap_or_default();
        for c in log.iter().rev().chain(self.unread.iter().rev().map(|(_, c)| c)) {
            if let Some(i) = usize::try_from(c.index).ok().filter(|i| range.contains(i)) {
                f(i.saturating_sub(range.start), c);
            }
        }
    }

    /// What changed since `cursor`, moving it to now.
    pub(crate) fn since(&self, cursor: &mut ReadAt) -> Since<'_> {
        let end = self.end();
        let same = cursor.read && cursor.epoch == self.epoch && cursor.at <= end;
        let from = cursor.at.checked_sub(self.dropped);
        *cursor = ReadAt {
            epoch: self.epoch,
            at: end,
            read: true,
        };
        if !same {
            return Since::Rebuilt;
        }
        match from.and_then(|from| self.log.get(from..)) {
            Some(log) => Since::Logged(log),
            None => Since::FellBehind,
        }
    }

    /// For the reader that must see every change (the frame report's diff):
    /// what was
    /// written in place since it last read, into `into`, in order (what a
    /// trim or a rebuild dropped first, folded by node). False the first
    /// time it reads this frame, which has no earlier state to differ
    /// from. A rebuild since keeps only what was written into the nodes
    /// before those it rebuilt (the rebuilt ones it compares as they were).
    pub(crate) fn unread(&mut self, into: &mut Vec<Changed>) -> bool {
        into.clear();
        let end = self.end();
        let Some(from) = self.unread_from.replace(end) else {
            self.unread.clear();
            return false;
        };
        into.extend(self.unread.drain(..).map(|(_, c)| c));
        let from = from.saturating_sub(self.dropped).min(self.log.len());
        into.extend(self.log.get(from..).unwrap_or_default().iter().copied());
        true
    }
}

impl Frame {
    pub(crate) fn behaviour(&self, i: usize) -> Option<&Behaviour> {
        self.behaviour.get(i).map(|b| &**b)
    }

    /// Whether node `i` is shown, the one rule for whether a node can act:
    /// it and every ancestor are rendered (not `invisible`, not an atomic
    /// inline its paragraph's clamp or cut hides; both reach the subtree
    /// as `look.invisible`) and not inert. A node not shown takes no focus,
    /// key, activation or screen reader's action, and is no member of a
    /// list ([`crate::Members`]): it behaves as one not laid out.
    #[must_use]
    pub fn shown(&self, i: usize) -> bool {
        self.nodes
            .get(i)
            .zip(self.behaviour(i))
            .is_some_and(|(n, b)| !n.look.invisible && !b.inert)
    }

    /// Whether node `i` is a member of its lists ([`crate::Members`]):
    /// shown, or hidden by its dormancy alone ([`crate::Div::dormant`]),
    /// which its list's owner walks while it sleeps.
    #[must_use]
    pub fn navigable(&self, i: usize) -> bool {
        self.shown(i)
            || self
                .nodes
                .get(i)
                .zip(self.behaviour(i))
                .is_some_and(|(n, b)| n.look.dormant && !b.inert)
    }

    /// Which build of which frame its node indices are of: the same while
    /// nodes are only written in place, another once they came, went or
    /// moved (in any frame of the thread).
    pub(crate) fn epoch(&self) -> u64 {
        self.changes.epoch
    }

    /// The boxes of the nodes that act on the pointer (a click, press, drag
    /// or hover handler, a cursor of their own, focus, a text field or an
    /// [`Interaction`](crate::Interaction) their look reads), in paint order: where a hover can change what the screen shows.
    #[must_use]
    pub fn pointer_targets(&self) -> Vec<Bounds> {
        self.nodes
            .iter()
            .zip(&self.behaviour)
            .filter(|(_, b)| {
                let h = &b.handlers;
                !b.disabled
                    && (h.click.is_some()
                        || h.mouse_down.is_some()
                        || h.drag.is_some()
                        || h.hover.is_some()
                        || b.cursor.is_some()
                        || b.focus.is_some()
                        || b.field.is_some()
                        || b.interaction.is_some())
            })
            .map(|(n, _)| n.bounds)
            .collect()
    }

    /// Where the first pointer-taking span showing exactly `text` (a link)
    /// lies: the box of its first line.
    #[must_use]
    pub fn span_bounds(&self, text: &str) -> Option<Bounds> {
        self.nodes.iter().zip(&self.behaviour).find_map(|(node, b)| {
            let Content::Text { content, .. } = &node.content else {
                return None;
            };
            b.spans
                .iter()
                .find(|s| content.get(s.range.clone()) == Some(text))
                .and_then(|s| s.boxes.first().copied())
        })
    }

    /// The node and its ancestors within its layer, innermost first.
    pub fn ancestors(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(Some(i), |&n| self.nodes.get(n).and_then(|node| node.parent))
    }
}

/// A scroll container without a handle, by identity, as every target is
/// named: its path and the component instance whose output it is (the
/// nearest above it, itself included; else its owner). A component that
/// another replaces at its key leaves its offset behind: the new one's
/// list starts at the top.
pub(crate) type ScrollId = (NodePath, Option<crate::reactive::OwnerId>);

/// Values by scroll container identity ([`ScrollId`]): one entry a path,
/// held for its owner. Found by path and owner at once; another owner's
/// write at a path replaces what the last left there (no scan). Each
/// path written is logged, once however often, until the next layout pass
/// takes the log ([`LayoutTree::layout`]): a pass costs what changed, not
/// what is kept, and the log holds at most the paths kept.
#[derive(Clone, Debug)]
pub(crate) struct ByScroller<V> {
    map: HashMap<NodePath, (Option<crate::reactive::OwnerId>, V)>,
    /// The paths written since the last layout pass took them (a pass
    /// reads the state shared, and takes them as it places them).
    changed: std::cell::RefCell<HashSet<NodePath>>,
    /// Entries visited by writes (tests: a write is O(1)).
    #[cfg(test)]
    pub(crate) visited: u64,
}

impl<V> Default for ByScroller<V> {
    fn default() -> Self {
        Self {
            map: HashMap::default(),
            changed: std::cell::RefCell::default(),
            #[cfg(test)]
            visited: 0,
        }
    }
}

impl<V> ByScroller<V> {
    /// The value container `id` holds: its path's, if its owner wrote it.
    pub(crate) fn get(&self, id: &ScrollId) -> Option<&V> {
        self.map.get(&id.0).filter(|(owner, _)| *owner == id.1).map(|(_, v)| v)
    }

    /// Container `id` holds `v`, in place of what was at its path.
    pub(crate) fn insert(&mut self, id: ScrollId, v: V) {
        #[cfg(test)]
        {
            self.visited = self.visited.saturating_add(1);
        }
        let changed = self.changed.get_mut();
        if !changed.contains(&id.0) {
            changed.insert(id.0.clone());
        }
        self.map.insert(id.0, (id.1, v));
    }

    /// The paths written since the last call, the log emptied.
    fn take_written(&self) -> HashSet<NodePath> {
        self.changed.take()
    }

    /// How many paths the log holds (tests).
    #[cfg(test)]
    pub(crate) fn logged(&self) -> usize {
        self.changed.borrow().len()
    }

    /// Forget every value (tests).
    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        let gone: Vec<NodePath> = self.map.drain().map(|(p, _)| p).collect();
        self.changed.get_mut().extend(gone);
    }

    /// The paths holding a value.
    #[cfg(test)]
    fn paths(&self) -> impl Iterator<Item = &NodePath> {
        self.map.keys()
    }
}

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

/// What placing a text field says of its scroll, where that is not what
/// its handle holds: clamped, or moved to show its caret
/// ([`caret_reveal`]), or a newer reveal taken ([`LayoutTree::swap_fields`]).
pub(crate) struct FieldSettled {
    pub field: TextField,
    pub offset: (f32, f32),
    /// The reveal generation it took.
    pub applied: u64,
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

/// A closure bound to signals (ADR-0013's bindings by phase), run with a
/// tracker of its own: it runs again only when what it read changed, the
/// closure is another one, or the input `E` it runs over changed; its
/// change dirties its node alone.
struct Tracked<K: ?Sized, T, E = ()> {
    node: crate::reactive::RenderNode,
    memo: Option<(Rc<K>, E, T)>,
}

impl<K: ?Sized, T: Clone, E: PartialEq + Clone> Tracked<K, T, E> {
    fn new(marks: &Marks, u: u32) -> Self {
        Self {
            node: tracker(&resolve_mark(marks, u)),
            memo: None,
        }
    }

    fn get_over(&mut self, f: &Rc<K>, input: &E, run: impl FnOnce() -> T) -> T {
        if let Some((g, e, v)) = &self.memo
            && Rc::ptr_eq(g, f)
            && e == input
            && !self.node.is_marked()
        {
            return v.clone();
        }
        crate::component::count_binding();
        let v = self.node.track(run);
        self.memo = Some((Rc::clone(f), input.clone(), v.clone()));
        v
    }
}

impl<K: ?Sized, T: Clone> Tracked<K, T> {
    fn get(&mut self, f: &Rc<K>, run: impl FnOnce() -> T) -> T {
        self.get_over(f, &(), run)
    }
}

impl<K: ?Sized, T, E> Drop for Tracked<K, T, E> {
    fn drop(&mut self) {
        // Even while unwinding (a binding panicked): a tracker left
        // subscribed would mark a node gone; disposing is safe then.
        self.node.dispose();
    }
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

/// One phase's subscription of one node (ADR-0013: a signal read while
/// patching or placing a node marks only that phase of that node). Made
/// by the first run that reads anything; until then the tree's probe runs
/// the phase and keeps nothing.
#[derive(Default)]
struct Tracker(Option<crate::reactive::RenderNode>);

impl Tracker {
    fn run<R>(
        &mut self,
        probe: crate::reactive::RenderNode,
        marks: &Marks,
        u: u32,
        need: u8,
        f: impl FnOnce() -> R,
    ) -> R {
        if let Some(t) = self.0 {
            return t.track(f);
        }
        let out = probe.track(f);
        if probe.has_deps() {
            let marks = Rc::clone(marks);
            let t = crate::reactive::detached(|| {
                crate::reactive::RenderNode::new(move || {
                    if let Ok(mut m) = marks.try_borrow_mut() {
                        m.push((u, need));
                    }
                    crate::component::frame_due();
                })
            });
            t.take_deps(probe);
            self.0 = Some(t);
        }
        out
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        if let Some(t) = self.0.take() {
            t.dispose();
        }
    }
}

/// A node's bound style (ADR-0022): each bound setter's closure runs under
/// a tracker of its own, so a signal one of them read changing runs that
/// one again and replays the others from what they last gave (a binding
/// that reads `focus_visible` does not run on hover). The style is the
/// fixed style with every write replayed over it in the order it was set;
/// it is kept, keyed by the writes and the fixed style they replay over (a
/// node restyled through [`Element::style_mut`] shares its writes with the
/// node it was cloned from), until one of them or a closure's value changes.
struct StyleBinding {
    /// The mark its trackers share: node `u` needs its resolve.
    mark: crate::reactive::Mark,
    /// One per write, for those that are bound: what its closure read, and
    /// what it gave.
    runs: Vec<Option<BoundRun>>,
    /// What the replay itself read (a parent's restyle may read signals).
    replay: BoundRun,
    /// The writes, the fixed style and the style replayed, as last resolved.
    memo: Option<(Rc<Vec<crate::value::Op>>, Style, Style)>,
}

/// One closure's subscription and the value it last gave.
struct BoundRun {
    node: crate::reactive::RenderNode,
    slot: Option<Box<dyn std::any::Any>>,
}

impl BoundRun {
    fn new(mark: &crate::reactive::Mark) -> Self {
        Self {
            node: tracker(mark),
            slot: None,
        }
    }
}

impl Drop for BoundRun {
    fn drop(&mut self) {
        self.node.dispose();
    }
}

/// A binding's tracker, owned by the binding that holds it (disposed when
/// it drops), not by a scope of its own: a scope per bound node would cost
/// an arena node and its list for nothing.
fn tracker(mark: &crate::reactive::Mark) -> crate::reactive::RenderNode {
    crate::reactive::detached(|| crate::reactive::RenderNode::marking(mark))
}

/// What a tracker of node `u`'s resolve runs when something it read
/// changed: it asks for that resolve. Whether it changed is the tracker's
/// own flag ([`crate::reactive::RenderNode::is_marked`]), so one mark serves
/// every tracker of the node.
fn resolve_mark(marks: &Marks, u: u32) -> crate::reactive::Mark {
    let marks = Rc::clone(marks);
    Rc::new(move || {
        if let Ok(mut m) = marks.try_borrow_mut() {
            m.push((u, need::RESOLVE));
        }
        crate::component::frame_due();
    })
}

impl StyleBinding {
    fn new(marks: &Marks, u: u32) -> Self {
        let mark = resolve_mark(marks, u);
        Self {
            replay: BoundRun::new(&mark),
            mark,
            runs: Vec::new(),
            memo: None,
        }
    }

    /// Every tracker it holds.
    fn nodes(&self) -> impl Iterator<Item = crate::reactive::RenderNode> + '_ {
        self.runs
            .iter()
            .flatten()
            .map(|r| r.node)
            .chain(std::iter::once(self.replay.node))
    }

    /// `base` with `ops` replayed over it: the closures whose reads changed
    /// (all of them for other writes) run again, alone.
    fn get(&mut self, ops: &Rc<Vec<crate::value::Op>>, base: &Style) -> Style {
        let same_ops = self.memo.as_ref().is_some_and(|(o, ..)| Rc::ptr_eq(o, ops));
        if !same_ops {
            // Trackers past the writes, or of a write no longer bound,
            // keep reading nothing.
            for (k, run) in self.runs.iter_mut().enumerate() {
                if let Some(r) = run
                    && ops.get(k).and_then(crate::value::Op::bound).is_none()
                {
                    r.node.track(|| ());
                }
            }
        }
        let mut ran = false;
        for (k, op) in ops.iter().enumerate() {
            let Some(b) = op.bound() else { continue };
            if self.runs.len() <= k {
                self.runs.resize_with(k.saturating_add(1), || None);
            }
            let Some(slot) = self.runs.get_mut(k) else { continue };
            let run = slot.get_or_insert_with(|| BoundRun::new(&self.mark));
            if !same_ops || run.node.is_marked() || run.slot.is_none() {
                crate::component::count_binding();
                let held = &mut run.slot;
                run.node.track(|| b.eval(held));
                ran = true;
            }
        }
        if !ran
            && !self.replay.node.is_marked()
            && let Some((_, b, out)) = &self.memo
            && same_ops
            && b == base
        {
            return out.clone();
        }
        let runs = &self.runs;
        let out = self.replay.node.track(|| {
            let mut s = base.clone();
            for (k, op) in ops.iter().enumerate() {
                match (
                    op.bound(),
                    runs.get(k).and_then(Option::as_ref).and_then(|r| r.slot.as_deref()),
                ) {
                    (Some(b), Some(v)) => b.write(v, &mut s),
                    _ => op.apply(&mut s),
                }
            }
            s
        });
        match &mut self.memo {
            Some((o, b, v)) => {
                if !Rc::ptr_eq(o, ops) {
                    *o = Rc::clone(ops);
                }
                b.clone_from(base);
                v.clone_from(&out);
            }
            None => self.memo = Some((Rc::clone(ops), base.clone(), out.clone())),
        }
        out
    }
}

/// Its memo holds the text as shown: a resolve that finds it unchanged
/// shares it, copying nothing.
type TextBinding = Tracked<dyn Fn() -> String, Arc<str>>;
type DrawBinding = Tracked<dyn Fn() -> crate::chunk::Drawing, crate::chunk::Drawing>;

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

/// The waiting layer that opened first: one kept from the last pass, oldest
/// first, else the first new one found.
fn take_oldest(
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
    /// The layers' paths, as the last pass placed them.
    layer_paths: Vec<NodePath>,
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

fn set_style(node: &mut Node, style: BoxStyle) {
    if node.style != style {
        node.style = style;
        node.invalidate();
    }
}

/// What resolving a node's own style and content found.
struct Resolved {
    style_changed: bool,
    leaf_changed: bool,
    mine: Inherit,
}

/// How a pass writes a layer's part of the frame: every node pushed anew
/// (nodes came, went or moved), or in place, only nodes that changed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Write {
    All,
    Changed,
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

    /// The fields that came to show a handle this pass claim it: a handle
    /// is shown by one field at a time (its focus, its input method and its
    /// caret are one node's). Run once the whole frame is reconciled, every
    /// layer with it, so a field moving from a closing layer to another
    /// finds the one leaving gone. A second field showing a handle while
    /// the first is mounted is a bug, a panic in a debug build; else the
    /// first keeps it, and its focus.
    fn claim_fields(&mut self) {
        let mut mounting = std::mem::take(&mut self.mounting);
        for (f, u) in mounting.drain(..) {
            let Some(ex) = self.extra(u) else { continue };
            if !matches!(&ex.content, Pending::Input(p) if p.field.focus.id() == f) {
                continue;
            }
            match self.by_field.get(&f).copied() {
                Some(first) if first != u => {
                    debug_assert!(
                        self.detached,
                        "a TextField is shown by two fields; give each field its own TextField bound to the same model"
                    );
                    self.by_focus.insert(f, first);
                }
                _ => {
                    self.by_field.insert(f, u);
                    if let Some(ex) = self.extra_mut(u) {
                        ex.claim = Some(f);
                    }
                }
            }
        }
        self.mounting = mounting;
    }

    /// The handles claimed: one per field shown.
    #[cfg(test)]
    pub(crate) fn field_claims(&self) -> usize {
        self.by_field.len()
    }

    /// The focus handles it finds nodes by.
    #[cfg(test)]
    pub(crate) fn focus_entries(&self) -> usize {
        self.by_focus.len()
    }

    /// What it keeps for text fields (claims, what they settle): bounded
    /// by the fields shown, however long they are edited.
    #[cfg(test)]
    pub(crate) fn field_bookkeeping(&self) -> usize {
        self.by_field
            .len()
            .saturating_add(self.mounting.capacity())
            .saturating_add(self.fields.borrow().capacity())
    }

    /// What placing text fields said of their scroll since last taken
    /// (offsets it clamped or moved to show a caret), swapped into `into`
    /// (emptied first): the two buffers go back and forth, so settling
    /// allocates nothing once they have grown.
    pub(crate) fn swap_fields(&self, into: &mut Vec<FieldSettled>) {
        into.clear();
        std::mem::swap(&mut *self.fields.borrow_mut(), into);
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

    /// Lay `root` out in a `width`×`height` window: patch the tree from it
    /// along the paths that lead to what changed, lay out what changed, and
    /// write the frame's nodes that changed (all of them when nodes came,
    /// went or moved).
    pub fn layout(
        &mut self,
        root: &Element,
        width: f32,
        height: f32,
        state: &UiState,
        motion: &mut Motion,
        text: &mut TextSystem,
    ) -> &Frame {
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
            let moved_in_order = self.layer_paths.get(layer.saturating_sub(1)) != Some(&t.path);
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
        self.layer_paths.extend(self.layers.iter().map(|l| l.path.clone()));
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
        &self.frame
    }

    /// Drop the frame's nodes from `start` on, keeping them as they were
    /// when the frame was last painted, for the frame report's diff: as they
    /// are now, with what earlier passes wrote into them in place undone
    /// (each node's first box before, and whether one repainted it).
    fn rebuild_from(frame: &mut Frame, start: usize) {
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

    /// The box of the node at `path`, as placed.
    /// Whether the node at `path`, placed in `frame`, is hidden other than
    /// by sleeping, dormant alone, and inert.
    fn shown_at(&self, frame: &Frame, path: &NodePath) -> (bool, bool, bool) {
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

    fn bounds_at(&self, frame: &Frame, path: &NodePath) -> Option<Bounds> {
        let i = self.by_path.get(path).and_then(|&u| self.extra(u)).map(|e| e.index)?;
        frame
            .nodes
            .get(usize::try_from(i).ok()?)
            .filter(|n| &n.path == path)
            .map(|n| n.bounds)
            .or_else(|| frame_bounds_of(frame, path))
    }

    /// Lay a layer's root out against its anchor; where it goes and how it opened.
    fn place_layer(
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
        if let Some(h) = self.extra(node.user).and_then(|e| e.behaviour.scroll.clone()) {
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

    /// Patch `old` into what `el` (at `path`) now is, or build it anew; `None`
    /// when `el` takes no space.
    #[expect(clippy::too_many_arguments, reason = "a node, where it is and what it inherits")]
    fn renew(
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
    fn renew_layer(
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
            interaction: crate::interact::NodeInteraction::default(),
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
        let (probe, marks) = (self.probe, Rc::clone(&self.marks));
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
        if let Some(h) = ex.hidden {
            // Its hidden children are built again below, and say again what they read.
            h.track(|| ());
        }
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
    /// what decided that (its patch's reads, its style binding's) as
    /// `parent`'s: a change there resolves `parent` again, which builds it.
    fn hide(&mut self, parent: u32, node: Node) {
        let reads: Vec<crate::reactive::RenderNode> = self.extra(node.user).map_or_else(Vec::new, |e| {
            e.resolve
                .0
                .into_iter()
                .chain(e.style_binding.iter().flat_map(|b| b.nodes()))
                .collect()
        });
        if !reads.is_empty() && parent != NO_SLOT {
            let marks = Rc::clone(&self.marks);
            if let Some(p) = self.extra_mut(parent) {
                let hidden = *p.hidden.get_or_insert_with(|| {
                    crate::reactive::detached(|| {
                        crate::reactive::RenderNode::new(move || {
                            if let Ok(mut m) = marks.try_borrow_mut() {
                                m.push((parent, need::RESOLVE));
                            }
                            crate::component::frame_due();
                        })
                    })
                });
                for r in reads {
                    hidden.take_deps(r);
                }
            }
        }
        self.release(node);
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
        let disabled = inherit.disabled || common.disabled;
        let (mut style, focused) = self.resolve(pass.state, el, common, disabled, ex, u, layer);
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
        let dormant = inherit.dormant || common.dormant;
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
                let styles = n.kind.spans.iter().zip(ex.span_bindings.iter_mut()).map(|(sp, b)| {
                    // A span that no longer binds drops its trackers:
                    // what its old closures read resolves nothing.
                    let Some(ops) = sp.bindings.ops() else {
                        *b = None;
                        return None;
                    };
                    Some(
                        b.get_or_insert_with(|| StyleBinding::new(&self.marks, u))
                            .get(ops, &sp.style),
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
        // all they cover (two may start at the same byte).
        let same = el.ptr_eq(&ex.element)
            && ex.behaviour.cursor == style.cursor
            && match el {
                Element::Text(n) if !bound_spans.is_empty() => acting(&n.kind.spans, &bound_spans)
                    .map(|(k, cursor)| (k, n.kind.spans.get(k).map(|sp| &sp.range), cursor))
                    .eq(ex.behaviour.spans.iter().map(|t| (t.index, Some(&t.range), t.cursor))),
                _ => true,
            };
        let spans: Vec<SpanTarget> = match el {
            Element::Text(_) if same => Vec::new(),
            Element::Text(n) => acting(&n.kind.spans, &bound_spans)
                .filter_map(|(k, cursor)| {
                    let sp = n.kind.spans.get(k)?;
                    Some(SpanTarget {
                        index: k,
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
                let bound = match n.kind.bound.as_ref() {
                    Some(f) => Some(
                        ex.text_binding
                            .get_or_insert_with(|| Box::new(Tracked::new(&self.marks, u)))
                            .get(f, || Arc::from(f())),
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
                let d = ex
                    .draw_binding
                    .get_or_insert_with(|| Box::new(Tracked::new(&self.marks, u)))
                    .get(f, || f());
                (Pending::Drawn(d), None)
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
        let inert = inherit.inert || common.inert;
        let passes = inherit.passes || common.pass_through_all;
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
                scroll: common.scroll.clone(),
                scroll_built: common.scroll_built.clone(),
                semantics: common.semantics.clone(),
                interaction: ex.interaction.flags(),
                inert,
                tab_stop: common.tab_stop.clone(),
                tab_reversed: common.tab_reversed,
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
        if focused {
            self.focused = Some(ex.path.clone());
            self.focused_layer = layer;
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

    #[expect(clippy::too_many_arguments, reason = "a node, what it inherits and where it is")]
    fn resolve(
        &self,
        state: &UiState,
        el: &Element,
        common: &Common,
        disabled: bool,
        ex: &mut Extra,
        u: u32,
        layer: usize,
    ) -> (Style, bool) {
        let path = &ex.path;
        let enabled = !disabled;
        let focused = focus_of(common, path).is_some_and(|h| Some(h.id()) == state.focused);
        // The node's own flags, bound to the handle its element gives and
        // written as the window's input state has them, before its
        // bindings read them: a handle a render made reads the node as it
        // is. Hit testing and focus write them from then on.
        match common.interaction {
            // A detached tree binds no handle a live node holds: its
            // bindings read the flags the window's own tree bound and
            // wrote, which it leaves alone. A handle its own renders made
            // (no live node has it) it binds, until it goes.
            Some(handle) if self.detached && handle.is_bound() => {}
            Some(handle) => {
                let i = ex.interaction.bind(handle);
                let (focused, focus_visible, focus_within) = crate::interact::focus_flags(
                    path,
                    layer,
                    focused,
                    matches!(el, Element::Input(_)),
                    self.focused.as_ref().map(|f| (f, self.focused_layer)),
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
        let style = match common.bindings.ops() {
            Some(ops) => {
                let base = &common.style;
                ex.style_binding
                    .get_or_insert_with(|| Box::new(StyleBinding::new(&self.marks, u)))
                    .get(ops, base)
            }
            None => common.style.clone(),
        };
        (style, focused)
    }
}

/// What one patch pass reads and fills.
struct Pass<'s> {
    state: &'s UiState,
    motion: &'s mut Motion,
    todo: &'s mut std::collections::VecDeque<LayerTodo>,
}

/// Where a placed node puts its children.
#[derive(Clone, Copy, PartialEq)]
struct Placed {
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
    /// hides ([`engine::Node::inline_hidden`]): its children paint nothing
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
    fn place(
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
                        focus_within: self.focused.as_ref().is_some_and(|f| f.0.starts_with(&ce.path.0)),
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

/// The box of the node at `path`, once placed.
fn frame_bounds_of(frame: &Frame, path: &NodePath) -> Option<Bounds> {
    frame.nodes.iter().find(|n| &n.path == path).map(|n| n.bounds)
}

/// The box of the node keyed `key` inside the node at `anchor`, once placed.
fn frame_bounds_by_key(frame: &Frame, anchor: &NodePath, key: ElementKey) -> Option<Bounds> {
    frame
        .nodes
        .iter()
        .find(|n| n.path.0.starts_with(&anchor.0) && n.path.0.last().and_then(|s| s.element_key()) == Some(key))
        .map(|n| n.bounds)
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

/// How a text input lays out its text: the style, the wrap width, and where
/// the text box starts in the window, moved by the field's scroll.
pub(crate) struct InputText {
    pub t: ResolvedText,
    pub width: Option<f32>,
    pub origin: (f32, f32),
}

/// A text input's text as laid out in its content box `area`, its content
/// moved by `scroll` (the field's [`Scroll::applied`]: what the content
/// transform paint draws it under moves it by, so paint passes none). Its
/// scroll offset is its handle's (`TextField::offset`), as a
/// scroll container's is its own: caret moves bring the caret into view
/// ([`caret_reveal`]), the wheel and a held drag move it ([`field_reach`]
/// bounds it). Not being edited, a truncating single line is cut with an
/// ellipsis.
pub(crate) fn input_text(
    text: &mut TextSystem,
    content: &str,
    t: &ResolvedText,
    area: Bounds,
    multiline: bool,
    focused: bool,
    scroll: (f32, f32),
) -> InputText {
    // Being edited, it is never cut short (`ResolvedText::editing`).
    let edited;
    let t = if focused && (t.truncate || t.line_clamp.is_some()) {
        edited = t.clone().editing();
        &edited
    } else {
        t
    };
    if multiline {
        // Being edited, its box keeps the height it was measured at (a
        // clamp's, as Chrome's box does not grow on focus); its text
        // scrolls in it.
        return InputText {
            t: t.clone(),
            width: Some(area.width),
            origin: (area.x + scroll.0, area.y + scroll.1),
        };
    }
    let y = area.y + ((area.height - t.line_height) / 2.0).max(0.0) + scroll.1;
    // Not being edited, a truncating one is cut at its box with an
    // ellipsis, as Chrome's `text-overflow` cuts an unfocused input.
    if t.truncate {
        return InputText {
            t: ResolvedText {
                nowrap: true,
                ..t.clone()
            },
            width: Some(area.width),
            origin: (area.x + scroll.0, y),
        };
    }
    let line = ResolvedText {
        nowrap: true,
        truncate: false,
        ..t.clone()
    };
    // Text that fits is aligned in the box; text that does not scrolls,
    // from its inline start: a right-to-left one overflows to the left.
    let (free, _) = text.measure(content, &line, None);
    if line.physical_align() != TextAlign::Left && free <= area.width {
        return InputText {
            t: line,
            width: Some(area.width),
            origin: (area.x + scroll.0, y),
        };
    }
    let x = if line.direction == Direction::Rtl && free + 1.0 > area.width {
        area.right() - free - 1.0
    } else {
        area.x
    };
    InputText {
        t: line,
        width: None,
        origin: (x + scroll.0, y),
    }
}

/// How far a text input's content scrolls on each axis (its scroll's
/// [`Scroll::max`]): what of its text, laid out as [`input_text`] lays it
/// out, and its caret (a pixel wide past a line's end) reach past its
/// content box `area`. A clamp not being edited fits its box: none.
pub(crate) fn field_reach(
    text: &mut TextSystem,
    content: &str,
    t: &ResolvedText,
    area: Bounds,
    multiline: bool,
    focused: bool,
) -> (f32, f32) {
    let it = input_text(text, content, t, area, multiline, focused, (0.0, 0.0));
    let (width, height) = text.measure(content, &it.t, it.width);
    // Across: a line left unwrapped (a single line's not aligned in its
    // box, or a `nowrap` textarea's), down: a textarea's lines.
    let across = it.width.is_none() || (multiline && it.t.nowrap);
    (
        if across {
            (width + 1.0 - area.width).max(0.0)
        } else {
            0.0
        },
        if multiline {
            (height - area.height).max(0.0)
        } else {
            0.0
        },
    )
}

/// The scroll offset that brings a field's caret (at byte `caret` of its
/// `content`) into view from `offset`, scrolling each axis as little as
/// it must, as Chrome does (not at all where the caret shows already),
/// within `max` ([`field_reach`]): one rule for both axes of a single
/// line and a textarea alike.
#[expect(clippy::too_many_arguments, reason = "a field's text, box and scroll")]
pub(crate) fn caret_reveal(
    text: &mut TextSystem,
    content: &str,
    caret: usize,
    t: &ResolvedText,
    area: Bounds,
    multiline: bool,
    focused: bool,
    (offset, max): ((f32, f32), (f32, f32)),
) -> (f32, f32) {
    let it = input_text(text, content, t, area, multiline, focused, (0.0, 0.0));
    let c = text.caret(content, &it.t, it.width, caret);
    // The caret unscrolled, and the offsets (lo..=hi) that show it.
    let (x, y) = (it.origin.0 + c.x, it.origin.1 + c.y);
    let into = |o: f32, lo: f32, hi: f32| if o > hi { hi.max(lo) } else { o.max(lo) };
    // A right-to-left field scrolls its content rightward ([`Scroll::applied`]).
    let ox = if t.direction == Direction::Rtl {
        into(offset.0, area.x - x, area.right() - 1.0 - x)
    } else {
        into(offset.0, x + 1.0 - area.right(), x - area.x)
    };
    let oy = into(offset.1, y + c.height - area.bottom(), y - area.y);
    (ox.clamp(0.0, max.0.max(0.0)), oy.clamp(0.0, max.1.max(0.0)))
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
mod tests {
    use super::*;
    use crate::element::{IntoElement, Side, SideAlign, Text, deferred, div, key_of, span, text};
    use crate::prelude::*;
    use crate::styled::Styled;

    fn frame(root: impl IntoElement) -> Frame {
        let mut ts = TextSystem::new();
        layout(
            &root.into_element(),
            400.0,
            300.0,
            &UiState::default(),
            &mut Motion::default(),
            &mut ts,
        )
    }

    fn find<'a>(f: &'a Frame, name: &str) -> &'a FrameNode {
        f.nodes.iter().find(|n| n.name == Some(name)).expect("node")
    }

    fn changed(index: u32, x: f32, repainted: bool) -> Changed {
        Changed {
            index,
            before: Bounds {
                x,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            repainted,
        }
    }

    /// A log of `n` entries, node `i % 4` each, its box before at `i`.
    fn logged(n: u16) -> Changes {
        let mut c = Changes::default();
        c.rebuilt(0);
        c.log
            .extend((0..n).map(|i| changed(u32::from(i % 4), f32::from(i), false)));
        c
    }

    /// Room for no more than 2 * 0 + 64 entries: a trim of 100 keeps the
    /// last 32 and counts 68 dropped.
    #[test]
    fn a_trim_drops_the_older_half_and_counts_it() {
        let mut c = logged(100);
        let mut at_start = ReadAt::default();
        assert_eq!(c.since(&mut at_start), Since::Rebuilt, "never read: the whole frame");
        let (mut at_dropped, mut before_dropped, mut at_end) = (at_start, at_start, at_start);
        at_dropped.at = 68;
        before_dropped.at = 67;
        at_end.at = 100;
        c.trim(0);
        assert_eq!((c.log.len(), c.end()), (32, 100));
        let Since::Logged(log) = c.since(&mut at_dropped) else {
            panic!("a reader at the first entry kept reads on");
        };
        assert_eq!(log, &c.log[..]);
        assert_eq!(log.first().map(|e| e.before.x), Some(68.0));
        assert_eq!(
            c.since(&mut before_dropped),
            Since::FellBehind,
            "one entry it had not read was dropped"
        );
        assert_eq!(c.since(&mut at_end), Since::Logged(&[]));
        // Each moved to the end.
        assert_eq!(c.since(&mut before_dropped), Since::Logged(&[]));
        assert_eq!(at_dropped.at, 100);
        // The count keeps growing past what the log holds.
        c.log.push(changed(1, 0.0, true));
        assert_eq!(c.end(), 101);
        assert!(matches!(c.since(&mut at_end), Since::Logged([e]) if e.repainted));
    }

    #[test]
    fn a_rebuild_resets_what_was_dropped_and_every_reader() {
        let mut c = logged(100);
        let mut read = ReadAt::default();
        let _ = c.since(&mut read);
        c.trim(0);
        let epoch = c.epoch;
        c.rebuilt(0);
        assert_ne!(c.epoch, epoch);
        assert_eq!((c.log.len(), c.dropped, c.end()), (0, 0, 0));
        assert_eq!(c.since(&mut read), Since::Rebuilt, "indices it held mean nothing");
        c.log.push(changed(0, 0.0, false));
        assert_eq!(c.since(&mut read), Since::Logged(&[changed(0, 0.0, false)]));
    }

    /// The reader of every change (the frame report's diff) never falls
    /// behind: what
    /// a trim drops before it read it is kept for it, a node's first box
    /// before and whether any entry repainted it.
    #[test]
    fn what_a_trim_drops_unread_is_kept_for_the_reader_of_every_change() {
        let mut c = logged(10);
        let mut got = Vec::new();
        assert!(!c.unread(&mut got), "its first read compares the whole frame");
        assert!(c.unread(&mut got) && got.is_empty());
        c.log
            .extend((10..100_u16).map(|i| changed(u32::from(i % 4), f32::from(i), i == 50)));
        c.trim(0);
        assert!(c.unread(&mut got));
        // The 58 dropped since it read (10..68), one per node, then the log.
        let (kept, log) = got.split_at(4);
        assert_eq!(
            kept,
            [
                changed(0, 12.0, false),
                changed(1, 13.0, false),
                changed(2, 10.0, true),
                changed(3, 11.0, false)
            ]
        );
        assert_eq!(log, &c.log[..]);
        // What it read is not kept again.
        c.log.extend((100..200_u16).map(|i| changed(5, f32::from(i), false)));
        c.trim(0);
        assert!(c.unread(&mut got));
        assert_eq!(got.first(), Some(&changed(5, 100.0, false)));
        assert_eq!(got.len(), 1 + 32);
        // A rebuild from node 3 on keeps what was written before it.
        c.log.push(changed(1, 7.0, true));
        c.log.push(changed(3, 8.0, true));
        c.rebuilt(3);
        assert!(c.unread(&mut got));
        assert_eq!(got, [changed(1, 7.0, true)]);
    }

    /// Pass 0 writes nodes in place; pass 1 rebuilds from a node before
    /// them, and pass 2 from one before that (the kept nodes merged): what
    /// is kept is the frame as painted, each node's first box before, and
    /// the one repainted marked so.
    #[test]
    fn a_rebuild_keeps_the_nodes_as_painted_not_as_earlier_passes_wrote_them() {
        let mut f = frame(div().children((0..8).map(|_| div().size(10.0))));
        let mut got = Vec::new();
        // Painted: the frame report read it.
        let _ = f.changes.unread(&mut got);
        f.before = None;
        let painted = f.nodes.clone();
        let len = f.nodes.len();
        assert!(len >= 8);
        let at = |x: f32| Bounds {
            x,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        // Pass 0: node 7 moved twice (its first box before counts), node 5
        // repainted, node 3 moved.
        for (index, x, repainted) in [
            (7_usize, 50.0, false),
            (7, 60.0, false),
            (5, -1.0, true),
            (3, 40.0, false),
        ] {
            let n = &mut f.nodes[index];
            f.changes.log.push(Changed {
                index: u32::try_from(index).expect("small"),
                before: n.bounds,
                repainted,
            });
            if x >= 0.0 {
                n.bounds = at(x);
            }
        }
        let written = f.nodes.clone();
        // Pass 1 rebuilds from 4, writing the nodes as they are now.
        LayoutTree::rebuild_from(&mut f, 4);
        f.nodes.extend_from_slice(&written[4..]);
        f.changes.rebuilt(4);
        // Pass 2 rebuilds from 2.
        LayoutTree::rebuild_from(&mut f, 2);
        f.nodes.extend_from_slice(&written[2..]);
        f.changes.rebuilt(2);
        let before = f.before.take().expect("kept");
        assert_eq!(before.at, 2);
        assert_eq!(
            before.nodes.iter().map(|n| n.bounds).collect::<Vec<_>>(),
            painted[2..].iter().map(|n| n.bounds).collect::<Vec<_>>(),
            "as painted"
        );
        assert_eq!(before.repainted, [5 - 2]);
        // Nothing written in place stays to read: the kept nodes hold it.
        assert!(f.changes.unread(&mut got) && got.is_empty());
        let d = before.diff(&f);
        let changed: Vec<(Bounds, Bounds, bool)> = d
            .changes
            .iter()
            .filter_map(|c| match c.change {
                crate::inspect::Change::Updated {
                    before,
                    after,
                    repainted,
                } => Some((before, after, repainted)),
                _ => None,
            })
            .collect();
        assert_eq!(d.changes.len(), 3, "{d}");
        assert_eq!(order(&f, &d), [3, 5, 7], "in tree order: {d}");
        assert!(changed.contains(&(painted[7].bounds, at(60.0), false)), "{d}");
        assert!(changed.contains(&(painted[3].bounds, at(40.0), false)), "{d}");
        assert!(changed.contains(&(painted[5].bounds, painted[5].bounds, true)), "{d}");
    }

    /// A frame of eight boxes the frame report has read (painted), as
    /// painted: what the tests of a rebuild's undo start from.
    fn painted_frame() -> (Frame, Vec<FrameNode>) {
        let mut f = frame(div().children((0..8).map(|_| div().size(10.0))));
        let _ = f.changes.unread(&mut Vec::new());
        f.before = None;
        let painted = f.nodes.clone();
        assert!(painted.len() >= 9);
        (f, painted)
    }

    /// A pass writes node `index` in place: its box to `x` (none when
    /// `x` is negative), logged with its box before.
    fn write_at(f: &mut Frame, index: usize, x: f32, repainted: bool) {
        let n = &mut f.nodes[index];
        f.changes.log.push(Changed {
            index: u32::try_from(index).expect("small"),
            before: n.bounds,
            repainted,
        });
        if x >= 0.0 {
            n.bounds.x = x;
        }
    }

    /// A pass rebuilds the frame from `start`, writing its nodes as `now`.
    fn rebuild_as(f: &mut Frame, start: usize, now: &[FrameNode]) {
        LayoutTree::rebuild_from(f, start);
        f.nodes.extend_from_slice(&now[start..]);
        f.changes.rebuilt(start);
    }

    /// The order of the changed nodes in `d`, by index in `f`: each
    /// change is of a node still in `f` (one that went has no index; see
    /// the removal test for those).
    fn order(f: &Frame, d: &crate::inspect::LayoutDiff) -> Vec<usize> {
        d.changes
            .iter()
            .map(|c| {
                f.nodes
                    .iter()
                    .position(|n| n.path == c.path)
                    .unwrap_or_else(|| panic!("{:?} is not in the frame: {d}", c.path))
            })
            .collect()
    }

    /// A write the frame report read before the rebuild is part of the
    /// frame as painted: the rebuild undoes only what came after it.
    #[test]
    fn a_rebuild_does_not_undo_what_the_report_already_read() {
        let (mut f, painted) = painted_frame();
        write_at(&mut f, 5, 50.0, false);
        // Painted again: the report read the move.
        let mut got = Vec::new();
        assert!(f.changes.unread(&mut got) && got.len() == 1);
        write_at(&mut f, 6, 60.0, true);
        let now = f.nodes.clone();
        rebuild_as(&mut f, 4, &now);
        let before = f.before.take().expect("kept");
        assert_eq!(before.nodes[5 - 4].bounds.x, 50.0, "read: as painted, moved");
        assert_eq!(before.nodes[6 - 4].bounds, painted[6].bounds, "unread: undone");
        assert_eq!(before.repainted, [6 - 4]);
        let d = before.diff(&f);
        assert_eq!(order(&f, &d), [6], "{d}");
    }

    /// A trim folds the writes the report had not read; a rebuild in the
    /// same frame undoes them from what it folded.
    #[test]
    fn a_rebuild_after_a_trim_undoes_what_the_trim_folded() {
        let (mut f, painted) = painted_frame();
        write_at(&mut f, 5, 50.0, false);
        write_at(&mut f, 7, -1.0, true);
        write_at(&mut f, 5, 55.0, false);
        // Enough writes (each leaving node 1 where it was) that a trim
        // drops those above.
        for _ in 0..(2 * f.nodes.len() + 64) {
            write_at(&mut f, 1, -1.0, false);
        }
        f.changes.trim(f.nodes.len());
        assert!(f.changes.dropped >= 3, "the trim dropped them");
        assert!(f.changes.log.iter().all(|c| c.index == 1));
        let now = f.nodes.clone();
        rebuild_as(&mut f, 4, &now);
        let before = f.before.take().expect("kept");
        assert_eq!(before.nodes[5 - 4].bounds, painted[5].bounds, "its first box before");
        assert_eq!(before.repainted, [7 - 4]);
        let d = before.diff(&f);
        assert_eq!(order(&f, &d), [5, 7], "in tree order: {d}");
    }

    /// Between two rebuilds, from A then from S before it, passes write in
    /// place: the second undoes the writes into [S, A), and ignores those
    /// into nodes from A on, which the first kept as painted.
    #[test]
    fn a_second_rebuild_undoes_only_writes_into_what_it_newly_keeps() {
        let (mut f, painted) = painted_frame();
        let (s, a) = (2, 5);
        let now = f.nodes.clone();
        rebuild_as(&mut f, a, &now);
        write_at(&mut f, 3, 30.0, false);
        write_at(&mut f, 3, 35.0, false);
        write_at(&mut f, 4, -1.0, true);
        write_at(&mut f, 6, 60.0, true);
        write_at(&mut f, 7, 70.0, false);
        let now = f.nodes.clone();
        rebuild_as(&mut f, s, &now);
        let before = f.before.take().expect("kept");
        assert_eq!(before.at, s);
        assert_eq!(
            before.nodes.iter().map(|n| n.bounds).collect::<Vec<_>>(),
            painted[s..].iter().map(|n| n.bounds).collect::<Vec<_>>(),
            "as painted"
        );
        assert_eq!(before.repainted, [4 - s], "node 6's write ignored");
        let d = before.diff(&f);
        assert_eq!(order(&f, &d), [3, 4, 6, 7], "in tree order: {d}");
    }

    /// A rebuild that drops a marked node and reorders keyed ones: every
    /// change, removals included, in tree order then gone, each once; the
    /// marked node that went shows only as removed.
    #[test]
    fn a_rebuild_reports_a_removal_beside_a_keyed_reorder_once_each() {
        use crate::inspect::Change;
        let mut f = frame(div().children((0..8).map(|k| div().key(key_of(&format!("c{k}"))).size(10.0))));
        let _ = f.changes.unread(&mut Vec::new());
        f.before = None;
        // Child `k` of the root lies at node `k + 1`.
        let child = |k: usize| k + 1;
        for k in 0..8 {
            let key = key_of(&format!("c{k}"));
            assert_eq!(f.nodes[child(k)].path.0.last(), Some(&PathSeg::Key(key)));
        }
        // Painted, then c2 and c5 repainted in place.
        write_at(&mut f, child(2), -1.0, true);
        write_at(&mut f, child(5), -1.0, true);
        let painted = f.nodes.clone();
        // Rebuilt without c5, c1 and c3 swapped: each takes the box of
        // the place it lands in.
        let landed = [0, child(0), child(3), child(2), child(1), child(4), child(6), child(7)];
        let now: Vec<FrameNode> = landed
            .iter()
            .enumerate()
            .map(|(at, &k)| FrameNode {
                bounds: painted[at].bounds,
                ..painted[k].clone()
            })
            .collect();
        rebuild_as(&mut f, 1, &now);
        let before = f.before.take().expect("kept");
        assert_eq!(before.repainted, [child(2) - 1, child(5) - 1]);
        let d = before.diff(&f);
        let got: Vec<(NodePath, Change)> = d.changes.iter().map(|c| (c.path.clone(), c.change.clone())).collect();
        let path = |k: usize| painted[child(k)].path.clone();
        let bounds = |at: usize| painted[at].bounds;
        let moved = |k: usize, to: usize| Change::Updated {
            before: bounds(child(k)),
            after: bounds(to),
            repainted: false,
        };
        let expected = vec![
            (path(3), moved(3, child(1))),
            (
                path(2),
                Change::Updated {
                    before: bounds(child(2)),
                    after: bounds(child(2)),
                    repainted: true,
                },
            ),
            (path(1), moved(1, child(3))),
            (path(6), moved(6, child(5))),
            (path(7), moved(7, child(6))),
            (
                path(5),
                Change::Removed {
                    before: bounds(child(5)),
                },
            ),
        ];
        assert_eq!(got, expected, "{d}");
    }

    /// A binding that panics leaves no tracker subscribed: what held its
    /// trackers drops while the thread unwinds, and disposes them then.
    #[test]
    fn trackers_dropped_while_unwinding_leave_the_graph() {
        let marks: Marks = Rc::default();
        let read = crate::reactive::Signal::new(1_u32);
        let before = crate::reactive::graph();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut tracked: Tracked<dyn Fn() -> u32, u32> = Tracked::new(&marks, 0);
            let run = BoundRun::new(&resolve_mark(&marks, 0));
            let f: Rc<dyn Fn() -> u32> = Rc::new(move || read.get());
            let _ = tracked.get(&f, || f());
            let _ = run.node.track(|| read.get());
            std::panic::panic_any("a binding panicked");
        }));
        assert!(unwound.is_err());
        let after = crate::reactive::graph();
        assert_eq!(after.renders, before.renders, "both trackers left the graph");
        assert_eq!(after.subscriptions, before.subscriptions, "and their reads with them");
        read.set(2);
        assert!(marks.borrow().is_empty(), "a write marks no node");
    }

    #[test]
    fn flex_row_places_children_side_by_side_with_gap() {
        let f = frame(
            div()
                .display(Display::Flex)
                .gap(8.0)
                .padding(16.0)
                .child(div().named("a").size(40.0))
                .child(div().named("b").size(40.0)),
        );
        let (a, b) = (find(&f, "a").bounds, find(&f, "b").bounds);
        assert_eq!((a.x, a.y, a.width), (16.0, 16.0, 40.0));
        assert_eq!(b.x, 64.0, "40px box + 8px gap after the first");
    }

    #[test]
    fn text_wraps_inside_a_narrow_box() {
        let f = frame(div().child(div().named("box").width(60.0).child(text("several words that wrap"))));
        let b = find(&f, "box").bounds;
        assert!(b.height >= 48.0, "at least two 24px lines: {b}");
    }

    #[test]
    fn a_flex_item_shrinks_to_its_widest_word_and_no_further() {
        let f = frame(
            div().child(
                div()
                    .display(Display::Flex)
                    .align_items(Align::Start)
                    .width(20.0)
                    .child(div().named("prefix").child(text("https://")))
                    .child(div().named("rest").flex(Flex::grow(1.0))),
            ),
        );
        let b = find(&f, "prefix").bounds;
        assert!(b.width > 20.0, "as wide as the word, overflowing: {b}");
        assert_eq!(b.height, 24.0, "the word on one line: {b}");
    }

    #[test]
    fn a_deferred_layer_opens_below_its_anchor_and_stays_on_screen() {
        let f = frame(
            div()
                .padding(8.0)
                .child(
                    div()
                        .named("anchor")
                        .margin_left(320.0)
                        .width(60.0)
                        .height(20.0)
                        .child(deferred(
                            Positioning::Anchored {
                                side: Side::Bottom,
                                align: SideAlign::Start,
                                offset: 4.0,
                                match_width: false,
                                fit_height: false,
                            },
                            div().named("menu").width(200.0).height(50.0),
                        )),
                ),
        );
        let anchor = find(&f, "anchor").bounds;
        let menu = find(&f, "menu");
        assert_eq!(menu.layer, 1);
        assert_eq!(menu.bounds.y, anchor.bottom() + 4.0);
        assert_eq!(
            menu.bounds.right(),
            400.0 - EDGE_MARGIN,
            "pushed back inside the window"
        );
    }

    #[test]
    fn a_scroll_offset_is_set_in_constant_work_however_many_paths_are_kept() {
        let mut state = UiState::default();
        let path = |i: usize| NodePath::new([PathSeg::Index(0), PathSeg::Index(i)].into());
        for i in 0..1_000 {
            state.set_scroll((path(i), None), (0.0, 1.0));
        }
        let (a, b) = (crate::reactive::Scope::new(), crate::reactive::Scope::new());
        let (a, b) = (a.run(crate::reactive::owner), b.run(crate::reactive::owner));
        let before = state.scroll.visited;
        state.set_scroll((path(500), a), (0.0, 20.0));
        state.set_scroll((path(500), b), (0.0, 30.0));
        assert_eq!(state.scroll.visited - before, 2, "one entry a write");
        assert_eq!(
            state.scroll.get(&(path(500), a)),
            None,
            "the owner replaced at the path left nothing"
        );
        assert_eq!(state.scroll.get(&(path(500), b)), Some(&(0.0, 30.0)));
        assert_eq!(state.scroll.get(&(path(500), None)), None);
        assert_eq!(state.scroll.paths().count(), 1_000, "one entry a path");
    }

    /// A layout pass places the scroll writes since the last pass, and
    /// only those: one change among 1,000 remembered offsets is one entry.
    #[test]
    fn a_pass_places_only_the_scrolls_written_since_the_last() {
        let mut state = UiState::default();
        let mut ts = TextSystem::new();
        let el = div()
            .child(
                div()
                    .named("list")
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .children((0..10).map(|_| div().height(40.0))),
            )
            .into_element();
        let mut tree = LayoutTree::new();
        tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        let list = find(tree.frame(), "list").path.clone();
        for i in 0..999 {
            let other = NodePath::new([PathSeg::Index(7), PathSeg::Index(i)].into());
            state.set_scroll((other, None), (0.0, 1.0));
        }
        state.set_scroll((list.clone(), None), (0.0, 10.0));
        tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        assert_eq!(state.scroll.logged(), 0, "the pass took the log");
        let before = tree.scroll_marked;
        for i in 0..10_000_u16 {
            state.set_scroll((list.clone(), None), (0.0, f32::from(i % 300)));
        }
        state.set_scroll((list, None), (0.0, 50.0));
        assert_eq!(state.scroll.logged(), 1, "one path written, one entry");
        tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        assert_eq!(tree.scroll_marked - before, 1, "one placement for it");
        let offset = find(tree.frame(), "list").scroll.map(|s| s.offset.1);
        assert_eq!(offset, Some(50.0), "and it is placed");
    }

    /// Driven by the public API alone, a frame at a time, the window's
    /// input and layout keep the scroll log bounded: each pass takes it.
    #[test]
    fn ten_thousand_scrolled_frames_leave_the_scroll_log_bounded() {
        let mut ui = crate::interact::InputState::default();
        let mut ts = TextSystem::new();
        let el = div()
            .child(
                div()
                    .named("list")
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .children((0..10).map(|_| div().height(40.0))),
            )
            .into_element();
        let mut tree = LayoutTree::new();
        tree.layout(&el, 400.0, 300.0, ui.state(), &mut Motion::default(), &mut ts);
        for k in 0..10_000_u32 {
            let step = crate::compositor::WheelStep {
                point: (50.0, 50.0),
                at: f32::from(u16::try_from(k % 1000).unwrap_or(0)),
                ..crate::compositor::WheelStep::default()
            };
            let dy = if k % 2 == 0 { 30.0 } else { -30.0 };
            ui.wheel_at(tree.frame(), step, (0.0, dy), None);
            tree.layout(&el, 400.0, 300.0, ui.state(), &mut Motion::default(), &mut ts);
            assert!(ui.state().scroll.logged() == 0, "frame {k}: the pass took the log");
            if k == 0 {
                let offset = find(tree.frame(), "list").scroll.map(|s| s.offset.1);
                assert_eq!(offset, Some(30.0), "the wheel scrolls it");
            }
        }
        let offset = find(tree.frame(), "list").scroll.map(|s| s.offset.1);
        assert_eq!(offset, Some(0.0), "it did scroll, back and forth");
    }

    #[test]
    fn scrolled_children_move_up_and_are_clipped() {
        let mut state = UiState::default();
        let mut ts = TextSystem::new();
        let tree = || {
            div()
                .child(
                    div()
                        .named("list")
                        .height(100.0)
                        .overflow_y(Overflow::Scroll)
                        .overflow_x(Overflow::Hidden)
                        .children((0..10).map(|i| div().named(if i == 0 { "first" } else { "item" }).height(40.0))),
                )
                .into_element()
        };
        let before = layout(&tree(), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        let path = find(&before, "list").path.clone();
        state.scroll.insert((path, None), (0.0, 1000.0));
        let after = layout(&tree(), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        let list = find(&after, "list");
        assert_eq!(list.scroll.map(|s| s.offset.1), Some(300.0), "clamped to 400 - 100");
        assert_eq!(find(&after, "first").bounds.y, -300.0);
        assert_eq!(find(&after, "first").clip, Some(list.bounds));
    }

    #[test]
    fn a_scroll_offset_moves_content_by_whole_device_pixels() {
        let mut state = UiState {
            scale: 2.0,
            ..UiState::default()
        };
        let mut ts = TextSystem::new();
        let tree = || {
            div()
                .child(
                    div()
                        .named("list")
                        .height(100.0)
                        .overflow_y(Overflow::Scroll)
                        .overflow_x(Overflow::Hidden)
                        .children((0..10).map(|i| div().named(if i == 0 { "first" } else { "item" }).height(40.0))),
                )
                .into_element()
        };
        let before = layout(&tree(), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        let path = find(&before, "list").path.clone();
        state.scroll.insert((path, None), (0.0, 10.3));
        let after = layout(&tree(), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        assert_eq!(
            find(&after, "first").bounds.y,
            -10.0,
            "10.3 is 20.6 device pixels: 20 whole ones"
        );
    }

    // ── The retained tree against a cold layout (L1, L3, L7) ────────────────

    /// A seeded generator (an LCG: tests repeat).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }
        fn below(&mut self, n: u64) -> usize {
            usize::try_from(self.next() % n.max(1)).unwrap()
        }
    }

    /// An element tree as data, mutated step by step.
    #[derive(Clone, Debug)]
    struct Spec {
        look: u8,
        key: Option<u64>,
        text: Option<String>,
        kids: Vec<Spec>,
    }

    const WORDS: [&str; 8] = ["a", "box", "several", "words", "that", "wrap", "longer-word", "x"];

    fn words(r: &mut Rng) -> String {
        (0..=r.below(6))
            .map(|_| WORDS[r.below(8)])
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn spec(r: &mut Rng, depth: usize) -> Spec {
        if depth == 0 || r.below(4) == 0 {
            // A paragraph, now and then holding elements in its lines.
            let inline = if depth > 0 && r.below(3) == 0 { r.below(3) } else { 0 };
            return Spec {
                look: u8::try_from(r.below(8)).unwrap(),
                key: None,
                text: Some(words(r)),
                kids: (0..inline).map(|_| spec(r, depth - 1)).collect(),
            };
        }
        Spec {
            look: u8::try_from(r.below(64)).unwrap(),
            key: (r.below(3) == 0).then(|| r.next() % 5),
            text: None,
            kids: (0..r.below(5)).map(|_| spec(r, depth - 1)).collect(),
        }
    }

    /// Siblings' keys are unique (a repeated key is the caller's error).
    fn element(s: &Spec) -> Element {
        built(s, true)
    }

    fn built(s: &Spec, keyed: bool) -> Element {
        if let Some(t) = &s.text {
            let mut seen = HashSet::new();
            let p = s.kids.iter().fold(text(t.clone()), |p, k| {
                p.child(built(k, k.key.is_none_or(|key| seen.insert(key))))
                    .span(" and more")
            });
            let d = div().child(p);
            return match s.look % 4 {
                0 => d.width(50.0),
                1 => d.flex(Flex::grow(1.0)),
                2 => d.padding(4.0),
                _ => d,
            }
            .into_element();
        }
        let mut d = match s.look % 8 {
            0 => div().display(Display::Flex),
            1 => div().display(Display::Flex).flex_direction(FlexDirection::Column),
            2 => div()
                .display(Display::Grid)
                .grid_columns(Tracks::repeat(2, Track::Fr(1.0))),
            3 => div().display(Display::Flex).flex_wrap(true),
            4 => div().display(Display::Block),
            5 => div().position(Position::Absolute).top(4.0).left(8.0),
            6 => div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .height(60.0)
                .overflow_y(Overflow::Scroll)
                .overflow_x(Overflow::Hidden),
            _ => div(),
        };
        d = match (s.look / 8) % 6 {
            0 => d.padding(8.0),
            1 => d.gap(4.0).align_items(Align::Center),
            2 => d.margin_left(Length::Auto),
            3 => d.width(120.0),
            4 => d.justify_content(Justify::Between).height(40.0),
            _ => d.max_width(Length::calc(1.0, -20.0)),
        };
        if s.look >= 60 {
            d = d.display(Display::None);
        }
        if let (Some(k), true) = (s.key, keyed) {
            d = d.key(k);
        }
        let mut seen = HashSet::new();
        d.children(
            s.kids
                .iter()
                .map(|k| built(k, k.key.is_none_or(|key| seen.insert(key)))),
        )
        .into_element()
    }

    /// One random change somewhere in `s`.
    fn mutate(s: &mut Spec, r: &mut Rng) {
        if !s.kids.is_empty() && r.below(3) != 0 {
            let i = r.below(s.kids.len() as u64);
            return mutate(&mut s.kids[i], r);
        }
        match r.below(7) {
            0 => s.look = u8::try_from(r.below(64)).unwrap(),
            1 if s.text.is_some() => s.text = Some(words(r)),
            2 => {
                let at = r.below(s.kids.len() as u64 + 1);
                s.kids.insert(at, spec(r, 2));
            }
            3 if !s.kids.is_empty() => {
                let at = r.below(s.kids.len() as u64);
                s.kids.remove(at);
            }
            4 if s.kids.len() > 1 => {
                let last = s.kids.len() - 1;
                s.kids.swap(0, last);
            }
            5 if s.text.is_none() => s.key = (r.below(2) == 0).then(|| r.next() % 5),
            _ => s.kids.reverse(),
        }
    }

    fn check(seed: u64, schedule: Schedule) {
        let mut r = Rng(seed);
        let mut s = spec(&mut r, 4);
        let mut tree = LayoutTree::with_schedule(schedule);
        let mut ts = TextSystem::new();
        let state = UiState::default();
        for step in 0..40 {
            let el = div().width(pct(100.0)).child(element(&s)).into_element();
            let width = if step % 13 == 12 { 300.0 } else { 400.0 };
            let warm = tree.layout(&el, width, 300.0, &state, &mut Motion::default(), &mut ts);
            let cold = layout(&el, width, 300.0, &state, &mut Motion::default(), &mut ts);
            let d = crate::inspect::diff(&cold.nodes, &warm.nodes);
            assert!(
                d.is_empty() && warm.nodes.len() == cold.nodes.len(),
                "seed {seed} step {step}: the retained tree differs from a cold layout:\n{d}\n{s:#?}"
            );
            mutate(&mut s, &mut r);
        }
    }

    #[test]
    fn patched_trees_lay_out_as_cold_ones() {
        for seed in 0..60 {
            check(seed, Schedule::default());
        }
    }

    #[test]
    fn patched_trees_lay_out_alike_in_parallel_and_in_reverse() {
        for seed in 0..30 {
            check(
                seed,
                Schedule {
                    parallel: true,
                    reverse: false,
                },
            );
            check(
                seed,
                Schedule {
                    parallel: false,
                    reverse: true,
                },
            );
        }
    }

    #[test]
    fn an_unchanged_tree_runs_no_layout() {
        let mut tree = LayoutTree::new();
        let mut ts = TextSystem::new();
        let el = div()
            .display(Display::Flex)
            .child(div().child(text("hello")))
            .child(div().width(10.0))
            .into_element();
        let state = UiState::default();
        tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        assert_eq!(tree.stats(), Stats::default());
    }

    /// A table-like tree big enough to be laid out in parallel jobs.
    fn rows(n: usize, tall: usize) -> Element {
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .width(900.0)
            .children((0..n).map(|i| {
                div()
                    .display(Display::Grid)
                    .grid_columns(Tracks::repeat(4, Track::Fr(1.0)))
                    .gap(4.0)
                    .children((0..4).map(|c| {
                        let words = if i == tall {
                            "a much longer cell that wraps over lines"
                        } else {
                            WORDS[(i + c) % 8]
                        };
                        let p = text(format!("{words} {i}.{c}"));
                        // A cell holding an element in its line.
                        let p = if c == 3 {
                            p.child(div().width(12.0).height(30.0).child(text("x"))).span(" y")
                        } else {
                            p
                        };
                        div().padding(4.0).child(p)
                    }))
            }))
            .into_element()
    }

    fn exact(f: &Frame) -> Vec<(u32, u32, u32, u32)> {
        f.nodes
            .iter()
            .map(|n| {
                (
                    n.bounds.x.to_bits(),
                    n.bounds.y.to_bits(),
                    n.bounds.width.to_bits(),
                    n.bounds.height.to_bits(),
                )
            })
            .collect()
    }

    #[test]
    fn parallel_serial_and_reversed_layouts_are_bit_identical() {
        let state = UiState::default();
        let mut ts = TextSystem::new();
        let schedules = [
            Schedule {
                parallel: false,
                reverse: false,
            },
            Schedule {
                parallel: true,
                reverse: false,
            },
            Schedule {
                parallel: false,
                reverse: true,
            },
            Schedule {
                parallel: true,
                reverse: true,
            },
        ];
        let mut trees: Vec<LayoutTree> = schedules.iter().map(|s| LayoutTree::with_schedule(*s)).collect();
        let mut jobs = 0;
        for (step, tall) in [usize::MAX, 17, 300, usize::MAX].into_iter().enumerate() {
            let el = rows(400, tall);
            let frames: Vec<Vec<_>> = trees
                .iter_mut()
                .map(|t| exact(t.layout(&el, 1000.0, 800.0, &state, &mut Motion::default(), &mut ts)))
                .collect();
            jobs += trees[1].stats().jobs;
            let cold = exact(&layout(&el, 1000.0, 800.0, &state, &mut Motion::default(), &mut ts));
            for (f, s) in frames.iter().zip(&schedules) {
                assert!(*f == cold, "step {step}: {s:?} differs from a cold layout");
            }
        }
        // The parallel schedule did run jobs on the pool.
        assert!(jobs > 0 || engine::pool().current_num_threads() < 2);
    }

    /// A cold layout of a tree the pool would take runs on its own thread:
    /// no job reaches a pool thread's text system, a live cache.
    #[test]
    fn a_cold_layout_hands_the_pool_no_jobs() {
        let state = UiState::default();
        let mut ts = TextSystem::new();
        let el = rows(400, usize::MAX);
        let mut live = LayoutTree::with_schedule(Schedule {
            parallel: true,
            reverse: false,
        });
        live.layout(&el, 1000.0, 800.0, &state, &mut Motion::default(), &mut ts);
        if engine::pool().current_num_threads() < 2 {
            return;
        }
        assert!(live.stats().jobs > 0, "the tree is one the pool takes");
        let before = engine::handed();
        let cold = crate::engine::in_cold_frame(|| {
            exact(&layout(&el, 1000.0, 800.0, &state, &mut Motion::default(), &mut ts))
        });
        assert_eq!(engine::handed(), before, "the cold layout handed the pool no job");
        assert!(cold == exact(live.frame()), "and lays out the same");
    }

    // ── Atomic inlines in a paragraph ───────────────────────────────────────

    /// A paragraph's first baseline and its one line's height, alone.
    fn strut() -> (f32, f32) {
        let mut ts = TextSystem::new();
        let t = crate::style::ResolvedText::root(Color::BLACK);
        let m = ts.metrics("ab", &t, None);
        (m.first_baseline.expect("a line"), m.height)
    }

    fn near(a: f32, b: f32, what: &str) {
        assert!((a - b).abs() < 1.0, "{what}: {a} is not {b}");
    }

    #[test]
    fn a_box_with_a_baseline_sits_on_it_hangs_below_and_grows_the_line() {
        let (ascent, line) = strut();
        // A 60px box whose own first line is at its top: its baseline is
        // the strut's, so it starts at the line's top and hangs 60 - ascent
        // below the baseline, past the strut's descent.
        let f = frame(
            div().width(300.0).child(
                text("ab ")
                    .named("p")
                    .child(div().named("box").width(30.0).height(60.0).child(text("x")))
                    .span(" cd"),
            ),
        );
        let (p, b) = (find(&f, "p").bounds, find(&f, "box").bounds);
        assert!(line < 60.0);
        near(p.height, 60.0, "the line holds the box above and below its baseline");
        near(b.y - p.y, 0.0, "its baseline on the line's, at the strut's ascent");
        assert_eq!(b.height, 60.0);
        let ab = TextSystem::new()
            .measure("ab ", &crate::style::ResolvedText::root(Color::BLACK), None)
            .0;
        near(b.x - p.x, ab, "after the text before it");
        assert!(ascent > 0.0);
    }

    #[test]
    fn a_line_holding_only_a_box_takes_the_strut_and_the_box() {
        let (ascent, line) = strut();
        let descent = line - ascent;
        let only = |h: f32| {
            let f = frame(
                div()
                    .width(300.0)
                    .child(text("").named("p").child(div().named("box").width(20.0).height(h))),
            );
            (find(&f, "p").bounds, find(&f, "box").bounds)
        };
        // No line box in it: its bottom edge on the baseline.
        let (p, b) = only(10.0);
        near(p.height, ascent.max(10.0) + descent, "a short box: the strut");
        near(b.y - p.y + b.height, ascent, "its bottom on the baseline");
        let (p, b) = only(40.0);
        near(
            p.height,
            40.0 + descent,
            "a tall box above the baseline, the strut's descent below",
        );
        near(b.y - p.y, 0.0, "at the line's top");
    }

    #[test]
    fn a_box_wraps_with_the_words_around_it() {
        let para = |w: f32| {
            let f = frame(
                div().child(
                    div().width(w).child(
                        text("several words before ")
                            .named("p")
                            .child(div().named("box").width(40.0).height(10.0)),
                    ),
                ),
            );
            (find(&f, "p").bounds, find(&f, "box").bounds)
        };
        let (wide, at_wide) = para(390.0);
        let (narrow, at_narrow) = para(150.0);
        assert!(
            at_wide.x - wide.x > 100.0,
            "on the first line after the words: {at_wide}"
        );
        assert!(
            narrow.height > wide.height,
            "two lines when narrow: {narrow} {at_narrow} vs {wide} {at_wide}"
        );
        assert!(at_narrow.y > at_wide.y, "moved down a line: {at_narrow} vs {at_wide}");
    }

    #[test]
    fn an_inline_box_sizes_to_fit_and_is_a_node_below_its_paragraph() {
        let f = frame(
            div().width(300.0).child(
                text("see ")
                    .named("p")
                    .child(div().named("chip").padding(4.0).child(text("host")))
                    .span(" here"),
            ),
        );
        let (p, chip) = (find(&f, "p"), find(&f, "chip"));
        let host = TextSystem::new()
            .measure("host", &crate::style::ResolvedText::root(Color::BLACK), None)
            .0;
        near(chip.bounds.width, host + 8.0, "shrunk to fit its content, not the line");
        let pi = f.nodes.iter().position(|n| n.name == Some("p")).unwrap();
        assert_eq!(chip.parent, Some(pi), "the chip is the paragraph's child");
        let Content::Text { content, boxes, .. } = &p.content else {
            panic!("a paragraph")
        };
        assert_eq!(boxes.len(), 1);
        assert_eq!(crate::text::readable(content), "see  here");
    }

    #[test]
    fn a_changed_inline_box_lays_out_as_a_cold_tree() {
        let mut tree = LayoutTree::new();
        let mut ts = TextSystem::new();
        let state = UiState::default();
        for w in [20.0, 80.0, 200.0, 20.0] {
            let el = div()
                .width(240.0)
                .child(
                    text("some words around ")
                        .child(div().named("box").width(w).height(w / 2.0).child(text("in")))
                        .span(" a box"),
                )
                .into_element();
            let warm = exact(tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts));
            let cold = exact(&layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts));
            assert!(
                warm == cold,
                "a {w}px box: the retained tree differs from a cold layout"
            );
        }
    }

    /// The text of the frame node showing `content`.
    fn shown<'a>(f: &'a Frame, content: &str) -> &'a ResolvedText {
        f.nodes
            .iter()
            .find_map(|n| match &n.content {
                Content::Text { content: c, text, .. } if &**c == content => Some(text),
                _ => None,
            })
            .expect("a node shows it")
    }

    /// A paragraph of many spans replaced at its key by plain text lets go
    /// of the buffer its runs were resolved into, and of the runs it showed.
    #[test]
    fn a_paragraph_that_loses_its_spans_lets_go_of_their_buffer() {
        let mut tree = LayoutTree::new();
        let mut ts = TextSystem::new();
        let state = UiState::default();
        let p = |el: Text| div().child(el.key(key_of("p")).named("p")).into_element();
        let spans = (0..20_000).fold(text(""), |t, _| t.span(span("x").font_weight(700)));
        tree.layout(&p(spans), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        let ex = |tree: &LayoutTree| {
            tree.extras
                .iter()
                .flatten()
                .find(|e| e.name == Some("p"))
                .map(|e| (e.runs.capacity(), e.span_content.capacity()))
                .expect("the paragraph")
        };
        let (runs, content) = ex(&tree);
        assert!(
            runs * std::mem::size_of::<Run>() > 64 * 1024,
            "its buffer is large: {runs} runs"
        );
        assert!(content >= 20_000, "one content style a span: {content}");
        tree.layout(&p(text("plain")), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        assert_eq!(ex(&tree), (0, 0), "plain text keeps no buffer for runs");
    }

    /// A bound paragraph replaced at its key by plain text drops its
    /// binding: the signal it read resolves nothing, and the text it gave
    /// is held nowhere.
    #[test]
    fn a_bound_paragraph_made_plain_drops_its_binding() {
        let mut tree = LayoutTree::new();
        let mut ts = TextSystem::new();
        let state = UiState::default();
        let read = crate::reactive::Signal::new(1_u32);
        let p = |el: Text| div().child(el.key(key_of("p"))).into_element();
        let bound = p(crate::element::text_bound(move || format!("bound {}", read.get())));
        let f = tree.layout(&bound, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        let old = f
            .nodes
            .iter()
            .find_map(|n| match &n.content {
                Content::Text { content, .. } if &**content == "bound 1" => Some(Arc::clone(content)),
                _ => None,
            })
            .expect("the bound text");
        tree.layout(&p(text("plain")), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        assert_eq!(Arc::strong_count(&old), 1, "the old text is held nowhere else");
        read.set(2);
        assert!(
            tree.marks.borrow().is_empty(),
            "a write to the old signal marks no node"
        );
    }

    /// A parent's underline reaches a child in its flow, not an atomic
    /// inline nor an absolutely positioned box (css-text-decor-3 §2.1),
    /// laid out and when the underline comes on later, repainted.
    #[test]
    fn decorations_stop_at_atomic_inlines_and_out_of_flow_boxes() {
        let el = |underline: bool| {
            div()
                .underline(underline)
                .child(text("normal"))
                .child(text("inline").inline(true))
                .child(text("absolute").position(Position::Absolute))
                .into_element()
        };
        let check = |f: &Frame| {
            assert!(shown(f, "normal").decorations.underline(), "a child in flow draws it");
            assert!(!shown(f, "inline").decorations.underline(), "an atomic inline does not");
            assert!(
                !shown(f, "absolute").decorations.underline(),
                "an out-of-flow box does not"
            );
        };
        check(&frame(el(true)));
        let mut tree = LayoutTree::new();
        let mut ts = TextSystem::new();
        let state = UiState::default();
        tree.layout(&el(false), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        check(tree.layout(&el(true), 400.0, 300.0, &state, &mut Motion::default(), &mut ts));
    }

    /// A span's vertical padding only grows its background (CSS pads an
    /// inline box so, its line box keeping its height): what the
    /// paragraph is measured by leaves it out.
    #[test]
    fn a_runs_vertical_padding_is_not_measured() {
        let run = Run {
            range: 0..1,
            text: ResolvedText::root(Color::BLACK),
            background: Some(Color::BLACK),
            radius: 2.0,
            padding: Edges {
                top: 3.0,
                right: 4.0,
                bottom: 5.0,
                left: 6.0,
            },
        };
        let m = Leaf::measuring_run(&run);
        assert_eq!(
            m.padding,
            Edges {
                top: 0.0,
                right: 4.0,
                bottom: 0.0,
                left: 6.0,
            },
            "the sides take space in the line, the top and bottom none"
        );
    }

    /// A paragraph that alternates between 20,000 and 9,000 spans keeps the
    /// room of the larger: its scratch is not cut on the small update and
    /// grown again on the large one.
    #[test]
    fn a_paragraph_alternating_sizes_keeps_its_scratch() {
        let mut tree = LayoutTree::new();
        let mut ts = TextSystem::new();
        let state = UiState::default();
        let p = |n: usize| {
            let spans = (0..n).fold(text(""), |t, _| t.span(span("x").font_weight(700)));
            div().child(spans.key(key_of("p")).named("p")).into_element()
        };
        let room = |tree: &LayoutTree| {
            tree.extras
                .iter()
                .flatten()
                .find(|e| e.name == Some("p"))
                .map(|e| e.runs.capacity())
                .expect("the paragraph")
        };
        let (big, small) = (p(20_000), p(9_000));
        tree.layout(&big, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        let peak = room(&tree);
        for step in 0..3 {
            tree.layout(&small, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
            assert_eq!(room(&tree), peak, "step {step}: the small update keeps the room");
            tree.layout(&big, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
            assert_eq!(room(&tree), peak, "step {step}: the large one needs no more");
        }
    }

    /// A layout pass that handed the pool no jobs leaves the pool's text
    /// systems alone (what they hold is another pass's to clear), and one
    /// that did clears what they shaped before it returns: no flag shared
    /// between passes on different threads decides either. The pool is the
    /// process's, shared with tests running beside this one: an attempt
    /// another test's pass disturbed is tried again.
    #[test]
    fn each_pass_clears_the_pool_after_its_own_jobs() {
        use super::engine::Worker;
        let attempt = || {
            // Another pass's jobs, still running: they shaped text.
            engine::pool().broadcast(|_| {
                TextSystem::with_worker(|t| {
                    t.measure("words another pass shaped", &ResolvedText::root(Color::BLACK), None);
                });
            });
            let shaped = leaf::idle_footprint();
            // A pass on another thread that handed the pool nothing ends.
            std::thread::spawn(leaf::end_pass).join().expect("a pass ends");
            let kept = leaf::idle_footprint();
            // A pass that handed it jobs clears after them.
            let mut tree = LayoutTree::with_schedule(Schedule {
                parallel: true,
                reverse: false,
            });
            let mut ts = TextSystem::new();
            tree.layout(
                &rows(400, 17),
                1000.0,
                800.0,
                &UiState::default(),
                &mut Motion::default(),
                &mut ts,
            );
            let cleared = tree.stats().jobs == 0 || leaf::idle_footprint() == 0;
            (shaped, kept, cleared)
        };
        let tries: Vec<_> = (0..8).map(|_| attempt()).collect();
        assert!(
            tries
                .iter()
                .any(|&(shaped, kept, cleared)| shaped > 0 && kept >= shaped && cleared),
            "(shaped, kept by a pass that handed nothing, cleared by one that did): {tries:?}"
        );
    }

    /// Two empty spans at the same byte and a binding that moves the same
    /// cursor from the first to the second: their range and cursor are the
    /// same, and the target is still the second's, told apart by which
    /// span it is.
    #[test]
    fn a_span_target_follows_its_span_not_its_start() {
        let mut tree = LayoutTree::new();
        let mut ts = TextSystem::new();
        let state = UiState::default();
        let on = crate::reactive::Signal::new(false);
        let el = div()
            .child(
                text("go ")
                    .span(span("").cursor(move || if on.get() { None } else { Some(Cursor::Pointer) }))
                    .span(span("").cursor(move || if on.get() { Some(Cursor::Pointer) } else { None }))
                    .span("link"),
            )
            .into_element();
        let targets = |f: &Frame| -> Vec<(usize, std::ops::Range<usize>)> {
            f.behaviour
                .iter()
                .flat_map(|b| b.spans.iter())
                .map(|t| (t.index, t.range.clone()))
                .collect()
        };
        let f = tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        assert_eq!(targets(f), [(0, 3..3)], "the first empty span takes the pointer");
        on.set(true);
        let f = tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        assert_eq!(targets(f), [(1, 3..3)], "then the second, at the same byte");
    }

    /// A button its paragraph's clamp hides is neither focusable nor a
    /// target of assistive technology; shown, it is both.
    #[test]
    fn a_hidden_inline_is_no_focus_and_no_action_target() {
        let focus = FocusHandle::new();
        let el = |width: f32| {
            div()
                .child(
                    text("words before ")
                        .child(
                            div()
                                .inline(true)
                                .padding_x(4.0)
                                .track_focus(focus)
                                .on_click(|_| {})
                                .child(text("the button")),
                        )
                        .line_clamp(1)
                        .width(width),
                )
                .into_element()
        };
        for (width, shown) in [(400.0, true), (100.0, false)] {
            let f = frame(el(width));
            let i = (0..f.nodes.len())
                .find(|&i| f.behaviour(i).is_some_and(|b| b.handlers.click.is_some()))
                .expect("the button");
            assert_eq!(f.nodes[i].look.invisible, !shown, "{width}px: hidden or shown");
            let id = crate::access::id_of(&f.nodes[i].path);
            assert_eq!(
                crate::access::target(&f, id).is_some(),
                shown,
                "{width}px: a screen reader's action reaches it only shown"
            );
            assert_eq!(
                crate::focus::node_of(&f, focus.id()).is_some(),
                shown,
                "{width}px: it holds focus only shown"
            );
        }
    }
}
