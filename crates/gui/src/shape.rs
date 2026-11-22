//! A node's shape (ADR-0022, research §7.11): the outline its background,
//! border, ring, shadows, clip and hit testing all follow. Corner radii
//! are what a node asks for; the radii it gets are CSS's used values,
//! scaled down together when two corners on one side would overlap
//! (CSS Backgrounds 3 §5.5, "Overlapping Curves"), so a radius larger
//! than the box allows is a stadium and a step of a radius scale never
//! needs to know the box's size.
//!
//! A corner's curve is CSS's `corner-shape: superellipse(K)` (CSS Borders
//! 4, §"corner-shape"): `K = 1` is the round corner of `border-radius`,
//! `K = 2` CSS's `squircle`, `0` a bevel, a negative `K` a concave scoop
//! and `-∞` a notch. A corner of radius `r` and curve `K` is the curve
//! `x^n + y^n = 1` (`n = 2^|K|`) through the two points `r` from the
//! corner on its sides, centred on the corner's inner point (`K > 0`) or
//! on the corner itself (`K < 0`), as the draft's
//! `border-aligned corner clip-out path` traces it.

use crate::geometry::Bounds;
use crate::style::{Corners, Edges};
use crate::value::Convert;

/// The outline of a node's border box.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Shape {
    /// Square corners.
    #[default]
    Rect,
    /// Round corners of these radii (pixels), scaled down together where
    /// they overlap, as CSS does.
    Rounded(Corners),
    /// Ends as round as the box allows: half the short side, at any size.
    Stadium,
    /// Corners of these radii, each the superellipse of CSS's
    /// `corner-shape: superellipse(k)` (`k` 1 is round, 2 `squircle`).
    Superellipse { radii: Corners, k: f32 },
}

impl Corners {
    /// A radius as large as the box allows (CSS's `calc(infinity * 1px)`,
    /// Tailwind's `rounded-full`): the overlapping-curves rule brings it
    /// down to half the side it rounds.
    pub const FULL: f32 = 1.0e30;

    /// The radii of a shape spread `d` out from (or, negative, into) one of
    /// these radii, as a box shadow's spread grows its corners (CSS
    /// Backgrounds 3's spread rule, CSS Shapes 1's margin-box one, which
    /// Chrome follows; measured in Chrome 145): a radius at least `d` grows by
    /// `d`, a smaller one by `d·(1 + (r/d − 1)³)`, so a square corner stays
    /// square and a small radius grows smoothly from it. Tailwind's `ring-*`
    /// is such a spread.
    #[must_use]
    pub fn spread(self, d: f32) -> Self {
        let grow = |r: f32| {
            if d <= 0.0 || r >= d {
                (r + d).max(0.0)
            } else {
                let q = r / d - 1.0;
                d.mul_add(q.mul_add(q * q, 1.0), r)
            }
        };
        self.grown(grow)
    }

    /// The radii of an outline `d` out from (or, negative, into) one of
    /// these radii, as Chrome draws CSS `outline` with `outline-offset`
    /// (measured in Chrome 145): a round corner grows by `d`, a square one
    /// stays square.
    #[must_use]
    pub fn outset(self, d: f32) -> Self {
        self.grown(|r| if r > 0.0 { (r + d).max(0.0) } else { 0.0 })
    }

    fn grown(self, f: impl Fn(f32) -> f32) -> Self {
        Self {
            top_left: f(self.top_left),
            top_right: f(self.top_right),
            bottom_right: f(self.bottom_right),
            bottom_left: f(self.bottom_left),
        }
    }
}

impl Convert<Shape> for f32 {
    /// Round corners of this radius.
    fn convert(self) -> Shape {
        Shape::Rounded(Corners::all(self))
    }
}

impl Convert<Shape> for Corners {
    /// Round corners of these radii.
    fn convert(self) -> Shape {
        Shape::Rounded(self)
    }
}

/// The curve of CSS's `corner-shape: round`.
pub const ROUND: f32 = 1.0;
/// The curve of CSS's `corner-shape: squircle` (`superellipse(2)`).
pub const SQUIRCLE: f32 = 2.0;

/// The largest `|k|` a curve is drawn and hit-tested with: past it,
/// `x^n + y^n = 1` (`n = 2^|k|`, 64 here) is within a hundredth of a radius
/// of the square (or the notch) it tends to. The renderer
/// (`gpu/common.wgsl`'s `curve_n`) and the paths ([`crate::figure`]) use
/// it too, so what is painted and what takes the pointer are one curve.
pub const CURVE_LIMIT: f32 = 6.0;

/// The exponent `n = 2^|k|` of a corner of curve `k`, `|k|` bounded by
/// [`CURVE_LIMIT`].
#[must_use]
pub fn curve_exponent(k: f32) -> f32 {
    2.0_f32.powf(k.abs().min(CURVE_LIMIT))
}

impl Shape {
    /// CSS `corner-shape: squircle` corners of radius `radius`.
    #[must_use]
    pub const fn squircle(radius: f32) -> Self {
        Self::Superellipse {
            radii: Corners::all(radius),
            k: SQUIRCLE,
        }
    }

    /// The radii asked for, before the box scales them: zero for a
    /// rectangle, [`Corners::FULL`] for a stadium.
    #[must_use]
    pub fn radii(&self) -> Corners {
        match *self {
            Self::Rect => Corners::default(),
            Self::Rounded(r) | Self::Superellipse { radii: r, .. } => r,
            Self::Stadium => Corners::all(Corners::FULL),
        }
    }

    /// The corners' curve, CSS's superellipse parameter: 1 is round.
    #[must_use]
    pub fn curve(&self) -> f32 {
        match *self {
            Self::Superellipse { k, .. } => k,
            Self::Rect | Self::Rounded(_) | Self::Stadium => ROUND,
        }
    }

    /// Whether every corner is square.
    #[must_use]
    pub fn is_rect(&self) -> bool {
        self.radii().is_zero()
    }

    /// The same shape with `f` applied to its radii (a stadium's are
    /// [`Corners::FULL`] each, so a corner written stays and the others
    /// stay as round as the box allows).
    #[must_use]
    pub fn with_radii(self, f: impl FnOnce(&mut Corners)) -> Self {
        let mut r = self.radii();
        f(&mut r);
        match self {
            Self::Superellipse { k, .. } => Self::Superellipse { radii: r, k },
            Self::Rect | Self::Rounded(_) | Self::Stadium => Self::Rounded(r),
        }
    }

    /// The same shape with square corners on `sides` (a button inside a
    /// group, square where it meets its neighbours), a stadium included.
    #[must_use]
    pub fn square(self, sides: Edges<bool>) -> Self {
        if !(sides.top || sides.right || sides.bottom || sides.left) {
            return self;
        }
        self.with_radii(|r| {
            if sides.top {
                r.top_left = 0.0;
                r.top_right = 0.0;
            }
            if sides.right {
                r.top_right = 0.0;
                r.bottom_right = 0.0;
            }
            if sides.bottom {
                r.bottom_right = 0.0;
                r.bottom_left = 0.0;
            }
            if sides.left {
                r.top_left = 0.0;
                r.bottom_left = 0.0;
            }
        })
    }

    /// The radii a box `width` × `height` gets: each at least 0, then all
    /// scaled by `f = min(Lᵢ / Sᵢ)` when `f < 1`, where `Sᵢ` is the sum of
    /// the two radii on side `i` and `Lᵢ` that side's length (CSS
    /// Backgrounds 3 §5.5). Computed in `f64`, so [`Corners::FULL`] gives
    /// half the side exactly.
    #[must_use]
    pub fn used(&self, width: f32, height: f32) -> Corners {
        let r = self.radii().clamped();
        if r.is_zero() {
            return Corners::default();
        }
        let (w, h) = (f64::from(width.max(0.0)), f64::from(height.max(0.0)));
        let [tl, tr, br, bl] = [r.top_left, r.top_right, r.bottom_right, r.bottom_left].map(f64::from);
        let mut f = 1.0_f64;
        for (len, sum) in [(w, tl + tr), (h, tr + br), (w, br + bl), (h, bl + tl)] {
            if sum > 0.0 {
                f = f.min(len / sum);
            }
        }
        if f >= 1.0 {
            return r;
        }
        let scale = |v: f64| crate::convert::f64_to_f32(v * f);
        Corners {
            top_left: scale(tl),
            top_right: scale(tr),
            bottom_right: scale(br),
            bottom_left: scale(bl),
        }
    }

    /// Whether `(x, y)` lies inside this shape laid on `bounds` (hit
    /// testing follows the corners, as Chrome's does `border-radius`).
    #[must_use]
    pub fn contains(&self, bounds: Bounds, x: f32, y: f32) -> bool {
        if !bounds.contains(x, y) {
            return false;
        }
        let r = self.used(bounds.width, bounds.height);
        corners_contain(bounds, r, self.curve(), x, y)
    }
}

/// Whether a node of border box `bounds`, `shape` and `slop` takes the
/// pointer at `(x, y)`: inside its shape, grown by its slop on each side
/// (the corners grown with it).
#[must_use]
pub fn takes_pointer(shape: &Shape, bounds: Bounds, slop: Edges<f32>, x: f32, y: f32) -> bool {
    if slop == Edges::all(0.0) {
        return shape.contains(bounds, x, y);
    }
    let grown = Bounds {
        x: bounds.x - slop.left,
        y: bounds.y - slop.top,
        width: (bounds.width + slop.left + slop.right).max(0.0),
        height: (bounds.height + slop.top + slop.bottom).max(0.0),
    };
    if !grown.contains(x, y) {
        return false;
    }
    let r = shape.used(bounds.width, bounds.height);
    let grow = |r: f32, a: f32, b: f32| if r > 0.0 { (r + a.max(b)).max(0.0) } else { 0.0 };
    let r = Corners {
        top_left: grow(r.top_left, slop.top, slop.left),
        top_right: grow(r.top_right, slop.top, slop.right),
        bottom_right: grow(r.bottom_right, slop.bottom, slop.right),
        bottom_left: grow(r.bottom_left, slop.bottom, slop.left),
    };
    let r = Shape::Rounded(r).used(grown.width, grown.height);
    corners_contain(grown, r, shape.curve(), x, y)
}

/// Whether `(x, y)`, inside `bounds`, is inside its corners of used radii
/// `r` and curve `k`.
pub(crate) fn corners_contain(bounds: Bounds, r: Corners, k: f32, x: f32, y: f32) -> bool {
    let (x0, y0, x1, y1) = (bounds.x, bounds.y, bounds.right(), bounds.bottom());
    // Each corner: its radius, its point, and the way into the box.
    let corners = [
        (r.top_left, x0, y0, 1.0, 1.0),
        (r.top_right, x1, y0, -1.0, 1.0),
        (r.bottom_right, x1, y1, -1.0, -1.0),
        (r.bottom_left, x0, y1, 1.0, -1.0),
    ];
    corners.iter().all(|&(radius, cx, cy, sx, sy)| {
        if radius <= 0.0 {
            return true;
        }
        // From the corner into the box, in radii.
        let u = (x - cx) * sx / radius;
        let v = (y - cy) * sy / radius;
        if u >= 1.0 || v >= 1.0 {
            return true;
        }
        corner_inside(1.0 - u, 1.0 - v, k)
    })
}

/// Whether a point of a corner's square is inside the shape, `(u, v)` its
/// distance in radii from the corner's inner point toward the corner on
/// each axis (both in `0..=1`). The curve is the one painted: `|k|` past
/// [`CURVE_LIMIT`] (a square corner, a notch) is drawn and tested at it.
fn corner_inside(u: f32, v: f32, k: f32) -> bool {
    let n = curve_exponent(k);
    if k >= 0.0 {
        u.powf(n) + v.powf(n) <= 1.0
    } else {
        // Concave: outside the curve centred on the corner itself.
        (1.0 - u).powf(n) + (1.0 - v).powf(n) >= 1.0
    }
}

/// What a node paints beyond its box, background, border and ring, each
/// following its shape (research §7.2): what a stylesheet reaches with
/// `::before` and `::after`, as typed values.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Decoration {
    pub edge_bar: Option<EdgeBar>,
    pub inset: Option<InsetLayer>,
    pub overlay: Option<OverlayBorder>,
    pub backdrop: Option<Backdrop>,
}

/// A bar along one side of the border box, over the border, its full
/// length (sera's alert accent: `after:absolute after:-inset-y-px
/// after:-left-px after:w-0.5`). Painted with the node's own box, above
/// its border; the node's padding keeps its content off it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeBar {
    pub side: crate::element::Side,
    pub width: f32,
    pub color: crate::Color,
}

/// A box inside the padding box, `inset` in from it (as an absolute
/// pseudo-element's `inset-*` is), with a shape, fill, border and shadows
/// of its own, painted behind the node's content (a floating drawer's
/// panel: `before:absolute before:inset-2 before:-z-10
/// before:rounded-4xl before:border before:bg-popover before:shadow-xl`).
/// Its shape is resolved against its own box.
#[derive(Clone, Debug, PartialEq)]
pub struct InsetLayer {
    pub inset: Edges<f32>,
    pub shape: Shape,
    pub background: Option<crate::Color>,
    pub border: f32,
    pub border_color: Option<crate::Color>,
    pub shadows: crate::style::Shadows,
}

/// A border painted over the node's content and children, along its
/// padding box's edge and inner corners, blended with what lies below (an avatar's
/// `after:absolute after:inset-0 after:border after:mix-blend-darken`, so
/// the line darkens a light photo's edge and vanishes on a dark one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayBorder {
    pub width: f32,
    pub color: crate::Color,
    pub blend: Blend,
}

/// How a colour combines with what lies below (CSS `mix-blend-mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Blend {
    #[default]
    Normal,
    /// The darker of the two, per channel.
    Darken,
    /// The lighter of the two, per channel.
    Lighten,
}

/// What lies behind the node, blurred and saturated, shown inside its
/// shape and under its background (CSS `backdrop-filter: blur() saturate()`;
/// a translucent menu's `before:backdrop-blur-2xl before:backdrop-saturate-150`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Backdrop {
    /// The Gaussian's standard deviation in pixels (`blur(40px)`), 0 for none.
    pub blur: f32,
    /// `saturate(k)`: 1 keeps the colours.
    pub saturate: f32,
}

impl Backdrop {
    /// `backdrop-filter: blur(px)`.
    #[must_use]
    pub const fn blur(px: f32) -> Self {
        Self {
            blur: px,
            saturate: 1.0,
        }
    }

    /// Whether it changes nothing.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.blur <= 0.0 && (self.saturate - 1.0).abs() < 1e-6
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_(w: f32, h: f32) -> Bounds {
        Bounds {
            x: 0.0,
            y: 0.0,
            width: w,
            height: h,
        }
    }

    #[test]
    fn a_spread_grows_corners_as_chrome_does() {
        // Chrome 145, a 4 px spread (`box-shadow: 0 0 0 4px`): the outer
        // radii measured from its pixels were 0, ~3.3, 5.5, ~6.9 and 10
        // for radii 0, 1, 2, 3 and 6.
        let grown = |r: f32| Corners::all(r).spread(4.0).top_left;
        assert!(grown(0.0).abs() < 1e-6);
        assert!((grown(1.0) - 3.3125).abs() < 1e-4);
        assert!((grown(2.0) - 5.5).abs() < 1e-4);
        assert!((grown(3.0) - 6.9375).abs() < 1e-4);
        assert!((grown(6.0) - 10.0).abs() < 1e-4);
        // A negative spread shrinks, never below zero.
        assert!((Corners::all(6.0).spread(-2.0).top_left - 4.0).abs() < 1e-6);
        assert!(Corners::all(1.0).spread(-2.0).top_left.abs() < 1e-6);
    }

    #[test]
    fn an_outline_grows_round_corners_and_keeps_square_ones() {
        // Chrome 145, `outline: 2px solid; outline-offset: 2px`: radii 0,
        // 1, 2, 3 and 6 gave outer radii 0, 5, 6, 7 and 10.
        let r = Corners {
            top_left: 0.0,
            top_right: 1.0,
            bottom_right: 2.0,
            bottom_left: 6.0,
        }
        .outset(4.0);
        assert_eq!(
            r,
            Corners {
                top_left: 0.0,
                top_right: 5.0,
                bottom_right: 6.0,
                bottom_left: 10.0,
            }
        );
    }

    #[test]
    fn radii_that_fit_are_kept() {
        assert_eq!(Shape::Rounded(Corners::all(8.0)).used(100.0, 36.0), Corners::all(8.0));
        assert_eq!(Shape::Rect.used(10.0, 10.0), Corners::default());
    }

    #[test]
    fn overlapping_radii_all_scale_by_the_tightest_side() {
        // `rounded-4xl` (26 px) on a 36 px control: 26 + 26 > 36, so every
        // corner is multiplied by 36 / 52 and the control is a stadium.
        let r = Shape::Rounded(Corners::all(26.0)).used(120.0, 36.0);
        assert!((r.top_left - 18.0).abs() < 1e-5 && (r.bottom_right - 18.0).abs() < 1e-5);
        // Every corner scales, the ones on roomy sides too: CSS keeps the
        // ratios between radii.
        let r = Shape::Rounded(Corners {
            top_left: 30.0,
            top_right: 10.0,
            bottom_right: 10.0,
            bottom_left: 10.0,
        })
        .used(100.0, 25.0);
        assert!((r.top_left - 18.75).abs() < 1e-5, "{r:?}");
        assert!((r.top_right - 6.25).abs() < 1e-5, "{r:?}");
    }

    #[test]
    fn a_radius_may_pass_half_a_side_when_its_neighbour_is_small() {
        // CSS does not clamp a radius to half the side: only the sum counts.
        let r = Shape::Rounded(Corners {
            top_left: 20.0,
            ..Corners::default()
        })
        .used(100.0, 30.0);
        assert_eq!(r.top_left, 20.0);
    }

    #[test]
    fn a_stadium_is_half_the_short_side_at_any_size() {
        for (w, h) in [(100.0, 36.0), (36.0, 100.0), (20.0, 20.0), (1e6, 3.0)] {
            let r = Shape::Stadium.used(w, h);
            let half = f32::min(w, h) / 2.0;
            assert!((r.top_left - half).abs() <= half * 1e-6, "{w}x{h}: {r:?}");
        }
        let full = Shape::Rounded(Corners::all(Corners::FULL)).used(80.0, 30.0);
        assert!((full.top_right - 15.0).abs() < 1e-4);
    }

    #[test]
    fn squaring_a_stadium_keeps_the_other_ends_round() {
        let s = Shape::Stadium.square(Edges {
            right: true,
            ..Edges::default()
        });
        let r = s.used(100.0, 36.0);
        assert_eq!((r.top_right, r.bottom_right), (0.0, 0.0));
        assert!((r.top_left - 18.0).abs() < 1e-4 && (r.bottom_left - 18.0).abs() < 1e-4);
    }

    #[test]
    fn hit_testing_follows_the_corners() {
        let b = box_(40.0, 40.0);
        let s = Shape::Stadium;
        assert!(s.contains(b, 20.0, 20.0));
        assert!(!s.contains(b, 1.0, 1.0));
        assert!(s.contains(b, 20.0, 0.5));
        assert!(Shape::Rect.contains(b, 0.5, 0.5));
        // A squircle reaches further into its corner than a circle does.
        let at = (4.0, 4.0);
        assert!(!Shape::Rounded(Corners::all(20.0)).contains(b, at.0, at.1));
        assert!(Shape::squircle(20.0).contains(b, at.0, at.1));
    }

    #[test]
    fn hit_slop_grows_the_target() {
        let b = box_(10.0, 10.0);
        let slop = Edges::all(8.0);
        assert!(!takes_pointer(&Shape::Rect, b, Edges::all(0.0), -4.0, 5.0));
        assert!(takes_pointer(&Shape::Rect, b, slop, -4.0, 5.0));
        assert!(!takes_pointer(&Shape::Rect, b, slop, -9.0, 5.0));
        // A round thumb stays round, larger.
        assert!(!takes_pointer(&Shape::Stadium, b, slop, -7.0, -7.0));
        assert!(takes_pointer(&Shape::Stadium, b, slop, 5.0, -7.5));
    }

    #[test]
    fn curves_of_the_draft() {
        // Bevel: the corner's diagonal.
        assert!(corner_inside(0.49, 0.49, 0.0));
        assert!(!corner_inside(0.51, 0.51, 0.0));
        // Round: the unit circle.
        assert!(corner_inside(0.7, 0.7, 1.0));
        assert!(!corner_inside(0.72, 0.72, 1.0));
        // A scoop keeps less than a bevel; a notch keeps nothing of the square.
        assert!(!corner_inside(0.4, 0.4, -1.0));
        assert!(!corner_inside(0.1, 0.1, f32::NEG_INFINITY));
        // Past the limit, the curve tested is the one painted.
        assert_eq!(
            corner_inside(0.99, 0.5, f32::INFINITY),
            corner_inside(0.99, 0.5, CURVE_LIMIT)
        );
    }
}
