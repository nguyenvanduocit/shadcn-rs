//! Where a layer goes ([`Positioning`]): beside its anchor, on the side
//! with room and kept on screen (floating-ui's `flip`, `shift` and `size`,
//! which reka's popper content uses), over its anchor with one of its parts
//! on the anchor's (reka's item-aligned select), at a point, or over the
//! window; and how it went ([`Placement`]), which a look reads.

use crate::element::{Positioning, Side, SideAlign};
use crate::geometry::Bounds;

/// How layout placed an anchored layer (floating-ui's result, as reka's
/// popper content exposes it: `data-side`, `data-align`,
/// `--reka-popper-available-width/height`, the anchor's rectangle and
/// `--reka-popper-transform-origin`). A look reads it through
/// [`crate::Deferred::placement`]; `gui` moves and scales a layer by it
/// itself ([`crate::style::Origin::Anchor`], `Slide::FromAnchor`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The side of the anchor it opened on, after flipping.
    pub side: Side,
    /// How it lines up with the anchor along that side.
    pub align: SideAlign,
    /// The room it may take, width and height: on its side, to the
    /// window's edge less the margin kept there; across, the window less
    /// both margins.
    pub available: (f32, f32),
    /// The anchor's border box in the window (a point, for a layer at one).
    pub anchor: Bounds,
    /// The point of the layer facing the anchor, from its top-left corner:
    /// the middle of the edge nearest the anchor for a centred layer, the
    /// edge's start or end for one aligned there.
    pub origin: (f32, f32),
}

/// Space kept between a placed layer and the window edge.
pub(crate) const EDGE_MARGIN: f32 = 8.0;
/// reka's `CONTENT_MARGIN`: the space an item-aligned layer keeps to the window edge.
const OVER_MARGIN: f32 = 10.0;

const fn opposite(side: Side) -> Side {
    match side {
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

/// The room on `side` of `anchor`, `offset` away from it, to the window's
/// edge less its margin.
fn room(side: Side, offset: f32, anchor: Bounds, viewport: Bounds) -> f32 {
    let room = match side {
        Side::Bottom => viewport.bottom() - EDGE_MARGIN - anchor.bottom() - offset,
        Side::Top => anchor.y - offset - viewport.y - EDGE_MARGIN,
        Side::Right => viewport.right() - EDGE_MARGIN - anchor.right() - offset,
        Side::Left => anchor.x - offset - viewport.x - EDGE_MARGIN,
    };
    room.max(0.0)
}

/// The side a layer of `size` opens on: `side` when it fits there, else the
/// opposite one when that fits, else the one with more room (floating-ui's
/// `flip` with its `bestFit` fallback).
fn flip(side: Side, offset: f32, anchor: Bounds, size: (f32, f32), viewport: Bounds) -> Side {
    let len = |s: Side| match s {
        Side::Top | Side::Bottom => size.1,
        Side::Left | Side::Right => size.0,
    };
    let other = opposite(side);
    let (here, there) = (
        room(side, offset, anchor, viewport),
        room(other, offset, anchor, viewport),
    );
    if len(side) <= here || (len(other) > there && here >= there) {
        side
    } else {
        other
    }
}

/// How tall a layer of `size` placed by `placement` may be, when it is less
/// than it would take: the room on the side it opens on, for one that fits
/// its height to it (floating-ui's `size`).
pub(crate) fn fitted_height(
    placement: Positioning,
    anchor: Option<Bounds>,
    size: (f32, f32),
    viewport: Bounds,
) -> Option<f32> {
    let Positioning::Anchored {
        side,
        offset,
        fit_height: true,
        ..
    } = placement
    else {
        return None;
    };
    let anchor = anchor?;
    let side = flip(side, offset, anchor, size, viewport);
    let room = match side {
        Side::Top | Side::Bottom => room(side, offset, anchor, viewport),
        Side::Left | Side::Right => return None,
    };
    (size.1 > room).then_some(room)
}

/// Where a layer of `size` placed by `placement` goes (not [`Positioning::Over`],
/// which [`over`] places).
pub(crate) fn position(
    placement: Positioning,
    anchor: Option<Bounds>,
    size: (f32, f32),
    viewport: Bounds,
) -> (f32, f32) {
    let (w, h) = size;
    let clamp = |v: f32, len: f32, max: f32| v.min(max - len - EDGE_MARGIN).max(EDGE_MARGIN);
    match placement {
        Positioning::Fill | Positioning::Over { .. } => (0.0, 0.0),
        Positioning::At { x, y } => (clamp(x, w, viewport.width), clamp(y, h, viewport.height)),
        Positioning::Anchored {
            side, align, offset, ..
        } => {
            let a = anchor.unwrap_or_default();
            let cross = |start: f32, len: f32, own: f32| match align {
                SideAlign::Start => start,
                SideAlign::Center => start + (len - own) / 2.0,
                SideAlign::End => start + len - own,
            };
            let (x, y) = match flip(side, offset, a, size, viewport) {
                Side::Bottom => (cross(a.x, a.width, w), a.bottom() + offset),
                Side::Top => (cross(a.x, a.width, w), a.y - offset - h),
                Side::Right => (a.right() + offset, cross(a.y, a.height, h)),
                Side::Left => (a.x - offset - w, cross(a.y, a.height, h)),
            };
            (clamp(x, w, viewport.width), clamp(y, h, viewport.height))
        }
    }
}

/// How a layer of `size` placed by `placement` went: an anchored layer's
/// side after flipping and its alignment, and a layer at a point opens to
/// its right, as reka's context menu does; `None` for the rest.
pub(crate) fn placed(
    placement: Positioning,
    anchor: Option<Bounds>,
    size: (f32, f32),
    viewport: Bounds,
) -> Option<Placement> {
    let (side, align, offset, anchor) = match placement {
        Positioning::Anchored {
            side, align, offset, ..
        } => {
            let a = anchor?;
            (flip(side, offset, a, size, viewport), align, offset, a)
        }
        Positioning::At { x, y } => (
            Side::Right,
            SideAlign::Start,
            0.0,
            Bounds {
                x,
                y,
                width: 0.0,
                height: 0.0,
            },
        ),
        Positioning::Fill | Positioning::Over { .. } => return None,
    };
    let across_x = (viewport.width - 2.0 * EDGE_MARGIN).max(0.0);
    let across_y = (viewport.height - 2.0 * EDGE_MARGIN).max(0.0);
    let here = room(side, offset, anchor, viewport);
    let available = match side {
        Side::Top | Side::Bottom => (across_x, here),
        Side::Left | Side::Right => (here, across_y),
    };
    let mut placed = Placement {
        side,
        align,
        available,
        anchor,
        origin: (0.0, 0.0),
    };
    placed.origin = placed.facing(size.0, size.1);
    Some(placed)
}

impl Placement {
    /// The point of a box of `width` × `height` in this layer that faces
    /// the anchor, from its top-left corner: [`Placement::origin`] for the
    /// layer itself, and the anchor origin ([`crate::style::Origin::Anchor`])
    /// of any node in it.
    #[must_use]
    pub fn facing(&self, width: f32, height: f32) -> (f32, f32) {
        let along = match self.align {
            SideAlign::Start => 0.0,
            SideAlign::Center => 0.5,
            SideAlign::End => 1.0,
        };
        match self.side {
            Side::Bottom => (width * along, 0.0),
            Side::Top => (width * along, height),
            Side::Right => (0.0, height * along),
            Side::Left => (width, height * along),
        }
    }
}

/// Where a [`Positioning::Over`] layer goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Over {
    pub origin: (f32, f32),
    /// The layer reaches at least to the anchor's right edge.
    pub min_width: f32,
    /// The height it is cut to, when the window cannot hold all of it.
    pub height: Option<f32>,
    /// How far its content scrolls so the part stays over the anchor when
    /// the room above runs out.
    pub scroll: f32,
}

/// Place a layer of `size` over `anchor` so `part` (its box from the
/// layer's top-left corner) lies on `anchor_part`, as reka's
/// `SelectItemAlignedPosition` does.
pub(crate) fn over(anchor: Bounds, anchor_part: Bounds, part: Bounds, size: (f32, f32), viewport: Bounds) -> Over {
    let left = anchor_part.x - part.x;
    let min_width = anchor.right() - left;
    let width = size.0.max(min_width);
    let x = left
        .min(viewport.right() - OVER_MARGIN - width)
        .max(viewport.x + OVER_MARGIN);
    let top = anchor.y + anchor.height / 2.0 - (part.y + part.height / 2.0);
    let y = top.max(viewport.y + OVER_MARGIN);
    let scroll = y - top;
    let height = (size.1 - scroll).min(viewport.bottom() - OVER_MARGIN - y);
    Over {
        origin: (x, y),
        min_width,
        height: (height < size.1).then_some(height.max(0.0)),
        scroll,
    }
}
