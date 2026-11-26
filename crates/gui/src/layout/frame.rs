use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use crate::color::Color;
use crate::edit::TextField;
use crate::element::{ElementKey, Handler, Handlers, InputMode, Semantics};
use crate::events::ClickEvent;
use crate::focus::FocusHandle;
use crate::geometry::{Bounds, NodePath};
use crate::icon::Icon;
use crate::scroll::ScrollHandle;
use crate::style::{Corners, Cursor, Edges, Effects, Painted, ResolvedText, Ring};
use crate::text::{InlineSlot, Run};

use super::Changes;

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
    /// The offset as laid out and drawn: [`Self::pos`] in pixels.
    pub offset: (f32, f32),
    /// What the offset moves its children by, in whole device pixels
    /// (ADR-0020): their paint space is the container's, this applied as a
    /// transform (ADR-0016), so scrolling records nothing again.
    pub applied: (f32, f32),
    /// The largest offset on each axis.
    pub max: (f32, f32),
    /// The offset exactly ([`crate::compositor::motion`]): what motion
    /// moves it from, on the UI's side and the compositor's.
    pub(crate) pos: crate::compositor::motion::Exact,
    /// The largest offset exactly: [`Self::max`] on the grid.
    pub(crate) end: crate::compositor::motion::Exact,
    /// Colour of the scrollbar thumb: the container's text colour, faded.
    pub thumb: Color,
    /// How many times the UI jumped the offset ([`crate::scroll::Move`]):
    /// another in a commit stops what the compositor moves by itself there
    /// (a fling), never the reader's input (ADR-0032, I4).
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

/// One laid-out node. Its geometry (its boxes, offset, clip and scroll)
/// is layout's to write and read only through a [`Sight`].
#[derive(Clone, Debug, PartialEq)]
pub struct FrameNode {
    pub path: NodePath,
    pub name: Option<&'static str>,
    /// The border box in window pixels: `space` moved by `offset`.
    pub(in crate::layout) bounds: Bounds,
    /// The box inside padding and border, where text is laid out.
    pub(in crate::layout) content_box: Bounds,
    /// The border box in the node's paint space (ADR-0016): where its paint
    /// chunk records it, without the scroll offsets and moving translations
    /// above it, which are transforms of the property tree.
    pub(in crate::layout) space: Bounds,
    /// The content box in the paint space.
    pub(in crate::layout) space_content: Bounds,
    /// What takes the paint space to the window: `bounds` is `space` moved
    /// by it.
    pub(in crate::layout) offset: (f32, f32),
    /// Its translation is moving (a transition or animation): it is in
    /// `offset`, not in `space`.
    pub translating: bool,
    /// The compositor moves its opacity and transform (ADR-0015).
    pub moving: Option<Moving>,
    /// What this node may paint into, when an ancestor clips.
    pub(in crate::layout) clip: Option<Bounds>,
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
    pub(in crate::layout) scroll: Option<Scroll>,
}

/// A reader allowed a laid-out node's geometry (ADR-0015, "One reader of
/// what was shown"): its boxes, its offset, its clip and its scroll, and
/// the frame's viewport are read only through accessors that take one.
/// Each module that reads them holds its own, which only it can make
/// (a field private to it): layout writes them, paint, the compositor,
/// clip, effects, the overlay, accessibility and the devtools read them,
/// and the input side reads them through `interact::shown` alone, so a
/// dispatch that read the frame laid out where the frame shown was meant
/// does not compile. Code outside the engine (tests, tools) holds
/// [`crate::inspect::Outside`]; `tests/shown_reads.rs` checks the input
/// side names none but its own.
pub trait Sight: sight::Sealed + Copy {}

mod sight {
    /// The modules a [`super::Sight`] may be made in, named here alone.
    pub trait Sealed {}
}

/// Seal each reader's token in.
macro_rules! sights {
    ($($t:ty),* $(,)?) => {
        $(
            impl sight::Sealed for $t {}
            impl Sight for $t {}
        )*
    };
}

sights!(
    crate::paint::Sight,
    crate::access::Sight,
    crate::clip::Sight,
    crate::effects::Sight,
    crate::overlay::Sight,
    crate::devtools::Sight,
    crate::inspect::Sight,
    crate::inspect::Outside,
    crate::interact::shown::Sight,
);

impl FrameNode {
    /// The border box in window pixels ([`Sight`]).
    #[must_use]
    pub fn bounds(&self, _: impl Sight) -> Bounds {
        self.bounds
    }

    /// The box inside padding and border ([`Sight`]).
    #[must_use]
    pub fn content_box(&self, _: impl Sight) -> Bounds {
        self.content_box
    }

    /// The border box in the node's paint space ([`Sight`]).
    #[must_use]
    pub fn space(&self, _: impl Sight) -> Bounds {
        self.space
    }

    /// The content box in the paint space ([`Sight`]).
    #[must_use]
    pub fn space_content(&self, _: impl Sight) -> Bounds {
        self.space_content
    }

    /// What takes the paint space to the window ([`Sight`]).
    #[must_use]
    pub fn offset(&self, _: impl Sight) -> (f32, f32) {
        self.offset
    }

    /// What it may paint into, when an ancestor clips ([`Sight`]).
    #[must_use]
    pub fn clip(&self, _: impl Sight) -> Option<Bounds> {
        self.clip
    }

    /// Its scroll, as a scroll container ([`Sight`]).
    #[must_use]
    pub fn scroll(&self, _: impl Sight) -> Option<Scroll> {
        self.scroll
    }

    /// A test's stale or forced clip.
    #[cfg(test)]
    pub(crate) fn set_clip(&mut self, clip: Option<Bounds>) {
        self.clip = clip;
    }

    /// A test's content box moved by `dx` behind layout's back (in the
    /// window and in its paint space).
    #[cfg(test)]
    pub(crate) fn move_content(&mut self, dx: f32) {
        self.content_box.x += dx;
        self.space_content.x += dx;
    }

    /// The node in its paint space with `look`, as its chunk records it:
    /// at its space's place, unclipped and unscrolled (those are the
    /// chunk's state), unmoved, with no content and no parent.
    pub(crate) fn in_space(&self, look: Look) -> Self {
        Self {
            path: self.path.clone(),
            name: self.name,
            bounds: self.space,
            content_box: self.space_content,
            space: self.space,
            space_content: self.space_content,
            offset: (0.0, 0.0),
            translating: false,
            moving: None,
            clip: None,
            clip_radius: Corners::default(),
            clip_curve: crate::shape::ROUND,
            clipper: None,
            layer: self.layer,
            parent: None,
            look,
            content: Content::None,
            scroll: None,
        }
    }
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
    /// Its key ([`crate::element::Span::key`]), if it has one.
    pub(crate) key: Option<crate::element::ElementKey>,
    pub(crate) range: std::ops::Range<usize>,
    pub(crate) click: Option<Handler<ClickEvent>>,
    pub(crate) cursor: Option<Cursor>,
    /// Where its text lies in the window, one box per line it takes.
    pub(crate) boxes: Vec<Bounds>,
}

impl SpanTarget {
    /// What it is across builds: its key, or its place.
    pub(crate) fn id(&self) -> crate::compositor::SpanId {
        match self.key {
            Some(k) => crate::compositor::SpanId::Keyed(k),
            None => crate::compositor::SpanId::At(u32::try_from(self.index).unwrap_or(u32::MAX)),
        }
    }
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
    pub(in crate::layout) viewport: Bounds,
    /// The device pixels to the logical one it was laid out at
    /// ([`crate::layout::UiState::scale`]; zero when not known, logical pixels taken
    /// for device ones): hit testing keeps a device pixel of a clip as
    /// paint does.
    pub scale: f32,
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

impl Frame {
    /// The window's box it was laid out in ([`Sight`]).
    #[must_use]
    pub fn viewport(&self, _: impl Sight) -> Bounds {
        self.viewport
    }

    pub(crate) fn behaviour(&self, i: usize) -> Option<&Behaviour> {
        self.behaviour.get(i).map(|b| &**b)
    }

    /// Whether node `i` takes the pointer, the one rule the compositor's
    /// hit table and the tools that name what is hit both read: it lets
    /// no pointer through (not `pass_through`, which inert and
    /// `pass_through_all` imply) and is drawn (not `look.invisible`).
    #[must_use]
    pub(crate) fn takes_pointer(&self, i: usize) -> bool {
        self.behaviour(i).is_some_and(|b| !b.pass_through) && self.nodes.get(i).is_some_and(|n| !n.look.invisible)
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
