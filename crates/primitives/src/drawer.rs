//! A panel on a window edge, swiped back to that edge to close (reka's
//! `Drawer`, a port of Base UI's; shadcn's `drawer` draws on it, not on
//! vaul).
//!
//! - **Modal** has three levels ([`DrawerModal`]): `On` (the engine's
//!   modal layer, an overlay), `TrapFocus` (focus trapped, the page still
//!   takes the pointer; reka still renders the overlay, as its source does
//!   though its docs say otherwise), and `Off` (no trap; Tab still cycles in
//!   the content).
//! - **Every change asks first.** Each request to open or close carries a
//!   [`ChangeReason`] and goes to the caller ([`DrawerOptions::veto`]),
//!   which may refuse it: the drawer stays as it is, and a swipe in
//!   progress slides back. A caller keeping the value that does not write
//!   it back also gets the drawer sliding back (the model's rule).
//! - **Settled** ([`DrawerOptions::on_settled`]): told `true` once the
//!   content's entry animation has run, `false` once its exit has; not when
//!   the other way interrupted it, nor when the root unmounts. An endless
//!   animation is not waited for (the engine's [`gui::Stage`]).
//! - **Closing** gives focus to the trigger (reka's `DrawerContent`), or to
//!   the node [`FinalFocus`] names, unless something else took focus
//!   meanwhile, or the user interacted outside a drawer that is not modal.
//!   reka takes focus back for the trigger when the drawer unmounts even if
//!   another layer took it while it animated out; it does so only when the
//!   content is kept mounted. Here the engine's rule holds in both cases,
//!   as for a dialog (research §4 item 13, recorded in ADR-0025).
//! - **Escape** closes it, as do a press outside (at every level; at `On`,
//!   not a secondary press or a Ctrl+press), focus outside (except at `On`),
//!   its close parts and the trigger. While a swipe is in progress, a press
//!   or focus outside is ignored.
//! - **Swiping** ([`Swipe`], reka's `useSwipeDismiss`): the content follows
//!   the pointer toward its edge, and away from it by the square root of the
//!   distance (a rubber band). It starts after 1 px; let go, it closes past
//!   40 px or faster than 0.3 px/ms toward the edge, unless the pointer
//!   turned back more than 10 px after going more than 20 px. The speed is
//!   the last sample's (at least 16 ms apart), and nothing if that sample is
//!   older than 80 ms. A swipe that closes leaves the content where the
//!   pointer let go, for its exit to run from there. A mouse or a pen
//!   pressed on a control inside (a button, a link, a field, a label) does
//!   not swipe; a finger does. A [`Drawer::no_swipe`] region never swipes.
//!   A list inside that can still scroll the way a finger moves scrolls
//!   first (the engine's rule for a pan). The window losing focus ends a
//!   swipe where it is. A drawer does not swipe while one nested in it does.
//! - **Snap points** ([`DrawerOptions::snap_points`], reka's
//!   `useDrawerSnapPoints`): heights it rests at; the active one is a
//!   two-way model (none: fully open). With snap points the content is
//!   dragged both ways, and let go it goes to the nearest point (the drag's
//!   end carried 300 ms at its speed), or one point at most when
//!   [`DrawerOptions::sequential_snap`]; it closes (reason `Swipe`) only
//!   when nearer closed than every point. The look moves the content by
//!   [`Drawer::snap_offset`].
//! - **Opening by a swipe** ([`Drawer::swipe_area`]): a region on the page
//!   swiped the other way opens it, at the same 40 px or 0.3 px/ms; any
//!   pointer pressed on a control there does not.
//! - **Nested drawers**: a drawer opened inside another's content tells it
//!   how many are open in it ([`Drawer::nested_open`]), the height of the
//!   frontmost ([`Drawer::frontmost_height`]; its own once they close,
//!   never 0), whether one is swiping and how far. A nested drawer has no
//!   overlay of its own unless [`DrawerOptions::overlay_nested`]. reka
//!   reports a child's swipe to its grandparent and not its parent; here
//!   each drawer tells its own parent, which tells its own (research §7,
//!   recorded in ADR-0025).
//! - **The page behind** ([`DrawerProvider`]): a provider over the page
//!   knows whether any drawer is open, the frontmost one's swipe progress
//!   and height, for the look that shrinks the page behind (reka's
//!   `DrawerProvider` and `DrawerIndent`).
//!
//! Keyboard: reka has no key for a snap point (only Escape); nor does this.
//! The virtual keyboard's inset (reka's `DrawerVirtualKeyboardProvider`)
//! has no counterpart: `gui` runs on desktops, with no on-screen keyboard to
//! cover a field.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gui::element::Node;
use gui::{
    AutoFocus, Bounds, Callback, Dismiss, DismissPolicy, DismissReason, Div, DragEvent, DragPhase, FocusHandle,
    FocusScope, HasPopup, Layer, PointerKind, Role, Signal, Stage, UseModel,
};

use crate::dialog::{Missing, Names, PartNames, is_context_press};
use crate::presence::Presence;
use crate::relation::{Linked, Points};

/// How far a swipe must carry the drawer to close it (reka's
/// `DEFAULT_SWIPE_THRESHOLD`).
pub const SWIPE_THRESHOLD: f32 = 40.0;
/// How fast toward the edge a release closes it, px/ms.
pub const SWIPE_VELOCITY: f32 = 0.3;
/// How far back from its furthest a swipe must turn to cancel, once it went
/// past half the threshold (reka's `REVERSE_CANCEL_THRESHOLD`).
pub const REVERSE_CANCEL: f32 = 10.0;
/// How far the pointer moves before a swipe starts (`MIN_DRAG_THRESHOLD`).
pub const START: f32 = 1.0;
/// The least time between two speed samples (`MIN_RELEASE_VELOCITY_DURATION_MS`).
pub const MIN_SAMPLE: Duration = Duration::from_millis(16);
/// A last sample older than this at release means no speed
/// (`MAX_RELEASE_VELOCITY_AGE_MS`).
pub const STALE_SAMPLE: Duration = Duration::from_millis(80);

/// The edge a drawer sits on, which is also the way it is swiped to close.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SwipeDirection {
    Up,
    #[default]
    Down,
    Left,
    Right,
}

impl SwipeDirection {
    /// How far `(dx, dy)` goes toward the edge (reka's `getDisplacement`).
    #[must_use]
    pub fn displacement(self, dx: f32, dy: f32) -> f32 {
        match self {
            Self::Up => -dy,
            Self::Down => dy,
            Self::Left => -dx,
            Self::Right => dx,
        }
    }

    /// The way back.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Down => Self::Up,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    /// Up or down.
    #[must_use]
    pub const fn vertical(self) -> bool {
        matches!(self, Self::Up | Self::Down)
    }

    /// `along` the axis, as a screen offset `(x, y)` signed toward this edge.
    #[must_use]
    pub fn offset(self, along: f32) -> (f32, f32) {
        match self {
            Self::Up => (0.0, -along),
            Self::Down => (0.0, along),
            Self::Left => (-along, 0.0),
            Self::Right => (along, 0.0),
        }
    }
}

/// A drawer's modality (reka's `modal`: `true`, `'trap-focus'`, `false`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum DrawerModal {
    /// Focus trapped, the page inert and hidden, an overlay.
    #[default]
    On,
    /// Focus trapped; the page still takes the pointer.
    TrapFocus,
    /// Nothing trapped.
    Off,
}

/// Why a drawer is asked to open or close (reka's `DrawerOpenChangeReason`;
/// its `click` and `cancel` are declared but never sent, and are left out).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChangeReason {
    Swipe,
    EscapeKey,
    OutsidePress,
    /// Focus went outside a drawer that is not `On`.
    FocusOutside,
    TriggerPress,
    ClosePress,
}

/// Where focus goes as a drawer closes (reka's `finalFocus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FinalFocus {
    /// Its trigger (reka's default).
    #[default]
    Trigger,
    /// Nowhere: focus is left where it is.
    Nowhere,
    /// This node.
    Node(FocusHandle),
}

/// What a drawer's caller chooses (reka's `DrawerRoot` and `DrawerContent`
/// props).
#[derive(Clone, Debug, PartialEq)]
pub struct DrawerOptions {
    pub modal: DrawerModal,
    /// The edge it sits on, the way it is swiped to close (reka's
    /// `swipeDirection`).
    pub direction: SwipeDirection,
    /// Kept mounted while closed (reka's `forceMount`).
    pub keep_mounted: bool,
    /// Where focus goes as it opens (reka's `initialFocus`: the first Tab
    /// stop by default, nowhere, or a node).
    pub initial_focus: AutoFocus,
    /// Where focus goes as it closes.
    pub final_focus: FinalFocus,
    /// The heights it rests at (reka's `snapPoints`); none: fully open.
    pub snap_points: Vec<SnapPoint>,
    /// Let go, it moves one snap point at most (reka's
    /// `snapToSequentialPoints`).
    pub sequential_snap: bool,
    /// Nested in another drawer, it still has an overlay (reka's
    /// `DrawerOverlay` `forceRender`).
    pub overlay_nested: bool,
    /// Asked before every change: `(open, reason)`, refused with
    /// [`Dismiss::Veto`].
    pub veto: Option<Callback<(bool, ChangeReason), Dismiss>>,
    /// Told when it has finished opening (`true`) or closing (reka's
    /// `update:openComplete`).
    pub on_settled: Option<Callback<bool>>,
}

impl Default for DrawerOptions {
    fn default() -> Self {
        Self {
            modal: DrawerModal::On,
            direction: SwipeDirection::Down,
            keep_mounted: false,
            initial_focus: AutoFocus::First,
            final_focus: FinalFocus::Trigger,
            snap_points: Vec::new(),
            sequential_snap: false,
            overlay_nested: false,
            veto: None,
            on_settled: None,
        }
    }
}

/// What a swipe did on a move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwipeMove {
    /// The content's offset from where it rests, damped against the edge.
    pub offset: (f32, f32),
    /// How far toward closed, 0 to 1: the displacement over the size plus
    /// the threshold.
    pub progress: f32,
}

/// What letting go decides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Release {
    /// No swipe started: a press, nothing to do.
    None,
    /// Close, from where it is.
    Dismiss,
    /// Slide back.
    Back,
}

/// A swipe toward a drawer's edge (reka's `useSwipeDismiss`), fed pointer
/// positions and times. Not reactive: the drawer holds one per drag.
#[derive(Clone, Debug, PartialEq)]
pub struct Swipe {
    direction: SwipeDirection,
    /// The content's size along the swipe.
    size: f32,
    /// How far it may follow the pointer away from its edge before the
    /// rubber band: a snap point's offset (0 without snap points).
    room: f32,
    start: (f32, f32),
    started: bool,
    vertical: bool,
    max: f32,
    cancelled: bool,
    offset: (f32, f32),
    sample: Option<((f32, f32), Duration)>,
    velocity: (f32, f32),
}

impl Swipe {
    /// A press at `start` on content `size` long along `direction`.
    #[must_use]
    pub fn new(direction: SwipeDirection, size: f32, start: (f32, f32)) -> Self {
        Self::with_room(direction, size, 0.0, start)
    }

    /// A press as [`Self::new`], on a drawer that may follow the pointer
    /// `room` px away from its edge before the rubber band (its snap
    /// point's offset: dragged open, it opens further).
    #[must_use]
    pub fn with_room(direction: SwipeDirection, size: f32, room: f32, start: (f32, f32)) -> Self {
        Self {
            direction,
            size,
            room: room.max(0.0),
            start,
            started: false,
            vertical: direction.vertical(),
            max: 0.0,
            cancelled: false,
            offset: (0.0, 0.0),
            sample: None,
            velocity: (0.0, 0.0),
        }
    }

    /// Whether the swipe has started (the pointer went 1 px).
    #[must_use]
    pub fn started(&self) -> bool {
        self.started
    }

    /// The content's size along the swipe.
    #[must_use]
    pub fn size(&self) -> f32 {
        self.size
    }

    /// The pointer at `at`, at `time` on the frame clock.
    pub fn moved(&mut self, at: (f32, f32), time: Duration) -> Option<SwipeMove> {
        let raw = (at.0 - self.start.0, at.1 - self.start.1);
        let (dx, dy) = if self.vertical { (0.0, raw.1) } else { (raw.0, 0.0) };
        if !self.started {
            if dx.abs().max(dy.abs()) < START {
                return None;
            }
            self.started = true;
        }
        let displacement = self.direction.displacement(dx, dy);
        self.max = self.max.max(displacement);
        if self.max > SWIPE_THRESHOLD / 2.0 && self.max - displacement > REVERSE_CANCEL {
            self.cancelled = true;
        }
        self.offset = self.damped(dx, dy);
        self.record(time);
        let progress = (self.direction.displacement(dx, dy) / (self.size + SWIPE_THRESHOLD)).clamp(0.0, 1.0);
        Some(SwipeMove {
            offset: self.offset,
            progress,
        })
    }

    /// Toward the edge passes, and away from it up to the room; past that
    /// it is the square root. Across the axis it is the square root too.
    fn damped(&self, dx: f32, dy: f32) -> (f32, f32) {
        let root = |v: f32| v.signum() * v.abs().sqrt();
        let along = self.direction.displacement(dx, dy);
        let along = if along >= -self.room {
            along
        } else {
            -self.room - (-along - self.room).sqrt()
        };
        let (ox, oy) = self.direction.offset(along);
        if self.vertical { (root(dx), oy) } else { (ox, root(dy)) }
    }

    fn record(&mut self, time: Duration) {
        if let Some((last, at)) = self.sample
            && time > at
        {
            let dt = time.saturating_sub(at).max(MIN_SAMPLE).as_secs_f32() * 1000.0;
            self.velocity = ((self.offset.0 - last.0) / dt, (self.offset.1 - last.1) / dt);
        }
        self.sample = Some((self.offset, time));
    }

    /// The speed at a release at `time`, px/ms: none if the last sample is
    /// older than 80 ms.
    #[must_use]
    pub fn velocity(&self, time: Duration) -> (f32, f32) {
        match self.sample {
            Some((_, at)) if time.saturating_sub(at) <= STALE_SAMPLE => self.velocity,
            _ => (0.0, 0.0),
        }
    }

    /// The offset as last moved.
    #[must_use]
    pub fn offset(&self) -> (f32, f32) {
        self.offset
    }

    /// How far the offset goes toward the edge (negative: away from it).
    #[must_use]
    pub fn displacement(&self) -> f32 {
        self.direction.displacement(self.offset.0, self.offset.1)
    }

    /// Let go at `time`.
    #[must_use]
    pub fn release(&self, time: Duration) -> Release {
        if !self.started {
            return Release::None;
        }
        let displacement = self.displacement();
        let (vx, vy) = self.velocity(time);
        let toward = self.direction.displacement(vx, vy);
        if !self.cancelled && (displacement >= SWIPE_THRESHOLD || toward > SWIPE_VELOCITY) {
            Release::Dismiss
        } else {
            Release::Back
        }
    }
}

/// How fast a released drawer finishes, 0.1 to 1 of its normal duration
/// (reka's `computeSwipeReleaseScalar`): the time left at the release
/// speed (0.2 to 4 px/ms), held to 80–360 ms and scaled. `None` when the
/// release does not move it toward its edge.
#[must_use]
pub fn release_scalar(direction: SwipeDirection, size: f32, along: f32, snap_offset: f32, speed: f32) -> Option<f32> {
    if !size.is_finite() || size <= 0.0 {
        return None;
    }
    let toward_positive = matches!(direction, SwipeDirection::Down | SwipeDirection::Right);
    let base = if toward_positive { snap_offset } else { -snap_offset };
    let translation = base + along;
    let travelled = if toward_positive { translation } else { -translation };
    let remaining = (size - travelled).max(0.0);
    if remaining <= 0.0 {
        return None;
    }
    let directional = if toward_positive { speed } else { -speed };
    if !directional.is_finite() || directional <= 0.2 {
        return None;
    }
    let ms = (remaining / directional.clamp(0.2, 4.0)).clamp(80.0, 360.0);
    let normalized = (ms - 80.0) / (360.0 - 80.0);
    Some(normalized.mul_add(0.9, 0.1).clamp(0.1, 1.0))
}

/// A height a drawer can rest at (reka's `DrawerSnapPoint`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnapPoint {
    /// 0 to 1: that share of the window's height; above 1: pixels.
    Number(f32),
    Px(f32),
    /// Times the root font size (16 px by default).
    Rem(f32),
}

/// A snap point resolved against a drawer and a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved {
    pub point: SnapPoint,
    /// How tall the drawer is there.
    pub height: f32,
    /// How far below fully open that is.
    pub offset: f32,
}

impl SnapPoint {
    /// Its height in pixels, rounded; `None` for a negative or non-finite one
    /// (reka's `parseSnapPoint`).
    #[must_use]
    pub fn height(self, viewport: f32, root_font: f32) -> Option<f32> {
        let h = match self {
            Self::Number(v) if (0.0..=1.0).contains(&v) => v * viewport,
            Self::Number(v) | Self::Px(v) => v,
            Self::Rem(v) => v * root_font,
        };
        (h.is_finite() && h >= 0.0).then(|| h.round())
    }
}

/// `points` resolved for content `content` tall in a window `viewport`
/// tall: each clamped to both, one of two within 1 px dropped, sorted by
/// height (reka's `resolvedSnapPoints`).
#[must_use]
pub fn resolve_snap_points(points: &[SnapPoint], content: f32, viewport: f32, root_font: f32) -> Vec<Resolved> {
    let mut out: Vec<Resolved> = Vec::with_capacity(points.len());
    for &point in points {
        let Some(height) = point.height(viewport, root_font) else {
            continue;
        };
        if out.iter().any(|r| (r.height - height).abs() <= 1.0) {
            continue;
        }
        let height = height.min(content.min(viewport));
        out.push(Resolved {
            point,
            height,
            offset: (content - height).max(0.0),
        });
    }
    out.sort_by(|a, b| a.height.total_cmp(&b.height));
    out
}

/// Where a released drawer with snap points goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnapTarget {
    /// Closed: nearer closed than every point.
    Close,
    /// The point at this index of the resolved list.
    Point(usize),
}

/// Where a drawer resting at `active` (an index of `points`, resolved),
/// dragged `delta` toward closed (negative: toward open) and let go at
/// `speed` px/ms toward closed, goes (reka's `snapToNearest`): the drag's
/// end, carried 300 ms further at a speed of at least 0.5 px/ms (at most 4),
/// to the nearest point, or closed when nearer closed than every point. In
/// `sequential` mode it moves one point at most: on a fast enough release
/// the same way as the drag, or when the target passed the next point.
#[must_use]
pub fn snap_release(
    points: &[Resolved],
    active: Option<usize>,
    content: f32,
    delta: f32,
    speed: f32,
    sequential: bool,
) -> Option<SnapTarget> {
    let first = points.first()?;
    let current = active.and_then(|i| points.get(i)).map_or(0.0, |p| p.offset);
    let dragged = (current + delta).clamp(0.0, content);
    let target = if speed.abs() >= 0.5 {
        speed.clamp(-4.0, 4.0).mul_add(300.0, dragged)
    } else {
        dragged
    };
    let mut closest = (0_usize, (target - first.offset).abs());
    for (i, p) in points.iter().enumerate() {
        let d = (target - p.offset).abs();
        if d < closest.1 {
            closest = (i, d);
        }
    }
    if (target - content).abs() < closest.1 {
        return Some(SnapTarget::Close);
    }
    if !sequential {
        return Some(SnapTarget::Point(closest.0));
    }
    let Some(at) = active.filter(|&i| i < points.len()) else {
        return Some(SnapTarget::Point(closest.0));
    };
    // Sorted by height, so by offset the other way round.
    let mut by_offset: Vec<usize> = (0..points.len()).collect();
    by_offset.sort_by(|&a, &b| {
        let (pa, pb) = (
            points.get(a).map_or(0.0, |p| p.offset),
            points.get(b).map_or(0.0, |p| p.offset),
        );
        pa.total_cmp(&pb)
    });
    let here = by_offset.iter().position(|&i| i == at).unwrap_or(0);
    let way = delta.signum();
    let next = if way > 0.0 {
        here.saturating_add(1).min(by_offset.len().saturating_sub(1))
    } else if way < 0.0 {
        here.saturating_sub(1)
    } else {
        here
    };
    let adjacent = by_offset.get(next).copied().unwrap_or(at);
    if speed.signum() == way && speed.abs() >= 0.5 {
        return Some(SnapTarget::Point(adjacent));
    }
    let offset = points.get(adjacent).map_or(0.0, |p| p.offset);
    let crossed = if way > 0.0 { target > offset } else { target < offset };
    Some(SnapTarget::Point(if crossed { adjacent } else { at }))
}

/// The root font size snap points in `rem` are measured by (reka's
/// default when the page sets none).
pub const ROOT_FONT: f32 = 16.0;

/// The drawers nested in a drawer (reka's `notifyParent*`, kept one level
/// each): each mounted one's own report, its entry made as it mounts and
/// taken out as it unmounts. Each writes only its own; the parent reads
/// them all, so one leaving never touches another's.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Nest {
    told: gui::Registry<Signal<Told>>,
}

/// What a nested drawer tells the drawer it is nested in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Told {
    /// When it opened, in the order drawers open (the latest is
    /// frontmost); `None` while closed.
    opened: Option<u64>,
    /// Its frontmost height (its own, or that of one nested in it).
    height: f32,
    /// How far its frontmost swipe is, 0 to 1.
    progress: f32,
    /// It or one nested in it (at any depth) is being swiped.
    swiping: bool,
}

impl Nest {
    #[track_caller]
    fn new() -> Self {
        Self {
            told: gui::Registry::new(),
        }
    }

    /// `f` over the reports of the nested drawers still mounted (tracked),
    /// read in place: nothing is copied out or allocated.
    fn reduce<R>(self, f: impl FnOnce(&mut dyn Iterator<Item = Told>) -> R) -> R {
        self.told
            .with(|it| f(&mut it.filter(|s| s.is_alive()).map(|s| s.get())))
    }

    /// The report of the one that opened last of those open (tracked).
    fn frontmost(self) -> Option<Told> {
        self.reduce(|it| it.filter(|t| t.opened.is_some()).max_by_key(|t| t.opened))
    }
}

/// What the page behind the drawers knows of them (reka's
/// `DrawerProvider`, read by its `DrawerIndent`): made once in the setup of
/// a component over the page and given with `gui::provide`; every drawer
/// under it reports to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawerProvider {
    open: Signal<Vec<u64>>,
    progress: Signal<f32>,
    height: Signal<f32>,
}

impl Default for DrawerProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl DrawerProvider {
    #[must_use]
    #[track_caller]
    pub fn new() -> Self {
        Self {
            open: Signal::labeled("drawers open", Vec::new()),
            progress: Signal::labeled("drawer provider progress", 0.0),
            height: Signal::labeled("drawer provider height", 0.0),
        }
    }

    /// Whether any drawer under it is open (tracked): the page behind
    /// shrinks back.
    #[must_use]
    pub fn active(self) -> bool {
        self.open.with(|o| !o.is_empty())
    }

    /// The frontmost drawer's swipe progress, 0 to 1 (tracked).
    #[must_use]
    pub fn progress(self) -> f32 {
        self.progress.get()
    }

    /// The last measured drawer's height (tracked).
    #[must_use]
    pub fn frontmost_height(self) -> f32 {
        self.height.get()
    }

    fn set_open(self, id: u64, open: bool) {
        if !self.open.is_alive() {
            return;
        }
        let has = self.open.with_untracked(|o| o.contains(&id));
        if open && !has {
            self.open.update(|o| o.push(id));
        } else if !open && has {
            self.open.update(|o| o.retain(|i| *i != id));
        }
    }

    fn set_progress(self, p: f32) {
        if self.progress.is_alive() && self.progress.get_untracked().to_bits() != p.to_bits() {
            self.progress.set(p);
        }
    }

    fn set_height(self, h: f32) {
        if self.height.is_alive() && self.height.get_untracked().to_bits() != h.to_bits() {
            self.height.set(h);
        }
    }
}

/// The drawers made, numbered.
static NEXT: gui::engine::IdKind = gui::engine::IdKind::new("drawer");
/// The openings, numbered: nested drawers are frontmost in the order they
/// open.
static OPENED: gui::engine::IdKind = gui::engine::IdKind::new("drawer opening");

/// The swipe in `slot`, taken out: the slot holds none after.
fn take_swipe(slot: Signal<Option<Swipe>>) -> Option<Swipe> {
    let swipe = slot.get_untracked();
    if swipe.is_some() {
        slot.set(None);
    }
    swipe
}

/// The pointer at `at`, at `now`, on the swipe in `slot` (if any): the
/// moved swipe replaces the one held.
fn move_swipe(slot: Signal<Option<Swipe>>, at: (f32, f32), now: Duration) -> Option<SwipeMove> {
    let mut swipe = slot.get_untracked()?;
    let moved = swipe.moved(at, now);
    slot.set(Some(swipe));
    moved
}

/// The number of an opening, later than every one before it.
fn next_opening() -> u64 {
    // Increasing on a thread, live and fork alike (a fork's come after
    // every live one), and never given twice (ADR-0028, rule 3).
    gui::engine::unique_id(&OPENED)
}

/// Write `v` to `s` only when it changed (a signal read per frame).
fn put(s: Signal<f32>, v: f32) {
    if s.is_alive() && s.get_untracked().to_bits() != v.to_bits() {
        s.set(v);
    }
}

/// A drawer's state: made once in its root's setup (`Copy`).
pub struct Drawer<P: 'static> {
    open: UseModel<P, bool>,
    /// The caller's active snap point, when it keeps it.
    snap: Option<UseModel<P, Option<SnapPoint>>>,
    /// The active snap point the drawer keeps itself.
    snap_local: Signal<Option<SnapPoint>>,
    options: Signal<Rc<dyn Fn() -> DrawerOptions>>,
    linked: Linked,
    names: Names,
    trigger: FocusHandle,
    /// The swipe in progress on its content: a plain value, replaced on
    /// every move. Nothing reads it tracked (`dragging`, `offset` and
    /// `progress` are what the view follows).
    swipe: Signal<Option<Swipe>>,
    /// The swipe of the area that opens it.
    opening: Signal<Option<Swipe>>,
    dragging: Signal<bool>,
    offset: Signal<(f32, f32)>,
    progress: Signal<f32>,
    /// It closed by a swipe: its exit runs from where it was let go.
    swiped: Signal<bool>,
    /// How fast its release finishes (0.1–1), when it was flung.
    release: Signal<Option<f32>>,
    /// The content's height, as last measured.
    height: Signal<f32>,
    /// The window's height, as the viewport part last measured it.
    viewport: Signal<f32>,
    nest: Nest,
    parent: Option<Nest>,
    provider: Option<DrawerProvider>,
    id: u64,
}

impl<P: 'static> Clone for Drawer<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Drawer<P> {}

impl<P: 'static> PartialEq for Drawer<P> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<P: 'static> std::fmt::Debug for Drawer<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Drawer").field("id", &self.id).finish_non_exhaustive()
    }
}

impl<P: 'static> Drawer<P> {
    /// A drawer open as `open` says, resting at the snap point `snap` says
    /// (the drawer keeps it itself when `None`), as `options` choose;
    /// called in its root's setup. A drawer whose root is inside another
    /// drawer's is nested in it; one under a [`DrawerProvider`] reports to it.
    #[must_use]
    #[track_caller]
    pub fn new(
        open: UseModel<P, bool>,
        snap: Option<UseModel<P, Option<SnapPoint>>>,
        options: impl Fn() -> DrawerOptions + 'static,
    ) -> Self {
        let parent = gui::use_context::<Nest>();
        let provider = gui::use_context::<DrawerProvider>();
        let nest = Nest::new();
        gui::provide(nest);
        let id = gui::engine::unique_id(&NEXT);
        let d = Self {
            open,
            snap,
            snap_local: Signal::labeled("drawer snap point", None),
            options: Signal::new(Rc::new(options)),
            linked: Linked::new(Points::Controls),
            names: Names::new(),
            trigger: FocusHandle::new(),
            swipe: Signal::labeled("drawer swipe", None),
            opening: Signal::labeled("drawer opening swipe", None),
            dragging: Signal::labeled("drawer dragging", false),
            offset: Signal::labeled("drawer offset", (0.0, 0.0)),
            progress: Signal::labeled("drawer progress", 0.0),
            swiped: Signal::labeled("drawer swiped", false),
            release: Signal::labeled("drawer release", None),
            height: Signal::labeled("drawer height", 0.0),
            viewport: Signal::labeled("drawer viewport", 0.0),
            nest,
            parent,
            provider,
            id,
        };
        // Counted open with the provider and the parent from its mount (a
        // setup, as a render, writes only what it made: an initially open
        // drawer registers as it mounts) until it unmounts, and each opening
        // and closing between, once each.
        let counted = Rc::new(Cell::new(false));
        let count = Rc::new(move |now: bool| {
            if counted.replace(now) != now {
                d.opened_changed(now);
            }
        });
        let c = Rc::clone(&count);
        gui::while_mounted(move || {
            c(open.get());
            move || c(false)
        });
        // Each opening starts where it rests; the parent counts it, the
        // provider knows; content kept mounted settles at once.
        gui::watch(
            move || open.get(),
            move |now, _| {
                if *now {
                    d.rest();
                    d.swiped.set(false);
                    d.release.set(None);
                }
                count(*now);
                if gui::untracked(|| d.options().keep_mounted) {
                    d.settled(*now);
                }
            },
        );
        // What it tells the parent, in its own entry there from its mount
        // to its unmount: when it opened, its frontmost height and swipe.
        if let Some(parent) = parent {
            let now = move || Told {
                opened: None,
                height: d.frontmost_height(),
                progress: d.frontmost_progress(),
                swiping: d.dragging.get() || d.nested_swiping(),
            };
            let first = gui::untracked(|| Told {
                opened: open.get().then(next_opening),
                ..now()
            });
            let told = Signal::labeled("drawer told its parent", first);
            parent.told.register(told);
            gui::watch(
                move || open.get(),
                move |&o, _| {
                    let was = told.get_untracked().opened;
                    let opened = if o { was.or_else(|| Some(next_opening())) } else { None };
                    if opened != was {
                        told.update(|t| t.opened = opened);
                    }
                },
            );
            gui::watch(now, move |&t, _| {
                let was = told.get_untracked();
                let t = Told {
                    opened: was.opened,
                    ..t
                };
                if t != was {
                    told.set(t);
                }
            });
        }
        if let Some(provider) = provider {
            gui::watch(move || d.frontmost_progress(), move |&p, _| provider.set_progress(p));
        }
        // The window losing focus ends a swipe where it is.
        gui::watch(gui::interact::window_active, move |&active, _| {
            if !active {
                take_swipe(d.opening);
                if let Some(swipe) = take_swipe(d.swipe) {
                    d.finish(&swipe, Self::now());
                }
            }
        });
        // Its count with the provider is undone with its mount (above); its
        // entry with the parent goes with its mount too.
        // reka's drawer warns of a missing title only.
        d.names.warn_when_shown(
            PartNames {
                content: "DrawerContent",
                title: "DrawerTitle",
                description: "DrawerDescription",
            },
            move || d.presence() == Presence::Open,
            || false,
        );
        d
    }

    /// The provider hears that it opened or closed.
    fn opened_changed(self, open: bool) {
        if let Some(provider) = self.provider {
            provider.set_open(self.id, open);
        }
    }

    /// The caller's options now (tracked).
    #[must_use]
    pub fn options(self) -> DrawerOptions {
        let f = self.options.with_untracked(Rc::clone);
        f()
    }

    /// Whether it is open (tracked).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.open.get()
    }

    /// Where the content is (tracked).
    #[must_use]
    pub fn presence(self) -> Presence {
        Presence::of(self.is_open(), self.options().keep_mounted)
    }

    /// The edge it sits on (tracked).
    #[must_use]
    pub fn direction(self) -> SwipeDirection {
        self.options().direction
    }

    /// Whether it is nested in another drawer.
    #[must_use]
    pub fn is_nested(self) -> bool {
        self.parent.is_some()
    }

    /// Whether the overlay shows (tracked): at every level but `Off`, and
    /// not on a nested drawer unless its caller asks.
    #[must_use]
    pub fn has_overlay(self) -> bool {
        let o = self.options();
        o.modal != DrawerModal::Off && (self.parent.is_none() || o.overlay_nested)
    }

    /// Whether a swipe is in progress (tracked): the look drops its
    /// transition while it is.
    #[must_use]
    pub fn dragging(self) -> bool {
        self.dragging.get()
    }

    /// The content's offset from where it rests (tracked).
    #[must_use]
    pub fn offset(self) -> (f32, f32) {
        self.offset.get()
    }

    /// How far toward closed the swipe is, 0 to 1 (tracked): what an
    /// overlay fades by.
    #[must_use]
    pub fn progress(self) -> f32 {
        self.progress.get()
    }

    /// Whether it closed by a swipe (tracked).
    #[must_use]
    pub fn swiped(self) -> bool {
        self.swiped.get()
    }

    /// How fast its release finishes, 0.1 to 1 of the look's duration
    /// (tracked; `None`: at its normal speed).
    #[must_use]
    pub fn release_scalar(self) -> Option<f32> {
        self.release.get()
    }

    /// The content's height, as last measured (tracked; reka's
    /// `--drawer-height`, measured on every edge). While a nested drawer is
    /// open it is not measured again.
    #[must_use]
    pub fn height(self) -> f32 {
        self.height.get()
    }

    /// How many drawers nested in it are open (tracked; reka's
    /// `--nested-drawers`).
    #[must_use]
    pub fn nested_open(self) -> u32 {
        let open = self.nest.reduce(|it| it.filter(|t| t.opened.is_some()).count());
        u32::try_from(open).unwrap_or(u32::MAX)
    }

    /// Whether a drawer nested in it is being swiped (tracked).
    #[must_use]
    pub fn nested_swiping(self) -> bool {
        self.nest.reduce(|mut it| Iterator::any(&mut it, |t| t.swiping))
    }

    /// How far the frontmost nested drawer's swipe is, 0 to 1 (tracked).
    #[must_use]
    pub fn nested_progress(self) -> f32 {
        self.nest.frontmost().map_or(0.0, |t| t.progress)
    }

    /// The frontmost drawer's height from here (tracked; reka's
    /// `--drawer-frontmost-height`): the frontmost nested one's while one is
    /// open, else its own.
    #[must_use]
    pub fn frontmost_height(self) -> f32 {
        match self.nest.frontmost() {
            Some(t) if t.height > 0.0 => t.height,
            _ => self.height.get(),
        }
    }

    /// The frontmost swipe progress from here (tracked).
    fn frontmost_progress(self) -> f32 {
        match self.nest.frontmost() {
            Some(t) => t.progress,
            None => self.progress.get(),
        }
    }

    /// The snap points, resolved against the content and the window
    /// (tracked).
    #[must_use]
    pub fn snap_points(self) -> Vec<Resolved> {
        let o = self.options();
        if o.snap_points.is_empty() {
            return Vec::new();
        }
        let height = self.height.get();
        let viewport = self.viewport.get();
        let viewport = if viewport > 0.0 { viewport } else { height };
        resolve_snap_points(&o.snap_points, height, viewport, ROOT_FONT)
    }

    /// The active snap point (tracked; `None`: fully open).
    #[must_use]
    pub fn active_snap(self) -> Option<SnapPoint> {
        match self.snap {
            Some(m) => m.get(),
            None => self.snap_local.get(),
        }
    }

    /// Rest at `point` (`None`: fully open).
    pub fn set_snap(self, point: Option<SnapPoint>) {
        match self.snap {
            Some(m) => m.set(point),
            None => self.snap_local.set(point),
        }
    }

    /// The index of the active point in `points`: the same value, or a
    /// height within 1 px of it (reka's `activeSnapPointIndex`).
    fn active_index(self, points: &[Resolved]) -> Option<usize> {
        let active = self.active_snap()?;
        let viewport = self.viewport.get();
        let viewport = if viewport > 0.0 { viewport } else { self.height.get() };
        let h = active.height(viewport, ROOT_FONT);
        points
            .iter()
            .position(|p| p.point == active || h.is_some_and(|h| (p.height - h.min(self.height.get())).abs() <= 1.0))
    }

    /// How far below fully open the active snap point rests (tracked; 0
    /// without one).
    fn snap_offset_abs(self) -> f32 {
        let points = self.snap_points();
        self.active_index(&points)
            .and_then(|i| points.get(i))
            .map_or(0.0, |p| p.offset)
    }

    /// The active snap point's offset, signed toward the edge (tracked;
    /// reka's `--drawer-snap-point-offset`): what the look moves the
    /// content by, beside [`Self::offset`].
    #[must_use]
    pub fn snap_offset(self) -> f32 {
        let off = self.snap_offset_abs();
        match self.options().direction {
            SwipeDirection::Up | SwipeDirection::Left => -off,
            SwipeDirection::Down | SwipeDirection::Right => off,
        }
    }

    /// Ask to open or close for `reason`; whether it changed.
    pub fn request(self, open: bool, reason: ChangeReason) -> bool {
        if !self.options.is_alive() {
            return false;
        }
        if self.open.get_untracked() == open {
            return true;
        }
        let o = gui::untracked(|| self.options());
        if let Some(veto) = &o.veto
            && veto.call((open, reason)) == Some(Dismiss::Veto)
        {
            return false;
        }
        self.open.set(open);
        // A caller keeping the value may not have written it.
        self.open.get_untracked() == open
    }

    fn rest(self) {
        self.dragging.set(false);
        self.offset.set((0.0, 0.0));
        self.progress.set(0.0);
        take_swipe(self.swipe);
    }

    fn settled(self, open: bool) {
        if !self.options.is_alive() {
            return;
        }
        if let Some(f) = gui::untracked(|| self.options()).on_settled {
            f.call(open);
        }
    }

    /// The content's node reached `stage`: an entry or an exit that ran to
    /// its end settles it.
    fn stage(self, stage: Stage) {
        if !self.options.is_alive() || gui::untracked(|| self.options().keep_mounted) {
            return;
        }
        let open = self.open.get_untracked();
        match stage {
            Stage::Entered if open => self.settled(true),
            Stage::Exited if !open => self.settled(false),
            _ => {}
        }
    }

    /// The focus of its trigger (the trigger part tracks it).
    #[must_use]
    pub fn trigger_focus(self) -> FocusHandle {
        self.trigger
    }

    /// `d` as the trigger: pressing it toggles the drawer (reason
    /// `TriggerPress`); it opens a dialog, says whether it is expanded and
    /// controls the content while open; it takes focus, which the drawer
    /// gives back to it.
    #[must_use]
    pub fn trigger(self, d: Div) -> Div {
        let open = self.is_open();
        self.linked
            .trigger(d.track_focus(self.trigger), open)
            .aria_has_popup(HasPopup::Dialog)
            .aria_expanded(open)
            .on_click(move |_| {
                self.request(!self.open.get_untracked(), ChangeReason::TriggerPress);
            })
    }

    /// `d` as a close button (reason `ClosePress`).
    #[must_use]
    pub fn close(self, d: Div) -> Div {
        d.on_click(move |_| {
            self.request(false, ChangeReason::ClosePress);
        })
    }

    /// `d` as the title: a heading, the content's name. Its part calls
    /// [`Self::register_title`] in its setup.
    #[must_use]
    pub fn title<K>(self, d: Node<K>) -> Node<K> {
        d.node_ref(self.names.title).role(Role::Heading).aria_level(2)
    }

    /// `d` as the description. Its part calls
    /// [`Self::register_description`] in its setup.
    #[must_use]
    pub fn description<K>(self, d: Node<K>) -> Node<K> {
        d.node_ref(self.names.description)
    }

    /// Called in the title part's setup: the content has a title while
    /// that instance is mounted.
    pub fn register_title(self) {
        self.names.register_title();
    }

    /// Called in the description part's setup.
    pub fn register_description(self) {
        self.names.register_description();
    }

    /// What the content lacks now (tracked): a title. A drawer's
    /// description is not checked (reka's drawer warns of the title only).
    /// A debug build reports it once the content is laid out open.
    #[must_use]
    pub fn missing(self) -> Missing {
        self.names.missing(false)
    }

    /// `d` as the grab handle: decoration, hidden from assistive technology.
    #[must_use]
    pub fn handle(self, d: Div) -> Div {
        d.aria_hidden(true)
    }

    /// `d` as the window-sized frame of the content's layer (reka's
    /// `DrawerViewport`): its height is what snap points in fractions are
    /// of.
    #[must_use]
    pub fn viewport(self, d: Div) -> Div {
        d.on_measured(move |b| put(self.viewport, b.height))
    }

    /// `d` as a region inside the content that never swipes it, whatever
    /// presses it (reka's `data-drawer-swipe-ignore`: a carousel, a map).
    #[must_use]
    pub fn no_swipe(self, d: Div) -> Div {
        d.on_drag(|_| {})
    }

    /// `d` as a region on the page that a swipe away from the drawer's edge
    /// opens it from (reka's `DrawerSwipeArea`, reason `Swipe`): the same
    /// 40 px or 0.3 px/ms; nothing while it is open, and nothing from a
    /// press on a control inside, by any pointer.
    #[must_use]
    pub fn swipe_area(self, d: Div) -> Div {
        d.on_drag(move |e| self.open_drag(e))
    }

    /// `d` as the content: a `dialog` named by its title, the layer and
    /// focus scope the modal level makes, swiped toward its edge to close,
    /// its height measured, its stages settling it. Render it as
    /// [`Self::presence`] says, inside a deferred layer, moved by
    /// [`Self::snap_offset`] and [`Self::offset`].
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        self.linked
            .content(d, false)
            .role(Role::Dialog)
            .labelled_by(self.names.title)
            .described_by(self.names.description)
            .layer(self.layer())
            .on_measured(move |b| self.measured(b))
            .on_stage(move |s| self.stage(s))
            .on_drag(move |e| self.drag(e))
    }

    fn measured(self, b: &Bounds) {
        // While a nested drawer is open its height stays as it was.
        if gui::untracked(|| self.nested_open()) > 0 && self.height.get_untracked() > 0.0 {
            return;
        }
        put(self.height, b.height);
        if let Some(provider) = self.provider {
            provider.set_height(b.height);
        }
    }

    fn layer(self) -> Layer {
        let o = self.options();
        let (modal, trapped, policy) = match o.modal {
            DrawerModal::On => (true, true, DismissPolicy::NONE.escape(true).pointer_outside(true)),
            DrawerModal::TrapFocus => (false, true, DismissPolicy::DEFAULT),
            DrawerModal::Off => (false, false, DismissPolicy::DEFAULT),
        };
        let scope = FocusScope::new().trap(trapped).auto_focus(o.initial_focus);
        let scope = match o.final_focus {
            FinalFocus::Trigger => scope.restore(true).return_to(self.trigger),
            FinalFocus::Node(h) => scope.restore(true).return_to(h),
            FinalFocus::Nowhere => scope.restore(false),
        };
        Layer::new()
            .modal(modal)
            .dismiss(policy)
            .focus_scope(scope)
            .on_dismiss(move |r| self.dismiss(r))
    }

    fn dismiss(self, reason: &DismissReason) -> Dismiss {
        if !self.options.is_alive() {
            return Dismiss::Veto;
        }
        let modal = gui::untracked(|| self.options()).modal;
        let reason = match reason {
            DismissReason::Escape => ChangeReason::EscapeKey,
            DismissReason::PointerOutside(_) => {
                if self.dragging.get_untracked() || (modal == DrawerModal::On && is_context_press(reason)) {
                    return Dismiss::Veto;
                }
                ChangeReason::OutsidePress
            }
            DismissReason::FocusOutside => {
                if self.dragging.get_untracked() {
                    return Dismiss::Veto;
                }
                ChangeReason::FocusOutside
            }
            DismissReason::Scroll | DismissReason::Programmatic => {
                self.open.set(false);
                return Dismiss::Close;
            }
        };
        if self.request(false, reason) {
            Dismiss::Close
        } else {
            Dismiss::Veto
        }
    }

    /// The clock, exact: `f32` seconds would quantize the 16 ms samples
    /// after days of uptime.
    fn now() -> Duration {
        gui::clock::current_time()
    }

    /// Whether a press `e` starts no swipe: a mouse's or a pen's on a
    /// control (`any_pointer`: anyone's).
    fn spared(e: &DragEvent, any_pointer: bool) -> bool {
        e.on_control && (any_pointer || e.kind != PointerKind::Touch)
    }

    fn drag(self, e: &DragEvent) {
        let now = Self::now();
        match e.phase {
            DragPhase::Start => {
                let o = gui::untracked(|| self.options());
                if !self.open.get_untracked() || gui::untracked(|| self.nested_swiping()) || Self::spared(e, false) {
                    return;
                }
                let direction = o.direction;
                let size = if direction.vertical() {
                    e.bounds.height
                } else {
                    e.bounds.width
                };
                let room = if o.snap_points.is_empty() {
                    0.0
                } else {
                    gui::untracked(|| self.snap_offset_abs())
                };
                self.swipe
                    .set(Some(Swipe::with_room(direction, size, room, (e.x, e.y))));
            }
            DragPhase::Move => {
                let moved = move_swipe(self.swipe, (e.x, e.y), now);
                if let Some(m) = moved {
                    if !self.dragging.get_untracked() {
                        self.dragging.set(true);
                    }
                    self.offset.set(m.offset);
                    self.progress.set(m.progress);
                }
            }
            DragPhase::End => {
                if let Some(swipe) = take_swipe(self.swipe) {
                    self.finish(&swipe, now);
                }
            }
            // Cancelled (the content was disabled mid-swipe): it slides
            // back where it rests, neither closing nor snapping.
            DragPhase::Cancel => {
                if self.swipe.with_untracked(Option::is_some) {
                    self.rest();
                }
            }
        }
    }

    /// Let go of `swipe` at `now`: close, slide back, or rest at a snap point.
    fn finish(self, swipe: &Swipe, now: Duration) {
        let release = swipe.release(now);
        if release == Release::None {
            return;
        }
        let o = gui::untracked(|| self.options());
        let direction = o.direction;
        let (vx, vy) = swipe.velocity(now);
        let (ox, oy) = swipe.offset();
        let (along, speed) = if direction.vertical() { (oy, vy) } else { (ox, vx) };
        let points = gui::untracked(|| self.snap_points());
        let snap_offset = gui::untracked(|| self.active_index(&points))
            .and_then(|i| points.get(i))
            .map_or(0.0, |p| p.offset);
        let scalar = release_scalar(direction, swipe.size(), along, snap_offset, speed);
        self.dragging.set(false);
        let close = if points.is_empty() {
            release == Release::Dismiss
        } else {
            let active = gui::untracked(|| self.active_index(&points));
            let height = self.height.get_untracked();
            let target = snap_release(
                &points,
                active,
                height,
                swipe.displacement(),
                direction.displacement(vx, vy),
                o.sequential_snap,
            );
            match target {
                Some(SnapTarget::Close) => true,
                Some(SnapTarget::Point(i)) => {
                    if let Some(p) = points.get(i) {
                        self.set_snap(Some(p.point));
                    }
                    false
                }
                None => false,
            }
        };
        if close {
            self.release.set(scalar);
            if self.request(false, ChangeReason::Swipe) {
                self.swiped.set(true);
                return;
            }
            self.release.set(None);
        }
        self.rest();
    }

    /// A drag on the swipe area.
    fn open_drag(self, e: &DragEvent) {
        let now = Self::now();
        match e.phase {
            DragPhase::Start => {
                if self.open.get_untracked() || Self::spared(e, true) {
                    return;
                }
                let way = gui::untracked(|| self.options()).direction.opposite();
                let size = if way.vertical() {
                    e.bounds.height
                } else {
                    e.bounds.width
                };
                self.opening.set(Some(Swipe::new(way, size, (e.x, e.y))));
            }
            DragPhase::Move => {
                move_swipe(self.opening, (e.x, e.y), now);
            }
            DragPhase::End => {
                let swipe = take_swipe(self.opening);
                if let Some(swipe) = swipe
                    && swipe.release(now) == Release::Dismiss
                {
                    self.request(true, ChangeReason::Swipe);
                }
            }
            DragPhase::Cancel => {
                take_swipe(self.opening);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        Release, SnapPoint, SnapTarget, Swipe, SwipeDirection, release_scalar, resolve_snap_points, snap_release,
    };

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A bottom drawer 300 px tall pressed at (100, 100), moved down to
    /// each `y` 20 ms apart; released `after` ms after the last move.
    fn swipe(ys: &[f32], after: u64) -> (Swipe, Release) {
        let mut s = Swipe::new(SwipeDirection::Down, 300.0, (100.0, 100.0));
        let mut t = 0_u64;
        for &y in ys {
            t = t.saturating_add(20);
            s.moved((100.0, y), ms(t));
        }
        let r = s.release(ms(t.saturating_add(after)));
        (s, r)
    }

    #[test]
    fn a_drawer_at_a_snap_point_follows_the_pointer_open_up_to_its_room() {
        // A bottom drawer 300 px tall resting 100 px down (its snap point).
        let mut s = Swipe::with_room(SwipeDirection::Down, 300.0, 100.0, (100.0, 200.0));
        s.moved((100.0, 140.0), ms(20));
        assert_eq!(s.offset(), (0.0, -60.0), "60 px open, within its 100 px room: followed");
        s.moved((100.0, 84.0), ms(40));
        assert_eq!(s.offset(), (0.0, -104.0), "16 px past its room: the square root, 4 px");
        let mut s = Swipe::new(SwipeDirection::Down, 300.0, (100.0, 200.0));
        s.moved((100.0, 184.0), ms(20));
        assert_eq!(s.offset(), (0.0, -4.0), "fully open, no room: damped from the start");
    }

    #[test]
    fn a_swipe_starts_after_one_pixel() {
        let (s, r) = swipe(&[100.5], 0);
        assert!(!s.started());
        assert_eq!(r, Release::None);
    }

    #[test]
    fn forty_pixels_or_a_flick_close_it() {
        // Slowly to 140: 40 px, 1 px per 20 ms... no, the last step is 5 px.
        let (_, r) = swipe(&[110.0, 120.0, 130.0, 135.0, 140.0], 100);
        assert_eq!(r, Release::Dismiss, "40 px closes it whatever the speed");
        let (_, r) = swipe(&[110.0, 120.0, 130.0, 135.0, 139.0], 100);
        assert_eq!(r, Release::Back, "39 px, its speed stale: back");
        // 8 px in 20 ms is 0.4 px/ms, past 0.3.
        let (_, r) = swipe(&[105.0, 113.0], 0);
        assert_eq!(r, Release::Dismiss, "a flick closes it");
        let (_, r) = swipe(&[105.0, 113.0], 81);
        assert_eq!(r, Release::Back, "a speed older than 80 ms is none");
    }

    #[test]
    fn turning_back_cancels_and_away_is_a_rubber_band() {
        // Past 20 px, then back more than 10.
        let (_, r) = swipe(&[130.0, 160.0, 149.0, 148.0], 0);
        assert_eq!(r, Release::Back, "turned back from 60 to 48: cancelled");
        let mut s = Swipe::new(SwipeDirection::Down, 300.0, (0.0, 100.0));
        let m = s.moved((0.0, 64.0), ms(20)).expect("started");
        assert!((m.offset.1 + 6.0).abs() < 1e-4, "36 px up is 6 px: {:?}", m.offset);
        assert!(m.progress.abs() < f32::EPSILON);
        let m = s.moved((0.0, 134.0), ms(40)).expect("moving");
        assert!((m.progress - 34.0 / 340.0).abs() < 1e-6, "progress over size plus 40");
    }

    #[test]
    fn the_release_scalar_follows_the_time_left() {
        // 260 px left at 4 px/ms is 65 ms, held to 80: the fastest, 0.1.
        let s = release_scalar(SwipeDirection::Down, 300.0, 40.0, 0.0, 4.0).expect("flung");
        assert!((s - 0.1).abs() < 1e-6);
        // Slow: 360 ms or more is 1.
        let s = release_scalar(SwipeDirection::Down, 300.0, 40.0, 0.0, 0.5).expect("flung");
        assert!((s - 1.0).abs() < 1e-6);
        assert_eq!(
            release_scalar(SwipeDirection::Up, 300.0, -40.0, 0.0, 0.1),
            None,
            "too slow"
        );
    }

    #[test]
    fn snap_points_resolve_and_release_as_reka() {
        let points = [
            SnapPoint::Number(0.5),
            SnapPoint::Px(148.0),
            SnapPoint::Rem(10.0),
            SnapPoint::Px(148.6),
            SnapPoint::Number(-1.0),
        ];
        let r = resolve_snap_points(&points, 500.0, 800.0, 16.0);
        let heights: Vec<f32> = r.iter().map(|p| p.height).collect();
        assert_eq!(
            heights,
            [148.0, 160.0, 400.0],
            "149 is within 1 px of 148; -1 is dropped"
        );
        assert_eq!(r.iter().map(|p| p.offset).collect::<Vec<_>>(), [352.0, 340.0, 100.0]);
        // Resting at 400 (offset 100), dragged 30 toward closed, slowly: back.
        assert_eq!(
            snap_release(&r, Some(2), 500.0, 30.0, 0.0, false),
            Some(SnapTarget::Point(2))
        );
        // Flung at 1 px/ms: 130 + 300 = 430, nearest closed? 70 from 500, 78 from 352.
        assert_eq!(
            snap_release(&r, Some(2), 500.0, 30.0, 1.0, false),
            Some(SnapTarget::Close)
        );
        // Sequential: one point at most, the way of the drag.
        assert_eq!(
            snap_release(&r, Some(2), 500.0, 30.0, 0.6, true),
            Some(SnapTarget::Point(1))
        );
        assert_eq!(
            snap_release(&r, None, 500.0, 0.0, 0.0, false),
            Some(SnapTarget::Point(2))
        );
    }
}
