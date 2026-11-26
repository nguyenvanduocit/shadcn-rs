//! The clip tree as the renderer keeps it (ADR-0016; Chrome's clip-node
//! tree): for each clip of the scene, the entry the shaders read and the
//! facts every cache and damage path reads from one place.
//!
//! **Entries.** A clip's entry is five vec4s: its own rounded rectangle
//! (physical, in its own space), its radii, the map from the canvas into
//! that space (`a, b, c, d`, then `e, f`, its scale and its corners'
//! curve), then the next clip of its *effective* chain, its own depth and
//! its parent. So a chain crosses scales and turns: each clip is tested in
//! its own space, its coverage measured in the pixels of the target drawn
//! into. A pass into a layer walks a chain up to its first clip as
//! shallow as the layer's output clip ([`crate::scene::EffectNode::clip`]):
//! those clip the layer's composite, after its filters and resampling.
//! A layer whose pixels are not the canvas's walks parents ([`own_grid`]).
//!
//! **Effective chains.** A clip's next is the first clip of its parent's
//! walk that does not keep the whole of every pixel it covers
//! ([`crate::clip::holds`], each ancestor's rectangle less its corners,
//! tested in the ancestor's own space, so a turned or rounded ancestor is
//! left out as a square one is). Its walk (it, then its next's walk)
//! keeps what its whole chain keeps, pixel for pixel, and the shaders walk
//! it to its end. A clip whose pixels lie where the whole of its parent's
//! walk keeps everything ([`Node::inner`]) ends its walk at once.
//!
//! **Revisions.** A clip's revision is the hash of its entry, its parent
//! and its next's revision: it changes when its own rectangle, radii or
//! space does, when any clip of its effective chain does, or when its
//! parent does. Chunk keys and layer contents hold it, and the damage of
//! a clip whose revision changed is what its chain kept before and keeps
//! now, each frame's from its own ancestry ([`Update::damage`]).
//!
//! **Updates.** A frame decides the clips that changed, then the children
//! of each clip decided anew, in one pass ordered by depth: a parent is
//! decided before its child, so each clip is decided once, against
//! ancestors already final. A clip's walk reads only its parent's walk,
//! and any change to that walk changes its parent's node, so its children
//! are all a change must reach; they are kept per clip and change only
//! when a clip's parent does. Deciding a clip anew allocates nothing: a
//! node is plain data, and the pass's queue and records are kept across
//! frames. Depths come from one walk of the clips met ([`Ancestry`]), each
//! ancestor stepped over once a frame.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::damage::Rect;
use super::path;
use crate::scene::Properties;

/// Vec4s a clip entry takes (`PCLIP` in `common.wgsl`), the one number: its
/// floats are four each.
pub(super) const ENTRY_VEC4S: u32 = 5;
/// Floats a clip entry takes.
#[expect(clippy::as_conversions, reason = "a count, widened in a const")]
pub(super) const ENTRY: usize = ENTRY_VEC4S as usize * 4;

pub(super) const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// A box (x0, y0, x1, y1) that holds nothing, and lies in none.
const EMPTY: [f32; 4] = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];

/// The scales and turns from transform `id` up to `until`, as one matrix.
pub(super) fn compose(props: &Properties, id: u32, until: u32) -> Option<[f32; 6]> {
    props.matrices(id, until).map(|g| g.matrix()).reduce(path::then)
}

/// The bounding box of `r` (x0, y0, x1, y1) through `m`.
pub(super) fn mapped(r: [f32; 4], m: [f32; 6]) -> [f32; 4] {
    bbox(&[[r[0], r[1]], [r[2], r[1]], [r[0], r[3]], [r[2], r[3]]].map(|p| path::apply(m, p)))
}

pub(super) fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

fn bbox(points: &[[f32; 2]]) -> [f32; 4] {
    points.iter().fold(EMPTY, |b, p| {
        [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
    })
}

/// Whether box `a` lies in box `b` (both x0, y0, x1, y1).
fn within(a: [f32; 4], b: [f32; 4]) -> bool {
    b[0] <= a[0] && b[1] <= a[1] && a[2] <= b[2] && a[3] <= b[3]
}

/// The bounding box of where convex quads `a` and `b` (corners in order
/// around) overlap: `a` cut by each edge of `b` in turn, so a turned clip
/// in another keeps the box of what both keep, not of either's box.
fn overlap(a: &[[f32; 2]; 4], b: &[[f32; 2]; 4]) -> [f32; 4] {
    let boxed = intersect(bbox(a), bbox(b));
    let turn: f32 = b
        .iter()
        .zip(b.iter().cycle().skip(1))
        .map(|(p, q)| p[0] * q[1] - q[0] * p[1])
        .sum();
    if turn.abs() < 1e-6 {
        return boxed;
    }
    // A convex quad cut by four lines has eight corners at most.
    let mut poly = [[0.0_f32; 2]; 8];
    for (d, s) in poly.iter_mut().zip(a) {
        *d = *s;
    }
    let mut n = 4_usize;
    for (p, q) in b.iter().zip(b.iter().cycle().skip(1)) {
        let side = |v: [f32; 2]| turn.signum() * ((q[0] - p[0]) * (v[1] - p[1]) - (q[1] - p[1]) * (v[0] - p[0]));
        let mut out = [[0.0_f32; 2]; 8];
        let mut m = 0_usize;
        let live = poly.get(..n).unwrap_or_default();
        for (u, v) in live.iter().zip(live.iter().cycle().skip(1)) {
            let (su, sv) = (side(*u), side(*v));
            let mut keep = |w: [f32; 2]| {
                if let Some(slot) = out.get_mut(m) {
                    *slot = w;
                    m = m.saturating_add(1);
                }
            };
            if su >= 0.0 {
                keep(*u);
            }
            if (su >= 0.0) != (sv >= 0.0) {
                let t = su / (su - sv);
                keep([u[0] + t * (v[0] - u[0]), u[1] + t * (v[1] - u[1])]);
            }
        }
        poly = out;
        n = m;
        if n == 0 {
            return EMPTY;
        }
    }
    intersect(bbox(poly.get(..n).unwrap_or_default()), boxed)
}

/// What the renderer knows of one clip: plain data, so deciding it anew
/// allocates nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Node {
    entry: [f32; ENTRY],
    transform: u32,
    parent: u32,
    next: u32,
    /// Its rectangle's corners in the canvas, in order around.
    quad: [[f32; 2]; 4],
    /// A box of the canvas (x0, y0, x1, y1) where every clip of its walk
    /// keeps the whole of every pixel: each one's rectangle less its
    /// corners and half a pixel, while all are axis-aligned (empty past a
    /// turned one).
    inner: [f32; 4],
    /// What its chain keeps, in the canvas (a bounding box).
    bounds: [f32; 4],
    rev: u64,
    /// The hash of its rectangle, radii, curve, space, depth and parent,
    /// and its parent's shape: all of its ancestry but where the canvas
    /// has it ([`Update::reshaped`]).
    shape: u64,
    /// Clips its walk tests.
    len: u32,
}

/// What bringing the tree up to a scene changed.
#[derive(Default)]
pub(super) struct Update {
    /// Clips whose entry changed: written to the table.
    pub written: Vec<u32>,
    /// Clips whose revision changed.
    pub moved: Vec<u32>,
    /// Clips whose shape changed (by id): not only where the canvas has
    /// them, but what they are in a space that moves with them. A clip
    /// moved with the layer it lies in keeps its shape, and so its key in
    /// that layer ([`ClipTree::key_in`]).
    pub reshaped: Vec<u32>,
    /// For each of those, what its chain kept before and keeps now.
    pub damage: Vec<Rect>,
    /// The longest walk among the clips decided.
    pub longest: usize,
    /// Clips decided, each once.
    pub decided: usize,
    /// Steps taken: clips decided, ancestors walked for depths, and
    /// ancestors looked at deciding nexts.
    pub work: usize,
}

/// What a pass of [`ClipTree::update`] works in, kept across frames so a
/// pass that decides as many clips as the last allocates nothing.
#[derive(Default)]
struct Scratch {
    dirty: Vec<u32>,
    /// Clips to decide, shallowest first.
    queue: BinaryHeap<Reverse<(u32, u32)>>,
    /// Each clip decided that changed, as it was at the frame's start.
    was: Vec<(u32, Node)>,
    up: Ancestry,
    /// An update given back ([`ClipTree::recycle`]): its lists' memory.
    spare: Update,
}

impl Update {
    /// Empty, its lists' memory kept.
    fn take(&mut self) -> Self {
        let mut u = std::mem::take(self);
        u.written.clear();
        u.moved.clear();
        u.reshaped.clear();
        u.damage.clear();
        u.longest = 0;
        u.decided = 0;
        u.work = 0;
        u
    }
}

#[derive(Default)]
pub(super) struct ClipTree {
    nodes: Vec<Node>,
    /// The clips whose parent each clip is.
    children: Vec<foldhash::HashSet<u32>>,
    /// The clips in each transform's space, and that space's map to the canvas.
    by_transform: foldhash::HashMap<u32, foldhash::HashSet<u32>>,
    matrices: foldhash::HashMap<u32, Option<[f32; 6]>>,
    scale: u32,
    /// The pass each clip was last decided in ([`Self::pass`]).
    visited: Vec<u32>,
    pass: u32,
    scratch: Scratch,
}

/// Whether a walk for a target whose output clip lies at depth `stop`
/// tests clip `n`: a clip deeper than it lies inside the layer (Chrome's
/// output clip); it and those above it clip the layer's composite. A
/// walk visits its first clip's ancestors only, so depth tells the two
/// apart (`clip_cov` in `shaders.wgsl` stops alike).
fn cut(n: &Node, stop: u32) -> bool {
    n.entry[17].to_bits() > stop
}

/// Whether a target whose space `to_canvas` maps to the canvas (`None`:
/// the canvas's own) has pixels of its own: scaled, turned or moved off
/// the canvas's. The effective chain is decided in the canvas's pixels
/// (an ancestor left out keeps each of those whole, not each of the
/// target's), so a walk for such a target steps to each clip's parent
/// ([`ClipTree::step`]); coverage is measured in its pixels
/// (`entry_seen` in `common.wgsl`, [`ClipTree::entry_in`]).
pub(super) fn own_grid(to_canvas: Option<[f32; 6]>) -> bool {
    to_canvas.is_some_and(|m| m != IDENTITY)
}

/// Whether transform `t` is `until` or lies under it.
pub(super) fn under(props: &Properties, t: u32, until: u32) -> bool {
    let mut at = t;
    for _ in 0..4096 {
        if at == until {
            return true;
        }
        if at == 0 {
            return false;
        }
        at = props.transform(at).parent;
    }
    false
}

/// A target clip walks are keyed for ([`ClipTree::key_in`]): the space it
/// lies in (transform `until`, which `to_canvas` maps to the canvas), where
/// its pixel (0, 0) lies in that space, and the depth its walks end at.
pub(super) struct KeyTarget {
    pub until: u32,
    pub to_canvas: Option<[f32; 6]>,
    pub origin: (f32, f32),
    pub stop: u32,
}

/// A target's clip keys, kept across frames ([`ClipTree::key_in`]), and
/// what they read: the clips walked, the transforms composed through, and
/// the target's own map to the canvas when a key went through the canvas.
/// A key holds until something it read changes; its owner tells it what
/// did ([`Self::forget`], [`Self::clear`], [`Self::moved_with`]), from the
/// reverse index it keeps of what each target read ([`Self::take_reads`]).
#[derive(Debug, Default)]
pub(super) struct ClipKeys {
    /// Each clip's key: whether it was made through the target's space,
    /// the key (of its walk from it on).
    keys: foldhash::HashMap<u32, (bool, u64)>,
    /// The target they were made for: whether its pixels are its own, its
    /// space, its walks' end, its origin.
    made: Option<(bool, u32, u32, u32, u32)>,
    to_canvas: Option<[f32; 6]>,
    /// Whether a key was made through the canvas in a target of pixels of
    /// its own (a clip outside its space): all go when the target moves.
    absolute: bool,
    /// The transforms between a clip keyed in the target's space and that
    /// space: a key through them holds while each does.
    through: foldhash::HashSet<u32>,
    /// Clips and transforms read for the first time since their owner last
    /// took them ([`Self::take_reads`]).
    fresh: Vec<u32>,
    fresh_through: Vec<u32>,
    walk: Vec<u32>,
    /// Clips keyed since [`Self::walked`] was last read.
    #[cfg(test)]
    walked: usize,
}

impl ClipKeys {
    /// Every key gone: a transform a key composed through moved (or what
    /// the keys were made for is gone).
    pub(super) fn clear(&mut self) {
        self.keys.clear();
        self.through.clear();
        self.absolute = false;
    }

    /// Clip `c` changed: its shape (`reshaped`), or only where it lies in
    /// the canvas. A key of it, and of every clip under it (whose walks
    /// read it, by its parent or by its next), goes when it read what
    /// changed: any key of a clip reshaped, a key through the canvas of
    /// one moved (one through the target's space reads only its shape,
    /// and the transforms between, [`Self::clear`]). Whether any went.
    pub(super) fn forget(&mut self, tree: &ClipTree, c: u32, reshaped: bool) -> bool {
        match self.keys.get(&c) {
            Some(&(inside, _)) if reshaped || !inside => {}
            _ => return false,
        }
        let mut stack = std::mem::take(&mut self.walk);
        stack.clear();
        stack.push(c);
        let mut steps = 0_usize;
        while let Some(at) = stack.pop() {
            steps = steps.saturating_add(1);
            if steps > tree.nodes.len() {
                break;
            }
            self.keys.remove(&at);
            if let Some(kids) = usize::try_from(at).ok().and_then(|i| tree.children.get(i)) {
                stack.extend(kids.iter().copied());
            }
        }
        self.walk = stack;
        true
    }

    /// Whether the target moved to `to_canvas` from where a key through
    /// the canvas was made: every key goes ([`ClipTree::key_in`]), and
    /// its texels are drawn again.
    pub(super) fn moved_with(&self, to_canvas: Option<[f32; 6]>) -> bool {
        self.absolute && self.to_canvas != to_canvas
    }

    /// The clips, and the transforms, read since last taken, each to
    /// `clip` and `through` (the owner's reverse index).
    pub(super) fn take_reads(&mut self, mut clip: impl FnMut(u32), mut through: impl FnMut(u32)) {
        self.fresh.drain(..).for_each(&mut clip);
        self.fresh_through.drain(..).for_each(&mut through);
    }

    /// Whether the keys were made for pixels of the target's own (none:
    /// not made yet).
    pub(super) fn made_own(&self) -> Option<bool> {
        self.made.map(|m| m.0)
    }

    /// Clips keyed since last asked.
    #[cfg(test)]
    pub(super) fn walked(&mut self) -> usize {
        std::mem::take(&mut self.walked)
    }
}

/// The pixels a chain's bounds (x0, y0, x1, y1) may touch, a pixel grown.
fn damage_of(b: [f32; 4]) -> Option<Rect> {
    Rect::covering([b[0] - 1.0, b[1] - 1.0, b[2] + 1.0, b[3] + 1.0])
}

impl ClipTree {
    fn node(&self, id: u32) -> Option<&Node> {
        usize::try_from(id)
            .ok()
            .filter(|&i| i != 0)
            .and_then(|i| self.nodes.get(i))
    }

    /// Clip `id`'s revision (0 for none).
    pub(super) fn rev(&self, id: u32) -> u64 {
        self.node(id).map_or(0, |n| n.rev)
    }

    /// What clip `id`'s chain keeps, in the canvas (x0, y0, x1, y1).
    pub(super) fn bounds(&self, id: u32) -> Option<[f32; 4]> {
        self.node(id).map(|n| n.bounds)
    }

    /// The next clip of `id`'s effective chain.
    #[cfg(test)]
    pub(super) fn next(&self, id: u32) -> u32 {
        self.node(id).map_or(0, |n| n.next)
    }

    /// The clip after `id` in a walk for a target of pixels of its own
    /// (`own`, [`own_grid`]: its parent) or the canvas's (its next).
    /// `step_at` in `shaders.wgsl` steps alike.
    pub(super) fn step(&self, id: u32, own: bool) -> u32 {
        self.node(id).map_or(0, |n| if own { n.parent } else { n.next })
    }

    /// Clip `id`'s depth: 1 for a clip the root holds, 0 for none. A walk
    /// for a target that a layer's output clip `o` clips ends at the first
    /// clip as shallow as `o` ([`cut`]).
    pub(super) fn depth(&self, id: u32) -> u32 {
        self.node(id).map_or(0, |n| n.entry[17].to_bits())
    }

    /// What clip `id`'s walk keeps up to (not including) its first clip
    /// at depth `stop` or less, in the canvas (x0, y0, x1, y1): its chain's
    /// bounds when `stop` is 0; `None` when that leaves nothing of it.
    pub(super) fn bounds_until(&self, id: u32, stop: u32) -> Option<[f32; 4]> {
        if stop == 0 {
            return self.bounds(id);
        }
        let mut kept: Option<[f32; 4]> = None;
        let mut at = id;
        let mut steps = 0_usize;
        while let Some(n) = self.node(at).filter(|n| cut(n, stop)) {
            steps = steps.saturating_add(1);
            if steps > self.nodes.len() {
                break;
            }
            let own = bbox(&n.quad);
            kept = Some(kept.map_or(own, |k| intersect(k, own)));
            at = n.next;
        }
        kept
    }

    /// How many clips `id`'s walk tests.
    #[cfg(test)]
    pub(super) fn walk_len(&self, id: u32) -> usize {
        self.node(id)
            .map_or(0, |n| usize::try_from(n.len).unwrap_or(usize::MAX))
    }

    /// Clip `id`'s entry (its map from the canvas).
    pub(super) fn entry(&self, id: u32) -> [f32; ENTRY] {
        self.node(id).map_or([0.0; ENTRY], |n| n.entry)
    }

    /// Every entry, by id, for the table written whole.
    pub(super) fn entries(&self) -> impl Iterator<Item = [f32; ENTRY]> + '_ {
        self.nodes.iter().map(|n| n.entry)
    }

    /// Clip `id`'s entry for a target whose space `to_canvas` maps to the
    /// canvas (`None`: the canvas's own), its next left to the caller.
    pub(super) fn entry_in(&self, id: u32, to_canvas: Option<[f32; 6]>) -> [f32; ENTRY] {
        let mut e = self.entry(id);
        let Some(t) = to_canvas else { return e };
        let map = [e[8], e[9], e[10], e[11], e[12], e[13]];
        let m = path::then(t, map);
        let scale = path::invert(m).map_or(0.0, |g| path::scales(g).0);
        e[8..14].copy_from_slice(&m);
        e[14] = scale;
        e
    }

    /// Give an update's lists back for the next to fill.
    pub(super) fn recycle(&mut self, u: Update) {
        self.scratch.spare = u;
    }

    /// A key of clip `id`'s walk ([`Self::step`]) as `target` sees it: of
    /// each clip, the map from the target's pixels to its own rectangle's
    /// corner, its size, radii and curve. That is what decides the pixels
    /// the walk keeps there, and nothing else is: a clip and a target
    /// moved together key the same; a target turned under a clip that
    /// stays does not. The walk ends at its first clip at depth
    /// `target.stop` or less (the target's output clip's, [`Self::depth`]).
    ///
    /// `keys` holds the target's keys across frames, and what each read: a
    /// clip in the target's space keys by its map to that space, which the
    /// target's own motion leaves alone, so it reads the clip's shape and
    /// the transforms between; one outside it, through the canvas, reads
    /// where the clip lies there and the target's own map. A key is not
    /// walked again until its owner tells `keys` something it read changed
    /// ([`ClipKeys::forget`], [`ClipKeys::clear`]).
    pub(super) fn key_in(&self, props: &Properties, id: u32, target: &KeyTarget, keys: &mut ClipKeys) -> u64 {
        let own = own_grid(target.to_canvas);
        let made = (
            own,
            target.until,
            target.stop,
            target.origin.0.to_bits(),
            target.origin.1.to_bits(),
        );
        if keys.made != Some(made) || keys.moved_with(target.to_canvas) {
            keys.clear();
            keys.made = Some(made);
        }
        keys.to_canvas = target.to_canvas;
        // A key holds until what it read changes, which its owner tells
        // `keys` ([`ClipKeys::forget`]).
        let mut walk = std::mem::take(&mut keys.walk);
        walk.clear();
        let mut at = id;
        let mut key = loop {
            if let Some(&(_, k)) = keys.keys.get(&at) {
                break k;
            }
            let Some(n) = self.node(at).filter(|n| cut(n, target.stop)) else {
                break 0;
            };
            walk.push(at);
            at = if own { n.parent } else { n.next };
        };
        let origin = [1.0, 0.0, 0.0, 1.0, target.origin.0, target.origin.1];
        let from = path::then(origin, target.to_canvas.unwrap_or(IDENTITY));
        for &c in walk.iter().rev() {
            let e = self.entry(c);
            let canvas = [e[8], e[9], e[10], e[11], e[12], e[13]];
            // From the target's pixels into the clip's space: through the
            // target's space when the clip lies in it, unmoved by what moves
            // both; else through the canvas.
            let (inside, into) = match self.node(c).filter(|_| own).map(|n| n.transform) {
                Some(t) if under(props, t, target.until) => {
                    let mut x = t;
                    for _ in 0..4096 {
                        if x == target.until || x == 0 {
                            break;
                        }
                        if keys.through.insert(x) {
                            keys.fresh_through.push(x);
                        }
                        x = props.transform(x).parent;
                    }
                    let g = compose(props, t, target.until).unwrap_or(IDENTITY);
                    (true, path::then(origin, path::invert(g).unwrap_or([0.0; 6])))
                }
                _ => {
                    keys.absolute |= own;
                    (false, path::then(from, canvas))
                }
            };
            let m = path::then(into, [1.0, 0.0, 0.0, 1.0, -e[0], -e[1]]);
            let mut h = DefaultHasher::new();
            key.hash(&mut h);
            for v in m.iter().chain(&e[2..8]).chain(std::iter::once(&e[15])) {
                // -0 and 0 keep the same pixels.
                (v + 0.0).to_bits().hash(&mut h);
            }
            key = h.finish().max(1);
            if keys.keys.insert(c, (inside, key)).is_none() {
                keys.fresh.push(c);
            }
            #[cfg(test)]
            {
                keys.walked = keys.walked.saturating_add(1);
            }
        }
        keys.walk = walk;
        key
    }

    /// The tree brought up to `props` at scale `s`: decided whole (`whole`,
    /// a new shape or scale), or from the clips and transforms that changed.
    pub(super) fn update(
        &mut self,
        props: &Properties,
        s: f32,
        whole: bool,
        clips: &[u32],
        transforms: &[u32],
    ) -> Update {
        if whole || self.scale != s.to_bits() || self.nodes.len() != props.clips.len() {
            return self.rebuild(props, s);
        }
        let mut sc = std::mem::take(&mut self.scratch);
        sc.dirty.clear();
        sc.queue.clear();
        sc.was.clear();
        sc.up.clear();
        sc.dirty.extend_from_slice(clips);
        for t in transforms {
            let Some(set) = self.by_transform.get(t) else { continue };
            let now = compose(props, *t, 0);
            if self.matrices.get(t) != Some(&now) {
                sc.dirty.extend(set.iter().copied());
                self.matrices.insert(*t, now);
            }
        }
        self.pass = self.pass.wrapping_add(1);
        if self.pass == 0 {
            self.visited.fill(0);
            self.pass = 1;
        }
        // Parents first: a clip waits behind every shallower one.
        for &id in &sc.dirty {
            if self.node(id).is_some() {
                sc.queue.push(Reverse((sc.up.depth(props, id), id)));
            }
        }
        let mut out = sc.spare.take();
        while let Some(Reverse((depth, id))) = sc.queue.pop() {
            let Some(mark) = usize::try_from(id).ok().and_then(|i| self.visited.get_mut(i)) else {
                continue;
            };
            if *mark == self.pass {
                continue;
            }
            *mark = self.pass;
            let Some(old) = self.node(id).copied() else { continue };
            let parent = sc.up.parent(props, id);
            let new = self.decide(props, s, (id, depth), parent, &mut out.work);
            out.decided = out.decided.saturating_add(1);
            if new == old {
                continue;
            }
            self.relink(id, Some(&old), &new);
            if let Some(slot) = usize::try_from(id).ok().and_then(|i| self.nodes.get_mut(i)) {
                *slot = new;
            }
            sc.was.push((id, old));
            // Its children's walks read its walk: each is decided again.
            if let Some(kids) = usize::try_from(id).ok().and_then(|i| self.children.get(i)) {
                for &k in kids {
                    let seen = usize::try_from(k)
                        .ok()
                        .and_then(|i| self.visited.get(i))
                        .is_some_and(|m| *m == self.pass);
                    if !seen {
                        sc.queue.push(Reverse((sc.up.depth(props, k), k)));
                    }
                }
            }
        }
        out.work = out.work.saturating_add(sc.up.work).saturating_add(out.decided);
        sc.was.sort_unstable_by_key(|(id, _)| *id);
        for (id, old) in &sc.was {
            if let Some(new) = self.node(*id) {
                diff(*id, Some(old), new, &mut out);
            }
        }
        self.scratch = sc;
        out
    }

    /// Every clip decided anew, parents first, against what was.
    fn rebuild(&mut self, props: &Properties, s: f32) -> Update {
        let n = props.clips.len();
        let old = std::mem::take(&mut self.nodes);
        self.nodes = vec![Node::default(); n];
        self.children = vec![foldhash::HashSet::default(); n];
        self.visited = vec![0; n];
        self.pass = 0;
        self.by_transform.clear();
        self.matrices.clear();
        self.scale = s.to_bits();
        let mut sc = std::mem::take(&mut self.scratch);
        sc.up.clear();
        let mut order: Vec<(u32, u32)> = (1..n)
            .filter_map(|i| u32::try_from(i).ok())
            .map(|id| (sc.up.depth(props, id), id))
            .collect();
        order.sort_unstable();
        let mut out = sc.spare.take();
        for (depth, id) in order {
            let parent = sc.up.parent(props, id);
            let node = self.decide(props, s, (id, depth), parent, &mut out.work);
            out.decided = out.decided.saturating_add(1);
            self.relink(id, None, &node);
            let i = usize::try_from(id).unwrap_or(0);
            diff(id, old.get(i), &node, &mut out);
            if let Some(slot) = self.nodes.get_mut(i) {
                *slot = node;
            }
        }
        out.work = out.work.saturating_add(sc.up.work).saturating_add(out.decided);
        out.reshaped.sort_unstable();
        self.scratch = sc;
        out
    }

    /// The child links and transform index from `old` (none: a clip met
    /// first) to `new` for clip `id`: they change only when its parent or
    /// its space does.
    fn relink(&mut self, id: u32, old: Option<&Node>, new: &Node) {
        if old.is_none_or(|o| o.parent != new.parent) {
            if let Some(o) = old
                && let Some(kids) = usize::try_from(o.parent).ok().and_then(|p| self.children.get_mut(p))
            {
                kids.remove(&id);
            }
            if new.parent != 0
                && let Some(kids) = usize::try_from(new.parent).ok().and_then(|p| self.children.get_mut(p))
            {
                kids.insert(id);
            }
        }
        if old.is_none_or(|o| o.transform != new.transform) {
            if let Some(o) = old
                && let Some(set) = self.by_transform.get_mut(&o.transform)
            {
                set.remove(&id);
            }
            self.by_transform.entry(new.transform).or_default().insert(id);
        }
    }

    /// Clip `id`, at `depth`, under `parent` as the tree's other clips have
    /// it now, the ancestors it looks at counted into `work`.
    fn decide(&mut self, props: &Properties, s: f32, (id, depth): (u32, u32), parent: u32, work: &mut usize) -> Node {
        let Some(c) = usize::try_from(id)
            .ok()
            .filter(|&i| i != 0)
            .and_then(|i| props.clips.get(i))
        else {
            return Node::default();
        };
        let fwd = *self
            .matrices
            .entry(c.transform)
            .or_insert_with(|| compose(props, c.transform, 0));
        let rect = super::phys(c.clip.bounds.snapped(s), s);
        let radii = super::radii(c.clip.radius, c.clip.bounds, s);
        let local = [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]];
        let g = fwd.unwrap_or(IDENTITY);
        let (map, scale) = match fwd {
            None => (IDENTITY, 1.0),
            Some(g) => (path::invert(g).unwrap_or([0.0; 6]), path::scales(g).0),
        };
        let quad = [
            [local[0], local[1]],
            [local[2], local[1]],
            [local[2], local[3]],
            [local[0], local[3]],
        ]
        .map(|p| path::apply(g, p));
        // The pixels it covers at all, and its own part of the box where
        // its walk keeps every pixel whole.
        let reach = crate::clip::reach(local, g, scale);
        let span = bbox(&reach);
        let own_inner = if g[1] == 0.0 && g[2] == 0.0 {
            let corner = radii.iter().copied().fold(0.0_f32, f32::max);
            let m = corner.max(0.5 / scale.max(1e-6));
            let r = [local[0] + m, local[1] + m, local[2] - m, local[3] - m];
            if r[0] <= r[2] && r[1] <= r[3] {
                mapped(r, g)
            } else {
                EMPTY
            }
        } else {
            EMPTY
        };
        // Its next: the first of its parent's walk that does not keep
        // every pixel it covers whole; none once the rest of the walk does.
        let mut at = parent;
        let mut steps = 0_usize;
        while let Some(a) = self.node(at) {
            steps = steps.saturating_add(1);
            *work = work.saturating_add(1);
            if steps > self.nodes.len() || within(span, a.inner) {
                at = 0;
                break;
            }
            let e = &a.entry;
            let held = crate::clip::holds(
                [e[0], e[1], e[2], e[3]],
                [e[4], e[5], e[6], e[7]],
                [e[8], e[9], e[10], e[11], e[12], e[13]],
                e[14],
                &reach,
            );
            if !held {
                break;
            }
            at = a.next;
        }
        let next = at;
        let (bounds, len, up, inner) = match self.node(next) {
            Some(n) => (
                intersect(overlap(&quad, &n.quad), n.bounds),
                n.len.saturating_add(1),
                n.rev,
                intersect(own_inner, n.inner),
            ),
            None => (bbox(&quad), 1, 0, own_inner),
        };
        let mut entry = [0.0; ENTRY];
        entry[..4].copy_from_slice(&rect);
        entry[4..8].copy_from_slice(&radii);
        entry[8..14].copy_from_slice(&map);
        entry[14] = scale;
        entry[15] = c.clip.curve;
        entry[16] = f32::from_bits(next);
        entry[17] = f32::from_bits(depth);
        entry[18] = f32::from_bits(parent);
        let mut h = DefaultHasher::new();
        for v in entry {
            v.to_bits().hash(&mut h);
        }
        (parent, up).hash(&mut h);
        let rev = h.finish().max(1);
        let mut h = DefaultHasher::new();
        for v in entry[..8].iter().chain(&entry[15..16]) {
            v.to_bits().hash(&mut h);
        }
        (c.transform, depth, parent, self.node(parent).map_or(0, |p| p.shape)).hash(&mut h);
        Node {
            entry,
            transform: c.transform,
            parent,
            next,
            quad,
            inner,
            bounds,
            rev,
            shape: h.finish().max(1),
            len,
        }
    }
}

/// What clip `id` going from `old` to `new` writes, moves and damages.
fn diff(id: u32, old: Option<&Node>, new: &Node, out: &mut Update) {
    if old.is_none_or(|o| o.entry != new.entry) {
        out.written.push(id);
    }
    if old.is_none_or(|o| o.shape != new.shape) {
        out.reshaped.push(id);
    }
    if old.is_none_or(|o| o.rev != new.rev) {
        out.moved.push(id);
        if let Some(o) = old {
            out.damage.extend(damage_of(o.bounds));
        }
        out.damage.extend(damage_of(new.bounds));
    }
    out.longest = out.longest.max(usize::try_from(new.len).unwrap_or(usize::MAX));
}

/// Clip `id`'s parent as the scene says, or 0 when out of range.
fn parent_of(props: &Properties, id: u32) -> u32 {
    let p = usize::try_from(id)
        .ok()
        .and_then(|i| props.clips.get(i))
        .map_or(0, |c| c.parent);
    if usize::try_from(p).is_ok_and(|p| p < props.clips.len()) {
        p
    } else {
        0
    }
}

/// The clips' depths this frame, each clip's ancestry walked once: a walk
/// stops at the root or at a clip already known. The scene's clips must
/// not loop (a debug build stops at one); a clip whose parents never reach
/// the root hangs from it.
#[derive(Default)]
struct Ancestry {
    /// Each clip met: its depth by its parents, or `None` when they never
    /// reach the root.
    reach: foldhash::HashMap<u32, Option<u32>>,
    walking: foldhash::HashSet<u32>,
    path: Vec<u32>,
    /// Clips stepped over.
    work: usize,
}

impl Ancestry {
    /// Ready for another frame, what it holds kept for it.
    fn clear(&mut self) {
        self.reach.clear();
        self.walking.clear();
        self.path.clear();
        self.work = 0;
    }

    fn reach(&mut self, props: &Properties, id: u32) -> Option<u32> {
        self.path.clear();
        self.walking.clear();
        let mut at = id;
        let mut d = loop {
            if at == 0 {
                break Some(0);
            }
            if let Some(d) = self.reach.get(&at) {
                break *d;
            }
            let fresh = self.walking.insert(at);
            debug_assert!(
                fresh,
                "clip {at}'s parents loop back to it: the scene's clips must form a tree"
            );
            if !fresh {
                break None;
            }
            self.work = self.work.saturating_add(1);
            self.path.push(at);
            at = parent_of(props, at);
        };
        for p in self.path.iter().rev() {
            d = d.map(|d| d.saturating_add(1));
            self.reach.insert(*p, d);
        }
        d
    }

    /// Clip `id`'s depth: 0 for the root, 1 for a clip it holds.
    fn depth(&mut self, props: &Properties, id: u32) -> u32 {
        self.reach(props, id).unwrap_or(1)
    }

    /// Clip `id`'s parent, or 0 when its parents never reach the root.
    fn parent(&mut self, props: &Properties, id: u32) -> u32 {
        let p = parent_of(props, id);
        if p != 0 && self.reach(props, p).is_some() { p } else { 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Bounds;
    use crate::scene::{Clip, ClipNode};
    use crate::style::Corners;

    /// `n` rounded clips, each inside the one before, inset a twentieth of
    /// a pixel.
    fn chain(n: u16, radius: f32) -> Properties {
        let mut props = Properties::default();
        for k in 0..n {
            let d = f32::from(k) * 0.05;
            props.clips.push(ClipNode {
                clip: Clip {
                    bounds: Bounds {
                        x: d,
                        y: d,
                        width: 1200.0 - d * 2.0,
                        height: 1200.0 - d * 2.0,
                    },
                    radius: Corners::all(radius),
                    ..Clip::default()
                },
                transform: 0,
                parent: u32::from(k),
            });
        }
        props
    }

    #[test]
    fn every_radius_of_a_deep_rounded_chain_changing_decides_each_clip_once() {
        let mut props = chain(200, 4.0);
        let mut tree = ClipTree::default();
        tree.update(&props, 1.0, true, &[], &[]);
        let before: Vec<[f32; 4]> = (1..=200).filter_map(|id| tree.bounds(id)).collect();
        for c in props.clips.iter_mut().skip(1) {
            c.clip.radius = Corners::all(9.0);
        }
        let dirty: Vec<u32> = (1..=200).collect();
        let up = tree.update(&props, 1.0, false, &dirty, &[]);
        assert_eq!(up.decided, 200, "each clip decided once");
        let mut fresh = ClipTree::default();
        fresh.update(&props, 1.0, true, &[], &[]);
        assert!(
            tree.entries().eq(fresh.entries()),
            "every entry as a fresh table has it"
        );
        for id in 1..=200 {
            assert_eq!(tree.rev(id), fresh.rev(id), "clip {id}'s revision");
        }
        assert_eq!(up.moved.len(), 200);
        for b in before {
            let old = damage_of(b).unwrap();
            assert!(up.damage.contains(&old), "the damage covers {old:?}");
        }
    }

    #[test]
    fn a_five_thousand_deep_rounded_chain_prepares_on_a_small_stack() {
        let props = chain(5000, 4.0);
        let walked = std::thread::Builder::new()
            .stack_size(256 << 10)
            .spawn(move || {
                let mut tree = ClipTree::default();
                tree.update(&props, 1.0, true, &[], &[]);
                let mut fr = super::super::frame::FrameData::default();
                let mut at = fr.pclip(&tree, 5000, (0, 0), || None);
                let mut n = 0_usize;
                while at != 0 {
                    n = n.saturating_add(1);
                    let e = fr.pclips.chunks(ENTRY).nth(usize::try_from(at).unwrap()).unwrap();
                    at = e[16].to_bits();
                }
                n
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(walked, 5000, "the whole walk written");
    }

    #[test]
    fn ten_thousand_flat_clips_then_a_two_thousand_deep_chain_decide_in_linear_work() {
        let mut flat = Properties::default();
        for k in 0..10_000_u16 {
            let d = f32::from(k % 100);
            flat.clips.push(ClipNode {
                clip: Clip {
                    bounds: Bounds {
                        x: d,
                        y: d,
                        width: 10.0,
                        height: 10.0,
                    },
                    ..Clip::default()
                },
                ..ClipNode::default()
            });
        }
        let mut tree = ClipTree::default();
        let up = tree.update(&flat, 1.0, true, &[], &[]);
        assert_eq!(up.decided, 10_000);
        assert!(up.work <= 40_000, "flat work {}", up.work);
        let up = tree.update(&chain(2000, 4.0), 1.0, true, &[], &[]);
        assert_eq!(up.decided, 2000);
        assert!(up.work <= 8000, "deep work {}", up.work);
        // A change at the root of the chain reaches each clip once.
        let mut deep = chain(2000, 4.0);
        if let Some(c) = deep.clips.get_mut(1) {
            c.clip.radius = Corners::all(6.0);
        }
        let up = tree.update(&deep, 1.0, false, &[1], &[]);
        assert_eq!(up.decided, 2000);
        assert!(up.work <= 8000, "incremental work {}", up.work);
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds {
        Bounds { x, y, width, height }
    }

    /// A clip node of `bounds` rounded `radius` under `parent`, in
    /// transform `transform`'s space.
    fn node(bounds: Bounds, radius: f32, parent: u32, transform: u32) -> ClipNode {
        ClipNode {
            clip: Clip {
                bounds,
                radius: Corners::all(radius),
                curve: crate::shape::ROUND,
            },
            transform,
            parent,
        }
    }

    /// A rounded container whose corners animate over a thousand rows it
    /// clips (each touching its sides, so it stays in their chains): each
    /// row is decided again, and none allocates. What a frame allocates is
    /// its update's lists of what moved, a handful of doublings, not a
    /// vector per row (frontiers: each row's dependency list was rebuilt).
    #[test]
    fn a_rounded_parent_animating_its_radius_over_a_thousand_children_allocates_nothing_per_child() {
        let mut props = Properties::default();
        props.clips.push(node(rect(0.0, 0.0, 1000.0, 1000.0), 12.0, 0, 0));
        for k in 0..1000_u16 {
            props.clips.push(node(rect(0.0, f32::from(k), 1000.0, 1.0), 0.0, 1, 0));
        }
        let mut tree = ClipTree::default();
        tree.update(&props, 1.0, true, &[], &[]);
        let mut frame = |r: f32| {
            if let Some(c) = props.clips.get_mut(1) {
                c.clip.radius = Corners::all(r);
            }
            tree.update(&props, 1.0, false, &[1], &[])
        };
        // The first frame sizes what passes work in.
        frame(12.5);
        for r in [13.0, 13.5, 14.0] {
            let (up, spent) = crate::heap::allocations(|| frame(r));
            assert_eq!(up.decided, 1001, "the container and each row");
            assert_eq!(up.written, vec![1], "the rows' entries stay");
            assert_eq!(up.moved.len(), 1001);
            assert!(spent <= 40, "{spent} allocations for a frame of 1001 clips");
        }
    }

    /// Thirty rounded cards, each inset sixteen within the last: the
    /// innermost's pixels lie where each card keeps every pixel whole
    /// (inside its corners), so its walk is itself alone; one that meets
    /// its card's corner keeps that card.
    #[test]
    fn thirty_nested_rounded_cards_leave_each_card_out_of_the_innermost_walk() {
        let mut props = Properties::default();
        for k in 0..30_u16 {
            let d = f32::from(k) * 16.0;
            props.clips.push(node(
                rect(d, d, 1200.0 - d * 2.0, 1200.0 - d * 2.0),
                8.0,
                u32::from(k),
                0,
            ));
        }
        // Inside the innermost card, then over its corner.
        props.clips.push(node(rect(500.0, 500.0, 100.0, 100.0), 8.0, 30, 0));
        props.clips.push(node(rect(464.0, 464.0, 100.0, 100.0), 0.0, 30, 0));
        let mut tree = ClipTree::default();
        tree.update(&props, 1.0, true, &[], &[]);
        assert_eq!(tree.walk_len(31), 1, "every card left out");
        assert_eq!(tree.next(32), 30, "the card whose corner it meets");
        assert_eq!(tree.walk_len(32), 2);
    }

    /// The effective chain keeps what the whole chain keeps, pixel for
    /// pixel: nested rounded and square clips at whole and odd places, each
    /// pixel's coverage (`shape_cov`) multiplied along the walk and along
    /// every ancestor agree.
    #[test]
    fn a_walk_keeps_each_pixel_as_the_whole_chain_does() {
        for (inset, radius) in [
            (0.0, 0.0),
            (0.3, 0.0),
            (0.5, 0.0),
            (1.0, 0.0),
            (0.5, 4.0),
            (4.0, 4.0),
            (4.5, 4.0),
            (8.0, 8.0),
        ] {
            let mut props = Properties::default();
            for k in 0..6_u16 {
                let d = f32::from(k) * inset + 0.25;
                props.clips.push(node(
                    rect(d, d, 64.0 - d * 2.0, 48.0 - d * 2.0),
                    if k % 2 == 0 { radius } else { 0.0 },
                    u32::from(k),
                    0,
                ));
            }
            let mut tree = ClipTree::default();
            tree.update(&props, 1.0, true, &[], &[]);
            let cov = |id: u32, p: [f32; 2]| {
                let e = tree.entry(id);
                crate::clip::coverage([e[0], e[1], e[2], e[3]], [e[4], e[5], e[6], e[7]], e[15], p)
            };
            for y in 0..48_u16 {
                for x in 0..64_u16 {
                    let p = [f32::from(x) + 0.5, f32::from(y) + 0.5];
                    let whole: f32 = (1..=6).map(|id| cov(id, p)).product();
                    let mut walked = 1.0;
                    let mut at = 6;
                    while at != 0 {
                        walked *= cov(at, p);
                        at = tree.next(at);
                    }
                    assert!(
                        (whole - walked).abs() < 1e-3,
                        "inset {inset} radius {radius} at {p:?}: {whole} and {walked}"
                    );
                }
            }
        }
    }

    /// A thousand nested square clips in a group scaled twice, their
    /// left edges one, a thousandth of a device pixel into the first column:
    /// each keeps 0.999 of it, short of whole, so none is left out of the
    /// walk, and the walk keeps what the whole chain keeps (0.999 to the
    /// thousandth, 0.37), not one clip's 0.999. A slack on whole, however
    /// small, adds up over a chain this long.
    #[test]
    fn a_thousand_coincident_clips_a_hair_inside_a_pixel_keep_it_as_the_whole_chain_does() {
        let mut props = Properties::default();
        props.transforms.push(crate::scene::TransformNode {
            parent: 0,
            offset: (0.0, 0.0),
            matrix: Some(crate::scene::GroupTransform {
                origin: (-0.001, 0.0),
                scale: 2.0,
                rotate: 0.0,
            }),
        });
        // Each a pixel short of its parent right and down: only the left
        // and top edges meet.
        for k in 0..1000_u16 {
            let size = 1100.0 - f32::from(k);
            props.clips.push(node(rect(0.0, 0.0, size, size), 0.0, u32::from(k), 1));
        }
        let mut tree = ClipTree::default();
        tree.update(&props, 1.0, true, &[], &[]);
        let cov = |id: u32, p: [f32; 2]| {
            let e = tree.entry(id);
            crate::clip::entry_coverage(
                [e[0], e[1], e[2], e[3]],
                [e[4], e[5], e[6], e[7]],
                e[15],
                [e[8], e[9], e[10], e[11], e[12], e[13]],
                e[14],
                p,
            )
        };
        for x in 0..3_u16 {
            let p = [f32::from(x) + 0.5, 50.5];
            let whole: f32 = (1..=1000).map(|id| cov(id, p)).product();
            let mut walked = 1.0;
            let mut at = 1000;
            while at != 0 {
                walked *= cov(at, p);
                at = tree.next(at);
            }
            assert!(
                (whole - walked).abs() < 1e-4,
                "at {p:?}: the chain keeps {whole}, the walk {walked}"
            );
        }
        assert!(cov(1, [0.5, 50.5]) < 1.0, "a hair short of whole");
    }

    /// A small clip in the middle of a turned square leaves it out (its
    /// pixels lie inside the square, tested in the square's space); a
    /// turned clip in a box it misses keeps nothing, not the corner of its
    /// bounding box the box meets.
    #[test]
    fn a_turned_clip_is_left_out_and_cut_by_its_corners_not_its_box() {
        let mut props = Properties::default();
        props.transforms.push(crate::scene::TransformNode {
            parent: 0,
            offset: (0.0, 0.0),
            matrix: Some(crate::scene::GroupTransform {
                origin: (600.0, 600.0),
                scale: 1.0,
                rotate: 45.0,
            }),
        });
        // A square turned to a diamond about (600, 600), and a clip in its middle.
        props.clips.push(node(rect(400.0, 400.0, 400.0, 400.0), 0.0, 0, 1));
        props.clips.push(node(rect(590.0, 590.0, 20.0, 20.0), 0.0, 1, 0));
        // A box at the diamond's bounding box's corner, and the diamond in it.
        props.clips.push(node(rect(300.0, 300.0, 100.0, 100.0), 0.0, 0, 0));
        props.clips.push(node(rect(400.0, 400.0, 400.0, 400.0), 0.0, 3, 1));
        let mut tree = ClipTree::default();
        tree.update(&props, 1.0, true, &[], &[]);
        assert_eq!(tree.walk_len(2), 1, "the diamond left out");
        assert_eq!(tree.next(4), 3, "the box cuts the diamond");
        assert!(
            tree.bounds(4).and_then(damage_of).is_none(),
            "nothing kept: {:?}",
            tree.bounds(4)
        );
    }

    /// Five hundred square clips, each cut by the next, and a thousand
    /// small clips deep inside them all: each small one's walk ends at
    /// once (the box its parent's whole walk keeps), not after five hundred
    /// steps.
    #[test]
    fn a_thousand_leaves_under_a_long_walk_decide_in_linear_work() {
        let mut props = Properties::default();
        for k in 1..=500_u16 {
            props.clips.push(node(
                rect(f32::from(k) - 1.0, 0.0, 1000.0, 1200.0),
                0.0,
                u32::from(k) - 1,
                0,
            ));
        }
        for k in 0..1000_u16 {
            let y = f32::from(k);
            props.clips.push(node(rect(500.0, y, 10.0, 1.0), 0.0, 500, 0));
        }
        let mut tree = ClipTree::default();
        let up = tree.update(&props, 1.0, true, &[], &[]);
        assert_eq!(tree.walk_len(500), 500, "each cut by the next");
        assert_eq!(tree.walk_len(1000), 1);
        assert!(up.work <= 6000, "work {}", up.work);
    }

    /// Clips whose parents loop: the scene is wrong, and a debug build
    /// says so as the renderer takes it in.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "loop back")]
    fn clips_whose_parents_loop_stop_a_debug_build() {
        let mut props = Properties::default();
        props.clips.push(node(rect(0.0, 0.0, 10.0, 10.0), 0.0, 2, 0));
        props.clips.push(node(rect(0.0, 0.0, 10.0, 10.0), 0.0, 1, 0));
        ClipTree::default().update(&props, 1.0, true, &[], &[]);
    }
}
