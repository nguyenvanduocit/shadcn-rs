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
use crate::compositor::Commit;
use crate::compositor::{Compositing, Hit, MovingNode, NodeKey, ScrollNode, ShownText, ThumbNode};
use crate::effects;
use crate::geometry::{Bounds, NodeId};
use crate::inspect::{PaintStats, Repaint};
use crate::layout::{Changed, Content, Frame, FrameNode, Look, ReadAt, Since, input_text, text_width};
use crate::scene::{
    Chunk, ChunkId, Clip, ClipNode, Display, EffectNode, GroupTransform, Item, Properties, Scene, State, TransformNode,
};
use crate::style::{Corners, Edges, ResolvedText};
use crate::text::{Paragraph, Target, TextSystem, physical, physical_pos};

/// Paint's reader of laid-out geometry ([`crate::layout::Sight`]).
#[derive(Clone, Copy)]
pub(crate) struct Sight(());

/// Paint's [`Sight`].
const SIGHT: Sight = Sight(());

/// The caret shows for this long, then hides for as long.
const BLINK: f32 = 0.53;

/// How long a caret blinks, in seconds, after it was last focused, moved
/// or typed at: then it stays shown, and nothing wakes to draw it (GTK's
/// `gtk-cursor-blink-timeout`, 10 s, restarted by each interaction). A
/// caret that blinks for as long as it has focus (Blink's `FrameCaret`,
/// VS Code, Flutter) keeps an idle window drawing twice a second forever.
/// A component's own caret blinks by it too ([`crate::Animation::caret_blink`]).
pub(crate) const CARET_BLINK_TIMEOUT: f32 = 10.0;

/// When the focused inputs' carets began to blink, and whether the window
/// has the keyboard focus: a caret blinks only in the active window, and
/// shows in none other (Blink's `FrameSelection::FocusedOrActiveStateChanged`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CaretClock {
    /// When they began, nanoseconds on the frame clock: the blink's phase
    /// is the span from it ([`clock::span`]), exact at any uptime.
    since: u64,
    active: bool,
}

impl CaretClock {
    /// Whether a focused caret shows at `t`: from `since`, shown for
    /// [`BLINK`], hidden for as long, and so on, until it stays shown.
    fn shows(self, t: impl clock::Moment) -> bool {
        let e = clock::span(self.since, t.nanos());
        self.active && (e < 0.0 || e >= CARET_BLINK_TIMEOUT || (e / BLINK).floor().rem_euclid(2.0) < 1.0)
    }

    /// How long after `t` a focused caret next shows or hides; none while
    /// it stays as it is (blinked out, or in an inactive window).
    fn next_change(self, t: impl clock::Moment) -> Option<f32> {
        let e = clock::span(self.since, t.nanos()).max(0.0);
        if !self.active || e >= CARET_BLINK_TIMEOUT {
            return None;
        }
        let next = (e / BLINK).floor().mul_add(BLINK, BLINK).min(CARET_BLINK_TIMEOUT);
        Some(next - e)
    }
}

/// The carets of the focused inputs: when they began to blink, what each
/// input showed then (its text's revision, caret and selection), by its
/// overlay chunk's id, and the window's activation then. A change of any
/// restarts the blink, shown.
#[derive(Clone, Debug, Default)]
struct Caret {
    clock: CaretClock,
    marks: Vec<(u64, crate::edit::ShownMark)>,
    activation: u64,
}

/// How far past its clip a chunk records (Chromium's
/// `kPixelDistanceToRecord`): a scroll within it records nothing again.
const CULL_MARGIN: f32 = 4000.0;

/// What recording keeps across frames: each node's chunk, the ids of the
/// property tree nodes, and the caret's blink timer.
#[derive(Default)]
pub struct Painter {
    blink: Option<clock::TimerId>,
    caret: Caret,
    /// The window has not the keyboard focus ([`Painter::set_window_active`]),
    /// and how many times it gained it.
    window_inactive: bool,
    activations: u64,
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
    /// Each scroll container of the last tables, by its identity (its key
    /// and the component instance whose output it is), as the instance it
    /// is ([`ScrollNode::instance`]): one that leaves them and comes back
    /// under the same key, or whose component another replaced at its key,
    /// is a new one.
    scroll_instances: foldhash::HashMap<(NodeKey, Option<crate::reactive::OwnerId>), u64>,
    /// The last map of them emptied, kept for the next layout's
    /// ([`Self::mount_scrollers`]).
    scroll_spare: foldhash::HashMap<(NodeKey, Option<crate::reactive::OwnerId>), u64>,
    /// The instance the next new scroll container is.
    next_scroll: u64,
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
    hit_rect: Vec<Option<HitBox>>,
    /// Per frame node, the cursor it shows as the last tables had it
    /// ([`crate::compositor::Hit::cursor`]): its own, or its nearest
    /// ancestor's with one.
    cursor_of: Vec<Option<crate::style::Cursor>>,
    /// Per frame node, the text its box shows input
    /// ([`crate::compositor::ShownText`]), and the last tables' while new
    /// ones are made: one kept while it shows the same.
    shown_text: Vec<Option<Arc<ShownText>>>,
    shown_text_was: Vec<Option<Arc<ShownText>>>,
    /// Scratch: the tables' texts as made, by the place of their box in
    /// the hit table ([`Compositing::texts`]).
    texts: Vec<(u32, Arc<ShownText>)>,
    /// Focused inputs, whose caret blinks.
    blinkers: Vec<usize>,
    /// Scratch: the nodes a record visits.
    work: Vec<usize>,
    /// Displays chunks no longer show, until the renderer lets go of them,
    /// and those it let go of, empty, to record into ([`Painter::blank`]).
    retired: Vec<Arc<Display>>,
    ready: Pool,
    /// Scratch kept across records, so a repaint allocates nothing: what
    /// in-place writes replaced, the chunks recorded again and their
    /// places, the scroll containers visited and their thumbs, the ids
    /// written, the displays of chunks that left.
    undo: Vec<Undo>,
    rewritten: Vec<(usize, Chunk)>,
    thumb_work: Vec<usize>,
    thumb_chunks: Vec<Chunk>,
    ids: (Vec<u32>, Vec<u32>, Vec<u32>),
    gone: Vec<Arc<Display>>,
    /// Scratch: the displays the renderer let go of, being taken back.
    freed: Vec<Arc<Display>>,
    /// Scratch: the items of a chunk whose size is not known before it is
    /// recorded (new, or not `steady`), the most it held over the records
    /// since [`AGE`] was last counted, and for how many records its room
    /// was large and past twice that.
    recording: Vec<Item>,
    recording_peak: usize,
    recording_wide: u32,
}

/// How many empty displays are kept to record into at least: what a burst
/// of repaints frees is reused by the next, and more is let go. Past it
/// the pool follows the working set ([`Pool::caps`]).
const READY: usize = 512;

/// How many bytes of items the displays kept to record into may hold
/// between them at least, with those still waiting for the renderer to let
/// go of them: more is let go. Large displays ([`LARGE_BYTES`]) are kept
/// apart, outside it. Past it the pool follows the working set.
const READY_BYTES: usize = 2 << 20;

/// A display whose room is past this many bytes is large: it is kept to
/// record into in a slot of its own ([`LARGE`]), outside [`READY_BYTES`],
/// so the one chunk that needs it (a long text) takes it back each repaint
/// and small chunks keep their pool; taken back, it is cut to what it last
/// held (and [`SLACK`] past twice that keeps its room).
const LARGE_BYTES: usize = 256 << 10;

/// How many large displays are kept to record into: the chunk being
/// recorded again takes one back while the renderer still draws the one
/// it replaced.
const LARGE: usize = 2;

/// The room past twice what a display last held that it may keep.
const SLACK: usize = 16;

/// How many records a display may wait unused to be recorded into before
/// it is let go: what a repaint frees, the next takes, and what no record
/// takes for this long was room a burst grew. The scratch a chunk records
/// into keeps a large room no longer used for as long.
const AGE: u32 = 8;

/// The empty displays kept to record into, by size class: class 0 holds
/// those with no room, class `k` those with room for `2^(k-1)` to `2^k - 1`
/// items. A chunk recorded takes one of the class whose least room holds
/// what it records, and a new display gets the class's least room, so two
/// chunks of different sizes recorded each frame (a hover leaving one row
/// for another) each take back a display of its own class, none grows
/// nor is cut again, and a small chunk never takes the room a big one
/// needs. Taking and keeping are O(1); each class is oldest first, so
/// aging drops from its front.
#[derive(Default)]
struct Pool {
    classes: Vec<std::collections::VecDeque<Pooled>>,
    /// How many displays the classes hold, and the bytes of their room.
    count: usize,
    bytes: usize,
    /// The large displays kept ([`LARGE`]), and the bytes of their room.
    large: Vec<Pooled>,
    large_bytes: usize,
    /// The records passed, to age by.
    now: u64,
    /// The small displays this record took (or made, none kept), and the
    /// bytes of their room: the working set.
    taken: (usize, usize),
    /// The most taken by a record in this window of [`AGE`] records and
    /// the one before: what the pool keeps room for ([`Pool::caps`]).
    peak: [(usize, usize); 2],
}

/// A display kept to record into, and the record it was kept at.
struct Pooled {
    display: Arc<Display>,
    at: u64,
}

impl Pooled {
    fn room(&self) -> usize {
        self.display.items.capacity()
    }

    fn stale(&self, now: u64) -> bool {
        now.saturating_sub(self.at) > u64::from(AGE)
    }
}

fn item_bytes(n: usize) -> usize {
    n.saturating_mul(std::mem::size_of::<Item>())
}

/// How many nodes a whole record paints between a check's cancellation
/// points: few enough that a slice stays far under the input budget
/// (ADR-0028's +2 ms), many enough that live paint pays one test per run
/// of them.
const CANCEL_EVERY: usize = 64;

/// Whether a display with room for `room` items is large ([`LARGE_BYTES`]).
fn large(room: usize) -> bool {
    item_bytes(room) > LARGE_BYTES
}

/// The size class of a display with room for `room` items.
fn class_of(room: usize) -> usize {
    usize::try_from(usize::BITS.saturating_sub(room.leading_zeros())).unwrap_or(usize::MAX)
}

/// The class whose every display has room for `want` items.
fn class_for(want: usize) -> usize {
    match want.checked_sub(1) {
        None => 0,
        Some(below) => class_of(below).saturating_add(1),
    }
}

/// The room a display taken for `want` items has at least, and a new one
/// is given: its class's least, or as many when large.
fn room_for(want: usize) -> usize {
    if want == 0 {
        0
    } else {
        match want.checked_next_power_of_two() {
            Some(room) if !large(room) => room,
            _ => want,
        }
    }
}

/// Whether the display taken for `held` items holds `recorded`, and the
/// one taken for `recorded` holds `held`: both want one of the same class,
/// or, large, no more than the one held.
fn like(held: usize, recorded: usize) -> bool {
    class_for(held) == class_for(recorded) && (!large(room_for(held)) || recorded <= held)
}

impl Pool {
    /// A display with room for `want` items, the latest kept of its class;
    /// a large one, the smallest that holds them.
    fn take(&mut self, want: usize) -> Option<Arc<Display>> {
        if large(room_for(want)) {
            let at = self
                .large
                .iter()
                .enumerate()
                .filter(|(_, p)| p.room() >= want)
                .min_by_key(|(_, p)| p.room())
                .map(|(at, _)| at)?;
            let p = self.large.swap_remove(at);
            self.large_bytes = self.large_bytes.saturating_sub(item_bytes(p.room()));
            return Some(p.display);
        }
        self.taken = (
            self.taken.0.saturating_add(1),
            self.taken.1.saturating_add(item_bytes(room_for(want))),
        );
        let p = self.classes.get_mut(class_for(want))?.pop_back()?;
        self.count = self.count.saturating_sub(1);
        self.bytes = self.bytes.saturating_sub(item_bytes(p.room()));
        Some(p.display)
    }

    /// Keep `display`, empty, beside the `waiting` bytes of small displays
    /// the renderer has yet to let go of, within [`READY`] displays and
    /// [`READY_BYTES`]: past them the oldest of the largest class kept
    /// gives way to a smaller one, so what a full pool holds is the room
    /// most chunks need, and stays so cycle after cycle. A large display
    /// takes the place of the oldest large one kept.
    fn keep(&mut self, display: Arc<Display>, waiting: usize) {
        let room = display.items.capacity();
        let bytes = item_bytes(room);
        let kept = Pooled { display, at: self.now };
        if large(room) {
            if self.large.len() >= LARGE
                && let Some(at) = self
                    .large
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, p)| p.at)
                    .map(|(at, _)| at)
            {
                let gone = self.large.swap_remove(at);
                self.large_bytes = self.large_bytes.saturating_sub(item_bytes(gone.room()));
            }
            self.large.push(kept);
            self.large_bytes = self.large_bytes.saturating_add(bytes);
            return;
        }
        let class = class_of(room);
        let (most, most_bytes) = self.caps();
        while self.count >= most || self.bytes.saturating_add(waiting).saturating_add(bytes) > most_bytes {
            let above = class.saturating_add(1);
            let Some(gone) = self
                .classes
                .get_mut(above..)
                .and_then(|c| c.iter_mut().rev().find(|c| !c.is_empty()))
                .and_then(std::collections::VecDeque::pop_front)
            else {
                return;
            };
            self.count = self.count.saturating_sub(1);
            self.bytes = self.bytes.saturating_sub(item_bytes(gone.room()));
        }
        if self.classes.len() <= class {
            self.classes.resize_with(class.saturating_add(1), Default::default);
        }
        if let Some(c) = self.classes.get_mut(class) {
            c.push_back(kept);
            self.count = self.count.saturating_add(1);
            self.bytes = self.bytes.saturating_add(bytes);
        }
    }

    /// How many small displays, and bytes of their room, the pool keeps:
    /// [`READY`] and [`READY_BYTES`], or twice what a recent record took
    /// if more, so a repaint of more chunks than those (a thousand rows
    /// recorded again) takes back what the last freed, and allocates
    /// nothing. Recent: this window of [`AGE`] records and the one before,
    /// so once repaints shrink the caps do, and with [`AGE`] what an idle
    /// pool holds.
    fn caps(&self) -> (usize, usize) {
        let [a, b] = self.peak;
        let count = a.0.max(b.0).max(self.taken.0);
        let bytes = a.1.max(b.1).max(self.taken.1);
        (
            READY.max(count.saturating_mul(2)),
            READY_BYTES.max(bytes.saturating_mul(2)),
        )
    }

    /// One record passed: what waited [`AGE`] records unused is let go.
    fn age(&mut self) {
        let [now_peak, _] = &mut self.peak;
        *now_peak = (now_peak.0.max(self.taken.0), now_peak.1.max(self.taken.1));
        self.taken = (0, 0);
        if self.now.is_multiple_of(u64::from(AGE)) {
            let [last, _] = self.peak;
            self.peak = [(0, 0), last];
        }
        self.now = self.now.saturating_add(1);
        let now = self.now;
        for c in &mut self.classes {
            while let Some(p) = c.front().filter(|p| p.stale(now)) {
                self.bytes = self.bytes.saturating_sub(item_bytes(p.room()));
                self.count = self.count.saturating_sub(1);
                c.pop_front();
            }
        }
        let mut gone = 0_usize;
        self.large.retain(|p| {
            let keep = !p.stale(now);
            if !keep {
                gone = gone.saturating_add(item_bytes(p.room()));
            }
            keep
        });
        self.large_bytes = self.large_bytes.saturating_sub(gone);
    }

    /// The bytes of the room kept.
    fn held(&self) -> usize {
        self.bytes.saturating_add(self.large_bytes)
    }
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
    /// Whether a display with room for what it last held holds what it
    /// records next: it held as many the last time, so it records straight
    /// into one (`Painter::chunk`).
    steady: bool,
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
            offset: node.offset(SIGHT),
            opacity: node.look.opacity,
            scale: node.look.transform.scale,
            rotate: node.look.transform.rotate,
            clip: node.clip(SIGHT),
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

fn key(id: NodeId, role: Role) -> u64 {
    // The node's published id (one word, ADR-0026), with the role: a fixed
    // seed, so a chunk's id is the same in every frame.
    let mut h = foldhash::fast::FixedState::with_seed(0x9a1e_7c0d).build_hasher();
    id.published().hash(&mut h);
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
    let mut n = bare_in_space(node);
    n.content = node.content.clone();
    n
}

/// [`in_space`] written over `into`, the buffers of its content kept: a
/// chunk recorded again keeps what it records from without allocating.
fn in_space_into(node: &FrameNode, into: &mut FrameNode) {
    let mut content = std::mem::replace(&mut into.content, Content::None);
    match (&mut content, &node.content) {
        (
            Content::Text {
                content: c,
                text: t,
                runs: r,
                boxes: b,
            },
            Content::Text {
                content: from,
                text,
                runs,
                boxes,
            },
        ) => {
            c.clone_from(from);
            *t = text.clone();
            r.clone_from(runs);
            b.clone_from(boxes);
        }
        (to, from) => to.clone_from(from),
    }
    *into = bare_in_space(node);
    into.content = content;
}

/// [`in_space`] without its content.
fn bare_in_space(node: &FrameNode) -> FrameNode {
    let mut look = node.look.clone();
    look.opacity = 1.0;
    look.transform = crate::style::Painted {
        scale: 1.0,
        rotate: 0.0,
        origin: (0.0, 0.0),
    };
    look.effects.filters.clear();
    look.effects.mask = None;
    node.in_space(look)
}

/// Whether the caret of a text input shows now, its blink as `caret` says.
fn caret_on(node: &FrameNode, caret: CaretClock) -> bool {
    match &node.content {
        Content::Input { focused, mode, .. } => *focused && !mode.read_only && caret.shows(clock::frame_nanos()),
        _ => false,
    }
}

/// The window's background: the root's own colour when it paints the whole
/// window opaque (the page's colour, dark in a dark scheme), else
/// `fallback`. The canvas is cleared to it, and so is what a resize shows
/// beyond the last frame until the UI lays out at the new size.
#[must_use]
pub fn window_background(frame: &Frame, size: (u32, u32), scale: f32, fallback: Color) -> Color {
    let Some(root) = frame.nodes.first() else {
        return fallback;
    };
    let (w, h) = (
        crate::convert::u32_to_f32(size.0) / scale.max(1e-3),
        crate::convert::u32_to_f32(size.1) / scale.max(1e-3),
    );
    let b = root.bounds(SIGHT);
    let covers = b.x <= 0.0 && b.y <= 0.0 && b.x + b.width >= w - 0.5 && b.y + b.height >= h - 0.5;
    match root.look.background {
        Some(c) if covers && c.a >= 1.0 && root.look.opacity >= 1.0 && !root.look.invisible => c,
        _ => fallback,
    }
}

/// What restarts a focused input's blink when it changes: what it shows,
/// by its overlay chunk's id (the chunk that draws its caret).
fn caret_mark(node: &FrameNode) -> Option<(u64, crate::edit::ShownMark)> {
    match &node.content {
        Content::Input { edit, .. } => Some((key(node.id, Role::Overlay), edit.mark())),
        _ => None,
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
    /// A text input's own clip, its padding box (0 for none): what its
    /// content paints under, as Chrome clips an input's or a textarea's.
    field: u32,
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
            clip: self.clip,
            effect: self.effect,
        }
    }

    /// The state its overlay chunk paints under: a text input's content
    /// (recorded there) under its scroll offset (its content transform, as
    /// a scroll container's children are), within the input's own clip.
    fn over_state(self) -> State {
        if self.field == 0 {
            return self.state();
        }
        State {
            transform: self.content,
            clip: self.field,
            ..self.state()
        }
    }
}

/// A focused input: its caret blinks.
fn is_blinker(node: &FrameNode) -> bool {
    matches!(node.content, Content::Input { focused: true, .. })
}

/// The box the compositor hit-tests a node by.
fn hit_box(node: &FrameNode) -> HitBox {
    (node.space(SIGHT), node.look.shape, node.look.hit_slop)
}

/// `b` (laid out in the window) from the corner of `node`'s border box.
fn from_corner(node: &FrameNode, b: Bounds) -> Bounds {
    Bounds {
        x: b.x - node.bounds(SIGHT).x,
        y: b.y - node.bounds(SIGHT).y,
        ..b
    }
}

/// The spans of node `i` that take the pointer, each box its text took
/// with the span's index, from its corner.
fn span_boxes<'a>(
    frame: &'a Frame,
    i: usize,
    node: &'a FrameNode,
) -> impl Iterator<Item = crate::compositor::ShownSpan> + 'a {
    frame
        .behaviour(i)
        .into_iter()
        .flat_map(|b| b.spans.iter())
        .flat_map(move |s| {
            let (span, cursor) = (s.id(), s.cursor);
            s.boxes.iter().map(move |b| crate::compositor::ShownSpan {
                span,
                cursor,
                rect: from_corner(node, *b),
            })
        })
}

/// [`ShownText::Field`]'s `scrolls`: the id of the content transform
/// paint writes for a scrolling field, `content` (its box's offset plus
/// the scroll applied; [`ShownText::scrolled`] takes the box's transform
/// off it). It is a transform id, not a scroll amount. `None` for a
/// field that does not scroll.
fn scrolls(node: &FrameNode, content: u32) -> Option<u32> {
    node.scroll(SIGHT).map(|_| content)
}

/// The text node `i`'s box shows input as laid out now
/// ([`ShownText`]): a field's, or a paragraph's spans that take the
/// pointer; `None` for neither. `content`: the content transform paint
/// placed it with.
fn shown_text(frame: &Frame, i: usize, node: &FrameNode, content: u32) -> Option<ShownText> {
    match &node.content {
        Content::Input {
            edit,
            text,
            focused,
            multiline,
            preedit,
            ..
        } => Some(ShownText::Field {
            content: Arc::clone(&edit.text),
            text: text.clone(),
            area: from_corner(node, node.content_box(SIGHT)),
            multiline: *multiline,
            focused: *focused,
            scrolls: scrolls(node, content),
            offsets: edit.offsets(*preedit),
            caret: edit.caret,
            selection: edit.selection(),
            preedit: *preedit,
        }),
        _ if frame.behaviour(i).is_some_and(|b| !b.spans.is_empty()) => {
            Some(ShownText::Spans(span_boxes(frame, i, node).collect()))
        }
        _ => None,
    }
}

/// Whether node `i` shows input the text `was` holds (`None`: none), by
/// comparison alone: a frame that changes no text makes nothing.
fn shows_text(frame: &Frame, i: usize, node: &FrameNode, content_at: u32, was: Option<&ShownText>) -> bool {
    match (&node.content, was) {
        (
            Content::Input {
                edit,
                text,
                focused,
                multiline,
                preedit: shown_preedit,
                ..
            },
            Some(ShownText::Field {
                content,
                text: t,
                area,
                multiline: m,
                focused: f,
                scrolls: s,
                offsets,
                caret,
                selection,
                preedit,
            }),
        ) => {
            (Arc::ptr_eq(&edit.text, content) || *edit.text == **content)
                && edit.offsets(*shown_preedit) == *offsets
                && text == t
                && from_corner(node, node.content_box(SIGHT)) == *area
                && multiline == m
                && focused == f
                && scrolls(node, content_at) == *s
                && edit.caret == *caret
                && edit.selection() == *selection
                && shown_preedit == preedit
        }
        (Content::Input { .. }, _) => false,
        (_, Some(ShownText::Spans(boxes))) => {
            frame.behaviour(i).is_some_and(|b| !b.spans.is_empty())
                && span_boxes(frame, i, node).eq(boxes.iter().copied())
        }
        (_, Some(ShownText::Field { .. })) => false,
        (_, None) => frame.behaviour(i).is_none_or(|b| b.spans.is_empty()),
    }
}

/// What the compositor hit-tests a node by, as the UI does
/// ([`crate::shape::takes_pointer`]): its border box in its paint space,
/// its shape and its hit slop.
type HitBox = (Bounds, crate::shape::Shape, crate::style::Edges<f32>);

/// Whether the node paints over its subtree.
fn has_overlay(node: &FrameNode) -> bool {
    node.look.decoration.overlay.is_some() || matches!(node.content, Content::Input { .. })
}

/// A scroll container that shows a thumb.
fn has_thumb(node: &FrameNode) -> bool {
    node.scroll(SIGHT)
        .is_some_and(|s| s.max.1 > 0.0 && !s.thumb.is_transparent())
}

/// The ids of the transforms, clips and effects in-place writes (`undo`)
/// changed, each list sorted and without repeats.
fn written_ids(undo: &[Undo], t: &mut Vec<u32>, c: &mut Vec<u32>, e: &mut Vec<u32>) {
    t.clear();
    c.clear();
    e.clear();
    for u in undo {
        match u {
            Undo::Transform(id, _) => t.push(*id),
            Undo::Clip(id, _) => c.push(*id),
            Undo::Effect(id, _) => e.push(*id),
        }
    }
    for ids in [t, c, e] {
        ids.sort_unstable();
        ids.dedup();
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
    ///
    /// The commit is written into `commit` (one the renderer gave back,
    /// emptied), and each chunk recorded again into a display the renderer
    /// let go of: a repaint of what was painted before allocates nothing.
    #[expect(clippy::too_many_arguments, reason = "a frame, its target, and the commit to write")]
    pub fn record(
        &mut self,
        frame: &Frame,
        text: &mut TextSystem,
        size: (u32, u32),
        scale: f32,
        background: Color,
        resend: bool,
        commit: &mut Commit,
    ) {
        self.frame = self.frame.saturating_add(1);
        self.stats = PaintStats::default();
        self.painted.clear();
        commit.clear();
        self.reclaim();
        let target = (size, scale, text.generation(), background);
        let log = frame.changes.since(&mut self.read);
        let same = self.target == Some(target) && self.placed.len() == frame.nodes.len();
        self.target = Some(target);
        let in_place = match log {
            Since::Logged(log) if same => self.record_changes(frame, log, text, size, scale, commit).is_ok(),
            Since::Logged(_) | Since::Rebuilt | Since::FellBehind => false,
        };
        if !in_place {
            self.record_all(frame, text, size, scale, background, commit);
        }
        if resend {
            commit.between(&Scene::default(), &self.scene);
        }
        self.stats.chunks = self.scene.chunks.len();
    }

    /// A display to hold `want` items: one the renderer let go of
    /// ([`Pool::take`]), emptied, else a new one of that room.
    fn blank(&mut self, want: usize) -> Arc<Display> {
        match self.ready.take(want) {
            Some(mut d) => {
                let m = Arc::get_mut(&mut d);
                debug_assert!(m.is_some(), "the pool holds only displays nothing else shares");
                if let Some(m) = m {
                    m.renew();
                }
                d
            }
            None => Arc::new(Display::new(Vec::with_capacity(room_for(want)))),
        }
    }

    /// Keep `display`, which a chunk no longer shows, to record into once
    /// the renderer lets go of it.
    fn retire(&mut self, display: Arc<Display>) {
        self.retired.push(display);
    }

    /// Take back the displays the renderer let go of since the last record
    /// (it drew what replaced them), emptied at once so what their items
    /// held (an image) goes with them; those it still draws wait.
    ///
    /// A display keeps its room, so the chunk that takes it back records
    /// into it without allocating ([`Pool`]); one with a large room is cut
    /// to what it last held, and one no record took for [`AGE`] records is
    /// let go, so the pool holds what recent repaints use, within
    /// [`READY`] displays and [`READY_BYTES`], and [`LARGE`] large ones.
    /// Whether one is kept does not hang on where it sits among those
    /// retired: all those let go of are kept beside the small ones the
    /// renderer still draws.
    fn reclaim(&mut self) {
        self.ready.age();
        self.trim_recording();
        let mut freed = std::mem::take(&mut self.freed);
        freed.extend(self.retired.extract_if(.., |d| Arc::get_mut(d).is_some()));
        let waiting = self.retired.iter().fold(0_usize, |n, d| {
            let room = d.items.capacity();
            if large(room) {
                n
            } else {
                n.saturating_add(item_bytes(room))
            }
        });
        for mut d in freed.drain(..) {
            if let Some(m) = Arc::get_mut(&mut d) {
                let held = m.items.len();
                m.items.clear();
                if large(m.items.capacity()) && m.items.capacity() > held.saturating_mul(2).saturating_add(SLACK) {
                    m.items.shrink_to(held);
                }
                self.ready.keep(d, waiting);
            }
        }
        self.freed = freed;
    }

    /// One record passed: the scratch a chunk records into, when its room
    /// was large and past twice the most it held for [`AGE`] records, is
    /// cut to that.
    fn trim_recording(&mut self) {
        let room = self.recording.capacity();
        let peak = std::mem::take(&mut self.recording_peak);
        if large(room) && room > peak.saturating_mul(2).saturating_add(SLACK) {
            self.recording_wide = self.recording_wide.saturating_add(1);
            if self.recording_wide >= AGE {
                self.recording.shrink_to(peak);
                self.recording_wide = 0;
            }
        } else {
            self.recording_wide = 0;
        }
    }

    /// The scene as last recorded.
    #[must_use]
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The mount the scroll container at `key`, of the component instance
    /// `owner`, is in the tables as last recorded ([`ScrollNode::instance`]):
    /// what the compositor's scroll for it must name to be taken in
    /// (ADR-0033); none when no such container is there.
    pub(crate) fn scroll_instance(&self, key: NodeKey, owner: Option<crate::reactive::OwnerId>) -> Option<u64> {
        self.scroll_instances.get(&(key, owner)).copied()
    }

    /// Take in the scroll containers `frame` laid out, painted or not: one
    /// gone from it is unmounted, and one at its identity later is a new
    /// mount ([`ScrollNode::instance`]), though no paint saw the gap (a UI
    /// behind its input lays out each input's change). What the compositor
    /// moved the old one by then names a mount that is gone (ADR-0033).
    pub(crate) fn mount_scrollers(&mut self, frame: &crate::layout::Frame) {
        let mut was = std::mem::take(&mut self.scroll_instances);
        let mut now = std::mem::take(&mut self.scroll_spare);
        for (i, node) in frame.nodes.iter().enumerate() {
            if node.scroll(SIGHT).is_none() {
                continue;
            }
            let identity = (NodeKey::of(node.id), crate::interact::owner_of(frame, i));
            let instance = was.get(&identity).copied().unwrap_or_else(|| {
                self.next_scroll = self.next_scroll.wrapping_add(1);
                self.next_scroll
            });
            now.insert(identity, instance);
        }
        was.clear();
        self.scroll_spare = was;
        self.scroll_instances = now;
    }

    /// Record the whole frame (reusing each chunk whose input is the same).
    fn record_all(
        &mut self,
        frame: &Frame,
        text: &mut TextSystem,
        size: (u32, u32),
        scale: f32,
        background: Color,
        commit: &mut Commit,
    ) {
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
        // The focused inputs first: their carets' blink is known before
        // their chunks are recorded.
        self.blinkers.clear();
        self.blinkers.extend(
            frame
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| is_blinker(n))
                .map(|(i, _)| i),
        );
        self.follow_caret(frame);
        self.thumb_nodes.clear();
        let mut thumbs: Vec<usize> = Vec::new();
        let generation = text.generation();
        let mut layer = frame.nodes.first().map_or(0, |n| n.layer);
        for (i, node) in frame.nodes.iter().enumerate() {
            // A check's paint can be given up every so many nodes, so no
            // slice of it is a whole paint (ADR-0028, ADR-0034 S10). What
            // it recorded is its own painter's, dropped with it.
            if i % CANCEL_EVERY == 0 {
                crate::engine::cancel_point();
            }
            self.overlays(frame, &mut over, Some(i), &mut scene, scale, generation, text);
            if node.layer != layer {
                // A layer's thumbs paint over all of it.
                self.thumbs(
                    frame,
                    &thumbs,
                    &mut scene.chunks,
                    size,
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
            if has_thumb(node) {
                thumbs.push(i);
            }
            let display = self.chunk(node, Role::Chunk, p.snap, scale, generation, text);
            if !display.items.is_empty() {
                if let Some(c) = self.chunk_of.get_mut(i) {
                    *c = Some(scene.chunks.len());
                }
                scene.chunks.push(Chunk {
                    id: ChunkId(key(node.id, Role::Chunk)),
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
            &mut scene.chunks,
            size,
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
        // What left the frame lets go of its text and ids, and its display
        // is kept to record into.
        let mut gone = std::mem::take(&mut self.gone);
        self.chunks.retain(|_, r| {
            let keep = r.seen == now;
            if !keep {
                text.unpin(&r.pieces);
                gone.push(Arc::clone(&r.display));
            }
            keep
        });
        for d in gone.drain(..) {
            self.retire(d);
        }
        self.gone = gone;
        self.transforms.sweep(now);
        self.clips.sweep(now);
        self.effects.sweep(now);
        props.transforms.resize(self.transforms.len(), TransformNode::default());
        props.clips.resize(self.clips.len(), ClipNode::default());
        props.effects.resize(self.effects.len(), EffectNode::default());
        scene.properties = props;
        self.schedule_blink();
        commit.between(&self.scene, &scene);
        self.scene = scene;
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
        commit: &mut Commit,
    ) -> Result<(), Whole> {
        let generation = text.generation();
        // The blink as of this frame, before any caret is recorded (focus
        // that moved records it all again, and follows it there).
        self.follow_caret(frame);
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
        let mut undo = std::mem::take(&mut self.undo);
        undo.clear();
        let mut rewritten = std::mem::take(&mut self.rewritten);
        rewritten.clear();
        // Out while the nodes write it, an empty one (no roots: nothing
        // allocated) in its place.
        let mut props = std::mem::replace(
            &mut self.scene.properties,
            Properties {
                transforms: Vec::new(),
                clips: Vec::new(),
                effects: Vec::new(),
            },
        );
        let result = self.rewrite(
            frame,
            &work,
            &mut props,
            &mut undo,
            &mut rewritten,
            text,
            generation,
            size,
            scale,
        );
        self.scene.properties = props;
        self.work = work;
        let out = match result {
            Ok(compositing) => {
                for (at, chunk) in rewritten.drain(..) {
                    if let Some(slot) = self.scene.chunks.get_mut(at) {
                        *slot = chunk.clone();
                    }
                    commit.chunks.push(chunk);
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
                let (t, c, e) = &mut self.ids;
                written_ids(&undo, t, c, e);
                commit.properties.written(props, (t, c, e));
                let before = self.compositing.clone();
                if compositing {
                    let (scrollers, hits, moving) = self.tables(frame);
                    self.keep_compositing(scrollers, hits, moving);
                    self.scene.compositing = self.compositing.clone();
                }
                self.schedule_blink();
                commit.tables(&self.scene, &before);
                Ok(())
            }
            Err(whole) => {
                // Put back what was written in place: the whole record
                // compares with the scene as committed.
                commit.chunks.clear();
                let props = &mut self.scene.properties;
                for u in undo.drain(..).rev() {
                    match u {
                        Undo::Transform(id, old) => restore(&mut props.transforms, id, old),
                        Undo::Clip(id, old) => restore(&mut props.clips, id, old),
                        Undo::Effect(id, old) => restore(&mut props.effects, id, old),
                    }
                }
                Err(whole)
            }
        };
        rewritten.clear();
        self.rewritten = rewritten;
        undo.clear();
        self.undo = undo;
        out
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
        chunks: &mut Vec<(usize, Chunk)>,
        text: &mut TextSystem,
        generation: u64,
        size: (u32, u32),
        scale: f32,
    ) -> Result<bool, Whole> {
        let mut thumbs = std::mem::take(&mut self.thumb_work);
        thumbs.clear();
        let result = self
            .rewrite_nodes(
                frame,
                work,
                props,
                undo,
                chunks,
                &mut thumbs,
                text,
                generation,
                size,
                scale,
            )
            .and_then(|compositing| {
                self.rewrite_thumbs(frame, &thumbs, props, undo, chunks, size, scale)?;
                Ok(compositing)
            });
        self.thumb_work = thumbs;
        result
    }

    /// The thumbs of the scroll containers `rewrite` visited, at their
    /// places. Sized as the frame: a thumb's region is clipped to it.
    #[expect(clippy::too_many_arguments, reason = "what a record reads")]
    fn rewrite_thumbs(
        &mut self,
        frame: &Frame,
        thumbs: &[usize],
        props: &mut Properties,
        undo: &mut Vec<Undo>,
        chunks: &mut Vec<(usize, Chunk)>,
        size: (u32, u32),
        scale: f32,
    ) -> Result<(), Whole> {
        if thumbs.is_empty() {
            return Ok(());
        }
        let mut recorded = std::mem::take(&mut self.thumb_chunks);
        recorded.clear();
        self.thumbs(
            frame,
            thumbs,
            &mut recorded,
            size,
            &mut Writes {
                props,
                undo: Some(undo),
            },
            scale,
            false,
        );
        let mut out = Ok(());
        for (&i, chunk) in thumbs.iter().zip(recorded.drain(..)) {
            let Some(c) = self.thumb_of.get(i).copied().flatten() else {
                out = Err(Whole);
                break;
            };
            let kept = self.scene.chunks.get(c);
            if kept.is_none_or(|k| k.state != chunk.state || !Arc::ptr_eq(&k.display, &chunk.display)) {
                chunks.push((c, chunk));
            }
        }
        recorded.clear();
        self.thumb_chunks = recorded;
        out
    }

    /// The nodes `rewrite` visits: their chunks into `chunks`, the scroll
    /// containers among them into `thumbs`; whether the compositor's tables
    /// changed.
    #[expect(clippy::too_many_arguments, reason = "what a record reads")]
    fn rewrite_nodes(
        &mut self,
        frame: &Frame,
        work: &[usize],
        props: &mut Properties,
        undo: &mut Vec<Undo>,
        chunks: &mut Vec<(usize, Chunk)>,
        thumbs: &mut Vec<usize>,
        text: &mut TextSystem,
        generation: u64,
        size: (u32, u32),
        scale: f32,
    ) -> Result<bool, Whole> {
        let mut compositing = false;
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
            compositing |= node.scroll(SIGHT).is_some()
                || node.moving.is_some()
                || self.hit_changed(frame, i, node)
                || self.text_changed(frame, i, node);
            if has_thumb(node) {
                thumbs.push(i);
            }
            if let Some(c) = self.overlay_of.get(i).copied().flatten() {
                let chunk = Chunk {
                    id: ChunkId(key(node.id, Role::Overlay)),
                    state: p.over_state(),
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
                        id: ChunkId(key(node.id, Role::Chunk)),
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
        Ok(compositing)
    }

    /// Whether node `i`'s box the compositor hit-tests changed.
    fn hit_changed(&self, frame: &Frame, i: usize, node: &FrameNode) -> bool {
        frame.takes_pointer(i).then(|| hit_box(node)) != self.hit_rect.get(i).copied().flatten()
            || self.cursor_at(frame, i, node) != self.cursor_of.get(i).copied().flatten()
    }

    /// The cursor node `i` shows: its own, or its parent's as the last
    /// tables had it. A node whose own cursor changed is visited, so its
    /// tables are made again, and every node under it resolved afresh.
    fn cursor_at(&self, frame: &Frame, i: usize, node: &FrameNode) -> Option<crate::style::Cursor> {
        frame
            .behaviour(i)
            .and_then(|b| b.cursor)
            .or_else(|| node.parent.and_then(|q| self.cursor_of.get(q).copied().flatten()))
    }

    /// Whether the text node `i`'s box shows input changed
    /// ([`ShownText`]).
    fn text_changed(&self, frame: &Frame, i: usize, node: &FrameNode) -> bool {
        let was = self.shown_text.get(i).and_then(Option::as_deref);
        if frame.takes_pointer(i) {
            let content = self.placed.get(i).map_or(0, |p| p.content);
            !shows_text(frame, i, node, content, was)
        } else {
            was.is_some()
        }
    }

    /// Whether `node`'s caret shows otherwise than its overlay chunk, the
    /// one that draws it, recorded it. (Its box chunk draws none of the
    /// content and keeps through a blink, so what it holds of the caret is
    /// as old as its own last recording, not the caret drawn.)
    fn caret_changed(&self, node: &FrameNode) -> bool {
        let id = ChunkId(key(node.id, Role::Overlay));
        self.chunks.get(&id).is_some_and(
            |r| matches!(&r.input, Input::Node { caret, .. } if *caret != caret_on(node, self.caret.clock)),
        )
    }

    /// The compositor's tables of the whole frame: scroll containers (with
    /// their thumbs), the boxes it hit-tests, the nodes it moves.
    fn tables(&mut self, frame: &Frame) -> (Vec<ScrollNode>, Vec<Hit>, Vec<MovingNode>) {
        let mut scrollers: Vec<ScrollNode> = Vec::new();
        let mut hits: Vec<Hit> = Vec::new();
        let mut moving: Vec<MovingNode> = Vec::new();
        let was = std::mem::take(&mut self.scroll_instances);
        // The scroll container each node is in, itself when it is one.
        let mut scroller_of: Vec<Option<u32>> = Vec::with_capacity(frame.nodes.len());
        // Whether a wheel handler is on the node or on an ancestor below
        // its scroll container (the container itself excluded).
        let mut wheel_below: Vec<bool> = Vec::with_capacity(frame.nodes.len());
        // The component instance whose output each node is in (the nearest
        // marked root above it, itself included): the node's identity, with
        // its path, which a wheel the UI is handed names.
        let mut instance_of: Vec<Option<crate::reactive::OwnerId>> = Vec::with_capacity(frame.nodes.len());
        self.hit_rect.clear();
        self.cursor_of.clear();
        std::mem::swap(&mut self.shown_text, &mut self.shown_text_was);
        self.shown_text.clear();
        self.texts.clear();
        // Below the top modal layer the wheel scrolls nothing (ADR-0024):
        // a window-sized hit in no scroll container, under that layer.
        let floor = crate::layer::Stack::of(frame).floor;
        for (i, node) in frame.nodes.iter().enumerate() {
            if i == floor && floor > 0 {
                hits.push(Hit {
                    rect: frame.viewport(SIGHT),
                    shape: crate::shape::Shape::default(),
                    slop: crate::style::Edges::all(0.0),
                    transform: 0,
                    clip: 0,
                    effect: 0,
                    scroller: None,
                    wheel: false,
                    target: crate::compositor::HitTarget {
                        key: NodeKey(0),
                        owner: None,
                    },
                    cursor: None,
                    below_modal: false,
                });
            }
            // Its parent's is resolved already (parents come first).
            let cursor = self.cursor_at(frame, i, node);
            self.cursor_of.push(cursor);
            let p = self.placed.get(i).copied().unwrap_or_default();
            let around = node.parent.and_then(|q| scroller_of.get(q).copied().flatten());
            let handles = frame.behaviour(i).is_some_and(|b| b.handlers.wheel.is_some());
            let instance = frame
                .behaviour(i)
                .and_then(|b| b.handlers.instance)
                .or_else(|| node.parent.and_then(|q| instance_of.get(q).copied().flatten()));
            instance_of.push(instance);
            // A parent that scrolls ends the run: what is above it is its own.
            let inherited = node.parent.is_some_and(|q| {
                frame.nodes.get(q).is_some_and(|n| n.scroll(SIGHT).is_none())
                    && wheel_below.get(q).copied().unwrap_or(false)
            });
            wheel_below.push(node.scroll(SIGHT).is_none() && (handles || inherited));
            let within = match node.scroll(SIGHT) {
                Some(sc) => {
                    let index = u32::try_from(scrollers.len()).ok();
                    let key = NodeKey::of(node.id);
                    // Its identity as the UI names it (`interact::owner_of`).
                    let owner = instance.or_else(|| frame.behaviour(i).and_then(|b| b.handlers.owner));
                    let identity = (key, owner);
                    let instance = was.get(&identity).copied().unwrap_or_else(|| {
                        self.next_scroll = self.next_scroll.wrapping_add(1);
                        self.next_scroll
                    });
                    self.scroll_instances.insert(identity, instance);
                    scrollers.push(ScrollNode {
                        key,
                        owner,
                        instance,
                        content: p.content,
                        offset: sc.offset,
                        applied: sc.applied,
                        max: sc.max,
                        pos: sc.pos,
                        end: sc.end,
                        rtl: node.look.text.direction == crate::style::Direction::Rtl,
                        generation: sc.generation,
                        thumb: self.thumb_nodes.get(&key).copied(),
                        parent: around,
                        wheel: handles,
                        wheel_above: inherited,
                        below_modal: i < floor,
                    });
                    index
                }
                None => around,
            };
            scroller_of.push(within);
            let takes = frame.takes_pointer(i);
            self.hit_rect.push(takes.then(|| hit_box(node)));
            // The text its box shows input: the one the last tables had,
            // while it shows the same.
            let was = self.shown_text_was.get_mut(i).and_then(Option::take);
            let text = takes
                .then(|| match was {
                    Some(t) if shows_text(frame, i, node, p.content, Some(&t)) => Some(t),
                    _ => shown_text(frame, i, node, p.content).map(Arc::new),
                })
                .flatten();
            if let (Some(t), Ok(at)) = (&text, u32::try_from(hits.len())) {
                self.texts.push((at, Arc::clone(t)));
            }
            self.shown_text.push(text);
            if takes {
                let (rect, shape, slop) = hit_box(node);
                hits.push(Hit {
                    rect,
                    shape,
                    slop,
                    transform: p.transform,
                    clip: p.clip,
                    effect: p.effect,
                    scroller: within,
                    wheel: wheel_below.last().copied().unwrap_or(false),
                    target: crate::compositor::HitTarget {
                        key: NodeKey::of(node.id),
                        owner: instance.or_else(|| frame.behaviour(i).and_then(|b| b.handlers.owner)),
                    },
                    cursor,
                    below_modal: i < floor,
                });
            }
            if let Some(m) = &node.moving {
                moving.push(MovingNode {
                    key: NodeKey::of(node.id),
                    transform: Some(p.transform),
                    effect: Some(p.effect),
                    moving: m.clone(),
                    space: node.space(SIGHT),
                    invisible: node.look.invisible,
                    turns: m.curves.turns(),
                });
            }
        }
        (scrollers, hits, moving)
    }

    /// Keep the compositor's tables, each the same `Arc` while equal, and
    /// their index ([`crate::compositor::TableIndex`]) made again only
    /// when what it indexes changed: a scroller's place or a hit's reach,
    /// not an offset (at the cost of the comparison already made).
    fn keep_compositing(&mut self, scrollers: Vec<ScrollNode>, hits: Vec<Hit>, moving: Vec<MovingNode>) {
        let c = &mut self.compositing;
        let mut reindex = false;
        if *c.scrollers != *scrollers {
            reindex |= !c
                .scrollers
                .iter()
                .map(|s| s.instance)
                .eq(scrollers.iter().map(|s| s.instance));
            c.scrollers = std::sync::Arc::from(scrollers);
        }
        if *c.hits != *hits {
            let reach = |h: &Hit| (h.target, h.below_modal);
            reindex |= !c.hits.iter().map(reach).eq(hits.iter().map(reach));
            c.hits = std::sync::Arc::from(hits);
        }
        if reindex {
            c.index = std::sync::Arc::new(crate::compositor::TableIndex::of(&c.scrollers, &c.hits));
        }
        if *c.moving != *moving {
            c.moving = std::sync::Arc::from(moving);
        }
        let same = |a: &(u32, Arc<ShownText>), b: &(u32, Arc<ShownText>)| a.0 == b.0 && Arc::ptr_eq(&a.1, &b.1);
        if c.texts.len() != self.texts.len() || !c.texts.iter().zip(&self.texts).all(|(a, b)| same(a, b)) {
            c.texts = std::sync::Arc::from(self.texts.as_slice());
        }
    }

    /// What the last [`Painter::record`] did.
    #[must_use]
    pub fn stats(&self) -> PaintStats {
        self.stats
    }

    /// The chunks kept and the bytes they hold: each chunk's record, its
    /// items and its pieces' ids, and the displays kept to record into
    /// again (the memory report's paint owner).
    #[must_use]
    pub fn held(&self) -> (usize, u64) {
        let size = |n: usize, each: usize| u64::try_from(n.saturating_mul(each)).unwrap_or(u64::MAX);
        let table = size(self.chunks.capacity(), std::mem::size_of::<(ChunkId, Retained)>());
        let bytes = self.chunks.values().fold(table, |n, r| {
            n.saturating_add(size(r.display.items.capacity(), std::mem::size_of::<Item>()))
                .saturating_add(size(r.pieces.capacity(), std::mem::size_of::<u64>()))
        });
        // The displays kept to record into again, and those waiting for the
        // renderer to let go of them.
        let bytes = bytes.saturating_add(u64::try_from(self.ready.held()).unwrap_or(u64::MAX));
        let bytes = self.retired.iter().fold(bytes, |n, d| {
            n.saturating_add(size(d.items.capacity(), std::mem::size_of::<Item>()))
        });
        // The scratch a chunk of unknown size records into.
        let bytes = bytes.saturating_add(size(self.recording.capacity(), std::mem::size_of::<Item>()));
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
                (node.bounds(SIGHT).x + t.origin.0) * scale,
                (node.bounds(SIGHT).y + t.origin.1) * scale,
            ),
            scale: t.scale,
            rotate: t.rotate,
        });
        let transform = if node.translating || matrix.is_some() || node.moving.is_some() {
            let id = self.transforms.id(key(node.id, Role::Transform), now);
            w.transform(
                id,
                TransformNode {
                    parent: up.content,
                    offset: node.offset(SIGHT),
                    matrix,
                },
            );
            id
        } else {
            up.content
        };
        // The clip of the node that clips it: that node's own padding box
        // and corners, within the clip it lies in itself (one tree of
        // clips, ADR-0016: the shaders multiply each one's coverage, so a
        // rounded clip inside another keeps both one's corners; the clip
        // layout intersected, `node.clip`, only culls).
        // A node is clipped when it has a clipper, as hit testing has it
        // (`clip::hit_in`, which reads this clip).
        let clip = match node.clipper {
            Some(c) => {
                let owner = frame.nodes.get(c);
                let id = owner.map_or(0, |o| self.clips.id(key(o.id, Role::Clip), now));
                let up = self.placed.get(c);
                let space = up.map_or(0, |o| o.transform);
                // One chain across transforms: each clip carries its own
                // space ([`ClipNode::transform`]), so the clip a transformed
                // group lies in is its clips' parent as any other is.
                let parent = up.map_or(0, |o| o.clip);
                let bounds = owner.map_or(Bounds::default(), crate::clip::clip_box);
                w.clip(
                    id,
                    ClipNode {
                        clip: Clip {
                            bounds,
                            radius: node.clip_radius,
                            curve: node.clip_curve,
                        },
                        transform: space,
                        parent,
                    },
                );
                id
            }
            None => 0,
        };
        let effect = if effects::grouped(node) || node.moving.is_some() {
            let id = self.effects.id(key(node.id, Role::Effect), now);
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
                    region,
                    clip,
                },
            );
            id
        } else {
            if let Some(r) = self.region.get_mut(i) {
                *r = false;
            }
            up.effect
        };
        let content = match node.scroll(SIGHT) {
            Some(s) => {
                let id = self.transforms.id(key(node.id, Role::Content), now);
                w.transform(
                    id,
                    TransformNode {
                        parent: transform,
                        offset: (node.offset(SIGHT).0 + s.applied.0, node.offset(SIGHT).1 + s.applied.1),
                        matrix: None,
                    },
                );
                id
            }
            None => transform,
        };
        // A text input clips its content (its text scrolled under its
        // content transform, as a scroll container's children are) to its
        // padding box, moving with it, within the clip of the container
        // around it, which stays with that container ([`ClipNode::parent`]):
        // as what is between them scrolls or moves, what comes into the
        // container shows.
        let field = if matches!(node.content, Content::Input { .. }) {
            let b = node.bounds(SIGHT);
            let radius = node.look.shape.used(b.width, b.height);
            let (padding, radius) = border_inner(b, radius, node.look.border).unwrap_or((b, radius));
            let id = self.clips.id(key(node.id, Role::Clip), now);
            w.clip(
                id,
                ClipNode {
                    clip: Clip {
                        bounds: padding,
                        radius,
                        curve: node.look.shape.curve(),
                    },
                    transform,
                    parent: clip,
                },
            );
            id
        } else {
            0
        };
        Placed {
            transform,
            content,
            effect,
            clip,
            field,
            snap: up.snap && matrix.is_none(),
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
        let id = ChunkId(key(node.id, role));
        let caret = caret_on(node, self.caret.clock);
        // What it may record: what its clip shows, and a margin.
        // A text input's content (its overlay chunk) lies under its scroll.
        let scrolled = match (overlay, &node.content, node.scroll(SIGHT)) {
            (true, Content::Input { .. }, Some(s)) => s.applied,
            _ => (0.0, 0.0),
        };
        let visible = node
            .clip(SIGHT)
            .map(|c| c.translated(-node.offset(SIGHT).0 - scrolled.0, -node.offset(SIGHT).1 - scrolled.1));
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
            // A text input's box chunk draws none of its content (its
            // overlay chunk does): a keystroke or a blink keeps it.
            let boxed = !overlay && matches!(node.content, Content::Input { .. });
            let same = was.space(SIGHT) == node.space(SIGHT)
                && was.space_content(SIGHT) == node.space_content(SIGHT)
                && *s == snap
                && (boxed || *c == caret)
                && *k == scale
                && *g == generation
                && (boxed || was.content == node.content)
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
                        bounds: node.bounds(SIGHT),
                        recorded: false,
                    });
                    r.properties = moved;
                }
                return Arc::clone(&r.display);
            }
            if was.space(SIGHT).width == node.space(SIGHT).width && was.space(SIGHT).height == node.space(SIGHT).height
            {
                self.stats.repainted = self.stats.repainted.saturating_add(1);
            } else {
                self.stats.relaid = self.stats.relaid.saturating_add(1);
            }
        }
        self.stats.recorded = self.stats.recorded.saturating_add(1);
        self.painted.push(Repaint {
            bounds: node.bounds(SIGHT),
            recorded: true,
        });
        let cull = visible.map(|v| v.outset(CULL_MARGIN));
        // A chunk that held as many the last time it recorded (`steady`)
        // records straight into a display with room for that (`Pool`);
        // one new, or whose size changed, into scratch first, then moved
        // into the display that fits what it recorded.
        let last = self.chunks.get(&id).map(|r| (r.display.items.len(), r.steady));
        let mut display = match last {
            Some((held, true)) => Some(self.blank(held)),
            _ => None,
        };
        let mut scratch = std::mem::take(&mut self.recording);
        scratch.clear();
        let shared = display.as_mut().is_some_and(|d| Arc::get_mut(d).is_none());
        debug_assert!(!shared, "a display just taken is shared with nothing");
        if shared {
            display = None;
        }
        let direct = display.as_mut().and_then(Arc::get_mut);
        let room = direct.as_ref().map(|d| d.items.capacity());
        let items: &mut Vec<Item> = match direct {
            Some(d) => &mut d.items,
            None => &mut scratch,
        };
        // The node as recorded, written over what the chunk kept of it.
        let mut fresh = None;
        let view: &FrameNode = match self.chunks.get_mut(&id) {
            Some(Retained {
                input: Input::Node { node: kept, .. },
                ..
            }) => {
                in_space_into(node, kept);
                kept
            }
            _ => fresh.insert(Box::new(in_space(node))),
        };
        text.clear_drawn();
        let mut target = Target {
            items,
            scale,
            size: (u32::MAX / 4, u32::MAX / 4),
            cull,
            snap,
        };
        if overlay {
            if !view.look.invisible {
                // A text input's content, under its own clip.
                if matches!(view.content, Content::Input { .. }) {
                    record_content(&mut target, view, text, caret);
                }
                record_overlay(&mut target, view);
            }
        } else {
            record_node(view, &mut target, text, cull, caret);
        }
        let recorded = target.items.len();
        // It stays steady while the display taken for what it held the
        // last time holds what it recorded now, and the other way round
        // (a chunk that grows and shrinks by turns records through scratch).
        let grew = room.is_some_and(|room| target.items.capacity() != room);
        let steady = !grew && last.is_none_or(|(held, _)| like(held, recorded));
        let display = match display {
            Some(d) => d,
            None => {
                self.recording_peak = self.recording_peak.max(recorded);
                let mut d = self.blank(recorded);
                let m = Arc::get_mut(&mut d);
                debug_assert!(m.is_some(), "a display just taken is shared with nothing");
                if let Some(m) = m {
                    m.items.append(&mut scratch);
                }
                d
            }
        };
        self.recording = scratch;
        text.pin_drawn();
        let replaced = match (self.chunks.get_mut(&id), fresh) {
            (Some(r), None) => {
                if let Input::Node {
                    snap: s,
                    caret: c,
                    scale: k,
                    text: g,
                    cull: was_cull,
                    ..
                } = &mut r.input
                {
                    (*s, *c, *k, *g, *was_cull) = (snap, caret, scale, generation, cull);
                }
                text.unpin(&r.pieces);
                text.swap_drawn(&mut r.pieces);
                r.properties = moved;
                r.seen = now;
                r.steady = steady;
                Some(std::mem::replace(&mut r.display, Arc::clone(&display)))
            }
            (_, fresh) => {
                let pieces = text.take_drawn();
                let input = Input::Node {
                    node: fresh.unwrap_or_else(|| Box::new(in_space(node))),
                    snap,
                    caret,
                    scale,
                    text: generation,
                    cull,
                };
                self.chunks
                    .insert(
                        id,
                        Retained {
                            input,
                            display: Arc::clone(&display),
                            pieces,
                            properties: moved,
                            seen: now,
                            steady,
                        },
                    )
                    .map(|old| {
                        text.unpin(&old.pieces);
                        old.display
                    })
            }
        };
        if let Some(old) = replaced {
            self.retire(old);
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
                id: ChunkId(key(node.id, Role::Overlay)),
                state: p.over_state(),
                display,
            });
        }
    }

    /// The thumbs of `scrollers`, each a chunk under a transform its scroll
    /// offset moves: scrolling records nothing again.
    #[expect(clippy::too_many_arguments, reason = "what a record reads")]
    fn thumbs(
        &mut self,
        frame: &Frame,
        scrollers: &[usize],
        out: &mut Vec<Chunk>,
        size: (u32, u32),
        w: &mut Writes<'_>,
        scale: f32,
        whole: bool,
    ) {
        let now = self.frame;
        for &i in scrollers {
            let (Some(node), Some(p)) = (frame.nodes.get(i), self.placed.get(i).copied()) else {
                continue;
            };
            let Some(scroll) = node.scroll(SIGHT) else { continue };
            let b = node.space(SIGHT).snapped(scale);
            let content_h = b.height + scroll.max.1;
            let thumb_h = (b.height * b.height / content_h).max(20.0);
            let travel = (b.height - thumb_h - 4.0).max(0.0);
            let along = travel * (scroll.offset.1 / scroll.max.1);
            let transform = self.transforms.id(key(node.id, Role::ThumbTransform), now);
            w.transform(
                transform,
                TransformNode {
                    // Under the container's own transform: what moves it moves its thumb.
                    parent: p.transform,
                    offset: (node.offset(SIGHT).0, node.offset(SIGHT).1 + along),
                    matrix: None,
                },
            );
            // An overlay scrollbar (Chrome's on macOS, cc's
            // `ScrollbarAnimationController`): hidden at rest, shown by the
            // compositor while the user scrolls it, then faded out. Its
            // opacity is this effect's, which only the compositor moves.
            let effect = self.effects.id(key(node.id, Role::ThumbEffect), now);
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
                            x: node.bounds(SIGHT).right() - 8.0,
                            width: 8.0,
                            ..node.bounds(SIGHT)
                        },
                        scale,
                        size,
                    ),
                    ..EffectNode::default()
                },
            );
            self.thumb_nodes.insert(
                NodeKey::of(node.id),
                ThumbNode {
                    transform,
                    effect,
                    top: node.offset(SIGHT),
                    travel,
                },
            );
            let id = ChunkId(key(node.id, Role::Thumb));
            let same = self.chunks.get(&id).is_some_and(|r| {
                matches!(r.input, Input::Thumb { space, max, thumb, scale: k }
                    if space == node.space(SIGHT) && max == scroll.max && thumb == scroll.thumb && k == scale)
            });
            let display = if let Some(r) = self.chunks.get_mut(&id).filter(|_| same) {
                r.seen = now;
                Arc::clone(&r.display)
            } else {
                self.stats.recorded = self.stats.recorded.saturating_add(1);
                self.painted.push(Repaint {
                    bounds: Bounds {
                        x: node.bounds(SIGHT).right() - 8.0,
                        width: 8.0,
                        ..node.bounds(SIGHT)
                    },
                    recorded: true,
                });
                let thumb = Bounds {
                    x: b.right() - 8.0,
                    y: b.y + 2.0,
                    width: 6.0,
                    height: thumb_h,
                };
                let mut display = self.blank(1);
                if let Some(d) = Arc::get_mut(&mut display) {
                    d.items.push(Item::RoundedRect {
                        bounds: thumb,
                        radius: Corners::all(3.0),
                        color: scroll.thumb,
                    });
                }
                if let Some(old) = self.chunks.insert(
                    id,
                    Retained {
                        input: Input::Thumb {
                            space: node.space(SIGHT),
                            max: scroll.max,
                            thumb: scroll.thumb,
                            scale,
                        },
                        display: Arc::clone(&display),
                        pieces: Vec::new(),
                        properties: Moved::of(node),
                        seen: now,
                        steady: true,
                    },
                ) {
                    self.retire(old.display);
                }
                display
            };
            if whole && let Some(t) = self.thumb_of.get_mut(i) {
                *t = Some(out.len());
            }
            out.push(Chunk {
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
    /// Its carets blink as this painter's do, from when they began.
    #[cfg(debug_assertions)]
    pub(crate) fn cold(
        &self,
        frame: &Frame,
        text: &mut TextSystem,
        size: (u32, u32),
        scale: f32,
        background: Color,
    ) -> Scene {
        let mut p = Self {
            caret: self.caret.clone(),
            window_inactive: self.window_inactive,
            activations: self.activations,
            ..Self::default()
        };
        p.record(frame, text, size, scale, background, true, &mut Commit::default());
        p.cancel_blink();
        for r in p.chunks.values() {
            text.unpin(&r.pieces);
        }
        std::mem::take(&mut p.scene)
    }

    /// The scene `frame` records into from nothing, by a painter with no
    /// history: what input to a frame no renderer presented is hit-tested
    /// against ([`crate::interact::hit`]), its boxes and property trees as
    /// paint makes them for any frame.
    pub(crate) fn fresh(frame: &Frame, text: &mut TextSystem, size: (u32, u32), scale: f32) -> Scene {
        let mut p = Self::default();
        p.record(frame, text, size, scale, Color::WHITE, true, &mut Commit::default());
        p.cancel_blink();
        for r in p.chunks.values() {
            text.unpin(&r.pieces);
        }
        std::mem::take(&mut p.scene)
    }

    /// Whether the window has the keyboard focus: a caret shows and blinks
    /// only while it has, read at the next record.
    pub fn set_window_active(&mut self, active: bool) {
        if active && self.window_inactive {
            self.activations = self.activations.wrapping_add(1);
        }
        self.window_inactive = !active;
    }

    /// Restart the blink, shown, when the window gained or lost the focus
    /// (both, between two records, included) or a focused input's text,
    /// caret or selection changed since the last record (focus moved to
    /// another input included).
    fn follow_caret(&mut self, frame: &Frame) {
        let active = !self.window_inactive;
        let marks = &self.caret.marks;
        let same = active == self.caret.clock.active
            && self.activations == self.caret.activation
            && marks.len() == self.blinkers.len()
            && self
                .blinkers
                .iter()
                .zip(marks)
                .all(|(&b, mark)| frame.nodes.get(b).and_then(caret_mark).as_ref() == Some(mark));
        if same {
            return;
        }
        self.caret.clock = CaretClock {
            since: clock::frame_nanos(),
            active,
        };
        self.caret.activation = self.activations;
        self.caret.marks.clear();
        for &b in &self.blinkers {
            if let Some(mark) = frame.nodes.get(b).and_then(caret_mark) {
                self.caret.marks.push(mark);
            }
        }
    }

    fn cancel_blink(&mut self) {
        if let Some(t) = self.blink.take() {
            clock::cancel(t);
        }
    }

    /// Wake for the next time a focused caret shows or hides, if it will.
    fn schedule_blink(&mut self) {
        self.cancel_blink();
        if self.blinkers.is_empty() {
            return;
        }
        if let Some(next) = self.caret.clock.next_change(clock::frame_nanos()) {
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
        && c.intersect(&node.bounds(SIGHT).outset(16.0)).is_empty()
    {
        return;
    }
    record_box(target, node, text);
    // A text input's content is its overlay chunk's, clipped ([`Placed::field`]).
    if !matches!(node.content, Content::Input { .. }) {
        record_content(target, node, text, caret);
    }
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
    let b = target.snapped(node.bounds(SIGHT));
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
    let b = target.snapped(node.bounds(SIGHT));
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
            let b = target.snapped(node.bounds(SIGHT));
            for shape in shapes {
                target.items.push(Item::Path {
                    origin: (b.x, b.y),
                    path: shape.path.clone(),
                    style: shape.style.clone(),
                });
            }
        }
        Content::Drawn(d) => {
            let key = foldhash::fast::FixedState::with_seed(0x51c7_d2a9).hash_one(node.id.published());
            d.shape(text);
            d.draw(node.content_box(SIGHT), target.scale, key, target.items);
        }
        Content::Image { source, radius } => {
            target.items.push(Item::Image {
                source: source.clone(),
                bounds: target.snapped(node.bounds(SIGHT)),
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
            let area = &node.content_box(SIGHT);
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
            let side = node.bounds(SIGHT).width.min(node.bounds(SIGHT).height);
            target.items.push(Item::Icon {
                icon: *icon,
                size: physical(side, scale),
                color: *color,
                stroke_width: *stroke_width,
                x: physical_pos(node.bounds(SIGHT).x + (node.bounds(SIGHT).width - side) / 2.0, scale),
                y: physical_pos(node.bounds(SIGHT).y + (node.bounds(SIGHT).height - side) / 2.0, scale),
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
            let area = &node.content_box(SIGHT);
            // Under the field's scroll offset (its content transform) and
            // clipped to it ([`Placed::field`]): laid out unscrolled.
            let it = input_text(text, &edit.text, t, *area, *multiline, *focused, (0.0, 0.0));
            let (line_t, width, origin) = (it.t, it.width, it.origin);
            let caret = text.caret(&edit.text, &line_t, width, edit.caret);
            if *focused && edit.has_selection() {
                let (from, to) = edit.selection();
                for r in text.selection(&*edit.text, &line_t, width, from, to) {
                    let b = Bounds {
                        x: origin.0 + r.x,
                        y: origin.1 + r.y,
                        ..r
                    };
                    target.fill_rect(b, *selection_color);
                }
            }
            if edit.text.is_empty() {
                text.draw(
                    target,
                    placeholder.as_str(),
                    &ResolvedText {
                        color: *placeholder_color,
                        ..line_t.clone()
                    },
                    width,
                    origin,
                    1.0,
                );
            } else {
                text.draw(target, &*edit.text, &line_t, width, origin, 1.0);
            }
            // What an input method is composing is underlined, as a
            // browser marks a composition, until it is committed.
            if let Some((from, to)) = *preedit {
                for r in text.selection(&*edit.text, &line_t, width, from, to) {
                    let b = Bounds {
                        x: origin.0 + r.x,
                        y: origin.1 + r.y + r.height - 1.0,
                        width: r.width,
                        height: 1.0,
                    };
                    target.fill_rect(b, t.color);
                }
            }
            if caret_shown {
                let b = Bounds {
                    // Kept in a box its text wraps or is aligned in (one
                    // that scrolls holds its caret's pixel: `field_reach`).
                    x: if width.is_some() {
                        (origin.0 + caret.x).min(area.right() - 1.0)
                    } else {
                        origin.0 + caret.x
                    },
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
    type Shown<'s> = (Vec<&'s Item>, Vec<crate::scene::Depends>);
    /// A chunk's items the window shows, under all its state depends on,
    /// read as values ([`Properties::depends`]: every clip, transform and
    /// effect its state reaches, not only its own, its groups numbered in
    /// `groups`).
    fn shown<'s>(s: &'s Scene, c: &'s Chunk, groups: &mut crate::scene::Groups) -> Option<Shown<'s>> {
        let p = &s.properties;
        let turned = p.matrices(c.state.transform, 0).next().is_some();
        let o = p.transform(c.state.transform).offset;
        let (mut x_min, mut y_min) = (0.0_f32, 0.0_f32);
        let (mut w, mut h) = (
            crate::convert::u32_to_f32(s.size.0),
            crate::convert::u32_to_f32(s.size.1),
        );
        // Within each clip of its chain too, each moved into window pixels
        // by its own transform before they are intersected (a scrolled
        // child clip lies elsewhere in its own space than in its parent's).
        // A clip under a scale or turn is not culled by.
        for (_, k) in p.clip_chain(c.state.clip) {
            if p.matrices(k.transform, 0).next().is_some() {
                continue;
            }
            let t = p.transform(k.transform).offset;
            let b = k.clip.bounds.translated(t.0, t.1);
            x_min = x_min.max(b.x * s.scale);
            y_min = y_min.max(b.y * s.scale);
            w = w.min(b.right() * s.scale);
            h = h.min(b.bottom() * s.scale);
        }
        // Clips that do not meet leave nothing to paint, whatever an item
        // reaches (one spanning both would pass each bound on its own).
        if x_min >= w || y_min >= h {
            return None;
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
        (!items.is_empty()).then(|| (items, p.depends(c.state, groups)))
    }
    let name = |id: ChunkId| {
        frame
            .nodes
            .iter()
            .position(|n| {
                [Role::Chunk, Role::Overlay, Role::Thumb]
                    .into_iter()
                    .any(|r| key(n.id, r) == id.0)
            })
            .map_or_else(|| format!("{id:?}"), |i| frame.path_of(i).to_string())
    };
    if a.background != b.background || a.size != b.size || a.scale != b.scale {
        return Some("the target".to_owned());
    }
    fn list(s: &Scene) -> Vec<(ChunkId, Shown<'_>)> {
        // One numbering of its effect groups a scene, in paint order.
        let mut groups = crate::scene::Groups::default();
        s.chunks
            .iter()
            .filter_map(|c| shown(s, c, &mut groups).map(|x| (c.id, x)))
            .collect()
    }
    let (x, y) = (list(a), list(b));
    if x.len() != y.len() {
        let ids = |l: &[(ChunkId, Shown<'_>)]| l.iter().map(|c| c.0).collect::<std::collections::BTreeSet<_>>();
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
            return Some(format!("chunk {}'s state: {state_a:?} against {state_b:?}", name(*ia)));
        }
    }
    None
}

#[cfg(test)]
#[expect(clippy::arithmetic_side_effects, reason = "test rooms are far from overflowing")]
mod tests {
    use super::*;

    #[test]
    fn a_caret_blinks_from_its_last_change_until_it_has_been_idle_a_while() {
        let c = CaretClock {
            since: clock::nanos_of(100.0),
            active: true,
        };
        assert!(c.shows(100.0) && c.shows(100.5), "shown at once, for a blink");
        assert!(!c.shows(100.6), "then hidden");
        assert_eq!(c.next_change(100.0), Some(BLINK));
        // 10.2 s on is in a hidden half (the 20th), past the timeout: shown,
        // and nothing more to wake for.
        assert!(c.shows(110.2));
        assert_eq!(c.next_change(110.2), None);
        // The last wake is the timeout itself, in a half that would hide.
        let late = 100.0 + CARET_BLINK_TIMEOUT - 0.1;
        assert!((c.next_change(late).unwrap() - 0.1).abs() < 1e-3);
        // An inactive window shows no caret, and wakes for none.
        let inactive = CaretClock { active: false, ..c };
        assert!(!inactive.shows(100.0));
        assert_eq!(inactive.next_change(100.0), None);
    }

    /// A caret blinks the same at 262,144 s of uptime (three days, where an
    /// f32 of seconds steps by 1/64 s) as at 1 s: shown and hidden at the
    /// same frames, its next change due at the same time to 1e-4 s.
    #[test]
    fn a_caret_blinks_the_same_at_any_uptime() {
        let ms = |n: u64| n * 1_000_000;
        let blinks = |base: u64| {
            let c = CaretClock {
                since: base + 4_321,
                active: true,
            };
            (0..200_u64)
                .map(|k| {
                    let t = base + ms(7 * k) + 999;
                    (c.shows(t), c.next_change(t))
                })
                .collect::<Vec<_>>()
        };
        let near = blinks(1_000_000_000);
        let far = blinks(262_144 * 1_000_000_000);
        for (k, (a, b)) in near.iter().zip(&far).enumerate() {
            let close = match (a.1, b.1) {
                (Some(x), Some(y)) => (x - y).abs() <= 1e-4,
                (x, y) => x == y,
            };
            assert!(a.0 == b.0 && close, "frame {k}: {a:?} at 1 s, {b:?} at 262,144 s");
        }
    }

    #[test]
    fn a_focused_caret_stops_blinking_once_idle_and_blinks_again_when_typed_at() {
        use crate::div;
        use crate::prelude::*;
        let field = crate::TextField::new("");
        let mut s = crate::app::Scenario::measuring(
            move || {
                div()
                    .padding(10.0)
                    .child(crate::text_input(&field).width(120.0))
                    .into_element()
            },
            200,
            60,
            1.0,
        )
        .expect("a scenario");
        // Its frames recorded cold blink as the live ones do.
        s.check_cold(true);
        s.settle().unwrap();
        s.click(40.0, 20.0);
        s.settle().unwrap();
        let blinks = || clock::next_deadline().is_some();
        assert!(blinks(), "focused: the caret blinks");
        s.at(5.0);
        s.settle().unwrap();
        assert!(blinks(), "still blinking 5 s on");
        s.at(30.0);
        s.settle().unwrap();
        assert!(!blinks(), "idle past the timeout: shown, and nothing wakes for it");
        s.type_text("a");
        s.settle().unwrap();
        assert!(blinks(), "typed at: it blinks again");
        s.at(31.0);
        s.settle().unwrap();
        s.set_window_active(false);
        s.settle().unwrap();
        assert!(!blinks(), "the window lost focus: no caret, nothing wakes");
        s.set_window_active(true);
        s.settle().unwrap();
        assert!(blinks(), "back: it blinks again");
        s.at(60.0);
        s.settle().unwrap();
        assert!(!blinks(), "idle again");
        // Away and back between two frames: it blinks again all the same.
        s.set_window_active(false);
        s.set_window_active(true);
        s.settle().unwrap();
        assert!(blinks(), "refocused between two frames: it blinks again");
    }

    #[test]
    fn the_window_background_is_the_roots_when_it_fills_the_window() {
        // A dark page: what a resize shows past the last frame is dark too.
        use crate::div;
        use crate::prelude::*;
        let dark = Color::hex(0x09090b);
        // The root takes the window; a dark box inside a root of no colour
        // is not the window's.
        let root = |own: bool| {
            move || {
                if own {
                    div().size(pct(100.0)).background(dark).into_element()
                } else {
                    div()
                        .child(div().width(50.0).height(20.0).background(dark))
                        .into_element()
                }
            }
        };
        for (own, want) in [(true, dark), (false, Color::WHITE)] {
            let mut s = crate::app::Scenario::measuring(root(own), 200, 100, 2.0).expect("a scenario");
            s.check_cold(true);
            s.settle().unwrap();
            s.frame("shown").expect("a frame");
            assert_eq!(s.parts().0.background, Some(want), "the root's own colour: {own}");
        }
    }

    fn display(room: usize) -> Arc<Display> {
        Arc::new(Display::new(Vec::with_capacity(room)))
    }

    /// A display that held as many items as its room.
    fn filled(room: usize) -> Arc<Display> {
        let item = Item::RoundedRect {
            bounds: Bounds::default(),
            radius: Corners::all(0.0),
            color: Color::BLACK,
        };
        Arc::new(Display::new(vec![item; room]))
    }

    /// Rooms past [`LARGE_BYTES`].
    fn large_room() -> usize {
        LARGE_BYTES / std::mem::size_of::<Item>() + 1
    }

    fn rooms(pool: &mut Pool, wants: &[usize]) -> Vec<Option<usize>> {
        wants
            .iter()
            .map(|&w| pool.take(w).map(|d| d.items.capacity()))
            .collect()
    }

    #[test]
    fn a_display_is_taken_from_the_class_whose_least_room_holds_what_is_wanted() {
        let mut pool = Pool::default();
        for want in [3, 9, 1000, 0] {
            pool.keep(display(room_for(want)), 0);
        }
        assert_eq!(pool.count, 4);
        // A want of 2 takes from class 2 (rooms 2 to 3), which holds none:
        // the display of room 4 is class 3's, for a want of 3 to 4.
        assert_eq!(
            rooms(&mut pool, &[2, 4, 16, 600, 0, 1]),
            [None, Some(4), Some(16), Some(1024), Some(0), None]
        );
        assert_eq!((pool.count, pool.bytes), (0, 0));
    }

    #[test]
    fn the_pool_keeps_room_for_its_working_set_and_lets_it_go_idle() {
        // Records that each take a thousand displays: the thousand freed
        // are kept, past READY; idle, the caps fall back and what waited
        // goes.
        let mut pool = Pool::default();
        for _ in 0..3 {
            for _ in 0..1000 {
                pool.take(4);
            }
            for _ in 0..1000 {
                pool.keep(display(4), 0);
            }
            pool.age();
        }
        assert!(pool.count >= 1000, "{} kept", pool.count);
        for _ in 0..3 * AGE {
            pool.age();
        }
        assert_eq!(pool.caps(), (READY, READY_BYTES));
        assert_eq!(pool.count, 0);
    }

    #[test]
    fn each_class_lets_go_of_what_waited_past_its_age() {
        let mut pool = Pool::default();
        pool.keep(display(8), 0);
        pool.age();
        pool.keep(display(8), 0);
        pool.keep(display(large_room()), 0);
        for _ in 1..AGE {
            pool.age();
        }
        assert_eq!((pool.count, pool.large.len()), (2, 1));
        pool.age();
        assert_eq!(
            (pool.count, pool.large.len()),
            (1, 1),
            "the older of class 4 goes first"
        );
        pool.age();
        assert_eq!((pool.count, pool.large.len(), pool.held()), (0, 0, 0));
    }

    #[test]
    fn a_full_pool_lets_the_oldest_of_its_largest_class_give_way() {
        let mut pool = Pool::default();
        let big = room_for(large_room() / 4);
        for _ in 0..64 {
            pool.keep(display(big), 0);
        }
        let fit = pool.count;
        assert!(fit < 64 && pool.bytes <= READY_BYTES, "{fit} kept");
        // Beside displays waiting for the renderer that fill the rest.
        let waiting = READY_BYTES - pool.bytes - item_bytes(4) + 1;
        pool.keep(display(4), waiting);
        assert_eq!(pool.count, fit, "one big gave way to the small one");
        assert_eq!(rooms(&mut pool, &[3]), [Some(4)]);
        // One no smaller than any kept is let go instead.
        pool.keep(display(big), READY_BYTES);
        assert_eq!(pool.count, fit - 1);
    }

    /// The large displays the renderer still draws stay out of the small
    /// displays' bytes, and whether one let go of is kept does not hang on
    /// where it sits among those retired.
    #[test]
    fn large_displays_are_kept_apart_from_the_small_ones() {
        for held_first in [true, false] {
            let mut painter = Painter::default();
            let drawn = filled(large_room() * 8);
            let small = display(16);
            let freed_large = filled(large_room());
            assert!(item_bytes(drawn.items.capacity()) > READY_BYTES);
            let mut retired = vec![Arc::clone(&drawn), small, freed_large];
            if !held_first {
                retired.reverse();
            }
            painter.retired = retired;
            painter.reclaim();
            assert_eq!(painter.retired.len(), 1, "the drawn one waits");
            assert_eq!((painter.ready.count, painter.ready.large.len()), (1, 1));
            assert_eq!(painter.ready.bytes, item_bytes(16));
            let held = painter.held().1;
            let want = item_bytes(16) + item_bytes(large_room()) + item_bytes(drawn.items.capacity());
            assert!(held >= u64::try_from(want).unwrap_or(u64::MAX), "held {held}");
        }
    }

    #[test]
    fn the_large_slots_keep_the_latest_large_displays() {
        let mut pool = Pool::default();
        for k in 1..=3 {
            pool.keep(display(large_room() * k), 0);
            pool.age();
        }
        assert_eq!(pool.large.len(), LARGE);
        assert_eq!(
            rooms(&mut pool, &[large_room()]),
            [Some(large_room() * 2)],
            "the oldest gave way"
        );
        assert_eq!(pool.held(), item_bytes(large_room() * 3));
    }

    /// The scratch a chunk of unknown size records into counts as held,
    /// and a large room it no longer uses is let go after [`AGE`] records.
    #[test]
    fn a_large_scratch_left_unused_is_cut() {
        let mut painter = Painter {
            recording: Vec::with_capacity(large_room()),
            ..Painter::default()
        };
        let before = painter.held().1;
        assert!(before >= u64::try_from(item_bytes(large_room())).unwrap_or(u64::MAX));
        for _ in 1..AGE {
            painter.recording_peak = 1;
            painter.reclaim();
        }
        assert_eq!(painter.recording.capacity(), large_room());
        // Used to the full once: it counts again from there.
        painter.recording_peak = large_room();
        painter.reclaim();
        for _ in 0..AGE {
            painter.reclaim();
        }
        assert!(painter.recording.capacity() < large_room());
        assert!(painter.held().1 < before);
    }

    /// Two scenes whose chunk has the same items, the same leaf clip and
    /// the same transform, but whose leaf clip lies within a rounded parent
    /// with other corners (an incremental scene that kept the wrong
    /// parent), or a parent elsewhere, paint differently: the cold check
    /// compares the whole clip chain a chunk lies in, each clip's own
    /// transform chain with it.
    #[test]
    fn the_cold_check_sees_a_clip_ancestor_that_differs() {
        let rounded = |r: f32| Clip {
            bounds: Bounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            radius: Corners::all(r),
            curve: crate::shape::ROUND,
        };
        let leaf = Clip {
            bounds: Bounds {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 80.0,
            },
            ..Clip::default()
        };
        let mut a = Scene::new((100, 100), 1.0, Some(Color::WHITE));
        let moved = a.add_transform((0.0, 0.0), None);
        let outer = a.add_clip(rounded(12.0), moved);
        let inner = a.add_clip(leaf, 0);
        if let Some(c) = usize::try_from(inner).ok().and_then(|i| a.properties.clips.get_mut(i)) {
            c.parent = outer;
        }
        a.push_under(
            State {
                transform: 0,
                clip: inner,
                effect: 0,
            },
            vec![Item::Rect {
                bounds: Bounds {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                },
                color: Color::BLACK,
            }],
        );
        let frame = Frame::default();
        assert_eq!(scene_differs(&frame, &a, &a.clone()), None, "the same scene");
        let outer_at = usize::try_from(outer).unwrap_or(0);
        let mut corners = a.clone();
        corners.properties.clips[outer_at].clip.radius = Corners::all(4.0);
        assert!(
            scene_differs(&frame, &a, &corners).is_some(),
            "a parent clip's corners differ"
        );
        let mut placed = a.clone();
        placed.properties.transforms[usize::try_from(moved).unwrap_or(0)].offset = (0.0, 6.0);
        assert!(
            scene_differs(&frame, &a, &placed).is_some(),
            "a parent clip's transform differs"
        );
    }

    fn square(at: f32, size: f32) -> Bounds {
        Bounds {
            x: 0.0,
            y: at,
            width: size,
            height: size,
        }
    }

    fn black(bounds: Bounds) -> Vec<Item> {
        vec![Item::Rect {
            bounds,
            color: Color::BLACK,
        }]
    }

    /// Two chunks under one 50% group composite as one layer (where they
    /// overlap, 50%); under two equal 50% groups, each its own (75%). The
    /// cold check sees which chunks share a group, and an effect's output
    /// clip (where its layer is cut once composited).
    #[test]
    fn the_cold_check_sees_an_effects_group_and_its_output_clip() {
        let half = EffectNode {
            opacity: 0.5,
            ..EffectNode::default()
        };
        let under = |effect: u32| State {
            transform: 0,
            clip: 0,
            effect,
        };
        let mut one = Scene::new((100, 100), 1.0, Some(Color::WHITE));
        let g = one.add_effect(half.clone());
        one.push_under(under(g), black(square(0.0, 60.0)));
        one.push_under(under(g), black(square(30.0, 60.0)));
        let mut two = one.clone();
        let g2 = two.add_effect(half.clone());
        if let Some(c) = two.chunks.last_mut() {
            c.state.effect = g2;
        }
        let frame = Frame::default();
        assert_eq!(scene_differs(&frame, &one, &one.clone()), None, "the same scene");
        assert!(scene_differs(&frame, &one, &two).is_some(), "one group is not two");
        let mut cut = Scene::new((100, 100), 1.0, Some(Color::WHITE));
        let c = cut.add_clip(
            Clip {
                bounds: square(0.0, 50.0),
                ..Clip::default()
            },
            0,
        );
        let g = cut.add_effect(EffectNode { clip: c, ..half });
        cut.push_under(under(g), black(square(0.0, 60.0)));
        let mut wider = cut.clone();
        wider.properties.clips[usize::try_from(c).unwrap_or(0)].clip.bounds = square(0.0, 80.0);
        assert!(
            scene_differs(&frame, &cut, &wider).is_some(),
            "an output clip that differs"
        );
    }

    /// A chunk inside two clips that do not meet ([0, 10] within [20, 30])
    /// paints nothing, though its item spans both: a scene without it is
    /// the same scene.
    #[test]
    fn the_cold_check_shows_nothing_inside_clips_that_do_not_meet() {
        let mut a = Scene::new((100, 100), 1.0, Some(Color::WHITE));
        let outer = a.add_clip(
            Clip {
                bounds: square(20.0, 10.0),
                ..Clip::default()
            },
            0,
        );
        let inner = a.add_clip(
            Clip {
                bounds: square(0.0, 10.0),
                ..Clip::default()
            },
            0,
        );
        if let Some(n) = usize::try_from(inner).ok().and_then(|i| a.properties.clips.get_mut(i)) {
            n.parent = outer;
        }
        let without = a.clone();
        a.push_under(
            State {
                transform: 0,
                clip: inner,
                effect: 0,
            },
            black(Bounds {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 30.0,
            }),
        );
        assert_eq!(
            scene_differs(&Frame::default(), &a, &without),
            None,
            "nothing of it shows"
        );
    }

    /// A clip scrolled with its content (its bounds in content space, far
    /// below its parent's) still shows in the window: the cold check moves
    /// each clip of the chain into window pixels before it culls by them,
    /// so what paints there is compared.
    #[test]
    fn the_cold_check_culls_by_each_clip_where_it_shows() {
        let mut a = Scene::new((100, 100), 1.0, Some(Color::WHITE));
        let viewport = a.add_clip(
            Clip {
                bounds: square(0.0, 100.0),
                ..Clip::default()
            },
            0,
        );
        let scrolled = a.add_transform((0.0, -200.0), None);
        let inner = a.add_clip(
            Clip {
                bounds: square(200.0, 80.0),
                ..Clip::default()
            },
            scrolled,
        );
        if let Some(n) = usize::try_from(inner).ok().and_then(|i| a.properties.clips.get_mut(i)) {
            n.parent = viewport;
        }
        a.push_under(
            State {
                transform: scrolled,
                clip: inner,
                effect: 0,
            },
            black(square(200.0, 80.0)),
        );
        let frame = Frame::default();
        assert_eq!(scene_differs(&frame, &a, &a.clone()), None, "the same scene");
        // Scrolled a pixel further, the same rows show elsewhere.
        let mut b = a.clone();
        b.properties.transforms[usize::try_from(scrolled).unwrap_or(0)].offset = (0.0, -201.0);
        assert!(
            scene_differs(&frame, &a, &b).is_some(),
            "a scrolled clip's content is compared"
        );
    }
}
