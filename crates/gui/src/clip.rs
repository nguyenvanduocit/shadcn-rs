//! What a clip keeps, on the CPU, as the shaders keep it: one rule for the
//! renderer's clip chains and for hit testing.
//!
//! A clip is a rounded rectangle in its own space; a clip chain is a clip
//! and those it lies in, each tested in its own space and their coverages
//! multiplied (Chrome's clip-node tree; Flutter's nested `ClipRRect`s).
//! The chain the shaders walk is the *effective* one: an ancestor that
//! keeps the whole of every pixel the clip covers ([`holds`]) is left out
//! of it, which changes no pixel; hit testing walks the whole chain.
//! [`coverage_at`] is `clip_entry_cov` of `gpu/common.wgsl`, ported: what
//! paints a pixel, and so what a point hits. Coverage is measured in the
//! pixels of the target drawn into (the canvas's device pixels, or a
//! layer's texels), whatever scales the clip's own space.

use crate::geometry::Bounds;
use crate::layout::FrameNode;
use crate::style::Corners;

/// How much of the pixel centred on `p` the box `rect` (x, y, width,
/// height) with used radii `radii` (top left, top right, bottom right,
/// bottom left) and corners `superellipse(k)` covers (`shape_cov`).
#[must_use]
pub(crate) fn coverage(rect: [f32; 4], radii: [f32; 4], k: f32, p: [f32; 2]) -> f32 {
    if rect[2] <= 0.0 || rect[3] <= 0.0 {
        return 0.0;
    }
    if k == 1.0 {
        let most = radii.iter().copied().fold(0.0, f32::max);
        if most <= 0.0 {
            // An axis-aligned rectangle covers the pixel's area exactly.
            let span =
                |c: f32, lo: f32, len: f32| ((c + 0.5).clamp(lo, lo + len) - (c - 0.5).clamp(lo, lo + len)).max(0.0);
            return span(p[0], rect[0], rect[2]) * span(p[1], rect[1], rect[3]);
        }
        return (0.5 - rrect_sdf(p, rect, radii)).clamp(0.0, 1.0);
    }
    (0.5 - shape_sdf(p, rect, radii, k)).clamp(0.0, 1.0)
}

fn pick_corner(c: [f32; 2], radii: [f32; 4]) -> f32 {
    match (c[0] < 0.0, c[1] < 0.0) {
        (true, true) => radii[0],
        (true, false) => radii[3],
        (false, true) => radii[1],
        (false, false) => radii[2],
    }
}

fn rrect_sdf(p: [f32; 2], rect: [f32; 4], radii: [f32; 4]) -> f32 {
    let half = [rect[2] / 2.0, rect[3] / 2.0];
    let c = [p[0] - (rect[0] + half[0]), p[1] - (rect[1] + half[1])];
    let corner = [c[0].abs() - half[0], c[1].abs() - half[1]];
    let most = radii.iter().copied().fold(0.0, f32::max);
    if most <= half[0].min(half[1]) {
        let r = pick_corner(c, radii);
        let q = [corner[0] + r, corner[1] + r];
        if r <= 0.0 {
            return q[0].max(q[1]);
        }
        return q[0].max(0.0).hypot(q[1].max(0.0)) + q[0].max(q[1]).min(0.0) - r;
    }
    shape_sdf(p, rect, radii, 1.0)
}

fn corner_sdf(q: [f32; 2], r: f32, k: f32) -> f32 {
    if k == 1.0 {
        return (r - q[0]).hypot(r - q[1]) - r;
    }
    let n = crate::shape::curve_exponent(k);
    let norm = |a: [f32; 2]| {
        let g = (a[0].powf(n) + a[1].powf(n)).powf(1.0 / n).max(1e-6);
        let grad = a[0].powf(n - 1.0).hypot(a[1].powf(n - 1.0)) / g.powf(n - 1.0);
        (g, grad.max(1e-6))
    };
    if k > 0.0 {
        let (g, grad) = norm([((r - q[0]) / r).max(1e-6), ((r - q[1]) / r).max(1e-6)]);
        return r * (g - 1.0) / grad;
    }
    let (g, grad) = norm([(q[0] / r).max(1e-6), (q[1] / r).max(1e-6)]);
    r * (1.0 - g) / grad
}

fn shape_sdf(p: [f32; 2], rect: [f32; 4], radii: [f32; 4], k: f32) -> f32 {
    let half = [rect[2] / 2.0, rect[3] / 2.0];
    let d = [
        (p[0] - (rect[0] + half[0])).abs() - half[0],
        (p[1] - (rect[1] + half[1])).abs() - half[1],
    ];
    let mut sdf = d[0].max(0.0).hypot(d[1].max(0.0)) + d[0].max(d[1]).min(0.0);
    let (lo, hi) = ([rect[0], rect[1]], [rect[0] + rect[2], rect[1] + rect[3]]);
    let qs = [
        [p[0] - lo[0], p[1] - lo[1]],
        [hi[0] - p[0], p[1] - lo[1]],
        [hi[0] - p[0], hi[1] - p[1]],
        [p[0] - lo[0], hi[1] - p[1]],
    ];
    for (r, q) in radii.into_iter().zip(qs) {
        if r > 0.0 && q[0] < r && q[1] < r {
            sdf = sdf.max(corner_sdf(q, r, k));
        }
    }
    sdf
}

fn curved_sdf(p: [f32; 2], rect: [f32; 4], radii: [f32; 4], k: f32) -> f32 {
    if k == 1.0 {
        return rrect_sdf(p, rect, radii);
    }
    shape_sdf(p, rect, radii, k)
}

/// How much of a target's pixel a clip keeps, the pixel's centre at `q` in
/// the clip's own space (physical): `clip_entry_cov` of `gpu/common.wgsl`,
/// ported, the one rule for paint and hit testing. `scale` is the larger
/// scale from the clip's space into the target's pixels, `aligned` whether
/// that map keeps axes: where both are one to one, by the pixel's area;
/// else by distance, measured in the target's pixels (one pixel wide).
/// A clip of no width or height keeps nothing, by area or by distance.
#[must_use]
pub(crate) fn coverage_at(rect: [f32; 4], radii: [f32; 4], k: f32, q: [f32; 2], scale: f32, aligned: bool) -> f32 {
    if rect[2] <= 0.0 || rect[3] <= 0.0 {
        return 0.0;
    }
    if scale == 1.0 && aligned {
        return coverage(rect, radii, k, q);
    }
    (0.5 - curved_sdf(q, rect, radii, k) * scale).clamp(0.0, 1.0)
}

/// [`coverage_at`] of a clip entry (`rect`, `radii`, curve `k`, `map` from
/// the target into its space, `scale`) at the target's point `p`.
#[cfg(test)]
#[must_use]
pub(crate) fn entry_coverage(rect: [f32; 4], radii: [f32; 4], k: f32, map: [f32; 6], scale: f32, p: [f32; 2]) -> f32 {
    coverage_at(rect, radii, k, apply(map, p), scale, map[1] == 0.0 && map[2] == 0.0)
}

fn apply(m: [f32; 6], p: [f32; 2]) -> [f32; 2] {
    [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]
}

/// Four points of the target (the canvas's device pixels) whose hull holds
/// the centre of every pixel the clip of rectangle `local` (x0, y0, x1, y1,
/// its own space, physical) covers at all, `fwd` mapping that space to the
/// target, `scale` its larger scale: what [`holds`] is asked of. Under an
/// axis-aligned map, the pixels it touches exactly (`shape_cov` is zero
/// past half a pixel of its edges, by area or by distance); turned, its
/// rectangle grown by half a device pixel.
#[must_use]
pub(crate) fn reach(local: [f32; 4], fwd: [f32; 6], scale: f32) -> [[f32; 2]; 4] {
    let corners = |r: [f32; 4]| [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
    if fwd[1] == 0.0 && fwd[2] == 0.0 {
        let [a, b] = [apply(fwd, [local[0], local[1]]), apply(fwd, [local[2], local[3]])];
        // The centres of the pixels it covers any of.
        let lo = |v: f32, w: f32| v.min(w).floor() + 0.5;
        let hi = |v: f32, w: f32| v.max(w).ceil() - 0.5;
        return corners([lo(a[0], b[0]), lo(a[1], b[1]), hi(a[0], b[0]), hi(a[1], b[1])]);
    }
    let g = 0.5 / scale.max(1e-6);
    corners([local[0] - g, local[1] - g, local[2] + g, local[3] + g]).map(|p| apply(fwd, p))
}

/// Whether a clip keeps the whole of every pixel whose centre lies in the
/// hull of `points` (the target's, [`reach`]): each lies inside its
/// rectangle `rect` (x, y, width, height, its own space, physical) less
/// its corners and half a device pixel of its edges, mapped there by `map`
/// (`scale`, its larger scale out of that space). `entry_holds` in
/// `gpu/shaders.wgsl` is this rule; there `shape_cov` is 1 exactly. No
/// slack: a point a hair short of that is not kept whole, so leaving
/// clips out never adds up to a coverage the whole chain does not keep.
///
/// The one rule of the effective chain: an ancestor that keeps the whole
/// of every pixel a clip covers is left out of its chain, and the chain
/// keeps what it would keep with it (its coverage there is 1). So whatever
/// is left out, hit testing walking every clip keeps what paint keeps.
#[must_use]
pub(crate) fn holds(rect: [f32; 4], radii: [f32; 4], map: [f32; 6], scale: f32, points: &[[f32; 2]; 4]) -> bool {
    let s = scale.max(1e-6);
    let corner = radii.iter().copied().fold(0.0_f32, f32::max);
    let m = corner.max(0.5 / s);
    let (lo, hi) = (
        [rect[0] + m, rect[1] + m],
        [rect[0] + rect[2] - m, rect[1] + rect[3] - m],
    );
    points.iter().all(|&p| {
        let q = apply(map, p);
        q[0] >= lo[0] && q[1] >= lo[1] && q[0] <= hi[0] && q[1] <= hi[1]
    })
}

/// Clip's reader of laid-out geometry ([`crate::layout::Sight`]).
#[derive(Clone, Copy)]
pub(crate) struct Sight(());

/// The box clipper `owner` clips what lies in it to: its padding box (as
/// Chrome clips `overflow` to the padding box). Its corners are those its
/// clipped nodes carry ([`FrameNode::clip_radius`]).
#[must_use]
pub(crate) fn clip_box(owner: &FrameNode) -> Bounds {
    let w = owner.look.border;
    let bounds = owner.bounds(Sight(()));
    if w.top > 0.0 || w.right > 0.0 || w.bottom > 0.0 || w.left > 0.0 {
        bounds.inset(w.top, w.right, w.bottom, w.left)
    } else {
        bounds
    }
}

/// The centre of the device pixel logical `v` lies in, at `scale` device
/// pixels to the logical one: what the screen shows there.
fn device_centre(v: f32, scale: f32) -> f32 {
    ((v * scale).floor() + 0.5) / scale
}

/// How much of a target's pixel one clip keeps, as the shaders keep it:
/// its box `rect` (logical, its own space) rounded `radius` with corners
/// `curve`, snapped to the device pixels as the renderer's entry has it
/// (its radii as used of the box as laid out), the pixel's centre at `q`
/// in its own space (logical), at `s` device pixels to the logical one,
/// measured in the target's pixels through `into`: the larger scale of
/// the clip's map into them and whether it keeps axes ([`coverage_at`];
/// a clip in a group scaled twice fades over half its own pixel).
fn link_coverage(rect: Bounds, radius: Corners, curve: f32, q: (f32, f32), s: f32, into: (f32, bool)) -> f32 {
    let b = rect.snapped(s);
    let r = crate::shape::Shape::Rounded(radius).used(rect.width, rect.height);
    let radii = [r.top_left, r.top_right, r.bottom_right, r.bottom_left].map(|v| v * s);
    coverage_at(
        [b.x * s, b.y * s, b.width * s, b.height * s],
        radii,
        curve,
        [q.0 * s, q.1 * s],
        into.0,
        into.1,
    )
}

/// The node a point of the screen hits in the frame `presented` showed:
/// the topmost of its boxes that take the pointer ([`crate::compositor::Hit`])
/// whose shape holds the point and whose pixel there the screen shows at
/// least half of, and that `accept` takes (its node, by path and by the
/// component instance that made it; a box `accept` refuses is passed over
/// for the one under it). `point` is logical, on screen. The one hit rule
/// for a click, the devtools' pick and a wheel gesture's first target
/// ([`HitView::hit`]); before any frame is presented, the frame as the UI
/// painted it, with no layer drawn. The node is named by both: one another
/// instance made at its path since is another node.
pub fn hit_in(
    presented: &crate::compositor::Presented,
    point: (f32, f32),
    accept: impl Fn(crate::compositor::HitTarget) -> bool,
) -> Option<crate::compositor::HitTarget> {
    HitView::of(presented, None)
        .hit(point, |h| accept(h.target))
        .map(|h| h.target)
}

/// One frame's hit-test data, as it was shown: its boxes that take the
/// pointer, its property trees as composed, the layers it was drawn with
/// and its scale (device pixels to the logical one). Every hit decision
/// reads these of one frame together, never one frame's layers with
/// another's clips.
pub(crate) struct HitView<'a> {
    pub hits: &'a [crate::compositor::Hit],
    /// Where each box of `hits` can take the pointer, by space ([`HitIndex`]).
    pub index: std::sync::Arc<HitIndex>,
    pub props: &'a crate::scene::Properties,
    pub stages: &'a [crate::scene::LayerStage],
    pub scale: f32,
    /// Counts the boxes whose clip chain is walked (the compositor's
    /// cost tests).
    pub walks: Option<&'a std::cell::Cell<usize>>,
    /// Which frame it is: where its spaces lie on the canvas is worked
    /// out once for it ([`HitIndex::spaces_at`]).
    pub frame: FrameKey,
    /// The window it was drawn for, logical (`(0, 0)`: not known): where
    /// points fall, what the space tree's cost is weighed over.
    pub viewport: (f32, f32),
}

/// The identity map.
const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[cfg(test)]
crate::engine::engine_local! {
    /// Transform chains composed for hit testing on this thread (a cost
    /// test reads it).
    [Fork] static COMPOSED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Transform chains composed for hit testing on this thread so far.
#[cfg(test)]
pub(crate) fn composed() -> u64 {
    COMPOSED.with(std::cell::Cell::get)
}

#[cfg(test)]
crate::engine::engine_local! {
    /// Candidates a hit examined on this thread: every node of an index
    /// and every entry whose bounds it tested (a cost test reads it).
    [Fork] static CANDIDATES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Spaces placed on the canvas for a frame on this thread (a cost
    /// test reads it: once a frame, not once a hit).
    [Fork] static PLACED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Candidates hits examined on this thread so far ([`CANDIDATES`]).
#[cfg(test)]
pub(crate) fn candidates() -> u64 {
    CANDIDATES.with(std::cell::Cell::get)
}

/// Spaces placed for frames on this thread so far ([`PLACED`]).
#[cfg(test)]
pub(crate) fn placed() -> u64 {
    PLACED.with(std::cell::Cell::get)
}

/// One more candidate examined.
fn candidate() {
    #[cfg(test)]
    CANDIDATES.with(|n| n.set(n.get().saturating_add(1)));
}

/// Transform `id`'s scales and turns up to, not through, `until`, the
/// innermost first, as the renderer composes them (`compose` in
/// `gpu/clips.rs`); `None` when there are none.
fn compose(props: &crate::scene::Properties, id: u32, until: u32) -> Option<[f32; 6]> {
    #[cfg(test)]
    COMPOSED.with(|n| n.set(n.get().saturating_add(1)));
    props
        .matrices(id, until)
        .map(|g| g.matrix())
        .reduce(crate::gpu::path::then)
}

/// Point `p` (physical) of the target space through the inverse of `m`
/// (`None`: the identity; the scene's matrices are in physical pixels),
/// in logical pixels at `s` device pixels to the logical one, with the
/// larger scale of `m` and whether it keeps axes: a clip's in its own
/// space, measured in the target's pixels as its renderer's entry
/// measures it. `None` under a scale of 0.
fn undone(m: Option<[f32; 6]>, p: (f32, f32), s: f32) -> Option<((f32, f32), (f32, bool))> {
    let Some(m) = m else {
        return Some(((p.0 / s, p.1 / s), (1.0, true)));
    };
    let q = apply(crate::gpu::path::invert(m)?, [p.0, p.1]);
    Some((
        (q[0] / s, q[1] / s),
        (crate::gpu::path::scales(m).0, m[1] == 0.0 && m[2] == 0.0),
    ))
}

impl<'a> HitView<'a> {
    /// What `presented` showed.
    pub(crate) fn of(presented: &'a crate::compositor::Presented, walks: Option<&'a std::cell::Cell<usize>>) -> Self {
        Self {
            hits: &presented.hits,
            index: HitIndex::of(&presented.hits),
            props: &presented.properties,
            stages: &presented.stages,
            scale: presented.scale,
            walks,
            frame: FrameKey::of(presented),
            viewport: presented.viewport,
        }
    }

    /// The topmost box at `at` (logical) that takes the pointer there and
    /// that `accept` takes ([`hit_in`]). Only the spaces whose bounds on
    /// the canvas hold the point are mapped into ([`HitIndex::spaces_at`]),
    /// and in each only the boxes whose rectangle holds it there are
    /// candidates ([`Rtree::query`]): a hit costs the logarithm of the
    /// spaces and of the boxes, and the boxes under the point, never a
    /// walk of the table. `accept` is asked next: a box it passes over
    /// costs no shape or clip-coverage measure.
    pub(crate) fn hit(
        &self,
        at: (f32, f32),
        accept: impl Fn(&crate::compositor::Hit) -> bool,
    ) -> Option<&'a crate::compositor::Hit> {
        self.hit_place(at, accept).map(|(_, h)| h)
    }

    /// [`Self::hit`], with the box's place in the table.
    pub(crate) fn hit_place(
        &self,
        at: (f32, f32),
        accept: impl Fn(&crate::compositor::Hit) -> bool,
    ) -> Option<(u32, &'a crate::compositor::Hit)> {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        let hits = self.hits;
        let index = &*self.index;
        let (mut spaces, mut boxes) = HIT_SCRATCH.with(|c| std::mem::take(&mut *c.borrow_mut()));
        spaces.clear();
        index.spaces_at(self.frame, self.props, s, self.viewport, at, &mut spaces);
        // The spaces holding the topmost boxes first: one below the best
        // found yet is passed over whole.
        spaces.sort_unstable_by_key(|&k| std::cmp::Reverse(index.space(k).map_or(0, |sp| sp.top)));
        let mut best: Option<u32> = None;
        for &k in &spaces {
            let Some(space) = index.space(k) else { continue };
            if best.is_some_and(|b| space.top <= b) {
                continue;
            }
            let Some((p, _)) = undone(compose(self.props, space.transform, 0), (at.0 * s, at.1 * s), s) else {
                continue;
            };
            let off = self.props.transform(space.transform).offset;
            let q = (p.0 - off.0, p.1 - off.1);
            boxes.clear();
            space.boxes.query(q, best, |i| index.bounds_of(i), &mut boxes);
            for &i in &space.loose {
                candidate();
                if best.is_none_or(|b| i > b) {
                    boxes.push(i);
                }
            }
            // The topmost first, above the best yet.
            boxes.sort_unstable_by(|a, b| b.cmp(a));
            for &i in &boxes {
                if let Some(h) = usize::try_from(i).ok().and_then(|i| hits.get(i))
                    && accept(h)
                    && self.takes(h, at)
                {
                    best = Some(i);
                    break;
                }
            }
        }
        HIT_SCRATCH.with(|c| *c.borrow_mut() = (spaces, boxes));
        best.and_then(|i| Some((i, hits.get(usize::try_from(i).ok()?)?)))
    }

    /// Where `at` (logical, on the canvas) lies in box `h`, from the corner
    /// of its rectangle: mapped into its space as [`Self::hit`] maps it
    /// (every scale and turn above it undone, then its offset); `None`
    /// under a scale of 0.
    pub(crate) fn local(&self, h: &crate::compositor::Hit, at: (f32, f32)) -> Option<(f32, f32)> {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        let (p, _) = undone(compose(self.props, h.transform, 0), (at.0 * s, at.1 * s), s)?;
        let off = self.props.transform(h.transform).offset;
        Some((p.0 - off.0 - h.rect.x, p.1 - off.1 - h.rect.y))
    }

    /// Whether box `h` takes the pointer at `at`: its shape holds the
    /// point where it shows (each scale and turn above it undone; under a
    /// scale of 0 nothing does), and its clip chain keeps at least half of
    /// the device pixel the point lies in, as the screen showed it
    /// ([`Self::keeps`]).
    fn takes(&self, h: &crate::compositor::Hit, at: (f32, f32)) -> bool {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        let Some((p, _)) = undone(compose(self.props, h.transform, 0), (at.0 * s, at.1 * s), s) else {
            return false;
        };
        let t = self.props.transform(h.transform);
        let b = h.rect.translated(t.offset.0, t.offset.1);
        if !crate::shape::takes_pointer(&h.shape, b, h.slop, p.0, p.1) {
            return false;
        }
        if h.clip == 0 {
            return true;
        }
        if let Some(w) = self.walks {
            w.set(w.get().saturating_add(1));
        }
        self.keeps(h, at).is_some_and(|k| k >= 0.5)
    }

    /// The stage of the layer of effect `e`, when one was drawn.
    fn stage(&self, e: u32) -> Option<&'a crate::scene::LayerStage> {
        let i = self.stages.binary_search_by_key(&e, |s| s.effect).ok()?;
        self.stages.get(i)
    }

    /// How much of the canvas's device pixel `at` lies in shows box `h`,
    /// as the renderer made that pixel: the layers it was drawn through
    /// whose composite resamples ([`crate::scene::LayerStage::resamples`]),
    /// inner to outer, each a level; the clips of its chain inside the
    /// innermost cut its texels, its composite samples four of them into
    /// the next level as its bilinear filter weighs them ([`resampled`]),
    /// the clips from its output clip on to the next's cut that level's
    /// texels, and so on out to the canvas. A layer that does not resample
    /// adds no level: its texels are its target's pixels. Every level is
    /// followed, however deep (a few are kept without allocating), and
    /// each texel of a level is measured once however many samples of
    /// the levels outside read it ([`Levelled::shown`]).
    fn keeps(&self, h: &crate::compositor::Hit, at: (f32, f32)) -> Option<f32> {
        self.keeps_with(h, at, true).0
    }

    /// [`Self::keeps`], each texel measured once when `memo` (else again
    /// for each sample that reads it), and how many were measured.
    fn keeps_with(&self, h: &crate::compositor::Hit, at: (f32, f32), memo: bool) -> (Option<f32>, usize) {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        let mut levels = Levels::default();
        // The innermost layer drawn holding its effect, then each it is
        // composited into.
        let mut e = h.effect;
        let mut steps = 0_usize;
        let limit = self.props.effects.len().saturating_add(self.stages.len());
        let mut stage = None;
        while e != 0 && steps <= limit {
            steps = steps.saturating_add(1);
            if let Some(st) = self.stage(e) {
                stage = Some(st);
                break;
            }
            let Some(fx) = self.props.effect(e) else {
                return (None, 0);
            };
            e = fx.parent;
        }
        while let Some(st) = stage {
            steps = steps.saturating_add(1);
            if steps > limit {
                break;
            }
            if st.resamples() {
                levels.push(st);
            }
            stage = self.stage(st.outer).filter(|_| st.outer != 0);
        }
        let chain = Levelled {
            view: self,
            h,
            levels: levels.as_slice(),
            s,
            memo: memo.then(|| crate::engine::WorldRefCell::new("a hit's measured texels", Measured::default())),
            visits: crate::engine::WorldCell::new("a hit's measured points", 0),
        };
        let p = (device_centre(at.0, s) * s, device_centre(at.1, s) * s);
        let kept = chain.shown(levels.as_slice().len(), p);
        (kept, chain.visits.get())
    }
}

/// Where each box of a hit table can take the pointer, at two levels, so
/// a hit costs `O(log N + k)` (`N` boxes or spaces, `k` of them under the
/// point), never a walk of the table:
///
/// - In each transform space boxes are hit in, an R-tree of the boxes'
///   rectangles (grown by their slop) in that space, before its offset and
///   its scales and turns ([`HitSpace::boxes`]). The compositor's scroll
///   and motion change only offsets and matrices, so it holds across every
///   frame the hit table is shared by and is made once per table
///   ([`Self::of`]), at the first hit, by the thread that hits.
/// - Over the spaces, an R-tree of each space's bounds on the canvas, so a
///   hit maps the point into, and composes the chain of, only the spaces
///   whose bounds hold it ([`Self::spaces_at`]). Those bounds move with
///   every scroll and motion: the tree's bounds are refit to each frame's
///   property trees, once a frame (`O(S)` for `S` spaces, the BVH
///   refitting of Wald, Boulos and Shirley, "Ray tracing deformable scenes
///   using dynamic bounding volume hierarchies", 2007), not once a hit,
///   and its shape is packed again when it no longer fits the frame
///   ([`SpaceTree::fits`]): a space came onto or left the canvas, or the
///   refit's expected cost of a point in the window grew past twice the
///   packed tree's ([`expected_cost`]; the refit-or-rebuild structure of
///   Lauterbach et al., "RT-DEFORM", 2006, with a cost and a threshold of
///   our own). Spaces scrolled together move rigidly and keep the shape.
///
/// Both are packed by Sort-Tile-Recursive (Leutenegger, Lopez and
/// Edgington, "STR: a simple and efficient algorithm for R-tree packing",
/// 1997; the R-tree is Guttman's, 1984): near-full nodes of [`FANOUT`]
/// entries that overlap little, so a point visits about one node a level.
/// `takes` stays the rule: the index only passes over boxes that cannot
/// take the point, and finds the rest in paint order, the topmost first.
#[derive(Debug, Default)]
pub(crate) struct HitIndex {
    spaces: Vec<HitSpace>,
    /// Each box's rectangle grown by its slop, in its space
    /// `[left, top, right, bottom]`.
    bounds: Vec<[f32; 4]>,
    /// The spaces on the canvas, for the frames hit last ([`Placement`]).
    placed: std::sync::Mutex<Placed>,
}

/// The boxes hit in one transform space.
#[derive(Debug)]
struct HitSpace {
    transform: u32,
    /// Its boxes with a finite rectangle, by where they lie in the space.
    boxes: Rtree,
    /// Its boxes without one: a candidate for every point.
    loose: Vec<u32>,
    /// The extent of `boxes` in the space, `None` when it has none.
    extent: Option<[f32; 4]>,
    /// Its topmost box (the last in paint order).
    top: u32,
}

/// The spaces of a hit table on the canvas, as frames placed them: the
/// space tree packed last, and the last few frames' bounds and the tree
/// each was refit to.
#[derive(Debug, Default)]
struct Placed {
    /// The space tree packed last, which the next frame is refit to while
    /// it fits.
    shape: Option<std::sync::Arc<SpaceTree>>,
    frames: std::collections::VecDeque<Placement>,
}

/// The tree of the spaces on the canvas, as packed from one frame.
#[derive(Debug)]
struct SpaceTree {
    tree: Rtree,
    /// Which spaces it holds: those whose bounds on the canvas were finite.
    members: Vec<bool>,
    /// Its expected cost of a point in the window when packed, and the
    /// window it was weighed over ([`expected_cost`]).
    cost: f64,
    viewport: (f32, f32),
}

/// One frame's spaces on the canvas.
#[derive(Debug)]
struct Placement {
    key: FrameKey,
    /// Each space's bounds on the canvas (logical), grown by [`MARGIN`].
    bounds: Vec<[f32; 4]>,
    /// Spaces a point is always mapped into: ones with a box without a
    /// finite rectangle, or whose bounds on the canvas are not finite.
    always: Vec<u32>,
    /// The space tree this frame is placed in, and its levels refit to
    /// `bounds` (a frame packed anew keeps the older frames' trees).
    shape: std::sync::Arc<SpaceTree>,
    levels: Vec<Vec<RNode>>,
}

/// Which frame placed the spaces: its number and its commit (a snapshot
/// is never changed once published and its number is its own, given as
/// it is published; before the first present, the snapshot of a commit
/// is numbered 0 and named by its commit), and, beside them, its
/// transforms' buffer and length and its scale.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FrameKey {
    frame: u64,
    commit: u64,
    buffer: usize,
    len: usize,
    scale: u32,
}

impl FrameKey {
    /// The key of `presented`.
    fn of(presented: &crate::compositor::Presented) -> Self {
        Self {
            frame: presented.frame,
            commit: presented.commit,
            ..Self::props(&presented.properties, presented.scale)
        }
    }

    /// The key of property trees `props` at `scale`, numbered nothing (a
    /// view made by hand).
    fn props(props: &crate::scene::Properties, scale: f32) -> Self {
        Self {
            frame: 0,
            commit: 0,
            buffer: props.transforms.as_ptr().addr(),
            len: props.transforms.len(),
            scale: scale.to_bits(),
        }
    }
}

/// Frames whose placement a hit table keeps (the frames in flight).
const PLACEMENTS: usize = 2;
/// Entries a node of an R-tree holds at most.
const FANOUT: usize = 16;
/// How far past its rectangle a box is still a candidate (logical px):
/// what rounding between `takes` and the index might move a point by.
const SLACK: f32 = 0.01;
/// How far past its bounds on the canvas a space is still mapped into
/// (logical px), and per unit of the farthest coordinate: what rounding
/// between mapping its corners out and the point in might move it by.
const MARGIN: (f32, f32) = (0.05, 1e-4);

crate::engine::engine_local! {
    /// The hit index of the hit tables this thread hit last, by table
    /// ([`HitIndex::of`]): a cache, made at first use.
    [Fresh] static HIT_INDEX: std::cell::RefCell<Vec<Indexed>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The spaces and boxes a hit is weighing, kept for their memory
    /// ([`HitView::hit`]).
    [Fresh] static HIT_SCRATCH: std::cell::RefCell<(Vec<u32>, Vec<u32>)> = const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
}

/// A hit table a thread indexed, held weakly (gone with its last frame),
/// and its index.
type Indexed = (std::sync::Weak<[crate::compositor::Hit]>, std::sync::Arc<HitIndex>);

/// Tables whose index a thread keeps (the frames in flight share a few):
/// what bounds the indexes alive on a thread, whatever was hit before.
pub(crate) const INDEXED: usize = 4;

impl HitIndex {
    /// The index of `hits`, made the first time this thread hits through
    /// that table and kept while it is in use. Keyed by the table itself
    /// (its allocation, held weakly so no other table takes its address).
    /// Indexes whose table is gone are let go at every lookup, so a thread
    /// holds at most [`INDEXED`], and none past the next hit once its
    /// table is gone.
    pub(crate) fn of(hits: &std::sync::Arc<[crate::compositor::Hit]>) -> std::sync::Arc<Self> {
        HIT_INDEX.with(|c| {
            let mut c = c.borrow_mut();
            c.retain(|(w, _)| w.strong_count() > 0);
            let same = |w: &std::sync::Weak<[crate::compositor::Hit]>| {
                std::ptr::addr_eq(w.as_ptr(), std::sync::Arc::as_ptr(hits))
            };
            if let Some(at) = c.iter().position(|(w, _)| same(w)) {
                let e = c.remove(at);
                let index = std::sync::Arc::clone(&e.1);
                c.insert(0, e);
                return index;
            }
            let index = std::sync::Arc::new(Self::build(hits));
            c.insert(0, (std::sync::Arc::downgrade(hits), std::sync::Arc::clone(&index)));
            c.truncate(INDEXED);
            index
        })
    }

    /// The index of `hits`, made now.
    pub(crate) fn build(hits: &[crate::compositor::Hit]) -> Self {
        let mut bounds = Vec::with_capacity(hits.len());
        let mut by_space: foldhash::HashMap<u32, usize> = foldhash::HashMap::default();
        let mut members: Vec<(u32, Vec<u32>)> = Vec::new();
        for (i, h) in hits.iter().enumerate() {
            let sl = |v: f32| v.max(0.0) + SLACK;
            let r = h.rect;
            bounds.push([
                r.x - sl(h.slop.left),
                r.y - sl(h.slop.top),
                r.x + r.width.max(0.0) + sl(h.slop.right),
                r.y + r.height.max(0.0) + sl(h.slop.bottom),
            ]);
            let k = *by_space.entry(h.transform).or_insert_with(|| {
                members.push((h.transform, Vec::new()));
                members.len().saturating_sub(1)
            });
            if let (Some(m), Ok(i)) = (members.get_mut(k), u32::try_from(i)) {
                m.1.push(i);
            }
        }
        let spaces = members
            .into_iter()
            .map(|(transform, m)| HitSpace::build(transform, &m, &bounds))
            .collect();
        Self {
            spaces,
            bounds,
            placed: std::sync::Mutex::default(),
        }
    }

    /// Space `k`.
    fn space(&self, k: u32) -> Option<&HitSpace> {
        usize::try_from(k).ok().and_then(|k| self.spaces.get(k))
    }

    /// Box `i`'s rectangle in its space, grown by its slop.
    fn bounds_of(&self, i: u32) -> Option<[f32; 4]> {
        usize::try_from(i).ok().and_then(|i| self.bounds.get(i)).copied()
    }

    /// The spaces whose bounds on the canvas hold `at` (logical), in the
    /// frame `key` with property trees `props` at `s` device pixels to the
    /// logical one, drawn for a window of `viewport`, into `out`: the space
    /// tree refit to that frame at its first hit ([`Placed`]), then a query
    /// of it.
    fn spaces_at(
        &self,
        key: FrameKey,
        props: &crate::scene::Properties,
        s: f32,
        viewport: (f32, f32),
        at: (f32, f32),
        out: &mut Vec<u32>,
    ) {
        // Made and used on one thread (each indexes the tables it hits,
        // [`Self::of`]): the lock is never contended.
        let mut placed = self.placed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let placed = &mut *placed;
        let fresh = !placed.frames.front().is_some_and(|f| f.key == key);
        if fresh {
            if let Some(i) = placed.frames.iter().position(|f| f.key == key) {
                if let Some(f) = placed.frames.remove(i) {
                    placed.frames.push_front(f);
                }
            } else {
                self.place(placed, key, props, s, viewport);
            }
        }
        let Some(frame) = placed.frames.front() else { return };
        out.extend(frame.always.iter().copied());
        frame
            .shape
            .tree
            .query_levels(&frame.levels, at, None, |k| frame.bounds_of(k), out);
    }

    /// Place the spaces on the canvas for frame `key`: each space's extent
    /// mapped out through its transform (its offset, then its scales and
    /// turns), and the space tree packed last refit to them, or packed
    /// anew from them when it no longer fits where points fall, the window
    /// of `viewport` ([`SpaceTree::fits`]).
    fn place(
        &self,
        placed: &mut Placed,
        key: FrameKey,
        props: &crate::scene::Properties,
        s: f32,
        viewport: (f32, f32),
    ) {
        let frame = placed
            .frames
            .len()
            .ge(&PLACEMENTS)
            .then(|| placed.frames.pop_back())
            .flatten();
        // The buffers of the frame let go, for this one.
        let (mut bounds, mut always, mut levels) = frame.map(|f| (f.bounds, f.always, f.levels)).unwrap_or_default();
        bounds.clear();
        always.clear();
        for (k, space) in (0_u32..).zip(&self.spaces) {
            #[cfg(test)]
            PLACED.with(|n| n.set(n.get().saturating_add(1)));
            let b = space.extent.and_then(|e| {
                let off = props.transform(space.transform).offset;
                let m = props
                    .matrices(space.transform, 0)
                    .map(|g| g.matrix())
                    .reduce(crate::gpu::path::then);
                let corners = [[e[0], e[1]], [e[2], e[1]], [e[2], e[3]], [e[0], e[3]]].map(|c| {
                    let p = [(c[0] + off.0) * s, (c[1] + off.1) * s];
                    let p = m.map_or(p, |m| apply(m, p));
                    [p[0] / s, p[1] / s]
                });
                let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
                for c in corners {
                    b = [b[0].min(c[0]), b[1].min(c[1]), b[2].max(c[0]), b[3].max(c[1])];
                }
                let far = b.iter().fold(0.0_f32, |a, v| a.max(v.abs()));
                let grow = MARGIN.0 + MARGIN.1 * far;
                let b = [b[0] - grow, b[1] - grow, b[2] + grow, b[3] + grow];
                b.iter().all(|v| v.is_finite()).then_some(b)
            });
            if (space.extent.is_some() && b.is_none()) || !space.loose.is_empty() {
                always.push(k);
            }
            // A space with no box of finite extent is found only through
            // `always`; it stands in the tree as nothing.
            bounds.push(b.unwrap_or([f32::NAN; 4]));
        }
        let at = |k: u32| usize::try_from(k).ok().and_then(|k| bounds.get(k)).copied();
        let kept = placed.shape.as_ref().filter(|shape| {
            shape.tree.refit(&mut levels, at);
            shape.fits(&bounds, &levels, viewport)
        });
        let shape = match kept {
            Some(shape) => std::sync::Arc::clone(shape),
            None => {
                let shape = std::sync::Arc::new(SpaceTree::pack(&bounds, viewport));
                shape.tree.refit(&mut levels, at);
                placed.shape = Some(std::sync::Arc::clone(&shape));
                shape
            }
        };
        placed.frames.push_front(Placement {
            key,
            bounds,
            always,
            shape,
            levels,
        });
    }
}

/// Whether bounds `b` lie on the canvas (a space whose bounds are not
/// finite is mapped into for every point, not found in the space tree).
fn finite(b: &[f32; 4]) -> bool {
    b.iter().all(|v| v.is_finite())
}

impl SpaceTree {
    /// The tree of the spaces whose `bounds` are finite, its cost weighed
    /// over a window of `viewport`.
    fn pack(bounds: &[[f32; 4]], viewport: (f32, f32)) -> Self {
        let entries: Vec<(u32, [f32; 4])> = (0_u32..)
            .zip(bounds.iter().copied())
            .filter(|(_, b)| finite(b))
            .collect();
        let tree = Rtree::pack(&entries);
        let cost = expected_cost(&tree.levels, viewport);
        Self {
            tree,
            members: bounds.iter().map(finite).collect(),
            cost,
            viewport,
        }
    }

    /// Whether a frame whose spaces lie at `bounds`, drawn for a window of
    /// `viewport`, is placed in this tree refit to them (`levels`): it
    /// holds the same spaces (a space that came onto the canvas would be in
    /// neither it nor the spaces mapped for every point), and the refit's
    /// expected cost of a point in the window is at most twice the packed
    /// tree's, both weighed over that window. The packing's nodes group
    /// spaces that lay near each other then: spaces that moved apart since
    /// grow every node that holds them, and a point visits all of them.
    fn fits(&self, bounds: &[[f32; 4]], levels: &[Vec<RNode>], viewport: (f32, f32)) -> bool {
        let packed = if self.viewport == viewport {
            self.cost
        } else {
            expected_cost(&self.tree.levels, viewport)
        };
        self.members.len() == bounds.len()
            && self.members.iter().zip(bounds).all(|(&m, b)| m == finite(b))
            && expected_cost(levels, viewport) <= packed * 2.0
    }
}

/// The entries a point drawn evenly from the window (`viewport`, logical,
/// from the canvas's corner) tests in a tree with these levels, on
/// average: the root's bounds, then each node's `count` children's bounds
/// when the node holds the point, which it does with the share of the
/// window its bounds cover. This is what a query counts as candidates. It
/// is the surface area heuristic of MacDonald and Booth ("Heuristics for
/// ray tracing using space subdivision", 1990) for points in the plane,
/// with its assumption about where queries fall made ours: a pointer is
/// only ever in the window. Weighed over the root's own bounds instead, one
/// space far off makes the root so large that nodes scattered across the
/// window weigh nothing (review r10: a miss 100–229 times slower than a
/// fresh pack went unseen). With no window known (a view made by hand), the
/// root's bounds stand for it.
fn expected_cost(levels: &[Vec<RNode>], viewport: (f32, f32)) -> f64 {
    let area = |b: &[f32; 4]| f64::from((b[2] - b[0]).max(0.0)) * f64::from((b[3] - b[1]).max(0.0));
    let window = [0.0, 0.0, viewport.0, viewport.1];
    let region = if area(&window) > 0.0 {
        window
    } else {
        match levels.last().and_then(|l| l.first()) {
            Some(root) => root.bounds,
            None => return 0.0,
        }
    };
    let whole = area(&region);
    if !(whole.is_finite() && whole > 0.0) {
        return 0.0;
    }
    let within = |b: &[f32; 4]| {
        [
            b[0].max(region[0]),
            b[1].max(region[1]),
            b[2].min(region[2]),
            b[3].min(region[3]),
        ]
    };
    levels
        .iter()
        .flatten()
        .map(|n| f64::from(n.count) * (area(&within(&n.bounds)) / whole).min(1.0))
        .sum::<f64>()
        + 1.0
}

impl Placement {
    /// Space `k`'s bounds on the canvas in this frame.
    fn bounds_of(&self, k: u32) -> Option<[f32; 4]> {
        usize::try_from(k).ok().and_then(|k| self.bounds.get(k)).copied()
    }
}

/// A static R-tree (Guttman 1984) packed by Sort-Tile-Recursive
/// (Leutenegger, Lopez and Edgington 1997): the entries sorted into
/// `ceil(sqrt(P))` vertical slices by the x of their centres, each slice
/// by y, and cut into leaves of [`FANOUT`]; the leaves packed into nodes
/// the same way, up to one root. Each node keeps its greatest entry, so a
/// query for entries above one already found passes over a node whose
/// entries all lie below it.
#[derive(Debug, Default)]
struct Rtree {
    /// The entries, in leaf order.
    items: Vec<u32>,
    /// The levels, leaves first: a node of level 0 spans `items`, one of
    /// level `l` spans level `l - 1`. The last holds the root alone.
    levels: Vec<Vec<RNode>>,
}

/// A node of an [`Rtree`].
#[derive(Clone, Copy, Debug)]
struct RNode {
    /// The union of its entries' bounds `[left, top, right, bottom]`.
    bounds: [f32; 4],
    /// Where its children start in the level below (or `items`).
    first: u32,
    count: u32,
    /// Its greatest entry.
    top: u32,
}

impl Rtree {
    /// The tree of `entries` (each an entry and its finite bounds).
    fn pack(entries: &[(u32, [f32; 4])]) -> Self {
        let mut leaves: Vec<(u32, [f32; 4], u32)> = entries.iter().map(|&(i, b)| (i, b, i)).collect();
        str_sort(&mut leaves);
        let items = leaves.iter().map(|e| e.0).collect();
        let mut levels = Vec::new();
        let mut below = leaves;
        loop {
            let level: Vec<RNode> = (0_u32..)
                .zip(below.chunks(FANOUT))
                .map(|(c, chunk)| {
                    let first = c.saturating_mul(u32::try_from(FANOUT).unwrap_or(u32::MAX));
                    RNode {
                        bounds: union(chunk.iter().map(|e| e.1)),
                        first,
                        count: u32::try_from(chunk.len()).unwrap_or(0),
                        top: chunk.iter().map(|e| e.2).max().unwrap_or(0),
                    }
                })
                .collect();
            if level.len() <= 1 {
                levels.push(level);
                break;
            }
            // The next level packs these nodes: sorted by STR, and laid
            // out in that order so each parent spans a run of them.
            let mut next: Vec<(u32, [f32; 4], u32)> =
                (0_u32..).zip(&level).map(|(i, n)| (i, n.bounds, n.top)).collect();
            str_sort(&mut next);
            let ordered = next
                .iter()
                .filter_map(|e| usize::try_from(e.0).ok().and_then(|i| level.get(i)).copied())
                .collect();
            levels.push(ordered);
            below = (0_u32..).zip(next).map(|(i, e)| (i, e.1, e.2)).collect();
        }
        Self { items, levels }
    }

    /// Its levels with each node's bounds the union of `bounds` of its
    /// entries as they are now, into `out` (its shape kept, in buffers
    /// `out` has).
    fn refit(&self, out: &mut Vec<Vec<RNode>>, bounds: impl Fn(u32) -> Option<[f32; 4]>) {
        out.resize_with(self.levels.len(), Vec::new);
        for (l, level) in self.levels.iter().enumerate() {
            let (done, rest) = out.split_at_mut(l);
            let Some(fit) = rest.first_mut() else { continue };
            fit.clear();
            for n in level {
                let span = usize::try_from(n.first).unwrap_or(0)
                    ..usize::try_from(n.first.saturating_add(n.count)).unwrap_or(0);
                let b = match done.last() {
                    None => union(self.items.get(span).unwrap_or(&[]).iter().filter_map(|&i| bounds(i))),
                    Some(below) => union(below.get(span).unwrap_or(&[]).iter().map(|c| c.bounds)),
                };
                fit.push(RNode { bounds: b, ..*n });
            }
        }
    }

    /// The entries whose bounds hold `q` and that lie above `floor`, into
    /// `out`, by the tree's own bounds.
    fn query(&self, q: (f32, f32), floor: Option<u32>, bounds: impl Fn(u32) -> Option<[f32; 4]>, out: &mut Vec<u32>) {
        self.query_levels(&self.levels, q, floor, bounds, out);
    }

    /// [`Self::query`] through `levels`: the tree's, or a refit of them.
    fn query_levels(
        &self,
        levels: &[Vec<RNode>],
        q: (f32, f32),
        floor: Option<u32>,
        bounds: impl Fn(u32) -> Option<[f32; 4]>,
        out: &mut Vec<u32>,
    ) {
        let Some(root) = levels.last().and_then(|l| l.first()) else {
            return;
        };
        candidate();
        if floor.is_some_and(|f| root.top <= f) || !within(root.bounds, q) {
            return;
        }
        self.visit(levels, levels.len().saturating_sub(1), root, q, floor, &bounds, out);
    }

    /// The entries under `node` of level `l` that hold `q` above `floor`.
    #[expect(clippy::too_many_arguments, reason = "one step of a descent, its state passed down")]
    fn visit(
        &self,
        levels: &[Vec<RNode>],
        l: usize,
        node: &RNode,
        q: (f32, f32),
        floor: Option<u32>,
        bounds: &impl Fn(u32) -> Option<[f32; 4]>,
        out: &mut Vec<u32>,
    ) {
        let span = usize::try_from(node.first).unwrap_or(0)
            ..usize::try_from(node.first.saturating_add(node.count)).unwrap_or(0);
        if l == 0 {
            for &i in self.items.get(span).unwrap_or(&[]) {
                candidate();
                if floor.is_none_or(|f| i > f) && bounds(i).is_some_and(|b| within(b, q)) {
                    out.push(i);
                }
            }
            return;
        }
        let Some(below) = levels.get(l.saturating_sub(1)) else {
            return;
        };
        for child in below.get(span).unwrap_or(&[]) {
            candidate();
            if floor.is_none_or(|f| child.top > f) && within(child.bounds, q) {
                self.visit(levels, l.saturating_sub(1), child, q, floor, bounds, out);
            }
        }
    }
}

/// Sort `entries` into Sort-Tile-Recursive order: `ceil(sqrt(P))` slices
/// (`P` the leaves of [`FANOUT`]) by the x of their centres, each by y.
/// Ties keep the entries' order, so rows of one width pack by their rows.
fn str_sort(entries: &mut [(u32, [f32; 4], u32)]) {
    let leaves = entries.len().div_ceil(FANOUT);
    let mut slices = 1_usize;
    while slices.saturating_mul(slices) < leaves {
        slices = slices.saturating_add(1);
    }
    let x = |b: &[f32; 4]| (b[0] + b[2]) * 0.5;
    let y = |b: &[f32; 4]| (b[1] + b[3]) * 0.5;
    entries.sort_by(|a, b| x(&a.1).total_cmp(&x(&b.1)).then(a.0.cmp(&b.0)));
    for slice in entries.chunks_mut(slices.saturating_mul(FANOUT).max(1)) {
        slice.sort_by(|a, b| y(&a.1).total_cmp(&y(&b.1)).then(a.0.cmp(&b.0)));
    }
}

/// The union of `bounds`, empty (inverted) for none.
fn union(bounds: impl Iterator<Item = [f32; 4]>) -> [f32; 4] {
    bounds.fold(
        [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY],
        |u, b| [u[0].min(b[0]), u[1].min(b[1]), u[2].max(b[2]), u[3].max(b[3])],
    )
}

/// Whether `b` holds `q`.
fn within(b: [f32; 4], q: (f32, f32)) -> bool {
    q.0 >= b[0] && q.0 <= b[2] && q.1 >= b[1] && q.1 <= b[3]
}

impl HitSpace {
    /// The space of transform `transform`, holding boxes `members` (in
    /// table order) whose rectangles are `bounds`.
    fn build(transform: u32, members: &[u32], bounds: &[[f32; 4]]) -> Self {
        let at = |i: u32| usize::try_from(i).ok().and_then(|i| bounds.get(i)).copied();
        let mut entries = Vec::with_capacity(members.len());
        let mut loose = Vec::new();
        for &i in members {
            match at(i).filter(|b| b.iter().all(|v| v.is_finite())) {
                Some(b) => entries.push((i, b)),
                None => loose.push(i),
            }
        }
        let extent = (!entries.is_empty()).then(|| union(entries.iter().map(|e| e.1)));
        Self {
            transform,
            boxes: Rtree::pack(&entries),
            loose,
            extent,
            top: members.iter().copied().max().unwrap_or(0),
        }
    }
}

/// The resampling layers a box is drawn through, inner first: a few kept
/// in place, more spilled to the heap (no depth is left out).
#[derive(Default)]
struct Levels<'a> {
    fixed: [Option<&'a crate::scene::LayerStage>; 8],
    n: usize,
    spill: Vec<&'a crate::scene::LayerStage>,
}

impl<'a> Levels<'a> {
    fn push(&mut self, st: &'a crate::scene::LayerStage) {
        if self.spill.is_empty()
            && let Some(slot) = self.fixed.get_mut(self.n)
        {
            *slot = Some(st);
            self.n = self.n.saturating_add(1);
            return;
        }
        if self.spill.is_empty() {
            self.spill.extend(self.fixed.iter().flatten().copied());
        }
        self.spill.push(st);
    }

    fn as_slice(&self) -> LevelSlice<'_, 'a> {
        if self.spill.is_empty() {
            LevelSlice::Fixed(self.fixed.get(..self.n).unwrap_or_default())
        } else {
            LevelSlice::Spilled(&self.spill)
        }
    }
}

/// [`Levels`] as read.
#[derive(Clone, Copy)]
enum LevelSlice<'b, 'a> {
    Fixed(&'b [Option<&'a crate::scene::LayerStage>]),
    Spilled(&'b [&'a crate::scene::LayerStage]),
}

impl<'a> LevelSlice<'_, 'a> {
    fn len(self) -> usize {
        match self {
            Self::Fixed(s) => s.len(),
            Self::Spilled(s) => s.len(),
        }
    }

    fn get(self, k: usize) -> Option<&'a crate::scene::LayerStage> {
        match self {
            Self::Fixed(s) => s.get(k).copied().flatten(),
            Self::Spilled(s) => s.get(k).copied(),
        }
    }
}

/// What a texel shows, by its level and its centre's bits ([`Levelled::shown`]).
type Measured = foldhash::HashMap<(usize, u32, u32), Option<f32>>;

/// A box's clip chain read level by level ([`HitView::keeps`]): level `k`
/// below `levels.len()` is the layer `levels.get(k)` (inner first), the
/// last the canvas.
struct Levelled<'v, 'a, 'b> {
    view: &'v HitView<'a>,
    h: &'v crate::compositor::Hit,
    levels: LevelSlice<'b, 'a>,
    s: f32,
    /// What each texel of an inner level shows, once measured, by level
    /// and centre: the composites about a point read overlapping texels
    /// (four a sample, a level's samples sharing them), so each is
    /// measured once and the cost is the texels each level holds under
    /// the point, not four to the power of the depth.
    /// Made for one query, in the world that asks it.
    memo: Option<crate::engine::WorldRefCell<Measured>>,
    /// How many points were measured (the cost tests).
    visits: crate::engine::WorldCell<usize>,
}

impl Levelled<'_, '_, '_> {
    /// The transform level `k`'s texels lie in (0: the canvas's).
    fn until(&self, k: usize) -> u32 {
        self.levels.get(k).map_or(0, |st| st.transform)
    }

    /// How much of the pixel of level `k`'s target centred on `g`
    /// (physical) shows the box: what the clips cutting that target keep
    /// of it, times, past the innermost level, what the composite of the
    /// level inside shows there. A texel of an inner level is measured
    /// once ([`Self::memo`]).
    fn shown(&self, k: usize, g: (f32, f32)) -> Option<f32> {
        let key = (k, g.0.to_bits(), g.1.to_bits());
        let inner = k < self.levels.len();
        if inner
            && let Some(memo) = &self.memo
            && let Some(known) = memo.with(|m| m.get(&key).copied())
        {
            return known;
        }
        let v = self.measure(k, g);
        if inner && let Some(memo) = &self.memo {
            memo.with_mut(|m| m.insert(key, v));
        }
        v
    }

    /// [`Self::shown`], measured.
    fn measure(&self, k: usize, g: (f32, f32)) -> Option<f32> {
        self.visits.set(self.visits.get().saturating_add(1));
        let props = self.view.props;
        let s = self.s;
        let n = self.levels.len();
        let mut kept = 1.0_f32;
        let mut level = 0_usize;
        let mut c = self.h.clip;
        let mut steps = 0_usize;
        while c != 0 && steps <= props.clips.len() {
            steps = steps.saturating_add(1);
            // A layer's output clip, and those above it, clip its
            // composite in the level it is composited into.
            while level < n && self.levels.get(level).is_some_and(|st| st.clip == c) {
                level = level.saturating_add(1);
            }
            if level > k {
                break;
            }
            let node = usize::try_from(c).ok().and_then(|i| props.clips.get(i))?;
            if level == k {
                let (q, into) = undone(compose(props, node.transform, self.until(k)), g, s)?;
                let clip = node.clip;
                kept *= link_coverage(clip.bounds, clip.radius, clip.curve, q, s, into);
                if kept <= 0.0 {
                    return Some(0.0);
                }
            }
            c = node.parent;
        }
        let Some(inner) = k.checked_sub(1) else {
            return Some(kept);
        };
        // The level inside, where this one samples it: its map into this
        // level's space, through each layer between (those that do not
        // resample map nothing).
        let outer = self.levels.get(k).map_or(0, |st| st.effect);
        let mut st = self.levels.get(inner)?;
        let mut m = IDENTITY;
        let mut steps = 0_usize;
        loop {
            m = crate::gpu::path::then(m, st.map);
            steps = steps.saturating_add(1);
            if st.outer == outer || st.outer == 0 || steps > self.view.stages.len() {
                break;
            }
            st = self.view.stage(st.outer)?;
        }
        let (q, _) = undone(Some(m), g, s)?;
        Some(kept * resampled((q.0 * s, q.1 * s), |t| self.shown(inner, t))?)
    }
}

/// What the composite of a layer shows at point `g` of the layer's space
/// (physical; its texels' centres at whole pixels and a half, its region
/// whole pixels): the four texels about it, each keeping `kept(centre)`,
/// weighed as the composite's bilinear filter weighs them (`KIND_LAYER`
/// in `gpu/shaders.wgsl` samples with the linear sampler).
fn resampled(g: (f32, f32), mut kept: impl FnMut((f32, f32)) -> Option<f32>) -> Option<f32> {
    let (ux, uy) = (g.0 - 0.5, g.1 - 0.5);
    let (ix, iy) = (ux.floor(), uy.floor());
    let (fx, fy) = (ux - ix, uy - iy);
    let mut sum = 0.0_f32;
    for (dx, dy, w) in [
        (0.0, 0.0, (1.0 - fx) * (1.0 - fy)),
        (1.0, 0.0, fx * (1.0 - fy)),
        (0.0, 1.0, (1.0 - fx) * fy),
        (1.0, 1.0, fx * fy),
    ] {
        if w > 0.0 {
            sum += w * kept((ix + dx + 0.5, iy + dy + 0.5))?;
        }
    }
    Some(sum)
}

#[cfg(test)]
mod tests {
    use crate::element::{IntoElement, div};
    use crate::layout::layout;
    use crate::style::{Corners, Overflow};
    use crate::styled::Styled;
    use crate::text::TextSystem;

    #[test]
    fn a_click_in_a_rounded_cards_cut_away_corner_misses_a_square_field_in_it() {
        // A card rounded 24 that clips, holding a square scroll container
        // that fills it and a field that fills that: the field's own clip
        // is the container's (square); the card's corner, one clip up its
        // chain, cuts the field there as paint cuts it.
        let root = div()
            .width(200.0)
            .height(200.0)
            .padding_top(20.0)
            .padding_left(20.0)
            .child(
                div()
                    .width(100.0)
                    .height(100.0)
                    .radius(Corners::all(24.0))
                    .overflow(Overflow::Hidden)
                    .child(
                        div()
                            .width(100.0)
                            .height(100.0)
                            .overflow_y(Overflow::Scroll)
                            .child(div().width(100.0).height(100.0).on_click(|_| {})),
                    ),
            )
            .into_element();
        let mut text = TextSystem::new();
        let ui = crate::interact::InputState::default();
        let f = layout(
            &root,
            200.0,
            200.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            &mut text,
        );
        let field = crate::interact::hit(&f, 70.0, 70.0).expect("the field, inside");
        assert!(f.behaviour(field).is_some_and(|b| b.handlers.click.is_some()));
        // Inside the field's square, outside the card's corner (radius 24
        // about (44, 44)).
        assert_ne!(crate::interact::hit(&f, 22.0, 22.0), Some(field), "the card's corner");
        assert_eq!(
            crate::interact::hit(&f, 22.0, 70.0),
            Some(field),
            "the card's edge, inside"
        );
    }

    /// A card at `x` (logical) that clips, rounded `radius`, holding a
    /// wrapper of its box that clips, rounded the same, holding a field
    /// that starts ten to the left of them (so the clips, not its own box,
    /// decide there), laid out at `scale`.
    fn coincident(x: f32, radius: f32, scale: f32) -> (crate::layout::Frame, usize) {
        let root = div()
            .width(200.0)
            .height(200.0)
            .padding_top(20.0)
            .padding_left(x)
            .child(
                div()
                    .width(100.0)
                    .height(100.0)
                    .radius(Corners::all(radius))
                    .overflow(Overflow::Hidden)
                    .child(
                        div()
                            .width(100.0)
                            .height(100.0)
                            .radius(Corners::all(radius))
                            .overflow(Overflow::Hidden)
                            .child(div().margin_left(-10.0).width(120.0).height(100.0).on_click(|_| {})),
                    ),
            )
            .into_element();
        let mut text = TextSystem::new();
        let mut state = crate::interact::InputState::default().state().clone();
        state.scale = scale;
        let f = layout(
            &root,
            200.0,
            200.0,
            &state,
            &mut crate::motion::Motion::default(),
            &mut text,
        );
        let field = crate::interact::hit(&f, 70.0, 70.0).expect("the field, inside");
        assert!(f.behaviour(field).is_some_and(|b| b.handlers.click.is_some()));
        (f, field)
    }

    #[test]
    fn a_click_hits_by_the_device_pixel_it_lands_in_as_painted() {
        // Two rounded edges at x = 20 each keep the pixel 20..21 whole and
        // nothing of 19..20: a click anywhere in the one hits, in the
        // other misses.
        let (f, field) = coincident(20.0, 8.0, 1.0);
        assert_eq!(crate::interact::hit(&f, 20.1, 70.0), Some(field), "the pixel painted");
        assert_eq!(crate::interact::hit(&f, 20.9, 70.0), Some(field));
        assert_ne!(crate::interact::hit(&f, 19.9, 70.0), Some(field), "the pixel before");
    }

    #[test]
    fn under_a_scale_of_one_and_a_half_odd_coincident_edges_hit_where_they_paint() {
        // The card at 10.2 logical is 15.3 device pixels in: paint snaps
        // its clips to 15 (`Bounds::snapped`), so the device pixel 15..16
        // (10.0..10.67 logical) shows the field whole, and 14..15 shows
        // nothing of it. A click at 10.05 lands in the first.
        let (f, field) = coincident(10.2, 8.0, 1.5);
        assert_eq!(crate::interact::hit(&f, 10.05, 70.0), Some(field), "shown whole there");
        assert_ne!(crate::interact::hit(&f, 9.95, 70.0), Some(field), "nothing shown there");
        // At 10.4 (15.6 device pixels) paint cuts at 16: the device pixel
        // 15..16 (to 10.67 logical) shows nothing of the field, though a
        // logical pixel about 10.63 would be kept 0.73 by each edge.
        let (f, field) = coincident(10.4, 8.0, 1.5);
        assert_ne!(
            crate::interact::hit(&f, 10.63, 70.0),
            Some(field),
            "nothing shown there"
        );
        assert_eq!(crate::interact::hit(&f, 10.7, 70.0), Some(field), "shown whole there");
        let (square, field) = coincident(10.4, 0.0, 1.5);
        assert_ne!(
            crate::interact::hit(&square, 10.63, 70.0),
            Some(field),
            "square clips too"
        );
        assert_eq!(crate::interact::hit(&square, 10.7, 70.0), Some(field));
    }

    /// The coincident rounded cards at 10.4 under a scale of one and a
    /// half, drawn by the GPU: across their left edge and their top left
    /// corner, a point hits exactly where the device pixel it lies in shows
    /// at least half of the field (black over white, read back).
    #[test]
    fn coincident_rounded_cards_hit_where_the_gpu_shows_half_of_the_field() {
        let group = |cards: crate::element::Div| div().child(cards);
        Sweep::new((10.4, 20.0), 1.5, (8.0, 18.0)).run(group, |_| {});
    }

    /// A sweep of hits against the GPU's pixels ([`Sweep::run`]).
    struct Sweep {
        /// Where the group lies (logical).
        at: (f32, f32),
        /// Device pixels to the logical one.
        scale: f32,
        /// The sweep's first point (logical).
        from: (f32, f32),
        /// What the group shows the field at.
        opacity: f32,
        /// How many layers whose composite resamples the renderer draws it
        /// through, as it publishes them.
        layers: usize,
        /// The cards' corners' radius, as the tree reads it.
        radius: crate::reactive::Signal<f32>,
        /// Points tried across and down (a tenth and a quarter apart).
        grid: (u16, u16),
        /// How tall the page is (logical): past a texture's side, there is
        /// no canvas and what is read back in bands is what shows.
        height: u32,
    }

    impl Sweep {
        fn new(at: (f32, f32), scale: f32, from: (f32, f32)) -> Self {
            Self {
                at,
                scale,
                from,
                opacity: 1.0,
                layers: 0,
                radius: crate::reactive::Signal::new(8.0),
                grid: (60, 40),
                height: 120,
            }
        }

        fn height(self, height: u32) -> Self {
            Self { height, ..self }
        }

        fn layers(self, layers: usize) -> Self {
            Self { layers, ..self }
        }

        fn radius(self, radius: crate::reactive::Signal<f32>) -> Self {
            Self { radius, ..self }
        }

        fn grid(self, grid: (u16, u16)) -> Self {
            Self { grid, ..self }
        }

        fn opacity(self, opacity: f32) -> Self {
            Self { opacity, ..self }
        }

        /// Two coincident rounded cards (`radius` round, 60 wide) that
        /// clip, a black field in both starting ten to their left (a black
        /// box over its far end, so the two overlap away from the points
        /// tried), the cards wrapped by `group` and set at `at` on white,
        /// drawn by the GPU at `scale` and presented, then `act` done and
        /// the UI's frame brought up to it before input is routed (what
        /// `act` changed is not presented): over the `grid` of points from
        /// `from` (a tenth and a quarter apart), input lands on the field
        /// exactly where the device pixel it lies in shows at least half of
        /// it in the frame presented (read back, `opacity` taken off; pixels
        /// a level from half are left out), and the devtools pick what a
        /// click lands on. Of a full grid, more than a thousand points are
        /// tried, more than ten of them on pixels part shown.
        fn run(
            self,
            group: impl Fn(crate::element::Div) -> crate::element::Div + 'static,
            act: impl FnOnce(&mut crate::app::Scenario),
        ) {
            use crate::color::Color;
            let Self {
                at,
                scale,
                from,
                opacity,
                layers,
                radius,
                grid,
                height,
            } = self;
            let group = std::rc::Rc::new(group);
            let tree = move || {
                let r = radius.get();
                let card = |inner: crate::element::Div| {
                    div()
                        .width(60.0)
                        .height(60.0)
                        .radius(Corners::all(r))
                        .overflow(Overflow::Hidden)
                        .child(inner)
                };
                let field = div()
                    .margin_left(-10.0)
                    .width(80.0)
                    .height(60.0)
                    .background(Color::BLACK)
                    .on_click(|_| {})
                    .child(
                        div()
                            .margin_left(60.0)
                            .width(10.0)
                            .height(10.0)
                            .background(Color::BLACK),
                    );
                div()
                    .width(400.0)
                    .height(crate::convert::u32_to_f32(height))
                    .background(Color::WHITE)
                    .padding_left(at.0)
                    .padding_top(at.1)
                    .child(group(card(card(field))))
                    .into_element()
            };
            let mut s = crate::app::Scenario::measuring(tree, 400, height, scale).expect("a scenario");
            s.settle().expect("settled");
            s.frame("shown").expect("a frame");
            act(&mut s);
            if height > 120 {
                assert!(
                    s.presented().stages.is_empty(),
                    "a page taller than a texture has no canvas to show"
                );
            }
            let shot = s.shot(false).expect("read back");
            // The UI lays out what `act` asked for as input comes, before
            // the frame of it is presented.
            s.send(crate::window::Input::PointerMoved { x: 399.0, y: 119.0 });
            let presented = s.presented();
            let own = presented.stages.iter().filter(|l| l.resamples()).count();
            assert_eq!(own, layers, "the layers the renderer drew that resample");
            let f = s.pipeline().frame();
            let input = crate::interact::InputState::showing(f, std::sync::Arc::clone(&presented));
            let field = (0..f.nodes.len())
                .rev()
                .find(|&i| f.behaviour(i).is_some_and(|b| b.handlers.click.is_some()))
                .expect("the field");
            let mut partly = 0_usize;
            let mut tried = 0_usize;
            for j in 0..grid.1 {
                for i in 0..grid.0 {
                    let (x, y) = (from.0 + f32::from(i) * 0.1, from.1 + f32::from(j) * 0.25);
                    let (px, py) = ((x * scale).floor(), (y * scale).floor());
                    #[expect(clippy::as_conversions, reason = "a pixel's whole index")]
                    let Some([r, ..]) = shot.pixel(px as u32, py as u32) else {
                        continue;
                    };
                    // Black over white: what the pixel shows of the field.
                    let shown = (1.0 - f32::from(r) / 255.0) / opacity;
                    if (shown - 0.5).abs() < 2.0 / 255.0 / opacity {
                        continue;
                    }
                    if shown > 0.02 && shown < 0.98 {
                        partly = partly.saturating_add(1);
                    }
                    tried = tried.saturating_add(1);
                    let landed = input.hit_shown(f, x, y);
                    // The devtools' pick, by the same rule.
                    assert_eq!(
                        crate::interact::hit_presented(f, Some(&presented), x, y),
                        landed,
                        "({x}, {y}): the devtools pick where a click lands"
                    );
                    let hits = landed.is_some_and(|h| f.ancestors(h).any(|a| a == field));
                    assert_eq!(
                        hits,
                        shown >= 0.5,
                        "({x}, {y}), device pixel ({px}, {py}) shows {shown}"
                    );
                }
            }
            let full = u32::from(grid.0).saturating_mul(u32::from(grid.1)) >= 2400;
            assert!(
                (tried > 1000 || !full) && partly > 10,
                "{tried} points, {partly} on pixels part shown"
            );
        }
    }

    /// A group scaled `k` times about its top left corner.
    fn scaled(k: f32, inner: crate::element::Div) -> crate::element::Div {
        div()
            .scale(k)
            .origin(crate::style::Origin::At([
                crate::style::Calc::fraction(0.0),
                crate::style::Calc::fraction(0.0),
            ]))
            .child(inner)
    }

    /// A filter that changes no pixel: the renderer draws what it holds
    /// into a layer.
    fn filtered(group: crate::element::Div) -> crate::element::Div {
        group.filters(crate::style::Filters::repeat(1, crate::style::Filter::Brightness(1.0)))
    }

    /// The cards in a group that filters (a layer) scaled three times
    /// about its corner at 9.825: its texels are not the canvas's pixels.
    /// Each clip cuts the texels (their edges at 10, on a texel's side, so
    /// each texel whole or not at all), and the composite samples them
    /// between, a ramp three pixels wide. The cards' edges land at 10.35 on
    /// the canvas, so the pixel 10..11 shows 0.55 of the field: measured on
    /// the canvas, by distance, the two clips would keep 0.65 each, 0.42
    /// in all, a miss. Hit testing reads the texels the composite samples,
    /// as the screen shows them.
    #[test]
    fn coincident_cards_in_a_scaled_layer_hit_where_the_gpu_shows_half() {
        // The end of the cards' corner (24 pixels round) and their edge.
        let layer = |cards| filtered(scaled(3.0, cards));
        Sweep::new((9.825, 19.825), 1.0, (8.0, 38.0))
            .layers(1)
            .run(layer, |_| {});
    }

    /// The cards in a filtered group scaled twice: input reads the texels
    /// of its layer as the composite samples them (bilinear, a ramp two
    /// pixels wide over texels each kept whole or not at all), and lands
    /// on the field exactly where the read-back pixel shows half or more.
    #[test]
    fn a_hit_in_a_layer_scaled_twice_reads_the_texels_its_composite_samples() {
        let layer = |cards| filtered(scaled(2.0, cards));
        Sweep::new((9.825, 19.825), 1.0, (8.0, 30.0))
            .layers(1)
            .run(layer, |_| {});
    }

    /// The cards in a group scaled three times that shows at 0.6 what
    /// overlaps in it: the renderer draws it into a layer (no filter asks
    /// for one) and composites it faded, its texels sampled as in a
    /// filtered one.
    #[test]
    fn coincident_cards_in_a_scaled_faded_layer_hit_where_the_gpu_shows_half() {
        let layer = |cards| scaled(3.0, cards).opacity(0.6);
        Sweep::new((9.825, 19.825), 1.0, (8.0, 38.0))
            .layers(1)
            .opacity(0.6)
            .run(layer, |_| {});
    }

    /// The cards in a group scaled three times beside a box with a shadow,
    /// which the renderer cannot draw scaled on the canvas: it draws the
    /// whole group into a layer instead (its fallback), and the cards'
    /// clips cut its texels.
    #[test]
    fn coincident_cards_in_a_scaled_fallback_layer_hit_where_the_gpu_shows_half() {
        let layer = |cards: crate::element::Div| {
            let shadowed = div()
                .margin_left(20.0)
                .width(10.0)
                .height(10.0)
                .background(crate::color::Color::WHITE)
                .shadow(crate::style::Shadow {
                    x: 0.0,
                    y: 2.0,
                    blur: 4.0,
                    spread: 0.0,
                    color: crate::color::Color::BLACK,
                });
            scaled(
                3.0,
                div()
                    .flex_direction(crate::style::FlexDirection::Row)
                    .child(cards)
                    .child(shadowed),
            )
        };
        Sweep::new((9.825, 19.825), 1.0, (8.0, 38.0))
            .layers(1)
            .run(layer, |_| {});
    }

    /// The cards in a filtered group scaled 0.75 times, in another scaled
    /// four times: the inner layer's texels (three quarters of the outer's)
    /// are resampled into the outer's, and those onto the canvas four
    /// pixels apart. Sampled once, from the canvas straight into the inner
    /// texels, the ramp the outer's texels interpolate is missed, and a
    /// pixel showing 0.47 of the field would hit.
    #[test]
    fn coincident_cards_in_nested_scaled_layers_hit_where_the_gpu_shows_half() {
        let layers = |cards| filtered(scaled(4.0, filtered(scaled(0.75, cards))));
        Sweep::new((9.825, 19.825), 1.0, (8.0, 38.0))
            .layers(2)
            .run(layers, |_| {});
    }

    /// The cards (their edge at 8) in a filtered group scaled half about
    /// its corner, in another scaled twice about the same point (8.6875,
    /// Codex's): on the canvas the two cancel, but each composite samples
    /// its texels into the target it is composited into, the inner's into
    /// the outer's at half, so each is a stage. Read against the canvas,
    /// the inner layer would seem the canvas's pixels and its stage would
    /// be lost: pixel 8 shows 0.83 of the field and would be a miss.
    #[test]
    fn cards_in_a_halved_layer_in_a_doubled_one_hit_where_the_gpu_shows_half() {
        let layers =
            |cards: crate::element::Div| filtered(scaled(2.0, filtered(scaled(0.5, cards.margin_left(-0.6875)))));
        Sweep::new((8.6875, 19.825), 1.0, (5.0, 38.0))
            .layers(2)
            .run(layers, |_| {});
    }

    /// The nested 4× and 0.75× layers, five filtered groups that scale
    /// nothing inside them: those draw layers too, but their composites
    /// sample their texels at their target's pixels' centres, so they add
    /// no stage, and the two that resample are both followed.
    #[test]
    fn layers_that_resample_nothing_add_no_stage_and_take_no_level() {
        let layers = |cards| {
            let mut inner = cards;
            for _ in 0..5 {
                inner = filtered(div().child(inner));
            }
            filtered(scaled(4.0, filtered(scaled(0.75, inner))))
        };
        Sweep::new((9.825, 19.825), 1.0, (8.0, 38.0))
            .layers(2)
            .run(layers, |_| {});
    }

    /// Seven filtered layers one in another, scaled by a quarter and four
    /// times by turns, then 0.75 and 4: seven stages, each resampled, one
    /// past the six round 3 followed. Every level is followed: with the
    /// outermost left out, pixel 10 showing 0.49 of the field would hit.
    #[test]
    fn seven_resampling_layers_one_in_another_are_all_followed() {
        let layers = |cards: crate::element::Div| {
            let mut inner = cards;
            for k in [0.25, 4.0, 0.25, 4.0, 0.25, 0.75, 4.0] {
                inner = filtered(scaled(k, inner));
            }
            inner
        };
        Sweep::new((9.825, 19.825), 1.0, (8.0, 38.0))
            .layers(7)
            .grid((40, 24))
            .run(layers, |_| {});
    }

    /// A scroll container (the field) in the two coincident rounded cards,
    /// in another that scrolls, all in a filtered group scaled three times
    /// (the round-3 fixture): on the pixel whose sampled texels show 0.55
    /// of the field (on the canvas, by distance, the clips keep 0.42), a
    /// click and the devtools' pick land on the field, and a wheel there
    /// scrolls it, not the container around: one rule, read through the
    /// layer the frame was drawn with.
    #[test]
    fn a_click_the_devtools_and_a_wheel_agree_on_a_pixel_a_layer_resamples() {
        use crate::color::Color;
        let (inner, outer) = (crate::ScrollHandle::new(), crate::ScrollHandle::new());
        let tree = move || {
            let card = |c: crate::element::Div| {
                div()
                    .width(60.0)
                    .height(60.0)
                    .radius(Corners::all(8.0))
                    .overflow(Overflow::Hidden)
                    .child(c)
            };
            let field = div()
                .margin_left(-10.0)
                .width(80.0)
                .height(60.0)
                .background(Color::BLACK)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&inner)
                .child(div().width(80.0).height(200.0));
            let around = div()
                .width(100.0)
                .height(60.0)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&outer)
                .child(div().width(100.0).height(200.0).child(card(card(field))));
            div()
                .width(400.0)
                .height(200.0)
                .background(Color::WHITE)
                .padding_left(9.825)
                .padding_top(19.825)
                .child(filtered(scaled(3.0, around)))
                .into_element()
        };
        let mut s = crate::app::Scenario::measuring(tree, 400, 200, 1.0).expect("a scenario");
        s.settle().expect("settled");
        s.frame("shown").expect("a frame");
        let shot = s.shot(false).expect("read back");
        // The pixel on row 60 that shows a little over half of the field.
        let y = 60_u32;
        let x = (0..40_u32)
            .find(|&x| {
                let r = shot.pixel(x, y).map_or(255, |p| p[0]);
                let shown = 1.0 - f32::from(r) / 255.0;
                (0.5..0.7).contains(&shown)
            })
            .expect("a pixel part shown");
        #[expect(clippy::as_conversions, reason = "a pixel index, small")]
        let at = (x as f32 + 0.5, y as f32 + 0.5);
        let presented = s.presented();
        let f = s.pipeline().frame();
        let field = (0..f.nodes.len())
            .find(|&i| {
                let n = &f.nodes[i];
                n.scroll(crate::inspect::OUTSIDE).is_some() && n.bounds(crate::inspect::OUTSIDE).width == 80.0
            })
            .expect("the field");
        let input = crate::interact::InputState::showing(f, std::sync::Arc::clone(&presented));
        let clicked = input.hit_shown(f, at.0, at.1);
        assert!(
            clicked.is_some_and(|h| f.ancestors(h).any(|a| a == field)),
            "the click lands on the field"
        );
        assert_eq!(
            crate::interact::hit_presented(f, Some(&presented), at.0, at.1),
            clicked,
            "the devtools agree"
        );
        assert!(s.compositor_wheel(at, (0.0, 10.0), 1.0), "the wheel scrolls");
        s.frame("scrolled").expect("a frame");
        assert!(inner.offset().1 > 0.0, "the field scrolled");
        assert_eq!(outer.offset().1, 0.0, "the container around it did not");
    }

    /// Cards in a group scaled three times, a field that clicks in them,
    /// on white; the group filtered (a layer) when `layered`.
    fn three_times(layered: bool, clicks: crate::reactive::Signal<u32>) -> crate::app::Scenario {
        use crate::color::Color;
        let tree = move || {
            let card = |c: crate::element::Div| {
                div()
                    .width(60.0)
                    .height(60.0)
                    .radius(Corners::all(8.0))
                    .overflow(Overflow::Hidden)
                    .child(c)
            };
            let field = div()
                .margin_left(-10.0)
                .width(80.0)
                .height(60.0)
                .background(Color::BLACK)
                .on_click(move |_| clicks.set(clicks.get_untracked().saturating_add(1)))
                .on_wheel(|_| true);
            let group = scaled(3.0, card(card(field)));
            div()
                .width(400.0)
                .height(200.0)
                .background(Color::WHITE)
                .padding_left(9.825)
                .padding_top(19.825)
                .child(if layered { filtered(group) } else { group })
                .into_element()
        };
        crate::app::Scenario::measuring(tree, 400, 200, 1.0).expect("a scenario")
    }

    /// Before any frame is presented no layer was drawn: input lands by
    /// the one hit rule on the scene the UI painted, its canvas's pixels
    /// (the group drawn straight to the canvas shows less than half of
    /// the field at a pixel of the cards' edge, and input there misses),
    /// and on the field's inside it lands. Once a frame drawn through the
    /// layer is presented, the same pixel shows more than half (the
    /// composite resamples), and input lands by it.
    #[test]
    fn input_before_the_first_present_lands_by_the_canvas_and_after_it_by_the_layer() {
        let clicks = crate::reactive::Signal::new(0_u32);
        let mut s = three_times(true, clicks);
        s.settle().expect("presented");
        let layer = s.shot(false).expect("read back");
        let mut flat = three_times(false, crate::reactive::Signal::new(0_u32));
        flat.settle().expect("presented");
        let canvas = flat.shot(false).expect("read back");
        let shown =
            |p: &crate::image::Pixels, x: u32, y: u32| p.pixel(x, y).map_or(0.0, |c| 1.0 - f32::from(c[0]) / 255.0);
        let y = 60_u32;
        let x = (0..40_u32)
            .find(|&x| shown(&layer, x, y) >= 0.5 && shown(&canvas, x, y) < 0.5 && shown(&canvas, x, y) > 0.02)
            .expect("a pixel the layer shows half of and the canvas does not");
        let centre = |v: u32| f32::from(u16::try_from(v).expect("small")) + 0.5;
        let at = (centre(x), centre(y));
        // Nothing presented yet: the scene the UI painted.
        let none = crate::compositor::Shown::BeforeFirst;
        let press = |s: &mut crate::app::Scenario, p: (f32, f32)| {
            let pipeline = s.pipeline_mut();
            pipeline.input_shown(crate::window::Input::MouseDown { x: p.0, y: p.1 }, &none);
            pipeline.input_shown(crate::window::Input::MouseUp { x: p.0, y: p.1 }, &none);
        };
        press(&mut s, at);
        assert_eq!(clicks.get_untracked(), 0, "by the canvas's pixel, less than half shown");
        press(&mut s, (100.0, 110.0));
        assert_eq!(clicks.get_untracked(), 1, "inside the field");
        // A wheel stamped before the first present is routed on the render
        // thread by the same rule, through the same scene: where the click
        // missed it finds nothing, where it landed the field takes it.
        let wheel = |s: &mut crate::app::Scenario, p: (f32, f32), seen| {
            s.compositor().wheel_input(crate::compositor::WheelInput {
                point: p,
                delta: (0.0, 10.0),
                time: crate::clock::current_nanos(),
                precise: false,
                phase: crate::compositor::WheelPhase::None,
                seen,
            })
        };
        let first = crate::compositor::Seen::BeforeFirst;
        assert_eq!(
            wheel(&mut s, at, first),
            crate::compositor::Wheeled::Nothing,
            "by the canvas's pixel, as the click"
        );
        assert!(
            matches!(
                wheel(&mut s, (100.0, 110.0), first),
                crate::compositor::Wheeled::Ui(Some(_), _)
            ),
            "inside the field, as the click"
        );
        let presented = s.shown().seen();
        assert!(
            matches!(wheel(&mut s, at, presented), crate::compositor::Wheeled::Ui(Some(_), _)),
            "stamped with the frame presented: by the layer's pixel"
        );
        let f = s.pipeline().frame();
        let field = (0..f.nodes.len())
            .rev()
            .find(|&i| f.behaviour(i).is_some_and(|b| b.handlers.click.is_some()))
            .expect("the field");
        assert_ne!(
            crate::interact::hit(f, at.0, at.1),
            Some(field),
            "the frame as painted: the canvas's rule"
        );
        assert_eq!(crate::interact::hit(f, 100.0, 110.0), Some(field), "inside it");
        // Presented through the layer: its composite's pixel decides.
        s.click(at.0, at.1);
        assert_eq!(clicks.get_untracked(), 2, "by the layer's pixel, more than half shown");
    }

    /// Past the eight levels kept in place, the rest spill to the heap,
    /// in order: no level is dropped, however deep.
    #[test]
    fn levels_past_those_kept_in_place_spill_in_order() {
        let stages: Vec<crate::scene::LayerStage> = (1..=11)
            .map(|effect| crate::scene::LayerStage {
                effect,
                outer: 0,
                transform: 0,
                map: [2.0, 0.0, 0.0, 2.0, 0.0, 0.0],
                clip: 0,
            })
            .collect();
        let mut levels = super::Levels::default();
        for st in &stages {
            levels.push(st);
        }
        let read = levels.as_slice();
        assert_eq!(read.len(), 11);
        let effects: Vec<u32> = (0..11).filter_map(|k| read.get(k)).map(|s| s.effect).collect();
        assert_eq!(effects, (1..=11).collect::<Vec<u32>>());
    }

    /// The cards in a filtered group scaled three times, presented with
    /// round corners; the UI squares them in its next frame, laid out as
    /// input comes but not yet presented. Input lands by the pixels the
    /// screen showed: the rounded corners of the frame presented, read
    /// through that frame's own layers, not the squared clips of the frame
    /// laid out since.
    #[test]
    fn input_before_a_frame_is_presented_lands_by_the_frame_shown() {
        let radius = crate::reactive::Signal::new(8.0_f32);
        let layer = |cards| filtered(scaled(3.0, cards));
        let squared = move |s: &mut crate::app::Scenario| s.act("squared", || radius.set(0.0));
        Sweep::new((9.825, 9.825), 1.0, (14.0, 12.0))
            .layers(1)
            .radius(radius)
            .run(layer, squared);
    }

    /// The cards in a filtered group scaled three times on a page taller
    /// than a texture: no canvas shows it, the capture draws it in bands,
    /// and input lands by the layers that capture was drawn with. A frame
    /// drawn after a capture shows nothing: the capture's layers go with it.
    #[test]
    fn on_a_page_taller_than_a_texture_input_reads_the_layers_its_capture_drew() {
        let layer = |cards| filtered(scaled(3.0, cards));
        let captured_then_drawn = |s: &mut crate::app::Scenario| {
            let drawn = s.presented();
            s.shot(false).expect("read back");
            let captured = s.presented();
            assert_eq!(captured.stages.len(), 1, "the capture's layer");
            // Other contents, another snapshot: another number, given as
            // it is published; the frame drawn before is never changed.
            assert_ne!(captured.frame, drawn.frame, "the capture is a frame of its own");
            assert!(drawn.stages.is_empty(), "the frame drawn before keeps its own");
            s.frame("drawn").expect("a frame");
        };
        Sweep::new((9.825, 9.825), 1.0, (14.0, 12.0))
            .layers(1)
            .grid((30, 20))
            .height(17_000)
            .run(layer, captured_then_drawn);
    }

    /// A window's capture (the devtools' screenshot) draws the frame aside:
    /// the frame on the timeline keeps the layers it was presented with,
    /// all of it, and the capture's go only with a copy, which no input
    /// stamped with that frame reads.
    #[test]
    fn a_capture_aside_leaves_the_frame_presented_as_it_was() {
        let layer = |cards| filtered(scaled(3.0, cards));
        let captured_aside = |s: &mut crate::app::Scenario| {
            let shown = s.presented();
            let stages = std::sync::Arc::clone(&shown.stages);
            let h = s.parts().1;
            h.compositor.capture(&mut h.gpu, None).expect("read back");
            assert!(
                !std::sync::Arc::ptr_eq(&h.compositor.presented().stages, &stages),
                "the capture drew other layers"
            );
            let now = h.compositor.on_screen().expect("presented");
            assert!(std::sync::Arc::ptr_eq(&now, &shown), "the same frame on screen");
            assert!(std::sync::Arc::ptr_eq(&now.stages, &stages), "with its own layers");
            s.frame("drawn").expect("a frame");
        };
        Sweep::new((9.825, 9.825), 1.0, (14.0, 12.0))
            .layers(1)
            .grid((30, 20))
            .height(17_000)
            .run(layer, captured_aside);
    }

    /// The cards in a scroll container in a filtered group scaled three
    /// times, at a scale of one and a half, the container scrolled across
    /// by the compositor alone (the frame laid out before it): the cards
    /// move by whole device pixels, a fraction of a logical one, inside the
    /// layer, and input finds them where the screen shows them.
    #[test]
    fn cards_scrolled_in_a_scaled_layer_by_the_compositor_hit_where_the_gpu_shows_half() {
        let layer = |cards: crate::element::Div| {
            let scroller = div()
                .width(50.0)
                .height(70.0)
                .overflow_x(Overflow::Scroll)
                .child(div().width(70.0).padding_left(10.0).child(cards));
            filtered(scaled(3.0, scroller))
        };
        let scrolled = |s: &mut crate::app::Scenario| {
            let before = s.presented().scroll.iter().map(|o| o.applied.0).sum::<f32>();
            assert!(
                s.compositor_wheel((60.0, 30.0), (1.0, 0.0), 1.0),
                "the container scrolls"
            );
            let after = s.presented().scroll.iter().map(|o| o.applied.0).sum::<f32>();
            let moved = (after - before).abs() * 1.5;
            assert!(
                moved > 0.5 && (moved - moved.round()).abs() < 1e-3 && (after - before).fract().abs() > 0.1,
                "scrolled {} logical, whole device pixels",
                after - before
            );
        };
        Sweep::new((9.825, 9.6), 1.5, (36.0, 20.0))
            .layers(1)
            .run(layer, scrolled);
    }

    /// A group zoomed 100,000 times about (512, 512), holding a box that
    /// clips and a field in it: a point in it hits. A logical step there is
    /// a hundred-thousandth of a pixel, below what f32 tells apart at 512;
    /// the clip's scale is read from the transform, not measured.
    #[test]
    fn a_click_in_a_group_zoomed_a_hundred_thousand_times_hits_inside_its_clip() {
        let root = div()
            .width(1024.0)
            .height(1024.0)
            .child(
                div()
                    .width(1024.0)
                    .height(1024.0)
                    .padding_left(500.0)
                    .padding_top(500.0)
                    .scale(100_000.0)
                    .child(
                        div()
                            .width(24.0)
                            .height(24.0)
                            .overflow(Overflow::Hidden)
                            .child(div().width(24.0).height(24.0).on_click(|_| {})),
                    ),
            )
            .into_element();
        let mut text = TextSystem::new();
        let ui = crate::interact::InputState::default();
        let f = layout(
            &root,
            1024.0,
            1024.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            &mut text,
        );
        let field = (0..f.nodes.len())
            .rev()
            .find(|&i| f.behaviour(i).is_some_and(|b| b.handlers.click.is_some()))
            .expect("the field");
        for p in [(512.3, 512.3), (100.0, 900.0), (1000.0, 20.0)] {
            assert_eq!(crate::interact::hit(&f, p.0, p.1), Some(field), "{p:?}");
        }
    }

    /// A box of no width that clips, in a group scaled two and a half
    /// times, a field wider than it: the box keeps nothing, so no point
    /// hits the field, not even one whose pixel's centre lies on the line
    /// it collapsed to (by distance, half kept).
    #[test]
    fn a_collapsed_clip_in_a_scaled_group_hits_nothing() {
        let root = div()
            .width(200.0)
            .height(100.0)
            .child(
                div()
                    .width(40.0)
                    .height(40.0)
                    .padding_left(9.0)
                    .scale(2.5)
                    .origin(crate::style::Origin::At([
                        crate::style::Calc::fraction(0.0),
                        crate::style::Calc::fraction(0.0),
                    ]))
                    .child(
                        div()
                            .width(0.0)
                            .height(40.0)
                            .overflow(Overflow::Hidden)
                            .child(div().margin_left(-10.0).width(20.0).height(40.0).on_click(|_| {})),
                    ),
            )
            .into_element();
        let mut text = TextSystem::new();
        let ui = crate::interact::InputState::default();
        let f = layout(
            &root,
            200.0,
            100.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            &mut text,
        );
        let field = (0..f.nodes.len())
            .rev()
            .find(|&i| f.behaviour(i).is_some_and(|b| b.handlers.click.is_some()))
            .expect("the field");
        // The pixel 22..23's centre, 22.5, is 9 in the group: on the line.
        for x in [22.6, 22.9, 21.0, 24.0] {
            assert_ne!(crate::interact::hit(&f, x, 50.0), Some(field), "x {x}");
        }
    }

    /// Two coincident square clips in a group scaled twice about its corner
    /// at 9.8: their boxes snap to 10, so their left edge lands at device
    /// x = 10.2, and the pixel 10..11 lies 0.3 device pixels inside. The
    /// shaders (by distance, in device pixels: the clip's whole scale)
    /// keep 0.8 of it each, 0.64 in all, and a click there hits. Measured
    /// in the clip's own pixels (the display scale alone) it would be 0.65
    /// each, 0.42, a miss where the screen shows the field.
    #[test]
    fn a_click_by_two_edges_in_a_group_scaled_twice_hits_as_the_shaders_cover() {
        let square =
            |inner: crate::element::Div| div().width(60.0).height(60.0).overflow(Overflow::Hidden).child(inner);
        let field = div().margin_left(-10.0).width(80.0).height(60.0).on_click(|_| {});
        let root = div()
            .width(200.0)
            .height(200.0)
            .padding_left(9.8)
            .padding_top(20.0)
            .child(scaled(2.0, square(square(field))))
            .into_element();
        let mut text = TextSystem::new();
        let ui = crate::interact::InputState::default();
        let f = layout(
            &root,
            200.0,
            200.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            &mut text,
        );
        let field = crate::interact::hit(&f, 60.0, 60.0).expect("the field, inside");
        // Each clip as the renderer's entry has it: the map from the
        // canvas into its space, and its scale out of it.
        let entry = super::entry_coverage(
            [10.0, 20.0, 60.0, 60.0],
            [0.0; 4],
            crate::shape::ROUND,
            [0.5, 0.0, 0.0, 0.5, 4.9, 10.0],
            2.0,
            [10.5, 50.5],
        );
        assert!((entry - 0.8).abs() < 1e-5, "the shaders keep {entry} for each");
        assert_eq!(crate::interact::hit(&f, 10.5, 50.0), Some(field), "0.64 shown");
        assert_ne!(crate::interact::hit(&f, 9.9, 50.0), Some(field), "nothing shown");
    }

    /// The compositor's wheel at a box in a group zoomed 100,000 times
    /// about (512, 512): it routes by the clip as the shaders keep it, read
    /// from the scene's transforms, and the container under the point
    /// scrolls. A pixel's step there is a hundred-thousandth of a logical
    /// one, below what f32 tells apart at 512: measured, it rounds to
    /// nothing and the wheel would go nowhere.
    #[test]
    fn a_wheel_in_a_group_zoomed_a_hundred_thousand_times_routes_inside_its_clip() {
        let tree = || {
            div()
                .width(1024.0)
                .height(1024.0)
                .child(
                    div()
                        .width(1024.0)
                        .height(1024.0)
                        .padding_left(500.0)
                        .padding_top(500.0)
                        .scale(100_000.0)
                        .child(
                            div().width(24.0).height(24.0).overflow(Overflow::Hidden).child(
                                div()
                                    .width(24.0)
                                    .height(24.0)
                                    .overflow_y(Overflow::Scroll)
                                    .child(div().width(24.0).height(48.0)),
                            ),
                        ),
                )
                .into_element()
        };
        let mut s = crate::app::Scenario::measuring(tree, 1024, 1024, 1.0).expect("a scenario");
        s.settle().expect("settled");
        s.frame("shown").expect("a frame");
        assert!(
            s.compositor_wheel((512.3, 512.3), (0.0, 10.0), 1.0),
            "the container scrolls"
        );
    }

    #[test]
    fn a_node_with_a_clip_box_but_no_clipper_hits_as_paint_draws_it_unclipped() {
        // Paint clips a node by its clipper (`paint.rs`) alone: a
        // box in `clip` without one cuts neither what it draws nor where
        // it is hit.
        let root = div()
            .width(200.0)
            .height(200.0)
            .child(div().width(100.0).height(100.0).on_click(|_| {}))
            .into_element();
        let mut text = TextSystem::new();
        let ui = crate::interact::InputState::default();
        let mut f = layout(
            &root,
            200.0,
            200.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            &mut text,
        );
        let field = crate::interact::hit(&f, 50.0, 50.0).expect("the field");
        let n = f.nodes.get_mut(field).expect("its node");
        n.set_clip(Some(crate::geometry::Bounds {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        }));
        n.clipper = None;
        assert_eq!(crate::interact::hit(&f, 50.0, 50.0), Some(field));
    }

    /// A box the caller does not take is not measured: what the caller
    /// takes is asked first, and only a box it takes has its clip chain
    /// walked at the point.
    #[test]
    fn a_box_the_caller_passes_over_is_not_measured() {
        use crate::geometry::Bounds;
        let mut props = crate::scene::Properties::default();
        props.clips.push(crate::scene::ClipNode {
            clip: crate::scene::Clip {
                bounds: Bounds {
                    x: 0.0,
                    y: 0.0,
                    width: 50.0,
                    height: 50.0,
                },
                ..crate::scene::Clip::default()
            },
            transform: 0,
            parent: 0,
        });
        let hit = crate::compositor::Hit {
            rect: Bounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            shape: crate::shape::Shape::default(),
            slop: crate::style::Edges::all(0.0),
            transform: 0,
            clip: 1,
            effect: 0,
            scroller: None,
            wheel: false,
            target: crate::compositor::HitTarget {
                key: crate::compositor::NodeKey(1),
                owner: None,
            },
            cursor: None,
            below_modal: false,
        };
        let walks = std::cell::Cell::new(0);
        let view = super::HitView {
            hits: std::slice::from_ref(&hit),
            index: std::sync::Arc::new(super::HitIndex::build(std::slice::from_ref(&hit))),
            props: &props,
            stages: &[],
            scale: 1.0,
            walks: Some(&walks),
            frame: super::FrameKey::props(&props, 1.0),
            viewport: (0.0, 0.0),
        };
        assert!(view.hit((10.0, 10.0), |_| false).is_none());
        assert_eq!(walks.get(), 0, "passed over unmeasured");
        assert!(view.hit((10.0, 10.0), |_| true).is_some(), "it is there");
        assert_eq!(walks.get(), 1, "taken, it is measured");
    }

    /// A box drawn through twelve layers one in another, their composites
    /// alternately doubling and halving, each clipped by an edge between
    /// texels of its own: every level is followed, each texel of a level
    /// measured once (the samples about the point share them), so the
    /// points measured are near the texels the twelve levels hold under
    /// the point, not four to the twelfth; and what it shows is what each
    /// sample measured anew shows.
    #[test]
    fn twelve_resampling_layers_measure_each_texel_once() {
        const N: usize = 12;
        let maps: Vec<f32> = (0..N).map(|k| if k % 2 == 0 { 2.0 } else { 0.5 }).collect();
        // Each level's pixels to the canvas's: the canvas 1, a level inside
        // a composite of `m` has 1 / m of the next's.
        let mut factor = [1.0_f32; N + 1];
        for k in (0..N).rev() {
            factor[k] = factor[k + 1] / maps[k];
        }
        use crate::geometry::Bounds;
        // Edges on whole pixels of each level's own (paint snaps them),
        // a pixel of the canvas past them: the composites spread them.
        let bounds = |f: f32| Bounds {
            x: (10.0 * f).floor(),
            y: (20.0 * f).floor(),
            width: 1000.0,
            height: 1000.0,
        };
        let mut props = crate::scene::Properties::default();
        for (k, f) in factor.iter().enumerate() {
            let next = if k < N { u32::try_from(k + 2).unwrap_or(0) } else { 0 };
            props.clips.push(crate::scene::ClipNode {
                clip: crate::scene::Clip {
                    bounds: bounds(*f),
                    ..crate::scene::Clip::default()
                },
                transform: 0,
                parent: next,
            });
        }
        for k in 0..N {
            props.effects.push(crate::scene::EffectNode {
                parent: u32::try_from(k + 2).unwrap_or(0) * u32::from(k + 1 < N),
                ..crate::scene::EffectNode::default()
            });
        }
        let stages: Vec<crate::scene::LayerStage> = (0..N)
            .map(|k| {
                let id = |v: usize| u32::try_from(v).unwrap_or(0);
                crate::scene::LayerStage {
                    effect: id(k + 1),
                    outer: if k + 1 < N { id(k + 2) } else { 0 },
                    transform: 0,
                    map: [maps[k], 0.0, 0.0, maps[k], 0.0, 0.0],
                    clip: id(k + 2),
                }
            })
            .collect();
        let hit = crate::compositor::Hit {
            rect: Bounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            shape: crate::shape::Shape::default(),
            slop: crate::style::Edges::all(0.0),
            transform: 0,
            clip: 1,
            effect: 1,
            scroller: None,
            wheel: false,
            target: crate::compositor::HitTarget {
                key: crate::compositor::NodeKey(1),
                owner: None,
            },
            cursor: None,
            below_modal: false,
        };
        let view = super::HitView {
            hits: std::slice::from_ref(&hit),
            index: std::sync::Arc::new(super::HitIndex::build(std::slice::from_ref(&hit))),
            props: &props,
            stages: &stages,
            scale: 1.0,
            walks: None,
            frame: super::FrameKey::props(&props, 1.0),
            viewport: (0.0, 0.0),
        };
        let (memo, visits) = view.keeps_with(&hit, (10.5, 20.5), true);
        let (anew, every) = view.keeps_with(&hit, (10.5, 20.5), false);
        let shown = memo.expect("measured");
        assert!(shown > 0.0 && shown < 1.0, "the edges cut it: {shown}");
        assert_eq!(memo, anew, "as each sample measured anew");
        assert!(visits <= 600, "{visits} points measured (each sample anew: {every})");
        assert!(
            every > 100 * visits,
            "the samples share texels: {every} against {visits}"
        );
    }

    /// What a hit costs at `at` in `presented`, the index made and the
    /// frame placed already: the candidates it examined (index nodes and
    /// entries), the transform chains it composed and the spaces it
    /// placed, and how many boxes' rectangles in their space hold the
    /// point (`k`), and whether it found a row or a control (a box in a
    /// space of its own; the page takes the pointer everywhere).
    fn hit_cost(presented: &crate::compositor::Presented, at: (f32, f32)) -> Cost {
        let (c, m, p) = (super::candidates(), super::composed(), super::placed());
        let hit = super::hit_in(presented, at, |_| true)
            .and_then(|t| presented.hits.iter().find(|h| h.target == t))
            .is_some_and(|h| h.transform != 0);
        let since = |now: u64, then: u64| now.saturating_sub(then);
        let (candidates, composed, placed) = (
            since(super::candidates(), c),
            since(super::composed(), m),
            since(super::placed(), p),
        );
        let s = presented.scale;
        let k = presented
            .hits
            .iter()
            .filter(|h| {
                super::undone(
                    super::compose(&presented.properties, h.transform, 0),
                    (at.0 * s, at.1 * s),
                    s,
                )
                .is_some_and(|(p, _)| {
                    let off = presented.properties.transform(h.transform).offset;
                    let r = h.rect;
                    let q = (p.0 - off.0, p.1 - off.1);
                    q.0 >= r.x - 1.0 && q.0 <= r.right() + 1.0 && q.1 >= r.y - 1.0 && q.1 <= r.bottom() + 1.0
                })
            })
            .count();
        Cost {
            candidates,
            composed,
            placed,
            k: u64::try_from(k).unwrap_or(u64::MAX),
            hit,
        }
    }

    /// What one hit cost ([`hit_cost`]).
    struct Cost {
        candidates: u64,
        composed: u64,
        placed: u64,
        k: u64,
        hit: bool,
    }

    /// The bound a hit is held to over `n` entries with `k` under the
    /// point: about [`super::FANOUT`] entries tested on each level of a
    /// tree of `ceil(log16 n)` levels, for the boxes and for the spaces,
    /// and as many again for each box under the point: `8·log2(n) + 16·k
    /// + 32`. A walk of the table is `n`.
    fn bound(n: usize, k: u64) -> u64 {
        let log2 = u64::from(usize::BITS.saturating_sub(n.leading_zeros()));
        log2.saturating_mul(8)
            .saturating_add(k.saturating_mul(16))
            .saturating_add(32)
    }

    /// A page of the cost test, its size, a point on a row or control and
    /// one on none.
    type Case = (
        &'static str,
        Box<dyn Fn() -> crate::Element>,
        (u32, u32),
        (f32, f32),
        (f32, f32),
    );

    /// Hits and misses over 20,000 boxes in each of three shapes of page
    /// (rows a pixel high and as wide as the page, rows 24 high and 1200
    /// wide, and controls each scaled and turned on its own, 20,000 spaces)
    /// examine `O(log N + k)` candidates and compose a few chains each:
    /// the boxes found in an R-tree of their space, the spaces in an
    /// R-tree of where they lie on the canvas, placed once a frame.
    #[test]
    fn a_hit_and_a_miss_cost_the_logarithm_of_the_boxes_and_the_spaces() {
        use crate::element::Div;
        const N: usize = 20_000;
        let rows = |w: f32, h: f32| {
            move || {
                div()
                    .width(w + 100.0)
                    .height(400.0)
                    .child(
                        div()
                            .width(w)
                            .height(400.0)
                            .overflow_y(Overflow::Scroll)
                            .children((0..N).map(|_| div().width(w).height(h).on_click(|_| {}))),
                    )
                    .into_element()
            }
        };
        let controls = || {
            let control = |i: usize| -> Div {
                div()
                    .width(8.0)
                    .height(8.0)
                    .scale(0.9)
                    .rotate(if i.is_multiple_of(2) { 10.0 } else { -10.0 })
                    .on_click(|_| {})
            };
            div()
                .width(1100.0)
                .height(400.0)
                .child(
                    div().width(1000.0).height(400.0).overflow_y(Overflow::Scroll).child(
                        div()
                            .width(1000.0)
                            .flex_wrap(true)
                            .children((0..N).map(|i| div().width(10.0).height(10.0).child(control(i)))),
                    ),
                )
                .into_element()
        };
        let cases: [Case; 3] = [
            (
                "rows 400x1",
                Box::new(rows(400.0, 1.0)),
                (500, 400),
                (200.0, 50.5),
                (450.0, 50.0),
            ),
            (
                "rows 1200x24",
                Box::new(rows(1200.0, 24.0)),
                (1300, 400),
                (600.0, 100.0),
                (1250.0, 100.0),
            ),
            (
                "controls",
                Box::new(controls),
                (1100, 400),
                (55.0, 35.0),
                (1050.0, 100.0),
            ),
        ];
        for (name, tree, (w, h), hit, miss) in cases {
            let mut s = crate::app::Scenario::measuring(tree, w, h, 1.0).expect("a scenario");
            s.frame("shown").expect("a frame");
            let p = s.presented();
            assert!(p.hits.len() >= N, "{name}: {} boxes", p.hits.len());
            // The index made and the frame placed at the first hit.
            assert!(hit_cost(&p, (1.0, 1.0)).placed > 0, "{name}: placed at the first hit");
            for (what, at, hits) in [("hit", hit, true), ("miss", miss, false)] {
                let Cost {
                    candidates,
                    composed,
                    placed,
                    k,
                    hit: found,
                } = hit_cost(&p, at);
                assert_eq!(found, hits, "{name}: a {what}");
                let most = bound(p.hits.len(), k);
                assert!(
                    candidates <= most,
                    "{name}: a {what} examined {candidates} candidates, at most {most} (k = {k})"
                );
                let chains = k.saturating_mul(2).saturating_add(4);
                assert!(
                    composed <= chains,
                    "{name}: a {what} composed {composed} chains (k = {k})"
                );
                assert_eq!(placed, 0, "{name}: the frame is placed once, not at each hit");
            }
        }
    }

    /// Over a page of scrolled rows, overlapping boxes, rounded ones,
    /// boxes with slop and groups scaled and turned, each in a frame and
    /// in the frame after the compositor scrolled the list alone (the same
    /// hit table, its spaces placed anew), a sweep of points finds through
    /// the index exactly the box a walk of the table, the topmost first,
    /// finds by the same rule.
    #[test]
    fn the_index_finds_what_a_walk_of_the_table_finds() {
        let tree = || {
            let boxed = |x: f32, y: f32, w: f32, h: f32| {
                div()
                    .position(crate::style::Position::Absolute)
                    .left(x)
                    .top(y)
                    .width(w)
                    .height(h)
                    .on_click(|_| {})
            };
            div()
                .width(400.0)
                .height(300.0)
                .child(
                    div()
                        .width(200.0)
                        .height(150.0)
                        .overflow_y(Overflow::Scroll)
                        .children((0..300).map(|i| {
                            let row = div()
                                .width(if i % 3 == 0 { 120.0 } else { 200.0 })
                                .height(if i % 2 == 0 { 3.0 } else { 7.5 })
                                .on_click(|_| {});
                            // Some rows scaled: spaces of their own that
                            // the scroll moves.
                            if i % 7 == 0 { row.scale(0.8).rotate(5.0) } else { row }
                        })),
                )
                .child(boxed(150.0, 100.0, 120.0, 80.0).radius(Corners::all(30.0)))
                .child(boxed(180.0, 120.0, 60.0, 60.0).rotate(30.0).scale(1.3))
                .child(
                    boxed(250.0, 150.0, 120.0, 120.0)
                        .rotate(-15.0)
                        .child(boxed(10.0, 10.0, 40.0, 40.0).scale(0.5).rotate(45.0)),
                )
                .child(boxed(300.0, 20.0, 50.0, 50.0).scale(0.0))
                .into_element()
        };
        let mut s = crate::app::Scenario::measuring(tree, 400, 300, 1.0).expect("a scenario");
        s.frame("shown").expect("a frame");
        let mut seed = 0x9e37_79b9_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            crate::convert::u32_to_f32(seed % 4_200_000) / 10_000.0 - 10.0
        };
        let mut compared = 0;
        for frame in 0..2 {
            if frame == 1 {
                assert!(s.compositor_wheel((50.0, 50.0), (0.0, 37.0), 1.0), "the list scrolls");
            }
            let p = s.presented();
            let view = super::HitView::of(&p, None);
            for _ in 0..4_000 {
                let at = (next(), next() * 0.75);
                let walked = p.hits.iter().rposition(|h| view.takes(h, at));
                let found = view.hit(at, |_| true).map(std::ptr::from_ref);
                assert_eq!(
                    found,
                    walked.and_then(|i| p.hits.get(i)).map(std::ptr::from_ref),
                    "frame {frame}, {at:?}"
                );
                compared += usize::from(walked.is_some());
            }
        }
        assert!(compared > 1_000, "{compared} points hit something");
    }

    /// A frame of one box that takes clicks, made 40 px square in its hit
    /// table, its property trees cut to the root's transform.
    fn one_box() -> crate::compositor::Presented {
        let mut s = crate::app::Scenario::measuring(
            || div().width(40.0).height(40.0).on_click(|_| {}).into_element(),
            64,
            64,
            1.0,
        )
        .expect("a scenario");
        s.frame("one box").expect("a frame");
        let mut p = (*s.presented()).clone();
        p.properties.transforms.truncate(1);
        p.hits = p
            .hits
            .iter()
            .map(|&h| crate::compositor::Hit {
                rect: crate::geometry::Bounds {
                    x: 0.0,
                    y: 0.0,
                    width: 40.0,
                    height: 40.0,
                },
                ..h
            })
            .collect();
        p
    }

    /// 20,000 spaces in a row, 50 px apart, each a box 40 px square: the
    /// space tree packed from that frame. A frame later they lie in two
    /// piles, 200 px apart; refit, every node of the tree would span both
    /// piles and a miss between them would test them all. The tree no
    /// longer fits and is packed again: the miss costs the logarithm.
    #[test]
    fn spaces_that_scattered_since_the_tree_was_packed_are_packed_again() {
        const N: u32 = 20_000;
        let mut p = one_box();
        let h = *p.hits.last().expect("the box");
        p.hits = (1..=N)
            .map(|transform| crate::compositor::Hit { transform, ..h })
            .collect();
        let root = *p.properties.transforms.first().expect("the root's transform");
        for i in 0..N {
            let mut t = root;
            t.offset.0 = crate::convert::u32_to_f32(i) * 50.0;
            p.properties.transforms.push(t);
        }
        let n = usize::try_from(N).unwrap_or(usize::MAX);
        let row = hit_cost(&p, (145.0, 10.0));
        assert!(!row.hit, "between two boxes of the row");
        assert!(
            row.candidates <= bound(n, 0),
            "a miss in the row tested {}",
            row.candidates
        );
        p.frame = p.frame.saturating_add(1);
        for (i, t) in p.properties.transforms.iter_mut().skip(1).enumerate() {
            t.offset.0 = if i % 2 == 0 { 0.0 } else { 200.0 };
        }
        let _ = hit_cost(&p, (1.0, 1.0));
        let piles = hit_cost(&p, (100.0, 10.0));
        assert!(!piles.hit, "between the two piles");
        assert!(
            piles.candidates <= bound(n, 0),
            "a miss between the piles tested {}, at most {}",
            piles.candidates,
            bound(n, 0)
        );
        assert!(hit_cost(&p, (210.0, 10.0)).hit, "a box of the second pile");
    }

    /// Review r10 (Codex 3): the row of [`spaces_that_scattered_since_the_tree_was_packed_are_packed_again`]
    /// with its last space left far off at x = 1e8 while the rest scatter
    /// into two piles. One distant space must not hide that the tree no
    /// longer fits where points fall: a miss between the piles costs the
    /// logarithm, counted.
    #[test]
    fn one_distant_space_does_not_hide_spaces_that_scattered() {
        const N: u32 = 20_000;
        let mut p = one_box();
        // A window the miss lies in: where points fall is what the tree's
        // cost is weighed over.
        p.viewport = (400.0, 100.0);
        let h = *p.hits.last().expect("the box");
        p.hits = (1..=N)
            .map(|transform| crate::compositor::Hit { transform, ..h })
            .collect();
        let root = *p.properties.transforms.first().expect("the root's transform");
        for i in 0..N {
            let mut t = root;
            t.offset.0 = if i + 1 == N {
                1e8
            } else {
                crate::convert::u32_to_f32(i) * 50.0
            };
            p.properties.transforms.push(t);
        }
        let n = usize::try_from(N).unwrap_or(usize::MAX);
        assert!(!hit_cost(&p, (145.0, 10.0)).hit, "between two boxes of the row");
        p.frame = p.frame.saturating_add(1);
        for (i, t) in p.properties.transforms.iter_mut().skip(1).take(n - 1).enumerate() {
            t.offset.0 = if i % 2 == 0 { 0.0 } else { 200.0 };
        }
        let _ = hit_cost(&p, (1.0, 1.0));
        let piles = hit_cost(&p, (145.0, 10.0));
        assert!(!piles.hit, "between the two piles");
        assert!(
            piles.candidates <= bound(n, 0),
            "a miss between the piles tested {}, at most {}",
            piles.candidates,
            bound(n, 0)
        );
    }

    /// Opus r10 I1: 20,000 boxes of 40 on a grid of 100 columns 50 apart,
    /// each in a space of its own, then shuffled among the grid's slots
    /// (slot `i * 7919 mod N`), one space left far off at x = -1e6. A miss
    /// between boxes costs the logarithm, counted.
    #[test]
    fn a_shuffle_with_one_far_space_is_packed_again() {
        const N: u32 = 20_000;
        let mut p = one_box();
        // The grid's window (100 columns and 200 rows, 50 apart).
        p.viewport = (5_000.0, 10_000.0);
        let mut h = *p.hits.last().expect("the box");
        h.rect = crate::geometry::Bounds {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 40.0,
        };
        p.hits = (1..=N)
            .map(|transform| crate::compositor::Hit { transform, ..h })
            .collect();
        let root = *p.properties.transforms.first().expect("the root's transform");
        let slot = |s: u32| {
            (
                crate::convert::u32_to_f32(s % 100) * 50.0,
                crate::convert::u32_to_f32(s / 100) * 50.0,
            )
        };
        for i in 0..N {
            let mut t = root;
            t.offset = slot(i);
            p.properties.transforms.push(t);
        }
        let n = usize::try_from(N).unwrap_or(usize::MAX);
        assert!(!hit_cost(&p, (2545.0, 4545.0)).hit, "between boxes of the grid");
        p.frame = p.frame.saturating_add(1);
        for (i, t) in (0_u32..).zip(p.properties.transforms.iter_mut().skip(1)) {
            t.offset = if i + 1 == N {
                (-1e6, 0.0)
            } else {
                slot(u32::try_from(u64::from(i) * 7919 % u64::from(N)).unwrap_or(0))
            };
        }
        let _ = hit_cost(&p, (1.0, 1.0));
        let miss = hit_cost(&p, (2545.0, 4545.0));
        assert!(!miss.hit, "between boxes of the grid");
        assert!(
            miss.candidates <= bound(n, 0),
            "a miss in the shuffled grid tested {}, at most {}",
            miss.candidates,
            bound(n, 0)
        );
    }

    /// Two frames of one hit table, hit by turns: in one its space lies on
    /// the canvas, in the other its bounds are not finite. Each frame keeps
    /// the space tree and the spaces mapped for every point it was placed
    /// with, so each finds what an index made fresh for it finds.
    #[test]
    fn frames_hit_by_turns_each_hit_as_a_fresh_index_hits_them() {
        let mut on = one_box();
        let mut h = *on.hits.last().expect("the box");
        h.transform = 1;
        on.hits = vec![h].into();
        on.properties.transforms.push(crate::scene::TransformNode::default());
        let mut off = on.clone();
        off.frame = on.frame.saturating_add(1);
        if let Some(t) = off.properties.transforms.get_mut(1) {
            t.offset.0 = f32::MAX;
        }
        let pick = |p: &crate::compositor::Presented| super::hit_in(p, (10.0, 10.0), |_| true);
        let fresh = |p: &crate::compositor::Presented| {
            let mut p = p.clone();
            p.hits = p.hits.to_vec().into();
            pick(&p)
        };
        let (want_on, want_off) = (fresh(&on), fresh(&off));
        assert!(want_on.is_some(), "the box is under the point");
        for turn in 0..3 {
            assert_eq!(pick(&on), want_on, "turn {turn}: on the canvas");
            assert_eq!(pick(&off), want_off, "turn {turn}: off it");
        }
    }
}
