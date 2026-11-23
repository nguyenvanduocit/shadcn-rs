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
    /// A thumb's transform and its offset at the top plus how far it is along.
    thumbs: Vec<(u32, (f32, f32))>,
    /// A moving effect's opacity (the last moving node naming it wins).
    opacity: foldhash::HashMap<u32, f32>,
    pub(super) moved: Vec<(NodeKey, (f32, f32))>,
    pub(super) turned: Vec<(NodeKey, crate::style::Painted)>,
    pub(super) scroll: Vec<ShownScroll>,
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
        self.scroll.clear();
    }
}

impl Compositor {
    /// What it moves at `at`, written into the sources the last frame
    /// left (their memory kept).
    pub(super) fn sources(&mut self, at: f32) -> Sources {
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
        src.scroll.reserve(scene.compositing.scrollers.len());
        for (i, s) in scene.compositing.scrollers.iter().enumerate() {
            let offset = self.scrolls.offset(s, u32::try_from(i).unwrap_or(u32::MAX));
            let snapped = crate::geometry::device_snapped(offset, scene.scale);
            let applied = (if s.rtl { snapped.0 } else { -snapped.0 }, -snapped.1);
            let d = (applied.0 - s.applied.0, applied.1 - s.applied.1);
            if d != (0.0, 0.0) && usize::try_from(s.content).is_ok_and(|c| c < n) {
                let o = src.own.entry(s.content).or_default();
                o.0 += d.0;
                o.1 += d.1;
            }
            src.scroll.push(ShownScroll {
                key: s.key,
                owner: s.owner,
                offset,
                applied,
            });
            if let Some(t) = s.thumb
                && s.max.1 > 0.0
            {
                let along = t.travel * (offset.1 / s.max.1);
                src.thumbs.push((t.transform, (t.top.0, t.top.1 + along)));
                // Shown only while it is: at rest it stays as committed (hidden).
                let shown = self.scrolls.thumb_opacity(s, at);
                if shown > 0.0 && usize::try_from(t.effect).is_ok_and(|e| e < props.effects.len()) {
                    src.opacity.insert(t.effect, shown);
                }
            }
        }
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
    for &(id, top) in &src.thumbs {
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
    /// Compose the frame at `at` into `shown`, writing what the scroll,
    /// motion and commits since the last frame reach, and noting in
    /// `changed` what of it differs from what was shown.
    pub(super) fn compose_into_shown(&mut self, at: f32) {
        // The frame on screen until this one is, kept as what this compose
        // overwrites of it (a wheel that came while it was is routed in
        // it): the work is what changed, not the whole tables.
        let mut was = if self.ring.len() >= super::ROUTED - 1 {
            self.ring.pop_back().unwrap_or_default()
        } else {
            Box::default()
        };
        was.frame = self.presented.frame;
        was.scale = self.shown.scale;
        was.compositing.clone_from(&self.shown.compositing);
        was.transforms.clear();
        was.clips.clear();
        was.whole = None;
        let mut src = self.sources(at);
        let (a, b) = (&self.shown.properties, &self.scene.properties);
        let reshaped = a.transforms.len() != b.transforms.len()
            || a.clips.len() != b.clips.len()
            || a.effects.len() != b.effects.len()
            || self.shown.chunks.len() != self.scene.chunks.len();
        if std::mem::take(&mut self.pending.all) || reshaped {
            self.pending.clear();
            self.links = Links::of(&self.scene.properties);
            was.whole = Some(std::mem::replace(&mut self.shown.properties, whole(&self.scene, &src)));
            self.shown.chunks.clone_from(&self.scene.chunks);
            self.changed.all = true;
        } else {
            self.compose_changed(&src, &mut was);
        }
        if self.presented.frame != 0 {
            self.ring.push_front(was);
        }
        let scene = &self.scene;
        self.shown.scale = scene.scale;
        self.shown.size = scene.size;
        self.shown.background = scene.background;
        self.shown.compositing.clone_from(&scene.compositing);
        let moved = &mut self.moved;
        moved.transforms.clear();
        moved.transforms.extend(
            src.own
                .keys()
                .chain(src.set.keys())
                .copied()
                .chain(src.thumbs.iter().map(|t| t.0)),
        );
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
        let (frame, commit) = (self.frame.saturating_add(1), self.commit);
        // The frame shown two before, if nothing holds it any more (the
        // render thread hands the UI the one before until this one is
        // drawn; no wheel under it is under way), becomes this one; its
        // lists are the next frame's sources.
        let next = if let Some(mut old) = self.retired.take()
            && let Some(p) = std::sync::Arc::get_mut(&mut old)
        {
            p.frame = frame;
            p.commit = commit;
            p.present_at = at;
            std::mem::swap(&mut p.scroll, &mut src.scroll);
            std::mem::swap(&mut p.moved, &mut src.moved);
            std::mem::swap(&mut p.turned, &mut src.turned);
            old
        } else {
            std::sync::Arc::new(super::Presented {
                frame,
                commit,
                present_at: at,
                scroll: std::mem::take(&mut src.scroll),
                moved: std::mem::take(&mut src.moved),
                turned: std::mem::take(&mut src.turned),
            })
        };
        self.retired = Some(std::mem::replace(&mut self.presented, next));
        self.spare = src;
    }

    /// Write what `src`, the last frame's motion and the commits since reach.
    #[expect(clippy::too_many_lines, reason = "transforms, clips, effects, chunks, in order")]
    fn compose_changed(&mut self, src: &Sources, was: &mut super::Before) {
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
        w.stack.extend(src.thumbs.iter().map(|t| t.0));
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
        let thumb = |t: u32| src.thumbs.iter().rev().find(|(id, _)| *id == t).map(|(_, top)| *top);
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
                was.transforms.push((t, *slot));
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
                was.clips.push((c, *slot));
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
