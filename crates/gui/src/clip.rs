//! What a clip keeps, on the CPU, as the shaders keep it: one rule for the
//! renderer's clip chains and for hit testing.
//!
//! A clip is a rounded rectangle in its own space; a clip chain is a clip
//! and those it lies in, each tested in its own space and their coverages
//! multiplied (Chrome's clip-node tree; Flutter's nested `ClipRRect`s).
//! The chain the shaders walk is the *effective* one ([`needed`]): an
//! ancestor that cannot change a pixel the clip keeps is left out of it.
//! [`coverage`] is `shape_cov` of `gpu/common.wgsl`, ported: what paints
//! a pixel, and so what a point hits.

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

/// Whether an ancestor clip is part of the effective chain of a clip whose
/// rectangle covers `region` (x0, y0, x1, y1, in the space both are
/// compared in): one that is turned or scaled against it, or rounded, or
/// that cuts `region`, is; a square, axis-aligned one containing it keeps
/// every pixel the clip keeps, and is not. Its rectangle is `rect`
/// (x0, y0, x1, y1, that space). A rounded ancestor is kept whatever it
/// contains, so a rounded container moving over clips inside it changes
/// its own entry alone.
#[must_use]
pub(crate) fn needed(rect: [f32; 4], square: bool, aligned: bool, region: [f32; 4]) -> bool {
    !(aligned && square && rect[0] <= region[0] && rect[1] <= region[1] && rect[2] >= region[2] && rect[3] >= region[3])
}

/// The box clipper `owner` clips what lies in it to: its padding box (as
/// Chrome clips `overflow` to the padding box). Its corners are those its
/// clipped nodes carry ([`FrameNode::clip_radius`]).
#[must_use]
pub(crate) fn clip_box(owner: &FrameNode) -> Bounds {
    let w = owner.look.border;
    if w.top > 0.0 || w.right > 0.0 || w.bottom > 0.0 || w.left > 0.0 {
        owner.bounds.inset(w.top, w.right, w.bottom, w.left)
    } else {
        owner.bounds
    }
}

/// One clip of a chain as hit testing reads it: its rectangle and used
/// radii in its own space, its corners' curve, and the next clip up.
pub(crate) struct Link<K> {
    pub rect: Bounds,
    pub radius: Corners,
    pub curve: f32,
    pub next: Option<K>,
}

/// Whether the clip chain from `first` keeps point `(x, y)` (screen): it
/// paints at least half of a pixel centred there ([`chain_coverage`]),
/// one threshold on the whole chain's coverage. The one hit rule for the UI
/// ([`chain_keeps`]) and the compositor's wheel routing. `None` when
/// `to_local` maps nothing into a clip (a scale of 0).
pub(crate) fn keeps<K: Copy>(
    first: Option<K>,
    link: impl Fn(K) -> Option<Link<K>>,
    to_local: impl Fn(K, (f32, f32)) -> Option<(f32, f32)>,
    limit: usize,
    x: f32,
    y: f32,
) -> Option<bool> {
    Some(chain_coverage(first, link, to_local, limit, x, y)? >= 0.5)
}

/// How much of the pixel centred on screen point `(x, y)` the clip chain
/// from `first` keeps, as the shaders keep it: each clip ([`Link`], by
/// `link`) along the effective chain ([`needed`]) covers it in its own
/// space (`to_local`, what scrolls, moves, scales and turns undone,
/// [`coverage`]), and the chain keeps the product of those coverages.
pub(crate) fn chain_coverage<K: Copy>(
    first: Option<K>,
    link: impl Fn(K) -> Option<Link<K>>,
    to_local: impl Fn(K, (f32, f32)) -> Option<(f32, f32)>,
    limit: usize,
    x: f32,
    y: f32,
) -> Option<f32> {
    // The first clip's rectangle and key, for the effective chain.
    let mut leaf: Option<([f32; 4], K)> = None;
    let mut kept = 1.0_f32;
    let mut at = first;
    let mut steps = 0_usize;
    while let Some(k) = at {
        steps = steps.saturating_add(1);
        if steps > limit {
            break;
        }
        let Link {
            rect: b,
            radius,
            curve,
            next,
        } = link(k)?;
        let r = [b.x, b.y, b.right(), b.bottom()];
        let (cx, cy) = to_local(k, (x, y))?;
        let keep = match leaf {
            None => true,
            Some((g, one)) => match moved_by(&to_local, one, k, (x, y))? {
                // The leaf's rectangle in this clip's space.
                Some(d) => needed(
                    r,
                    radius.is_zero(),
                    true,
                    [g[0] + d.0, g[1] + d.1, g[2] + d.0, g[3] + d.1],
                ),
                None => true,
            },
        };
        if keep {
            let radii = [
                radius.top_left,
                radius.top_right,
                radius.bottom_right,
                radius.bottom_left,
            ];
            kept *= coverage([b.x, b.y, b.width, b.height], radii, curve, [cx, cy]);
            if kept <= 0.0 {
                return Some(0.0);
            }
        }
        if leaf.is_none() {
            leaf = Some((r, k));
        }
        at = next;
    }
    Some(kept)
}

/// Whether node `i`'s clip chain keeps point `(x, y)` ([`keeps`]): each
/// clip the padding box of the node that clips ([`clip_box`]), tested in
/// that node's space (`to_node`).
pub(crate) fn chain_keeps(
    frame: &crate::layout::Frame,
    i: usize,
    to_node: &impl Fn(usize, (f32, f32)) -> Option<(f32, f32)>,
    x: f32,
    y: f32,
) -> Option<bool> {
    // A key is a node; its clip is the one it lies in, its clipper's box
    // with its own clip corners.
    let link = |j: usize| {
        let n = frame.nodes.get(j)?;
        let k = n.clipper?;
        let owner = frame.nodes.get(k)?;
        Some(Link {
            rect: clip_box(owner),
            radius: n.clip_radius,
            curve: n.clip_curve,
            next: owner.clipper.map(|_| k),
        })
    };
    let to_local = |j: usize, p: (f32, f32)| to_node(frame.nodes.get(j)?.clipper?, p);
    let first = frame.nodes.get(i)?.clipper.map(|_| i);
    keeps(first, link, to_local, frame.nodes.len(), x, y)
}

/// How far clip `b`'s space lies from clip `a`'s near screen point `p`,
/// when one is the other moved (no scale or turn between them).
fn moved_by<K: Copy>(
    to_local: &impl Fn(K, (f32, f32)) -> Option<(f32, f32)>,
    a: K,
    b: K,
    p: (f32, f32),
) -> Option<Option<(f32, f32)>> {
    let delta = |q: (f32, f32)| -> Option<(f32, f32)> {
        let (u, v) = (to_local(a, q)?, to_local(b, q)?);
        Some((v.0 - u.0, v.1 - u.1))
    };
    let d = delta(p)?;
    let same = |e: (f32, f32)| (e.0 - d.0).abs() < 1e-3 && (e.1 - d.1).abs() < 1e-3;
    let moved = same(delta((p.0 + 64.0, p.1))?) && same(delta((p.0, p.1 + 64.0))?);
    Some(moved.then_some(d))
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

    #[test]
    fn a_click_on_two_coincident_rounded_edges_each_keeping_most_of_the_pixel_misses() {
        // A card rounded 8 that clips, holding a wrapper of its box rounded
        // 8 that clips, holding a field: at x = 20.1 each edge covers 60%
        // of the pixel, and the chain paints 36% of it, under half.
        let root = div()
            .width(200.0)
            .height(200.0)
            .padding_top(20.0)
            .padding_left(20.0)
            .child(
                div()
                    .width(100.0)
                    .height(100.0)
                    .radius(Corners::all(8.0))
                    .overflow(Overflow::Hidden)
                    .child(
                        div()
                            .width(100.0)
                            .height(100.0)
                            .radius(Corners::all(8.0))
                            .overflow(Overflow::Hidden)
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
        assert_ne!(crate::interact::hit(&f, 20.1, 70.0), Some(field), "36% of the pixel");
        assert_eq!(crate::interact::hit(&f, 20.6, 70.0), Some(field), "all of it");
    }
}
