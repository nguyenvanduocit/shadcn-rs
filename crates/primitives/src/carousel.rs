//! Slides shown a view at a time, moved by buttons, keys, the pointer, the
//! wheel, indicators or a timer (reka's `Carousel`: `CarouselRoot`,
//! `CarouselViewport`, `CarouselContent`, `CarouselItem`,
//! `CarouselPrevious`, `CarouselNext`, `CarouselIndicatorGroup`,
//! `CarouselIndicator`, `CarouselRotationControl`; shadcn's carousel is
//! Embla's, whose demos the verdict compares against).
//!
//! **Model.** The snap shown, from 0, the part's `v-model`. A track of n
//! slides, each `per_view` of the viewport, aligned to the start with the
//! trailing space trimmed, stops at [`snaps`] places (Embla's
//! `scrollSnapList()`). A snap past the last reads as the last: fewer
//! slides clamp it (`Carousel/useCarousel.ts:383-386`), and the user moves
//! only within them (`:369-379`). A step is possible with more than one
//! snap, and wrapping or short of the end (`:446-447`).
//!
//! **Where the track is.** [`Carousel::offset`] is what the look translates
//! the track by: the snap's place, plus what a drag or a free fling has
//! moved it, mirrored right to left; [`Carousel::motion`] and
//! [`Carousel::move_duration`] say how it gets there (eased over the
//! caller's [`CarouselOptions::duration`] to a snap, 250 ms back from an
//! overscroll, at once while the pointer holds it and under reduced
//! motion). Bound to the track's node, a move renders nothing.
//!
//! **Keys** (focus on the root itself; a descendant's keys are its own,
//! `Carousel/useCarouselViewport.ts:819-820`):
//!
//! | Key | Result |
//! | --- | --- |
//! | Tab | the root is a Tab stop |
//! | ArrowRight (horizontal; ArrowLeft right to left), ArrowDown (vertical) | the next snap |
//! | the opposite arrow | the previous snap |
//! | Home, End | the first, the last snap |
//! | any of them with Alt, Control or Command | not handled |
//!
//! In a group of indicators the arrows (both axes, wrapping) move focus
//! between them and show the snap of the one focused, Home and End go to
//! the first and the last (reka's tabs, [`crate::Roving`]).
//!
//! **Pointer** (`Carousel/useCarouselViewport.ts:621-726`), when
//! [`CarouselOptions::draggable`]: a press on the viewport drags once it
//! has moved [`DRAG_START`] px along the axis; a move mostly across it is
//! the page's and drags nothing. The track follows the pointer; past
//! either end it moves [`OVERSCROLL_RESISTANCE`] of the way and springs
//! back over [`BOUNCE`] when let go. A drag of more than [`CLICK_SLOP`] px
//! is not a click: nothing under it takes the press. On release the
//! velocity of the last [`VELOCITY_WINDOW`] projects [`PROJECTION`] ahead,
//! and the nearest snap to that place is shown (a tie of half a pixel keeps
//! the current one), one snap from where the drag began at most unless
//! [`CarouselOptions::skip_snaps`]. With [`CarouselOptions::drag_free`] the
//! track glides instead, slowing by [`DECAY`] each millisecond, and rests
//! where it stops; the snap shown is then the nearest.
//!
//! **Wheel** (`:772-818`): a turn along the axis pages the track, as a
//! scroll that then snaps would; one across it pages too with
//! [`CarouselOptions::wheel`] (reka's `wheel`), else it is the page's.
//! [`WHEEL_PAGE`] of delta is one snap; one gesture (turns less than
//! [`WHEEL_GAP`] apart) is one snap at most. At an end a new gesture is the
//! page's.
//!
//! A key handled, a button or an indicator pressed, a drag or a wheel is
//! the user's: it stops the rotation when the caller asks for that
//! ([`Rotation::stop_on_interaction`]).
//!
//! **Going round** ([`CarouselOptions::wrap`], reka's `loop`): when a
//! slide's worth stays out of view on either side ([`loop_fits`]), there
//! is a snap at every slide and no end. Each slide is moved by whole
//! tracks into the lap that puts it within half a track of the view
//! (Embla's slide looper; reka reorders with CSS `order`), placed around
//! the middle of each move so the slides it passes are there all the
//! way; a move longer than they cover goes at once. Steps keep on round,
//! a caller's or an indicator's snap is reached the shorter way, a drag or
//! a fling has no end, and at rest the track is taken back to its first
//! lap with nothing moving on screen. A track too short to go round runs
//! back from the last snap to the first instead.
//!
//! **Fading** ([`CarouselOptions::fade`], reka's `fade`): the slides lie
//! on the first, one snap each; the one shown is opaque, the rest clear
//! and inert, and the track does not move. A swipe of [`SWIPE_DISTANCE`]
//! px, or one faster than [`SWIPE_VELOCITY`] px/ms, steps once; with
//! [`CarouselOptions::wheel`] any wheel pages. It settles at once (the
//! look eases the opacity).
//!
//! **Height** ([`CarouselOptions::auto_height`], reka's `autoHeight`,
//! horizontal only): each slide keeps its own height and the viewport is
//! as tall as the tallest of those at rest in view ([`Carousel::height`],
//! bound to the viewport's node).
//!
//! **Focus handed on** (`useCarouselViewport.ts:208-238`): focus inside a
//! slide that goes out of view goes to the first Tab stop of the slide
//! coming in, else to the root, so it is never lost on an inert slide
//! ([`gui::focus::hand_on`]).
//!
//! **Rotation** (reka's `autoplay` and `autoScroll`, Embla's Autoplay and
//! AutoScroll plugins, `Carousel/useCarouselRotation.ts`): [`Pace::Step`]
//! is one snap on every `delay` (reka's 4000 ms unless set), counted again
//! from each change of snap ([`Carousel::next_step`] says when it is due,
//! for a look's progress); after the last it goes back to the first, or
//! stops there when asked, and goes on round a going-round track.
//! [`Pace::Scroll`] runs the track on at a speed ([`AUTO_SCROLL`], 60 px/s,
//! unless set), the snap shown following it; it stops at an end of a
//! track that does not go round, and paused or stopped the track goes to
//! the nearest snap. Either is *playing* until the user stops it, and
//! *paused* while the mouse is over the carousel (when asked), a finger
//! holds it, the pointer drags it, the window is hidden, or the carousel
//! is out of the window. Keyboard focus coming into it stops it (APG),
//! and so does a user who prefers reduced motion, at once and whenever
//! that changes; the rotation button plays it again.
//!
//! **Availability.** Previous and Next cannot be used where they cannot
//! step. reka's are unavailable there (`aria-disabled`, keeping focus,
//! `Carousel/useCarousel.ts:80-82`); Embla's, which shadcn's look and its
//! verdict follow, are disabled. The caller says which
//! ([`CarouselOptions::at_end`]).
//!
//! **To assistive technology.** The root is a region described as a
//! "carousel"; each slide a group described as a "slide" and named "N of
//! M", or, with a group of indicators, a tab panel named by its snap's
//! tab; a slide out of view is hidden from it and inert. The content is a
//! polite live region, off while rotating (`Carousel/useCarousel.ts:162-163`,
//! `CarouselRoot.vue:249-250`); Previous and Next are named "Previous
//! slide" and "Next slide" and control the content (`useCarousel.ts:74-95`);
//! indicators are tabs named "Slide N" in a tab list named "Choose slide to
//! display", or, outside a group, buttons named "Go to slide N"; the
//! rotation button is named "Stop slide rotation" or "Start slide
//! rotation" as it is. Those are English strings, the defaults of
//! [`CarouselNames`].
//!
//! **Settled.** [`Carousel::on_settle`] hears the snap shown once the track
//! has come to rest on it, once per change (reka's `@settle`); not while an
//! auto-scroll runs.

use gui::engine::{WorldCell, WorldRefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gui::prelude::*;
use gui::style::Calc;
use gui::{
    Axis, Bounds, Direction, Div, DragEvent, DragPhase, FocusHandle, Interaction, KeyEvent, Live, Members, NamedKey,
    NodeRef, Signal, UseModel,
};

use crate::availability::Availability;
use crate::collection::Item;
use crate::direction::ReadDirection;
use crate::hover::Delay;
use crate::keymap::{Arrows, Keymap, Wrap};
use crate::relation::{Linked, Points};
use crate::roving::{Nav, Roving};

/// How often a rotation steps unless the caller says otherwise
/// (`Carousel/useCarousel.ts:42`).
pub const ROTATION: Duration = Duration::from_millis(4000);
/// How fast an auto-scroll runs unless the caller says otherwise, px/s
/// (`useCarousel.ts:52`).
pub const AUTO_SCROLL: f32 = 60.0;
/// How far a press moves along the axis before it drags
/// (`useCarouselViewport.ts:20`).
pub const DRAG_START: f32 = 3.0;
/// How far a drag goes before it is no click (`:22`).
pub const CLICK_SLOP: f32 = 10.0;
/// How far a swipe goes, or how fast (px/ms), to change a faded slide (`:24-25`).
pub const SWIPE_DISTANCE: f32 = 40.0;
pub const SWIPE_VELOCITY: f32 = 0.3;
/// How far ahead the release velocity projects, in ms (`:27`).
pub const PROJECTION: f32 = 120.0;
/// The time the release velocity is measured over (`:29`).
pub const VELOCITY_WINDOW: Duration = Duration::from_millis(80);
/// What a free fling keeps of its speed each millisecond (`:31`).
pub const DECAY: f32 = 0.996;
/// The wheel delta that is one snap (`:33`).
pub const WHEEL_PAGE: f32 = 30.0;
/// The pause that ends a wheel gesture (`:34`).
pub const WHEEL_GAP: Duration = Duration::from_millis(180);
/// The share of a drag past an end the track follows (`:38`).
pub const OVERSCROLL_RESISTANCE: f32 = 0.3;
/// How long the track takes back from an overscroll (`:613`).
pub const BOUNCE: Duration = Duration::from_millis(250);
/// How long a move to a snap takes unless the caller says otherwise
/// (shadcn's Embla, as its look eases it).
pub const MOVE: Duration = Duration::from_millis(400);
/// One frame of a free fling or an auto-scroll, in ms.
const FRAME_MS: f32 = 16.0;
const FRAME: Duration = Duration::from_millis(16);
/// The longest frame an auto-scroll steps by, in ms (`:948`).
const LONGEST_FRAME_MS: f32 = 64.0;

fn float(n: usize) -> f32 {
    u16::try_from(n).map_or(f32::from(u16::MAX), f32::from)
}

#[expect(
    clippy::as_conversions,
    reason = "a small whole number of snaps or laps, exactly an f32"
)]
fn real(n: i32) -> f32 {
    n as f32
}

#[expect(
    clippy::as_conversions,
    reason = "a rounded place in snaps, saturating as a float-to-int cast does"
)]
fn whole(f: f32) -> i32 {
    f.round() as i32
}

fn signed(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// How many places a track of `count` slides, each `per_view` of the
/// viewport, stops at, aligned to the start with no space trailing the
/// last slide (Embla's `containScroll: 'trimSnaps'`).
#[must_use]
pub fn snaps(count: usize, per_view: f32) -> usize {
    let end = end(count, per_view);
    // Every slide start short of the end, then the end itself; the epsilon
    // keeps a start that lands on the end from counting twice.
    let short = (end / per_view - 1e-3).ceil().max(0.0);
    (0..count)
        .take_while(|&i| float(i) < short)
        .count()
        .saturating_add(1)
        .min(count.max(1))
}

/// The track's furthest place, in viewports from its start.
fn end(count: usize, per_view: f32) -> f32 {
    (float(count) * per_view - 1.0).max(0.0)
}

/// Where snap `k` is, in viewports from the track's start.
#[must_use]
pub fn snap_position(k: usize, count: usize, per_view: f32) -> f32 {
    (float(k) * per_view).min(end(count, per_view))
}

/// Whether a track of `count` slides, each `per_view` of a viewport
/// `viewport` px long, can go round: a slide's worth stays out of view on
/// either side (`useCarouselViewport.ts:290-295`, one slide a step). Not
/// laid out yet, it is taken to fit, as reka takes it.
#[must_use]
pub fn loop_fits(count: usize, per_view: f32, viewport: f32) -> bool {
    let room = float(count) * per_view - 1.0;
    count > 1 && (viewport <= 0.0 || room * viewport >= 2.0 * per_view * viewport - 1.0)
}

/// The lap slide `i` of a going-round track is placed in, so that it lies
/// within half a track of the view's centre with the view at `anchor`
/// (viewports; Embla's `slideLooper`).
fn slide_lap(i: usize, count: usize, per_view: f32, anchor: f32) -> i32 {
    let length = float(count) * per_view;
    let centre = float(i) * per_view + per_view / 2.0;
    whole((anchor + 0.5 - centre) / length)
}

/// The snap nearest `position` (viewports), `preferred` kept on a tie of
/// `tie` (`Carousel/utils.ts:170-184`).
fn nearest_snap(position: f32, count: usize, per_view: f32, preferred: Option<usize>, tie: f32) -> usize {
    let n = snaps(count, per_view);
    let mut nearest = 0;
    let mut distance = f32::INFINITY;
    for k in 0..n {
        let d = (snap_position(k, count, per_view) - position).abs();
        if d < distance - tie {
            distance = d;
            nearest = k;
        }
    }
    match preferred {
        Some(p) if p < n && (snap_position(p, count, per_view) - position).abs() < distance + tie => p,
        _ => nearest,
    }
}

/// Which slides, starting at `starts` (viewports), show any of themselves
/// (more than `slop` viewports) with the view at `position`
/// (`Carousel/utils.ts:197-210`, its threshold 0).
fn in_view(starts: &[f32], per_view: f32, position: f32, slop: f32) -> Vec<bool> {
    starts
        .iter()
        .map(|&start| (start + per_view).min(position + 1.0) - start.max(position) > slop)
        .collect()
}

/// Where each of `count` slides starts, in viewports: in a row, or each
/// in its lap around `anchor` on a going-round track.
fn starts(count: usize, per_view: f32, looping: bool, anchor: f32) -> Vec<f32> {
    let length = float(count) * per_view;
    (0..count)
        .map(|i| {
            let lap = if looping {
                slide_lap(i, count, per_view, anchor)
            } else {
                0
            };
            float(i) * per_view + real(lap) * length
        })
        .collect()
}

/// The first snap whose view holds the whole of slide `i`: the snap its
/// tab panel is named by.
fn snap_of(i: usize, count: usize, per_view: f32) -> usize {
    let n = snaps(count, per_view);
    let start = float(i) * per_view;
    (0..n)
        .find(|&k| {
            let p = snap_position(k, count, per_view);
            start >= p - 1e-3 && start + per_view <= p + 1.0 + 1e-3
        })
        .unwrap_or_else(|| n.saturating_sub(1))
}

/// The velocity of `samples` (ms, px) over the window before `now`, in px
/// per ms (`useCarouselViewport.ts:504-511`).
fn release_velocity(samples: &[(f32, f32)], now: f32) -> f32 {
    let window = VELOCITY_WINDOW.as_secs_f32() * 1000.0;
    let recent: Vec<_> = samples.iter().filter(|(t, _)| now - t <= window).collect();
    match (recent.first(), recent.last()) {
        (Some(first), Some(last)) if recent.len() >= 2 && last.0 > first.0 => (last.1 - first.1) / (last.0 - first.0),
        _ => 0.0,
    }
}

/// How a rotation moves the track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pace {
    /// One snap every `delay` (reka's `autoplay`, Embla's Autoplay); after
    /// the last back to the first, or a stop there with `stop_on_last`
    /// (ignored on a track that goes round).
    Step { delay: Duration, stop_on_last: bool },
    /// On and on at `speed` px/s (reka's `autoScroll`, Embla's AutoScroll),
    /// towards the start with `backward`; it stops at an end of a track
    /// that does not go round.
    Scroll { speed: f32, backward: bool },
}

/// How a carousel rotates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rotation {
    pub pace: Pace,
    /// The mouse over the carousel pauses it until it leaves.
    pub pause_on_hover: bool,
    /// The user's keys, presses, drags and wheel stop it.
    pub stop_on_interaction: bool,
}

impl Rotation {
    /// A step every `delay` (reka's `autoplay`).
    #[must_use]
    pub const fn step(delay: Duration) -> Self {
        Self {
            pace: Pace::Step {
                delay,
                stop_on_last: false,
            },
            pause_on_hover: true,
            stop_on_interaction: true,
        }
    }

    /// An auto-scroll at `speed` px/s (reka's `autoScroll`).
    #[must_use]
    pub const fn scroll(speed: f32) -> Self {
        Self {
            pace: Pace::Scroll { speed, backward: false },
            pause_on_hover: true,
            stop_on_interaction: true,
        }
    }

    const fn delay(self) -> Option<Duration> {
        match self.pace {
            Pace::Step { delay, .. } => Some(delay),
            Pace::Scroll { .. } => None,
        }
    }
}

impl Default for Rotation {
    /// reka's `autoplay`: a step every 4000 ms.
    fn default() -> Self {
        Self::step(ROTATION)
    }
}

/// What the caller decides of a carousel, read when it is used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarouselOptions {
    /// The axis the slides run along.
    pub orientation: Axis,
    /// The share of the viewport one slide takes.
    pub per_view: f32,
    /// Past the last snap comes the first (reka's `loop`): the slides go
    /// round when a slide's worth stays out of view on either side
    /// ([`loop_fits`]), else the track runs back.
    pub wrap: bool,
    /// The slides lie on one another and the one shown fades in (reka's
    /// `fade`): one snap a slide, the track does not move.
    pub fade: bool,
    /// The viewport is as tall as the slides shown (reka's `autoHeight`;
    /// horizontal carousels).
    pub auto_height: bool,
    /// What Previous and Next are where they cannot step.
    pub at_end: Availability,
    /// The pointer drags the track (reka's `draggable`, on unless set).
    pub draggable: bool,
    /// A fling glides and rests where it stops (reka's `dragFree`).
    pub drag_free: bool,
    /// A fling may pass more than one snap (reka's `skipSnaps`).
    pub skip_snaps: bool,
    /// The wheel across the axis pages too, and any wheel pages a faded
    /// carousel (reka's `wheel`).
    pub wheel: bool,
    /// How long a move to a snap takes: the look eases over it, and the
    /// snap is settled after it.
    pub duration: Duration,
}

impl Default for CarouselOptions {
    fn default() -> Self {
        Self {
            orientation: Axis::Horizontal,
            per_view: 1.0,
            wrap: false,
            fade: false,
            auto_height: false,
            at_end: Availability::Unavailable,
            draggable: true,
            drag_free: false,
            skip_snaps: false,
            wheel: false,
            duration: MOVE,
        }
    }
}

/// What a screen reader is told: English strings by default; the numbers
/// count from 1.
#[derive(Clone)]
pub struct CarouselNames {
    pub carousel: String,
    pub slide: String,
    pub previous: String,
    pub next: String,
    /// A slide's name, of its number and the count ("2 of 5").
    pub slide_label: Rc<dyn Fn(usize, usize) -> String>,
    /// The tab list of indicators.
    pub indicators: String,
    /// An indicator in a group: a tab ("Slide 2").
    pub tab: Rc<dyn Fn(usize) -> String>,
    /// An indicator alone: a button ("Go to slide 2").
    pub indicator: Rc<dyn Fn(usize) -> String>,
    /// The rotation button while it rotates, and while it does not.
    pub stop_rotation: String,
    pub start_rotation: String,
}

impl std::fmt::Debug for CarouselNames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CarouselNames")
            .field("carousel", &self.carousel)
            .field("slide", &self.slide)
            .finish_non_exhaustive()
    }
}

impl Default for CarouselNames {
    fn default() -> Self {
        Self {
            carousel: "carousel".to_owned(),
            slide: "slide".to_owned(),
            previous: "Previous slide".to_owned(),
            next: "Next slide".to_owned(),
            slide_label: Rc::new(|n, m| format!("{n} of {m}")),
            indicators: "Choose slide to display".to_owned(),
            tab: Rc::new(|n| format!("Slide {n}")),
            indicator: Rc::new(|n| format!("Go to slide {n}")),
            stop_rotation: "Stop slide rotation".to_owned(),
            start_rotation: "Start slide rotation".to_owned(),
        }
    }
}

/// How the track gets to where [`Carousel::offset`] puts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// Eased to a snap over [`CarouselOptions::duration`].
    Snap,
    /// Back from an overscroll, easing out over [`BOUNCE`].
    Bounce,
    /// At once: the pointer holds it, a fling or an auto-scroll moves it,
    /// a going-round track starts a lap afresh, or the user prefers
    /// reduced motion.
    Follow,
}

/// A slide's identity among the carousel's slides, made in its setup: it
/// carries what the slide has of its own, so the carousel keeps no table of
/// slides a setup would write into (the slides laid out are the ones it
/// knows).
#[derive(Clone, Copy)]
pub struct SlideKey {
    id: gui::Registration,
    meta: SlideMeta,
}

impl PartialEq for SlideKey {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for SlideKey {}

impl std::hash::Hash for SlideKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl std::fmt::Debug for SlideKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SlideKey").field(&self.id).finish()
    }
}

/// What a slide has of its own, made with its key.
#[derive(Clone, Copy)]
struct SlideMeta {
    /// Whether focus is inside it.
    interaction: Interaction,
    /// Its node, for the focus it is handed.
    node: NodeRef,
}

/// A press on the viewport, from its start.
struct Drag {
    from: (f32, f32),
    /// When it went down (ms), for a faded swipe's speed.
    time: f32,
    /// The track's place (px) when it began, and the snap shown, as a
    /// place among every lap.
    base: f32,
    virt: i32,
    started: bool,
    /// Mostly across the axis: the page's.
    aborted: bool,
    distance: f32,
    /// The track's place (px, unclamped) against time (ms).
    samples: Vec<(f32, f32)>,
    position: f32,
    /// How far the pointer went towards the end (px), signed.
    travel: f32,
}

/// Its cells are the world's that made the carousel: a check writes none
/// of a live one's (ADR-0034, S7).
struct State {
    options: Rc<dyn Fn() -> CarouselOptions>,
    direction: ReadDirection,
    names: CarouselNames,
    content: NodeRef,
    /// The root's focus: its keys act only while it has it.
    focus: FocusHandle,
    interaction: Interaction,
    drag: WorldRefCell<Option<Drag>>,
    /// The wheel gesture's delta so far, and whether it has paged.
    wheel: WorldCell<(f32, bool)>,
    /// The snap last settled, and who hears it.
    settled: WorldCell<usize>,
    on_settle: Option<gui::Callback<usize>>,
    /// A release, a fling or an auto-scroll set where the track goes and
    /// how: the next change of snap keeps that.
    rest: WorldCell<bool>,
    /// Where the track was last sent, in viewports among every lap.
    at: WorldCell<f32>,
    /// The snap a move asked for, waiting for the model to take it: its
    /// lap (the shorter way round for none), and whether it moves the track
    /// on its way (a fling, an auto-scroll), settling nothing.
    pending: WorldCell<Option<(usize, Option<i32>, bool)>>,
    /// An auto-scroll runs: its last frame (ms).
    scrolling: WorldCell<Option<f32>>,
    /// The slides laid out, as a set, kept from pass to pass for letting go
    /// of the heights of those that left.
    present: WorldCell<HashSet<SlideKey>>,
    /// The slides and heights those passes stepped through: the measure of
    /// their work, for tests.
    prune_work: WorldCell<u64>,
}

/// A carousel's state: made once in its root's setup, `Copy`.
pub struct Carousel<P: 'static> {
    index: UseModel<P, usize>,
    /// Which lap of a going-round track the snap shown is in.
    lap: Signal<i32>,
    /// Where the slides of a going-round track are placed around, in
    /// viewports among every lap: the middle of a move, the place of a
    /// drag, the rest.
    anchor: Signal<f32>,
    slides: Signal<Members<SlideKey>>,
    /// Each snap's tab and panel nodes, for a group of indicators: issued
    /// as the snap is first asked for, held by its indicator and its slides
    /// while each is on screen, let go as the last of them leaves.
    links: gui::Keyed<usize, (NodeRef, NodeRef)>,
    /// The slides as last laid out, in order.
    order: Signal<Vec<SlideKey>>,
    /// Each slide's height as last laid out.
    heights: Signal<HashMap<SlideKey, f32>>,
    /// The viewport's length along the axis, as last laid out.
    viewport: Signal<f32>,
    /// What a drag or a fling has moved the track past its snap, px.
    shift: Signal<f32>,
    motion: Signal<Motion>,
    dragging: Signal<bool>,
    /// Which slides show, by their order.
    shown: Signal<Vec<bool>>,
    /// The slides the track passes on its way to the snap (each in view
    /// somewhere between where it was and where it goes; the one fading
    /// out), shown until it gets there.
    passing: Signal<Vec<bool>>,
    rotation: Signal<Option<Rotation>>,
    playing: Signal<bool>,
    /// When the count to the next step began (the clock's time), and its delay.
    due: Signal<Option<(Duration, Duration)>>,
    /// The mouse is over it; a finger holds it.
    hovered: Signal<bool>,
    held: Signal<bool>,
    /// The root's box in the window, as last laid out.
    bounds: Signal<Option<Bounds>>,
    /// Groups of indicators mounted: the slides are tab panels.
    tabbed: Signal<usize>,
    roving: Roving<usize>,
    tick: Delay,
    fling: Delay,
    wheel_gap: Delay,
    settle: Delay,
    /// When the track gets to the snap: the slides it passed sleep.
    passed: Delay,
    scroll: Delay,
    state: Signal<State>,
}

impl<P: 'static> Clone for Carousel<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Carousel<P> {}

impl<P: 'static> PartialEq for Carousel<P> {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}

impl<P: 'static> std::fmt::Debug for Carousel<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Carousel").finish_non_exhaustive()
    }
}

impl<P: 'static> Carousel<P> {
    /// A carousel on the snap `index` holds (the part's `v-model`), with
    /// the caller's `options` read when used, rotating as `rotation` says
    /// from the start, its keys turning in `direction`. Called in setup.
    #[must_use]
    #[track_caller]
    pub fn new(
        index: UseModel<P, usize>,
        options: impl Fn() -> CarouselOptions + 'static,
        rotation: Option<Rotation>,
        direction: impl Into<ReadDirection>,
    ) -> Self {
        let direction = direction.into();
        let me = Self {
            index,
            lap: Signal::labeled("carousel lap", 0),
            anchor: Signal::labeled("carousel anchor", 0.0),
            slides: Signal::new(Members::new()),
            links: gui::Keyed::new(),
            order: Signal::labeled("carousel slides", Vec::new()),
            heights: Signal::labeled("carousel slide heights", HashMap::new()),
            viewport: Signal::labeled("carousel viewport", 0.0),
            shift: Signal::labeled("carousel shift", 0.0),
            motion: Signal::labeled("carousel motion", Motion::Snap),
            dragging: Signal::labeled("carousel dragging", false),
            shown: Signal::labeled("carousel in view", Vec::new()),
            passing: Signal::labeled("carousel slides passing", Vec::new()),
            rotation: Signal::labeled("carousel rotation", rotation),
            playing: Signal::labeled("carousel playing", rotation.is_some()),
            due: Signal::labeled("carousel next step", None),
            hovered: Signal::labeled("carousel hovered", false),
            held: Signal::labeled("carousel held", false),
            bounds: Signal::labeled("carousel bounds", None),
            tabbed: Signal::labeled("carousel indicator groups", 0),
            roving: Roving::new(Keymap::new(Arrows::Both, Wrap::Around), direction),
            tick: Delay::new(),
            fling: Delay::new(),
            wheel_gap: Delay::new(),
            settle: Delay::new(),
            passed: Delay::new(),
            scroll: Delay::new(),
            state: Signal::new(State {
                options: Rc::new(options),
                direction,
                names: CarouselNames::default(),
                content: NodeRef::new(),
                focus: FocusHandle::new(),
                interaction: Interaction::new(),
                drag: WorldRefCell::new("a carousel's drag", None),
                wheel: WorldCell::new("a carousel's wheel gesture", (0.0, false)),
                settled: WorldCell::new("a carousel's settled snap", 0),
                on_settle: None,
                rest: WorldCell::new("whether a carousel's track rests", false),
                at: WorldCell::new("where a carousel's track was sent", 0.0),
                pending: WorldCell::new("a carousel's pending snap", None),
                scrolling: WorldCell::new("a carousel's auto-scroll", None),
                present: WorldCell::new("a carousel's slides laid out", HashSet::new()),
                prune_work: WorldCell::new("a carousel's prune work (tests)", 0),
            }),
        };
        let at = gui::untracked(|| me.index());
        me.state.with_untracked(|s| s.settled.set(at));
        // The slides in order, written once a pass when they changed: none
        // laid out (the last one gone) empties it. A slide no longer laid
        // out lets go of its height: on a change, through a set of those laid
        // out made once (linear work, in a buffer kept from pass to pass);
        // an unchanged pass does nothing (a height is measured only of a
        // slide laid out).
        me.slides.with_untracked(|l| {
            l.on_laid_out(move |now| {
                if !me.order.is_alive() || me.order.with_untracked(|o| o[..] == *now) {
                    return;
                }
                me.order.set(now.to_vec());
                if !me.heights.is_alive() || !me.state.is_alive() {
                    return;
                }
                let mut present = me.state.with_untracked(|s| s.present.take());
                present.clear();
                present.extend(now.iter().copied());
                let kept = me.heights.with_untracked(HashMap::len);
                let steps = u64::try_from(now.len().saturating_add(kept)).unwrap_or(u64::MAX);
                if me.heights.with_untracked(|h| h.keys().any(|k| !present.contains(k))) {
                    me.heights.update(|h| h.retain(|k, _| present.contains(k)));
                }
                if me.state.is_alive() {
                    me.state.with_untracked(|s| {
                        s.prune_work.set(s.prune_work.get().saturating_add(steps));
                        s.present.set(present);
                    });
                }
            });
        });
        // A change of snap (or of lap): the track goes to it (eased),
        // unless a release, a fling or an auto-scroll said otherwise, and
        // settles there once it has arrived.
        gui::watch(move || (me.index(), me.lap.get()), move |now, _| me.moved(now.0, now.1));
        // Which slides show, written only when that changes; focus inside
        // one that goes out of view is handed on.
        gui::Effect::new(move || {
            let shown = me.showing(true);
            let before = me.shown.get_untracked();
            if before != shown {
                gui::untracked(|| me.hand_focus_on(&before, &shown));
                me.shown.set(shown);
            }
        });
        // Reduced motion stops a playing rotation, now and when it changes.
        gui::Effect::new(move || {
            if gui::environment::reduced_motion() && me.rotation.get().is_some() && me.playing.get_untracked() {
                me.playing.set(false);
            }
        });
        // Keyboard focus coming into the carousel stops it (APG). Watched
        // from the first frame on: the root's flags exist once it is laid out.
        let interaction = me.state.with_untracked(|s| s.interaction);
        let owner = gui::Scope::new();
        gui::on_mounted(move || {
            owner.run(|| {
                gui::watch(
                    move || interaction.focus_within(),
                    move |within, before| {
                        if *within
                            && !*before
                            && gui::focus::modality() == gui::focus::Modality::Keyboard
                            && me.playing.get_untracked()
                        {
                            me.playing.set(false);
                        }
                    },
                );
            });
        });
        // A stepping rotation's next step, counted again from each change
        // of snap and whenever it starts running.
        gui::Effect::new(move || {
            let running = me.running();
            let _snap = me.index();
            match me.rotation.get().and_then(Rotation::delay) {
                Some(delay) if running => {
                    me.tick.after(delay, move || me.advance());
                    me.due.set(Some((gui::clock::current_time(), delay)));
                }
                _ => {
                    me.tick.cancel();
                    if me.due.get_untracked().is_some() {
                        me.due.set(None);
                    }
                }
            }
        });
        // An auto-scroll runs while the rotation does; stopped or paused
        // (not by a drag, which holds the track itself), the track goes to
        // the nearest snap.
        gui::Effect::new(move || {
            let scrolls = me.running() && me.rotation.get().is_some_and(|r| r.delay().is_none());
            let was = me.state.with_untracked(|s| s.scrolling.get().is_some());
            if scrolls && !was {
                let now = gui::clock::current_seconds() * 1000.0;
                me.state.with_untracked(|s| s.scrolling.set(Some(now)));
                me.scroll.after(FRAME, move || me.scroll_frame());
            } else if !scrolls && was {
                me.scroll.cancel();
                me.state.with_untracked(|s| s.scrolling.set(None));
                if !me.dragging.get_untracked() {
                    gui::untracked(|| me.rest_nearest());
                }
            }
        });
        me
    }

    /// Tell a screen reader the parts by `names` from now on.
    pub fn set_names(self, names: CarouselNames) {
        self.state.update(|s| s.names = names);
    }

    /// Call `f` with the snap shown each time the track has come to rest on
    /// a new one (reka's `@settle`).
    pub fn on_settle(self, f: impl Into<gui::Callback<usize>>) {
        let f = f.into();
        self.state.update(|s| s.on_settle = Some(f));
    }

    /// The caller's options now (tracked through what its closure reads).
    #[must_use]
    pub fn options(self) -> CarouselOptions {
        let f = self.state.with_untracked(|s| Rc::clone(&s.options));
        f()
    }

    fn rtl(self) -> bool {
        self.state.with_untracked(|s| s.direction.get()) == Direction::Rtl
    }

    /// How many slides there are, as last laid out (tracked).
    #[must_use]
    pub fn count(self) -> usize {
        self.order.with(Vec::len)
    }

    /// Whether the slides go round now (tracked): the caller asks for it,
    /// they do not fade, and the track is long enough ([`loop_fits`]).
    #[must_use]
    pub fn looping(self) -> bool {
        let o = self.options();
        o.wrap && !o.fade && loop_fits(self.count(), o.per_view, self.viewport.get())
    }

    /// How many snaps there are (tracked): one a slide when they fade or
    /// go round, else the trimmed track's.
    #[must_use]
    pub fn snaps(self) -> usize {
        let o = self.options();
        if o.fade || self.looping() {
            self.count().max(1)
        } else {
            snaps(self.count(), o.per_view)
        }
    }

    /// The snap shown (tracked): the model's, the last when past it.
    #[must_use]
    pub fn index(self) -> usize {
        self.index.get().min(self.snaps().saturating_sub(1))
    }

    /// The place of snap `v` among every lap, in viewports (tracked).
    fn place_of(self, v: i32) -> f32 {
        let o = self.options();
        if o.fade {
            0.0
        } else if self.looping() {
            real(v) * o.per_view
        } else {
            snap_position(usize::try_from(v).unwrap_or(0), self.count(), o.per_view)
        }
    }

    /// The snap shown as a place among every lap (tracked).
    fn virt(self) -> i32 {
        let i = signed(self.index());
        if self.looping() {
            i.saturating_add(self.lap.get().saturating_mul(signed(self.count())))
        } else {
            i
        }
    }

    /// Where the track stops for the snap shown, in viewports from its
    /// start, a going-round track's laps counted (tracked).
    #[must_use]
    pub fn position(self) -> f32 {
        self.place_of(self.virt())
    }

    /// What the look translates the track by along the axis (tracked):
    /// a fraction of the track's own length and pixels, mirrored right to
    /// left. Faded slides do not move it.
    #[must_use]
    pub fn offset(self) -> Calc {
        if self.options().fade {
            return Calc::px(0.0);
        }
        let sign = if self.options().orientation == Axis::Horizontal && self.rtl() {
            1.0
        } else {
            -1.0
        };
        Calc {
            fraction: sign * self.position(),
            px: sign * self.shift.get(),
            ch: 0.0,
        }
    }

    /// How the track gets to [`Self::offset`] (tracked).
    #[must_use]
    pub fn motion(self) -> Motion {
        if gui::environment::reduced_motion() {
            Motion::Follow
        } else {
            self.motion.get()
        }
    }

    /// How long the track takes to [`Self::offset`] (tracked): what the
    /// look's transition lasts. A faded slide's transition is the look's
    /// own, over [`CarouselOptions::duration`].
    #[must_use]
    pub fn move_duration(self) -> Duration {
        match self.motion() {
            Motion::Snap => self.options().duration,
            Motion::Bounce => BOUNCE,
            Motion::Follow => Duration::ZERO,
        }
    }

    /// Whether the pointer drags the track now (tracked).
    #[must_use]
    pub fn dragging(self) -> bool {
        self.dragging.get()
    }

    /// Whether Previous (or Next) can step (tracked).
    #[must_use]
    pub fn can_step(self, next: bool) -> bool {
        let (at, snaps, wrap) = (self.index(), self.snaps(), self.options().wrap);
        snaps > 1 && (wrap || if next { at.saturating_add(1) < snaps } else { at > 0 })
    }

    /// Whether a rotation is playing: not stopped by the user (tracked).
    #[must_use]
    pub fn playing(self) -> bool {
        self.rotation.get().is_some() && self.playing.get()
    }

    /// Whether a playing rotation is paused now (tracked): the mouse over
    /// it when asked, a finger on it, a drag, the window hidden, or the
    /// carousel out of the window.
    #[must_use]
    pub fn paused(self) -> bool {
        let hover = self.rotation.get().is_some_and(|r| r.pause_on_hover) && self.hovered.get();
        let offscreen = match (self.bounds.get(), gui::environment::window_size()) {
            (Some(b), Some((w, h))) => b.x + b.width <= 0.0 || b.y + b.height <= 0.0 || b.x >= w || b.y >= h,
            _ => false,
        };
        hover || self.held.get() || self.dragging.get() || !gui::environment::window_visible() || offscreen
    }

    fn running(self) -> bool {
        self.playing() && !self.paused()
    }

    /// Whether it is rotating now (tracked): its look and its live region
    /// read it.
    #[must_use]
    pub fn rotating(self) -> bool {
        self.running()
    }

    /// When the count to a stepping rotation's next step began (the
    /// clock's time, [`gui::clock::current_time`]) and how long it is,
    /// while it counts (tracked): what a look's progress to the next
    /// slide is drawn from, as a transition over the rest of it.
    #[must_use]
    pub fn next_step(self) -> Option<(Duration, Duration)> {
        self.due.get()
    }

    /// Go to snap `to`, within the snaps there are; a going-round track
    /// goes the shorter way round.
    pub fn go(self, to: usize) {
        let snaps = gui::untracked(|| self.snaps());
        self.travel(to.min(snaps.saturating_sub(1)), None, false);
    }

    /// One snap on (or back), wrapping when the caller asks for it: a
    /// going-round track keeps on round, one that does not runs back.
    pub fn step(self, next: bool) {
        let (at, snaps, wrap, looping, virt) = gui::untracked(|| {
            (
                self.index(),
                self.snaps(),
                self.options().wrap,
                self.looping(),
                self.virt(),
            )
        });
        if looping {
            let v = if next {
                virt.saturating_add(1)
            } else {
                virt.saturating_sub(1)
            };
            self.travel_virtual(v, false);
            return;
        }
        let to = match (next, wrap) {
            (true, true) if at.saturating_add(1) >= snaps => 0,
            (false, true) if at == 0 => snaps.saturating_sub(1),
            (true, _) => at.saturating_add(1),
            (false, _) => at.saturating_sub(1),
        };
        self.go(to);
    }

    /// Show snap `v` among every lap of a going-round track.
    fn travel_virtual(self, v: i32, quiet: bool) {
        let n = signed(gui::untracked(|| self.count())).max(1);
        let index = usize::try_from(v.rem_euclid(n)).unwrap_or(0);
        self.travel(index, Some(v.div_euclid(n)), quiet);
    }

    /// Show snap `index`, in `lap` of a going-round track (the shorter way
    /// round for none); `quiet`: on the way of a fling or an auto-scroll,
    /// settling nothing. That waits for the model to take the snap.
    fn travel(self, index: usize, lap: Option<i32>, quiet: bool) {
        self.state.with_untracked(|s| s.pending.set(Some((index, lap, quiet))));
        if gui::untracked(|| self.index()) == index {
            // The same snap in another lap (a fling round the track).
            if let Some(lap) = lap
                && self.lap.get_untracked() != lap
            {
                self.lap.set(lap);
            }
            return;
        }
        self.index.set(index);
    }

    /// The track was sent to snap `index` in `lap`: on a going-round track
    /// the lap is the move's (or the shorter way round), the slides are
    /// placed around the middle of the way, and a way longer than the
    /// slides cover goes at once; then it eases there, unless a release,
    /// a fling or an auto-scroll set how, and settles.
    fn moved(self, index: usize, lap: i32) {
        if !self.state.is_alive() {
            return;
        }
        let (looping, count, per_view) = gui::untracked(|| (self.looping(), self.count(), self.options().per_view));
        let n = signed(count).max(1);
        let from = self.state.with_untracked(|s| s.at.get());
        let asked = self.state.with_untracked(|s| s.pending.take()).filter(|p| p.0 == index);
        let quiet = asked.is_some_and(|p| p.2);
        // From outside, or the slides came to go round: the shorter way
        // from where the track is. No more going round: the first lap.
        let want = if looping {
            asked
                .and_then(|p| p.1)
                .unwrap_or_else(|| whole((from / per_view - real(signed(index))) / real(n)))
        } else {
            0
        };
        if want != lap {
            // The lap follows; the move is the next run's.
            self.state.with_untracked(|s| s.pending.set(asked));
            self.lap.set(want);
            return;
        }
        let rest = self.state.with_untracked(|s| s.rest.take());
        let to = gui::untracked(|| self.position());
        let viewport = self.viewport.get_untracked().max(1.0);
        // Where the track is now, a drag's shift counted.
        let now = from + self.shift.get_untracked() / viewport;
        let shift = if rest {
            self.shift.get_untracked() / viewport
        } else {
            0.0
        };
        if looping {
            let length = float(count) * per_view;
            let away = (to + shift - from).abs();
            if !rest && away + 1.0 > length {
                // Farther than the slides placed around the middle cover.
                self.motion.set(Motion::Follow);
                self.anchor.set(to);
            } else {
                self.anchor.set(f32::midpoint(from, to + shift));
            }
        }
        if !rest {
            self.shift.set(0.0);
            if !looping || (to - from).abs() + 1.0 <= float(count) * per_view {
                self.motion.set(Motion::Snap);
            }
        }
        self.state.with_untracked(|s| s.at.set(to + shift));
        if quiet {
            return;
        }
        let fade = gui::untracked(|| self.options().fade);
        // The track (or the fade) takes this long to get there: the slides
        // it passes show until then, as what is on screen is them.
        let arrive = gui::untracked(|| self.move_duration());
        self.pass_by(now, to + shift, fade, arrive);
        let wait = if fade { Duration::ZERO } else { arrive };
        self.settle.after(wait, move || self.settled(index));
    }

    /// Rotate as `rotation` says from now on, playing, or stop (none).
    pub fn set_rotation(self, rotation: Option<Rotation>) {
        self.rotation.set(rotation);
        self.playing.set(rotation.is_some());
    }

    /// Play a stopped rotation again, or stop it (the rotation button).
    pub fn set_playing(self, playing: bool) {
        self.playing.set(playing);
    }

    /// The user acted: the rotation stops when it should.
    fn interacted(self) {
        if self.rotation.get_untracked().is_some_and(|r| r.stop_on_interaction) {
            self.playing.set(false);
        }
    }

    /// A stepping rotation's step: the next snap, round a going-round
    /// track, the first after the last, or a stop there when asked.
    fn advance(self) {
        let (at, snaps, rotation, looping) =
            gui::untracked(|| (self.index(), self.snaps(), self.rotation.get(), self.looping()));
        let stop_on_last = rotation.is_some_and(|r| matches!(r.pace, Pace::Step { stop_on_last: true, .. }));
        if looping {
            self.step(true);
        } else if at.saturating_add(1) < snaps {
            self.go(at.saturating_add(1));
        } else if stop_on_last {
            self.playing.set(false);
        } else if at == 0 {
            // One snap only: nothing changes, so plan the next step here.
            if let Some(delay) = rotation.and_then(Rotation::delay) {
                self.tick.after(delay, move || self.advance());
            }
        } else {
            self.go(0);
        }
    }

    /// One frame of an auto-scroll: the track runs on by the time since
    /// the last, the snap shown follows it, and at an end of a track that
    /// does not go round it stops.
    fn scroll_frame(self) {
        if !self.state.is_alive() {
            return;
        }
        let now = gui::clock::current_seconds() * 1000.0;
        let Some(last) = self.state.with_untracked(|s| s.scrolling.replace(Some(now))) else {
            return;
        };
        let Some(Pace::Scroll { speed, backward }) = self.rotation.get_untracked().map(|r| r.pace) else {
            return;
        };
        let dt = (now - last).clamp(0.0, LONGEST_FRAME_MS);
        let step = speed * dt / 1000.0 * if backward { -1.0 } else { 1.0 };
        let (viewport, looping) = gui::untracked(|| (self.viewport.get(), self.looping()));
        if viewport <= 0.0 {
            self.scroll.after(FRAME, move || self.scroll_frame());
            return;
        }
        let (base, _, max) = self.place();
        let mut visible = base + self.shift.get_untracked() + step;
        let mut ended = false;
        if !looping {
            ended = (step > 0.0 && visible >= max - 0.5) || (step < 0.0 && visible <= 0.5);
            visible = visible.clamp(0.0, max);
        }
        let _ = self.follow(visible);
        if ended {
            // At the end: the rotation stops, and the track rests on its snap.
            self.playing.set(false);
            return;
        }
        self.scroll.after(FRAME, move || self.scroll_frame());
    }

    /// The track shows `visible` (px among every lap) at once: the snap
    /// shown is the nearest, a going-round track's first lap. Where that
    /// puts it, px among every lap.
    fn follow(self, visible: f32) -> f32 {
        let viewport = self.viewport.get_untracked().max(1.0);
        let (looping, count, per_view, virt, at) = gui::untracked(|| {
            (
                self.looping(),
                self.count(),
                self.options().per_view,
                self.virt(),
                self.index(),
            )
        });
        let (target, visible) = if looping {
            let v = whole(visible / viewport / per_view);
            let n = signed(count).max(1);
            // Back to the first lap, which moves nothing on screen.
            let laps = v.div_euclid(n);
            (
                v.rem_euclid(n),
                visible - real(laps) * float(count) * per_view * viewport,
            )
        } else {
            let k = nearest_snap(visible / viewport, count, per_view, Some(at), 0.5 / viewport);
            (signed(k), visible)
        };
        let place = gui::untracked(|| self.place_of(target)) * viewport;
        self.motion.set(Motion::Follow);
        self.shift.set(visible - place);
        self.state.with_untracked(|s| s.at.set(visible / viewport));
        if looping {
            self.anchor.set(visible / viewport);
        }
        if target != virt {
            self.state.with_untracked(|s| s.rest.set(true));
            self.travel(usize::try_from(target).unwrap_or(0), looping.then_some(0), true);
        }
        visible
    }

    /// The track goes to the snap nearest where it is (an auto-scroll stopped).
    fn rest_nearest(self) {
        let (base, ..) = self.place();
        let visible = base + self.shift.get_untracked();
        let _ = self.follow(visible);
        let at = self.index();
        self.motion.set(Motion::Snap);
        let from = self.position() + self.shift.get_untracked() / self.viewport.get_untracked().max(1.0);
        self.shift.set(0.0);
        let to = self.position();
        self.state.with_untracked(|s| s.at.set(to));
        let wait = self.move_duration();
        self.pass_by(from, to, false, wait);
        self.settle.after(wait, move || self.settled(at));
    }

    fn settled(self, index: usize) {
        if !self.state.is_alive() || self.state.with_untracked(|s| s.scrolling.get().is_some()) {
            return;
        }
        let (before, f) = self
            .state
            .with_untracked(|s| (s.settled.replace(index), s.on_settle.clone()));
        if before != index
            && let Some(f) = f
        {
            f.call(index);
        }
        // At rest on a going-round track: back to the first lap, which
        // moves nothing on screen (the track and the slides at once).
        let lap = self.lap.get_untracked();
        if lap != 0 && !self.dragging.get_untracked() && !self.fling.pending() {
            let (count, per_view) = gui::untracked(|| (self.count(), self.options().per_view));
            let lap_length = real(lap) * float(count) * per_view;
            self.state.with_untracked(|s| {
                s.rest.set(true);
                s.at.set(s.at.get() - lap_length);
                s.pending.set(Some((index, Some(0), true)));
            });
            self.motion.set(Motion::Follow);
            self.anchor.update(|a| *a -= lap_length);
            self.lap.set(0);
        } else if gui::untracked(|| self.looping()) {
            let to = gui::untracked(|| self.position());
            let shift = self.shift.get_untracked() / self.viewport.get_untracked().max(1.0);
            if (self.anchor.get_untracked() - (to + shift)).abs() > 1e-4 {
                self.anchor.set(to + shift);
            }
        }
    }

    /// The track goes from `from` to `to` (viewports), getting there in
    /// `arrive`: the slides it passes join those passing, each in view
    /// anywhere between (faded, the one shown as it starts, fading out),
    /// and sleep once it gets there.
    fn pass_by(self, from: f32, to: f32, fade: bool, arrive: Duration) {
        let (lo, hi) = (from.min(to), from.max(to));
        let passing = gui::untracked(|| {
            let o = self.options();
            let count = self.order.with(Vec::len);
            if fade {
                let at = self.state.with_untracked(|s| s.settled.get());
                return (0..count).map(|i| i == at).collect::<Vec<_>>();
            }
            let looping = self.looping();
            let anchor = if looping { self.anchor.get() } else { 0.0 };
            let slop = 0.5 / self.viewport.get().max(1.0);
            starts(count, o.per_view, looping, anchor)
                .iter()
                .map(|&start| (start + o.per_view).min(hi + 1.0) - start.max(lo) > slop)
                .collect()
        });
        let before = self.passing.get_untracked();
        let all: Vec<bool> = passing
            .iter()
            .enumerate()
            .map(|(i, &p)| p || before.get(i).copied().unwrap_or(false))
            .collect();
        if all != before {
            self.passing.set(all);
        }
        self.passed.after(arrive, move || {
            if self.passing.is_alive() {
                self.passing.set(Vec::new());
            }
        });
    }

    /// Which slides show (tracked): the one shown when they fade, else
    /// those the view holds any of (`moving`: where the track is now, a
    /// drag's shift counted; else at rest on the snap).
    fn showing(self, moving: bool) -> Vec<bool> {
        let mut shown = self.resting(moving);
        if moving {
            // And those the track passes on its way, until it gets there.
            self.passing.with(|p| {
                for (s, &p) in shown.iter_mut().zip(p) {
                    *s |= p;
                }
            });
        }
        shown
    }

    /// Which slides the view holds (tracked), the track at the snap
    /// (`moving`: a drag's shift counted).
    fn resting(self, moving: bool) -> Vec<bool> {
        let o = self.options();
        let count = self.order.with(Vec::len);
        if o.fade {
            let at = self.index();
            return (0..count).map(|i| i == at).collect();
        }
        let viewport = self.viewport.get();
        let shift = if moving && viewport > 0.0 {
            self.shift.get() / viewport
        } else {
            0.0
        };
        let looping = self.looping();
        let anchor = if looping { self.anchor.get() } else { 0.0 };
        let position = self.position() + shift;
        let slop = if viewport > 0.0 { 0.5 / viewport } else { 1e-3 };
        in_view(&starts(count, o.per_view, looping, anchor), o.per_view, position, slop)
    }

    /// Focus inside a slide that `shown` takes out of view goes to the
    /// first Tab stop of the slide coming in, else to the root, so it is
    /// not lost (`useCarouselViewport.ts:208-238`).
    fn hand_focus_on(self, before: &[bool], shown: &[bool]) {
        let order = self.order.get_untracked();
        let losing = order.iter().enumerate().any(|(i, k)| {
            before.get(i).copied().unwrap_or(false)
                && !shown.get(i).copied().unwrap_or(true)
                && k.meta.interaction.focus_within()
        });
        let Some(from) = gui::focus::focused().filter(|_| losing) else {
            return;
        };
        let rest = self.showing(false);
        let Some(incoming) = rest.iter().position(|&s| s).or_else(|| shown.iter().position(|&s| s)) else {
            return;
        };
        let into = if self.tabbed.get_untracked() > 0 {
            let snap = self.snap_of_slide(incoming);
            self.links.peek(&snap).map(|(_, content)| content)
        } else {
            None
        }
        .or_else(|| order.get(incoming).map(|k| k.meta.node));
        let root = self.state.with_untracked(|s| s.focus);
        if let Some(into) = into {
            gui::focus::hand_on(from, into, Some(root));
        }
    }

    /// The snap whose tab names slide `i` (tracked).
    fn snap_of_slide(self, i: usize) -> usize {
        let o = self.options();
        if o.fade || self.looping() {
            i
        } else {
            snap_of(i, self.count(), o.per_view)
        }
    }

    /// `d`, the look's box, as the root: a region described as a carousel,
    /// a Tab stop taking the keys; the mouse over it or a finger on it
    /// pauses a rotation, focus coming in by keyboard stops it. Sets
    /// `interaction`, `on_key_down`, `on_hover`, `on_touch` and
    /// `on_measured`.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        let (name, focus, interaction) = self
            .state
            .with_untracked(|s| (s.names.carousel.clone(), s.focus, s.interaction));
        d.role(gui::Role::Region)
            .aria_role_description(name)
            .track_focus(focus)
            .interaction(interaction)
            .on_key_down(move |ev| focus.is_focused() && self.key(ev))
            .on_hover(move |inside| self.hovered.set(inside))
            .on_touch(move |t| {
                let held = !matches!(
                    t.phase,
                    gui::events::TouchPhase::Ended | gui::events::TouchPhase::Cancelled
                );
                if self.held.get_untracked() != held {
                    self.held.set(held);
                }
            })
            .on_measured(move |b| {
                if self.bounds.is_alive() && self.bounds.get_untracked() != Some(*b) {
                    self.bounds.set(Some(*b));
                }
            })
    }

    fn key(self, ev: &KeyEvent) -> bool {
        let m = ev.modifiers;
        if m.control || m.alt || m.command {
            return false;
        }
        let orientation = gui::untracked(|| self.options()).orientation;
        let (back, forward) = match orientation {
            Axis::Horizontal if self.rtl() => (NamedKey::ArrowRight, NamedKey::ArrowLeft),
            Axis::Horizontal => (NamedKey::ArrowLeft, NamedKey::ArrowRight),
            Axis::Vertical => (NamedKey::ArrowUp, NamedKey::ArrowDown),
        };
        if ev.is(forward) {
            self.step(true);
        } else if ev.is(back) {
            self.step(false);
        } else if ev.is(NamedKey::Home) {
            self.go(0);
        } else if ev.is(NamedKey::End) {
            self.go(usize::MAX);
        } else {
            return false;
        }
        self.interacted();
        true
    }

    /// The viewport's height while the caller asks for it to follow the
    /// slides (tracked): the tallest of those at rest in view, as last
    /// laid out (reka's `autoHeight`, `useCarouselViewport.ts:247-259`);
    /// none for a vertical carousel, or before they are laid out.
    #[must_use]
    pub fn height(self) -> Option<f32> {
        let o = self.options();
        if !o.auto_height || o.orientation == Axis::Vertical {
            return None;
        }
        let shown = self.showing(false);
        let order = self.order.get();
        let tallest = self.heights.with(|h| {
            order
                .iter()
                .zip(&shown)
                .filter(|(_, s)| **s)
                .filter_map(|(k, _)| h.get(k).copied())
                .fold(0.0_f32, f32::max)
        });
        (tallest > 0.0).then_some(tallest)
    }

    /// `d` as the content, the viewport the track moves in: a live region,
    /// polite unless rotating, dragged by the pointer and paged by the
    /// wheel, as tall as the slides shown when the caller asks
    /// ([`Self::height`], bound to the node). Sets `on_measured`, `on_drag`
    /// and `on_wheel` (and `height` with [`CarouselOptions::auto_height`]).
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        let node = self.state.with_untracked(|s| s.content);
        let live = if self.rotating() { Live::Off } else { Live::Polite };
        let d = if self.options().auto_height {
            d.height(move || self.height().map_or(Length::Auto, Length::Px))
        } else {
            d
        };
        d.node_ref(node)
            .aria_live(live)
            .on_measured(move |b| {
                let length = match gui::untracked(|| self.options()).orientation {
                    Axis::Horizontal => b.width,
                    Axis::Vertical => b.height,
                };
                if self.viewport.is_alive() && self.viewport.get_untracked() != length {
                    self.viewport.set(length);
                }
            })
            .on_drag(move |e| self.drag(e))
            .on_wheel(move |w| self.wheel(w.dx, w.dy))
    }

    /// The track's place now at rest, px from its start among every lap,
    /// and how far it goes either way (no end on a going-round track).
    fn place(self) -> (f32, f32, f32) {
        gui::untracked(|| {
            let viewport = self.viewport.get();
            let rest = self.position() * viewport;
            if self.looping() {
                (rest, f32::NEG_INFINITY, f32::INFINITY)
            } else {
                (rest, 0.0, end(self.count(), self.options().per_view) * viewport)
            }
        })
    }

    fn drag(self, e: &DragEvent) {
        let options = gui::untracked(|| self.options());
        let horizontal = options.orientation == Axis::Horizontal;
        match e.phase {
            DragPhase::Start => {
                self.fling.cancel();
                self.state.with_untracked(|s| s.rest.set(false));
                if !options.draggable {
                    return;
                }
                let (base, ..) = self.place();
                let base = base + self.shift.get_untracked();
                let virt = gui::untracked(|| self.virt());
                let time = gui::clock::current_seconds() * 1000.0;
                self.state.with_untracked(|s| {
                    s.drag.replace(Some(Drag {
                        from: (e.x, e.y),
                        time,
                        base,
                        virt,
                        started: false,
                        aborted: false,
                        distance: 0.0,
                        samples: Vec::new(),
                        position: base,
                        travel: 0.0,
                    }));
                });
            }
            DragPhase::Move => self.drag_move(e, horizontal, options.fade),
            DragPhase::End => {
                let drag = self.state.with_untracked(|s| s.drag.with_mut(Option::take));
                let Some(drag) = drag.filter(|d| d.started && !d.aborted) else {
                    return;
                };
                self.dragging.set(false);
                if options.fade {
                    self.swipe(&drag);
                } else {
                    self.release(&drag, options);
                }
            }
            // Stopped without a release: back to the snap it was on.
            DragPhase::Cancel => {
                let drag = self.state.with_untracked(|s| s.drag.with_mut(Option::take));
                if drag.is_some_and(|d| d.started && !d.aborted) {
                    self.dragging.set(false);
                    let to = gui::untracked(|| self.position());
                    let now = to + self.shift.get_untracked() / self.viewport.get_untracked().max(1.0);
                    self.shift.set(0.0);
                    self.motion.set(Motion::Snap);
                    let wait = gui::untracked(|| self.move_duration());
                    self.pass_by(now, to, false, wait);
                }
            }
        }
    }

    fn drag_move(self, e: &DragEvent, horizontal: bool, fade: bool) {
        let (dx, dy) = self
            .state
            .with_untracked(|s| s.drag.with(|d| d.as_ref().map(|d| (e.x - d.from.0, e.y - d.from.1))))
            .unwrap_or_default();
        let (along, across) = if horizontal { (dx, dy) } else { (dy, dx) };
        // Pointer towards the start moves the track on; right to left the
        // start is the right.
        let travel = if horizontal && self.rtl() { along } else { -along };
        let now = gui::clock::current_seconds() * 1000.0;
        let mut began = false;
        // A drag starting now holds the track where an auto-scroll left it.
        let (here, ..) = self.place();
        let here = here + self.shift.get_untracked();
        let moved = self.state.with_untracked(|s| {
            s.drag.with_mut(|drag| {
                let d = drag.as_mut()?;
                if d.aborted {
                    return None;
                }
                if !d.started {
                    if along.abs() < DRAG_START {
                        return None;
                    }
                    // Mostly across the axis: the page's gesture.
                    if across.abs() > along.abs() {
                        d.aborted = true;
                        return None;
                    }
                    d.started = true;
                    d.base = here;
                    began = true;
                }
                d.distance = d.distance.max(along.abs());
                d.travel = travel;
                d.position = d.base + travel;
                d.samples.push((now, d.position));
                if d.samples.len() > 20 {
                    d.samples.remove(0);
                }
                Some((d.position, d.distance))
            })
        });
        let Some((position, distance)) = moved else {
            return;
        };
        if began {
            self.dragging.set(true);
            self.motion.set(Motion::Follow);
            self.interacted();
        }
        // Past the slop it is no click: nothing under it takes the release.
        if distance > CLICK_SLOP {
            gui::interact::claim_press();
        }
        if fade {
            // A faded carousel swipes: the slides do not follow.
            return;
        }
        let (rest, min, max) = self.place();
        let clamped = position.clamp(min, max);
        let shown = clamped + (position - clamped) * OVERSCROLL_RESISTANCE;
        self.shift.set(shown - rest);
        let viewport = self.viewport.get_untracked().max(1.0);
        self.state.with_untracked(|s| s.at.set(shown / viewport));
        if gui::untracked(|| self.looping()) {
            self.anchor.set(shown / viewport);
        }
    }

    /// A faded carousel's swipe: far or fast enough, the next or previous
    /// slide (`useCarouselViewport.ts:696-704`).
    fn swipe(self, drag: &Drag) {
        let elapsed = (gui::clock::current_seconds() * 1000.0 - drag.time).max(1.0);
        let d = drag.travel;
        if d.abs() >= SWIPE_DISTANCE || d.abs() / elapsed >= SWIPE_VELOCITY {
            self.step(d > 0.0);
        }
    }

    fn release(self, drag: &Drag, options: CarouselOptions) {
        let (rest, min, max) = self.place();
        let viewport = self.viewport.get_untracked();
        if viewport <= 0.0 {
            return;
        }
        let now = gui::clock::current_seconds() * 1000.0;
        let velocity = release_velocity(&drag.samples, now);
        let clamped = drag.position.clamp(min, max);
        let over = (drag.position - clamped).abs() > 0.5;
        let (count, per_view, looping) = gui::untracked(|| (self.count(), options.per_view, self.looping()));
        if options.drag_free {
            if over {
                self.motion.set(Motion::Bounce);
                self.shift.set(clamped - rest);
            }
            self.glide(clamped, velocity, (min, max));
            return;
        }
        let projected = (clamped + velocity * PROJECTION) / viewport;
        let mut target = if looping {
            whole(projected / per_view)
        } else {
            signed(nearest_snap(projected, count, per_view, None, 0.5 / viewport))
        };
        if !options.skip_snaps {
            // One gesture moves one snap at most.
            target = target.clamp(drag.virt.saturating_sub(1), drag.virt.saturating_add(1));
        }
        let motion = if over { Motion::Bounce } else { Motion::Snap };
        let virt = gui::untracked(|| self.virt());
        self.shift.set(0.0);
        self.motion.set(motion);
        self.state.with_untracked(|s| s.at.set(clamped / viewport));
        if target == virt {
            // Back to the same snap: it settles where it was, what the drag
            // showed of the next slide shown until the track has left it.
            let at = gui::untracked(|| self.index());
            let to = gui::untracked(|| self.position());
            if looping {
                self.anchor.set(f32::midpoint(clamped / viewport, to));
            }
            self.state.with_untracked(|s| s.at.set(to));
            let wait = gui::untracked(|| self.move_duration());
            self.pass_by(clamped / viewport, to, false, wait);
            self.settle.after(wait, move || self.settled(at));
        } else {
            // The change of snap keeps the motion set here.
            self.state.with_untracked(|s| s.rest.set(true));
            if looping {
                let to = real(target) * per_view;
                self.anchor.set(f32::midpoint(clamped / viewport, to));
                self.travel_virtual(target, false);
            } else {
                self.go(usize::try_from(target).unwrap_or(0));
            }
        }
    }

    /// A free fling from `position` (px) at `velocity` (px/ms), one frame
    /// at a time; at rest the nearest snap is shown, the track left where
    /// it stopped.
    fn glide(self, position: f32, velocity: f32, (min, max): (f32, f32)) {
        let velocity = velocity * DECAY.powf(FRAME_MS);
        let next = (position + velocity * FRAME_MS).clamp(min, max);
        let stuck = (next - position).abs() < 0.1;
        // On a going-round track it is now in the first lap.
        let next = self.follow(next);
        if velocity.abs() < 0.02 || stuck {
            let at = gui::untracked(|| self.index());
            self.settled(at);
            return;
        }
        self.fling.after(FRAME, move || self.glide(next, velocity, (min, max)));
    }

    /// A wheel turn of `(dx, dy)` over the viewport; whether it was taken.
    fn wheel(self, dx: f32, dy: f32) -> bool {
        let options = gui::untracked(|| self.options());
        let vertical = options.orientation == Axis::Vertical;
        let across = if vertical {
            dx.abs() > dy.abs()
        } else {
            dy.abs() > dx.abs()
        };
        // A faded carousel pages by any wheel when asked, by none else
        // (`useCarouselViewport.ts:772-784`).
        let pages = if options.fade || across {
            options.wheel
        } else {
            options.draggable
        };
        if !pages {
            return false;
        }
        let delta = if options.fade {
            match (dx.abs() > dy.abs(), self.rtl()) {
                (true, true) => -dx,
                (true, false) => dx,
                (false, _) => dy,
            }
        } else {
            match (vertical, across) {
                (false, false) if self.rtl() => -dx,
                (false, false) => dx,
                (true, false) | (false, true) => dy,
                (true, true) => dx,
            }
        };
        let state = self.state;
        self.wheel_gap.after(WHEEL_GAP, move || {
            if state.is_alive() {
                state.with_untracked(|s| s.wheel.set((0.0, false)));
            }
        });
        let forward = delta > 0.0;
        let (sum, locked) = self.state.with_untracked(|s| s.wheel.get());
        // At an end a new gesture is the page's.
        if !gui::untracked(|| self.can_step(forward)) && !locked {
            return false;
        }
        self.interacted();
        if locked {
            return true;
        }
        let sum = sum + delta;
        if sum.abs() < WHEEL_PAGE {
            self.state.with_untracked(|s| s.wheel.set((sum, false)));
            return true;
        }
        self.state.with_untracked(|s| s.wheel.set((0.0, true)));
        self.step(forward);
        true
    }

    /// A slide's key: called in its setup.
    #[must_use]
    pub fn slide_key(self) -> SlideKey {
        let meta = SlideMeta {
            interaction: Interaction::new(),
            node: NodeRef::new(),
        };
        SlideKey {
            id: gui::Registration::new(),
            meta,
        }
    }

    /// The slides and heights the passes letting go of heights stepped
    /// through so far: the measure of their work.
    #[must_use]
    pub fn prune_work(self) -> u64 {
        if !self.state.is_alive() {
            return 0;
        }
        self.state.with_untracked(|s| s.prune_work.get())
    }

    /// How many slides the carousel keeps anything of (their place, their
    /// height, their snap's nodes): those laid out.
    #[must_use]
    pub fn kept_slides(self) -> usize {
        let order = self.order.with_untracked(Vec::len);
        self.heights
            .with_untracked(HashMap::len)
            .max(order)
            .max(self.links.len())
    }

    /// Where slide `key` is among the slides, as last laid out (tracked).
    #[must_use]
    pub fn slide_index(self, key: SlideKey) -> Option<usize> {
        self.order.with(|o| o.iter().position(|k| *k == key))
    }

    /// Whether slide `key` shows any of itself (tracked).
    #[must_use]
    pub fn in_view(self, key: SlideKey) -> bool {
        self.slide_index(key)
            .is_none_or(|i| self.shown.with(|s| s.get(i).copied().unwrap_or(true)))
    }

    /// Whether slide `key` is the one a faded carousel shows (tracked).
    #[must_use]
    pub fn is_current(self, key: SlideKey) -> bool {
        self.slide_index(key) == Some(self.index())
    }

    /// Where slide `key` is moved from its place in the row (tracked), a
    /// fraction of its own length: onto the first slide when they fade,
    /// into its lap on a going-round track, else nowhere.
    fn slide_shift(self, key: SlideKey) -> Calc {
        let Some(i) = self.slide_index(key) else {
            return Calc::px(0.0);
        };
        let o = self.options();
        // Towards the end along the axis: the right, or the left right to left.
        let dir = if o.orientation == Axis::Horizontal && self.rtl() {
            -1.0
        } else {
            1.0
        };
        if o.fade {
            return Calc::fraction(-dir * float(i));
        }
        if !self.looping() {
            return Calc::px(0.0);
        }
        let lap = slide_lap(i, self.count(), o.per_view, self.anchor.get());
        Calc::fraction(dir * real(lap) * float(self.count()))
    }

    /// `d` as slide `key` (made by [`Self::slide_key`] in its setup): a
    /// group described as a slide and named "N of M", or a tab panel named
    /// by its snap's tab; out of view, hidden from assistive technology and
    /// inert. Faded, the slides lie on the first and the one shown is
    /// opaque, the others clear (the look eases the opacity); going round,
    /// each is moved into its lap; following the slides' height, each
    /// keeps its own. Those are bound to the node, so a move renders
    /// nothing. Sets `interaction`, `on_measured` (and `translate`,
    /// `opacity` and `align_self` as the options ask).
    #[must_use]
    pub fn slide(self, d: Div, key: SlideKey) -> Div {
        let (name, label, meta) = self
            .state
            .with_untracked(|s| (s.names.slide.clone(), Rc::clone(&s.names.slide_label), Some(key.meta)));
        let slides = self.slides.get_untracked();
        let index = self.slide_index(key);
        let count = self.count();
        let o = self.options();
        let d = match index {
            Some(i) if self.tabbed.get() > 0 => {
                let snap = self.snap_of_slide(i);
                self.link(snap).content(d.role(gui::Role::TabPanel), true)
            }
            Some(i) => d
                .role(gui::Role::Group)
                .aria_role_description(name)
                .aria_label(label(i.saturating_add(1), count)),
            None => d.role(gui::Role::Group).aria_role_description(name),
        };
        let d = match meta {
            Some(m) if self.tabbed.get() == 0 => d.node_ref(m.node).interaction(m.interaction),
            Some(m) => d.interaction(m.interaction),
            None => d,
        };
        // Out of view, it sleeps: still a slide to scroll to.
        let d = if self.in_view(key) { d } else { d.dormant(true) };
        let d = if o.fade || o.wrap {
            let shift = move || self.slide_shift(key);
            match o.orientation {
                Axis::Horizontal => d.translate_x(shift),
                Axis::Vertical => d.translate_y(shift),
            }
        } else {
            d
        };
        let d = if o.fade {
            d.opacity(move || if self.is_current(key) { 1.0 } else { 0.0 })
        } else {
            d
        };
        let d = if o.auto_height && o.orientation == Axis::Horizontal {
            d.align_self(Align::Start)
        } else {
            d
        };
        d.member(&slides, key).on_measured(move |b| {
            if self.heights.is_alive() && self.heights.with_untracked(|h| h.get(&key) != Some(&b.height)) {
                self.heights.update(|h| {
                    h.insert(key, b.height);
                });
            }
        })
    }

    /// The nodes of `snap`'s tab and panel, held while the calling render's
    /// output may be on screen.
    #[track_caller]
    fn link(self, snap: usize) -> Linked {
        let (trigger, content) = self.links.hold(&snap, || (NodeRef::new(), NodeRef::new()));
        Linked {
            trigger,
            content,
            points: Points::Controls,
        }
    }

    /// What Previous (or Next) is now (tracked): what its look dims by.
    #[must_use]
    pub fn step_availability(self, next: bool) -> Availability {
        if self.can_step(next) {
            Availability::Enabled
        } else {
            self.options().at_end
        }
    }

    /// `d`, the look's button, as Previous (or Next): named for it,
    /// controlling the content, it steps when it can. Sets `on_click`.
    #[must_use]
    pub fn step_button(self, d: Div, next: bool) -> Div {
        let (name, content) = self.state.with_untracked(|s| {
            let name = if next { &s.names.next } else { &s.names.previous };
            (name.clone(), s.content)
        });
        self.step_availability(next)
            .apply(d.role(gui::Role::Button).focusable().aria_label(name).controls(content))
            .on_click(move |_| {
                self.step(next);
                self.interacted();
            })
    }

    /// A group of indicators mounts: from now on the slides are tab panels.
    /// Called in the group's setup; its scope's end takes it back.
    pub fn join_indicators(self) {
        // Counted from its mount to its unmount (a setup, as a render,
        // writes only what it made): an unmount undoes only a mount that
        // counted.
        gui::while_mounted(move || {
            self.tabbed.update(|n| *n = n.saturating_add(1));
            move || {
                if self.tabbed.is_alive() {
                    self.tabbed.update(|n| *n = n.saturating_sub(1));
                }
            }
        });
    }

    /// `d` as the group of indicators (after [`Self::join_indicators`] in
    /// its setup): a tab list named for choosing a slide.
    #[must_use]
    pub fn indicators(self, d: Div) -> Div {
        let name = self.state.with_untracked(|s| s.names.indicators.clone());
        d.role(gui::Role::TabList).aria_label(name)
    }

    /// `d`, the look's dot, as the indicator of `snap`, focused through
    /// `focus` (its own, made in its setup). In a group: a tab of the
    /// group's roving focus, selected while its snap shows; alone: a
    /// button going to its snap, controlling the content (as the steps),
    /// unavailable while it shows (`useCarousel.ts:114-124`). Pressing it
    /// shows its snap. Sets `on_click` (and `on_key_down` in a group).
    #[must_use]
    pub fn indicator(self, d: Div, snap: usize, focus: FocusHandle) -> Div {
        let current = self.index() == snap;
        let d = if self.tabbed.get() > 0 {
            let name = self.state.with_untracked(|s| (s.names.tab)(snap.saturating_add(1)));
            let d = self
                .link(snap)
                .trigger(d.role(gui::Role::Tab).aria_selected(current), true);
            let item = Item::new(snap, focus).active(current);
            // Its own keys, not the group's plain roving (handlers add up).
            self.roving
                .member(d.aria_label(name), item)
                .on_key_down(move |ev| match self.roving.key(focus, ev) {
                    Nav::Moved(to) => {
                        self.go(to);
                        self.interacted();
                        true
                    }
                    nav => nav.consumed(),
                })
        } else {
            let (name, content) = self
                .state
                .with_untracked(|s| ((s.names.indicator)(snap.saturating_add(1)), s.content));
            let availability = if current {
                Availability::Unavailable
            } else {
                Availability::Enabled
            };
            availability.apply(
                d.role(gui::Role::Button)
                    .track_focus(focus)
                    .focusable()
                    .aria_label(name)
                    .controls(content),
            )
        };
        d.on_click(move |_| {
            self.go(snap);
            self.interacted();
        })
    }

    /// `d`, the look's button, as the rotation button: named for what it
    /// does now, controlling the content; it stops a playing rotation and
    /// plays a stopped one. `None` without a rotation. Sets `on_click`.
    #[must_use]
    pub fn rotation_button(self, d: Div) -> Option<Div> {
        self.rotation.get()?;
        let playing = self.playing();
        let (name, content) = self.state.with_untracked(|s| {
            let name = if playing {
                &s.names.stop_rotation
            } else {
                &s.names.start_rotation
            };
            (name.clone(), s.content)
        });
        Some(
            d.role(gui::Role::Button)
                .focusable()
                .aria_label(name)
                .controls(content)
                .on_click(move |_| self.playing.set(!self.playing.get_untracked())),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{in_view, loop_fits, nearest_snap, release_velocity, slide_lap, snap_of, snaps, starts};

    #[test]
    fn snaps_trim_the_trailing_space() {
        assert_eq!(snaps(5, 1.0), 5);
        assert_eq!(snaps(5, 1.0 / 3.0), 3, "three a view: 1-3, 2-4, 3-5");
        assert_eq!(snaps(5, 0.5), 4);
        assert_eq!(snaps(0, 1.0), 1);
        assert_eq!(snaps(2, 1.0 / 3.0), 1, "all in view");
    }

    #[test]
    fn nearest_snap_keeps_the_current_on_a_tie() {
        assert_eq!(nearest_snap(1.4, 5, 1.0, None, 0.0), 1);
        assert_eq!(nearest_snap(1.6, 5, 1.0, None, 0.0), 2);
        assert_eq!(nearest_snap(1.5, 5, 1.0, Some(2), 0.01), 2, "a tie keeps the preferred");
        assert_eq!(nearest_snap(1.5, 5, 1.0, Some(1), 0.01), 1);
        assert_eq!(nearest_snap(9.0, 5, 1.0, None, 0.0), 4);
    }

    #[test]
    fn slides_in_view_show_any_of_themselves() {
        let row = |n, w| starts(n, w, false, 0.0);
        assert_eq!(in_view(&row(3, 1.0), 1.0, 0.0, 1e-3), [true, false, false]);
        assert_eq!(in_view(&row(3, 1.0), 1.0, 0.5, 1e-3), [true, true, false]);
        assert_eq!(in_view(&row(4, 0.5), 0.5, 0.0, 1e-3), [true, true, false, false]);
    }

    #[test]
    fn a_track_goes_round_with_a_slide_spare_on_either_side() {
        assert!(loop_fits(3, 1.0, 300.0), "three a view each: one either side");
        assert!(!loop_fits(2, 1.0, 300.0));
        assert!(loop_fits(5, 1.0 / 3.0, 300.0), "five a third each: 2/3 spare");
        assert!(!loop_fits(4, 1.0 / 3.0, 300.0));
        assert!(!loop_fits(1, 1.0, 0.0));
        assert!(loop_fits(2, 1.0, 0.0), "not laid out: taken to fit");
    }

    #[test]
    fn a_going_round_slide_lies_within_half_a_track_of_the_view() {
        // Four a view each: the view at the last snap has the first after it.
        assert_eq!(slide_lap(0, 4, 1.0, 3.0), 1);
        assert_eq!(slide_lap(3, 4, 1.0, 3.0), 0);
        assert_eq!(slide_lap(3, 4, 1.0, 0.0), -1, "at the first, the last before it");
        assert_eq!(slide_lap(1, 4, 1.0, 0.0), 0);
        let placed = starts(4, 1.0, true, 3.5);
        assert_eq!(placed, [4.0, 5.0, 2.0, 3.0], "the last and the first side by side");
        assert_eq!(in_view(&placed, 1.0, 3.5, 1e-3), [true, false, false, true]);
    }

    #[test]
    fn a_slide_is_the_panel_of_the_first_snap_holding_it() {
        // Five a third each: snaps at 0, 1/3, 2/3.
        assert_eq!(snap_of(0, 5, 1.0 / 3.0), 0);
        assert_eq!(snap_of(2, 5, 1.0 / 3.0), 0);
        assert_eq!(snap_of(3, 5, 1.0 / 3.0), 1);
        assert_eq!(snap_of(4, 5, 1.0 / 3.0), 2);
    }

    #[test]
    fn release_velocity_reads_the_last_80_ms() {
        let samples = [(0.0, 0.0), (100.0, 10.0), (150.0, 30.0), (180.0, 60.0)];
        assert!((release_velocity(&samples, 180.0) - 50.0 / 80.0).abs() < 1e-4);
        assert!(release_velocity(&samples, 400.0).abs() < 1e-6, "nothing recent");
    }
}
