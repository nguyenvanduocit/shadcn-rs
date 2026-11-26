//! The one reader of what the screen showed ([`Shown`]): every point an
//! event carries, every box and text it is hit in and every place
//! something is put on screen by (the cursor, an input method's window)
//! is read through it, off one frame the compositor presented, never off
//! the frame laid out since (ADR-0015, "one frame per question").
//!
//! Why one type: rounds 9 to 11 of the clip review each found a payload
//! or a lookup that read the frame laid out where the frame shown was
//! meant (a click's bounds, a removed node's cursor, the input method's
//! caret, a text area's scroll copied at paint): every rule written next
//! to the code it governed was missed somewhere else. Here the geometry
//! of the frame shown has no other way out: what maps a point
//! ([`Anchor`], [`Placing`]) and the index it is found by ([`HitKeys`])
//! are private to this module, and the input side's code reads a node's
//! box, its text and its scroll only by asking a [`Shown`]; the source
//! check `tests/shown_reads.rs` fails a dispatch path that reads the
//! frame laid out's geometry itself. The frame laid out answers only
//! what has no geometry: a node's identity and its behaviour (handlers,
//! fields, spans by identity), as Blink keeps the hit result's geometry
//! with the hit (`HitTestResult::SetNodeAndPosition`) and asks the node
//! only for what it does.

use crate::compositor::{Hit, HitTarget, NodeKey, Presented, ShownText, SpanId};
use crate::geometry::Bounds;
use crate::layout::{Content, Frame, FrameNode, Scroll};
use crate::style::{Cursor, ResolvedText};
use crate::text::TextSystem;

use super::{node_of, owner_of, visited};

/// The input side's reader of laid-out geometry ([`crate::layout::Sight`]),
/// made here alone: what `interact.rs` reads of the frame laid out, it
/// reads through this module, by [`Shown`] or by one of the `laid_`
/// readers below, each named for the model it serves.
#[derive(Clone, Copy)]
pub(crate) struct Sight(());

/// The input side's [`Sight`].
const SIGHT: Sight = Sight(());

/// A node's scroll as laid out: the scroll model (the offset and range
/// the UI holds, which wheels, autoscroll and the compositor's deltas
/// step, and which nodes are scroll containers). Never where a point
/// lands: that is [`Shown`]'s.
pub(super) fn laid_scroll(node: &FrameNode) -> Option<Scroll> {
    node.scroll(SIGHT)
}

/// A field's content box as laid out: keyboard editing and the field's
/// scroll model work in it (a line up or down is the text laid out now).
pub(super) fn laid_content_box(node: &FrameNode) -> Bounds {
    node.content_box(SIGHT)
}

/// A node's box as laid out: what scrolling it into view brings into its
/// containers' scroll model, and the devtools' pick before any frame was
/// presented.
pub(super) fn laid_bounds(node: &FrameNode) -> Bounds {
    node.bounds(SIGHT)
}

/// A hit table's boxes by node key, sorted: the box a node was shown as is
/// found by a binary search ([`Anchor::of`]). Made once per hit table
/// (shared by every frame a scroll or an animation composed of the same
/// commit), its buffer kept.
#[derive(Default)]
pub(super) struct HitKeys {
    /// The table it was made of.
    table: Option<std::sync::Arc<[Hit]>>,
    /// Each box's key, and its place in the table.
    keys: Vec<(NodeKey, u32)>,
}

impl HitKeys {
    /// Index `hits`, unless it is the table indexed already.
    pub(super) fn fill(&mut self, hits: &std::sync::Arc<[Hit]>) {
        if self.table.as_ref().is_some_and(|t| std::sync::Arc::ptr_eq(t, hits)) {
            return;
        }
        self.keys.clear();
        self.keys.extend(
            hits.iter()
                .enumerate()
                .filter_map(|(i, h)| Some((h.target.key, u32::try_from(i).ok()?))),
        );
        self.keys.sort_unstable_by_key(|e| e.0);
        self.table = Some(std::sync::Arc::clone(hits));
    }

    /// The box of `hits` that node `i` of `frame` was shown as: by its key
    /// and the instance that made it.
    fn find<'a>(&self, hits: &'a [Hit], frame: &Frame, i: usize) -> Option<(u32, &'a Hit)> {
        debug_assert!(
            self.table.as_deref().is_some_and(|t| std::ptr::eq(t, hits)),
            "asked of the table it indexed"
        );
        let key = NodeKey::of(frame.nodes.get(i)?.id);
        let owner = owner_of(frame, i);
        let from = self.keys.partition_point(|e| e.0 < key);
        self.keys
            .get(from..)?
            .iter()
            .take_while(|e| e.0 == key)
            .filter_map(|e| Some((e.1, hits.get(usize::try_from(e.1).ok()?)?)))
            .inspect(|(_, h)| debug_assert_eq!(h.target.key, key, "the index pairs each key with its own box"))
            .find(|(_, h)| h.target.owner == owner)
    }
}

/// Where an event point lies for a node, as the frame the event was
/// routed in showed it: the one rule every event's point is mapped by (a
/// node's own point, an inline span's, a text offset's, a drag's), read
/// off that frame's property trees (its transform's offset as the
/// compositor scrolled and moved it, every scale and turn above it as
/// composed then) and the box the frame showed the node as, never the
/// frame laid out since. Blink records the local point of a hit in the
/// same walk, over the same geometry, as it picks the node
/// (`HitTestResult::SetNodeAndPosition`, `hit_test_result.cc:210-219`),
/// and maps between spaces through one mapper over the property trees
/// (`GeometryMapper::SourceToDestinationProjection`,
/// `geometry_mapper.h:43`).
#[derive(Clone, Copy)]
struct Anchor<'a> {
    props: &'a crate::scene::Properties,
    /// Device pixels to the logical one.
    scale: f32,
    /// The box shown: its transform and its rectangle in that transform's
    /// space, before the offset.
    hit: &'a Hit,
    /// The box is the node's own (not an ancestor's it is carried with).
    own: bool,
    /// Where the node the box belongs to lies in the frame laid out: the
    /// origin an event's point is given from (a point is the node's
    /// identity there, its geometry the frame shown's).
    origin: (f32, f32),
    /// The text the node asked about showed input in the box, when the
    /// box is its own ([`ShownText`]): what a span or a text offset is hit
    /// in, as the frame showed it.
    text: Option<&'a std::sync::Arc<ShownText>>,
}

impl<'a> Anchor<'a> {
    /// Node `i` of `frame` as `shown` showed it: its own box, or that of
    /// its nearest ancestor that took the pointer (a node that takes none
    /// has no box shown: it goes where that ancestor goes). `None` when
    /// none of its chain showed one.
    fn of(shown: &'a Presented, keys: &HitKeys, frame: &Frame, i: usize) -> Option<Self> {
        let mut at = Some(i);
        while let Some(k) = at {
            visited();
            if let Some((place, hit)) = keys.find(&shown.hits, frame, k) {
                let b = frame.nodes.get(k).map(|n| n.bounds(SIGHT)).unwrap_or_default();
                let text = (k == i)
                    .then(|| {
                        let at = shown.texts.binary_search_by_key(&place, |e| e.0).ok()?;
                        shown.texts.get(at).map(|e| &e.1)
                    })
                    .flatten();
                return Some(Self {
                    props: &shown.properties,
                    scale: if shown.scale > 0.0 { shown.scale } else { 1.0 },
                    hit,
                    own: k == i,
                    origin: (b.x, b.y),
                    text,
                });
            }
            at = frame.nodes.get(k).and_then(|n| n.parent);
        }
        None
    }

    /// The scales and turns above the box, the innermost first (physical
    /// pixels), as the renderer composes them; `None` for none.
    fn matrix(&self) -> Option<[f32; 6]> {
        self.props
            .matrices(self.hit.transform, 0)
            .map(|g| g.matrix())
            .reduce(crate::gpu::path::then)
    }

    /// Where the box's corner lies before any scale or turn (logical).
    fn corner(&self) -> (f32, f32) {
        let off = self.props.transform(self.hit.transform).offset;
        (self.hit.rect.x + off.0, self.hit.rect.y + off.1)
    }

    /// `p` on screen (logical) from the box's corner, every scale and
    /// turn above it undone; `None` under a scale of 0.
    fn local(&self, (x, y): (f32, f32)) -> Option<(f32, f32)> {
        let s = self.scale;
        let q = match self.matrix() {
            Some(m) => {
                let q = crate::gpu::path::apply(crate::gpu::path::invert(m)?, [x * s, y * s]);
                (q[0] / s, q[1] / s)
            }
            None => (x, y),
        };
        let c = self.corner();
        Some((q.0 - c.0, q.1 - c.1))
    }

    /// Where `local` (from the box's corner) shows on screen: [`Self::local`]
    /// undone (on the way back from the frame laid out, [`Placing::shown_at`]).
    fn screen(&self, (x, y): (f32, f32)) -> (f32, f32) {
        let s = self.scale;
        let c = self.corner();
        let q = (x + c.0, y + c.1);
        match self.matrix() {
            Some(m) => {
                let p = crate::gpu::path::apply(m, [q.0 * s, q.1 * s]);
                (p[0] / s, p[1] / s)
            }
            None => q,
        }
    }
}

/// How an event point maps to where the frame laid out has it for a node
/// ([`Shown::to_node`]): through the box the frame shown showed it as,
/// or, when the frame laid out is the one shown, through its own scales
/// and turns (`None`: none); `Nowhere`: a frame was shown and no box of
/// the node's chain was in it (made since): it lies where it was laid
/// out, and nothing of its own (a span, a text offset) is hit.
#[derive(Clone, Copy)]
enum Placing<'a> {
    Shown(Anchor<'a>),
    Laid(Option<[f32; 6]>),
    Nowhere,
}

impl Placing<'_> {
    /// `p` on screen in the frame laid out; `None` under a scale of 0.
    fn laid_at(self, p: (f32, f32)) -> Option<(f32, f32)> {
        match self {
            Self::Shown(a) => a.local(p).map(|(x, y)| (a.origin.0 + x, a.origin.1 + y)),
            Self::Laid(None) | Self::Nowhere => Some(p),
            Self::Laid(Some(m)) => {
                let q = crate::gpu::path::apply(crate::gpu::path::invert(m)?, [p.0, p.1]);
                Some((q[0], q[1]))
            }
        }
    }

    /// Where `q` of the frame laid out shows on screen: [`Self::laid_at`]
    /// undone.
    fn shown_at(self, q: (f32, f32)) -> (f32, f32) {
        match self {
            Self::Shown(a) => a.screen((q.0 - a.origin.0, q.1 - a.origin.1)),
            Self::Laid(None) | Self::Nowhere => q,
            Self::Laid(Some(m)) => {
                let p = crate::gpu::path::apply(m, [q.0, q.1]);
                (p[0], p[1])
            }
        }
    }
}

/// The scales and turns node `i` of `frame` and its ancestors are painted
/// at, each about its origin, the innermost first (logical pixels): the
/// frame laid out as paint draws it; `None` for none.
fn laid_matrix(frame: &Frame, i: usize) -> Option<[f32; 6]> {
    let mut m: Option<[f32; 6]> = None;
    let mut at = Some(i);
    while let Some(k) = at {
        visited();
        let Some(n) = frame.nodes.get(k) else { break };
        let t = n.look.transform;
        if !t.is_identity() {
            let g = crate::scene::GroupTransform {
                origin: (n.bounds(SIGHT).x + t.origin.0, n.bounds(SIGHT).y + t.origin.1),
                scale: t.scale,
                rotate: t.rotate,
            };
            m = Some(m.map_or_else(|| g.matrix(), |inner| crate::gpu::path::then(inner, g.matrix())));
        }
        at = n.parent;
    }
    m
}

crate::engine::engine_local! {
    /// The text system the frames no renderer presented are painted with
    /// for hit testing ([`painted`]): a cache, made at first use.
    [Fresh] static PAINT_TEXT: std::cell::RefCell<Option<TextSystem>> = const { std::cell::RefCell::new(None) };
}

/// What input to `frame` is hit-tested against before any frame of it
/// was presented: its boxes and property trees as paint makes them from
/// nothing ([`crate::paint::Painter::fresh`]), at its scale, no layer
/// drawn (none was), so the canvas's pixels decide
/// ([`Presented::of`]).
pub(super) fn painted(frame: &Frame) -> Presented {
    let s = if frame.scale > 0.0 { frame.scale } else { 1.0 };
    let v = frame.viewport(SIGHT);
    let size = (
        crate::text::physical(v.right().max(1.0), s),
        crate::text::physical(v.bottom().max(1.0), s),
    );
    PAINT_TEXT.with(|t| {
        let mut t = t.borrow_mut();
        let text = t.get_or_insert_with(TextSystem::new);
        Presented::of(&crate::paint::Painter::fresh(frame, text, size, s), 0)
    })
}

/// The node of `frame`, from `floor` on, that `point` hits in the frame
/// `presented` showed ([`crate::clip::hit_in`]): what the screen showed
/// there decides, all of one frame (its boxes, its clips as composed and
/// the layers it was drawn with), and the node is the one `frame` has for
/// the target hit, by path and instance. A node gone since, made again by
/// another instance, or now below the top modal layer, takes nothing.
/// `past_modal`: the box that shut what lay below the top modal layer of
/// `presented` is passed over (a secondary press that closed it).
pub(super) fn shown_node(
    frame: &Frame,
    presented: &Presented,
    point: (f32, f32),
    floor: usize,
    past_modal: bool,
) -> Option<usize> {
    if past_modal {
        let blocker = NodeKey(0);
        let target = crate::clip::hit_in(presented, point, |t: HitTarget| {
            t.key != blocker && node_of(frame, t).is_some_and(|i| i >= floor)
        })?;
        return node_of(frame, target);
    }
    let target = crate::clip::hit_in(presented, point, |_| true)?;
    node_of(frame, target).filter(|&i| i >= floor)
}

/// A span hit ([`Shown::span_at`]): the frame showed a span there, and
/// `live` is the span laid out now that it is (by identity), `None` once
/// gone. (Its cursor is the frame's, read with no span laid out:
/// [`Shown::cursor_at`].)
pub(super) struct ShownSpanAt<'f> {
    pub(super) live: Option<&'f crate::layout::SpanTarget>,
}

/// A text field as a frame showed it ([`Shown::field`]): what a point is
/// hit in for a text offset, and where its caret was, read off that
/// frame (its record, [`ShownText::Field`], and its property trees for
/// the scroll), set where the frame laid out has the field.
pub(super) struct ShownField {
    /// The record it was read from, `None` for the frame laid out: what
    /// was laid out of it is the same while the record is (it is shared,
    /// unchanged, by every frame that shows it the same), so what is
    /// worked out of its text alone is kept by it ([`super::ImeText`]).
    /// Held weakly, so the identity outlives no frame, and an address is
    /// never taken for another record's.
    pub(super) record: Option<std::sync::Weak<ShownText>>,
    pub(super) content: std::sync::Arc<str>,
    pub(super) text: ResolvedText,
    /// Its content box where the frame laid out has the field.
    pub(super) area: Bounds,
    pub(super) multiline: bool,
    /// Shown focused (its text not cut short).
    pub(super) focused: bool,
    /// How far its content showed scrolled.
    pub(super) scrolled: (f32, f32),
    /// How an offset in `content` falls in the value it showed.
    pub(super) offsets: crate::edit::Offsets,
    pub(super) caret: usize,
    pub(super) selection: (usize, usize),
    pub(super) preedit: Option<(usize, usize)>,
}

/// What the screen showed, as one frame: the frame input is routed in
/// for an event, the frame on screen for hover, the cursor and the input
/// method; with no frame shown to the input side (the devtools' frame as
/// laid out), the frame laid out as paint draws it. The only way the
/// input side reads geometry and presentation (module docs).
#[derive(Clone, Copy)]
pub(super) struct Shown<'a> {
    /// The frame laid out: identities and behaviour only.
    frame: &'a Frame,
    presented: Option<&'a Presented>,
    keys: &'a HitKeys,
}

impl<'a> Shown<'a> {
    pub(super) fn new(frame: &'a Frame, presented: Option<&'a Presented>, keys: &'a HitKeys) -> Self {
        Self { frame, presented, keys }
    }

    /// Node `i` as the frame showed it ([`Anchor::of`]); with none shown
    /// (the devtools' frame as laid out), the frame laid out is the one
    /// shown, as paint draws it ([`Placing::Laid`]). Its cost is the
    /// node's depth, never a walk of the frame.
    fn place(&self, i: usize) -> Placing<'a> {
        match self.presented {
            Some(p) => Anchor::of(p, self.keys, self.frame, i).map_or(Placing::Nowhere, Placing::Shown),
            None => Placing::Laid(laid_matrix(self.frame, i)),
        }
    }

    /// The node at `(x, y)` on screen from `floor` on, by the one hit rule
    /// ([`shown_node`]): in what the frame showed, or, with none shown, in
    /// the frame as paint makes it ([`painted`]).
    pub(super) fn hit(&self, floor: usize, past_modal: bool, p: (f32, f32)) -> Option<usize> {
        match self.presented {
            Some(s) => shown_node(self.frame, s, p, floor, past_modal),
            None => shown_node(self.frame, &painted(self.frame), p, floor, past_modal),
        }
    }

    /// `p` on screen where the frame laid out has it for node `i`: the
    /// point from the corner of the box the frame showed the node as, in
    /// that frame's property trees ([`Anchor::local`]), set at the node's
    /// corner as laid out ([`Placing::laid_at`]). The one map of every
    /// event's point (the node's, a span's, a text offset's, a drag's). A
    /// node no box of its chain was shown for lies where it was laid out.
    /// Under a scale of 0, where nothing maps, the point with only the
    /// offset the frame showed the box at taken off.
    pub(super) fn to_node(self, i: usize, p: (f32, f32)) -> (f32, f32) {
        Self::point_in(self.place(i), p)
    }

    /// What an event at `p` on screen tells node `i`: the point
    /// ([`Self::to_node`]) and the box it is in ([`Self::bounds`]), both
    /// off one lookup of the box shown.
    pub(super) fn event(&self, i: usize, p: (f32, f32)) -> ((f32, f32), Bounds) {
        let place = self.place(i);
        (Self::point_in(place, p), self.box_of(i, place))
    }

    /// [`Self::to_node`] through `placing`.
    fn point_in(placing: Placing, p: (f32, f32)) -> (f32, f32) {
        placing.laid_at(p).unwrap_or_else(|| match placing {
            Placing::Shown(a) => {
                let c = a.corner();
                (a.origin.0 + p.0 - c.0, a.origin.1 + p.1 - c.1)
            }
            Placing::Laid(_) | Placing::Nowhere => p,
        })
    }

    /// The box around rectangle `b` of node `i`'s space (as laid out) as
    /// it shows on screen: [`Self::to_node`] undone, in the same frame's
    /// property trees (moved, scaled and turned as drawn), so what is
    /// placed by it lands where it is drawn.
    pub(super) fn rect_on_screen(&self, i: usize, b: Bounds) -> Bounds {
        let place = self.place(i);
        let corners =
            [(b.x, b.y), (b.right(), b.y), (b.x, b.bottom()), (b.right(), b.bottom())].map(|p| place.shown_at(p));
        let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        for (x, y) in corners {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        Bounds {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        }
    }

    /// `p` on screen in the own space of node `i`: from its box's corner
    /// as the frame showed it (scroll offsets and translations above it
    /// included, as the compositor showed them, and as scaled and
    /// turned); `None` under a scale of 0.
    pub(super) fn local(&self, i: usize, p: (f32, f32)) -> Option<(f32, f32)> {
        let place = self.place(i);
        let (fx, fy) = place.laid_at(p)?;
        let b = self.box_of(i, place);
        Some((fx - b.x, fy - b.y))
    }

    /// The box an event names node `i` by: the box the frame showed it as
    /// (its size), where the frame laid out has the node (the origin its
    /// points are given from, [`Self::to_node`]), so a point and the box
    /// it is in are one frame's. A node that showed no box of its own
    /// (it takes no pointer, or was made since) has its box as laid out.
    pub(super) fn bounds(&self, i: usize) -> Bounds {
        self.box_of(i, self.place(i))
    }

    /// [`Self::bounds`] through `placing`, node `i`'s.
    fn box_of(&self, i: usize, placing: Placing) -> Bounds {
        let laid = self.frame.nodes.get(i).map(|n| n.bounds(SIGHT)).unwrap_or_default();
        match placing {
            Placing::Shown(a) if a.own => Bounds {
                x: a.origin.0,
                y: a.origin.1,
                width: a.hit.rect.width,
                height: a.hit.rect.height,
            },
            _ => laid,
        }
    }

    /// The span of node `i` that takes the pointer at `p` (where the frame
    /// laid out has it, [`Self::to_node`]): hit in the spans' text as the
    /// frame showed it ([`Anchor::text`]), so a span laid out since
    /// elsewhere is not taken for the one shown there. What was shown is
    /// that frame's (which span, its cursor), read without the node's
    /// behaviour; only the handler is the frame laid out's, found by the
    /// span's identity ([`SpanId`]): a span gone since has none. With the
    /// frame laid out the one shown, in its own boxes.
    pub(super) fn span_at(&self, i: usize, p: (f32, f32)) -> Option<ShownSpanAt<'a>> {
        match self.place(i) {
            Placing::Shown(a) => {
                let Some(ShownText::Spans(boxes)) = a.text.map(|t| &**t) else {
                    return None;
                };
                let s = boxes
                    .iter()
                    .find(|s| s.rect.contains(p.0 - a.origin.0, p.1 - a.origin.1))?;
                Some(ShownSpanAt {
                    live: self.live_span(i, s.span),
                })
            }
            Placing::Laid(_) => self
                .frame
                .behaviour(i)?
                .span_at(p.0, p.1)
                .map(|t| ShownSpanAt { live: Some(t) }),
            Placing::Nowhere => None,
        }
    }

    /// Span `id` of node `i` as laid out now, if it is still there.
    fn live_span(&self, i: usize, id: SpanId) -> Option<&'a crate::layout::SpanTarget> {
        self.frame.behaviour(i)?.spans.iter().find(|t| t.id() == id)
    }

    /// The cursor the screen shows at `p`: the one the frame showed there
    /// ([`crate::compositor::under`]: its box's, or a span's over its
    /// text), a value of that frame as its pixels are, whether or not the
    /// node is still laid out; below its top modal layer, or over
    /// nothing, the default.
    pub(super) fn cursor_at(&self, p: (f32, f32)) -> Cursor {
        let under = match self.presented {
            Some(s) => crate::compositor::under(s, p),
            None => crate::compositor::under(&painted(self.frame), p),
        };
        under.filter(|u| !u.2).and_then(|u| u.3).unwrap_or_default()
    }

    /// The cursor node `i` shows (a drag's, wherever the pointer is): as
    /// the frame showed it over the node's box ([`Hit::cursor`]); with
    /// none shown, or no box of the node shown, the frame laid out's (its
    /// own or its nearest ancestor's).
    pub(super) fn cursor_of(&self, i: usize) -> Option<Cursor> {
        if let Some(shown) = self.presented
            && let Some((_, hit)) = self.keys.find(&shown.hits, self.frame, i)
        {
            return hit.cursor;
        }
        self.frame
            .ancestors(i)
            .find_map(|i| self.frame.behaviour(i).and_then(|b| b.cursor))
    }

    /// Text field `i` as the frame showed it: its text, type, box, scroll
    /// (off the frame's property trees: as the compositor scrolled it),
    /// offsets and caret. With no frame shown, as laid out (paint draws
    /// it so). `None` for a node that is no field, or a field the frame
    /// showed no text of (made since): it is placed nothing.
    pub(super) fn field(&self, i: usize) -> Option<ShownField> {
        match self.place(i) {
            Placing::Shown(a) => {
                let t = a.text?;
                let ShownText::Field {
                    content,
                    text,
                    area,
                    multiline,
                    focused,
                    offsets,
                    caret,
                    selection,
                    preedit,
                    ..
                } = &**t
                else {
                    return None;
                };
                Some(ShownField {
                    record: Some(std::sync::Arc::downgrade(t)),
                    content: std::sync::Arc::clone(content),
                    text: text.clone(),
                    area: Bounds {
                        x: a.origin.0 + area.x,
                        y: a.origin.1 + area.y,
                        ..*area
                    },
                    multiline: *multiline,
                    focused: *focused,
                    scrolled: t.scrolled(a.props, a.hit.transform),
                    offsets: *offsets,
                    caret: *caret,
                    selection: *selection,
                    preedit: *preedit,
                })
            }
            Placing::Laid(_) => {
                let node = self.frame.nodes.get(i)?;
                let Content::Input {
                    edit,
                    text,
                    focused,
                    multiline,
                    preedit,
                    ..
                } = &node.content
                else {
                    return None;
                };
                Some(ShownField {
                    record: None,
                    content: std::sync::Arc::clone(&edit.text),
                    text: text.clone(),
                    area: node.content_box(SIGHT),
                    multiline: *multiline,
                    focused: *focused,
                    scrolled: node.scroll(SIGHT).map_or((0.0, 0.0), |s| s.applied),
                    offsets: edit.offsets(*preedit),
                    caret: edit.caret,
                    selection: edit.selection(),
                    preedit: *preedit,
                })
            }
            Placing::Nowhere => None,
        }
    }
}
