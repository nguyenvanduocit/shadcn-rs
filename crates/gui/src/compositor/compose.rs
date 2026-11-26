//! Composing a frame (ADR-0015, ADR-0016): the committed property trees
//! with what the compositor moves written over them, and what of them that
//! changed since the renderer last drew ([`Changed`]).
//!
//! A frame writes only the entries its scroll and motion reach: the
//! subtrees of the transforms it displaces or turns now or did last frame
//! (to put them back), the clips and effects under them, the effects whose
//! opacity it moves, and the entries the commits since the last frame
//! wrote. A frame where nothing scrolls, moves or was committed writes
//! nothing and names nothing changed. [`whole`] composes every entry, as a
//! first frame does and as the cold check compares with.

use super::{Compositor, NodeKey, ShownScroll, moving_at};
use crate::scene::{GroupTransform, Marks, Properties, Scene, TransformNode};

/// The trees' links a frame follows to reach a subtree: each transform's
/// children, the clips and the effects placed under each transform, each
/// effect's children. Made again when the trees' shape changes.
#[derive(Default)]
pub(super) struct Links {
    children: Vec<Vec<u32>>,
    clips: Vec<Vec<u32>>,
    effects: Vec<Vec<u32>>,
    effect_children: Vec<Vec<u32>>,
}

impl Links {
    pub(super) fn of(props: &Properties) -> Self {
        let n = props.transforms.len();
        let mut l = Self {
            children: vec![Vec::new(); n],
            clips: vec![Vec::new(); n],
            effects: vec![Vec::new(); n],
            effect_children: vec![Vec::new(); props.effects.len()],
        };
        let at = |v: &mut Vec<Vec<u32>>, i: u32, id: u32| {
            if let Some(list) = usize::try_from(i).ok().and_then(|i| v.get_mut(i)) {
                list.push(id);
            }
        };
        for (id, t) in (0_u32..).zip(&props.transforms).skip(1) {
            if t.parent != id {
                at(&mut l.children, t.parent, id);
            }
        }
        for (id, c) in (0_u32..).zip(&props.clips).skip(1) {
            at(&mut l.clips, c.transform, id);
        }
        for (id, e) in (0_u32..).zip(&props.effects).skip(1) {
            at(&mut l.effects, e.transform, id);
            if e.parent != id {
                at(&mut l.effect_children, e.parent, id);
            }
        }
        l
    }
}

fn get(v: &[Vec<u32>], i: u32) -> &[u32] {
    usize::try_from(i)
        .ok()
        .and_then(|i| v.get(i))
        .map_or(&[], Vec::as_slice)
}

/// What the compositor moves at a frame's time: each transform's own
/// displacement and matrix, the thumbs, each moving effect's opacity.
#[derive(Default)]
pub(super) struct Sources {
    /// A transform's own displacement (a moving node's, a scroll content's).
    own: foldhash::HashMap<u32, (f32, f32)>,
    /// A moving node's matrix, replacing its transform's.
    set: foldhash::HashMap<u32, Option<GroupTransform>>,
    /// A thumb's transform and its offset at the top plus how far it is
    /// along, by its transform: looked up once each (Opus r11 I1).
    thumbs: foldhash::HashMap<u32, (f32, f32)>,
    /// A moving effect's opacity (the last moving node naming it wins).
    opacity: foldhash::HashMap<u32, f32>,
    pub(super) moved: Vec<(NodeKey, (f32, f32))>,
    pub(super) turned: Vec<(NodeKey, crate::style::Painted)>,
}

impl Sources {
    /// Empty, its memory kept.
    fn clear(&mut self) {
        self.own.clear();
        self.set.clear();
        self.thumbs.clear();
        self.opacity.clear();
        self.moved.clear();
        self.turned.clear();
    }
}

/// Each scroll container as shown, kept from frame to frame (ADR-0032 I7,
/// Opus r11 I1): a frame visits only the containers the compositor holds
/// apart from what was committed (owed a displacement, their scrollbar
/// shown), those the last frame visited (one come to rest is shown as
/// committed again) and those shown otherwise than committed whatever
/// they hold. A commit's table adds the entries that differ from the last
/// (a comparison, not a compose, each); every container only when the
/// containers themselves changed (another index).
/// Any other shows as committed, its thumb where it was painted, so a
/// frame at rest costs nothing whatever the count of containers.
#[derive(Default)]
pub(super) struct ScrollShown {
    /// The table it was made for, and that table's index: a table with
    /// the same index holds the same containers in the same order.
    table: Option<std::sync::Arc<[super::ScrollNode]>>,
    index: Option<std::sync::Arc<super::TableIndex>>,
    /// This frame's: the containers its commit changed, ascending.
    differ: Vec<u32>,
    /// Each container as shown, in the table's order.
    pub(super) list: Vec<ShownScroll>,
    /// Bumped each time `list` changed: a frame's buffer written at
    /// another is written again ([`super::ShownScrolls::write`]).
    pub(super) version: u64,
    /// Containers whose content is displaced from what paint applied
    /// whatever the compositor holds (its snapping): visited every frame.
    always: Vec<u32>,
    /// The containers the last frame visited.
    last: Vec<u32>,
    /// This frame's: what it visits, and each once.
    visit: Vec<u32>,
    once: foldhash::HashSet<u32>,
}

impl Compositor {
    /// What it moves at `at`, written into the sources the last frame
    /// left (their memory kept).
    pub(super) fn sources(&mut self, at: u64) -> Sources {
        let mut src = std::mem::take(&mut self.spare);
        src.clear();
        let scene = &self.scene;
        let props = &scene.properties;
        let n = props.transforms.len();
        for m in scene.compositing.moving.iter() {
            let (entry, opacity, painted) = moving_at(m, at, scene.scale);
            if let Some(t) = m.transform
                && usize::try_from(t).is_ok_and(|i| i < n)
            {
                src.turned.push((m.key, painted));
                let was = props.transform(t);
                let d = (entry.offset.0 - was.offset.0, entry.offset.1 - was.offset.1);
                src.own.insert(t, d);
                src.set.insert(t, entry.matrix);
                if d != (0.0, 0.0) {
                    src.moved.push((m.key, d));
                }
            }
            if let Some(e) = m.effect
                && usize::try_from(e).is_ok_and(|i| i < props.effects.len())
            {
                src.opacity.insert(e, opacity);
            }
        }
        let ss = &mut self.scroll_shown;
        let table = &scene.compositing.scrollers;
        let another = ss.table.as_ref().is_none_or(|t| !std::sync::Arc::ptr_eq(t, table));
        // The same containers in the same order (the producer kept their
        // index): only the entries that differ are news, found by
        // comparing them, not composing each.
        let same = another
            && ss
                .index
                .as_ref()
                .is_some_and(|x| std::sync::Arc::ptr_eq(x, &scene.compositing.index))
            && ss.table.as_ref().is_some_and(|t| t.len() == table.len())
            && ss.list.len() == table.len();
        let fresh = another && !same;
        let mut visit = std::mem::take(&mut ss.visit);
        visit.clear();
        ss.once.clear();
        ss.differ.clear();
        if same && let Some(was) = ss.table.as_ref() {
            for (i, (a, b)) in (0_u32..).zip(was.iter().zip(table.iter())) {
                if a != b {
                    ss.differ.push(i);
                    ss.once.insert(i);
                    visit.push(i);
                }
            }
            let differ = &ss.differ;
            ss.always.retain(|i| differ.binary_search(i).is_err());
        }
        // The containers the compositor holds apart from committed now:
        // visited now, and again next frame (to show one come to rest as
        // committed).
        let mut live = std::mem::take(&mut ss.last);
        let last_len = live.len();
        live.extend(
            self.scrolls
                .live()
                .filter_map(|id| scene.compositing.scroller(id))
                .filter_map(|(i, _)| u32::try_from(i).ok()),
        );
        if fresh {
            ss.list.clear();
            ss.always.clear();
            visit.extend((0..table.len()).filter_map(|i| u32::try_from(i).ok()));
        } else {
            for &i in live.iter().chain(&ss.always) {
                if ss.once.insert(i) {
                    visit.push(i);
                }
            }
        }
        live.drain(..last_len);
        let mut changed = fresh;
        for &i in &visit {
            let Some(s) = usize::try_from(i).ok().and_then(|i| table.get(i)) else {
                continue;
            };
            super::scroll::visit();
            let offset = self.scrolls.offset(s, i);
            let snapped = crate::geometry::device_snapped(offset, scene.scale);
            let applied = (if s.rtl { snapped.0 } else { -snapped.0 }, -snapped.1);
            let d = (applied.0 - s.applied.0, applied.1 - s.applied.1);
            if d != (0.0, 0.0) && usize::try_from(s.content).is_ok_and(|c| c < n) {
                let o = src.own.entry(s.content).or_default();
                o.0 += d.0;
                o.1 += d.1;
            }
            let entry = ShownScroll {
                key: s.key,
                owner: s.owner,
                offset,
                applied,
            };
            if fresh {
                ss.list.push(entry);
            } else if let Some(e) = usize::try_from(i).ok().and_then(|i| ss.list.get_mut(i))
                && *e != entry
            {
                *e = entry;
                changed = true;
            }
            // A container committed anew: whether it shows displaced at
            // its committed offset itself (its snapping), so every frame.
            if fresh || ss.differ.binary_search(&i).is_ok() {
                let at_rest = crate::geometry::device_snapped(
                    (
                        s.offset.0.clamp(0.0, s.max.0.max(0.0)),
                        s.offset.1.clamp(0.0, s.max.1.max(0.0)),
                    ),
                    scene.scale,
                );
                let at_rest = (if s.rtl { at_rest.0 } else { -at_rest.0 }, -at_rest.1);
                if at_rest != s.applied {
                    ss.always.push(i);
                }
            }
            if let Some(t) = s.thumb
                && s.max.1 > 0.0
            {
                let along = t.travel * (offset.1 / s.max.1);
                src.thumbs.insert(t.transform, (t.top.0, t.top.1 + along));
                // Shown only while it is: at rest it stays as committed (hidden).
                let shown = self.scrolls.thumb_opacity(s, at);
                if shown > 0.0 && usize::try_from(t.effect).is_ok_and(|e| e < props.effects.len()) {
                    src.opacity.insert(t.effect, shown);
                }
            }
        }
        if changed {
            ss.version = ss.version.wrapping_add(1).max(1);
        }
        ss.last = live;
        ss.visit = visit;
        ss.table = Some(std::sync::Arc::clone(table));
        ss.index = Some(std::sync::Arc::clone(&scene.compositing.index));
        src
    }
}

/// Each transform's displacement, its own and its ancestors' (a
/// transform's offset is the whole way to the window).
pub(super) fn totals(props: &Properties, own: &foldhash::HashMap<u32, (f32, f32)>) -> Vec<(f32, f32)> {
    let n = props.transforms.len();
    let own_of = |i: usize| {
        u32::try_from(i)
            .ok()
            .and_then(|i| own.get(&i))
            .copied()
            .unwrap_or_default()
    };
    let mut total: Vec<Option<(f32, f32)>> = vec![None; n];
    if let Some(t) = total.first_mut() {
        *t = Some(own_of(0));
    }
    let mut chain = Vec::new();
    for start in 0..n {
        let mut at = start;
        chain.clear();
        // Up to the first with its total known (ids may point either way).
        while total.get(at).is_some_and(Option::is_none) && chain.len() <= n {
            chain.push(at);
            at = usize::try_from(props.transforms.get(at).map_or(0, |t| t.parent)).unwrap_or(0);
        }
        let mut acc = total.get(at).copied().flatten().unwrap_or_default();
        for &i in chain.iter().rev() {
            let o = own_of(i);
            acc = (acc.0 + o.0, acc.1 + o.1);
            if let Some(t) = total.get_mut(i) {
                *t = Some(acc);
            }
        }
    }
    total.into_iter().map(Option::unwrap_or_default).collect()
}

/// Transform `t` of `committed` moved by `d` (its total), its parent's
/// total `up`, as `src` moves it.
fn transform_at(
    committed: &TransformNode,
    t: u32,
    d: (f32, f32),
    up: (f32, f32),
    k: f32,
    src: &Sources,
) -> TransformNode {
    let mut out = *committed;
    out.offset = (out.offset.0 + d.0, out.offset.1 + d.1);
    match src.set.get(&t).copied() {
        // A moving node's matrix, placed from its own new offset: what is above moves it on.
        Some(matrix) => {
            out.matrix = matrix.map(|m| GroupTransform {
                origin: (m.origin.0 + up.0 * k, m.origin.1 + up.1 * k),
                ..m
            });
        }
        None => {
            if let Some(m) = &mut out.matrix {
                m.origin = (m.origin.0 + d.0 * k, m.origin.1 + d.1 * k);
            }
        }
    }
    out
}

fn displaced_clip(c: &mut crate::scene::ClipNode, d: (f32, f32)) {
    c.clip.bounds = c.clip.bounds.translated(d.0, d.1);
}

fn displaced_effect(e: &mut crate::scene::EffectNode, d: (f32, f32), k: f32) {
    if d == (0.0, 0.0) {
        return;
    }
    if let Some(r) = &mut e.region {
        r.x = r.x.saturating_add(crate::convert::f32_to_i32((d.0 * k).round()));
        r.y = r.y.saturating_add(crate::convert::f32_to_i32((d.1 * k).round()));
    }
    if let Some(crate::scene::Gradient::Linear { start, end, .. }) = &mut e.mask {
        *start = (start.0 + d.0, start.1 + d.1);
        *end = (end.0 + d.0, end.1 + d.1);
    }
}

/// `scene`'s property trees with every entry `src` moves written over them.
pub(super) fn whole(scene: &Scene, src: &Sources) -> Properties {
    let mut props = scene.properties.clone();
    let k = scene.scale;
    for (&e, &o) in &src.opacity {
        if let Some(effect) = usize::try_from(e).ok().and_then(|i| props.effects.get_mut(i)) {
            effect.opacity = o;
        }
    }
    let total = totals(&props, &src.own);
    let at = |i: u32| {
        usize::try_from(i)
            .ok()
            .and_then(|i| total.get(i))
            .copied()
            .unwrap_or_default()
    };
    for (i, t) in (0_u32..).zip(props.transforms.iter_mut()) {
        *t = transform_at(t, i, at(i), at(t.parent), k, src);
    }
    for (&id, &top) in &src.thumbs {
        if let Some(t) = usize::try_from(id).ok().and_then(|i| props.transforms.get_mut(i)) {
            let up = at(t.parent);
            t.offset = (top.0 + up.0, top.1 + up.1);
        }
    }
    for c in props.clips.iter_mut().skip(1) {
        displaced_clip(c, at(c.transform));
    }
    for e in props.effects.iter_mut().skip(1) {
        displaced_effect(e, at(e.transform), k);
    }
    props
}

/// What the last frame moved, so the next puts back what it no longer does.
#[derive(Default)]
pub(super) struct Moved {
    transforms: Vec<u32>,
    effects: Vec<u32>,
}

/// What one compose wrote of the property trees, by id ([`catch_up`]).
#[derive(Default)]
pub(super) struct PropDelta {
    /// The frame it composed.
    frame: u64,
    /// It made them whole (a structural change): every entry.
    all: bool,
    transforms: Vec<u32>,
    clips: Vec<u32>,
    effects: Vec<u32>,
}

/// Frames composed and not held that wait to be used again: the one the
/// timeline let go last, the last composed and not presented, one more.
/// The frames the timeline keeps come back here when it lets them go.
pub(super) const POOL: usize = 3;

/// The most records of what a compose wrote that are kept: enough to
/// catch up any frame the timeline keeps ([`super::KEPT`]) or the pool
/// holds, and the captures among them.
const DELTAS: usize = super::KEPT + POOL + 2;

/// Drop the oldest waiting frames past [`POOL`].
pub(super) fn trim_pool(pool: &mut std::collections::VecDeque<std::sync::Arc<super::Presented>>) {
    while pool.len() > POOL {
        pool.pop_front();
    }
}

/// Bring `props`, the trees as frame `from` had them, up to `now` (the
/// trees as composed this frame) by what the composes since wrote, as cc
/// moves only the nodes its change tracking lists: the entries they wrote,
/// not the whole trees, each once however many composes wrote it. Through
/// a compose not kept, or one that made them whole, every entry is copied,
/// in the buffers it has. How many entries it copied.
fn catch_up(
    props: &mut crate::scene::Properties,
    from: u64,
    deltas: &std::collections::VecDeque<PropDelta>,
    now: &crate::scene::Properties,
    ids: &mut Vec<u32>,
) -> usize {
    let since = || deltas.iter().take_while(|d| d.frame > from);
    let kept =
        from != 0 && since().last().is_some_and(|d| d.frame == from.saturating_add(1)) && since().all(|d| !d.all);
    // More ids written since than the trees have entries: copying them
    // whole is cheaper than gathering the ids, and bounds the ids' buffer
    // by the trees, not by how much the composes since wrote.
    let written = since()
        .map(|d| {
            d.transforms
                .len()
                .saturating_add(d.clips.len())
                .saturating_add(d.effects.len())
        })
        .fold(0_usize, usize::saturating_add);
    if !kept || written > entries(now) {
        props.transforms.clone_from(&now.transforms);
        props.clips.clone_from(&now.clips);
        props.effects.clone_from(&now.effects);
        return entries(now);
    }
    fn take<T: Clone>(to: &mut [T], from: &[T], ids: &mut Vec<u32>) -> usize {
        ids.sort_unstable();
        ids.dedup();
        let mut n = 0_usize;
        for i in ids.iter().filter_map(|&i| usize::try_from(i).ok()) {
            if let (Some(to), Some(v)) = (to.get_mut(i), from.get(i)) {
                to.clone_from(v);
                n = n.saturating_add(1);
            }
        }
        n
    }
    let mut copied = 0_usize;
    ids.clear();
    ids.extend(since().flat_map(|d| d.transforms.iter().copied()));
    copied = copied.saturating_add(take(&mut props.transforms, &now.transforms, ids));
    ids.clear();
    ids.extend(since().flat_map(|d| d.clips.iter().copied()));
    copied = copied.saturating_add(take(&mut props.clips, &now.clips, ids));
    ids.clear();
    ids.extend(since().flat_map(|d| d.effects.iter().copied()));
    copied.saturating_add(take(&mut props.effects, &now.effects, ids))
}

/// How many entries the trees have.
fn entries(p: &crate::scene::Properties) -> usize {
    p.transforms
        .len()
        .saturating_add(p.clips.len())
        .saturating_add(p.effects.len())
}

/// The working sets of one incremental frame, kept for their memory.
#[derive(Default)]
pub(super) struct Work {
    reached: Marks,
    clips: Marks,
    effects: Marks,
    changed_t: Marks,
    changed_e: Marks,
    list: Vec<u32>,
    stack: Vec<u32>,
    totals: foldhash::HashMap<u32, (f32, f32)>,
    /// A transform's ancestors whose totals are not known yet.
    chain: Vec<u32>,
    /// The clips and effects to compose again.
    clip_list: Vec<u32>,
    effect_list: Vec<u32>,
}

impl Compositor {
    /// Frame `frame` is a snapshot made without composing (a capture's):
    /// it wrote nothing, so a buffer catches up through it for free.
    pub(super) fn wrote_nothing(&mut self, frame: u64) {
        self.deltas.push_front(PropDelta {
            frame,
            ..PropDelta::default()
        });
    }

    /// Compose the frame at `at` into `shown`, writing what the scroll,
    /// motion and commits since the last frame reach, and noting in
    /// `changed` what of it differs from what was shown.
    pub(super) fn compose_into_shown(&mut self, at: u64) {
        let (frame, commit) = (self.frame.saturating_add(1), self.commit);
        // What this compose writes of the property trees, by id: what a
        // frame's own copy of them takes in to become this one's.
        // Kept back to the oldest frame that may be used again: one the
        // pool holds that no one else does, or the oldest the timeline
        // keeps, which comes to the pool when let go. One the UI still
        // holds (it routes in it) pins nothing: if it comes free, it is
        // copied whole ([`catch_up`]).
        let oldest = self
            .pool
            .iter()
            .filter(|p| std::sync::Arc::strong_count(p) == 1)
            .map(|p| p.frame)
            .chain(self.timeline.oldest())
            .min()
            .unwrap_or(frame);
        // At most [`DELTAS`] whatever is held (presents that keep failing
        // hold the timeline's oldest): a frame older than them is copied
        // whole when it comes back ([`catch_up`]).
        let spent = |d: &PropDelta, n: usize| d.frame <= oldest || n >= DELTAS;
        let mut delta = if self.deltas.back().is_some_and(|d| spent(d, self.deltas.len())) {
            self.deltas.pop_back().unwrap_or_default()
        } else {
            PropDelta::default()
        };
        while self.deltas.back().is_some_and(|d| spent(d, self.deltas.len())) {
            self.deltas.pop_back();
        }
        delta.frame = frame;
        delta.all = false;
        delta.transforms.clear();
        delta.clips.clear();
        delta.effects.clear();
        let mut src = self.sources(at);
        let (a, b) = (&self.shown.properties, &self.scene.properties);
        let reshaped = a.transforms.len() != b.transforms.len()
            || a.clips.len() != b.clips.len()
            || a.effects.len() != b.effects.len()
            || self.shown.chunks.len() != self.scene.chunks.len();
        if std::mem::take(&mut self.pending.all) || reshaped {
            self.pending.clear();
            self.links = Links::of(&self.scene.properties);
            self.shown.properties = whole(&self.scene, &src);
            self.shown.chunks.clone_from(&self.scene.chunks);
            self.changed.all = true;
            delta.all = true;
        } else {
            self.compose_changed(&src, &mut delta);
        }
        self.deltas.push_front(delta);
        let scene = &self.scene;
        self.shown.scale = scene.scale;
        self.shown.size = scene.size;
        self.shown.background = scene.background;
        self.shown.compositing.clone_from(&scene.compositing);
        let moved = &mut self.moved;
        moved.transforms.clear();
        moved
            .transforms
            .extend(src.own.keys().chain(src.set.keys()).chain(src.thumbs.keys()).copied());
        moved.effects.clear();
        moved.effects.extend(src.opacity.keys().copied());
        if self.cold_check {
            let cold = whole(&self.scene, &src);
            assert!(
                cold == self.shown.properties,
                "the compositor's frame {} differs from one composed whole",
                self.frame
            );
        }
        // A frame no one holds any more (not on screen, not kept for input
        // stamped with it, not read by the UI) becomes this one: its lists
        // are the next frame's sources, and its property trees take in
        // only what the composes since it wrote.
        let free = self.pool.iter().position(|p| std::sync::Arc::strong_count(p) == 1);
        let next = if let Some(mut old) = free.and_then(|i| self.pool.remove(i))
            && let Some(p) = std::sync::Arc::get_mut(&mut old)
        {
            let copied = catch_up(
                &mut p.properties,
                p.frame,
                &self.deltas,
                &self.shown.properties,
                &mut self.work.list,
            );
            self.props_copied = self.props_copied.saturating_add(copied);
            if self.cold_check {
                assert!(
                    p.properties == self.shown.properties,
                    "frame {frame}'s property trees, caught up from frame {}, differ from those composed",
                    p.frame
                );
            }
            // Its tables: this frame's, as [`super::Presented::of`] takes
            // them, in the buffers it had.
            p.take_tables(&self.shown);
            p.commit = commit;
            old
        } else {
            self.props_copied = self.props_copied.saturating_add(entries(&self.shown.properties));
            #[cfg_attr(not(test), expect(unused_mut, reason = "a test marks the snapshot"))]
            let mut fresh = super::Presented::of(&self.shown, commit);
            #[cfg(test)]
            {
                fresh.alive.clone_from(&self.alive);
            }
            std::sync::Arc::new(fresh)
        };
        let mut next = next;
        if let Some(p) = std::sync::Arc::get_mut(&mut next) {
            p.frame = frame;
            p.present_at = at;
            p.scroll.write(&self.scroll_shown.list, self.scroll_shown.version);
            std::mem::swap(&mut p.moved, &mut src.moved);
            std::mem::swap(&mut p.turned, &mut src.turned);
            // The renderer's, once it draws this frame ([`super::Compositor::draw`]).
            p.stages = std::sync::Arc::clone(&self.presented.stages);
        }
        let last = std::mem::replace(&mut self.presented, next);
        // One the timeline holds comes back when it lets it go.
        if std::sync::Arc::strong_count(&last) == 1 {
            self.pool.push_back(last);
            trim_pool(&mut self.pool);
        }
        self.spare = src;
    }

    /// Write what `src`, the last frame's motion and the commits since reach.
    #[expect(clippy::too_many_lines, reason = "transforms, clips, effects, chunks, in order")]
    fn compose_changed(&mut self, src: &Sources, delta: &mut PropDelta) {
        let k = self.scene.scale;
        let committed = &self.scene.properties;
        let shown = &mut self.shown.properties;
        let links = &self.links;
        let w = &mut self.work;
        let pending = &mut self.pending;
        let changed = &mut self.changed;
        let nt = committed.transforms.len();
        w.reached.begin(nt);
        w.changed_t.begin(nt);
        w.clips.begin(committed.clips.len());
        w.effects.begin(committed.effects.len());
        w.changed_e.begin(committed.effects.len());
        // The subtrees it moves now or moved last frame.
        w.list.clear();
        w.stack.clear();
        w.stack.extend(src.own.keys().chain(src.set.keys()).copied());
        w.stack.extend(src.thumbs.keys().copied());
        w.stack.extend(self.moved.transforms.iter().copied());
        while let Some(t) = w.stack.pop() {
            if w.reached.mark(t) {
                w.list.push(t);
                w.stack.extend_from_slice(get(&links.children, t));
            }
        }
        // Their totals: only what lies under a displacement has one.
        w.totals.clear();
        let total = |t: u32, w: &mut Work| -> (f32, f32) {
            let mut chain = std::mem::take(&mut w.chain);
            chain.clear();
            let mut at = t;
            let mut acc = (0.0, 0.0);
            while w.reached.has(at) && chain.len() <= nt {
                if let Some(v) = w.totals.get(&at) {
                    acc = *v;
                    break;
                }
                chain.push(at);
                if at == 0 {
                    break;
                }
                at = committed.transform(at).parent;
            }
            for &i in chain.iter().rev() {
                let o = src.own.get(&i).copied().unwrap_or_default();
                acc = (acc.0 + o.0, acc.1 + o.1);
                w.totals.insert(i, acc);
            }
            w.chain = chain;
            acc
        };
        // One lookup each, by its transform (Opus r11 I1: a scan of every
        // thumb for each was quadratic in the containers).
        let thumb = |t: u32| src.thumbs.get(&t).copied();
        let mut todo = std::mem::take(&mut w.list);
        for &t in &pending.transforms {
            if w.reached.mark(t) {
                todo.push(t);
            }
        }
        for &t in &todo {
            let Some(c) = usize::try_from(t).ok().and_then(|i| committed.transforms.get(i)) else {
                continue;
            };
            let d = total(t, w);
            let up = total(c.parent, w);
            let mut next = transform_at(c, t, d, up, k, src);
            if let Some(top) = thumb(t) {
                next.offset = (top.0 + up.0, top.1 + up.1);
            }
            let Some(slot) = usize::try_from(t).ok().and_then(|i| shown.transforms.get_mut(i)) else {
                continue;
            };
            if *slot != next {
                let turned = slot.matrix != next.matrix;
                delta.transforms.push(t);
                *slot = next;
                if w.changed_t.mark(t) {
                    changed.transforms.push(t);
                }
                // What lies under a matrix that changed is drawn otherwise.
                if turned {
                    w.stack.clear();
                    w.stack.extend_from_slice(get(&links.children, t));
                    while let Some(u) = w.stack.pop() {
                        if w.changed_t.mark(u) {
                            changed.transforms.push(u);
                            w.stack.extend_from_slice(get(&links.children, u));
                        }
                    }
                }
            }
        }
        // Clips and effects under what moved, and those committed.
        let mut clips = std::mem::take(&mut w.clip_list);
        let mut effects = std::mem::take(&mut w.effect_list);
        clips.clear();
        effects.clear();
        for &t in &todo {
            for &c in get(&links.clips, t) {
                if w.clips.mark(c) {
                    clips.push(c);
                }
            }
            for &e in get(&links.effects, t) {
                if w.effects.mark(e) {
                    effects.push(e);
                }
            }
        }
        for &c in &pending.clips {
            if w.clips.mark(c) {
                clips.push(c);
            }
        }
        for &e in src.opacity.keys().chain(&self.moved.effects).chain(&pending.effects) {
            if w.effects.mark(e) {
                effects.push(e);
            }
        }
        for &c in &clips {
            if c == 0 {
                continue;
            }
            let Some(base) = usize::try_from(c).ok().and_then(|i| committed.clips.get(i)) else {
                continue;
            };
            let mut next = *base;
            displaced_clip(&mut next, total(base.transform, w));
            if let Some(slot) = usize::try_from(c).ok().and_then(|i| shown.clips.get_mut(i))
                && *slot != next
            {
                delta.clips.push(c);
                *slot = next;
                changed.clips.push(c);
            }
        }
        for &e in &effects {
            let Some(base) = usize::try_from(e).ok().and_then(|i| committed.effects.get(i)) else {
                continue;
            };
            let mut next = base.clone();
            if let Some(o) = src.opacity.get(&e) {
                next.opacity = *o;
            }
            if e != 0 {
                displaced_effect(&mut next, total(base.transform, w), k);
            }
            let Some(slot) = usize::try_from(e).ok().and_then(|i| shown.effects.get_mut(i)) else {
                continue;
            };
            if *slot != next {
                *slot = next;
                delta.effects.push(e);
                // Its opacity is folded into every effect under it.
                w.stack.clear();
                w.stack.push(e);
                while let Some(u) = w.stack.pop() {
                    if w.changed_e.mark(u) {
                        changed.effects.push(u);
                        w.stack.extend_from_slice(get(&links.effect_children, u));
                    }
                }
            }
        }
        todo.clear();
        w.list = todo;
        w.clip_list = clips;
        w.effect_list = effects;
        for &i in &pending.chunks {
            if let (Some(slot), Some(c)) = (self.shown.chunks.get_mut(i), self.scene.chunks.get(i)) {
                slot.clone_from(c);
                changed.chunks.push(i);
            }
        }
        pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{PropDelta, catch_up};

    /// Opus r7 nit 2: forty composes since a frame each wrote the same
    /// eight of ten transforms. Catching up copies the ten whole rather
    /// than gathering 320 ids: the work and the ids' buffer are bounded by
    /// the trees, not by how much the composes wrote.
    #[test]
    fn catching_up_through_more_writes_than_entries_copies_the_trees_whole() {
        // The eight written moved; the last two did not.
        let now = crate::scene::Properties {
            transforms: (0..10_u16)
                .map(|i| crate::scene::TransformNode {
                    offset: (if i < 8 { f32::from(i) + 1.0 } else { 0.0 }, 0.0),
                    ..crate::scene::TransformNode::default()
                })
                .collect(),
            ..crate::scene::Properties::default()
        };
        let mut props = crate::scene::Properties {
            transforms: vec![crate::scene::TransformNode::default(); 10],
            ..crate::scene::Properties::default()
        };
        let deltas: std::collections::VecDeque<PropDelta> = (2..=41_u64)
            .rev()
            .map(|frame| PropDelta {
                frame,
                transforms: (0..8).collect(),
                ..PropDelta::default()
            })
            .collect();
        let mut ids = Vec::new();
        catch_up(&mut props, 1, &deltas, &now, &mut ids);
        assert_eq!(props, now, "caught up");
        assert!(ids.capacity() <= 12, "no ids gathered: {}", ids.capacity());
    }
}
