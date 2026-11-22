//! Recording a [`Frame`] as a [`Scene`] (ADR-0016): one paint chunk per
//! node (its shadows, rounded background and border, focus ring, text, icon,
//! paths or text input) and one per scrollbar thumb, each recorded in its
//! node's paint space, and the property trees the chunks paint under:
//! transforms (scroll offsets, moving translations, scales and turns), clips
//! (one per clipping node, shared by everything it clips) and effects
//! (opacity, masks, filters).
//!
//! A chunk is kept across frames and recorded again only when what it is
//! recorded from changed, which is how Blink's style diff splits a change
//! (`StyleDifference`): a node whose box size changed was laid out again
//! (`NeedsLayout`), one whose paint changed is recorded again (`NeedsPaint`),
//! and one whose offset, opacity, transform or clip alone changed writes a
//! property tree entry (`NeedsProperty`) and records nothing. Scrolling and
//! opacity or transform animation therefore record nothing again. A chunk
//! records what lies within 4000 pixels of its clip (Chromium's cull rect,
//! `kPixelDistanceToRecord`), and again only when its clip leaves that.
//! Box edges snap to device pixels only where the transform to the screen
//! keeps the pixel grid (ADR-0020). What the chunks become in pixels is the
//! renderer's ([`crate::gpu`]).

use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use crate::clock;
use crate::color::Color;
use crate::compositor::{Commit, PropertiesDelta};
use crate::compositor::{Compositing, Hit, MovingNode, NodeKey, ScrollNode, ThumbNode};
use crate::effects;
use crate::geometry::{Bounds, NodePath};
use crate::inspect::{PaintStats, Repaint};
use crate::layout::{Changed, Content, Frame, FrameNode, Look, ReadAt, input_text, text_width};
use crate::scene::{
    Chunk, ChunkId, Clip, ClipNode, Display, EffectNode, GroupTransform, Item, Properties, Scene, State, TransformNode,
};
use crate::style::{Corners, Edges, ResolvedText};
use crate::text::{Paragraph, Target, TextSystem, physical, physical_pos};

/// The caret shows for this long, then hides for as long.
const BLINK: f32 = 0.53;

/// How far past its clip a chunk records (Chromium's
/// `kPixelDistanceToRecord`): a scroll within it records nothing again.
const CULL_MARGIN: f32 = 4000.0;

/// What recording keeps across frames: each node's chunk, the ids of the
/// property tree nodes, and the caret's blink timer.
#[derive(Default)]
pub struct Painter {
    blink: Option<clock::TimerId>,
    chunks: foldhash::HashMap<ChunkId, Retained>,
    transforms: Ids,
    clips: Ids,
    effects: Ids,
    frame: u64,
    stats: PaintStats,
    /// Where the last [`Painter::record`] recorded again or moved what it kept.
    painted: Vec<Repaint>,
    /// The tables of what the compositor moves, as last recorded: kept
    /// (the same `Arc`s) while equal, so a commit leaves them out.
    compositing: Compositing,
    /// The thumbs placed, by scroll container.
    thumb_nodes: foldhash::HashMap<NodeKey, ThumbNode>,
    /// The scene as last recorded (and committed).
    scene: Scene,
    /// Where it read the frame's changes up to, and what it recorded for.
    read: ReadAt,
    target: Option<((u32, u32), f32, u64, Color)>,
    /// Per frame node, as last recorded: its property state, where its
    /// subtree ends, its chunk's and its thumb's place in the scene, whether
    /// its effect's region is its subtree's, its hit box.
    placed: Vec<Placed>,
    ends: Vec<usize>,
    chunk_of: Vec<Option<usize>>,
    thumb_of: Vec<Option<usize>>,
    /// Per frame node, its overlay chunk's place in the scene (a border
    /// over its content, painted after its subtree).
    overlay_of: Vec<Option<usize>>,
    region: Vec<bool>,
    hit_rect: Vec<Option<Bounds>>,
    /// Focused inputs, whose caret blinks.
    blinkers: Vec<usize>,
    /// Scratch: the nodes a record visits.
    work: Vec<usize>,
}

/// A chunk kept across frames, and what it was recorded from.
struct Retained {
    input: Input,
    display: Arc<Display>,
    /// The text pieces it shows, kept shaped while it lives.
    pieces: Vec<u64>,
    /// Its properties as last painted: a change alone is `NeedsProperty`.
    properties: Moved,
    seen: u64,
}

/// What a chunk is recorded from.
enum Input {
    /// A node's box and content, as in its paint space.
    Node {
        node: Box<FrameNode>,
        snap: bool,
        caret: bool,
        scale: f32,
        text: u64,
        /// What it recorded within (its space), `None` all of it.
        cull: Option<Bounds>,
    },
    /// A scroll container's thumb: its box (in the container's space), how
    /// far it scrolls, its colour.
    Thumb {
        space: Bounds,
        max: (f32, f32),
        thumb: Color,
        scale: f32,
    },
}

/// What may change about a node without recording it again.
#[derive(Clone, Copy, PartialEq)]
struct Moved {
    offset: (f32, f32),
    opacity: f32,
    scale: f32,
    rotate: f32,
    clip: Option<Bounds>,
}

impl Moved {
    fn of(node: &FrameNode) -> Self {
        Self {
            offset: node.offset,
            opacity: node.look.opacity,
            scale: node.look.transform.scale,
            rotate: node.look.transform.rotate,
            clip: node.clip,
        }
    }
}

/// Ids kept for keys across frames, freed when a frame no longer has them.
#[derive(Default)]
struct Ids {
    by_key: foldhash::HashMap<u64, (u32, u64)>,
    free: Vec<u32>,
    /// One past the highest id handed out.
    next: u32,
}

impl Ids {
    fn id(&mut self, key: u64, frame: u64) -> u32 {
        if let Some((id, seen)) = self.by_key.get_mut(&key) {
            *seen = frame;
            return *id;
        }
        let id = self.free.pop().unwrap_or_else(|| {
            // 0 is the root.
            self.next = self.next.max(1).saturating_add(1);
            self.next.saturating_sub(1)
        });
        self.by_key.insert(key, (id, frame));
        id
    }

    /// Free the ids `frame` did not ask for.
    fn sweep(&mut self, frame: u64) {
        let free = &mut self.free;
        self.by_key.retain(|_, (id, seen)| {
            let keep = *seen == frame;
            if !keep {
                free.push(*id);
            }
            keep
        });
    }

    fn len(&self) -> usize {
        usize::try_from(self.next.max(1)).unwrap_or(1)
    }
}

/// What a key names for a node.
#[derive(Hash)]
enum Role {
    Chunk,
    /// What it paints over its subtree.
    Overlay,
    Thumb,
    /// The transform of its own moving translation, scale or turn.
    Transform,
    /// The transform its children paint under: its scroll offset.
    Content,
    ThumbTransform,
    /// The thumb's opacity, which the compositor fades.
    ThumbEffect,
    Clip,
    Effect,
}

fn key(path: &NodePath, role: Role) -> u64 {
    // The path's own key (one word), with the role: a fixed seed, so a
    // chunk's id is the same in every frame.
    let mut h = foldhash::fast::FixedState::with_seed(0x9a1e_7c0d).build_hasher();
    path.key().hash(&mut h);
    role.hash(&mut h);
    h.finish()
}

/// Where each node's subtree ends in the frame (one past its last node).
fn subtree_ends(frame: &Frame) -> Vec<usize> {
    let n = frame.nodes.len();
    let mut ends: Vec<usize> = (1..=n).collect();
    for i in (0..n).rev() {
        let end = ends.get(i).copied().unwrap_or(i);
        if let Some(p) = frame.nodes.get(i).and_then(|node| node.parent)
            && let Some(e) = ends.get_mut(p)
        {
            *e = (*e).max(end);
        }
    }
    ends
}

/// Whether the look paints the same, its opacity, transform, mask and
/// filters aside (properties of the effect and transform trees).
fn same_look(a: &Look, b: &Look) -> bool {
    a.background == b.background
        && a.border == b.border
        && a.border_color == b.border_color
        && a.shape == b.shape
        && a.shadows == b.shadows
        && a.ring == b.ring
        && a.decoration == b.decoration
        && a.invisible == b.invisible
        && a.effects.background_image == b.effects.background_image
        && a.text == b.text
}

/// The node as its chunk records it: in its paint space, unclipped (its
/// clip is the chunk's state), its properties aside.
fn in_space(node: &FrameNode) -> FrameNode {
    let mut look = node.look.clone();
    look.opacity = 1.0;
    look.transform = crate::style::Painted {
        scale: 1.0,
        rotate: 0.0,
        origin: (0.0, 0.0),
    };
    look.effects.filters.clear();
    look.effects.mask = None;
    FrameNode {
        path: node.path.clone(),
        name: node.name,
        bounds: node.space,
        content_box: node.space_content,
        space: node.space,
        space_content: node.space_content,
        offset: (0.0, 0.0),
        translating: false,
        moving: None,
        clip: None,
        clip_radius: Corners::default(),
        clip_curve: crate::shape::ROUND,
        clipper: None,
        layer: node.layer,
        parent: None,
        look,
        content: node.content.clone(),
        scroll: None,
    }
}

/// Whether the caret of a text input shows now.
fn caret_on(node: &FrameNode) -> bool {
    match &node.content {
        Content::Input { focused, mode, .. } => {
            *focused && !mode.read_only && (clock::frame_time() / BLINK).floor().rem_euclid(2.0) < 1.0
        }
        _ => false,
    }
}

/// What the property trees and the chunk of each node are, per node.
#[derive(Clone, Copy, Default, PartialEq)]
struct Placed {
    /// The transform it paints under, and the one its children do.
    transform: u32,
    content: u32,
    effect: u32,
    clip: u32,
    /// Its edges snap: no scale or turn above it.
    snap: bool,
    /// The clip a transformed group above it applies once transformed.
    lifted: Option<u32>,
}

/// Where [`Painter::place`] writes property entries: the table being made
/// (a whole record), or the scene's own, with what each write replaced so a
/// record that cannot go on in place puts it back.
struct Writes<'a> {
    props: &'a mut Properties,
    undo: Option<&'a mut Vec<Undo>>,
}

/// A property entry as it was before an in-place write.
enum Undo {
    Transform(u32, Option<TransformNode>),
    Clip(u32, Option<ClipNode>),
    Effect(u32, Option<EffectNode>),
}

impl Writes<'_> {
    fn transform(&mut self, id: u32, t: TransformNode) {
        let old = put(&mut self.props.transforms, id, t);
        if let (Some(u), Some(old)) = (&mut self.undo, old) {
            u.push(Undo::Transform(id, old));
        }
    }

    fn clip(&mut self, id: u32, c: ClipNode) {
        let old = put(&mut self.props.clips, id, c);
        if let (Some(u), Some(old)) = (&mut self.undo, old) {
            u.push(Undo::Clip(id, old));
        }
    }

    fn effect(&mut self, id: u32, e: EffectNode) {
        let old = put(&mut self.props.effects, id, e);
        if let (Some(u), Some(old)) = (&mut self.undo, old) {
            u.push(Undo::Effect(id, old));
        }
    }
}

/// Write `t` at `id`; what was there when it differs (`Some(None)` past the end).
fn put<T: Default + Clone + PartialEq>(v: &mut Vec<T>, id: u32, t: T) -> Option<Option<T>> {
    let i = usize::try_from(id).ok()?;
    if let Some(slot) = v.get_mut(i) {
        if *slot == t {
            return None;
        }
        return Some(Some(std::mem::replace(slot, t)));
    }
    v.resize(i.saturating_add(1), T::default());
    if let Some(slot) = v.get_mut(i) {
        *slot = t;
    }
    Some(None)
}

/// What a record that started in place found it cannot do there (a chunk
/// or thumb appears or goes, an input gains or loses focus): the frame is
/// recorded whole.
struct Whole;

impl Placed {
    /// The state its chunk paints under.
    fn state(self) -> State {
        State {
            transform: self.transform,
            clip: if self.lifted == Some(self.clip) { 0 } else { self.clip },
            effect: self.effect,
        }
    }
}

/// A focused input: its caret blinks.
fn is_blinker(node: &FrameNode) -> bool {
    matches!(node.content, Content::Input { focused: true, .. })
}

/// The box the compositor hit-tests a node by: its border box in its
/// paint space, grown by its hit slop.
fn hit_box(node: &FrameNode) -> Bounds {
    let e = node.look.hit_slop;
    let b = node.space;
    Bounds {
        x: b.x - e.left,
        y: b.y - e.top,
        width: (b.width + e.left + e.right).max(0.0),
        height: (b.height + e.top + e.bottom).max(0.0),
    }
}

/// Whether the node paints over its subtree.
fn has_overlay(node: &FrameNode) -> bool {
    node.look.decoration.overlay.is_some()
}

/// A scroll container that shows a thumb.
fn has_thumb(node: &FrameNode) -> bool {
    node.scroll.is_some_and(|s| s.max.1 > 0.0 && !s.thumb.is_transparent())
}

/// The entries of `props` that in-place writes (`undo`) changed.
fn written(props: &Properties, undo: &[Undo]) -> PropertiesDelta {
    fn of<T: Clone>(v: &[T], mut ids: Vec<u32>) -> Vec<(u32, T)> {
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
            .filter_map(|id| Some((id, v.get(usize::try_from(id).ok()?)?.clone())))
            .collect()
    }
    let (mut t, mut c, mut e) = (Vec::new(), Vec::new(), Vec::new());
    for u in undo {
        match u {
            Undo::Transform(id, _) => t.push(*id),
            Undo::Clip(id, _) => c.push(*id),
            Undo::Effect(id, _) => e.push(*id),
        }
    }
    PropertiesDelta {
        lens: (props.transforms.len(), props.clips.len(), props.effects.len()),
        transforms: of(&props.transforms, t),
        clips: of(&props.clips, c),
        effects: of(&props.effects, e),
    }
}

/// Put back what an in-place write replaced.
fn restore<T: Default>(v: &mut [T], id: u32, old: Option<T>) {
    if let Some(slot) = usize::try_from(id).ok().and_then(|i| v.get_mut(i)) {
        *slot = old.unwrap_or_default();
    }
}

impl Painter {
    /// Record `frame` over `background`, `size` physical pixels at `scale`
    /// physical pixels per logical pixel, and say what changed since the
    /// last record as a commit (ADR-0015); with `resend`, the whole scene.
    ///
    /// Only the nodes the frame says changed since the last record are
    /// visited (ADR-0013), with the subtrees whose property state moved
    /// and the groups whose region holds them: a clean subtree is not
    /// walked, its chunks stand as kept. A frame rebuilt since (nodes came,
    /// went or moved) is walked whole, reusing every chunk whose input
    /// did not change.
    pub fn record(
        &mut self,
        frame: &Frame,
        text: &mut TextSystem,
        size: (u32, u32),
        scale: f32,
        background: Color,
        resend: bool,
    ) -> Commit {
        self.frame = self.frame.saturating_add(1);
        self.stats = PaintStats::default();
        self.painted.clear();
        let target = (size, scale, text.generation(), background);
        let log = frame.changes.since(&mut self.read);
        let same = self.target == Some(target) && self.placed.len() == frame.nodes.len();
        self.target = Some(target);
        let commit = match log.filter(|_| same) {
            Some(log) => self.record_changes(frame, log, text, size, scale).ok(),
            None => None,
        };
        let mut commit = commit.unwrap_or_else(|| self.record_all(frame, text, size, scale, background));
        if resend {
            commit = Commit::between(&Scene::default(), &self.scene);
        }
        self.stats.chunks = self.scene.chunks.len();
        commit
    }

    /// The scene as last recorded.
    #[must_use]
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Record the whole frame (reusing each chunk whose input is the same).
    fn record_all(
        &mut self,
        frame: &Frame,
        text: &mut TextSystem,
        size: (u32, u32),
        scale: f32,
        background: Color,
    ) -> Commit {
        let now = self.frame;
        let mut scene = Scene::new(size, scale, Some(background));
        let mut props = Properties::default();
        self.ends = subtree_ends(frame);
        let n = frame.nodes.len();
        self.placed.clear();
        self.chunk_of.clear();
        self.chunk_of.resize(n, None);
        self.thumb_of.clear();
        self.thumb_of.resize(n, None);
        self.overlay_of.clear();
        self.overlay_of.resize(n, None);
        // Nodes whose overlay paints once their subtree has.
        let mut over: Vec<(usize, Placed)> = Vec::new();
        self.region.clear();
        self.region.resize(n, false);
        self.blinkers.clear();
        self.thumb_nodes.clear();
        let mut thumbs: Vec<usize> = Vec::new();
        let generation = text.generation();
        let mut layer = frame.nodes.first().map_or(0, |n| n.layer);
        for (i, node) in frame.nodes.iter().enumerate() {
            self.overlays(frame, &mut over, Some(i), &mut scene, scale, generation, text);
            if node.layer != layer {
                // A layer's thumbs paint over all of it.
                self.thumbs(
                    frame,
                    &thumbs,
                    &mut scene,
                    &mut Writes {
                        props: &mut props,
                        undo: None,
                    },
                    scale,
                    true,
                );
                thumbs.clear();
                layer = node.layer;
            }
            self.stats.visited = self.stats.visited.saturating_add(1);
            let up = node.parent.and_then(|p| self.placed.get(p).copied()).unwrap_or(Placed {
                snap: true,
                ..Placed::default()
            });
            let p = self.place(
                frame,
                node,
                i,
                up,
                &mut Writes {
                    props: &mut props,
                    undo: None,
                },
                scale,
                size,
            );
            self.placed.push(p);
            if is_blinker(node) {
                self.blinkers.push(i);
            }
            if has_thumb(node) {
                thumbs.push(i);
            }
            let display = self.chunk(node, Role::Chunk, p.snap, scale, generation, text);
            if !display.items.is_empty() {
                if let Some(c) = self.chunk_of.get_mut(i) {
                    *c = Some(scene.chunks.len());
                }
                scene.chunks.push(Chunk {
                    id: ChunkId(key(&node.path, Role::Chunk)),
                    state: p.state(),
                    display,
                });
            }
            if has_overlay(node) {
                over.push((i, p));
            }
        }
        self.overlays(frame, &mut over, None, &mut scene, scale, generation, text);
        self.thumbs(
            frame,
            &thumbs,
            &mut scene,
            &mut Writes {
                props: &mut props,
                undo: None,
            },
            scale,
            true,
        );
        let (scrollers, hits, moving) = self.tables(frame);
        self.keep_compositing(scrollers, hits, moving);
        scene.compositing = self.compositing.clone();
        // What left the frame lets go of its text and ids.
        self.chunks.retain(|_, r| {
            let keep = r.seen == now;
            if !keep {
                text.unpin(&r.pieces);
            }
            keep
        });
        self.transforms.sweep(now);
        self.clips.sweep(now);
        self.effects.sweep(now);
        props.transforms.resize(self.transforms.len(), TransformNode::default());
        props.clips.resize(self.clips.len(), ClipNode::default());
        props.effects.resize(self.effects.len(), EffectNode::default());
        scene.properties = props;
        self.schedule_blink(!self.blinkers.is_empty());
        let commit = Commit::between(&self.scene, &scene);
        self.scene = scene;
        commit
    }

    /// Record in place what `log` says changed since the last record: each
    /// such node's property entries and chunk, the subtrees whose property
    /// state moved with them, the groups whose region holds them, and the
    /// inputs whose caret blinked. `Err` leaves the scene as it was.
    fn record_changes(
        &mut self,
        frame: &Frame,
        log: &[Changed],
        text: &mut TextSystem,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Commit, Whole> {
        let generation = text.generation();
        let mut work = std::mem::take(&mut self.work);
        work.clear();
        for c in log {
            let i = usize::try_from(c.index).unwrap_or(usize::MAX);
            work.push(i);
            // A group whose region is its subtree's holds what moved in it.
            for a in frame.ancestors(i).skip(1) {
                if self.region.get(a).copied().unwrap_or(false) {
                    work.push(a);
                }
            }
        }
        for &b in &self.blinkers {
            if frame.nodes.get(b).is_some_and(|n| self.caret_changed(n)) {
                work.push(b);
            }
        }
        work.sort_unstable();
        work.dedup();
        let mut undo: Vec<Undo> = Vec::new();
        let mut props = std::mem::take(&mut self.scene.properties);
        let result = self.rewrite(frame, &work, &mut props, &mut undo, text, generation, size, scale);
        self.scene.properties = props;
        self.work = work;
        match result {
            Ok((chunks, compositing)) => {
                let mut changed = Vec::with_capacity(chunks.len());
                for (at, chunk) in chunks {
                    if let Some(slot) = self.scene.chunks.get_mut(at) {
                        *slot = chunk.clone();
                    }
                    changed.push(chunk);
                }
                let props = &mut self.scene.properties;
                props.transforms.resize(
                    self.transforms.len().max(props.transforms.len()),
                    TransformNode::default(),
                );
                props
                    .clips
                    .resize(self.clips.len().max(props.clips.len()), ClipNode::default());
                props
                    .effects
                    .resize(self.effects.len().max(props.effects.len()), EffectNode::default());
                let delta = written(props, &undo);
                let before = self.compositing.clone();
                if compositing {
                    let (scrollers, hits, moving) = self.tables(frame);
                    self.keep_compositing(scrollers, hits, moving);
                    self.scene.compositing = self.compositing.clone();
                }
                self.schedule_blink(!self.blinkers.is_empty());
                Ok(Commit::changed(&self.scene, &before, changed, delta))
            }
            Err(whole) => {
                // Put back what was written in place: the whole record
                // compares with the scene as committed.
                let props = &mut self.scene.properties;
                for u in undo.into_iter().rev() {
                    match u {
                        Undo::Transform(id, old) => restore(&mut props.transforms, id, old),
                        Undo::Clip(id, old) => restore(&mut props.clips, id, old),
                        Undo::Effect(id, old) => restore(&mut props.effects, id, old),
                    }
                }
                Err(whole)
            }
        }
    }

    /// Place and record the nodes of `work` (ascending) and the subtrees
    /// whose property state moved: the chunks to write into the scene (at
    /// their place in it), and whether the compositor's tables changed.
    #[expect(clippy::too_many_arguments, reason = "what a record reads")]
    fn rewrite(
        &mut self,
        frame: &Frame,
        work: &[usize],
        props: &mut Properties,
        undo: &mut Vec<Undo>,
        text: &mut TextSystem,
        generation: u64,
        size: (u32, u32),
        scale: f32,
    ) -> Result<(Vec<(usize, Chunk)>, bool), Whole> {
        let mut chunks: Vec<(usize, Chunk)> = Vec::new();
        let mut compositing = false;
        let mut thumbs: Vec<usize> = Vec::new();
        let mut next = work.iter().copied().peekable();
        // Nodes before this index are visited one after another (a subtree
        // whose property state moved).
        let mut through = 0_usize;
        let mut at = None::<usize>;
        loop {
            let i = match at {
                Some(prev) if prev.saturating_add(1) < through => prev.saturating_add(1),
                _ => match next.next() {
                    Some(i) => i,
                    None => break,
                },
            };
            // Skip what the run over a subtree already visited.
            while next.peek().is_some_and(|&w| w <= i) {
                next.next();
            }
            at = Some(i);
            let Some(node) = frame.nodes.get(i) else {
                return Err(Whole);
            };
            self.stats.visited = self.stats.visited.saturating_add(1);
            let up = node.parent.and_then(|p| self.placed.get(p).copied()).unwrap_or(Placed {
                snap: true,
                ..Placed::default()
            });
            let was = self.placed.get(i).copied().unwrap_or_default();
            let p = self.place(
                frame,
                node,
                i,
                up,
                &mut Writes {
                    props: &mut *props,
                    undo: Some(&mut *undo),
                },
                scale,
                size,
            );
            if p != was {
                // What its subtree paints under moved with it.
                through = through.max(self.ends.get(i).copied().unwrap_or(i));
                compositing = true;
            }
            if let Some(slot) = self.placed.get_mut(i) {
                *slot = p;
            }
            if is_blinker(node) != self.blinkers.contains(&i)
                || has_thumb(node) != self.thumb_of.get(i).is_some_and(Option::is_some)
                || has_overlay(node) != self.overlay_of.get(i).is_some_and(Option::is_some)
            {
                return Err(Whole);
            }
            compositing |= node.scroll.is_some() || node.moving.is_some() || self.hit_changed(frame, i, node);
            if has_thumb(node) {
                thumbs.push(i);
            }
            if let Some(c) = self.overlay_of.get(i).copied().flatten() {
                let chunk = Chunk {
                    id: ChunkId(key(&node.path, Role::Overlay)),
                    state: p.state(),
                    display: self.chunk(node, Role::Overlay, p.snap, scale, generation, text),
                };
                let kept = self.scene.chunks.get(c);
                if kept.is_none_or(|k| k.state != chunk.state || !Arc::ptr_eq(&k.display, &chunk.display)) {
                    chunks.push((c, chunk));
                }
            }
            let display = self.chunk(node, Role::Chunk, p.snap, scale, generation, text);
            match (self.chunk_of.get(i).copied().flatten(), display.items.is_empty()) {
                (Some(c), false) => {
                    let chunk = Chunk {
                        id: ChunkId(key(&node.path, Role::Chunk)),
                        state: p.state(),
                        display,
                    };
                    let kept = self.scene.chunks.get(c);
                    if kept.is_none_or(|k| k.state != chunk.state || !Arc::ptr_eq(&k.display, &chunk.display)) {
                        chunks.push((c, chunk));
                    }
                }
                (None, true) => {}
                _ => return Err(Whole),
            }
        }
        // The thumbs of the scroll containers visited, at their places.
        // Sized as the frame: a thumb's region is clipped to it.
        let mut scene = Scene {
            size,
            ..Scene::default()
        };
        self.thumbs(
            frame,
            &thumbs,
            &mut scene,
            &mut Writes {
                props,
                undo: Some(undo),
            },
            scale,
            false,
        );
        for (&i, chunk) in thumbs.iter().zip(scene.chunks) {
            let Some(c) = self.thumb_of.get(i).copied().flatten() else {
                return Err(Whole);
            };
            let kept = self.scene.chunks.get(c);
            if kept.is_none_or(|k| k.state != chunk.state || !Arc::ptr_eq(&k.display, &chunk.display)) {
                chunks.push((c, chunk));
            }
        }
        Ok((chunks, compositing))
    }

    /// Whether node `i`'s box the compositor hit-tests changed.
    fn hit_changed(&self, frame: &Frame, i: usize, node: &FrameNode) -> bool {
        let takes = frame.behaviour(i).is_some_and(|b| !b.pass_through) && !node.look.invisible;
        takes.then(|| hit_box(node)) != self.hit_rect.get(i).copied().flatten()
    }

    /// Whether `node`'s caret shows otherwise than its chunk recorded it.
    fn caret_changed(&self, node: &FrameNode) -> bool {
        let id = ChunkId(key(&node.path, Role::Chunk));
        self.chunks
            .get(&id)
            .is_some_and(|r| matches!(&r.input, Input::Node { caret, .. } if *caret != caret_on(node)))
    }

    /// The compositor's tables of the whole frame: scroll containers (with
    /// their thumbs), the boxes it hit-tests, the nodes it moves.
    fn tables(&mut self, frame: &Frame) -> (Vec<ScrollNode>, Vec<Hit>, Vec<MovingNode>) {
        let mut scrollers: Vec<ScrollNode> = Vec::new();
        let mut hits: Vec<Hit> = Vec::new();
        let mut moving: Vec<MovingNode> = Vec::new();
        // The scroll container each node is in, itself when it is one.
        let mut scroller_of: Vec<Option<u32>> = Vec::with_capacity(frame.nodes.len());
        self.hit_rect.clear();
        for (i, node) in frame.nodes.iter().enumerate() {
            let p = self.placed.get(i).copied().unwrap_or_default();
            let around = node.parent.and_then(|q| scroller_of.get(q).copied().flatten());
            let within = match node.scroll {
                Some(sc) => {
                    let index = u32::try_from(scrollers.len()).ok();
                    let key = NodeKey::of(&node.path);
                    scrollers.push(ScrollNode {
                        key,
                        content: p.content,
                        offset: sc.offset,
                        applied: sc.applied,
                        max: sc.max,
                        rtl: node.look.text.direction == crate::style::Direction::Rtl,
                        generation: sc.generation,
                        thumb: self.thumb_nodes.get(&key).copied(),
                        parent: around,
                    });
                    index
                }
                None => around,
            };
            scroller_of.push(within);
            let takes = frame.behaviour(i).is_some_and(|b| !b.pass_through) && !node.look.invisible;
            self.hit_rect.push(takes.then(|| hit_box(node)));
            if takes {
                hits.push(Hit {
                    rect: hit_box(node),
                    transform: p.transform,
                    clip: p.clip,
                    scroller: within,
                });
            }
            if let Some(m) = &node.moving {
                moving.push(MovingNode {
                    key: NodeKey::of(&node.path),
                    transform: Some(p.transform),
                    effect: Some(p.effect),
                    moving: m.clone(),
                    space: node.space,
                    invisible: node.look.invisible,
                    turns: m.curves.turns(),
                });
            }
        }
        (scrollers, hits, moving)
    }

    /// Keep the compositor's tables, each the same `Arc` while equal.
    fn keep_compositing(&mut self, scrollers: Vec<ScrollNode>, hits: Vec<Hit>, moving: Vec<MovingNode>) {
        let c = &mut self.compositing;
        if *c.scrollers != *scrollers {
            c.scrollers = std::sync::Arc::from(scrollers);
        }
        if *c.hits != *hits {
            c.hits = std::sync::Arc::from(hits);
        }
        if *c.moving != *moving {
            c.moving = std::sync::Arc::from(moving);
        }
    }

    /// What the last [`Painter::record`] did.
    #[must_use]
    pub fn stats(&self) -> PaintStats {
        self.stats
    }

    /// The chunks kept and the bytes they hold: each chunk's record, its
    /// items and its pieces' ids (the memory report's paint owner).
    #[must_use]
    pub fn held(&self) -> (usize, u64) {
        let size = |n: usize, each: usize| u64::try_from(n.saturating_mul(each)).unwrap_or(u64::MAX);
        let table = size(self.chunks.capacity(), std::mem::size_of::<(ChunkId, Retained)>());
        let bytes = self.chunks.values().fold(table, |n, r| {
            n.saturating_add(size(r.display.items.capacity(), std::mem::size_of::<Item>()))
                .saturating_add(size(r.pieces.capacity(), std::mem::size_of::<u64>()))
        });
        (self.chunks.len(), bytes)
    }

    /// Where the last [`Painter::record`] recorded chunks again, or moved,
    /// faded or clipped what it kept, in window coordinates (paint flashing).
    #[must_use]
    pub fn painted(&self) -> &[Repaint] {
        &self.painted
    }

    /// The property tree nodes `node` (at `i`) makes, under `up`, its parent's.
    #[expect(clippy::too_many_arguments, reason = "a node and the trees it adds to")]
    fn place(
        &mut self,
        frame: &Frame,
        node: &FrameNode,
        i: usize,
        up: Placed,
        w: &mut Writes<'_>,
        scale: f32,
        size: (u32, u32),
    ) -> Placed {
        let now = self.frame;
        let t = node.look.transform;
        // A scale or turn the compositor moves keeps its matrix throughout.
        let turns = node.moving.as_ref().is_some_and(|m| m.curves.turns());
        let matrix = (turns || !t.is_identity()).then_some(GroupTransform {
            origin: (
                (node.bounds.x + t.origin.0) * scale,
                (node.bounds.y + t.origin.1) * scale,
            ),
            scale: t.scale,
            rotate: t.rotate,
        });
        let transform = if node.translating || matrix.is_some() || node.moving.is_some() {
            let id = self.transforms.id(key(&node.path, Role::Transform), now);
            w.transform(
                id,
                TransformNode {
                    parent: up.content,
                    offset: node.offset,
                    matrix,
                },
            );
            id
        } else {
            up.content
        };
        let clip = match (node.clipper, node.clip) {
            (Some(c), Some(bounds)) => {
                let owner = frame.nodes.get(c);
                let id = owner.map_or(0, |o| self.clips.id(key(&o.path, Role::Clip), now));
                let space = self.placed.get(c).map_or(0, |o| o.transform);
                w.clip(
                    id,
                    ClipNode {
                        clip: Clip {
                            bounds,
                            radius: node.clip_radius,
                            curve: node.clip_curve,
                        },
                        transform: space,
                    },
                );
                id
            }
            _ => 0,
        };
        let effect = if effects::grouped(node) || node.moving.is_some() {
            let id = self.effects.id(key(&node.path, Role::Effect), now);
            let opacity = if node.look.invisible { 0.0 } else { node.look.opacity };
            let subtree = matrix.is_some() || node.look.opacity < 1.0;
            if let Some(r) = self.region.get_mut(i) {
                *r = subtree;
            }
            let region = if subtree {
                let end = self.ends.get(i).copied().unwrap_or(i);
                effects::subtree_region(frame.nodes.get(i..end).unwrap_or_default(), scale, size)
            } else {
                effects::group_region(node, scale, size)
            };
            w.effect(
                id,
                EffectNode {
                    parent: up.effect,
                    opacity,
                    filters: node.look.effects.filters.clone(),
                    mask: effects::mask_gradient(node),
                    transform,
                    transformed: matrix.is_some(),
                    clip: if matrix.is_some() { clip } else { 0 },
                    region,
                },
            );
            id
        } else {
            if let Some(r) = self.region.get_mut(i) {
                *r = false;
            }
            up.effect
        };
        let content = match node.scroll {
            Some(s) => {
                let id = self.transforms.id(key(&node.path, Role::Content), now);
                w.transform(
                    id,
                    TransformNode {
                        parent: transform,
                        offset: (node.offset.0 + s.applied.0, node.offset.1 + s.applied.1),
                        matrix: None,
                    },
                );
                id
            }
            None => transform,
        };
        Placed {
            transform,
            content,
            effect,
            clip,
            snap: up.snap && matrix.is_none(),
            // A transformed group applies the clip from outside it once
            // transformed: what inside it that clip alone clips paints unclipped.
            lifted: if matrix.is_some() { Some(clip) } else { up.lifted },
        }
    }

    /// `node`'s chunk: as kept when nothing it is recorded from changed,
    /// else recorded again.
    fn chunk(
        &mut self,
        node: &FrameNode,
        role: Role,
        snap: bool,
        scale: f32,
        generation: u64,
        text: &mut TextSystem,
    ) -> Arc<Display> {
        let now = self.frame;
        let overlay = matches!(role, Role::Overlay);
        let id = ChunkId(key(&node.path, role));
        let caret = caret_on(node);
        // What it may record: what its clip shows, and a margin.
        let visible = node.clip.map(|c| c.translated(-node.offset.0, -node.offset.1));
        let moved = Moved::of(node);
        if let Some(r) = self.chunks.get_mut(&id)
            && let Input::Node {
                node: was,
                snap: s,
                caret: c,
                scale: k,
                text: g,
                cull,
            } = &r.input
        {
            let same = was.space == node.space
                && was.space_content == node.space_content
                && *s == snap
                && *c == caret
                && *k == scale
                && *g == generation
                && was.content == node.content
                && same_look(&was.look, &node.look);
            let within = match (cull, visible) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some(r), Some(v)) => r.holds(&v),
            };
            if same && within {
                r.seen = now;
                if r.properties != moved {
                    self.stats.moved = self.stats.moved.saturating_add(1);
                    self.painted.push(Repaint {
                        bounds: node.bounds,
                        recorded: false,
                    });
                    r.properties = moved;
                }
                return Arc::clone(&r.display);
            }
            if was.space.width == node.space.width && was.space.height == node.space.height {
                self.stats.repainted = self.stats.repainted.saturating_add(1);
            } else {
                self.stats.relaid = self.stats.relaid.saturating_add(1);
            }
        }
        self.stats.recorded = self.stats.recorded.saturating_add(1);
        self.painted.push(Repaint {
            bounds: node.bounds,
            recorded: true,
        });
        let cull = visible.map(|v| v.outset(CULL_MARGIN));
        let view = in_space(node);
        let mut items = Vec::new();
        drop(text.take_drawn());
        let mut target = Target {
            items: &mut items,
            scale,
            size: (u32::MAX / 4, u32::MAX / 4),
            cull,
            snap,
        };
        if overlay {
            if !view.look.invisible {
                record_overlay(&mut target, &view);
            }
        } else {
            record_node(&view, &mut target, text, cull, caret);
        }
        let pieces = text.take_drawn();
        text.pin(&pieces);
        let display = Arc::new(Display::new(items));
        let input = Input::Node {
            node: Box::new(view),
            snap,
            caret,
            scale,
            text: generation,
            cull,
        };
        if let Some(old) = self.chunks.insert(
            id,
            Retained {
                input,
                display: Arc::clone(&display),
                pieces,
                properties: moved,
                seen: now,
            },
        ) {
            text.unpin(&old.pieces);
        }
        display
    }

    /// The overlay chunks of the nodes in `over` whose subtree ends before
    /// node `before` (all of them at `None`), innermost first: each paints
    /// over its whole subtree, under the node's own state.
    #[expect(clippy::too_many_arguments, reason = "what a record reads")]
    fn overlays(
        &mut self,
        frame: &Frame,
        over: &mut Vec<(usize, Placed)>,
        before: Option<usize>,
        scene: &mut Scene,
        scale: f32,
        generation: u64,
        text: &mut TextSystem,
    ) {
        while let Some(&(j, p)) = over.last() {
            if before.is_some_and(|i| self.ends.get(j).copied().unwrap_or(0) > i) {
                break;
            }
            over.pop();
            let Some(node) = frame.nodes.get(j) else { continue };
            let display = self.chunk(node, Role::Overlay, p.snap, scale, generation, text);
            if let Some(c) = self.overlay_of.get_mut(j) {
                *c = Some(scene.chunks.len());
            }
            scene.chunks.push(Chunk {
                id: ChunkId(key(&node.path, Role::Overlay)),
                state: p.state(),
                display,
            });
        }
    }

    /// The thumbs of `scrollers`, each a chunk under a transform its scroll
    /// offset moves: scrolling records nothing again.
    fn thumbs(
        &mut self,
        frame: &Frame,
        scrollers: &[usize],
        scene: &mut Scene,
        w: &mut Writes<'_>,
        scale: f32,
        whole: bool,
    ) {
        let now = self.frame;
        for &i in scrollers {
            let (Some(node), Some(p)) = (frame.nodes.get(i), self.placed.get(i).copied()) else {
                continue;
            };
            let Some(scroll) = node.scroll else { continue };
            let b = node.space.snapped(scale);
            let content_h = b.height + scroll.max.1;
            let thumb_h = (b.height * b.height / content_h).max(20.0);
            let travel = (b.height - thumb_h - 4.0).max(0.0);
            let along = travel * (scroll.offset.1 / scroll.max.1);
            let transform = self.transforms.id(key(&node.path, Role::ThumbTransform), now);
            w.transform(
                transform,
                TransformNode {
                    // Under the container's own transform: what moves it moves its thumb.
                    parent: p.transform,
                    offset: (node.offset.0, node.offset.1 + along),
                    matrix: None,
                },
            );
            // An overlay scrollbar (Chrome's on macOS, cc's
            // `ScrollbarAnimationController`): hidden at rest, shown by the
            // compositor while the user scrolls it, then faded out. Its
            // opacity is this effect's, which only the compositor moves.
            let effect = self.effects.id(key(&node.path, Role::ThumbEffect), now);
            w.effect(
                effect,
                EffectNode {
                    parent: p.effect,
                    opacity: 0.0,
                    // The container's: its region is the track, which the
                    // thumb moves along, not with.
                    transform: p.transform,
                    // The track: wherever along it the thumb is.
                    region: effects::physical_rect(
                        Bounds {
                            x: node.bounds.right() - 8.0,
                            width: 8.0,
                            ..node.bounds
                        },
                        scale,
                        scene.size,
                    ),
                    ..EffectNode::default()
                },
            );
            self.thumb_nodes.insert(
                NodeKey::of(&node.path),
                ThumbNode {
                    transform,
                    effect,
                    top: node.offset,
                    travel,
                },
            );
            let id = ChunkId(key(&node.path, Role::Thumb));
            let same = self.chunks.get(&id).is_some_and(|r| {
                matches!(r.input, Input::Thumb { space, max, thumb, scale: k }
                    if space == node.space && max == scroll.max && thumb == scroll.thumb && k == scale)
            });
            let display = if let Some(r) = self.chunks.get_mut(&id).filter(|_| same) {
                r.seen = now;
                Arc::clone(&r.display)
            } else {
                self.stats.recorded = self.stats.recorded.saturating_add(1);
                self.painted.push(Repaint {
                    bounds: Bounds {
                        x: node.bounds.right() - 8.0,
                        width: 8.0,
                        ..node.bounds
                    },
                    recorded: true,
                });
                let thumb = Bounds {
                    x: b.right() - 8.0,
                    y: b.y + 2.0,
                    width: 6.0,
                    height: thumb_h,
                };
                let display = Arc::new(Display::new(vec![Item::RoundedRect {
                    bounds: thumb,
                    radius: Corners::all(3.0),
                    color: scroll.thumb,
                }]));
                self.chunks.insert(
                    id,
                    Retained {
                        input: Input::Thumb {
                            space: node.space,
                            max: scroll.max,
                            thumb: scroll.thumb,
                            scale,
                        },
                        display: Arc::clone(&display),
                        pieces: Vec::new(),
                        properties: Moved::of(node),
                        seen: now,
                    },
                );
                display
            };
            if whole && let Some(t) = self.thumb_of.get_mut(i) {
                *t = Some(scene.chunks.len());
            }
            scene.chunks.push(Chunk {
                id,
                state: State {
                    transform,
                    clip: p.clip,
                    effect,
                },
                display,
            });
        }
    }

    /// The scene `frame` records into from nothing (the debug check that
    /// incremental paint equals cold): nothing it records stays pinned or
    /// scheduled.
    pub(crate) fn cold(frame: &Frame, text: &mut TextSystem, size: (u32, u32), scale: f32, background: Color) -> Scene {
        let mut p = Self::default();
        let _ = p.record(frame, text, size, scale, background, true);
        p.schedule_blink(false);
        for r in p.chunks.values() {
            text.unpin(&r.pieces);
        }
        std::mem::take(&mut p.scene)
    }

    fn schedule_blink(&mut self, blinking: bool) {
        if let Some(t) = self.blink.take() {
            clock::cancel(t);
        }
        if blinking {
            let t = clock::frame_time();
            let next = (t / BLINK).floor().mul_add(BLINK, BLINK) - t;
            self.blink = Some(clock::after(Duration::from_secs_f32(next.max(0.01)), || {}));
        }
    }
}

/// Record one node's box and content into `target` (its paint space); `cull`
/// is what may be recorded, `caret` whether a text input's caret shows.
fn record_node(node: &FrameNode, target: &mut Target<'_>, text: &mut TextSystem, cull: Option<Bounds>, caret: bool) {
    if node.look.invisible {
        return;
    }
    if let Some(c) = cull
        && c.intersect(&node.bounds.outset(16.0)).is_empty()
    {
        return;
    }
    record_box(target, node, text);
    record_content(target, node, text, caret);
}

fn record_box(target: &mut Target<'_>, node: &FrameNode, text: &mut TextSystem) {
    let Look {
        background,
        border,
        border_color,
        shape,
        shadows,
        ring,
        decoration,
        ..
    } = &node.look;
    let b = target.snapped(node.bounds);
    // The radii the box gets (CSS's overlapping-curves rule, against the
    // box as painted, as Blink constrains them on the snapped border box).
    let radius = shape.used(b.width, b.height);
    let k = shape.curve();
    // What lies behind, filtered, under all the node paints: its shadows
    // fall outside the box and the backdrop only inside it.
    if let Some(backdrop) = decoration.backdrop.filter(|d| !d.is_identity()) {
        target.items.push(Item::Backdrop {
            bounds: b,
            radius,
            curve: k,
            backdrop,
        });
    }
    for s in shadows.iter().filter(|s| !s.color.is_transparent()) {
        target.items.push(Item::Shadow {
            bounds: b,
            radius,
            curve: k,
            shadow: *s,
        });
    }
    if let Some(ring) = ring.filter(|r| r.width > 0.0 && !r.color.is_transparent()) {
        // A band `offset` out from the border box, its corners grown from
        // the shape's as Chrome grows a spread's or an outline's.
        let (near, far) = (ring.offset, ring.offset + ring.width);
        band(
            target,
            (b.outset(far), ring.radii(radius, far)),
            (b.outset(near), ring.radii(radius, near)),
            k,
            ring.color,
        );
    }
    // A background clipped to the text is painted with the text.
    if let Some(bg) = background.filter(|c| !c.is_transparent())
        && effects::text_background(node).is_none()
    {
        fill(target, b, radius, k, bg);
    }
    if let Some(img) = effects::box_background(node) {
        effects::record_background(target, node, img, text);
    }
    if let Some(color) = border_color.filter(|c| !c.is_transparent()) {
        border_band(target, b, radius, *border, k, color);
    }
    if let Some(layer) = &decoration.inset {
        // Placed against the padding box, as an absolute pseudo-element's
        // `inset-*` is.
        let padding = border_inner(b, radius, *border).map_or(b, |(p, _)| p);
        record_inset(target, padding, layer);
    }
    if let Some(bar) = decoration
        .edge_bar
        .filter(|e| e.width > 0.0 && !e.color.is_transparent())
    {
        use crate::element::Side;
        let w = bar.width.min(b.width).min(b.height);
        let bounds = match bar.side {
            Side::Top => Bounds { height: w, ..b },
            Side::Bottom => Bounds {
                y: b.bottom() - w,
                height: w,
                ..b
            },
            Side::Left => Bounds { width: w, ..b },
            Side::Right => Bounds {
                x: b.right() - w,
                width: w,
                ..b
            },
        };
        target.items.push(Item::Rect {
            bounds,
            color: bar.color,
        });
    }
}

/// `b` filled in `color`, its corners of used radii `radius` curving as `k`.
fn fill(target: &mut Target<'_>, b: Bounds, radius: Corners, k: f32, color: Color) {
    if (k - crate::shape::ROUND).abs() > f32::EPSILON && !radius.is_zero() {
        target.items.push(Item::Superellipse {
            outer: b,
            outer_radius: radius,
            inner: Bounds::default(),
            inner_radius: Corners::default(),
            k,
            color,
        });
    } else {
        target.items.push(Item::RoundedRect {
            bounds: b,
            radius,
            color,
        });
    }
}

/// What lies between `outer` and `inner` (each a box and its radii), in `color`.
fn band(target: &mut Target<'_>, outer: (Bounds, Corners), inner: (Bounds, Corners), k: f32, color: Color) {
    if (k - crate::shape::ROUND).abs() > f32::EPSILON && !outer.1.is_zero() {
        target.items.push(Item::Superellipse {
            outer: outer.0,
            outer_radius: outer.1,
            inner: inner.0,
            inner_radius: inner.1,
            k,
            color,
        });
    } else {
        target.items.push(Item::Band {
            outer: outer.0,
            outer_radius: outer.1,
            inner: inner.0,
            inner_radius: inner.1,
            color,
        });
    }
}

/// A border of `widths` inside `b` (used radii `radius`): its inner edge's
/// corners are the outer ones less the wider of the two sides meeting there.
fn border_band(target: &mut Target<'_>, b: Bounds, radius: Corners, widths: Edges<f32>, k: f32, color: Color) {
    if let Some(inner) = border_inner(b, radius, widths) {
        band(target, (b, radius), inner, k, color);
    }
}

/// The inner edge of a border of `widths` inside `b` (used radii
/// `radius`), `None` when no side has width.
fn border_inner(b: Bounds, radius: Corners, widths: Edges<f32>) -> Option<(Bounds, Corners)> {
    let w = widths;
    if !(w.top > 0.0 || w.right > 0.0 || w.bottom > 0.0 || w.left > 0.0) {
        return None;
    }
    let inner = b.inset(w.top, w.right, w.bottom, w.left);
    let inner_radius = Corners {
        top_left: (radius.top_left - w.top.max(w.left)).max(0.0),
        top_right: (radius.top_right - w.top.max(w.right)).max(0.0),
        bottom_right: (radius.bottom_right - w.bottom.max(w.right)).max(0.0),
        bottom_left: (radius.bottom_left - w.bottom.max(w.left)).max(0.0),
    };
    Some((inner, inner_radius))
}

/// An inset layer of a node whose border box is `b`: its shadows, fill and
/// border, its shape resolved against its own box.
fn record_inset(target: &mut Target<'_>, b: Bounds, layer: &crate::shape::InsetLayer) {
    let e = layer.inset;
    let at = b.inset(e.top, e.right, e.bottom, e.left);
    if at.is_empty() {
        return;
    }
    let at = target.snapped(at);
    let radius = layer.shape.used(at.width, at.height);
    let k = layer.shape.curve();
    for s in layer.shadows.iter().filter(|s| !s.color.is_transparent()) {
        target.items.push(Item::Shadow {
            bounds: at,
            radius,
            curve: k,
            shadow: *s,
        });
    }
    if let Some(bg) = layer.background.filter(|c| !c.is_transparent()) {
        fill(target, at, radius, k, bg);
    }
    if let Some(color) = layer.border_color.filter(|c| !c.is_transparent()) {
        border_band(target, at, radius, Edges::all(layer.border), k, color);
    }
}

/// The border a node paints over its content and children
/// ([`crate::shape::OverlayBorder`]), along its shape, blended with what
/// lies below as it asks.
pub(crate) fn record_overlay(target: &mut Target<'_>, node: &FrameNode) {
    let Some(o) = node
        .look
        .decoration
        .overlay
        .filter(|o| o.width > 0.0 && !o.color.is_transparent())
    else {
        return;
    };
    let b = target.snapped(node.bounds);
    let radius = node.look.shape.used(b.width, b.height);
    let k = node.look.shape.curve();
    // Along the padding box's edge and its inner corners, as an absolute
    // `after:inset-0` with the node's corners is.
    let (b, radius) = border_inner(b, radius, node.look.border).unwrap_or((b, radius));
    if o.blend == crate::shape::Blend::Normal {
        border_band(target, b, radius, Edges::all(o.width), k, o.color);
        return;
    }
    if let Some((inner, inner_radius)) = border_inner(b, radius, Edges::all(o.width)) {
        target.items.push(Item::Blended {
            outer: b,
            outer_radius: radius,
            inner,
            inner_radius,
            curve: k,
            color: o.color,
            blend: o.blend,
        });
    }
}

/// Record what the node holds.
fn record_content(target: &mut Target<'_>, node: &FrameNode, text: &mut TextSystem, caret_shown: bool) {
    match &node.content {
        Content::None => {}
        Content::Figures(shapes) => {
            let b = target.snapped(node.bounds);
            for shape in shapes {
                target.items.push(Item::Path {
                    origin: (b.x, b.y),
                    path: shape.path.clone(),
                    style: shape.style.clone(),
                });
            }
        }
        Content::Drawn(d) => {
            let key = foldhash::fast::FixedState::with_seed(0x51c7_d2a9).hash_one(&node.path);
            d.shape(text);
            d.draw(node.content_box, target.scale, key, target.items);
        }
        Content::Image { source, radius } => {
            target.items.push(Item::Image {
                source: source.clone(),
                bounds: target.snapped(node.bounds),
                radius: *radius,
                curve: node.look.shape.curve(),
            });
        }
        Content::Text {
            content,
            text: t,
            runs,
            boxes,
        } => {
            let area = &node.content_box;
            let p = Paragraph { content, runs, boxes };
            if let Some(img) = effects::text_background(node) {
                effects::record_text_through(target, node, img, text, p, t, text_width(t, area));
            } else {
                text.draw(target, p, t, text_width(t, area), (area.x, area.y), 1.0);
            }
        }
        Content::Icon {
            icon,
            color,
            stroke_width,
        } => {
            let scale = target.scale;
            let side = node.bounds.width.min(node.bounds.height);
            target.items.push(Item::Icon {
                icon: *icon,
                size: physical(side, scale),
                color: *color,
                stroke_width: *stroke_width,
                x: physical_pos(node.bounds.x + (node.bounds.width - side) / 2.0, scale),
                y: physical_pos(node.bounds.y + (node.bounds.height - side) / 2.0, scale),
            });
        }
        Content::Input {
            edit,
            preedit,
            placeholder,
            placeholder_color,
            selection_color,
            text: t,
            focused,
            multiline,
            ..
        } => {
            let area = &node.content_box;
            let it = input_text(text, &edit.text, t, *area, *multiline, edit.caret);
            let (line_t, width, origin) = (it.t, it.width, it.origin);
            let caret = text.caret(&edit.text, &line_t, width, edit.caret);
            if *focused && edit.has_selection() {
                let (from, to) = edit.selection();
                for r in text.selection(&edit.text, &line_t, width, from, to) {
                    let b = Bounds {
                        x: origin.0 + r.x,
                        y: origin.1 + r.y,
                        ..r
                    }
                    .intersect(&area.outset(1.0));
                    target.fill_rect(b, *selection_color);
                }
            }
            if edit.text.is_empty() {
                text.draw(
                    target,
                    placeholder.as_str(),
                    &ResolvedText {
                        color: *placeholder_color,
                        ..line_t
                    },
                    width,
                    origin,
                    1.0,
                );
            } else {
                text.draw(target, edit.text.as_str(), &line_t, width, origin, 1.0);
            }
            // What an input method is composing is underlined, as a
            // browser marks a composition, until it is committed.
            if let Some((from, to)) = *preedit {
                for r in text.selection(&edit.text, &line_t, width, from, to) {
                    let b = Bounds {
                        x: origin.0 + r.x,
                        y: origin.1 + r.y + r.height - 1.0,
                        width: r.width,
                        height: 1.0,
                    }
                    .intersect(&area.outset(1.0));
                    target.fill_rect(b, t.color);
                }
            }
            if caret_shown {
                let b = Bounds {
                    x: (origin.0 + caret.x).min(area.right() - 1.0),
                    y: origin.1 + caret.y,
                    width: 1.0,
                    height: caret.height,
                };
                target.fill_rect(b, t.color);
            }
        }
    }
}

/// What differs between two scenes recorded from one frame, with property
/// ids read as what they name (ids are kept across frames, so two painters
/// number them apart); `None` when they paint the same. Only what the window
/// shows is compared: a kept chunk recorded within its old cull rect holds
/// less (or more) beyond the window than one recorded now, and records
/// again before that part can show.
pub(crate) fn scene_differs(frame: &Frame, a: &Scene, b: &Scene) -> Option<String> {
    fn transforms(p: &Properties, mut id: u32) -> Vec<((f32, f32), Option<GroupTransform>)> {
        let mut out = Vec::new();
        while id != 0 && out.len() < 4096 {
            let t = p.transform(id);
            out.push((t.offset, t.matrix));
            id = t.parent;
        }
        out
    }
    type Clipped = Option<(Clip, Vec<((f32, f32), Option<GroupTransform>)>)>;
    fn clip(p: &Properties, id: u32) -> Clipped {
        p.clip(id).map(|c| (c.clip, transforms(p, c.transform)))
    }
    fn effects(p: &Properties, id: u32) -> String {
        p.effect_chain(id, 0)
            .map(|(_, e)| {
                format!(
                    "{} {:?} {:?} {} {:?} {:?} {:?}",
                    e.opacity,
                    e.filters,
                    e.mask,
                    e.transformed,
                    e.region,
                    transforms(p, e.transform),
                    clip(p, e.clip)
                )
            })
            .collect::<Vec<_>>()
            .join(" / ")
    }
    /// A chunk's items the window shows, under its states read as values.
    fn shown<'s>(s: &'s Scene, c: &'s Chunk) -> Option<(Vec<&'s Item>, String)> {
        let p = &s.properties;
        let chain = transforms(p, c.state.transform);
        let turned = chain.iter().any(|(_, m)| m.is_some());
        let o = p.transform(c.state.transform).offset;
        let (mut x_min, mut y_min) = (0.0_f32, 0.0_f32);
        let (mut w, mut h) = (
            crate::convert::u32_to_f32(s.size.0),
            crate::convert::u32_to_f32(s.size.1),
        );
        // Within its clip too, in window pixels.
        if let Some(k) = p.clip(c.state.clip)
            && !transforms(p, k.transform).iter().any(|(_, m)| m.is_some())
        {
            let t = p.transform(k.transform).offset;
            let b = k.clip.bounds.translated(t.0, t.1);
            x_min = x_min.max(b.x * s.scale);
            y_min = y_min.max(b.y * s.scale);
            w = w.min(b.right() * s.scale);
            h = h.min(b.bottom() * s.scale);
        }
        let items: Vec<&Item> = c
            .display
            .items
            .iter()
            .filter(|item| {
                turned
                    || item.reach(s.scale).is_none_or(|r| {
                        let (x0, y0) = (o.0.mul_add(s.scale, r[0]), o.1.mul_add(s.scale, r[1]));
                        let (x1, y1) = (o.0.mul_add(s.scale, r[2]), o.1.mul_add(s.scale, r[3]));
                        x1 > x_min && y1 > y_min && x0 < w && y0 < h
                    })
            })
            .collect();
        (!items.is_empty()).then(|| {
            let state = format!("{chain:?} {:?} {}", clip(p, c.state.clip), effects(p, c.state.effect));
            (items, state)
        })
    }
    let name = |id: ChunkId| {
        frame
            .nodes
            .iter()
            .find(|n| {
                [Role::Chunk, Role::Overlay, Role::Thumb]
                    .into_iter()
                    .any(|r| key(&n.path, r) == id.0)
            })
            .map_or_else(|| format!("{id:?}"), |n| n.path.to_string())
    };
    if a.background != b.background || a.size != b.size || a.scale != b.scale {
        return Some("the target".to_owned());
    }
    fn list(s: &Scene) -> Vec<(ChunkId, (Vec<&Item>, String))> {
        s.chunks.iter().filter_map(|c| shown(s, c).map(|x| (c.id, x))).collect()
    }
    let (x, y) = (list(a), list(b));
    if x.len() != y.len() {
        let ids =
            |l: &[(ChunkId, (Vec<&Item>, String))]| l.iter().map(|c| c.0).collect::<std::collections::BTreeSet<_>>();
        let (i, j) = (ids(&x), ids(&y));
        return Some(format!(
            "{} chunks shown against {}: only here {:?}, only there {:?}",
            x.len(),
            y.len(),
            i.difference(&j).take(8).map(|&c| name(c)).collect::<Vec<_>>(),
            j.difference(&i)
                .take(8)
                .map(|&c| format!(
                    "{} (here {:?}, there {:?})",
                    name(c),
                    a.chunks
                        .iter()
                        .find(|k| k.id == c)
                        .map(|k| (k.display.items.len(), a.properties.transform(k.state.transform).offset)),
                    b.chunks
                        .iter()
                        .find(|k| k.id == c)
                        .map(|k| (k.display.items.len(), b.properties.transform(k.state.transform).offset))
                ))
                .collect::<Vec<_>>()
        ));
    }
    for ((ia, (items_a, state_a)), (ib, (items_b, state_b))) in x.iter().zip(&y) {
        if ia != ib {
            return Some(format!("chunk {} where {} is", name(*ia), name(*ib)));
        }
        if items_a != items_b {
            return Some(format!(
                "chunk {}'s items:\n{items_a:?}\nagainst\n{items_b:?}",
                name(*ia)
            ));
        }
        if state_a != state_b {
            return Some(format!("chunk {}'s state: {state_a} against {state_b}", name(*ia)));
        }
    }
    None
}
