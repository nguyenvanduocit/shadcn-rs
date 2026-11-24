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
#[cfg(test)]
mod replay;
mod scroll;
pub(crate) mod thread;

use std::sync::Arc;

pub(crate) use latch::WheelGesture;
pub use latch::{LATCH_SLOP, MOMENTUM_GAP, WHEEL_LATCH, WheelStep};

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
    /// The offset as laid out, within `0..=max`.
    pub offset: (f32, f32),
    /// What the offset moved the children by, in whole device pixels and
    /// signed as paint applies it (ADR-0020).
    pub applied: (f32, f32),
    pub max: (f32, f32),
    /// Its offsets run from the right.
    pub rtl: bool,
    /// Bumped when the program (or the UI's own wheel) set the offset:
    /// deltas the compositor scrolled before it are dropped.
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
/// hit-tests a wheel event against, as the UI hit-tests (`interact::hit`).
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
    /// The scroll container it is in (itself, when it is one), by index.
    pub scroller: Option<u32>,
    /// A wheel handler is on it or on an ancestor below [`Self::scroller`]:
    /// the wheel over it goes to the UI, which may consume it.
    pub wheel: bool,
    /// Which node it is: what the UI hands a wheel over it to.
    pub target: HitTarget,
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
    /// When, seconds on the frame clock.
    pub at: f32,
    /// A trackpad's, resampled; else a notched wheel's, applied.
    pub precise: bool,
    /// A trackpad's fingers went down: a new gesture.
    pub began: bool,
    /// A trackpad's fingers lifted (its momentum, if any, follows).
    pub ended: bool,
    /// The frame shown as it came ([`Presented::frame`]).
    pub seen: u64,
}

impl WheelInput {
    /// Its place in its gesture.
    #[must_use]
    pub fn step(&self) -> WheelStep {
        WheelStep {
            point: self.point,
            at: self.at,
            precise: self.precise,
            began: self.began,
            ended: self.ended,
        }
    }
}

/// What a wheel event became.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wheeled {
    /// Handed to the UI: it is over a wheel handler, the node it hands it
    /// to with it.
    Ui(Option<HitTarget>),
    /// The compositor scrolls a container.
    Scrolled,
    Nothing,
}

/// A frame as shown, kept once the next is composed over it: what a wheel
/// event that came while it was on screen is routed in. Kept as what the
/// next compose overwrote of it, made again from the frames after it.
#[derive(Default)]
struct Before {
    frame: u64,
    scale: f32,
    compositing: Compositing,
    /// Each transform and clip the next compose changed, as it was.
    transforms: Vec<(u32, crate::scene::TransformNode)>,
    clips: Vec<(u32, crate::scene::ClipNode)>,
    /// All of it, when the next compose made the tables whole.
    whole: Option<crate::scene::Properties>,
}

/// How many frames a wheel is routed in as it came: the last composed and
/// those shown before it (kept as what the composes after each
/// overwrote, [`Before`]). A wheel stamped with one older is routed in the
/// last composed, as Chrome routes a wheel in its current frame.
const ROUTED: usize = 5;

/// The node a wheel is routed to, as a frame showed it: its compositor key
/// and the component instance that made it ([`crate::interact`]'s node
/// identity). The UI delivers the wheel to that node while it is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

impl Default for Compositing {
    fn default() -> Self {
        Self {
            scrollers: Arc::from(Vec::new()),
            hits: Arc::from(Vec::new()),
            moving: Arc::from(Vec::new()),
        }
    }
}

/// A moving node's transform entry and opacity at `at` seconds, as layout
/// and paint place it (the same code, so a compositor frame at the UI
/// frame's own time draws what the UI recorded).
#[must_use]
pub(crate) fn moving_at(m: &MovingNode, at: f32, scale: f32) -> (TransformNode, f32, crate::style::Painted) {
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

/// `at` (logical window pixels) with every scale and turn from transform
/// `id` up undone, outermost first (the matrices are in physical pixels,
/// `k` per logical one); `None` when one does not invert (a scale of 0).
fn unturned(props: &crate::scene::Properties, id: u32, at: (f32, f32), k: f32) -> Option<(f32, f32)> {
    // `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
    let then = |m: [f32; 6], n: [f32; 6]| {
        let [a, b, c, d, e, f] = m;
        let [na, nb, nc, nd, ne, nf] = n;
        [
            na * a + nc * b,
            nb * a + nd * b,
            na * c + nc * d,
            nb * c + nd * d,
            na * e + nc * f + ne,
            nb * e + nd * f + nf,
        ]
    };
    let mut undo = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut any = false;
    // Innermost first: each one's inverse goes before those found so far.
    for m in props.matrices(id, 0) {
        let [a, b, c, d, e, f] = m.matrix();
        let det = a * d - b * c;
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let inv = [
            d / det,
            -b / det,
            -c / det,
            a / det,
            (c * f - d * e) / det,
            (b * e - a * f) / det,
        ];
        undo = then(inv, undo);
        any = true;
    }
    if !any || k <= 0.0 {
        return Some(at);
    }
    let [a, b, c, d, e, f] = undo;
    let (x, y) = (at.0 * k, at.1 * k);
    Some(((a * x + c * y + e) / k, (b * x + d * y + f) / k))
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
    /// The [`MainFrame`] it answers: its scroll deltas are in it.
    pub answers: u64,
    /// The frame time it was laid out at, seconds on the frame clock.
    pub at: f32,
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
    /// The faces, coordinates and settings the glyphs drawn need.
    pub glyphs: GlyphUpdate,
}

impl Commit {
    /// Empty, its buffers kept for the next frame's.
    pub fn clear(&mut self) {
        self.sequence = 0;
        self.answers = 0;
        self.at = 0.0;
        self.full = false;
        self.reorder = false;
        self.order.clear();
        self.chunks.clear();
        self.properties.clear();
        self.scrollers = None;
        self.hits = None;
        self.moving = None;
        self.glyphs = GlyphUpdate::default();
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

/// The scroll the compositor did since the last [`MainFrame`], for one
/// scroll container: the UI adds it to its own offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollDelta {
    pub key: NodeKey,
    /// With `key`, the container's identity (its [`ScrollNode`]'s owner): a
    /// container another instance replaced at the same path is another.
    pub(crate) owner: Option<crate::reactive::OwnerId>,
    pub delta: (f32, f32),
    /// The scroll generation ([`ScrollNode::generation`]) `delta` was
    /// scrolled from: the UI drops it if it set the offset since, as its
    /// own scroll wins (Chrome's).
    pub generation: u64,
    /// How far, at the speed it scrolls now, the compositor may scroll it
    /// before the commit after this frame's shows (signed as `delta`): the
    /// content a virtual list builds beyond its viewport that way, as
    /// Chromium's interest rect and Flutter's `cacheExtent` (ADR-0015).
    pub reach: (f32, f32),
}

/// Leave to run one UI frame (`BeginMainFrame`): the time it will be
/// presented at, by when the render thread wants its commit, and the scroll
/// done since the last commit. Not `Clone`: one token, one commit.
#[derive(Debug)]
pub struct MainFrame {
    pub sequence: u64,
    /// The predicted present time, seconds on the frame clock.
    pub present_at: f32,
    /// After this the render thread draws without the commit.
    pub deadline: std::time::Instant,
    /// The display's frame interval.
    pub interval: std::time::Duration,
    /// The scroll done since the last main frame, and how far each moving
    /// container may yet go before the next commit after this one shows.
    pub scroll: Vec<ScrollDelta>,
}

impl MainFrame {
    /// A token for a frame shown at `present_at`, with nothing scrolled.
    #[must_use]
    pub fn at(present_at: f32) -> Self {
        Self {
            sequence: 0,
            present_at,
            deadline: std::time::Instant::now(),
            interval: std::time::Duration::from_micros(16_667),
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
    /// When it was to be presented, seconds on the frame clock.
    pub present_at: f32,
    /// Each scroll container as shown.
    pub scroll: Vec<ShownScroll>,
    /// How far each moving node was shown from where its commit placed it.
    pub moved: Vec<(NodeKey, (f32, f32))>,
    /// The scale and turn each moving node was shown at, as sampled then
    /// (its origin from the corner of its border box): input lands in it
    /// as it showed, whatever scale and turn its commit had.
    pub turned: Vec<(NodeKey, crate::style::Painted)>,
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
    committed_at: f32,
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
    /// The frame presented before the last, kept to be the next once
    /// nothing else holds it.
    retired: Option<Arc<Presented>>,
    work: compose::Work,
    /// Each frame is compared with one composed whole (`GUI_COLD_CHECK`).
    cold_check: bool,
    frame: u64,
    presented: Arc<Presented>,
    /// The frames shown before the last composed one, the latest first:
    /// `ROUTED - 1` of them at most.
    ring: std::collections::VecDeque<Box<Before>>,
    /// The wheel gesture under way.
    latch: WheelGesture<Latched, u64>,
    /// Clip chains wheel routing has walked: only those of boxes under
    /// the wheel.
    chain_walks: std::cell::Cell<usize>,
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
            committed_at: 0.0,
            shown: Scene::default(),
            pending: Changed::all(),
            changed: Changed::default(),
            links: compose::Links::default(),
            moved: compose::Moved::default(),
            spare: compose::Sources::default(),
            retired: None,
            work: compose::Work::default(),
            cold_check: cfg!(debug_assertions) && std::env::var_os("GUI_COLD_CHECK").is_some(),
            frame: 0,
            presented: Arc::new(Presented::default()),
            ring: std::collections::VecDeque::new(),
            latch: WheelGesture::default(),
            chain_walks: std::cell::Cell::new(0),
        }
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

    /// Take in the last commit's scroll containers again, as each commit
    /// does: for the test that a commit's scroll bookkeeping allocates
    /// nothing (its own binary counts the heap).
    #[doc(hidden)]
    pub fn recommit_scrollers(&mut self) {
        self.scrolls.committed(&self.scene.compositing.scrollers, self.commit);
    }

    /// The glyph tables it draws with.
    pub fn glyphs(&mut self) -> &mut Glyphs {
        &mut self.glyphs
    }

    /// Take in `commit`, whole, emptying it: what it carried is the
    /// scene's now, and its buffers go back to the UI for the next frame.
    pub fn apply(&mut self, commit: &mut Commit) {
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
        self.scrolls.committed(&scene.compositing.scrollers, commit.answers);
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
    /// table and transforms both, while that frame is kept ([`ROUTED`]).
    pub fn wheel_input(&mut self, w: WheelInput) -> Wheeled {
        let to = self.latched(w);
        let scrollers = &self.scene.compositing.scrollers;
        let target = match to {
            Some(Latched::Ui(target)) => return Wheeled::Ui(target),
            Some(Latched::Scroll(instance)) => scrollers
                .iter()
                .position(|s| s.instance == instance)
                .and_then(|i| u32::try_from(i).ok())
                .map(WheelTarget),
            None => None,
        };
        let moved = if w.precise {
            self.scrolls.gesture(scrollers, target, w.delta, w.at, w.ended)
        } else {
            target.is_some_and(|t| self.scrolls.scroll(scrollers, t, w.delta, w.at))
        };
        if moved { Wheeled::Scrolled } else { Wheeled::Nothing }
    }

    /// What `w` goes to: what its gesture latched to while it goes on,
    /// else what it is routed to, latched for the gesture.
    fn latched(&mut self, w: WheelInput) -> Option<Latched> {
        let compositing = &self.scene.compositing;
        let routed = self.route(w.point, w.delta, w.seen);
        let step = w.step();
        // One rule for every latch, checked on each event against what is
        // there now: its target by identity (a node by key and the
        // instance that made it, a container by the instance it is).
        // Gone, or below a modal layer opened since: the gesture's latch
        // ends, and what it goes on over is routed afresh.
        let held = self
            .latch
            .held(&step, routed.as_ref().map(|r| r.under))
            .copied()
            .filter(|to| match to {
                Latched::Ui(None) => true,
                Latched::Ui(Some(target)) => compositing.hits.iter().any(|h| h.target == *target && !h.below_modal),
                Latched::Scroll(instance) => compositing
                    .scrollers
                    .iter()
                    .any(|s| s.instance == *instance && !s.below_modal),
            });
        let (to, under) = match held {
            Some(to) => (Some(to), None),
            None => routed.map_or((None, None), |r| (r.to, r.under)),
        };
        self.latch.went(&step, under, to, held.is_some());
        to
    }

    /// The hit table, scroll containers, transforms and scale of the frame
    /// numbered `seen` as shown: the last composed, or one before it made
    /// again from what the composes after it overwrote. One older than
    /// those kept is routed in the last composed, as Chrome routes a wheel
    /// in its current frame.
    fn tables(&self, seen: u64) -> (&Compositing, std::borrow::Cow<'_, crate::scene::Properties>, f32) {
        let current = (
            &self.shown.compositing,
            std::borrow::Cow::Borrowed(&self.shown.properties),
            self.shown.scale,
        );
        if seen == 0 || seen >= self.presented.frame {
            return current;
        }
        let Some(k) = self.ring.iter().position(|b| b.frame == seen) else {
            return current;
        };
        let now = &self.shown.properties;
        let mut p = crate::scene::Properties {
            transforms: now.transforms.clone(),
            clips: now.clips.clone(),
            effects: Vec::new(),
        };
        for b in self.ring.iter().take(k.saturating_add(1)) {
            if let Some(whole) = &b.whole {
                p.transforms.clone_from(&whole.transforms);
                p.clips.clone_from(&whole.clips);
                continue;
            }
            for &(t, v) in b.transforms.iter().rev() {
                if let Some(slot) = usize::try_from(t).ok().and_then(|i| p.transforms.get_mut(i)) {
                    *slot = v;
                }
            }
            for &(c, v) in b.clips.iter().rev() {
                if let Some(slot) = usize::try_from(c).ok().and_then(|i| p.clips.get_mut(i)) {
                    *slot = v;
                }
            }
        }
        let b = self.ring.get(k).map_or(&self.shown.compositing, |b| &b.compositing);
        let scale = self.ring.get(k).map_or(self.shown.scale, |b| b.scale);
        (b, std::borrow::Cow::Owned(p), scale)
    }

    /// Whether `node` (as some frame showed it) can scroll by `delta` from
    /// where it is now.
    fn can_scroll(&self, node: &ScrollNode, delta: (f32, f32)) -> bool {
        let now = self
            .scene
            .compositing
            .scrollers
            .iter()
            .find(|s| s.key == node.key)
            .unwrap_or(node);
        let o = self.scrolls.offset(now, 0);
        let along = if now.rtl { -delta.0 } else { delta.0 };
        (delta.1 > 0.0 && o.1 < now.max.1)
            || (delta.1 < 0.0 && o.1 > 0.0)
            || (along > 0.0 && o.0 < now.max.0)
            || (along < 0.0 && o.0 > 0.0)
    }

    /// Where a wheel at `at` moving `delta` that starts a gesture goes, in
    /// the frame numbered `seen`: walking up from the box under it as the
    /// UI's routing does, a wheel handler (the UI's, with the box), or the
    /// innermost container that can still scroll that way; with the
    /// innermost container under it, whatever it does.
    fn route(&self, at: (f32, f32), delta: (f32, f32), seen: u64) -> Option<Routed> {
        let (c, props, k) = self.tables(seen);
        let props = &*props;
        let hit = c.hits.iter().rev().find(|h| {
            // Where the point lies under each scale and turn above the box
            // as shown, as the UI's hit testing maps it: a box under a
            // scale of 0 is hit by nothing.
            let Some(p) = unturned(props, h.transform, at, k) else {
                return false;
            };
            let t = props.transform(h.transform);
            let b = h.rect.translated(t.offset.0, t.offset.1);
            if !crate::shape::takes_pointer(&h.shape, b, h.slop, p.0, p.1) {
                return false;
            }
            if h.clip == 0 {
                return true;
            }
            // Under it: its clip chain, rounded corners included, as paint
            // and the UI's hit testing keep it ([`crate::clip::keeps`]).
            self.chain_walks.set(self.chain_walks.get().saturating_add(1));
            let clip_of = |id: u32| {
                (id != 0)
                    .then(|| usize::try_from(id).ok().and_then(|i| props.clips.get(i)))
                    .flatten()
            };
            let link = |id: u32| {
                let c = clip_of(id)?;
                let b = c.clip.bounds;
                Some(crate::clip::Link {
                    rect: b,
                    radius: crate::shape::Shape::Rounded(c.clip.radius).used(b.width, b.height),
                    curve: c.clip.curve,
                    next: (c.parent != 0).then_some(c.parent),
                })
            };
            let to_local = |id: u32, q: (f32, f32)| unturned(props, clip_of(id)?.transform, q, k);
            crate::clip::keeps(Some(h.clip), link, to_local, props.clips.len(), at.0, at.1) == Some(true)
        })?;
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
        routed(None)
    }

    /// The scroll done since the last [`MainFrame`], handed to one with
    /// sequence `sequence` granted at `at` (and kept until a commit
    /// answering it), with how far each moving container may go within
    /// `horizon`: until the commit after the one answering it shows.
    pub fn begin_main_frame(&mut self, sequence: u64, at: f32, horizon: std::time::Duration) -> Vec<ScrollDelta> {
        self.scrolls
            .send(&self.scene.compositing.scrollers, sequence, at, horizon.as_secs_f32())
    }

    /// Whether anything it moves moves at `at` (scroll prediction, a fling,
    /// a motion), so frames go on without the UI.
    #[must_use]
    pub fn animating(&self, at: f32) -> bool {
        self.scrolls.animating(at) || !self.scene.compositing.moving.is_empty()
    }

    /// Whether it scrolled anything the UI has not been handed yet: the UI
    /// is owed a main frame to take it in (and to build what it scrolls to).
    #[must_use]
    pub fn scrolled(&self) -> bool {
        self.scrolls.pending()
    }

    /// Advance scroll prediction and flings to `at` (seconds on the frame clock).
    pub fn tick(&mut self, at: f32) {
        self.scrolls.tick(&self.scene.compositing.scrollers, at);
    }

    /// The scene to draw at `at`: as committed, with the scroll it did and
    /// the motion it runs written over its property trees. What of it
    /// changed since the renderer last drew is kept for [`Self::draw`].
    pub fn compose(&mut self, at: f32) -> &Scene {
        self.compose_into_shown(at);
        self.frame = self.frame.saturating_add(1);
        &self.shown
    }

    /// What the last composed frame showed.
    #[must_use]
    pub fn presented(&self) -> Arc<Presented> {
        Arc::clone(&self.presented)
    }

    /// Compose the frame at `at` and draw it with `gpu`, the overlay over it.
    pub fn draw(&mut self, gpu: &mut GpuRenderer, at: f32) {
        self.tick(at);
        self.compose(at);
        gpu.render(&self.shown, &self.changed, &mut self.glyphs);
        self.changed.clear();
        if let Some(o) = &self.overlay {
            gpu.overlay_as_of(o, self.overlay_generation, &mut self.glyphs);
        }
    }

    /// The frame as last composed, `overlay` over it, painted whole aside and read back.
    pub fn capture(&mut self, gpu: &mut GpuRenderer, overlay: Option<&Scene>) -> Option<Pixels> {
        gpu.capture(&self.shown, overlay, &mut self.glyphs)
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
    /// What the last frame showed.
    fn presented(&self) -> Arc<Presented>;
    /// A token for the next UI frame shown at `at`: the scroll done since.
    fn begin(&mut self, at: f32) -> MainFrame;
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
}

impl Headless {
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
        }
    }

    /// Plan main frames as if the UI took `late` to answer each (a test of
    /// a busy UI: the reach of a scroll covers it).
    pub fn set_ui_late(&mut self, late: std::time::Duration) {
        self.ui_late = late;
    }

    /// Scroll as the platform's wheel would, then draw the frame that shows
    /// it at `at`, as the render thread draws one without the UI.
    pub fn wheel(&mut self, at_point: (f32, f32), delta: (f32, f32), at: f32) -> bool {
        let seen = self.compositor.presented.frame;
        let scrolled = self.compositor.wheel_input(WheelInput {
            point: at_point,
            delta,
            at,
            precise: false,
            began: false,
            ended: false,
            seen,
        }) == Wheeled::Scrolled;
        if scrolled {
            self.draw(at);
        }
        scrolled
    }

    /// Draw a compositor-only frame at `at`.
    pub fn draw(&mut self, at: f32) {
        let c = &mut self.compositor;
        c.tick(at);
        c.compose(at);
        self.gpu.draw_and_wait(&c.shown, &c.changed, &mut c.glyphs);
        c.changed.clear();
        if let Some(o) = &c.overlay {
            self.gpu.overlay(o, &mut c.glyphs);
        }
        if self.read {
            self.pixels = self.gpu.capture(&c.shown, c.overlay.as_ref(), &mut c.glyphs);
        }
        self.gpu.end_frame();
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
}

impl Compose for Headless {
    fn commit(&mut self, mut commit: Box<Commit>, overlay: Option<Scene>) {
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
        self.compositor.capture(&mut self.gpu, overlay.as_ref())
    }

    fn waited(&mut self) -> std::time::Duration {
        self.gpu.waited()
    }

    fn presented(&self) -> Arc<Presented> {
        self.compositor.presented()
    }

    fn begin(&mut self, at: f32) -> MainFrame {
        self.sequence = self.sequence.saturating_add(1);
        let frame = MainFrame::at(at);
        let horizon = horizon(self.ui_late, frame.interval);
        MainFrame {
            sequence: self.sequence,
            scroll: self.compositor.begin_main_frame(self.sequence, at, horizon),
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
