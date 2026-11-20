//! Where a layer goes ([`Placement`]): beside its anchor, on the side with
//! room and kept on screen (floating-ui's `flip`, `shift` and `size`, which
//! reka's popper content uses), over its anchor with one of its parts on the
//! anchor's (reka's item-aligned select), at a point, or over the window.

use crate::element::{Placement, Side, SideAlign};
use crate::geometry::Bounds;

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
    placement: Placement,
    anchor: Option<Bounds>,
    size: (f32, f32),
    viewport: Bounds,
) -> Option<f32> {
    let Placement::Anchored {
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

/// Where a layer of `size` placed by `placement` goes (not [`Placement::Over`],
/// which [`over`] places).
pub(crate) fn position(placement: Placement, anchor: Option<Bounds>, size: (f32, f32), viewport: Bounds) -> (f32, f32) {
    let (w, h) = size;
    let clamp = |v: f32, len: f32, max: f32| v.min(max - len - EDGE_MARGIN).max(EDGE_MARGIN);
    match placement {
        Placement::Fill | Placement::Over { .. } => (0.0, 0.0),
        Placement::At { x, y } => (clamp(x, w, viewport.width), clamp(y, h, viewport.height)),
        Placement::Anchored {
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

/// The side a layer of `size` placed by `placement` opened on and how it
/// lines up there (reka's `data-side` and `data-align`): an anchored layer's
/// after flipping, and a layer at a point opens to its right, as reka's
/// context menu does; `None` for the rest.
pub(crate) fn placed(
    placement: Placement,
    anchor: Option<Bounds>,
    size: (f32, f32),
    viewport: Bounds,
) -> Option<(Side, SideAlign)> {
    match placement {
        Placement::Anchored {
            side, align, offset, ..
        } => Some((flip(side, offset, anchor?, size, viewport), align)),
        Placement::At { .. } => Some((Side::Right, SideAlign::Start)),
        Placement::Fill | Placement::Over { .. } => None,
    }
}

/// Where a [`Placement::Over`] layer goes.
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
