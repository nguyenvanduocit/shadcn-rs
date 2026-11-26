//! The compositor (ADR-0015): what the render thread keeps of the UI's
//! frames and moves by itself between them.
//!
//! The UI thread lays out and records a [`Scene`] and sends the render
//! thread a [`Commit`]: only what changed since the last one, behind `Arc`s
//! (chunks are `Arc<Display>`, the scroll, hit and motion tables are
//! `Arc<[_]>`), applied whole. The [`Compositor`] keeps the scene as
//! committed and, at each frame's present time, composes the scene to draw:
//! the committed property trees with what it moves itself written over
//! them, scroll offsets (the wheel, its prediction and fling, synced back to
//! the UI as Chromium's `SyncedProperty` is) and opacity and transform
//! motion ([`crate::motion::Curves`]). A frame the UI did not commit in time
//! still moves those: the compositor-only frame of cc's high-latency mode.
//!
//! What it showed is published as a [`Presented`]; each input event carries
//! the one current when it arrived, and the UI hit-tests through it, so a
//! click lands on what was on screen while the compositor scrolled.
//!
//! The same code runs headless in one thread ([`Headless`], under
//! [`crate::Scenario`]): commit, compose and draw at the scenario's clock,
//! in order, so a scripted run is the same every time.

mod compose;
pub mod input_log;
mod latch;
pub(crate) mod motion;
mod outbox;
#[cfg(test)]
mod replay;
mod scroll;
pub(crate) mod thread;

use std::sync::Arc;

pub(crate) use latch::WheelGesture;
pub use latch::{LATCH_SLOP, WHEEL_LATCH, WheelPhase, WheelStep};
pub use outbox::{Outbox, Pulled};
pub(crate) use scroll::{GestureMotion, wheel_moves, wheel_takes};

pub use self::scroll::{Fling, ScrollPredictor};
use self::scroll::{Scrolls, WheelTarget};
pub use self::thread::FrameStat;
use crate::geometry::Bounds;
use crate::gpu::GpuRenderer;
use crate::image::Pixels;
use crate::layout::Moving;
use crate::scene::{
    Changed, Chunk, ChunkId, ClipNode, Drawn, EffectNode, GroupTransform, Held, Properties, Renderer, Scene,
    TransformNode,
};
use crate::text::{GlyphUpdate, Glyphs};

/// A laid-out node across frames, by its path (what the painter keys its
/// property tree nodes by).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeKey(pub u64);

impl NodeKey {
    #[must_use]
    pub fn of(path: &crate::geometry::NodePath) -> Self {
        Self(path.key())
    }
}

/// A scroll container as the compositor scrolls it.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollNode {
    pub key: NodeKey,
    /// The component instance whose output it is: with [`Self::key`], its
    /// identity as the UI names it (`ScrollId`), which every piece of
    /// scroll state and every message about it is keyed by.
    pub(crate) owner: Option<crate::reactive::OwnerId>,
    /// Which scroll container it is: given as it first shows, a new one
    /// when its key leaves and comes back. A gesture, its latch and its
    /// fling are the instance's.
    pub instance: u64,
    /// The transform its children paint under: the scroll offset is its offset.
    pub content: u32,
    /// The offset as laid out, within `0..=max`, as drawn: [`Self::pos`]
    /// in pixels.
    pub offset: (f32, f32),
    /// What the offset moved the children by, in whole device pixels and
    /// signed as paint applies it (ADR-0020).
    pub applied: (f32, f32),
    pub max: (f32, f32),
    /// The offset exactly ([`motion`]): what the compositor moves it from.
    pub(crate) pos: motion::Exact,
    /// The largest offset exactly: [`Self::max`] on the grid.
    pub(crate) end: motion::Exact,
    /// Its offsets run from the right.
    pub rtl: bool,
    /// Bumped by each jump the program made ([`crate::scroll::Move::Jump`]):
    /// a commit with another than the last stops what the compositor moves
    /// by itself there (a fling), as Chromium's clobbering commit does
    /// (ADR-0032, I4). A count, compared only with itself.
    pub generation: u64,
    pub thumb: Option<ThumbNode>,
    /// The scroll container around it, by index in [`Compositing::scrollers`].
    pub parent: Option<u32>,
    /// It has a wheel handler of its own: the UI is asked before it scrolls.
    pub wheel: bool,
    /// A node between it and [`Self::parent`] has one: the UI is asked
    /// when it cannot scroll that way.
    pub wheel_above: bool,
    /// It is below the top modal layer: no wheel reaches it, a gesture
    /// latched to it before that layer opened included (the UI's rule).
    pub below_modal: bool,
}

/// A scrollbar thumb the compositor moves with its scroll offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThumbNode {
    pub transform: u32,
    /// Its opacity: 0 as committed, raised while the user scrolls.
    pub effect: u32,
    /// The thumb's offset at the top.
    pub top: (f32, f32),
    /// How far it travels from the top to the bottom.
    pub travel: f32,
}

/// A box that takes the pointer, in painting order: what the compositor
/// hit-tests a wheel event against, by the one rule the UI hit-tests by
/// ([`crate::clip::hit_in`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Its border box in its paint space, its shape and its hit slop: it
    /// takes the pointer as the UI's node does
    /// ([`crate::shape::takes_pointer`]).
    pub rect: Bounds,
    pub shape: crate::shape::Shape,
    pub slop: crate::style::Edges<f32>,
    pub transform: u32,
    /// The clip it is hit inside, 0 for none.
    pub clip: u32,
    /// The effect it paints under, 0 for none: the layers it is drawn
    /// through ([`crate::scene::LayerStage`]).
    pub effect: u32,
    /// The scroll container it is in (itself, when it is one), by index.
    pub scroller: Option<u32>,
    /// A wheel handler is on it or on an ancestor below [`Self::scroller`]:
    /// the wheel over it goes to the UI, which may consume it.
    pub wheel: bool,
    /// Which node it is: what the UI hands a wheel over it to.
    pub target: HitTarget,
    /// The cursor it shows: its node's, or the nearest ancestor's with one
    /// (a span's over its text: [`ShownSpan::cursor`]). A value the frame
    /// shows, as its pixels are: a cursor changed shows with its frame.
    pub cursor: Option<crate::style::Cursor>,
    /// It is below the top modal layer: a gesture latched to it before
    /// that layer opened is routed afresh (the UI's rule).
    pub below_modal: bool,
}

/// What a wheel gesture goes to: the UI (a handler may consume it) or the
/// compositor, scrolling a container itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Latched {
    /// The UI, with the node the gesture's first event was over.
    Ui(Option<HitTarget>),
    /// A scroll container, the instance it is ([`ScrollNode::instance`]).
    Scroll(u64),
}

/// Everything the compositor holds of input it accepted (ADR-0033,
/// "Accepted motion"), carried whole to a compositor made afresh after a
/// panic ([`Compositor::after`]), so no input it took is lost or changed:
///
/// - what it scrolled on its way to the UI (the outbox), its gesture's
///   resampling settled into it first ([`Compositor::settle_gesture`]);
/// - the fling under way, as data (a function of time), which the next
///   compositor carries on by the time of its frames;
/// - whether it flings trackpad gestures (a replayed log's platform).
///
/// The rest it holds of input is not its to carry: the gesture's latch
/// went to the UI with the input ([`HandedGesture`]); a scrollbar's
/// fading and a container's speed are shown state, made again by the
/// next motion; the scene comes again whole from the UI. Built by
/// [`Compositor::into_accepted`], which names every field of the
/// compositor and of its scroll state, so a field added there is a
/// compile error until it is carried or said to be shown state.
#[derive(Debug, Default)]
pub struct Accepted {
    outbox: Outbox,
    fling: Option<scroll::Flinging>,
    flings: bool,
}

impl Accepted {
    fn new(outbox: Outbox, fling: Option<scroll::Flinging>, flings: bool) -> Self {
        Self { outbox, fling, flings }
    }
}

/// The wheel gesture under way, its target container and the containers
/// it tells apart named as the UI names them: the one latch, handed with
/// the route (ADR-0033, r12). Exactly one side holds a gesture's latch at
/// a time: the compositor hands it to the UI and keeps none as input is
/// diverted ([`Compositor::hand_gesture`]); the UI hands it back as input
/// comes through the compositor again ([`Compositor::take_gesture`]),
/// with whether the UI keeps it (`.1`: a wheel handler on the container
/// or between it and the one around it, which the UI asks first, as the
/// compositor routes such a gesture to the UI), and its motion so far
/// (`.2`: its recent events, its lift), so its fling is the same whichever
/// side took its events or its lift (round 13, the fault matrix: a lift
/// the UI took while input was diverted flung nothing).
#[derive(Clone, Debug)]
pub struct HandedGesture(
    pub(crate) WheelGesture<HitTarget>,
    pub(crate) bool,
    pub(crate) Option<scroll::GestureMotion>,
);

/// Where a gesture's first event goes, and the innermost scroll container
/// under it.
#[derive(Clone, Copy, Debug)]
struct Routed {
    to: Option<Latched>,
    under: Option<u64>,
}

/// A wheel event from the platform, as the render thread takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelInput {
    /// Where, logical window pixels.
    pub point: (f32, f32),
    /// How far: positive moves the content up and toward the inline start.
    pub delta: (f32, f32),
    /// When, nanoseconds on the frame clock ([`crate::clock::nanos`]).
    pub time: u64,
    /// A trackpad's, resampled; else a notched wheel's, applied.
    pub precise: bool,
    /// Its phase in the platform's gesture.
    pub phase: WheelPhase,
    /// The frame shown as it came ([`Shown::seen`]).
    pub seen: Seen,
}

impl WheelInput {
    /// Its place in its gesture.
    #[must_use]
    pub fn step(&self) -> WheelStep {
        WheelStep {
            point: self.point,
            time: self.time,
            precise: self.precise,
            phase: self.phase,
            gesture: None,
        }
    }
}

/// What a wheel event became.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wheeled {
    /// Handed to the UI: it is over a wheel handler, the node it hands it
    /// to with it, and the step in the gesture the compositor found it in
    /// ([`WheelStep::gesture`]), which the UI's latch follows.
    Ui(Option<HitTarget>, WheelStep),
    /// The compositor scrolls a container.
    Scrolled,
    Nothing,
}

/// What the render thread does with a platform wheel event
/// ([`Compositor::dispatch_wheel`]): the one dispatch the live render
/// thread, a scenario and a replayed input log all run.
#[derive(Clone, Debug, PartialEq)]
pub enum Dispatched {
    /// The UI's input to run in the frame shown as the event came: the
    /// compositor handed it on ([`Wheeled::Ui`]).
    Ui(crate::window::Input),
    /// The compositor scrolled: it draws, and the UI takes the scroll in
    /// with its next main frame.
    Scrolled,
    Nothing,
}

/// The node a wheel is routed to, as a frame showed it: its compositor key
/// and the component instance that made it ([`crate::interact`]'s node
/// identity). The UI delivers the wheel to that node while it is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HitTarget {
    pub(crate) key: NodeKey,
    pub(crate) owner: Option<crate::reactive::OwnerId>,
}

/// A node whose opacity and transform the compositor moves.
#[derive(Clone, Debug, PartialEq)]
pub struct MovingNode {
    pub key: NodeKey,
    /// Its own transform node, and its effect node, when it has them.
    pub transform: Option<u32>,
    pub effect: Option<u32>,
    pub moving: Moving,
    /// Its border box in its paint space.
    pub space: Bounds,
    pub invisible: bool,
    /// Whether its matrix is drawn whatever its value (a scale or turn moves).
    pub turns: bool,
}

/// What the compositor moves by itself, as the painter left it.
#[derive(Clone, Debug, PartialEq)]
pub struct Compositing {
    pub scrollers: Arc<[ScrollNode]>,
    pub hits: Arc<[Hit]>,
    pub moving: Arc<[MovingNode]>,
    /// The text of each box of `hits` that shows text input hits into
    /// ([`ShownText`]), by the box's place in `hits`, in that order.
    pub texts: ShownTexts,
    /// Its tables' index, made by their producer with them
    /// ([`TableIndex::of`]): the render thread looks a held gesture's
    /// target up in it, never walks a table for one.
    pub(crate) index: Arc<TableIndex>,
}

impl Default for Compositing {
    fn default() -> Self {
        Self {
            scrollers: Arc::from(Vec::new()),
            hits: Arc::from(Vec::new()),
            moving: Arc::from(Vec::new()),
            texts: Arc::from(Vec::new()),
            index: Arc::default(),
        }
    }
}

/// The texts of a hit table's boxes ([`Compositing::texts`]): each by the
/// place of its box in the table, in that order.
pub type ShownTexts = Arc<[(u32, Arc<ShownText>)]>;

/// The text a box showed, as its frame laid it out: what a point routed
/// in that frame is hit against for a span or a text offset, so text is
/// hit where the frame showed it, as its box is, never as laid out since.
/// Each is shared, unchanged, by every frame that shows it the same.
#[derive(Clone, Debug, PartialEq)]
pub enum ShownText {
    /// A paragraph's spans that take the pointer: a box each span's text
    /// took, one per line, from the corner of the paragraph's border box.
    Spans(Arc<[ShownSpan]>),
    /// A text field's text: what its offsets are hit in, as painted.
    Field {
        content: Arc<str>,
        text: crate::style::ResolvedText,
        /// Its content box, from the corner of its border box.
        area: Bounds,
        multiline: bool,
        /// Painted focused (its text not cut short).
        focused: bool,
        /// The content transform its text scrolls under, `None` for a field
        /// that does not scroll. How far it is scrolled is read off the
        /// property trees of the frame asked ([`Self::scrolled`]), as the
        /// compositor moved it since the commit, never copied at paint.
        scrolls: Option<u32>,
        /// How an offset in `content` falls in the value it showed
        /// (masked, a composition in it).
        offsets: crate::edit::Offsets,
        /// The caret it showed, its selection (anchor and caret, in
        /// order) and the range a composition took: what an input
        /// method's candidate window is placed by.
        caret: usize,
        selection: (usize, usize),
        preedit: Option<(usize, usize)>,
    },
}

impl ShownText {
    /// How far a field's content shows scrolled in `props`, the property
    /// trees of a frame that showed this text ([`ShownText::Field`]), whose
    /// box is in transform `boxed` (its hit's): its content transform's
    /// offset from its box's, both of that frame. Paint writes the content
    /// transform at the box's offset plus the scroll applied
    /// ([`crate::layout::FrameNode::offset`] is the box's transform's), and
    /// whatever moves the box (an ancestor's scroll, the compositor's)
    /// moves both alike, so the difference is the field's own scroll alone.
    /// Nothing for a paragraph's spans, or a field that does not scroll.
    #[must_use]
    pub fn scrolled(&self, props: &crate::scene::Properties, boxed: u32) -> (f32, f32) {
        match self {
            Self::Field {
                scrolls: Some(content), ..
            } => {
                let at = props.transform(*content).offset;
                let base = props.transform(boxed).offset;
                (at.0 - base.0, at.1 - base.1)
            }
            _ => (0.0, 0.0),
        }
    }
}

/// A box a span's text took in a frame, as that frame showed it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShownSpan {
    /// Which span it is ([`SpanId`]): its handler is found by it in the
    /// frame laid out, as a node is by its key.
    pub span: SpanId,
    /// The cursor it showed over it.
    pub cursor: Option<crate::style::Cursor>,
    /// The box, from the corner of the paragraph's border box.
    pub rect: Bounds,
}

/// What a span of a paragraph is across builds: its key
/// ([`crate::element::Span::key`]), or with none, its place among the
/// paragraph's spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpanId {
    Keyed(u64),
    At(u32),
}

impl Compositing {
    /// The scroll container that is `instance`, and where it is in the
    /// table: one lookup.
    pub(crate) fn scroller(&self, instance: u64) -> Option<(usize, &ScrollNode)> {
        #[cfg(test)]
        TABLE_READS.with(|n| n.set(n.get().saturating_add(1)));
        let i = usize::try_from(*self.index.by_instance.get(&instance)?).ok()?;
        self.scrollers.get(i).filter(|s| s.instance == instance).map(|s| (i, s))
    }

    /// Whether input reaches the node `target` (it is a hit box not below
    /// a modal layer): one lookup.
    pub(crate) fn reaches(&self, target: HitTarget) -> bool {
        #[cfg(test)]
        TABLE_READS.with(|n| n.set(n.get().saturating_add(1)));
        self.index.reached.contains(&target)
    }
}

/// The compositor's tables by identity: each scroll container's place by
/// its instance, and the nodes input reaches. Made with the tables, by
/// the painter that makes them, on the UI thread (at the cost of the
/// table it already compares whole), so the render thread holds a gesture
/// on a page of any size with lookups alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TableIndex {
    by_instance: foldhash::HashMap<u64, u32>,
    reached: foldhash::HashSet<HitTarget>,
}

impl TableIndex {
    /// The index of `scrollers` and `hits`.
    pub(crate) fn of(scrollers: &[ScrollNode], hits: &[Hit]) -> Self {
        Self {
            by_instance: scrollers
                .iter()
                .enumerate()
                .filter_map(|(i, s)| Some((s.instance, u32::try_from(i).ok()?)))
                .collect(),
            reached: hits.iter().filter(|h| !h.below_modal).map(|h| h.target).collect(),
        }
    }
}

#[cfg(test)]
crate::engine::engine_local! {
    /// Lookups the render thread made in its tables for a held gesture.
    [Fork] static TABLE_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The render thread's lookups in its tables so far (a cost test).
#[cfg(test)]
pub(crate) fn table_reads() -> u64 {
    TABLE_READS.with(std::cell::Cell::get)
}

/// A moving node's transform entry and opacity at `at` (nanoseconds on the
/// frame clock), as layout and paint place it (the same code, so a
/// compositor frame at the UI frame's own time draws what the UI recorded).
#[must_use]
pub(crate) fn moving_at(m: &MovingNode, at: u64, scale: f32) -> (TransformNode, f32, crate::style::Painted) {
    let moment = m.moving.curves.at(at, m.moving.transform);
    let shift = moment
        .transform
        .offset((m.space.width, m.space.height), m.moving.opened);
    let moved = crate::geometry::device_rounded(shift, scale);
    let offset = (m.moving.above.0 + moved.0, m.moving.above.1 + moved.1);
    let bounds = m.space.translated(offset.0, offset.1);
    let painted = moment.transform.painted(bounds, m.moving.opened);
    let matrix = (m.turns || !painted.is_identity()).then_some(GroupTransform {
        origin: (
            (bounds.x + painted.origin.0) * scale,
            (bounds.y + painted.origin.1) * scale,
        ),
        scale: painted.scale,
        rotate: painted.rotate,
    });
    let opacity = if m.invisible { 0.0 } else { moment.opacity };
    (
        TransformNode {
            parent: 0,
            offset,
            matrix,
        },
        opacity,
        painted,
    )
}

/// The transforms, clips and effects that changed between two frames, by id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PropertiesDelta {
    pub lens: (usize, usize, usize),
    pub transforms: Vec<(u32, TransformNode)>,
    pub clips: Vec<(u32, ClipNode)>,
    pub effects: Vec<(u32, EffectNode)>,
}

impl PropertiesDelta {
    /// Write what turns `old` into `new` over what it held, its buffers kept.
    fn between(&mut self, old: &Properties, new: &Properties) {
        fn changed<T: Clone + PartialEq>(into: &mut Vec<(u32, T)>, old: &[T], new: &[T]) {
            into.clear();
            into.extend(
                new.iter()
                    .enumerate()
                    .filter(|(i, t)| old.get(*i) != Some(*t))
                    .map(|(i, t)| (u32::try_from(i).unwrap_or(u32::MAX), t.clone())),
            );
        }
        self.lens = (new.transforms.len(), new.clips.len(), new.effects.len());
        changed(&mut self.transforms, &old.transforms, &new.transforms);
        changed(&mut self.clips, &old.clips, &new.clips);
        changed(&mut self.effects, &old.effects, &new.effects);
    }

    /// Write the entries of `props` whose ids are in each list (sorted,
    /// unique) over what it held, its buffers kept: what in-place writes
    /// changed.
    pub(crate) fn written(&mut self, props: &Properties, ids: (&[u32], &[u32], &[u32])) {
        fn of<T: Clone>(into: &mut Vec<(u32, T)>, v: &[T], ids: &[u32]) {
            into.clear();
            into.extend(
                ids.iter()
                    .filter_map(|&id| Some((id, v.get(usize::try_from(id).ok()?)?.clone()))),
            );
        }
        self.lens = (props.transforms.len(), props.clips.len(), props.effects.len());
        of(&mut self.transforms, &props.transforms, ids.0);
        of(&mut self.clips, &props.clips, ids.1);
        of(&mut self.effects, &props.effects, ids.2);
    }

    /// Write its entries into `to`, emptying it (its buffers kept).
    fn apply(&mut self, to: &mut Properties) {
        fn put<T: Clone + Default>(v: &mut Vec<T>, len: usize, changed: &mut Vec<(u32, T)>) {
            v.resize(len, T::default());
            for (i, t) in changed.drain(..) {
                if let Some(slot) = usize::try_from(i).ok().and_then(|i| v.get_mut(i)) {
                    *slot = t;
                }
            }
        }
        put(&mut to.transforms, self.lens.0, &mut self.transforms);
        put(&mut to.clips, self.lens.1, &mut self.clips);
        put(&mut to.effects, self.lens.2, &mut self.effects);
    }

    fn clear(&mut self) {
        self.lens = (0, 0, 0);
        self.transforms.clear();
        self.clips.clear();
        self.effects.clear();
    }

    fn is_empty(&self) -> bool {
        self.transforms.is_empty() && self.clips.is_empty() && self.effects.is_empty()
    }

    /// Whether applied to `old` it changes the trees' shape: their sizes,
    /// or what an entry hangs from (a transform's or an effect's parent, the
    /// transform a clip or an effect lies under, an effect's own turn and
    /// the clip it applies once turned).
    fn reshapes(&self, old: &Properties) -> bool {
        let lens = (old.transforms.len(), old.clips.len(), old.effects.len());
        let at = |v: usize, i: u32| usize::try_from(i).ok().filter(|&i| i < v);
        self.lens != lens
            || self.transforms.iter().any(|(i, t)| {
                at(old.transforms.len(), *i).is_none_or(|i| old.transforms.get(i).is_none_or(|o| o.parent != t.parent))
            })
            || self.clips.iter().any(|(i, c)| {
                at(old.clips.len(), *i).is_none_or(|i| old.clips.get(i).is_none_or(|o| o.transform != c.transform))
            })
            || self.effects.iter().any(|(i, e)| {
                at(old.effects.len(), *i).is_none_or(|i| {
                    old.effects.get(i).is_none_or(|o| {
                        o.parent != e.parent || o.transform != e.transform || o.transformed != e.transformed
                    })
                })
            })
    }
}

/// One UI frame for the render thread: what changed since the last commit
/// (ADR-0015). Applied whole, in sequence. The render thread empties it as
/// it applies it and sends it back ([`Compose::spare`]): the next frame
/// writes into its buffers, so a frame that changes as much as one before
/// allocates nothing to say so.
#[derive(Clone, Debug, Default)]
pub struct Commit {
    /// One more than the last commit's.
    pub sequence: u64,
    /// The [`MainFrame`] it answers.
    pub answers: u64,
    /// The pulls of the compositor's scroll its bases hold: every one the
    /// UI took in through this number ([`Pulled::take`], ADR-0032 I4).
    pub taken: u64,
    /// The frame time it was laid out at, nanoseconds on the frame clock.
    pub at: u64,
    /// It carries the whole scene and every glyph table, not what changed
    /// since the last: what a renderer made afresh takes in first.
    pub full: bool,
    pub size: (u32, u32),
    pub scale: f32,
    pub background: Option<crate::color::Color>,
    /// The chunks' order changed: `order` holds their ids in painting order.
    pub reorder: bool,
    pub order: Vec<ChunkId>,
    /// Chunks recorded again, new or under another state.
    pub chunks: Vec<Chunk>,
    pub properties: PropertiesDelta,
    /// The tables of what it moves, each when it changed.
    pub scrollers: Option<Arc<[ScrollNode]>>,
    pub hits: Option<Arc<[Hit]>>,
    pub moving: Option<Arc<[MovingNode]>>,
    pub texts: Option<ShownTexts>,
    /// The index of the scroll and hit tables, with either when it changed.
    pub(crate) index: Option<Arc<TableIndex>>,
    /// The faces, coordinates and settings the glyphs drawn need.
    pub glyphs: GlyphUpdate,
    /// The reach of the main frame it answers ([`MainFrame::scroll`]),
    /// emptied: given back to the compositor, which grants the next with
    /// it, so a main frame while a container moves allocates nothing.
    pub(crate) reach: Vec<ScrollDelta>,
    /// The scroll pulled with the main frame it answers
    /// ([`MainFrame::taken`]), emptied and given back alike.
    pub(crate) pulled: Vec<ScrollDelta>,
}

impl Commit {
    /// Empty, its buffers kept for the next frame's.
    pub fn clear(&mut self) {
        self.sequence = 0;
        self.answers = 0;
        self.taken = 0;
        self.at = 0;
        self.full = false;
        self.reorder = false;
        self.order.clear();
        self.chunks.clear();
        self.properties.clear();
        self.scrollers = None;
        self.hits = None;
        self.moving = None;
        self.texts = None;
        self.index = None;
        self.glyphs = GlyphUpdate::default();
        self.reach.clear();
        self.pulled.clear();
    }

    /// What turns the scene `old` (as last committed) into `new`, written
    /// over what it held.
    pub fn between(&mut self, old: &Scene, new: &Scene) {
        self.clear();
        let same_order =
            old.chunks.len() == new.chunks.len() && old.chunks.iter().zip(&new.chunks).all(|(a, b)| a.id == b.id);
        let kept: foldhash::HashMap<ChunkId, &Chunk> = old.chunks.iter().map(|c| (c.id, c)).collect();
        self.chunks.extend(
            new.chunks
                .iter()
                .filter(|c| {
                    kept.get(&c.id)
                        .is_none_or(|k| k.state != c.state || !Arc::ptr_eq(&k.display, &c.display))
                })
                .cloned(),
        );
        if !same_order {
            self.reorder = true;
            self.order.extend(new.chunks.iter().map(|c| c.id));
        }
        self.properties.between(&old.properties, &new.properties);
        self.tables(new, &old.compositing);
    }

    /// The frame's size and background, and the compositor's tables of
    /// `scene` where they differ from `before`.
    pub(crate) fn tables(&mut self, scene: &Scene, before: &Compositing) {
        let nc = &scene.compositing;
        self.size = scene.size;
        self.scale = scene.scale;
        self.background = scene.background;
        self.scrollers = replaced(&before.scrollers, &nc.scrollers).then(|| Arc::clone(&nc.scrollers));
        self.hits = replaced(&before.hits, &nc.hits).then(|| Arc::clone(&nc.hits));
        self.moving = replaced(&before.moving, &nc.moving).then(|| Arc::clone(&nc.moving));
        self.texts = replaced(&before.texts, &nc.texts).then(|| Arc::clone(&nc.texts));
        self.index = (self.scrollers.is_some() || self.hits.is_some()).then(|| Arc::clone(&nc.index));
    }

    /// Whether it changes nothing the compositor draws or moves.
    #[must_use]
    pub fn is_empty(&self, last: &Scene) -> bool {
        !self.reorder
            && self.chunks.is_empty()
            && self.properties.is_empty()
            && self.scrollers.is_none()
            && self.hits.is_none()
            && self.moving.is_none()
            && self.texts.is_none()
            && self.glyphs.is_empty()
            && self.size == last.size
            && self.scale == last.scale
            && self.background == last.background
    }

    /// Chunks and bytes of records it carries (devtools).
    #[must_use]
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

/// What the compositor moved one scroll container by that the UI takes in
/// ([`Outbox::take`]): the UI adds it to its own offset (ADR-0032).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollDelta {
    pub key: NodeKey,
    /// With `key`, the container's identity (its [`ScrollNode`]'s owner): a
    /// container another instance replaced at the same path is another.
    pub(crate) owner: Option<crate::reactive::OwnerId>,
    /// The mount it moved ([`ScrollNode::instance`], minted by the UI's
    /// painter, never reused): the UI takes it in only while the container
    /// at `key` and `owner` is still that mount, as cc applies a delta only
    /// to the element id it was pulled for (ADR-0033).
    pub(crate) instance: u64,
    /// What it moved, exactly ([`motion`]): the UI adds it to where it
    /// holds the container, so a motion cut at any frames and pulls sums
    /// to the same place.
    pub(crate) delta: motion::Exact,
    /// Of `delta`, what the compositor moved by itself (its fling), and
    /// the container's [`ScrollNode::generation`] it moved it under: the UI
    /// leaves that part out when it has jumped the container since (cc's
    /// clobbering commit; ADR-0032 I4). Real input it always takes.
    pub(crate) synthetic: motion::Exact,
    pub(crate) generation: u64,
    /// How far, at the speed it scrolls now, the compositor may scroll it
    /// before the commit after this frame's shows (signed as `delta`): the
    /// content a virtual list builds beyond its viewport that way, as
    /// Chromium's interest rect and Flutter's `cacheExtent` (ADR-0015).
    /// A main frame's alone ([`MainFrame::scroll`]).
    pub reach: (f32, f32),
    /// How many wheel steps the reader scrolled it by, whether or not they
    /// moved it (a wheel against its edge): each is taken once, and
    /// counted once ([`crate::ScrollHandle::wheels`]).
    pub steps: u32,
}

/// Leave to run one UI frame (`BeginMainFrame`): the time it will be
/// presented at, by when the render thread wants its commit, and how far
/// each container moving may go. Not `Clone`: one token, one commit.
#[derive(Debug)]
pub struct MainFrame {
    pub sequence: u64,
    /// The predicted present time, nanoseconds on the frame clock.
    pub present_at: u64,
    /// After this the render thread draws without the commit.
    pub deadline: std::time::Instant,
    /// The display's frame interval.
    pub interval: std::time::Duration,
    /// What the compositor moved since the last pull, pulled as the render
    /// thread granted the frame: the UI takes it in before it builds
    /// (ADR-0033).
    pub taken: Pulled,
    /// How far each moving container may yet go before the commit after
    /// this one shows ([`ScrollDelta::reach`]); no displacement.
    pub scroll: Vec<ScrollDelta>,
}

impl MainFrame {
    /// A token for a frame shown at `present_at`, with no scroll pulled.
    #[must_use]
    pub fn at(present_at: impl crate::clock::Moment) -> Self {
        Self {
            sequence: 0,
            present_at: present_at.nanos(),
            deadline: std::time::Instant::now(),
            interval: std::time::Duration::from_micros(16_667),
            taken: Pulled::default(),
            scroll: Vec::new(),
        }
    }
}

/// What a frame showed (ADR-0015): the input that arrives while it is on
/// screen is hit-tested through it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presented {
    /// Frames the compositor drew, this one included.
    pub frame: u64,
    /// The last commit it showed.
    pub commit: u64,
    /// When it was to be presented, nanoseconds on the frame clock.
    pub present_at: u64,
    /// Each scroll container as shown ([`compose::ScrollShown`]).
    pub scroll: ShownScrolls,
    /// How far each moving node was shown from where its commit placed it.
    pub moved: Vec<(NodeKey, (f32, f32))>,
    /// The scale and turn each moving node was shown at, as sampled then
    /// (its origin from the corner of its border box): input lands in it
    /// as it showed, whatever scale and turn its commit had.
    pub turned: Vec<(NodeKey, crate::style::Painted)>,
    /// What input is hit-tested against, all of this one frame (the
    /// frame's hit-test data, as Chromium's viz keeps a hit-test region
    /// list per compositor frame): its boxes that take the pointer, its
    /// property trees as composed (the compositor's scroll and motion in
    /// them), its scale, and the layers the renderer drew it with
    /// ([`crate::clip::hit_in`]), and the scroll containers a wheel's
    /// first event is routed among. The tables are shared with the frame
    /// before while unchanged; its property trees are its own, brought up
    /// from the frame its buffer last held by the entries written since.
    pub hits: Arc<[Hit]>,
    pub scrollers: Arc<[ScrollNode]>,
    /// The text its boxes showed ([`Compositing::texts`]), shared as its
    /// hit table is.
    pub texts: ShownTexts,
    pub properties: crate::scene::Properties,
    pub scale: f32,
    /// The window it was drawn for, logical: where a pointer can fall in
    /// it (what the hit index's cost is weighed over,
    /// [`crate::clip::HitIndex`]).
    pub viewport: (f32, f32),
    /// The layers it was drawn with, by effect id
    /// ([`crate::gpu::GpuRenderer::shown_stages`]), set when the frame is
    /// drawn, before input can see it.
    pub stages: Arc<[crate::scene::LayerStage]>,
    /// Tests: a token each snapshot a compositor makes holds while it
    /// lives (copies too, [`Compositor::alive`]), so a test counts the
    /// snapshots alive without a weak reference, which would keep the
    /// compositor from using one again.
    #[cfg(test)]
    pub(crate) alive: Option<Arc<()>>,
}

impl Presented {
    /// The one way a snapshot is made from a scene (the frame the
    /// compositor composed, or, before any frame was presented, the scene
    /// committed): its tables, its property trees, its scale, no layer
    /// drawn yet, frame number 0 until it is presented. Compose and the
    /// input routed before the first present read the same snapshot of the
    /// same commit, on either thread.
    #[must_use]
    pub fn of(scene: &Scene, commit: u64) -> Self {
        let mut p = Self {
            commit,
            properties: scene.properties.clone(),
            ..Self::default()
        };
        p.take_tables(scene);
        p
    }

    /// Take `scene`'s tables (all but the property trees, which a frame
    /// copies or catches up): its hit table and scroll containers, shared,
    /// and its scale.
    pub(crate) fn take_tables(&mut self, scene: &Scene) {
        self.hits = Arc::clone(&scene.compositing.hits);
        self.scrollers = Arc::clone(&scene.compositing.scrollers);
        self.texts = Arc::clone(&scene.compositing.texts);
        self.scale = scene.scale;
        self.viewport = scene.logical_size();
    }

    /// Tests: frame `frame`, shown from `present_at`, of a 64 px window
    /// whose one box that takes the pointer, node `key`, lies at `rect`
    /// (or none).
    #[cfg(test)]
    pub(crate) fn boxed(frame: u64, present_at: u64, boxed: Option<(u64, crate::geometry::Bounds)>) -> Self {
        let mut p = Self::of(&Scene::new((64, 64), 1.0, None), 0);
        p.frame = frame;
        p.present_at = present_at;
        p.hits = boxed
            .map(|(key, rect)| Hit {
                rect,
                shape: crate::shape::Shape::default(),
                slop: crate::style::Edges::default(),
                transform: 0,
                clip: 0,
                effect: 0,
                scroller: None,
                wheel: false,
                target: HitTarget {
                    key: NodeKey(key),
                    owner: None,
                },
                cursor: None,
                below_modal: false,
            })
            .into_iter()
            .collect();
        p
    }
}

/// Which frame the screen showed as an input came, as a stamp the render
/// thread routes a wheel by ([`Compositor::wheel_input`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    /// Nothing was presented yet: the scene last committed decides, no
    /// layer drawn ([`Presented::of`]), for every kind of input.
    BeforeFirst,
    /// The frame numbered so ([`Presented::frame`]): routed in it while
    /// the timeline keeps it, nowhere after.
    Frame(u64),
}

/// What the screen showed as an input came, its hit-test data with it:
/// what the UI thread routes a click, a move or a key by.
#[derive(Clone, Debug)]
pub enum Shown {
    /// Nothing was presented yet ([`Seen::BeforeFirst`]).
    BeforeFirst,
    /// A frame presented, its vblank come.
    Frame(Arc<Presented>),
    /// A frame the timeline no longer keeps, numbered so: input from it
    /// lands nowhere rather than in a frame that did not show.
    Gone(u64),
}

impl Shown {
    /// Its stamp.
    #[must_use]
    pub fn seen(&self) -> Seen {
        match self {
            Self::BeforeFirst => Seen::BeforeFirst,
            Self::Frame(p) => Seen::Frame(p.frame),
            Self::Gone(frame) => Seen::Frame(*frame),
        }
    }

    /// Whether it is the very same presentation as `other`.
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::BeforeFirst, Self::BeforeFirst) => true,
            (Self::Frame(a), Self::Frame(b)) => Arc::ptr_eq(a, b),
            (Self::Gone(a), Self::Gone(b)) => a == b,
            _ => false,
        }
    }
}

/// The presentation timeline (ADR-0015), the one source of what the user
/// could see at a time: each frame the screen showed, for the vblank it
/// was shown from, oldest first. A frame enters when the render thread
/// presented it, or drew it with pixels the same as the frame on screen
/// (nothing to blit: a commit that changed only what takes the pointer
/// is on screen from that vblank all the same); a draw whose present
/// failed enters nothing, so it never pushes out the frame on screen. A
/// frame entered is never changed after, and each has a number of its
/// own. What the screen showed at a time is a pure query over it
/// ([`Self::at`]): a frame shows from its vblank (a present returning is
/// not the pixels changing: the display latches the new image at its
/// vblank, as SurfaceFlinger latches a buffer only once it is ready and
/// keeps the current one until then), so asking about a later time never
/// changes the answer for an earlier. It keeps every frame shown within
/// [`RETAIN`] of the newest's vblank (and the one showing at its start):
/// what input that old still routes in.
#[derive(Debug, Default)]
pub struct OnScreen {
    timeline: std::sync::Mutex<Timeline>,
    /// The UI was told of frames it has not looked at yet.
    told: std::sync::atomic::AtomicBool,
}

/// How long before the newest frame's vblank a frame shown is kept for
/// input stamped with it, nanoseconds: an input this late still lands
/// where the screen showed it. Event Timing reports interactions slower
/// than 104 ms by default and INP calls an interaction of 200 ms or less
/// good: input waiting longer than that is a stall, not the norm; a quarter
/// second covers it and one frame more.
pub const RETAIN: u64 = 250_000_000;

/// The most frames the timeline keeps, whatever their times: a quarter
/// second at 250 Hz and the frame showing at its start. Faster (or a
/// clock a script holds still), input is kept for 64 frames rather than
/// the quarter second. With [`RETAIN`], what bounds the snapshots alive
/// (ADR-0015): `min(ceil(RETAIN / interval) + 1, KEPT)`.
pub const KEPT: usize = 64;

#[derive(Debug, Default)]
struct Timeline {
    /// Shown, oldest first.
    shown: std::collections::VecDeque<Arc<Presented>>,
    /// The newest frame no longer kept, if one was let go.
    gone: Option<u64>,
    /// Where the platform last saw the pointer in the window (logical),
    /// `None` outside it ([`OnScreen::point`]).
    pointer: Option<(f32, f32)>,
}

impl Timeline {
    fn seen_at(&self, t: u64) -> Seen {
        match self.shown.iter().rev().find(|p| p.present_at <= t) {
            Some(p) => Seen::Frame(p.frame),
            None => self.gone.map_or(Seen::BeforeFirst, Seen::Frame),
        }
    }

    fn resolve(&self, seen: Seen) -> Shown {
        match seen {
            Seen::BeforeFirst => Shown::BeforeFirst,
            Seen::Frame(n) => self
                .shown
                .iter()
                .rev()
                .find(|p| p.frame == n)
                .map_or(Shown::Gone(n), |p| Shown::Frame(Arc::clone(p))),
        }
    }
}

/// What a resting pointer at `at` is over in `frame`, by the one hit rule
/// ([`crate::clip::HitView`]): everything hover and the cursor are decided
/// by there — the box, the span of its text the frame showed under the
/// point (a span has a cursor and a hover of its own, so a scroll that
/// brings another span of one paragraph under the pointer changes them),
/// whether a modal layer is above it, and the cursor the frame showed
/// there (a cursor changed alone shows when its frame does).
pub(crate) type Under = (HitTarget, Option<SpanId>, bool, Option<crate::style::Cursor>);

/// [`Under`] at `at` in `frame`.
pub(crate) fn under(frame: &Presented, at: (f32, f32)) -> Option<Under> {
    let view = crate::clip::HitView::of(frame, None);
    let (place, h) = view.hit_place(at, |_| true)?;
    let span = frame.span_at(place, || view.local(h, at)).map(|s| (s.span, s.cursor));
    let cursor = span.and_then(|s| s.1).or(h.cursor);
    Some((h.target, span.map(|s| s.0), h.below_modal, cursor))
}

impl Presented {
    /// The span of the box at `place` in its hits whose text this frame
    /// showed under the point `local` gives (from the box's corner), if
    /// the box showed a paragraph's spans.
    pub(crate) fn span_at(&self, place: u32, local: impl FnOnce() -> Option<(f32, f32)>) -> Option<&ShownSpan> {
        let at = self.texts.binary_search_by_key(&place, |e| e.0).ok()?;
        let ShownText::Spans(boxes) = &*self.texts.get(at)?.1 else {
            return None;
        };
        let p = local()?;
        boxes.iter().find(|s| s.rect.contains(p.0, p.1))
    }
}

/// The one rule for when hover must be worked out again with no input
/// (ADR-0015): the vblank of the first frame waiting after `t` that shows
/// something else under `pointer` than `shown`, the frame on screen at `t`
/// (before the first present, or once gone, nothing). A pure function of
/// the pointer and the frames, as Flutter's and Blink's re-hit at the last
/// pointer position after content changed: nothing worked out earlier is
/// trusted. `waiting(after)` gives the next frame waiting after frame
/// `after` (the first with `None`), oldest first; with no pointer in the
/// window, never.
fn next_change(
    pointer: Option<(f32, f32)>,
    shown: &Shown,
    t: u64,
    mut waiting: impl FnMut(Option<u64>) -> Option<Arc<Presented>>,
) -> Option<u64> {
    let at = pointer?;
    let here = match shown {
        Shown::Frame(p) => under(p, at),
        Shown::BeforeFirst | Shown::Gone(_) => None,
    };
    let mut after = None;
    // At most the frames kept: each look moves past one.
    for _ in 0..KEPT {
        let next = waiting(after)?;
        debug_assert!(next.present_at > t, "only frames waiting for their vblank");
        if under(&next, at) != here {
            return Some(next.present_at);
        }
        after = Some(next.frame);
    }
    None
}

impl OnScreen {
    /// One with nothing presented yet.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn timeline(&self) -> std::sync::MutexGuard<'_, Timeline> {
        self.timeline.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether to tell the UI of a frame entered: only if it looked since
    /// it was last told, so at most one message waits, holding no frame.
    pub(crate) fn tell(&self) -> bool {
        !self.told.swap(true, std::sync::atomic::Ordering::AcqRel)
    }

    /// The UI looks at the timeline: the next frame entered tells it again.
    pub(crate) fn look(&self) {
        self.told.store(false, std::sync::atomic::Ordering::Release);
    }

    /// The platform saw the pointer at `at` (logical), or leave the window
    /// (`None`). It is set before the input that moved it is sent on, so
    /// the UI handling that input samples the timeline with it
    /// ([`Self::sample`]): nothing about the frames is worked out here.
    pub(crate) fn point(&self, at: Option<(f32, f32)>) {
        self.timeline().pointer = at;
    }

    /// `frame` shows from the vblank at its `present_at`; the frames it
    /// leaves no longer kept go to `let_go`, for the compositor to use
    /// again (into buffers it has: a present allocates nothing). Whether to
    /// wake the UI: only when `frame` changes what is under the pointer
    /// where it is now, compared with the frame before it (with no pointer
    /// in the window, never), so a compositor animation under a pointer at
    /// rest or outside the window wakes it for nothing. This is a wake, not
    /// a prediction kept: when the UI wakes, or handles a move the pointer
    /// made since, it works out when hover changes from the pointer and the
    /// frames waiting then ([`Self::sample`]). Both frames are hit outside
    /// the lock (an index made for a new table costs up to milliseconds,
    /// and the platform stamps input under the lock).
    pub(crate) fn presented(
        &self,
        frame: Arc<Presented>,
        let_go: &mut std::collections::VecDeque<Arc<Presented>>,
    ) -> bool {
        let (before, pointer) = {
            let mut t = self.timeline();
            if t.shown.back().is_some_and(|b| Arc::ptr_eq(b, &frame)) {
                return false;
            }
            let before = t.shown.back().cloned();
            Self::enter(&mut t, Arc::clone(&frame), let_go);
            (before, t.pointer)
        };
        pointer.is_some_and(|at| before.is_none_or(|b| under(&b, at) != under(&frame, at)))
    }

    /// `frame` enters the timeline `t`, the frames it leaves no longer kept
    /// going to `let_go`.
    fn enter(t: &mut Timeline, frame: Arc<Presented>, let_go: &mut std::collections::VecDeque<Arc<Presented>>) {
        debug_assert!(
            t.shown
                .back()
                .is_none_or(|b| b.present_at <= frame.present_at && b.frame < frame.frame),
            "the timeline goes forward: frame {} at {} after {:?}",
            frame.frame,
            frame.present_at,
            t.shown.back().map(|b| (b.frame, b.present_at))
        );
        let start = frame.present_at.saturating_sub(RETAIN);
        t.shown.push_back(frame);
        // The oldest goes once the next one showed before the start: one
        // that showed up to it is kept.
        while t.shown.len() > KEPT || t.shown.get(1).is_some_and(|next| next.present_at < start) {
            if let Some(p) = t.shown.pop_front() {
                t.gone = Some(p.frame);
                let_go.push_back(p);
            }
        }
    }

    /// The stamp of what the display showed at `t` (nanoseconds on the
    /// frame clock): the last frame shown whose vblank had come by then.
    /// Nothing changes: any order of questions gets the same answers.
    #[must_use]
    pub fn seen_at(&self, t: u64) -> Seen {
        self.timeline().seen_at(t)
    }

    /// What a stamp names, while kept.
    #[must_use]
    pub fn resolve(&self, seen: Seen) -> Shown {
        self.timeline().resolve(seen)
    }

    /// What the display shows at `t`, and the next vblank after `t` from
    /// which a frame entered shows something else under the pointer
    /// ([`next_change`]). Worked out now, from the pointer and the frames
    /// as they are, never kept: a move made after a frame entered is
    /// weighed against it, as is a frame entered after the move. The
    /// frame on screen is read in one look; the frames waiting for their
    /// vblank are read after it, each in a look of its own (they only
    /// ever join the end), and hit outside the lock.
    #[must_use]
    pub fn sample(&self, t: u64) -> (Shown, Option<u64>) {
        let (shown, pointer) = {
            let s = self.timeline();
            (s.resolve(s.seen_at(t)), s.pointer)
        };
        let next = next_change(pointer, &shown, t, |after| {
            let s = self.timeline();
            s.shown
                .iter()
                .find(|p| p.present_at > t && after.is_none_or(|a| p.frame > a))
                .cloned()
        });
        (shown, next)
    }

    /// What the display showed at `t`.
    #[must_use]
    pub fn at(&self, t: u64) -> Shown {
        let s = self.timeline();
        s.resolve(s.seen_at(t))
    }

    /// What the display shows now.
    #[must_use]
    pub fn now(&self) -> Shown {
        self.at(crate::clock::nanos(std::time::Instant::now()))
    }

    /// The frame shown last, its vblank come or not.
    #[must_use]
    pub fn latest(&self) -> Option<Arc<Presented>> {
        self.timeline().shown.back().map(Arc::clone)
    }

    /// The frame numbered `frame`, while kept.
    #[must_use]
    pub fn frame(&self, frame: u64) -> Option<Arc<Presented>> {
        self.timeline()
            .shown
            .iter()
            .rev()
            .find(|p| p.frame == frame)
            .map(Arc::clone)
    }

    /// How many frames it keeps.
    #[must_use]
    pub fn kept(&self) -> usize {
        self.timeline().shown.len()
    }

    /// The number of the oldest frame it keeps.
    #[must_use]
    pub fn oldest(&self) -> Option<u64> {
        self.timeline().shown.front().map(|p| p.frame)
    }
}

/// Each scroll container as a frame showed it ([`Presented::scroll`]), in
/// the order of its scroll table. A frame's buffer is reused by a later
/// frame and written again only when what the containers show changed
/// since it was last written (its version: compose::ScrollShown's), so a
/// frame at rest copies nothing and steady composing allocates nothing.
#[derive(Clone, Debug, Default)]
pub struct ShownScrolls {
    list: Vec<ShownScroll>,
    /// What it was written from; 0 for none.
    version: u64,
}

impl ShownScrolls {
    /// Hold `list`, version `version`: copied only if it holds another.
    pub(crate) fn write(&mut self, list: &[ShownScroll], version: u64) {
        if version == 0 || self.version != version {
            self.list.clear();
            self.list.extend_from_slice(list);
            self.version = version;
        }
    }
}

impl PartialEq for ShownScrolls {
    fn eq(&self, other: &Self) -> bool {
        self.list == other.list
    }
}

impl std::ops::Deref for ShownScrolls {
    type Target = [ShownScroll];

    fn deref(&self) -> &[ShownScroll] {
        &self.list
    }
}

impl<'a> IntoIterator for &'a ShownScrolls {
    type Item = &'a ShownScroll;
    type IntoIter = std::slice::Iter<'a, ShownScroll>;

    fn into_iter(self) -> Self::IntoIter {
        self.list.iter()
    }
}

impl FromIterator<ShownScroll> for ShownScrolls {
    fn from_iter<I: IntoIterator<Item = ShownScroll>>(iter: I) -> Self {
        Self {
            list: iter.into_iter().collect(),
            version: 0,
        }
    }
}

/// A scroll container as a frame showed it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShownScroll {
    pub key: NodeKey,
    /// With `key`, which container it is (its [`ScrollNode`]'s owner).
    pub(crate) owner: Option<crate::reactive::OwnerId>,
    pub offset: (f32, f32),
    /// What the offset moved its children by, as paint applies it.
    pub applied: (f32, f32),
}

/// The render thread's side of the frame (ADR-0015): the scene as
/// committed, the scroll it did, and what it composes and draws at each
/// frame's present time.
pub struct Compositor {
    scene: Scene,
    /// Where each chunk is in `scene.chunks`.
    index: foldhash::HashMap<ChunkId, usize>,
    scrolls: Scrolls,
    glyphs: Glyphs,
    /// Drawn over every frame until replaced (the debug overlay, the HUD),
    /// and its content's generation ([`Self::set_overlay`]).
    overlay: Option<Scene>,
    overlay_generation: u64,
    /// The last commit applied, and the time it was laid out at.
    commit: u64,
    committed_at: u64,
    /// The last composed frame: what is on screen.
    shown: Scene,
    /// What of the committed scene the commits since the last frame wrote.
    pending: Changed,
    /// What of `shown` changed since the renderer last drew it.
    changed: Changed,
    links: compose::Links,
    moved: compose::Moved,
    /// The last frame's sources, kept for their memory.
    spare: compose::Sources,
    /// Each scroll container as shown, kept from frame to frame.
    scroll_shown: compose::ScrollShown,
    /// The frames composed before the last, oldest first: one nothing
    /// else holds becomes the next ([`compose::POOL`] at most).
    pool: std::collections::VecDeque<Arc<Presented>>,
    /// The last frame a compositor before it drew (made afresh after a
    /// panic, [`Self::after`]): a frame so numbered or older that the
    /// timeline lets go was written by that one, its scroll list's version
    /// and its property trees' frame by that one's counters, which this
    /// one's run past again; it is never used again here, where its
    /// numbers would pass for this one's (fault matrix: a frame showed
    /// a list where it was before the panic).
    born: u64,
    /// What each compose kept wrote of the property trees, the latest first.
    deltas: std::collections::VecDeque<compose::PropDelta>,
    /// Property entries copied into frames composed (tests: the cost is
    /// what changed).
    props_copied: usize,
    work: compose::Work,
    /// Each frame is compared with one composed whole (`GUI_COLD_CHECK`).
    cold_check: bool,
    frame: u64,
    /// The last frame composed: what the next draw shows.
    presented: Arc<Presented>,
    /// The frames that reached the screen ([`OnScreen`]), shared with the
    /// threads that stamp input: input stamped with one is routed in it,
    /// all of that one frame.
    timeline: Arc<OnScreen>,
    /// What input that came before any frame was presented is routed in
    /// ([`Seen::BeforeFirst`]): the scene committed, no layer drawn, made
    /// once per commit.
    first: std::cell::OnceCell<Arc<Presented>>,
    /// The platform's wheel gesture under way: what it latched to, and
    /// when it began (its identity). What the compositor resamples and flings of it is its
    /// ([`Scrolls::gesture`], by when it began).
    latch: WheelGesture<Latched, u64>,
    /// Clip chains wheel routing has walked: only those of boxes under
    /// the wheel.
    chain_walks: std::cell::Cell<usize>,
    /// Tests: the token each snapshot it composes afresh holds
    /// ([`Presented::alive`]).
    #[cfg(test)]
    pub(crate) alive: Option<Arc<()>>,
}

impl Compositor {
    #[must_use]
    pub fn new(glyphs: Glyphs) -> Self {
        Self {
            scene: Scene::default(),
            index: foldhash::HashMap::default(),
            scrolls: Scrolls::default(),
            glyphs,
            overlay: None,
            overlay_generation: 0,
            commit: 0,
            committed_at: 0,
            shown: Scene::default(),
            pending: Changed::all(),
            changed: Changed::default(),
            links: compose::Links::default(),
            moved: compose::Moved::default(),
            spare: compose::Sources::default(),
            scroll_shown: compose::ScrollShown::default(),
            pool: std::collections::VecDeque::new(),
            born: 0,
            deltas: std::collections::VecDeque::new(),
            props_copied: 0,
            work: compose::Work::default(),
            cold_check: crate::app::cold_check_asked(),
            frame: 0,
            presented: Arc::new(Presented::default()),
            timeline: OnScreen::new(),
            first: std::cell::OnceCell::new(),
            latch: WheelGesture::default(),
            chain_walks: std::cell::Cell::new(0),
            #[cfg(test)]
            alive: None,
        }
    }

    /// One made afresh (after a render thread panic) on the presentation
    /// timeline of the one before: what was presented still shows, and
    /// its frames are numbered on from the last presented, so no stamp
    /// names two frames. It goes on with what the one before accepted of
    /// input ([`Accepted`]): its scroll goes to the UI through that one's
    /// outbox, which keeps what it moved and the UI took (ADR-0032), so
    /// the first commit it takes in shows it on top of its base, and its
    /// fling goes on by time from where the one before brought it. It
    /// holds no gesture's latch: the one under way went to the UI as input
    /// was diverted ([`Self::hand_gesture`]), and comes back as input is
    /// restored ([`Self::take_gesture`]).
    #[must_use]
    pub fn after(glyphs: Glyphs, timeline: Arc<OnScreen>, accepted: Accepted) -> Self {
        let frame = timeline.latest().map_or(0, |p| p.frame);
        let Accepted { outbox, fling, flings } = accepted;
        Self {
            timeline,
            frame,
            born: frame,
            scrolls: Scrolls::with(outbox, fling, flings),
            ..Self::new(glyphs)
        }
    }

    /// Its presentation timeline and all it accepted of input, for a
    /// compositor made afresh after a panic ([`Self::after`]). Settle its
    /// gesture first ([`Self::settle_gesture`]), so what its resampling
    /// held is in the outbox. Every field is named here: each is carried,
    /// or is shown state the next compositor makes again (ADR-0033).
    #[must_use]
    pub fn into_accepted(self) -> (Arc<OnScreen>, Accepted) {
        let Self {
            scene: _,
            index: _,
            scrolls,
            glyphs: _,
            overlay: _,
            overlay_generation: _,
            commit: _,
            committed_at: _,
            shown: _,
            pending: _,
            changed: _,
            links: _,
            moved: _,
            spare: _,
            scroll_shown: _,
            pool: _,
            born: _,
            deltas: _,
            props_copied: _,
            work: _,
            cold_check: _,
            frame: _,
            presented: _,
            timeline,
            first: _,
            // Handed to the UI as input was diverted ([`Self::hand_gesture`]).
            latch: _,
            chain_walks: _,
            #[cfg(test)]
                alive: _,
        } = self;
        (timeline, scrolls.into_accepted())
    }

    /// [`Self::into_accepted`]'s part, the compositor going on (a test, a
    /// replay of a recovery): what it accepted, taken out.
    #[must_use]
    pub fn take_accepted(&mut self) -> Accepted {
        std::mem::take(&mut self.scrolls).into_accepted()
    }

    /// Apply at `at` all the gesture under way holds of its events, and
    /// its fling up to `at` ([`Scrolls::settle_gesture`]): what it accepted
    /// is moved, and in the outbox, before input stops coming through it
    /// or with no frame drawn (ADR-0033). Its fling is not ended: it goes
    /// on by time. Its latch stays, for [`Self::hand_gesture`].
    pub fn settle_gesture(&mut self, at: u64) {
        self.scrolls.settle_gesture(at);
    }

    /// A fling handed on stops: a wheel reached the UI meanwhile.
    pub(crate) fn stop_fling(&mut self) {
        self.scrolls.stop_fling();
    }

    /// A gesture is resampled or flung: some of its motion waits for the
    /// next frame's tick ([`Self::settle_gesture`] applies it).
    #[must_use]
    pub fn gesture_moving(&self) -> bool {
        self.scrolls.gesture_moving()
    }

    /// The wheel gesture under way, as the UI latches one, while it is
    /// latched to a scroll container, handed over as input stops coming
    /// through the compositor: the rest of the gesture the UI scrolls stays
    /// on that container (ADR-0033). It keeps none: the latch is the UI's
    /// until the UI hands it back ([`Self::take_gesture`]). A gesture
    /// latched to the UI is the UI's latch already; none is handed then.
    pub(crate) fn hand_gesture(&mut self) -> Option<HandedGesture> {
        let compositing = &self.scene.compositing;
        let scroller = |instance: u64| compositing.scroller(instance).map(|(_, s)| s);
        let handed = self.latch.map(
            |to| match *to {
                Latched::Scroll(instance) => scroller(instance).map(|s| HitTarget {
                    key: s.key,
                    owner: s.owner,
                }),
                Latched::Ui(_) => None,
            },
            |under| scroller(under).map(|s| s.key),
        );
        self.latch.end();
        // Its motion with it, fingers still down: the rest of its events
        // and its lift may come to the UI.
        handed.map(|g| {
            let motion = g.holds().and_then(|(began, _)| self.scrolls.motion_of(began));
            HandedGesture(g, false, motion)
        })
    }

    /// The UI's wheel gesture, handed back as input comes through the
    /// compositor again (ADR-0033): its latch from now on, on the same
    /// container. One the UI keeps (a wheel handler there) is latched to
    /// the UI, so its rest goes on there; one on a container no longer in
    /// the tables is none. Once a recovery: a walk of the containers.
    pub(crate) fn take_gesture(&mut self, gesture: HandedGesture) {
        let scrollers = &self.scene.compositing.scrollers;
        let find = |key: NodeKey, owner: Option<Option<crate::reactive::OwnerId>>| {
            scrollers
                .iter()
                .find(|s| s.key == key && owner.is_none_or(|o| s.owner == o))
                .map(|s| s.instance)
        };
        let latch = gesture.0.map(
            |t| {
                if gesture.1 {
                    Some(Latched::Ui(None))
                } else {
                    find(t.key, Some(t.owner)).map(Latched::Scroll)
                }
            },
            |under| find(under, None),
        );
        self.latch = latch.unwrap_or_default();
        // Its motion goes on here: on the container it latched to, its
        // recent events kept, or its fling from the UI's lift; a lift
        // whose latch ended since (the platform's momentum over) on the
        // container it moved, by that container's identity.
        let Some(motion) = gesture.2 else { return };
        let instance = match self.latch.holds() {
            Some((_, Some(Latched::Scroll(instance)))) => Some(*instance),
            Some(_) => None,
            None => motion.lifted.and(motion.to).and_then(|t| find(t.key, Some(t.owner))),
        };
        if let Some((_, node)) = instance.and_then(|i| self.scene.compositing.scroller(i)) {
            let node = node.clone();
            self.scrolls.adopt(&node, motion);
        }
    }

    /// The gesture it latched, by identity, and whether it scrolls a
    /// container itself in it (not the UI's): what the single-owner rule
    /// is checked against ([`Self::hand_gesture`]).
    #[cfg(test)]
    pub(crate) fn holds_gesture(&self) -> Option<(u64, bool)> {
        self.latch
            .holds()
            .map(|(id, to)| (id, matches!(to, Some(Latched::Scroll(_)))))
    }

    /// What it scrolled since the last pull, for the input the render
    /// thread hands the UI now (ADR-0033): the UI takes it in before it
    /// handles that input.
    #[must_use]
    pub fn pull(&mut self) -> Pulled {
        let mut deltas = Vec::new();
        let take = self.scrolls.pull(&mut deltas);
        Pulled { take, deltas }
    }

    /// [`Self::pull`] for a main frame, into the buffer the last commit
    /// gave back ([`Commit::pulled`]): granting frames while a container
    /// moves allocates nothing once warm.
    pub(crate) fn pull_for_frame(&mut self) -> Pulled {
        self.scrolls.pull_for_frame()
    }

    /// How many clip chains wheel routing has walked.
    #[cfg(test)]
    pub(crate) fn chain_walks(&self) -> usize {
        self.chain_walks.get()
    }

    /// Compare each frame it composes with one composed whole, panicking
    /// where they differ (or stop).
    pub fn check_cold(&mut self, on: bool) {
        self.cold_check = on;
    }

    /// Fling trackpad gestures after the fingers lift (`true`), or leave
    /// it to the platform's own momentum events (macOS, Windows). It
    /// defaults to what the platform built for does; a replayed input log
    /// sets what its platform did.
    pub fn set_flings(&mut self, flings: bool) {
        self.scrolls.set_flings(flings);
    }

    /// The scene as last committed.
    #[must_use]
    pub fn committed(&self) -> &Scene {
        &self.scene
    }

    /// The glyph tables it draws with.
    pub fn glyphs(&mut self) -> &mut Glyphs {
        &mut self.glyphs
    }

    /// Take in `commit`, whole, emptying it: what it carried is the
    /// scene's now, and its buffers go back to the UI for the next frame.
    pub fn apply(&mut self, commit: &mut Commit) {
        self.first.take();
        self.scrolls.recycle_reach(std::mem::take(&mut commit.reach));
        self.scrolls.recycle_pull(std::mem::take(&mut commit.pulled));
        self.glyphs.apply(std::mem::take(&mut commit.glyphs));
        let scene = &mut self.scene;
        let pending = &mut self.pending;
        if commit.full
            || commit.reorder
            || commit.size != scene.size
            || commit.scale != scene.scale
            || commit.background != scene.background
        {
            pending.all = true;
        }
        let properties = &mut commit.properties;
        if !pending.all && properties.reshapes(&scene.properties) {
            pending.all = true;
        }
        if !pending.all {
            pending.transforms.extend(properties.transforms.iter().map(|(i, _)| *i));
            pending.clips.extend(properties.clips.iter().map(|(i, _)| *i));
            pending.effects.extend(properties.effects.iter().map(|(i, _)| *i));
        }
        scene.size = commit.size;
        scene.scale = commit.scale;
        scene.background = commit.background;
        if commit.reorder {
            let mut by_id: foldhash::HashMap<ChunkId, Chunk> = std::mem::take(&mut scene.chunks)
                .into_iter()
                .map(|c| (c.id, c))
                .collect();
            for c in commit.chunks.drain(..) {
                by_id.insert(c.id, c);
            }
            scene.chunks = commit.order.iter().filter_map(|id| by_id.remove(id)).collect();
            self.index = scene.chunks.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        } else {
            for c in commit.chunks.drain(..) {
                if let Some(&i) = self.index.get(&c.id)
                    && let Some(slot) = scene.chunks.get_mut(i)
                {
                    *slot = c;
                    pending.chunks.push(i);
                }
            }
        }
        properties.apply(&mut scene.properties);
        if let Some(s) = commit.scrollers.take() {
            scene.compositing.scrollers = s;
        }
        if let Some(h) = commit.hits.take() {
            scene.compositing.hits = h;
        }
        if let Some(m) = commit.moving.take() {
            scene.compositing.moving = m;
        }
        if let Some(t) = commit.texts.take() {
            scene.compositing.texts = t;
        }
        if let Some(i) = commit.index.take() {
            scene.compositing.index = i;
        }
        // Its bases hold what the UI took; the rest goes on top of them
        // (ADR-0032, I4).
        self.scrolls.committed(&scene.compositing, commit.taken, commit.at);
        self.commit = commit.sequence;
        self.committed_at = commit.at;
        commit.clear();
    }

    /// Replace what is drawn over every frame. The UI sends it with each
    /// commit, made again or not: its content generation moves only when
    /// what it shows does, so an overlay unchanged is not drawn again.
    pub fn set_overlay(&mut self, overlay: Option<Scene>) {
        let same = match (&self.overlay, &overlay) {
            (Some(a), Some(b)) => same_overlay(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.overlay_generation = self.overlay_generation.wrapping_add(1);
        }
        self.overlay = overlay;
    }

    /// Take the platform's wheel event `w`: hand it to the UI (it is over
    /// a wheel handler), with the node it is over, or scroll a container,
    /// as Chrome scrolls. A gesture stays latched to what took its first
    /// event, even once that container cannot go on (no chaining within a
    /// gesture): a trackpad's from its fingers going down to their lifting
    /// and the momentum after, however long it rests; a notched wheel's
    /// while it turns within [`WHEEL_LATCH`] of its last turn, the pointer
    /// not moved past [`LATCH_SLOP`] nor onto another container. A new one
    /// starts at the innermost container under it that can still scroll
    /// that way, passing those at their end to the one around them. It is
    /// routed in the frame shown as it came (`w.seen`), that frame's hit
    /// table and transforms both, while the timeline keeps that frame
    /// ([`RETAIN`]).
    pub fn wheel_input(&mut self, w: WheelInput) -> Wheeled {
        // Fingers down are an event whether or not anything takes them: they
        // end the gesture before, and stop what the compositor resamples or
        // flings of it, before they are routed (a touch with no movement, a
        // touch-to-stop, finds no container that would move, yet stops the
        // fling under it, as AppKit's `MayBegin` and Chrome's fling cancel
        // on touch do).
        // Each stops the gesture before with all of its events' motion
        // applied (ADR-0032, I6): fingers that went down and came up with
        // no frame between still moved what they moved.
        if w.precise && w.phase == WheelPhase::Touched {
            self.latch.touch(w.time);
            self.scrolls.stop_gesture(w.time);
            return Wheeled::Nothing;
        }
        if w.precise && w.phase == WheelPhase::Began {
            self.latch.begin();
            self.scrolls.stop_gesture(w.time);
        }
        let Some((to, went)) = self.latched(w) else {
            return Wheeled::Nothing;
        };
        // A gesture begun ends what the compositor resampled or flung of
        // the one before (Chrome's scroll begin ends the scroll under way).
        if went.first {
            self.scrolls.stop_gesture(w.time);
        }
        let compositing = &self.scene.compositing;
        let scrollers = &compositing.scrollers;
        let target = match to {
            Latched::Ui(target) => {
                let step = WheelStep {
                    gesture: Some(went.began),
                    ..w.step()
                };
                return Wheeled::Ui(target, step);
            }
            Latched::Scroll(instance) => compositing
                .scroller(instance)
                .and_then(|(i, _)| u32::try_from(i).ok())
                .map(WheelTarget),
        };
        // In units, exactly: the router handed the delta on as a whole
        // number of them ([`motion::Carry`]).
        let delta = motion::units2(w.delta);
        let moved = if w.precise {
            self.scrolls
                .gesture(scrollers, target, delta, (w.time, went.began), w.phase.lifted())
        } else {
            target.is_some_and(|t| self.scrolls.scroll(scrollers, t, delta, w.time))
        };
        if moved { Wheeled::Scrolled } else { Wheeled::Nothing }
    }

    /// Take the platform's wheel event `w` ([`Self::wheel_input`]), and
    /// make what it hands the UI the UI's input (`Input::WheelAt`, its
    /// delta as the event gave it): the one dispatch the live render
    /// thread, a scenario and a replay run, every outcome handled by each.
    pub fn dispatch_wheel(&mut self, w: WheelInput) -> Dispatched {
        match self.wheel_input(w) {
            Wheeled::Ui(target, step) => Dispatched::Ui(crate::window::Input::WheelAt {
                step,
                dx: w.delta.0,
                dy: w.delta.1,
                target,
            }),
            Wheeled::Scrolled => Dispatched::Scrolled,
            Wheeled::Nothing => Dispatched::Nothing,
        }
    }

    /// What `w` goes to, in which gesture: what its gesture latched to
    /// while it goes on, else what it is routed to, latched for a new one.
    fn latched(&mut self, w: WheelInput) -> Option<(Latched, latch::Went)> {
        let compositing = &self.scene.compositing;
        let step = w.step();
        // Routed in the frame it was stamped with only when it begins a
        // gesture (or a notched wheel's pointer moved): the rest of a
        // gesture follows its latch, whatever a commit since put under it.
        let mut routed: Option<Option<Routed>> = None;
        // One rule for every latch, checked on each event against what is
        // there now: its target by identity (a node by key and the
        // instance that made it, a container by the instance it is).
        // Gone, or below a modal layer opened since: the gesture's latch
        // ends, and what it goes on over is routed afresh.
        let held = self
            .latch
            .held(&step, || {
                routed
                    .get_or_insert_with(|| self.route(w.point, w.delta, w.seen))
                    .as_ref()
                    .map(|r| r.under)
            })
            .copied()
            .filter(|to| match to {
                Latched::Ui(None) => true,
                Latched::Ui(Some(target)) => compositing.reaches(*target),
                Latched::Scroll(instance) => compositing.scroller(*instance).is_some_and(|(_, s)| !s.below_modal),
            });
        let (to, under) = match held {
            Some(to) => (Some(to), None),
            None => routed
                .unwrap_or_else(|| self.route(w.point, w.delta, w.seen))
                .map_or((None, None), |r| (r.to, r.under)),
        };
        let went = self.latch.went(&step, under, to, held.is_some())?;
        to.map(|to| (to, went))
    }

    /// What input stamped `seen` is routed in, all of its hit-test data
    /// together (its tables, property trees, scale and layers): the frame
    /// so numbered while the timeline keeps it, or, before any frame was
    /// presented, the scene committed with no layer drawn, as the UI
    /// routes a click then ([`Presented::of`], keyed by the same commit:
    /// one rule for every kind of input). A frame no longer kept finds nothing: input routed
    /// in another frame than the one it saw would land where no pixel
    /// showed it.
    fn tables(&self, seen: Seen) -> Option<Arc<Presented>> {
        match seen {
            Seen::BeforeFirst => Some(Arc::clone(
                self.first
                    .get_or_init(|| Arc::new(Presented::of(&self.scene, self.commit))),
            )),
            Seen::Frame(frame) => self.timeline.frame(frame),
        }
    }

    /// The last frame composed shows from its vblank: it enters the
    /// timeline, input stamped with it is routed in it. Called when its
    /// present was made, or when nothing needed presenting (its pixels
    /// are those on screen); never for a present that failed. The frames
    /// the timeline lets go come back to be used again. Whether it changed
    /// what is under the pointer ([`OnScreen::presented`]): only then is
    /// the UI told.
    pub fn did_present(&mut self) -> bool {
        let moves = self.timeline.presented(Arc::clone(&self.presented), &mut self.pool);
        let born = self.born;
        self.pool.retain(|p| p.frame > born);
        compose::trim_pool(&mut self.pool);
        moves
    }

    /// The frame presented last.
    #[must_use]
    pub fn on_screen(&self) -> Option<Arc<Presented>> {
        self.timeline.latest()
    }

    /// Tests that compose as the render thread: the frame composed last
    /// is presented, and input is stamped with it.
    #[cfg(test)]
    pub(crate) fn present_last(&mut self) -> Seen {
        self.did_present();
        Seen::Frame(self.presented.frame)
    }

    /// The presentation timeline, for the threads that stamp input.
    #[must_use]
    pub fn timeline(&self) -> Arc<OnScreen> {
        Arc::clone(&self.timeline)
    }

    /// Property entries copied into the frames composed so far.
    #[must_use]
    pub fn props_copied(&self) -> usize {
        self.props_copied
    }

    /// Whether `node` (as some frame showed it) can scroll by `delta` from
    /// where it is now.
    /// It is looked up by identity in the index ([`Compositing::scroller`]):
    /// a step of routing costs the hit path's, not the page's.
    fn can_scroll(&self, node: &ScrollNode, delta: (f32, f32)) -> bool {
        let now = self.scene.compositing.scroller(node.instance).map_or(node, |(_, s)| s);
        let along = if now.rtl { -delta.0 } else { delta.0 };
        scroll::wheel_moves(self.scrolls.pos(now, 0), now.end, motion::units2((along, delta.1)))
    }

    /// Whether `node` takes a wheel moving `delta` at all, moved or held
    /// against its edge ([`scroll::wheel_takes`]).
    fn takes_wheel(node: &ScrollNode, delta: (f32, f32)) -> bool {
        let along = if node.rtl { -delta.0 } else { delta.0 };
        scroll::wheel_takes(node.end, motion::units2((along, delta.1)))
    }

    /// Where a wheel at `at` moving `delta` that starts a gesture goes, in
    /// the frame numbered `seen`: walking up from the box under it as the
    /// UI's routing does, a wheel handler (the UI's, with the box), or the
    /// innermost container that can still scroll that way; with the
    /// innermost container under it, whatever it does.
    fn route(&self, at: (f32, f32), delta: (f32, f32), seen: Seen) -> Option<Routed> {
        let c = self.tables(seen)?;
        // The box under it by the one hit rule a click lands by
        // ([`crate::clip::hit_in`]), through that one frame's tables,
        // property trees and the layers it was drawn with.
        let view = crate::clip::HitView::of(&c, Some(&self.chain_walks));
        let hit = view.hit(at, |_| true)?;
        let under = hit
            .scroller
            .and_then(|i| c.scrollers.get(usize::try_from(i).ok()?))
            .map(|n| n.instance);
        let routed = |to: Option<Latched>| Some(Routed { to, under });
        if hit.wheel {
            return routed(Some(Latched::Ui(Some(hit.target))));
        }
        let mut s = hit.scroller;
        while let Some(i) = s {
            let Some(node) = usize::try_from(i).ok().and_then(|i| c.scrollers.get(i)) else {
                break;
            };
            if node.wheel {
                return routed(Some(Latched::Ui(Some(hit.target))));
            }
            if self.can_scroll(node, delta) {
                return routed(Some(Latched::Scroll(node.instance)));
            }
            if node.wheel_above {
                return routed(Some(Latched::Ui(Some(hit.target))));
            }
            s = node.parent;
        }
        // None can move that way: the innermost that scrolls along that
        // axis takes it against its edge (its overlay scrollbar shows), as
        // the UI's route has it ([`crate::interact`]'s `scroll_from`).
        let mut s = hit.scroller;
        while let Some(node) = s.and_then(|i| c.scrollers.get(usize::try_from(i).ok()?)) {
            if Self::takes_wheel(node, delta) && !node.below_modal {
                return routed(Some(Latched::Scroll(node.instance)));
            }
            s = node.parent;
        }
        routed(None)
    }

    /// A main frame's reach given back ([`Commit::reach`]): granting the
    /// next allocates nothing (tests; [`Self::apply`] takes the commit's).
    #[cfg(test)]
    pub(crate) fn recycle_reach(&mut self, reach: Vec<ScrollDelta>) {
        self.scrolls.recycle_reach(reach);
    }

    /// How far each moving container may go within `horizon` of a main
    /// frame granted at `at` (until the commit after the one answering it
    /// shows): over the containers that moved lately alone.
    pub fn begin_main_frame(
        &mut self,
        at: impl crate::clock::Moment,
        horizon: std::time::Duration,
    ) -> Vec<ScrollDelta> {
        self.scrolls.reach(&self.scene.compositing, at, horizon.as_secs_f32())
    }

    /// Whether anything it moves moves at `at` (scroll prediction, a fling,
    /// a motion), so frames go on without the UI.
    #[must_use]
    pub fn animating(&self, at: impl crate::clock::Moment) -> bool {
        self.scrolls.animating(at) || !self.scene.compositing.moving.is_empty()
    }

    /// Whether it scrolled anything the UI has not been handed yet: the UI
    /// is owed a main frame to take it in (and to build what it scrolls to).
    #[must_use]
    pub fn scrolled(&self) -> bool {
        self.scrolls.pending()
    }

    /// Advance scroll prediction and flings to `at` on the frame clock
    /// (exact nanoseconds, or seconds near its start: [`crate::clock::Moment`]).
    pub fn tick(&mut self, at: impl crate::clock::Moment) {
        self.scrolls.tick(&self.scene.compositing, at);
    }

    /// The scene to draw at `at`: as committed, with the scroll it did and
    /// the motion it runs written over its property trees. What of it
    /// changed since the renderer last drew is kept for [`Self::draw`].
    pub fn compose(&mut self, at: impl crate::clock::Moment) -> &Scene {
        self.compose_into_shown(at.nanos());
        self.frame = self.frame.saturating_add(1);
        &self.shown
    }

    /// What the last composed frame showed.
    #[must_use]
    pub fn presented(&self) -> Arc<Presented> {
        Arc::clone(&self.presented)
    }

    /// A renderer drew the frame composed last ([`Self::compose`]): what
    /// changed of it is taken, and the next frame's changes start empty.
    pub fn drew(&mut self) {
        self.changed.clear();
    }

    /// Compose the frame at `at` and draw it with `gpu`, the overlay over it.
    pub fn draw(&mut self, gpu: &mut GpuRenderer, at: impl crate::clock::Moment) {
        self.tick(at);
        self.compose(at);
        gpu.render(&self.shown, &self.changed, &mut self.glyphs);
        self.publish_layers(gpu);
        self.drew();
        if let Some(o) = &self.overlay {
            gpu.overlay_as_of(o, self.overlay_generation, &mut self.glyphs);
        }
    }

    /// The frame just drawn shows the layers `gpu` drew it with: what
    /// hit testing through it reads ([`Presented::stages`]). Set before
    /// the frame is handed to input (after its present, `thread.rs`). A
    /// frame already on the timeline is never changed: drawn again (a
    /// capture), its layers go with a copy, which the timeline takes only
    /// if that copy is itself presented ([`Headless::capture`]).
    fn publish_layers(&mut self, gpu: &GpuRenderer) {
        let stages = gpu.shown_stages();
        if Arc::ptr_eq(&self.presented.stages, &stages) {
            return;
        }
        match Arc::get_mut(&mut self.presented) {
            // Not published yet: still the frame being made.
            Some(p) => p.stages = stages,
            // Published already (a capture after its present): another
            // snapshot, so another number. It is composed of nothing new,
            // so the record of what it wrote is empty.
            None => {
                let mut p = Presented::clone(&self.presented);
                p.stages = stages;
                self.frame = self.frame.saturating_add(1);
                p.frame = self.frame;
                self.wrote_nothing(self.frame);
                self.presented = Arc::new(p);
            }
        }
    }

    /// The frame as last composed, `overlay` over it, painted whole aside
    /// and read back. A page taller than a texture has no canvas: what is
    /// read back is what shows, and input reads the layers it was drawn with.
    pub fn capture(&mut self, gpu: &mut GpuRenderer, overlay: Option<&Scene>) -> Option<Pixels> {
        let pixels = gpu.capture(&self.shown, overlay, &mut self.glyphs);
        self.publish_layers(gpu);
        pixels
    }
}

/// Where a pipeline's frames go: a compositor drawing them now, headless,
/// or the render thread's.
pub trait Compose {
    /// Take in a frame's commit, and with it what is drawn over it.
    fn commit(&mut self, commit: Box<Commit>, overlay: Option<Scene>);
    /// A commit the renderer applied and gave back, emptied, for the next
    /// frame to write into: commits go round, not one made per frame.
    fn spare(&mut self) -> Option<Box<Commit>>;
    /// Replace what is drawn over the frames (the HUD, after its frame).
    fn overlay(&mut self, overlay: Option<Scene>);
    /// What the renderer holds across frames, as last known.
    fn held(&self) -> Held;
    /// What the renderer did for the last frame drawn, as last known.
    fn drawn(&self) -> Drawn;
    /// The frame as last drawn, `overlay` over it, read back.
    fn capture(&mut self, overlay: Option<Scene>) -> Option<Pixels>;
    /// How long committing waited for the display since last asked: a
    /// compositor that draws inline may; the render thread's never makes
    /// the UI wait, so its frames report none.
    fn waited(&mut self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
    /// What the screen shows now ([`OnScreen::now`]).
    fn presented(&self) -> Shown;
    /// What the screen shows now and the next vblank, nanoseconds on the
    /// frame clock, from which a frame already entered changes what is
    /// under the pointer, in one look ([`OnScreen::sample`]): hover is
    /// worked out again then. One drawing inline shows each frame at once:
    /// none to wait for.
    fn shown_and_next(&self) -> (Shown, Option<u64>) {
        (self.presented(), None)
    }
    /// A token for the next UI frame shown at `at`.
    fn begin(&mut self, at: u64) -> MainFrame;
}

/// The compositor and the GPU in one thread: each commit is applied,
/// composed at its own time and drawn at once (a [`crate::Scenario`]'s
/// deterministic schedule of ADR-0015's stages).
pub struct Headless {
    pub(crate) compositor: Compositor,
    pub(crate) gpu: Box<GpuRenderer>,
    /// Read each frame back; else only wait for the GPU.
    read: bool,
    /// The last frame drawn, read back.
    pixels: Option<Pixels>,
    sequence: u64,
    /// How long the UI takes to answer a main frame, as a render thread
    /// would have measured it: here it answers at once, unless a test says.
    ui_late: std::time::Duration,
    /// The last commit, applied and emptied, for the next frame.
    spare: Option<Box<Commit>>,
    /// Drop the next commit, as a render thread awaiting a full one after
    /// a panic aborts a delta (tests, ADR-0032 I4).
    #[cfg(test)]
    pub(crate) abort_next: bool,
    /// Hold each commit until told to apply or drop it, as a render thread
    /// takes one in later than the UI sends it (a recorded log's replay,
    /// which says where it did), and the commit held.
    #[cfg(test)]
    pub(crate) hold: bool,
    #[cfg(test)]
    pub(crate) held: Option<(Box<Commit>, Option<Scene>)>,
}

impl Headless {
    /// What the compositor scrolled since the last pull, for input it
    /// hands the UI now, as the render thread does (ADR-0033).
    pub fn pull(&mut self) -> Pulled {
        self.compositor.pull()
    }

    /// The commit held ([`Self::hold`]) applied now: whether one was.
    #[cfg(test)]
    pub(crate) fn release(&mut self) -> bool {
        let Some((mut commit, overlay)) = self.held.take() else {
            return false;
        };
        // Drawn when it is taken in, on the replay's clock, as every frame
        // drawn while it waited was: the timeline goes forward.
        commit.at = crate::clock::current_nanos();
        let hold = std::mem::replace(&mut self.hold, false);
        self.commit(commit, overlay);
        self.hold = hold;
        true
    }

    /// Take the held commit in without drawing, as the live render thread
    /// takes one in and draws it at its next tick: a replay whose log has
    /// each frame drawn draws there ([`replay`]'s `obs drawn`), so its
    /// frames are the live run's, numbered and timed alike.
    #[cfg(test)]
    pub(crate) fn release_undrawn(&mut self) -> bool {
        let Some((mut commit, overlay)) = self.held.take() else {
            return false;
        };
        self.compositor.apply(&mut commit);
        self.spare = Some(commit);
        self.compositor.set_overlay(overlay);
        true
    }

    #[must_use]
    pub fn new(mut gpu: GpuRenderer, glyphs: Glyphs, read: bool) -> Self {
        // Each frame sees every canvas plan the last one asked for, so runs repeat.
        gpu.set_raster_blocking(true);
        Self {
            compositor: Compositor::new(glyphs),
            gpu: Box::new(gpu),
            read,
            pixels: None,
            sequence: 0,
            ui_late: std::time::Duration::ZERO,
            spare: None,
            #[cfg(test)]
            abort_next: false,
            #[cfg(test)]
            hold: false,
            #[cfg(test)]
            held: None,
        }
    }

    /// Plan main frames as if the UI took `late` to answer each (a test of
    /// a busy UI: the reach of a scroll covers it).
    pub fn set_ui_late(&mut self, late: std::time::Duration) {
        self.ui_late = late;
    }

    /// Scroll as the platform's wheel would, then draw the frame that shows
    /// it at `at`, as the render thread draws one without the UI.
    pub fn wheel(&mut self, at_point: (f32, f32), delta: (f32, f32), at: impl crate::clock::Moment) -> bool {
        let seen = self.shown().seen();
        let scrolled = self.compositor.wheel_input(WheelInput {
            point: at_point,
            delta,
            time: at.nanos(),
            precise: false,
            phase: WheelPhase::None,
            seen,
        }) == Wheeled::Scrolled;
        if scrolled {
            self.draw(at);
        }
        scrolled
    }

    /// Draw a compositor-only frame at `at`.
    pub fn draw(&mut self, at: impl crate::clock::Moment) {
        let c = &mut self.compositor;
        c.tick(at);
        c.compose(at);
        self.gpu.draw_and_wait(&c.shown, &c.changed, &mut c.glyphs);
        c.drew();
        if let Some(o) = &c.overlay {
            self.gpu.overlay(o, &mut c.glyphs);
        }
        if self.read {
            self.pixels = self.gpu.capture(&c.shown, c.overlay.as_ref(), &mut c.glyphs);
        }
        // What was drawn, or read back in bands (a page taller than a
        // texture has no canvas: its capture is what shows).
        c.publish_layers(&self.gpu);
        self.gpu.end_frame();
        // Headless, every frame drawn is shown: nothing fails to present.
        c.did_present();
    }

    /// Whether canvas raster is under way: compositor frames would show more.
    #[must_use]
    pub fn raster_pending(&self) -> bool {
        self.gpu.raster_pending()
    }

    /// The last frame drawn, when it reads frames back.
    #[must_use]
    pub fn pixels(&self) -> Option<&Pixels> {
        self.pixels.as_ref()
    }

    /// What shows: the frame presented last, or nothing yet.
    #[must_use]
    pub fn shown(&self) -> Shown {
        self.compositor.on_screen().map_or(Shown::BeforeFirst, Shown::Frame)
    }

    /// The frame presented last; before any, the last composed.
    #[must_use]
    pub fn presented(&self) -> Arc<Presented> {
        self.compositor
            .on_screen()
            .unwrap_or_else(|| self.compositor.presented())
    }

    /// The frame as last composed, `overlay` over it, painted whole aside
    /// and read back. Headless, what is read back is what shows (a page
    /// taller than a texture has no canvas): it is presented, its layers
    /// with it, a new entry on the timeline; the frame drawn before keeps
    /// its own.
    pub fn capture(&mut self, overlay: Option<&Scene>) -> Option<Pixels> {
        let pixels = self.compositor.capture(&mut self.gpu, overlay);
        self.compositor.did_present();
        pixels
    }
}

impl Compose for Headless {
    fn commit(&mut self, mut commit: Box<Commit>, overlay: Option<Scene>) {
        #[cfg(test)]
        if self.hold {
            self.held = Some((commit, overlay));
            return;
        }
        #[cfg(test)]
        if std::mem::take(&mut self.abort_next) {
            commit.clear();
            self.spare = Some(commit);
            return;
        }
        let at = commit.at;
        self.compositor.apply(&mut commit);
        self.spare = Some(commit);
        self.compositor.set_overlay(overlay);
        self.draw(at);
    }

    fn spare(&mut self) -> Option<Box<Commit>> {
        self.spare.take()
    }

    fn overlay(&mut self, overlay: Option<Scene>) {
        self.compositor.set_overlay(overlay);
    }

    fn held(&self) -> Held {
        self.gpu.held()
    }

    fn drawn(&self) -> Drawn {
        self.gpu.drawn()
    }

    fn capture(&mut self, overlay: Option<Scene>) -> Option<Pixels> {
        Self::capture(self, overlay.as_ref())
    }

    fn waited(&mut self) -> std::time::Duration {
        self.gpu.waited()
    }

    fn presented(&self) -> Shown {
        self.shown()
    }

    fn begin(&mut self, at: u64) -> MainFrame {
        self.sequence = self.sequence.saturating_add(1);
        let frame = MainFrame::at(at);
        let horizon = horizon(self.ui_late, frame.interval);
        MainFrame {
            sequence: self.sequence,
            taken: self.compositor.pull_for_frame(),
            scroll: self.compositor.begin_main_frame(at, horizon),
            ..frame
        }
    }
}

#[cfg(test)]
mod tests;

/// How long after a main frame is granted the compositor may scroll before
/// the commit after the one answering it shows, when the UI takes up to
/// `ui` to answer: this frame's commit is drawn one interval after it comes
/// (a deadline missed rounds it up), and the next one is granted at the
/// vblank after that and takes as long again.
#[must_use]
pub fn horizon(ui: std::time::Duration, interval: std::time::Duration) -> std::time::Duration {
    ui.saturating_mul(2).saturating_add(interval.saturating_mul(3))
}

/// Whether a table the UI sends whole differs (the same `Arc` never does).
/// Whether two overlays show the same: their chunks, drawn at the root
/// state, paint the same items in order (the same displays, or displays
/// recorded again alike).
fn same_overlay(a: &Scene, b: &Scene) -> bool {
    a.size == b.size
        && a.scale.to_bits() == b.scale.to_bits()
        && a.background == b.background
        && a.chunks.len() == b.chunks.len()
        && a.chunks.iter().zip(&b.chunks).all(|(x, y)| {
            x.id == y.id
                && x.state == y.state
                && (Arc::ptr_eq(&x.display, &y.display) || x.display.items == y.display.items)
        })
}

fn replaced<T: PartialEq>(old: &Arc<[T]>, new: &Arc<[T]>) -> bool {
    !Arc::ptr_eq(old, new) && old != new
}
