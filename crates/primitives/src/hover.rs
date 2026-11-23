//! What a hover card and a tooltip share: the pointer's grace area between a
//! trigger and its floating content (reka's `useGraceArea`), and a delayed
//! change on the frame clock.
//!
//! **The grace area.** When the pointer leaves the trigger toward the
//! content (or the content toward the trigger), it may cross the gap
//! between them without closing anything: the area is the convex hull of
//! the point where it left, padded by 1 px on each side across its exit
//! side, and the four corners of the box it heads for. While an area is
//! held, every pointer move is checked: reaching the trigger or the content
//! drops the area; leaving the hull drops it and tells the owner, which
//! closes (a tooltip at once, a hover card after its close delay). The
//! pointer is "in transit" from the moment it leaves until 300 ms later
//! (reka's `refAutoReset(false, 300)`: "Reset the inTransit state if
//! idle/scrolled"); the area itself stays until the pointer settles one way
//! or the other.
//!
//! reka listens to `pointermove` on the document while an area is held;
//! here the area's watcher reads [`gui::interact::pointer_position`], which
//! it tracks only while an area is held, so a pointer move costs nothing
//! otherwise. As reka's (`[data-grace-area-trigger]`), the pointer reaching
//! another hover trigger (a tooltip's or a hover card's, [`Grace::trigger`])
//! drops the area and tells the owner, even inside the hull: a hover card
//! whose area covers another card's trigger starts its close as that one
//! opens.

use std::cell::Cell;
use std::time::Duration;

use gui::clock::{self, TimerId};
use gui::{Bounds, Placement, Scope, Side, Signal};

/// How long the pointer counts as crossing to the content after it leaves
/// (reka's `useGraceArea`).
pub const TRANSIT: Duration = Duration::from_millis(300);

/// A point in window pixels.
pub type Point = (f32, f32);

/// The side of `rect` nearest `p` (reka's `getExitSideFromRect`: left,
/// then right, top, bottom on a tie).
#[must_use]
pub fn exit_side(p: Point, rect: Bounds) -> Side {
    let top = (rect.y - p.1).abs();
    let bottom = (rect.y + rect.height - p.1).abs();
    let right = (rect.x + rect.width - p.0).abs();
    let left = (rect.x - p.0).abs();
    let least = top.min(bottom).min(right).min(left);
    #[expect(
        clippy::float_cmp,
        reason = "reka's `switch (Math.min(…))`: the least is one of the four exactly"
    )]
    if least == left {
        Side::Left
    } else if least == right {
        Side::Right
    } else if least == top {
        Side::Top
    } else {
        Side::Bottom
    }
}

/// The exit point moved `padding` back into the box it left, spread
/// `padding` along its side (reka's `getPaddedExitPoints`).
#[must_use]
pub fn padded_exit(p: Point, side: Side, padding: f32) -> [Point; 2] {
    let (x, y) = p;
    match side {
        Side::Top => [(x - padding, y + padding), (x + padding, y + padding)],
        Side::Bottom => [(x - padding, y - padding), (x + padding, y - padding)],
        Side::Left => [(x + padding, y - padding), (x + padding, y + padding)],
        Side::Right => [(x - padding, y - padding), (x - padding, y + padding)],
    }
}

/// The corners of `r`.
fn corners(r: Bounds) -> [Point; 4] {
    [
        (r.x, r.y),
        (r.x + r.width, r.y),
        (r.x + r.width, r.y + r.height),
        (r.x, r.y + r.height),
    ]
}

/// The convex hull of `points` (Andrew's monotone chain, as reka's
/// `getHull`), counter-clockwise in screen coordinates.
#[must_use]
pub fn hull(points: &[Point]) -> Vec<Point> {
    let mut p: Vec<Point> = points.to_vec();
    p.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    if p.len() <= 1 {
        return p;
    }
    let turns = |o: Point, a: Point, b: Point| (a.0 - o.0) * (b.1 - o.1) >= (a.1 - o.1) * (b.0 - o.0);
    let chain = |iter: &mut dyn Iterator<Item = Point>| {
        let mut out: Vec<Point> = Vec::new();
        for q in iter {
            while let [.., r, s] = out[..] {
                if turns(r, s, q) {
                    out.pop();
                } else {
                    break;
                }
            }
            out.push(q);
        }
        out.pop();
        out
    };
    let upper = chain(&mut p.iter().copied());
    let lower = chain(&mut p.iter().rev().copied());
    if upper.len() == 1 && lower.len() == 1 && upper == lower {
        return upper;
    }
    upper.into_iter().chain(lower).collect()
}

/// Whether `p` is inside `polygon` (even-odd, reka's `isPointInPolygon`).
#[must_use]
pub fn inside(p: Point, polygon: &[Point]) -> bool {
    let (x, y) = p;
    let mut inside = false;
    let mut j = polygon.len().wrapping_sub(1);
    for (i, &(xi, yi)) in polygon.iter().enumerate() {
        let Some(&(xj, yj)) = polygon.get(j) else {
            break;
        };
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Whether `p` is in `r` as hit testing finds it: the left and top edges
/// in, the right and bottom out (a pointer on the right edge has left the
/// node, and its leave carries that point).
fn within(p: Point, r: Bounds) -> bool {
    p.0 >= r.x && p.0 < r.x + r.width && p.1 >= r.y && p.1 < r.y + r.height
}

/// The area the pointer may cross, leaving `from` at `exit` for `to`.
#[must_use]
pub fn area(exit: Point, from: Bounds, to: Bounds) -> Vec<Point> {
    let mut points = Vec::with_capacity(6);
    points.extend(padded_exit(exit, exit_side(exit, from), 1.0));
    points.extend(corners(to));
    hull(&points)
}

/// Where the trigger and the content are, as the content's layer was last
/// placed: the anchor is the trigger's side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Boxes {
    pub trigger: Bounds,
    pub content: Bounds,
}

impl Boxes {
    #[must_use]
    pub fn of(p: &Placement) -> Self {
        Self {
            trigger: p.anchor,
            content: p.bounds,
        }
    }
}

gui::engine_local! {
    /// The grace area whose trigger the pointer is on, by its number.
    [Fork] static ON_TRIGGER: Cell<Option<u64>> = Cell::new(None);
    /// The next grace area's number.
    [Fork] static NEXT_GRACE: Cell<u64> = Cell::new(0);
}

/// A grace area as one floating part holds it: made once in its root's
/// setup (`Copy`).
#[derive(Clone, Copy, PartialEq)]
pub struct Grace {
    /// Its number, which its trigger's hover is told by.
    id: u64,
    held: Signal<Option<(Vec<Point>, Boxes)>>,
    in_transit: Signal<bool>,
    transit: Delay,
    on_exit: Signal<gui::Callback<()>>,
}

impl std::fmt::Debug for Grace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grace").finish_non_exhaustive()
    }
}

impl Grace {
    /// A grace area telling `on_exit` when the pointer leaves it; called in
    /// setup, whose scope owns its watcher.
    #[must_use]
    #[track_caller]
    pub fn new(on_exit: impl Into<gui::Callback<()>>) -> Self {
        let on_exit = on_exit.into();
        let id = NEXT_GRACE.with(|n| {
            let id = n.get();
            n.set(id.wrapping_add(1));
            id
        });
        let g = Self {
            id,
            held: Signal::labeled("grace area", None),
            in_transit: Signal::labeled("grace in transit", false),
            transit: Delay::new(),
            on_exit: Signal::new(on_exit),
        };
        // The pointer is read only while an area is held.
        gui::watch(
            move || {
                if g.held.with(Option::is_some) {
                    gui::interact::pointer_position()
                } else {
                    None
                }
            },
            move |at, _| {
                if let Some(at) = *at {
                    g.moved(at);
                }
            },
        );
        gui::on_cleanup(move || {
            if g.in_transit.is_alive() {
                g.in_transit.set(false);
            }
            ON_TRIGGER.with(|t| {
                if t.get() == Some(id) {
                    t.set(None);
                }
            });
        });
        g
    }

    /// Its trigger's hover changed (the pointer came onto it, `inside`, or
    /// left it): called first by the trigger's `on_hover`, so any other
    /// area held is dropped as the pointer reaches this trigger.
    pub fn trigger(self, inside: bool) {
        ON_TRIGGER.with(|t| {
            if inside {
                t.set(Some(self.id));
            } else if t.get() == Some(self.id) {
                t.set(None);
            }
        });
    }

    /// The pointer left the trigger (`from_trigger`) or the content where
    /// it is now, heading for the other; boxes as last placed.
    pub fn leave(self, from_trigger: bool, boxes: Boxes) {
        let Some(exit) = gui::untracked(gui::interact::pointer_position) else {
            return;
        };
        let (from, to) = if from_trigger {
            (boxes.trigger, boxes.content)
        } else {
            (boxes.content, boxes.trigger)
        };
        self.in_transit.set(true);
        let in_transit = self.in_transit;
        self.transit.after(TRANSIT, move || {
            if in_transit.is_alive() {
                in_transit.set(false);
            }
        });
        // Last: the watcher may judge the pointer at once, and clear what
        // was just set (a pointer already outside the area).
        self.held.set(Some((area(exit, from, to), boxes)));
    }

    /// Whether the pointer is crossing to the content now (tracked).
    #[must_use]
    pub fn in_transit(self) -> bool {
        self.in_transit.get()
    }

    /// Drop the area: the pointer reached where it was heading.
    pub fn clear(self) {
        if self.held.with_untracked(Option::is_some) {
            self.held.set(None);
        }
        if self.in_transit.get_untracked() {
            self.in_transit.set(false);
        }
    }

    fn moved(self, at: Point) {
        let Some(verdict) = self.held.with_untracked(|h| {
            h.as_ref().map(|(polygon, boxes)| {
                let on_other = ON_TRIGGER.with(Cell::get).is_some_and(|t| t != self.id);
                if within(at, boxes.trigger) || within(at, boxes.content) {
                    Some(false)
                } else if on_other || !inside(at, polygon) {
                    Some(true)
                } else {
                    None
                }
            })
        }) else {
            return;
        };
        if let Some(exited) = verdict {
            self.clear();
            if exited {
                let f = self.on_exit.get_untracked();
                f.call(());
            }
        }
    }
}

/// One change waiting for its delay on the frame clock: a newer one
/// replaces it. Made once in setup (`Copy`); its timer belongs to that
/// scope, so a pending change goes with the instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delay {
    /// The timer waiting, and the scope of its own it belongs to (under
    /// [`Self::owner`]): disposed as another replaces it, which takes the
    /// timer's cleanup with it, so a long-lived delay holds one at most.
    timer: Signal<Option<(TimerId, Scope)>>,
    /// What its timers belong to: the scope it was made in, not whatever
    /// watcher asks (whose next run would cancel it).
    owner: Scope,
}

impl Delay {
    #[must_use]
    #[track_caller]
    pub fn new() -> Self {
        Self {
            timer: Signal::labeled("delay", None),
            owner: Scope::new(),
        }
    }

    /// Whether a change is waiting.
    #[must_use]
    pub fn pending(self) -> bool {
        self.timer.is_alive()
            && self
                .timer
                .with_untracked(|t| t.is_some_and(|(t, _)| clock::is_pending(t)))
    }

    /// Drop the waiting change.
    pub fn cancel(self) {
        if !self.timer.is_alive() {
            return;
        }
        if let Some((t, scope)) = self.timer.get_untracked() {
            clock::cancel(t);
            self.timer.set(None);
            // Its cleanup (cancelling it) goes with its scope.
            if scope.is_alive() {
                scope.dispose();
            }
        }
    }

    /// Run `f` once `delay` has passed (at once for none), replacing what
    /// was waiting.
    pub fn after(self, delay: Duration, f: impl Fn() + 'static) {
        self.cancel();
        if delay.is_zero() {
            f();
            return;
        }
        if !self.owner.is_alive() {
            return;
        }
        let scope = self.owner.run(Scope::new);
        // It runs as the delay's owner's, so what it starts (the next
        // timer, replacing this one) outlives this timer's scope.
        let owner = self.owner;
        let t = scope.run(|| {
            clock::after(delay, move || {
                if owner.is_alive() {
                    owner.run(&f);
                }
            })
        });
        self.timer.set(Some((t, scope)));
    }
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use gui::{Bounds, Side};

    use super::{area, exit_side, hull, inside};

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds {
        Bounds { x, y, width, height }
    }

    /// A delay replaced again and again (a rotating carousel's) holds one
    /// timer's cleanup at most, not one per timer it ever waited on.
    #[test]
    fn a_replaced_delay_leaves_no_cleanup_behind() {
        let scope = gui::Scope::new();
        let delay = scope.run(super::Delay::new);
        let before = gui::reactive::cleanups();
        for _ in 0..100 {
            delay.after(std::time::Duration::from_secs(1), || {});
        }
        let held = gui::reactive::cleanups().saturating_sub(before);
        assert!(held <= 1, "{held} cleanups held for one waiting timer");
        assert!(delay.pending());
        delay.cancel();
        assert!(!delay.pending());
        scope.dispose();
    }

    /// A cold frame sees the live timers waiting: a delay waiting live
    /// renders as waiting in both frames (the check compares them).
    #[test]
    fn a_waiting_delay_renders_the_same_cold() {
        use gui::{IntoElement, div, text};
        let delay = gui::detached(super::Delay::new);
        // Read untracked, so asked again as this moves.
        let asked = gui::Signal::new(0_u32);
        let mut s = gui::Scenario::measuring(
            move || {
                asked.get();
                div()
                    .child(text(if delay.pending() {
                        "waiting a long while"
                    } else {
                        "idle"
                    }))
                    .into_element()
            },
            200,
            50,
            1.0,
        )
        .expect("a scenario");
        s.check_cold(true);
        s.frame("idle").expect("a frame");
        s.act("wait", || {
            delay.after(std::time::Duration::from_secs(60), || {});
            asked.set(1);
        });
        s.frame("waiting")
            .expect("the cold frame renders it as the live one does");
        assert!(gui::engine::in_cold_frame(|| delay.pending()), "pending, read cold");
        delay.cancel();
    }

    #[test]
    fn the_exit_side_is_the_nearest_edge() {
        let r = rect(0.0, 0.0, 100.0, 40.0);
        assert_eq!(exit_side((50.0, 41.0), r), Side::Bottom);
        assert_eq!(exit_side((50.0, -1.0), r), Side::Top);
        assert_eq!(exit_side((101.0, 20.0), r), Side::Right);
        assert_eq!(exit_side((-1.0, 20.0), r), Side::Left);
    }

    #[test]
    fn the_hull_drops_inner_points() {
        let h = hull(&[(0.0, 0.0), (10.0, 0.0), (5.0, 5.0), (10.0, 10.0), (0.0, 10.0)]);
        assert_eq!(h.len(), 4, "the middle point is inside: {h:?}");
        assert!(!h.contains(&(5.0, 5.0)));
    }

    #[test]
    fn the_area_covers_the_gap_toward_the_content_and_nothing_beside_it() {
        // A trigger at the top, the content 4 px below and wider.
        let trigger = rect(100.0, 0.0, 40.0, 20.0);
        let content = rect(60.0, 24.0, 120.0, 80.0);
        let a = area((120.0, 21.0), trigger, content);
        assert!(inside((120.0, 22.5), &a), "the gap under the exit point");
        assert!(inside((70.0, 26.0), &a), "toward the content's corner");
        assert!(!inside((20.0, 22.0), &a), "far to the left of the path");
        assert!(!inside((120.0, 120.0), &a), "past the content");
    }
}
