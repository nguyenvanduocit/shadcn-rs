//! The clip tree as the renderer keeps it (ADR-0016; Chrome's clip-node
//! tree): for each clip of the scene, the entry the shaders read and the
//! facts every cache and damage path reads from one place.
//!
//! **Entries.** A clip's entry is five vec4s: its own rounded rectangle
//! (physical, in its own space), its radii, the map from the canvas into
//! that space (`a, b, c, d`, then `e, f`, its scale and its corners'
//! curve), and the next clip of its *effective* chain. So a chain crosses
//! scales and turns: each clip is tested in its own space.
//!
//! **Effective chains.** A clip's next is the first clip of its parent's
//! walk that can change a pixel it keeps ([`crate::clip::needed`]): a
//! square, axis-aligned ancestor containing it is left out. Its walk
//! (it, then its next's walk) keeps what its whole chain keeps, and the
//! shaders walk it to its end. Every clip lists the ancestors it looked
//! at deciding its next; those reverse links reach whatever must be
//! decided again when one of them changes, never a subtree walk.
//!
//! **Revisions.** A clip's revision is the hash of its entry, its parent
//! and its next's revision: it changes when its own rectangle, radii or
//! space does, when any clip of its effective chain does, or when its
//! parent does. Chunk keys and layer contents hold it, and the damage of
//! a clip whose revision changed is what its chain kept before and keeps
//! now, each frame's from its own ancestry ([`Update::damage`]).
//!
//! **Updates.** A frame decides the clips that changed, and those whose
//! looked-at ancestors did, in one pass ordered by depth: a parent is
//! decided before its child, so each clip is decided once, against
//! ancestors already final. Depths come from one walk of the clips met
//! ([`Ancestry`]), each ancestor stepped over once a frame.

use std::collections::BTreeSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::damage::Rect;
use super::path;
use crate::scene::Properties;

/// Floats a clip entry takes (`PCLIP` vec4s in `common.wgsl`).
pub(super) const ENTRY: usize = 20;

pub(super) const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// The scales and turns from transform `id` up to `until`, as one matrix.
pub(super) fn compose(props: &Properties, id: u32, until: u32) -> Option<[f32; 6]> {
    props.matrices(id, until).map(|g| g.matrix()).reduce(path::then)
}

/// The bounding box of `r` (x0, y0, x1, y1) through `m`.
pub(super) fn mapped(r: [f32; 4], m: [f32; 6]) -> [f32; 4] {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in [[r[0], r[1]], [r[2], r[1]], [r[0], r[3]], [r[2], r[3]]] {
        let [x, y] = path::apply(m, p);
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    b
}

pub(super) fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

/// What the renderer knows of one clip.
#[derive(Clone, Debug, Default, PartialEq)]
struct Node {
    entry: [f32; ENTRY],
    transform: u32,
    parent: u32,
    next: u32,
    /// Its own rectangle in the canvas (its bounding box there), and
    /// whether it is square there and axis-aligned.
    own: [f32; 4],
    square: bool,
    aligned: bool,
    /// What its chain keeps, in the canvas (a bounding box).
    bounds: [f32; 4],
    rev: u64,
    /// Clips its walk tests.
    len: u32,
    /// The ancestors it looked at deciding its next.
    looked: Vec<u32>,
}

/// What bringing the tree up to a scene changed.
#[derive(Default)]
pub(super) struct Update {
    /// Clips whose entry changed: written to the table.
    pub written: Vec<u32>,
    /// Clips whose revision changed.
    pub moved: Vec<u32>,
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

#[derive(Default)]
pub(super) struct ClipTree {
    nodes: Vec<Node>,
    /// The clips that looked at each clip deciding their next.
    users: Vec<foldhash::HashSet<u32>>,
    /// The clips in each transform's space, and that space's map to the canvas.
    by_transform: foldhash::HashMap<u32, foldhash::HashSet<u32>>,
    matrices: foldhash::HashMap<u32, Option<[f32; 6]>>,
    scale: u32,
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
    pub(super) fn next(&self, id: u32) -> u32 {
        self.node(id).map_or(0, |n| n.next)
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

    /// A key of clip `id`'s walk as a target sees it whose pixel (0, 0)
    /// lies at `origin` of the space `to_canvas` maps to the canvas: of
    /// each clip, the map from the target's pixels to its own rectangle's
    /// corner, its size, radii and curve. That is what decides the pixels
    /// the walk keeps there, and nothing else is: a clip and a target
    /// moved together key the same; a target turned under a clip that
    /// stays does not. `memo` holds the keys of walks met before for this
    /// target; the walk is followed up to one of them.
    pub(super) fn key_in(
        &self,
        id: u32,
        to_canvas: Option<[f32; 6]>,
        origin: (f32, f32),
        memo: &mut foldhash::HashMap<u32, u64>,
    ) -> u64 {
        let mut walk = Vec::new();
        let mut at = id;
        let mut key = loop {
            if let Some(k) = memo.get(&at) {
                break *k;
            }
            let Some(n) = self.node(at) else { break 0 };
            walk.push(at);
            at = n.next;
        };
        let from = path::then([1.0, 0.0, 0.0, 1.0, origin.0, origin.1], to_canvas.unwrap_or(IDENTITY));
        for c in walk.into_iter().rev() {
            let e = self.entry(c);
            let map = [e[8], e[9], e[10], e[11], e[12], e[13]];
            let m = path::then(path::then(from, map), [1.0, 0.0, 0.0, 1.0, -e[0], -e[1]]);
            let mut h = DefaultHasher::new();
            key.hash(&mut h);
            for v in m.iter().chain(&e[2..8]).chain(std::iter::once(&e[15])) {
                // -0 and 0 keep the same pixels.
                (v + 0.0).to_bits().hash(&mut h);
            }
            key = h.finish().max(1);
            memo.insert(c, key);
        }
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
        let mut dirty: Vec<u32> = clips.to_vec();
        for t in transforms {
            let Some(set) = self.by_transform.get(t) else { continue };
            let now = compose(props, *t, 0);
            if self.matrices.get(t) != Some(&now) {
                dirty.extend(set.iter().copied());
                self.matrices.insert(*t, now);
            }
        }
        // Parents first: a clip waits behind every shallower one.
        let mut up = Ancestry::default();
        let mut queue: BTreeSet<(u32, u32)> = BTreeSet::new();
        for id in dirty {
            if self.node(id).is_some() {
                queue.insert((up.depth(props, id), id));
            }
        }
        let mut out = Update::default();
        let mut done: foldhash::HashSet<u32> = foldhash::HashSet::default();
        // Each clip decided that changed, as it was at the frame's start.
        let mut was: foldhash::HashMap<u32, Node> = foldhash::HashMap::default();
        while let Some((_, id)) = queue.pop_first() {
            if !done.insert(id) {
                continue;
            }
            let Some(old) = self.node(id).cloned() else { continue };
            let parent = up.parent(props, id);
            let new = self.decide(props, s, id, parent, &mut out.work);
            out.decided = out.decided.saturating_add(1);
            if new == old {
                continue;
            }
            self.relink(id, &old, &new);
            let looks_same = new.own == old.own
                && new.square == old.square
                && new.aligned == old.aligned
                && new.bounds == old.bounds
                && new.rev == old.rev
                && new.next == old.next
                && new.len == old.len;
            was.insert(id, old);
            if let Some(slot) = usize::try_from(id).ok().and_then(|i| self.nodes.get_mut(i)) {
                *slot = new;
            }
            if !looks_same && let Some(u) = usize::try_from(id).ok().and_then(|i| self.users.get(i)) {
                for &user in u {
                    if !done.contains(&user) {
                        queue.insert((up.depth(props, user), user));
                    }
                }
            }
        }
        out.work = out.work.saturating_add(up.work).saturating_add(out.decided);
        let mut ids: Vec<u32> = was.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let (Some(old), Some(new)) = (was.get(&id), self.node(id)) else {
                continue;
            };
            self.diff(id, Some(old), new, &mut out);
        }
        out
    }

    fn diff(&self, id: u32, old: Option<&Node>, new: &Node, out: &mut Update) {
        if old.is_none_or(|o| o.entry != new.entry) {
            out.written.push(id);
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

    /// Every clip decided anew, parents first, against what was.
    fn rebuild(&mut self, props: &Properties, s: f32) -> Update {
        let n = props.clips.len();
        let old = std::mem::take(&mut self.nodes);
        self.nodes = vec![Node::default(); n];
        self.users = vec![foldhash::HashSet::default(); n];
        self.by_transform.clear();
        self.matrices.clear();
        self.scale = s.to_bits();
        let mut up = Ancestry::default();
        let mut order: Vec<(u32, u32)> = (1..n)
            .filter_map(|i| u32::try_from(i).ok())
            .map(|id| (up.depth(props, id), id))
            .collect();
        order.sort_unstable();
        let mut out = Update::default();
        for (_, id) in order {
            let parent = up.parent(props, id);
            let node = self.decide(props, s, id, parent, &mut out.work);
            out.decided = out.decided.saturating_add(1);
            self.relink(id, &Node::default(), &node);
            let i = usize::try_from(id).unwrap_or(0);
            self.diff(id, old.get(i), &node, &mut out);
            if let Some(slot) = self.nodes.get_mut(i) {
                *slot = node;
            }
        }
        out.work = out.work.saturating_add(up.work).saturating_add(out.decided);
        out
    }

    /// The reverse links and transform index from `old` to `new` for clip `id`.
    fn relink(&mut self, id: u32, old: &Node, new: &Node) {
        if old.looked != new.looked {
            for a in &old.looked {
                if let Some(u) = usize::try_from(*a).ok().and_then(|a| self.users.get_mut(a)) {
                    u.remove(&id);
                }
            }
            for a in &new.looked {
                if let Some(u) = usize::try_from(*a).ok().and_then(|a| self.users.get_mut(a)) {
                    u.insert(id);
                }
            }
        }
        if old.transform != new.transform || old == &Node::default() {
            if let Some(set) = self.by_transform.get_mut(&old.transform) {
                set.remove(&id);
            }
            self.by_transform.entry(new.transform).or_default().insert(id);
        }
    }

    /// Clip `id` under `parent` as the tree's other clips have it now,
    /// the ancestors it looks at counted into `work`.
    fn decide(&mut self, props: &Properties, s: f32, id: u32, parent: u32, work: &mut usize) -> Node {
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
        let (map, scale, own) = match fwd {
            None => (IDENTITY, 1.0, local),
            Some(g) => (path::invert(g).unwrap_or([0.0; 6]), path::scales(g).0, mapped(local, g)),
        };
        let aligned = fwd.is_none_or(|g| g[1] == 0.0 && g[2] == 0.0);
        let square = radii.iter().all(|r| *r <= 0.0);
        // Its next: the first of its parent's walk it needs.
        let mut looked = Vec::new();
        let mut at = parent;
        while let Some(a) = self.node(at) {
            if looked.len() > self.nodes.len() {
                at = 0;
                break;
            }
            looked.push(at);
            *work = work.saturating_add(1);
            if crate::clip::needed(a.own, a.square, a.aligned, own) {
                break;
            }
            at = a.next;
        }
        let next = at;
        let (bounds, len, up) = match self.node(next) {
            Some(n) => (intersect(own, n.bounds), n.len.saturating_add(1), n.rev),
            None => (own, 1, 0),
        };
        let mut entry = [0.0; ENTRY];
        entry[..4].copy_from_slice(&rect);
        entry[4..8].copy_from_slice(&radii);
        entry[8..14].copy_from_slice(&map);
        entry[14] = scale;
        entry[15] = c.clip.curve;
        entry[16] = f32::from_bits(next);
        let mut h = DefaultHasher::new();
        for v in entry {
            v.to_bits().hash(&mut h);
        }
        (parent, up).hash(&mut h);
        Node {
            entry,
            transform: c.transform,
            parent,
            next,
            own,
            square,
            aligned,
            bounds,
            rev: h.finish().max(1),
            len,
            looked,
        }
    }
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
/// stops at the root or at a clip already known. A clip whose parents
/// never reach the root (a cycle, which the scene must not have) hangs
/// from the root.
#[derive(Default)]
struct Ancestry {
    /// Each clip met: its depth by its parents, or `None` when they never
    /// reach the root.
    reach: foldhash::HashMap<u32, Option<u32>>,
    walking: foldhash::HashSet<u32>,
    /// Clips stepped over.
    work: usize,
}

impl Ancestry {
    fn reach(&mut self, props: &Properties, id: u32) -> Option<u32> {
        let mut path = Vec::new();
        self.walking.clear();
        let mut at = id;
        let mut d = loop {
            if at == 0 {
                break Some(0);
            }
            if let Some(d) = self.reach.get(&at) {
                break *d;
            }
            if !self.walking.insert(at) {
                break None;
            }
            self.work = self.work.saturating_add(1);
            path.push(at);
            at = parent_of(props, at);
        };
        for p in path.into_iter().rev() {
            d = d.map(|d| d.saturating_add(1));
            self.reach.insert(p, d);
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
                let mut at = fr.pclip(&tree, 5000, 0, || None);
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
}
