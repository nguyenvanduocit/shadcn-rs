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
#[cfg(test)]
mod replay;
mod scroll;
pub(crate) mod thread;

use std::sync::Arc;

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
}

/// A scrollbar thumb the compositor moves with its scroll offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThumbNode {
    pub transform: u32,
    /// The thumb's offset at the top.
    pub top: (f32, f32),
    /// How far it travels from the top to the bottom.
    pub travel: f32,
}

/// A box that takes the pointer, in painting order: what the compositor
/// hit-tests a wheel event against, as the UI hit-tests (`interact::hit`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Its border box in its paint space.
    pub rect: Bounds,
    pub transform: u32,
    /// The clip it is hit inside, 0 for none.
    pub clip: u32,
    /// The scroll container it is in (itself, when it is one), by index.
    pub scroller: Option<u32>,
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
pub(crate) fn moving_at(m: &MovingNode, at: f32, scale: f32) -> (TransformNode, f32) {
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
    fn between(old: &Properties, new: &Properties) -> Self {
        fn changed<T: Clone + PartialEq>(old: &[T], new: &[T]) -> Vec<(u32, T)> {
            new.iter()
                .enumerate()
                .filter(|(i, t)| old.get(*i) != Some(*t))
                .map(|(i, t)| (u32::try_from(i).unwrap_or(u32::MAX), t.clone()))
                .collect()
        }
        Self {
            lens: (new.transforms.len(), new.clips.len(), new.effects.len()),
            transforms: changed(&old.transforms, &new.transforms),
            clips: changed(&old.clips, &new.clips),
            effects: changed(&old.effects, &new.effects),
        }
    }

    fn apply(self, to: &mut Properties) {
        fn put<T: Clone + Default>(v: &mut Vec<T>, len: usize, changed: Vec<(u32, T)>) {
            v.resize(len, T::default());
            for (i, t) in changed {
                if let Some(slot) = usize::try_from(i).ok().and_then(|i| v.get_mut(i)) {
                    *slot = t;
                }
            }
        }
        put(&mut to.transforms, self.lens.0, self.transforms);
        put(&mut to.clips, self.lens.1, self.clips);
        put(&mut to.effects, self.lens.2, self.effects);
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
                        o.parent != e.parent
                            || o.transform != e.transform
                            || o.transformed != e.transformed
                            || o.clip != e.clip
                    })
                })
            })
    }
}

/// One UI frame for the render thread: what changed since the last commit
/// (ADR-0015). Applied whole, in sequence.
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
    /// The chunks' ids in painting order, when that changed.
    pub order: Option<Vec<ChunkId>>,
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
    /// What turns the scene `old` (as last committed) into `new`.
    #[must_use]
    pub fn between(old: &Scene, new: &Scene) -> Self {
        let same_order =
            old.chunks.len() == new.chunks.len() && old.chunks.iter().zip(&new.chunks).all(|(a, b)| a.id == b.id);
        let kept: foldhash::HashMap<ChunkId, &Chunk> = old.chunks.iter().map(|c| (c.id, c)).collect();
        let chunks = new
            .chunks
            .iter()
            .filter(|c| {
                kept.get(&c.id)
                    .is_none_or(|k| k.state != c.state || !Arc::ptr_eq(&k.display, &c.display))
            })
            .cloned()
            .collect();
        let (oc, nc) = (&old.compositing, &new.compositing);
        Self {
            sequence: 0,
            answers: 0,
            at: 0.0,
            full: false,
            size: new.size,
            scale: new.scale,
            background: new.background,
            order: (!same_order).then(|| new.chunks.iter().map(|c| c.id).collect()),
            chunks,
            properties: PropertiesDelta::between(&old.properties, &new.properties),
            scrollers: replaced(&oc.scrollers, &nc.scrollers).then(|| Arc::clone(&nc.scrollers)),
            hits: replaced(&oc.hits, &nc.hits).then(|| Arc::clone(&nc.hits)),
            moving: replaced(&oc.moving, &nc.moving).then(|| Arc::clone(&nc.moving)),
            glyphs: GlyphUpdate::default(),
        }
    }

    /// What a record in place changed of `scene`: the chunks recorded
    /// again or under another state, the property entries written, and the
    /// compositor's tables where they differ from `before` (ADR-0013: what
    /// a frame visited, never a comparison of the whole scene).
    #[must_use]
    pub(crate) fn changed(
        scene: &Scene,
        before: &Compositing,
        chunks: Vec<Chunk>,
        properties: PropertiesDelta,
    ) -> Self {
        let nc = &scene.compositing;
        Self {
            sequence: 0,
            answers: 0,
            at: 0.0,
            full: false,
            size: scene.size,
            scale: scene.scale,
            background: scene.background,
            order: None,
            chunks,
            properties,
            scrollers: replaced(&before.scrollers, &nc.scrollers).then(|| Arc::clone(&nc.scrollers)),
            hits: replaced(&before.hits, &nc.hits).then(|| Arc::clone(&nc.hits)),
            moving: replaced(&before.moving, &nc.moving).then(|| Arc::clone(&nc.moving)),
            glyphs: GlyphUpdate::default(),
        }
    }

    /// Whether it changes nothing the compositor draws or moves.
    #[must_use]
    pub fn is_empty(&self, last: &Scene) -> bool {
        self.order.is_none()
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
    pub delta: (f32, f32),
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
}

/// A scroll container as a frame showed it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShownScroll {
    pub key: NodeKey,
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
    /// Drawn over every frame until replaced (the debug overlay, the HUD).
    overlay: Option<Scene>,
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
    work: compose::Work,
    /// Each frame is compared with one composed whole (`GUI_COLD_CHECK`).
    cold_check: bool,
    frame: u64,
    presented: Arc<Presented>,
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
            commit: 0,
            committed_at: 0.0,
            shown: Scene::default(),
            pending: Changed::all(),
            changed: Changed::default(),
            links: compose::Links::default(),
            moved: compose::Moved::default(),
            work: compose::Work::default(),
            cold_check: std::env::var_os("GUI_COLD_CHECK").is_some(),
            frame: 0,
            presented: Arc::new(Presented::default()),
        }
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

    /// Take in `commit`, whole.
    pub fn apply(&mut self, commit: Commit) {
        let Commit {
            sequence,
            answers,
            at,
            full,
            size,
            scale,
            background,
            order,
            chunks,
            properties,
            scrollers,
            hits,
            moving,
            glyphs,
        } = commit;
        self.glyphs.apply(glyphs);
        let scene = &mut self.scene;
        let pending = &mut self.pending;
        if full || order.is_some() || size != scene.size || scale != scene.scale || background != scene.background {
            pending.all = true;
        }
        if !pending.all && properties.reshapes(&scene.properties) {
            pending.all = true;
        }
        if !pending.all {
            pending.transforms.extend(properties.transforms.iter().map(|(i, _)| *i));
            pending.clips.extend(properties.clips.iter().map(|(i, _)| *i));
            pending.effects.extend(properties.effects.iter().map(|(i, _)| *i));
        }
        scene.size = size;
        scene.scale = scale;
        scene.background = background;
        if let Some(order) = order {
            let mut by_id: foldhash::HashMap<ChunkId, Chunk> = std::mem::take(&mut scene.chunks)
                .into_iter()
                .map(|c| (c.id, c))
                .collect();
            for c in chunks {
                by_id.insert(c.id, c);
            }
            scene.chunks = order.iter().filter_map(|id| by_id.remove(id)).collect();
            self.index = scene.chunks.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        } else {
            for c in chunks {
                if let Some(&i) = self.index.get(&c.id)
                    && let Some(slot) = scene.chunks.get_mut(i)
                {
                    *slot = c;
                    pending.chunks.push(i);
                }
            }
        }
        properties.apply(&mut scene.properties);
        if let Some(s) = scrollers {
            scene.compositing.scrollers = s;
        }
        if let Some(h) = hits {
            scene.compositing.hits = h;
        }
        if let Some(m) = moving {
            scene.compositing.moving = m;
        }
        self.scrolls.committed(&scene.compositing.scrollers, answers);
        self.commit = sequence;
        self.committed_at = at;
    }

    /// Replace what is drawn over every frame.
    pub fn set_overlay(&mut self, overlay: Option<Scene>) {
        self.overlay = overlay;
    }

    /// Scroll by `delta` (pixels, positive moves the content up and left)
    /// the scroll container under `at` (logical window pixels) that can
    /// scroll that way, as the UI's wheel would pick it; whether one did.
    /// `when` is the event's time, seconds on the frame clock.
    pub fn wheel(&mut self, at: (f32, f32), delta: (f32, f32), when: f32) -> bool {
        let Some(target) = self.wheel_target(at, delta) else {
            return false;
        };
        self.scrolls
            .scroll(&self.scene.compositing.scrollers, target, delta, when)
    }

    /// A trackpad's wheel event at `point` moving `delta`, at `at` seconds
    /// on the frame clock: resampled at each frame's present time, and
    /// flung on once `ended` where the platform does not fling.
    pub fn gesture(&mut self, point: (f32, f32), delta: (f32, f32), at: f32, ended: bool) -> bool {
        let target = self.wheel_target(point, delta);
        self.scrolls
            .gesture(&self.scene.compositing.scrollers, target, delta, at, ended);
        target.is_some()
    }

    /// The scroll container a wheel at `at` moving `delta` scrolls.
    fn wheel_target(&self, at: (f32, f32), delta: (f32, f32)) -> Option<WheelTarget> {
        let c = &self.scene.compositing;
        let props = &self.shown.properties;
        let hit = c.hits.iter().rev().find(|h| {
            let t = props.transform(h.transform);
            let inside = h.rect.translated(t.offset.0, t.offset.1).contains(at.0, at.1);
            let clipped = props.clip(h.clip).is_some_and(|c| !c.clip.bounds.contains(at.0, at.1));
            inside && !clipped
        })?;
        let mut s = hit.scroller;
        while let Some(i) = s {
            let node = c.scrollers.get(usize::try_from(i).ok()?)?;
            if (delta.1 != 0.0 && node.max.1 > 0.0) || (delta.0 != 0.0 && node.max.0 > 0.0) {
                return Some(WheelTarget(i));
            }
            s = node.parent;
        }
        None
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
            gpu.overlay(o, &mut self.glyphs);
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
    fn commit(&mut self, commit: Commit, overlay: Option<Scene>);
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
        let scrolled = self.compositor.wheel(at_point, delta, at);
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
    fn commit(&mut self, commit: Commit, overlay: Option<Scene>) {
        let at = commit.at;
        self.compositor.apply(commit);
        self.compositor.set_overlay(overlay);
        self.draw(at);
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
fn replaced<T: PartialEq>(old: &Arc<[T]>, new: &Arc<[T]>) -> bool {
    !Arc::ptr_eq(old, new) && old != new
}
