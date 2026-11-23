//! A floating part's place beside its anchor (reka's `Popper`), on `gui`'s
//! [`gui::Positioning`] and [`gui::Placement`].
//!
//! Layout does what floating-ui does for reka: it flips to the opposite side
//! when the preferred one has no room (never to a perpendicular side), keeps
//! the part on screen, bounds its size, follows the anchor, and reports the
//! side, alignment, room, the anchor's box and the part's own
//! ([`gui::Placement`], which the look reads in place of `data-side` and
//! `--reka-popper-*`). The primitive holds reka's defaults and what reka
//! computes from that result:
//!
//! - the gap to the anchor is the side offset plus the arrow's height;
//! - the arrow ([`Popper::arrow_at`], floating-ui's `arrow` middleware): on
//!   the part's edge facing the anchor, its centre on the anchor's centre,
//!   kept `arrow_padding` from the part's corners, and hidden (keeping its
//!   place) when it cannot point at the anchor's centre;
//! - the transform origin ([`Popper::transform_origin`], reka's
//!   `transformOrigin` middleware): the arrow's tip while the arrow shows,
//!   else the point of the edge facing the anchor where the part is
//!   aligned (start, centre, end; the start and end of a top or bottom edge
//!   swap right to left).

use gui::{Direction, Placement, Positioning, Side, SideAlign};

/// reka's arrow, width by height (`shared/component/Arrow.vue`).
pub const ARROW: (f32, f32) = (10.0, 5.0);

/// Where a floating part asks to go, with reka's defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Popper {
    /// The preferred side (reka: bottom).
    pub side: Side,
    /// The gap to the anchor, arrow aside (reka: 0).
    pub side_offset: f32,
    /// How it lines up along the side (reka: center).
    pub align: SideAlign,
    /// The arrow's width and height, when it has one.
    pub arrow: Option<(f32, f32)>,
    /// At least as wide as the anchor (a select's or combobox's list).
    pub match_width: bool,
    /// At most as tall as the room on its side (a menu's or select's
    /// `max-h` of the available height).
    pub fit_height: bool,
    /// The least room between the arrow and the part's corners (reka: 0).
    pub arrow_padding: f32,
    /// Hide the arrow when it cannot point at the anchor's centre (reka:
    /// true).
    pub hide_shifted_arrow: bool,
}

/// Where the arrow goes on a placed part (floating-ui's `arrow` data).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArrowPlace {
    /// The part's edge it stands on: the one facing the anchor, opposite
    /// the side the part opened on. Its base lies on that edge and its tip
    /// points away from the part, the arrow's height out.
    pub edge: Side,
    /// Where its box starts along that edge, from the part's left (top or
    /// bottom edge) or top (left or right edge): reka's `arrowX`/`arrowY`.
    pub offset: f32,
    /// It cannot point at the anchor's centre (the part was kept on screen
    /// past it) and is hidden, keeping its place (`visibility: hidden`).
    pub hidden: bool,
}

impl Default for Popper {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Popper {
    /// reka's defaults.
    pub const DEFAULT: Self = Self {
        side: Side::Bottom,
        side_offset: 0.0,
        align: SideAlign::Center,
        arrow: None,
        match_width: false,
        fit_height: false,
        arrow_padding: 0.0,
        hide_shifted_arrow: true,
    };

    #[must_use]
    pub const fn side(self, side: Side) -> Self {
        Self { side, ..self }
    }

    #[must_use]
    pub const fn side_offset(self, side_offset: f32) -> Self {
        Self { side_offset, ..self }
    }

    #[must_use]
    pub const fn align(self, align: SideAlign) -> Self {
        Self { align, ..self }
    }

    /// With reka's arrow.
    #[must_use]
    pub const fn arrow(self) -> Self {
        Self {
            arrow: Some(ARROW),
            ..self
        }
    }

    #[must_use]
    pub const fn match_width(self, match_width: bool) -> Self {
        Self { match_width, ..self }
    }

    #[must_use]
    pub const fn fit_height(self, fit_height: bool) -> Self {
        Self { fit_height, ..self }
    }

    #[must_use]
    pub const fn arrow_padding(self, arrow_padding: f32) -> Self {
        Self { arrow_padding, ..self }
    }

    #[must_use]
    pub const fn hide_shifted_arrow(self, hide_shifted_arrow: bool) -> Self {
        Self {
            hide_shifted_arrow,
            ..self
        }
    }

    /// The arrow on a part placed as `placed` says; none without an arrow.
    ///
    /// floating-ui's rule: the arrow's centre goes on the anchor's centre,
    /// clamped to the part's edge less `arrow_padding` at each end (at most
    /// half the edge less half the arrow less 1 px, so a short edge still
    /// holds it); where clamping moved it, it cannot point at the centre.
    #[must_use]
    pub fn arrow_at(&self, placed: &Placement) -> Option<ArrowPlace> {
        let (width, _) = self.arrow?;
        let (part, anchor) = (placed.bounds, placed.anchor);
        let (start, len, anchor_start, anchor_len) = match placed.side {
            Side::Top | Side::Bottom => (part.x, part.width, anchor.x, anchor.width),
            Side::Left | Side::Right => (part.y, part.height, anchor.y, anchor.height),
        };
        let center = anchor_start + anchor_len / 2.0 - start - width / 2.0;
        let largest = len / 2.0 - width / 2.0 - 1.0;
        let padding = self.arrow_padding.min(largest);
        let (min, max) = (padding, len - width - padding);
        let offset = center.min(max).max(min);
        Some(ArrowPlace {
            edge: opposite(placed.side),
            offset,
            #[expect(
                clippy::float_cmp,
                reason = "floating-ui's `centerOffset !== 0`: the clamp moved it or not"
            )]
            hidden: self.hide_shifted_arrow && offset != center,
        })
    }

    /// The point a part placed as `placed` grows from and shrinks to, from
    /// its top-left corner (`--reka-popper-transform-origin`): the arrow's
    /// tip while the arrow shows, else where the part is aligned on its
    /// edge facing the anchor, start and end of a top or bottom edge
    /// swapping right to left.
    #[must_use]
    pub fn transform_origin(&self, placed: &Placement, direction: Direction) -> (f32, f32) {
        // An arrow that cannot point at the centre is no origin, shown or
        // not (reka's `transformOrigin` reads `centerOffset` alone).
        let arrow = self
            .hide_shifted_arrow(true)
            .arrow_at(placed)
            .filter(|a| !a.hidden)
            .zip(self.arrow);
        let (w, h) = (placed.bounds.width, placed.bounds.height);
        let rtl = direction == Direction::Rtl;
        let share = |align: SideAlign, flips: bool| match (align, flips && rtl) {
            (SideAlign::Center, _) => 0.5,
            (SideAlign::Start, false) | (SideAlign::End, true) => 0.0,
            (SideAlign::End, false) | (SideAlign::Start, true) => 1.0,
        };
        let (tip_along, tip_out) = match arrow {
            Some((a, (aw, ah))) => (Some(a.offset + aw / 2.0), ah),
            None => (None, 0.0),
        };
        match placed.side {
            Side::Bottom => (tip_along.unwrap_or(w * share(placed.align, true)), -tip_out),
            Side::Top => (tip_along.unwrap_or(w * share(placed.align, true)), h + tip_out),
            Side::Right => (-tip_out, tip_along.unwrap_or(h * share(placed.align, false))),
            Side::Left => (w + tip_out, tip_along.unwrap_or(h * share(placed.align, false))),
        }
    }

    /// The gap to the anchor: the side offset plus the arrow's height.
    #[must_use]
    pub fn gap(&self) -> f32 {
        self.side_offset + self.arrow.map_or(0.0, |(_, h)| h)
    }

    /// What layout is asked.
    #[must_use]
    pub fn positioning(&self) -> Positioning {
        Positioning::Anchored {
            side: self.side,
            align: self.align,
            offset: self.gap(),
            match_width: self.match_width,
            fit_height: self.fit_height,
        }
    }
}

const fn opposite(side: Side) -> Side {
    match side {
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

#[cfg(test)]
mod tests {
    use gui::{Bounds, Direction, Placement, Positioning, Side, SideAlign};

    use super::{ArrowPlace, Popper};

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds {
        Bounds { x, y, width, height }
    }

    /// A 100 × 40 part below a 40 px wide anchor at x 100, its left at `x`.
    fn below(x: f32, align: SideAlign) -> Placement {
        Placement {
            side: Side::Bottom,
            align,
            available: (800.0, 400.0),
            anchor: bounds(100.0, 10.0, 40.0, 20.0),
            origin: (0.0, 0.0),
            bounds: bounds(x, 35.0, 100.0, 40.0),
        }
    }

    #[test]
    fn the_arrow_points_at_the_anchors_centre_or_hides() {
        let p = Popper::DEFAULT.arrow();
        // Centred: the anchor's centre (120) is the part's (70 + 50).
        assert_eq!(
            p.arrow_at(&below(70.0, SideAlign::Center)),
            Some(ArrowPlace {
                edge: Side::Top,
                offset: 45.0,
                hidden: false
            }),
            "its 10 px box starts 5 px before the centre, on the edge facing the anchor"
        );
        // Kept on screen far to the right, the part's left at 115: the
        // centre is 5 px in, less half the arrow is 0: still reachable.
        assert_eq!(
            p.arrow_at(&below(115.0, SideAlign::Start)).map(|a| a.hidden),
            Some(false)
        );
        // Further: the centre lies before the part's left edge.
        let shifted = p.arrow_at(&below(130.0, SideAlign::Start)).expect("an arrow");
        assert!(shifted.hidden, "it cannot point at the centre");
        assert!(shifted.offset.abs() < f32::EPSILON, "and is clamped to the start");
        assert!(
            !p.hide_shifted_arrow(false)
                .arrow_at(&below(130.0, SideAlign::Start))
                .expect("an arrow")
                .hidden
        );
        // Padding keeps it off the corners.
        let padded = p
            .arrow_padding(8.0)
            .arrow_at(&below(130.0, SideAlign::Start))
            .expect("an arrow");
        assert!((padded.offset - 8.0).abs() < f32::EPSILON);
        assert_eq!(Popper::DEFAULT.arrow_at(&below(70.0, SideAlign::Center)), None);
    }

    #[test]
    fn the_transform_origin_is_the_arrow_tip_or_the_aligned_edge() {
        let p = Popper::DEFAULT.arrow();
        assert_eq!(
            p.transform_origin(&below(70.0, SideAlign::Center), Direction::Ltr),
            (50.0, -5.0),
            "the tip, 5 px above the top edge"
        );
        let mut top = below(70.0, SideAlign::Center);
        top.side = Side::Top;
        assert_eq!(p.transform_origin(&top, Direction::Ltr), (50.0, 45.0));
        // Hidden or absent arrow: the aligned point of the facing edge.
        assert_eq!(
            p.transform_origin(&below(130.0, SideAlign::Start), Direction::Ltr),
            (0.0, 0.0)
        );
        assert_eq!(
            p.transform_origin(&below(130.0, SideAlign::Start), Direction::Rtl),
            (100.0, 0.0),
            "start is the right end of a bottom edge, right to left"
        );
        let mut right = below(70.0, SideAlign::End);
        right.side = Side::Right;
        assert_eq!(
            Popper::DEFAULT.transform_origin(&right, Direction::Rtl),
            (0.0, 40.0),
            "a side edge does not swap"
        );
    }

    #[test]
    fn reka_defaults_and_the_arrow_in_the_gap() {
        assert_eq!(
            Popper::DEFAULT.positioning(),
            Positioning::Anchored {
                side: Side::Bottom,
                align: SideAlign::Center,
                offset: 0.0,
                match_width: false,
                fit_height: false,
            }
        );
        let p = Popper::DEFAULT.side(Side::Top).side_offset(4.0).arrow();
        assert!((p.gap() - 9.0).abs() < f32::EPSILON, "4 px plus the 5 px arrow");
    }
}
