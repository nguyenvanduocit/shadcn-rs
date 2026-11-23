//! Motion, the CSS way. A property listed in a node's transition eases from
//! its old value to its new one ([`MotionStyle::transition`]); a node plays a
//! keyframe [`Animation`] from the frame it first has it
//! ([`MotionStyle::animation`]); and a node that leaves the tree stays
//! painted, inert, while its exit animation plays ([`MotionStyle::exit`],
//! reka's `Presence`).
//!
//! A node is the same node across frames when its [`NodePath`] is (keys keep
//! it through a reorder). [`Motion`] holds, per path, what runs; the layout
//! asks it for each node's values this frame ([`Motion::sample`]) and tells it
//! what the frame held ([`Motion::finish`]), which is when a node that was
//! there and is gone starts leaving. Time is the frame clock's.

use std::collections::HashMap;
use std::ops::BitOr;
use std::rc::Rc;
use std::sync::Arc;

use crate::clock;
use crate::color::Color;
use crate::element::Element;
use crate::geometry::{NodePath, PathSeg};
use crate::style::{Calc, Edges, Length, Ring, Shadow, Style};

/// A CSS timing function: how far along a value is at a share of its time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    Linear,
    /// `cubic-bezier(x1, y1, x2, y2)`.
    CubicBezier(f32, f32, f32, f32),
}

impl Easing {
    /// CSS `ease`, an animation's default timing.
    pub const EASE: Self = Self::CubicBezier(0.25, 0.1, 0.25, 1.0);
    /// CSS `ease-in`.
    pub const EASE_IN: Self = Self::CubicBezier(0.42, 0.0, 1.0, 1.0);
    /// CSS `ease-out`.
    pub const EASE_OUT: Self = Self::CubicBezier(0.0, 0.0, 0.58, 1.0);
    /// CSS `ease-in-out`.
    pub const EASE_IN_OUT: Self = Self::CubicBezier(0.42, 0.0, 0.58, 1.0);
    /// `cubic-bezier(0.4, 0, 1, 1)`, leaving fast (Material's accelerate
    /// curve; Tailwind's `--ease-in`).
    pub const ACCELERATE: Self = Self::CubicBezier(0.4, 0.0, 1.0, 1.0);
    /// `cubic-bezier(0, 0, 0.2, 1)`, arriving slow (Material's decelerate
    /// curve; Tailwind's `--ease-out`).
    pub const DECELERATE: Self = Self::CubicBezier(0.0, 0.0, 0.2, 1.0);
    /// `cubic-bezier(0.4, 0, 0.2, 1)` (Material's standard curve;
    /// Tailwind's `--ease-in-out`), a transition's default timing.
    pub const STANDARD: Self = Self::CubicBezier(0.4, 0.0, 0.2, 1.0);

    /// The output progress at input progress `x` (`0.0..=1.0`): the curve's
    /// `y` where its `x` is `x`.
    #[must_use]
    pub fn at(self, x: f32) -> f32 {
        let Self::CubicBezier(x1, y1, x2, y2) = self else {
            return x;
        };
        let bezier = |t: f32, p1: f32, p2: f32| {
            let u = 1.0 - t;
            3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
        };
        // x(t) rises monotonically on [0, 1], so bisection finds t.
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for _ in 0..24 {
            let mid = f32::midpoint(lo, hi);
            if bezier(mid, x1, x2) < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        bezier(f32::midpoint(lo, hi), y1, y2)
    }
}

/// The properties a transition eases (CSS `transition-property`), as
/// Tailwind's `transition-*` utilities list them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Properties(u16);

impl Properties {
    pub const NONE: Self = Self(0);
    /// `color`: the text colour (and the icons that take it).
    pub const COLOR: Self = Self(1);
    pub const BACKGROUND: Self = Self(2);
    pub const BORDER_COLOR: Self = Self(4);
    pub const OPACITY: Self = Self(8);
    /// `box-shadow`: the shadows and the ring Tailwind draws as one.
    pub const SHADOW: Self = Self(16);
    /// `transform`, `translate`, `scale`, `rotate`.
    pub const TRANSFORM: Self = Self(32);
    pub const WIDTH: Self = Self(64);
    pub const HEIGHT: Self = Self(128);
    pub const MARGIN: Self = Self(256);
    pub const PADDING: Self = Self(512);
    /// `top`, `right`, `bottom`, `left`.
    pub const INSET: Self = Self(1024);
    /// `transition-colors`.
    pub const COLORS: Self = Self(1 | 2 | 4);
    /// `transition`: colours, opacity, shadow and transform.
    pub const DEFAULT: Self = Self(1 | 2 | 4 | 8 | 16 | 32);
    /// `transition-all`.
    pub const ALL: Self = Self(u16::MAX);

    /// Whether every property of `other` is listed.
    #[must_use]
    pub const fn has(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Properties {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// How a node moves, as CSS's `transition` and `animation` properties and
/// Tailwind's `duration-*`, `ease-*` and `delay-*` say.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MotionStyle {
    /// What eases to a new value instead of jumping to it.
    pub transition: Properties,
    /// `duration-*`, in seconds: the transitions' and every themed animation's.
    pub duration: Option<f32>,
    /// `ease-*`: the transitions' and every themed animation's.
    pub easing: Option<Easing>,
    /// `delay-*`, in seconds, before a transition starts.
    pub delay: Option<f32>,
    /// What plays from the frame the node first has it, or has a new one.
    pub animation: Option<Rc<Animation>>,
    /// What plays when the node leaves the tree; it stays painted, inert,
    /// until that ends (`data-[state=closed]:animate-out` under reka's `Presence`).
    pub exit: Option<Rc<Animation>>,
    /// Told each [`Stage`] the node reaches, as it reaches it
    /// ([`crate::Div::on_stage`]).
    pub report: Option<StageReport>,
}

/// A point in a node's life its look's animations mark (reka's `Presence`
/// events `enter`, `after-enter`, `leave`, `after-leave`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    /// It is laid out for the first time (or again, back before its exit
    /// ended): its entry animation, if any, starts.
    Entering,
    /// Its entry animation ended, or it has none (an endless one never
    /// ends).
    Entered,
    /// It left the tree: its exit animation, if any, starts.
    Exiting,
    /// Its exit ended (or it had none, or its parent left first): it is
    /// gone from the screen.
    Exited,
}

/// What a node is told of its [`Stage`]s: run after the layout pass that
/// reached each, in order, every one of them, as the work of the owner
/// that built the node (none once it is disposed).
#[derive(Clone)]
pub struct StageReport(pub Rc<dyn Fn(Stage)>, pub Option<crate::component::Owner>);

/// The same closure is the same report.
impl PartialEq for StageReport {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for StageReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StageReport")
    }
}

impl MotionStyle {
    /// Whether it sets anything moving.
    fn moves(&self) -> bool {
        self.transition != Properties::NONE || self.animation.is_some() || self.exit.is_some() || self.report.is_some()
    }
}

/// A height a keyframe sets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Extent {
    Px(f32),
    /// The height the node takes without one (reka's
    /// `--reka-accordion-content-height`, measured once it has laid out).
    Natural,
}

/// The values one keyframe sets; what it leaves `None` follows the node's
/// own value.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Pose {
    pub opacity: Option<f32>,
    pub translate_x: Option<Calc>,
    pub translate_y: Option<Calc>,
    /// Pixels toward the layer's anchor (see [`crate::style::Transform::toward_anchor`]).
    pub toward_anchor: Option<f32>,
    pub scale: Option<f32>,
    /// Degrees.
    pub rotate: Option<f32>,
    pub height: Option<Extent>,
    /// `background-position-x`, a fraction of the room the background tile moves in.
    pub background_x: Option<f32>,
}

/// Where an [`Animation::enter`] slides in from, or an
/// [`Animation::exit`] slides out to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slide {
    /// Along the row: `Calc::px(-8.0)` comes in from 8 px to the left,
    /// `Calc::fraction(1.0)` leaves by its own width to the right.
    X(Calc),
    /// Down the column: `Calc::px(-8.0)` comes in from 8 px above.
    Y(Calc),
    /// This many pixels toward the anchor of its layer, on whichever side
    /// the layer opened ([`crate::Placement::side`]; reka's `data-side`
    /// with tw-animate's four `slide-in-from-*` at once).
    FromAnchor(f32),
}

/// A keyframe: a [`Pose`] at a share `at` (`0.0..=1.0`) of the animation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe {
    pub at: f32,
    pub pose: Pose,
}

/// A keyframe animation (`@keyframes` with its `animation` timing).
#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    keyframes: Vec<Keyframe>,
    /// One iteration, in seconds.
    duration: f32,
    easing: Easing,
    delay: f32,
    /// How many times it runs; `None` runs forever.
    iterations: Option<f32>,
    reverse: bool,
    /// The node's `duration-*` and `ease-*` win over the animation's own, as
    /// tw-animate's `var(--tw-duration, …)` and `var(--tw-ease, …)` let them.
    themed: bool,
    /// It plays only on a node that appears under a parent already shown.
    after_mount: bool,
}

impl Animation {
    /// `keyframes` once over `duration` seconds, each segment eased by `easing`.
    #[must_use]
    pub fn new(keyframes: Vec<Keyframe>, duration: f32, easing: Easing) -> Self {
        Self {
            keyframes,
            duration,
            easing,
            delay: 0.0,
            iterations: Some(1.0),
            reverse: false,
            themed: false,
            after_mount: false,
        }
    }

    /// One iteration takes `seconds`, unless the node's `duration-*` says
    /// otherwise for a themed one (`data-[state=open]:duration-500`).
    #[must_use]
    pub fn duration(self, seconds: f32) -> Self {
        Self {
            duration: seconds,
            ..self
        }
    }

    /// Run forever (`infinite`).
    #[must_use]
    pub fn infinite(self) -> Self {
        Self {
            iterations: None,
            ..self
        }
    }

    /// Run `n` iterations (`animation-iteration-count`); a fraction stops part
    /// way through the last one.
    #[must_use]
    pub fn iterations(self, n: f32) -> Self {
        Self {
            iterations: Some(n),
            ..self
        }
    }

    /// Run backwards (`animation-direction: reverse`).
    #[must_use]
    pub fn reverse(self) -> Self {
        Self { reverse: true, ..self }
    }

    /// Take the node's `duration-*` and `ease-*` over its own.
    #[must_use]
    pub fn themed(self) -> Self {
        Self { themed: true, ..self }
    }

    /// Play only on a node that appears under a parent already shown: one that
    /// mounts with its parent shows at once (reka's collapsible content, open
    /// as its item mounts, prevents its mount animation).
    #[must_use]
    pub fn after_mount(self) -> Self {
        Self {
            after_mount: true,
            ..self
        }
    }

    /// tw-animate's `animate-in`: from the pose its modifiers set to the
    /// node's own, over `--tw-duration` (150ms) with `--tw-ease` (`ease`).
    #[must_use]
    pub fn enter() -> Self {
        Self::new(
            vec![Keyframe {
                at: 0.0,
                pose: Pose::default(),
            }],
            0.15,
            Easing::EASE,
        )
        .themed()
    }

    /// tw-animate's `animate-out`: from the node's own pose to the one its
    /// modifiers set.
    #[must_use]
    pub fn exit() -> Self {
        Self::new(
            vec![Keyframe {
                at: 1.0,
                pose: Pose::default(),
            }],
            0.15,
            Easing::EASE,
        )
        .themed()
    }

    /// The pose of an [`Animation::enter`] or [`Animation::exit`] its modifiers edit.
    fn edge(mut self, f: impl FnOnce(&mut Pose)) -> Self {
        if let Some(k) = self.keyframes.first_mut() {
            f(&mut k.pose);
        }
        self
    }

    /// `fade-in-n`, `fade-out-n`: opacity `n / 100` at the edge (`fade-in` is `fade(0.0)`).
    #[must_use]
    pub fn fade(self, opacity: f32) -> Self {
        self.edge(|p| p.opacity = Some(opacity))
    }

    /// `zoom-in-n`, `zoom-out-n`: scale `n / 100` at the edge.
    #[must_use]
    pub fn zoom(self, scale: f32) -> Self {
        self.edge(|p| p.scale = Some(scale))
    }

    /// Where it slides from (`slide-in-from-*`) or to (`slide-out-to-*`)
    /// at the edge.
    #[must_use]
    pub fn slide(self, slide: Slide) -> Self {
        self.edge(|p| match slide {
            Slide::X(x) => p.translate_x = Some(x),
            Slide::Y(y) => p.translate_y = Some(y),
            Slide::FromAnchor(px) => p.toward_anchor = Some(px),
        })
    }

    /// `animate-accordion-down` (and `animate-collapsible-down`): height
    /// from 0 to the content's, 200ms `ease-out`.
    #[must_use]
    pub fn accordion_down() -> Self {
        let at = |at, h| Keyframe {
            at,
            pose: Pose {
                height: Some(h),
                ..Pose::default()
            },
        };
        Self::new(
            vec![at(0.0, Extent::Px(0.0)), at(1.0, Extent::Natural)],
            0.2,
            Easing::EASE_OUT,
        )
        .themed()
    }

    /// `animate-accordion-up` (and `animate-collapsible-up`): height from the
    /// content's to 0.
    #[must_use]
    pub fn accordion_up() -> Self {
        let at = |at, h| Keyframe {
            at,
            pose: Pose {
                height: Some(h),
                ..Pose::default()
            },
        };
        Self::new(
            vec![at(0.0, Extent::Natural), at(1.0, Extent::Px(0.0))],
            0.2,
            Easing::EASE_OUT,
        )
        .themed()
    }

    /// `animate-spin`: one linear turn a second, forever.
    #[must_use]
    pub fn spin() -> Self {
        Self::new(
            vec![Keyframe {
                at: 1.0,
                pose: Pose {
                    rotate: Some(360.0),
                    ..Pose::default()
                },
            }],
            1.0,
            Easing::Linear,
        )
        .infinite()
    }

    /// `animate-pulse`: opacity to 0.5 and back every two seconds,
    /// `cubic-bezier(0.4, 0, 0.6, 1)`.
    #[must_use]
    pub fn pulse() -> Self {
        Self::new(
            vec![Keyframe {
                at: 0.5,
                pose: Pose {
                    opacity: Some(0.5),
                    ..Pose::default()
                },
            }],
            2.0,
            Easing::CubicBezier(0.4, 0.0, 0.6, 1.0),
        )
        .infinite()
    }

    /// `animate-caret-blink`: shown at 0%, 70% and 100%, hidden at 20% and
    /// 50%, 1.25s `ease-out`, forever.
    #[must_use]
    pub fn caret_blink() -> Self {
        let at = |at, opacity| Keyframe {
            at,
            pose: Pose {
                opacity: Some(opacity),
                ..Pose::default()
            },
        };
        Self::new(
            vec![at(0.0, 1.0), at(0.2, 0.0), at(0.5, 0.0), at(0.7, 1.0), at(1.0, 1.0)],
            1.25,
            Easing::EASE_OUT,
        )
        .infinite()
    }

    /// How long and how it runs on a node styled `style`.
    fn timing(&self, style: &MotionStyle) -> (f32, Easing) {
        if self.themed {
            (
                style.duration.unwrap_or(self.duration),
                style.easing.unwrap_or(self.easing),
            )
        } else {
            (self.duration, self.easing)
        }
    }

    /// When it ends, `duration` seconds an iteration after `start`; never when infinite.
    fn end(&self, start: f32, duration: f32) -> Option<f32> {
        self.iterations.map(|n| start + self.delay + duration * n)
    }

    /// Whether it moves something only a UI frame paints: a height (which
    /// lays out again) or a background's position.
    fn paints(&self) -> bool {
        self.keyframes
            .iter()
            .any(|k| k.pose.height.is_some() || k.pose.background_x.is_some())
    }

    fn reads_natural_height(&self) -> bool {
        self.keyframes
            .iter()
            .any(|k| matches!(k.pose.height, Some(Extent::Natural)))
    }
}

/// Values that ease from one to another.
trait Mix: Clone + PartialEq {
    fn mix(&self, to: &Self, t: f32) -> Self;
}

impl Mix for f32 {
    fn mix(&self, to: &Self, t: f32) -> Self {
        self + (to - self) * t
    }
}

impl Mix for Calc {
    fn mix(&self, to: &Self, t: f32) -> Self {
        Self {
            fraction: self.fraction.mix(&to.fraction, t),
            px: self.px.mix(&to.px, t),
            ch: self.ch.mix(&to.ch, t),
        }
    }
}

impl Mix for Color {
    fn mix(&self, to: &Self, t: f32) -> Self {
        self.mix_oklab(*to, t)
    }
}

/// A background or border colour: none is transparent.
impl Mix for Option<Color> {
    fn mix(&self, to: &Self, t: f32) -> Self {
        if self.is_none() && to.is_none() {
            return None;
        }
        let (a, b) = (self.unwrap_or(Color::TRANSPARENT), to.unwrap_or(Color::TRANSPARENT));
        Some(a.mix(&b, t))
    }
}

/// Tailwind draws the ring as a box shadow, which eases from `0 0 #0000`.
impl Mix for Option<Ring> {
    fn mix(&self, to: &Self, t: f32) -> Self {
        let none = |r: &Ring| Ring {
            width: 0.0,
            offset: r.offset,
            color: Color::TRANSPARENT,
            corners: r.corners,
        };
        match (self, to) {
            (None, None) => None,
            (a, b) => {
                // A ring easing in or out keeps the offset and corners of
                // the one shown; between two, the corners switch halfway.
                let (a, b) = match (a, b) {
                    (Some(a), Some(b)) => (*a, *b),
                    (Some(a), None) => (*a, none(a)),
                    (None, Some(b)) => (none(b), *b),
                    (None, None) => return None,
                };
                Some(Ring {
                    width: a.width.mix(&b.width, t),
                    offset: a.offset.mix(&b.offset, t),
                    color: a.color.mix(&b.color, t),
                    corners: if t < 0.5 { a.corners } else { b.corners },
                })
            }
        }
    }
}

/// CSS pads the shorter shadow list with transparent zero shadows.
impl Mix for crate::style::Shadows {
    fn mix(&self, to: &Self, t: f32) -> Self {
        let none = Shadow {
            x: 0.0,
            y: 0.0,
            blur: 0.0,
            spread: 0.0,
            color: Color::TRANSPARENT,
        };
        (0..self.len().max(to.len()))
            .map(|i| {
                let (a, b) = (self.get(i).copied().unwrap_or(none), to.get(i).copied().unwrap_or(none));
                Shadow {
                    x: a.x.mix(&b.x, t),
                    y: a.y.mix(&b.y, t),
                    blur: a.blur.mix(&b.blur, t),
                    spread: a.spread.mix(&b.spread, t),
                    color: a.color.mix(&b.color, t),
                }
            })
            .collect()
    }
}

/// Lengths of one unit ease; `auto` against a length does not, and flips half way.
impl<T: Mix> Mix for Edges<T> {
    fn mix(&self, to: &Self, t: f32) -> Self {
        Self {
            top: self.top.mix(&to.top, t),
            right: self.right.mix(&to.right, t),
            bottom: self.bottom.mix(&to.bottom, t),
            left: self.left.mix(&to.left, t),
        }
    }
}

impl Mix for Length {
    fn mix(&self, to: &Self, t: f32) -> Self {
        match (self, to) {
            (Self::Px(a), Self::Px(b)) => Self::Px(a.mix(b, t)),
            (Self::Fraction(a), Self::Fraction(b)) => Self::Fraction(a.mix(b, t)),
            (Self::Calc { fraction: fa, px: pa }, Self::Calc { fraction: fb, px: pb }) => Self::Calc {
                fraction: fa.mix(fb, t),
                px: pa.mix(pb, t),
            },
            _ if t < 0.5 => *self,
            _ => *to,
        }
    }
}

/// When a transition runs and how.
#[derive(Clone, Copy)]
struct Timing {
    duration: f32,
    easing: Easing,
    delay: f32,
}

/// One running transition (css-transitions-1 §3).
#[derive(Clone, Debug, PartialEq)]
struct Run<T> {
    from: T,
    to: T,
    start: f32,
    duration: f32,
    easing: Easing,
    /// What a reversal back to it is measured against.
    reversing_from: T,
    reversing_factor: f32,
}

impl<T: Mix> Run<T> {
    fn output(&self, now: f32) -> f32 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        self.easing.at(((now - self.start) / self.duration).clamp(0.0, 1.0))
    }

    fn end(&self) -> f32 {
        self.start + self.duration
    }

    fn value(&self, now: f32) -> T {
        self.from.mix(&self.to, self.output(now))
    }
}

/// A transitioned property: the value it is going to, and the run there.
#[derive(Clone)]
struct Track<T> {
    target: T,
    /// Boxed: a node keeps its targets always, a run only while it eases.
    run: Option<Box<Run<T>>>,
}

impl<T: Clone> Track<T> {
    /// Its run, while one runs at `now`.
    fn live(&self, now: f32) -> Option<Run<T>> {
        self.run.as_deref().filter(|r| now < r.start + r.duration).cloned()
    }
}

/// What the frame being built still has moving.
#[derive(Default)]
struct Busy {
    /// When the last finite run ends.
    until: Option<f32>,
    /// A loop plays.
    looping: bool,
}

impl Busy {
    fn ends_at(&mut self, end: f32) {
        self.until = Some(self.until.map_or(end, |u| u.max(end)));
    }
}

/// The value of a transitioned property this frame: `after` when it has not
/// changed or does not transition (`timing` is `None`), else eased from what
/// shows now.
fn transition<T: Mix>(track: &mut Option<Track<T>>, after: T, timing: Option<Timing>, now: f32, busy: &mut Busy) -> T {
    let Some(track) = track else {
        // A node's first values show as they are, as CSS starts no
        // transition on an element's first style.
        *track = Some(Track {
            target: after.clone(),
            run: None,
        });
        return after;
    };
    if track.run.as_ref().is_some_and(|r| now >= r.end()) {
        track.run = None;
    }
    if after != track.target {
        let current = track
            .run
            .as_ref()
            .map_or_else(|| track.target.clone(), |r| r.value(now));
        track.run = match timing.filter(|t| t.duration + t.delay > 0.0) {
            _ if current == after => None,
            None => None,
            Some(t) => Some(Box::new(match &track.run {
                // Going back to where a run began: as much shorter as the
                // way back is (the spec's reversing shortening factor).
                Some(r) if r.reversing_from == after => {
                    let factor = (r.output(now) * r.reversing_factor + 1.0 - r.reversing_factor)
                        .abs()
                        .clamp(0.0, 1.0);
                    Run {
                        from: current,
                        to: after.clone(),
                        start: now + t.delay,
                        duration: t.duration * factor,
                        easing: t.easing,
                        reversing_from: r.to.clone(),
                        reversing_factor: factor,
                    }
                }
                _ => Run {
                    from: current.clone(),
                    to: after.clone(),
                    start: now + t.delay,
                    duration: t.duration,
                    easing: t.easing,
                    reversing_from: current,
                    reversing_factor: 1.0,
                },
            })),
        };
        track.target = after;
    }
    match &track.run {
        Some(r) => {
            busy.ends_at(r.end());
            r.value(now)
        }
        None => track.target.clone(),
    }
}

/// An animation playing on a node since `start`.
#[derive(Clone)]
struct Playing {
    animation: Rc<Animation>,
    start: f32,
    /// It is the exit of a node that left, which holds its last pose once it
    /// ends (`forwards`), as the node is gone by then.
    leaving: bool,
}

impl Playing {
    /// Whether it has ended by `now` and no longer sets anything: a finite
    /// animation past its last iteration (an exit holds its last pose).
    fn ended(&self, m: &MotionStyle, now: f32) -> bool {
        let (duration, _) = self.animation.timing(m);
        duration <= 0.0
            || (!self.leaving
                && self
                    .animation
                    .iterations
                    .is_some_and(|n| now - self.start - self.animation.delay >= duration * n))
    }
}

/// Everything [`Motion`] keeps about one node.
#[derive(Clone, Default)]
struct NodeState {
    background: Option<Track<Option<Color>>>,
    border: Option<Track<Option<Color>>>,
    color: Option<Track<Color>>,
    opacity: Option<Track<f32>>,
    ring: Option<Track<Option<Ring>>>,
    shadows: Option<Track<crate::style::Shadows>>,
    translate_x: Option<Track<Calc>>,
    translate_y: Option<Track<Calc>>,
    scale: Option<Track<f32>>,
    rotate: Option<Track<f32>>,
    width: Option<Track<Length>>,
    height: Option<Track<Length>>,
    margin: Option<Track<Edges<Length>>>,
    padding: Option<Track<Edges<f32>>>,
    inset: Option<Track<Edges<Length>>>,
    playing: Option<Playing>,
    /// Its exit animation and that animation's length, as last laid out.
    exit: Option<(Rc<Animation>, f32)>,
    /// Its height with no animation setting one, for keyframes at [`Extent::Natural`].
    natural: Option<f32>,
    /// Its box this pass is its natural height: record it.
    measure: bool,
    /// It wanted its natural height this pass and had none yet.
    wanted_natural: bool,
    /// Its [`Animation::after_mount`] animation started this pass: it stops
    /// unless its parent was shown the frame before.
    mounting: bool,
    /// What the compositor moves it by, while only its opacity and transform move.
    curves: Option<Arc<Curves>>,
    /// It moves (a transition, an animation, an exit, the compositor's
    /// curves, a height it waits for): it is sampled again next pass.
    busy: bool,
    /// Where it reports its stages, and the last it reported.
    report: Option<StageReport>,
    told: Option<Stage>,
}

impl NodeState {
    /// Whether a transition of something only a UI frame paints (a colour,
    /// a shadow, a size) runs at `now`.
    fn runs_paint(&self, now: f32) -> bool {
        fn on<T>(t: Option<&Track<T>>, now: f32) -> bool {
            t.is_some_and(|t| t.run.as_ref().is_some_and(|r| now < r.start + r.duration))
        }
        on(self.background.as_ref(), now)
            || on(self.border.as_ref(), now)
            || on(self.color.as_ref(), now)
            || on(self.ring.as_ref(), now)
            || on(self.shadows.as_ref(), now)
            || on(self.width.as_ref(), now)
            || on(self.height.as_ref(), now)
            || on(self.margin.as_ref(), now)
            || on(self.padding.as_ref(), now)
            || on(self.inset.as_ref(), now)
    }

    /// Whether a transition of its opacity or transform runs at `now`.
    fn runs_composited(&self, now: f32) -> bool {
        [&self.opacity, &self.scale, &self.rotate]
            .iter()
            .any(|t| t.as_ref().is_some_and(|t| t.live(now).is_some()))
            || [&self.translate_x, &self.translate_y]
                .iter()
                .any(|t| t.as_ref().is_some_and(|t| t.live(now).is_some()))
    }

    /// End its transitions and its finite animation.
    fn end_runs(&mut self) {
        fn end<T>(track: &mut Option<Track<T>>) {
            if let Some(t) = track {
                t.run = None;
            }
        }
        end(&mut self.background);
        end(&mut self.border);
        end(&mut self.color);
        end(&mut self.opacity);
        end(&mut self.ring);
        end(&mut self.shadows);
        end(&mut self.translate_x);
        end(&mut self.translate_y);
        end(&mut self.scale);
        end(&mut self.rotate);
        end(&mut self.width);
        end(&mut self.height);
        end(&mut self.margin);
        end(&mut self.padding);
        end(&mut self.inset);
        if let Some(p) = self.playing.as_mut().filter(|p| p.animation.iterations.is_some()) {
            p.start = f32::NEG_INFINITY;
        }
    }
}

/// A subtree that left the tree and plays its exit animations.
#[derive(Clone)]
struct Leaving {
    /// The path of its root, which is also its slot's in the parent.
    path: NodePath,
    /// The element in the parent's slot as it last rendered, made inert (a
    /// [`Element::Deferred`] when it is a layer).
    slot: Element,
    /// Where the slot sat among its siblings, for a keyed one.
    index: usize,
    since: f32,
    until: f32,
}

/// What runs, across frames, by node path; the pipeline owns one.
#[derive(Clone, Default)]
pub struct Motion {
    nodes: HashMap<NodePath, NodeState>,
    leaving: Vec<Leaving>,
    /// When the last finite motion of the last pass ends.
    settles_at: Option<f32>,
    reduced: bool,
    instant: bool,
    hold_loops: bool,
    /// When the earliest motion the compositor runs alone ends: the UI
    /// lays out once then, to show its end and drop it (ADR-0015).
    wake_at: Option<f32>,
    /// Nodes whose after-mount animation started this pass.
    mounting: Vec<NodePath>,
    /// Nodes with an exit that left the tree this pass.
    gone: Vec<NodePath>,
    /// Stages reached this pass, told once it is laid out.
    stages: Vec<(StageReport, Stage)>,
    /// A node waited for its natural height and has it now: lay out again.
    again: bool,
    /// Bumped when what runs changes for every node at once (reduced
    /// motion, instant, held loops): layout samples each node again.
    generation: u64,
}

impl Motion {
    /// `prefers-reduced-motion: reduce`: transitions and animations take no
    /// time, loops stand still, and a node leaves at once.
    pub fn set_reduced(&mut self, reduced: bool) {
        if self.reduced != reduced {
            self.generation = self.generation.wrapping_add(1);
        }
        self.reduced = reduced;
    }

    /// End every transition, finite animation and exit as it starts, as
    /// Web Animations' `finish()` would at once, while loops play on: a
    /// scripted still of each state an input leads to. Turned on, it ends
    /// what runs now too.
    pub fn set_instant(&mut self, instant: bool) {
        if self.instant != instant {
            self.generation = self.generation.wrapping_add(1);
        }
        self.instant = instant;
        if instant {
            self.finish_all();
        }
    }

    /// End every transition, finite animation and exit running now.
    fn finish_all(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let left: Vec<NodePath> = self.leaving.drain(..).map(|l| l.path).collect();
        self.nodes
            .retain(|path, _| !left.iter().any(|l| path.0.starts_with(&l.0)));
        for state in self.nodes.values_mut() {
            state.end_runs();
        }
    }

    /// Hold every loop at its first frame, as a browser's paused animation
    /// at `currentTime` 0 shows (a still of the page).
    pub fn hold_loops(&mut self, hold: bool) {
        if self.hold_loops != hold {
            self.generation = self.generation.wrapping_add(1);
        }
        self.hold_loops = hold;
    }

    /// What changes for every node at once (reduced motion, instant, held
    /// loops) changed this many times: layout samples each node again when
    /// it differs from what it saw.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// The nodes it keeps a state for.
    pub(crate) fn paths(&self) -> impl Iterator<Item = &NodePath> {
        self.nodes.keys()
    }

    /// Whether the node at `path` moves: it is sampled again next pass.
    pub(crate) fn busy(&self, path: &NodePath) -> bool {
        self.nodes.get(path).is_some_and(|s| s.busy) || self.leaving_since(path).is_some()
    }

    /// The node at `path`, laid out, is `height` tall: its natural height
    /// when it measures one.
    pub(crate) fn natural(&mut self, path: &NodePath, height: f32) {
        if let Some(s) = self.nodes.get_mut(path)
            && s.measure
        {
            self.again |= s.wanted_natural && s.natural.is_none();
            s.natural = Some(height);
        }
    }

    /// The nodes at `released` left the tree this pass (`present` says
    /// which paths it still holds): those with an exit start leaving at
    /// [`Motion::finish`], the others are forgotten.
    pub(crate) fn left(&mut self, released: &[NodePath], present: &dyn Fn(&NodePath) -> bool) {
        for path in released {
            if present(path) {
                continue;
            }
            let leaving = self.leaving.iter().any(|l| path.0.starts_with(&l.path.0));
            match self.nodes.get(path) {
                Some(s) if s.exit.is_some() && !leaving => self.gone.push(path.clone()),
                Some(_) if !leaving => {
                    if let Some(r) = self.nodes.remove(path).and_then(|s| s.report) {
                        self.stages.push((r.clone(), Stage::Exiting));
                        self.stages.push((r, Stage::Exited));
                    }
                }
                _ => {}
            }
        }
    }

    /// When the last transition, finite animation or exit that ran in the
    /// last layout ends, if one runs.
    #[must_use]
    pub fn settles_at(&self) -> Option<f32> {
        self.settles_at
    }

    /// When a motion the compositor runs alone ends and the UI must lay
    /// out again, if one runs (ADR-0015: nothing else asks for UI frames).
    #[must_use]
    pub fn wake_at(&self) -> Option<f32> {
        self.wake_at
    }

    /// Start a layout pass.
    pub(crate) fn begin(&mut self) {
        self.settles_at = None;
        self.wake_at = None;
    }

    fn wake_by(&mut self, at: f32) {
        self.wake_at = Some(self.wake_at.map_or(at, |w| w.min(at)));
    }

    /// When the subtree holding `path` started leaving.
    fn leaving_since(&self, path: &NodePath) -> Option<f32> {
        self.leaving
            .iter()
            .find(|l| path.0.starts_with(&l.path.0))
            .map(|l| l.since)
    }

    /// Set what moves in `style`, the resolved style of the node at `path`,
    /// to its value this frame; whether its translation is moving (a
    /// transition or an animation set it apart from its own value), which
    /// paint then applies as a transform rather than in the node's place
    /// (ADR-0016: animated values beside layout values).
    pub(crate) fn sample(&mut self, path: &NodePath, style: &mut Style) -> Sampled {
        let moves = style.motion.as_deref().is_some_and(MotionStyle::moves);
        if !moves && (self.nodes.is_empty() || !self.nodes.contains_key(path)) {
            return Sampled::default();
        }
        let rest = (style.transform.translate, style.transform.toward_anchor);
        let m = style.motion.take().unwrap_or_default();
        let leaving = self.leaving_since(path);
        let now = clock::frame_time();
        let (reduced, hold) = (self.reduced, self.hold_loops);
        let finite_still = reduced || self.instant;
        let state = self.nodes.entry(path.clone()).or_default();
        let mut busy = Busy::default();
        transitions(state, style, &m, finite_still, now, &mut busy);

        state.exit = m.exit.as_ref().map(|a| {
            let (duration, _) = a.timing(&m);
            (a.clone(), duration)
        });
        let natural_height = style.height == Length::Auto;
        state.measure = natural_height;
        state.wanted_natural = false;
        let playing = match (leaving, &m.animation) {
            (Some(since), _) => m.exit.clone().map(|animation| Playing {
                animation,
                start: since,
                leaving: true,
            }),
            (None, Some(a)) => {
                if state.playing.as_ref().is_none_or(|p| &p.animation != a) {
                    state.mounting = a.after_mount;
                    if a.after_mount {
                        self.mounting.push(path.clone());
                    }
                    // One that may not run has ended as it starts, so it
                    // does not run later either (a scenario's motion played
                    // after a still).
                    let ended = finite_still && a.iterations.is_some();
                    state.playing = Some(Playing {
                        animation: a.clone(),
                        start: if ended { f32::NEG_INFINITY } else { now },
                        leaving: false,
                    });
                }
                state.playing.clone()
            }
            (None, None) => {
                state.playing = None;
                None
            }
        };
        let still = |a: &Animation| reduced || (finite_still && a.iterations.is_some());
        let playing = playing.filter(|p| !still(&p.animation));
        state.report.clone_from(&m.report);
        if leaving.is_none()
            && let Some(r) = &m.report
        {
            if state.told.is_none() {
                self.stages.push((r.clone(), Stage::Entering));
                state.told = Some(Stage::Entering);
            }
            if state.told == Some(Stage::Entering) && playing.as_ref().is_none_or(|p| p.ended(&m, now)) {
                self.stages.push((r.clone(), Stage::Entered));
                state.told = Some(Stage::Entered);
            }
        }
        // Where the transitions go: what keyframes fill open ends with once
        // they end, and what shows when nothing runs.
        let mut own = Own::of(style);
        let target = |t: &Option<Track<f32>>, v: &mut f32| {
            if let Some(t) = t {
                *v = t.target;
            }
        };
        target(&state.opacity, &mut own.opacity);
        target(&state.scale, &mut own.scale);
        target(&state.rotate, &mut own.rotate);
        for (t, v) in [&state.translate_x, &state.translate_y]
            .into_iter()
            .zip(&mut own.translate)
        {
            if let Some(t) = t {
                *v = t.target;
            }
        }
        if let Some(p) = &playing {
            let set_height = animate(state, style, &m, p, hold, now, &mut busy);
            state.measure = natural_height && !set_height;
        }

        // ADR-0015: a node whose opacity and transform alone move is moved by
        // the compositor at its own frame times; the UI lays it out once more
        // when it ends. Anything else that moves needs a UI frame each frame.
        let main = state.runs_paint(now) || playing.as_ref().is_some_and(|p| p.animation.paints());
        let composited = !main && (state.runs_composited(now) || playing.as_ref().is_some_and(|p| !p.ended(&m, now)));
        let curves = composited.then(|| {
            let curves = Curves {
                own,
                opacity: state.opacity.as_ref().and_then(|t| t.live(now)),
                translate_x: state.translate_x.as_ref().and_then(|t| t.live(now)),
                translate_y: state.translate_y.as_ref().and_then(|t| t.live(now)),
                scale: state.scale.as_ref().and_then(|t| t.live(now)),
                rotate: state.rotate.as_ref().and_then(|t| t.live(now)),
                playing: playing.as_ref().map(|p| {
                    let (duration, easing) = p.animation.timing(&m);
                    CurvePlay {
                        animation: Arc::new((*p.animation).clone()),
                        start: p.start,
                        leaving: p.leaving,
                        duration,
                        easing,
                        hold,
                    }
                }),
            };
            match &state.curves {
                Some(kept) if **kept == curves => Arc::clone(kept),
                _ => Arc::new(curves),
            }
        });
        state.curves.clone_from(&curves);

        if let Some(until) = busy.until.filter(|&u| u > now) {
            self.settles_at = Some(self.settles_at.map_or(until, |s| s.max(until)));
        }
        let moving = busy.looping || busy.until.is_some_and(|u| u > now) || leaving.is_some();
        state.busy = moving || curves.is_some() || state.mounting || state.wanted_natural;
        let measures = state.measure;
        if composited {
            if let Some(until) = busy.until.filter(|&u| u > now) {
                self.wake_by(until);
            }
            if leaving.is_some()
                && let Some(l) = self.leaving.iter().find(|l| path.0.starts_with(&l.path.0))
            {
                self.wake_by(l.until);
            }
        } else if moving {
            clock::request_frame();
        }
        style.motion = Some(m);
        Sampled {
            translating: (style.transform.translate, style.transform.toward_anchor) != rest || curves.is_some(),
            curves,
            kept: true,
            measures,
        }
    }

    /// Take in the frame a pass laid out; `previous` is the tree the pass
    /// before it laid out. Returns whether the frame must be laid out again:
    /// a node that left starts leaving and must show, or a node waited for
    /// its natural height.
    pub(crate) fn finish(&mut self, previous: Option<&Element>, present: &dyn Fn(&NodePath) -> bool) -> bool {
        let mut again = std::mem::take(&mut self.again);

        for path in std::mem::take(&mut self.mounting) {
            let Some(s) = self.nodes.get_mut(&path) else { continue };
            if !s.mounting {
                continue;
            }
            s.mounting = false;
            let parent = path.0.split_last().map(|(_, p)| p);
            if !parent.is_some_and(|p| previous.is_some_and(|r| holds(r, p))) {
                if let Some(p) = &mut s.playing {
                    p.start = f32::NEG_INFINITY;
                }
                again = true;
            }
        }

        let gone: Vec<NodePath> = std::mem::take(&mut self.gone)
            .into_iter()
            .filter(|path| self.leaving_since(path).is_none())
            .collect();
        if !gone.is_empty() && !self.reduced && !self.instant {
            let now = clock::frame_time();
            for path in &gone {
                // The topmost node that left with it is the one to keep.
                let mut len = path.0.len();
                while len > 1
                    && path
                        .0
                        .get(..len.saturating_sub(1))
                        .is_some_and(|parent| !present(&NodePath::new(parent.into())))
                {
                    len = len.saturating_sub(1);
                }
                let root = NodePath::new(path.0.get(..len).map(Into::into).unwrap_or_default());
                if self.leaving.iter().any(|l| root.0.starts_with(&l.path.0)) {
                    continue;
                }
                let until = gone
                    .iter()
                    .filter(|p| p.0.starts_with(&root.0))
                    .filter_map(|p| self.nodes.get(p)?.exit.as_ref())
                    .filter_map(|(a, duration)| a.end(now, *duration))
                    .fold(now, f32::max);
                if let Some((mut slot, index)) = previous.and_then(|p| slot_at(p, &root)) {
                    for p in gone.iter().filter(|p| p.0.starts_with(&root.0)) {
                        if let Some(r) = self.nodes.get(p).and_then(|s| s.report.clone()) {
                            self.stages.push((r, Stage::Exiting));
                        }
                    }
                    inert(&mut slot);
                    self.leaving.push(Leaving {
                        path: root,
                        slot,
                        index,
                        since: now,
                        until,
                    });
                    again = true;
                }
            }
        }
        // What left and does not leave with an exit is forgotten.
        for path in &gone {
            if self.leaving_since(path).is_none()
                && let Some(r) = self.nodes.remove(path).and_then(|s| s.report)
            {
                self.stages.push((r.clone(), Stage::Exiting));
                self.stages.push((r, Stage::Exited));
            }
        }
        // Each told as the work of its node's owner, in order.
        for (StageReport(f, owner), stage) in self.stages.drain(..) {
            let tell = move || f(stage);
            match owner {
                Some(owner) => owner.after_layout(tell),
                None => crate::component::after_layout(tell),
            }
        }
        again
    }

    /// `root` with every subtree still leaving put back in its slot. A
    /// subtree whose exit has ended, whose parent is gone, or whose slot is
    /// taken again (it came back) is let go.
    pub(crate) fn splice(&mut self, mut root: Element) -> Element {
        let now = clock::frame_time();
        let mut ended = Vec::new();
        for l in std::mem::take(&mut self.leaving) {
            // Its slot holds a node again: it came back before its exit
            // ended, and enters anew rather than ending.
            let mut back = false;
            let kept = now < l.until
                && l.path.0.split_last().is_some_and(|(last, parent)| {
                    siblings_at(&mut root, parent).is_some_and(|kids| {
                        let put = put_back(kids, *last, l.index, &l.slot);
                        back = !put;
                        put
                    })
                });
            if kept {
                self.leaving.push(l);
            } else {
                if !back {
                    // In tree order, outer first, whatever the map's order.
                    let mut told: Vec<(&NodePath, &StageReport)> = self
                        .nodes
                        .iter()
                        .filter(|(path, _)| path.0.starts_with(&l.path.0))
                        .filter_map(|(path, s)| Some((path, s.report.as_ref()?)))
                        .collect();
                    told.sort_by_cached_key(|(path, _)| (path.0.len(), format!("{:?}", path.0)));
                    self.stages
                        .extend(told.into_iter().map(|(_, r)| (r.clone(), Stage::Exited)));
                }
                ended.push(l.path);
            }
        }
        // What left is forgotten; back again, it enters anew.
        self.nodes
            .retain(|path, _| !ended.iter().any(|e| path.0.starts_with(&e.0)));
        root
    }
}

/// Ease each property `style`'s transition lists; when `still`, none.
fn transitions(state: &mut NodeState, style: &mut Style, m: &MotionStyle, still: bool, now: f32, busy: &mut Busy) {
    let timing = Timing {
        duration: m.duration.unwrap_or(0.15),
        easing: m.easing.unwrap_or(Easing::STANDARD),
        delay: m.delay.unwrap_or(0.0),
    };
    let listed = m.transition;
    let on = |p: Properties| (!still && listed.has(p)).then_some(timing);
    style.background = transition(
        &mut state.background,
        style.background,
        on(Properties::BACKGROUND),
        now,
        busy,
    );
    style.border_color = transition(
        &mut state.border,
        style.border_color,
        on(Properties::BORDER_COLOR),
        now,
        busy,
    );
    // A colour provided above is its provider's to ease.
    match style.content.color {
        Some(c) => style.content.color = Some(transition(&mut state.color, c, on(Properties::COLOR), now, busy)),
        None => state.color = None,
    }
    style.opacity = transition(&mut state.opacity, style.opacity, on(Properties::OPACITY), now, busy);
    style.ring = transition(&mut state.ring, style.ring, on(Properties::SHADOW), now, busy);
    style.shadows = transition(
        &mut state.shadows,
        std::mem::take(&mut style.shadows),
        on(Properties::SHADOW),
        now,
        busy,
    );
    let t = &mut style.transform;
    let [x, y] = t.translate;
    t.translate = [
        transition(&mut state.translate_x, x, on(Properties::TRANSFORM), now, busy),
        transition(&mut state.translate_y, y, on(Properties::TRANSFORM), now, busy),
    ];
    t.scale = transition(&mut state.scale, t.scale, on(Properties::TRANSFORM), now, busy);
    t.rotate = transition(&mut state.rotate, t.rotate, on(Properties::TRANSFORM), now, busy);
    style.width = transition(&mut state.width, style.width, on(Properties::WIDTH), now, busy);
    style.height = transition(&mut state.height, style.height, on(Properties::HEIGHT), now, busy);
    style.margin = transition(&mut state.margin, style.margin, on(Properties::MARGIN), now, busy);
    style.padding = transition(&mut state.padding, style.padding, on(Properties::PADDING), now, busy);
    style.inset = transition(&mut state.inset, style.inset, on(Properties::INSET), now, busy);
}

/// Set what `p` animates in `style`, moving as `m` says, to its value now;
/// returns whether it set the height.
fn animate(
    state: &mut NodeState,
    style: &mut Style,
    m: &MotionStyle,
    p: &Playing,
    hold: bool,
    now: f32,
    busy: &mut Busy,
) -> bool {
    let a = &p.animation;
    let (duration, easing) = a.timing(m);
    let Some(progress) = progress(a, p.start, p.leaving, duration, hold, now, Some(busy)) else {
        return false;
    };
    let frames = &a.keyframes;
    let mut own = Own::of(style);
    own.keyframed(frames, progress, easing);
    own.set(style);
    if let Some(image) = &mut style.effects.background_image {
        let [x, _] = &mut image.position;
        if let Some(v) = keyframed(frames, |k| k.background_x.map(Calc::fraction), *x, progress, easing) {
            *x = v;
        }
    }
    if !frames.iter().any(|k| k.pose.height.is_some()) {
        return false;
    }
    let natural = match style.height {
        Length::Px(h) => Some(h),
        _ => state.natural,
    };
    let Some(natural) = natural.or_else(|| (!a.reads_natural_height()).then_some(0.0)) else {
        state.wanted_natural = true;
        return false;
    };
    let px = |e: Extent| match e {
        Extent::Px(h) => h,
        Extent::Natural => natural,
    };
    match keyframed(frames, |k| k.height.map(px), natural, progress, easing) {
        Some(h) => {
            style.height = Length::Px(h);
            true
        }
        None => false,
    }
}

/// How far through its iteration animation `a`, playing since `start` for
/// `duration` seconds an iteration, is at `now`, direction applied; `None`
/// before it starts and after a finite one ends (an exit, `leaving`, holds
/// its end). What still runs is noted in `busy`.
fn progress(
    a: &Animation,
    start: f32,
    leaving: bool,
    duration: f32,
    hold: bool,
    now: f32,
    busy: Option<&mut Busy>,
) -> Option<f32> {
    let elapsed = now - start - a.delay;
    if elapsed < 0.0 || duration <= 0.0 {
        return None;
    }
    let progress = match a.iterations {
        Some(n) if elapsed >= duration * n => {
            if !leaving {
                return None;
            }
            // The end of the last iteration.
            let last = n.fract();
            if last == 0.0 { 1.0 } else { last }
        }
        Some(n) => {
            if let Some(b) = busy {
                b.ends_at(start + a.delay + duration * n);
            }
            (elapsed / duration).fract()
        }
        None if hold => 0.0,
        None => {
            if let Some(b) = busy {
                b.looping = true;
            }
            (elapsed / duration).fract()
        }
    };
    Some(if a.reverse { 1.0 - progress } else { progress })
}

/// The values the compositor moves: opacity and the transform's parts.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Own {
    opacity: f32,
    translate: [Calc; 2],
    toward_anchor: f32,
    scale: f32,
    rotate: f32,
}

impl Own {
    fn of(style: &Style) -> Self {
        let t = &style.transform;
        Self {
            opacity: style.opacity,
            translate: t.translate,
            toward_anchor: t.toward_anchor,
            scale: t.scale,
            rotate: t.rotate,
        }
    }

    fn set(&self, style: &mut Style) {
        style.opacity = self.opacity;
        let t = &mut style.transform;
        t.translate = self.translate;
        t.toward_anchor = self.toward_anchor;
        t.scale = self.scale;
        t.rotate = self.rotate;
    }

    /// Set what keyframes `frames` animate at `progress`.
    fn keyframed(&mut self, frames: &[Keyframe], progress: f32, easing: Easing) {
        if let Some(v) = keyframed(frames, |k| k.opacity, self.opacity, progress, easing) {
            self.opacity = v;
        }
        if let Some(v) = keyframed(frames, |k| k.translate_x, self.translate[0], progress, easing) {
            self.translate[0] = v;
        }
        if let Some(v) = keyframed(frames, |k| k.translate_y, self.translate[1], progress, easing) {
            self.translate[1] = v;
        }
        if let Some(v) = keyframed(frames, |k| k.toward_anchor, self.toward_anchor, progress, easing) {
            self.toward_anchor = v;
        }
        if let Some(v) = keyframed(frames, |k| k.scale, self.scale, progress, easing) {
            self.scale = v;
        }
        if let Some(v) = keyframed(frames, |k| k.rotate, self.rotate, progress, easing) {
            self.rotate = v;
        }
    }
}

/// What [`Motion::sample`] found of a node.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sampled {
    /// Its translation moves (a transition, an animation or the compositor
    /// sets it apart from its own value): paint applies it as a transform.
    pub translating: bool,
    /// Only its opacity and transform move, and the compositor moves them
    /// by these at its own frame times (ADR-0015).
    pub curves: Option<Arc<Curves>>,
    /// Motion keeps a state for the node.
    pub kept: bool,
    /// Its laid-out height is its natural height ([`Motion::natural`]).
    pub measures: bool,
}

/// A node's moving opacity and transform as plain data the render thread
/// samples at each frame's present time (ADR-0015), by the code the layout
/// samples them with: the values with nothing running, the transitions that
/// run and the animation that plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Curves {
    own: Own,
    opacity: Option<Run<f32>>,
    translate_x: Option<Run<Calc>>,
    translate_y: Option<Run<Calc>>,
    scale: Option<Run<f32>>,
    rotate: Option<Run<f32>>,
    playing: Option<CurvePlay>,
}

/// An animation the compositor plays: as [`Playing`], its timing resolved.
#[derive(Clone, Debug, PartialEq)]
struct CurvePlay {
    animation: Arc<Animation>,
    start: f32,
    leaving: bool,
    duration: f32,
    easing: Easing,
    hold: bool,
}

/// A node's opacity and transform at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moment {
    pub opacity: f32,
    pub transform: crate::style::Transform,
}

impl Curves {
    /// Whether it scales or turns the node (its matrix is drawn throughout).
    #[must_use]
    pub fn turns(&self) -> bool {
        self.scale.is_some()
            || self.rotate.is_some()
            || (self.own.scale - 1.0).abs() > 1e-4
            || self.own.rotate.rem_euclid(360.0) > 1e-4
            || self.playing.as_ref().is_some_and(|p| {
                p.animation
                    .keyframes
                    .iter()
                    .any(|k| k.pose.scale.is_some() || k.pose.rotate.is_some())
            })
    }

    /// The values at `now` (seconds on the frame clock), starting from the
    /// node's `transform` (its origin and what does not move).
    #[must_use]
    pub fn at(&self, now: f32, transform: crate::style::Transform) -> Moment {
        fn run<T: Mix>(r: Option<&Run<T>>, target: T, now: f32) -> T {
            match r {
                Some(r) if now < r.end() => r.value(now),
                _ => target,
            }
        }
        let o = self.own;
        let mut v = Own {
            opacity: run(self.opacity.as_ref(), o.opacity, now),
            translate: [
                run(self.translate_x.as_ref(), o.translate[0], now),
                run(self.translate_y.as_ref(), o.translate[1], now),
            ],
            toward_anchor: o.toward_anchor,
            scale: run(self.scale.as_ref(), o.scale, now),
            rotate: run(self.rotate.as_ref(), o.rotate, now),
        };
        if let Some(p) = &self.playing
            && let Some(at) = progress(&p.animation, p.start, p.leaving, p.duration, p.hold, now, None)
        {
            v.keyframed(&p.animation.keyframes, at, p.easing);
        }
        Moment {
            opacity: v.opacity,
            transform: crate::style::Transform {
                translate: v.translate,
                toward_anchor: v.toward_anchor,
                scale: v.scale,
                rotate: v.rotate,
                ..transform
            },
        }
    }
}

/// The value a property takes at `progress` of keyframes `frames`, each
/// segment eased by `easing`; ends no keyframe sets take `own`. `None` when
/// no keyframe sets it.
fn keyframed<T: Mix>(
    frames: &[Keyframe],
    get: impl Fn(&Pose) -> Option<T>,
    own: T,
    progress: f32,
    easing: Easing,
) -> Option<T> {
    let mut points: Vec<(f32, T)> = frames.iter().filter_map(|k| get(&k.pose).map(|v| (k.at, v))).collect();
    let (first, last) = (points.first()?.0, points.last()?.0);
    if first > 0.0 {
        points.insert(0, (0.0, own.clone()));
    }
    if last < 1.0 {
        points.push((1.0, own));
    }
    let segment = points
        .windows(2)
        .find(|w| matches!(w, [_, b] if progress <= b.0))
        .or_else(|| points.windows(2).last())?;
    let [(from_at, from), (to_at, to)] = segment else {
        return points.first().map(|(_, v)| v.clone());
    };
    let span = to_at - from_at;
    let local = if span > 0.0 {
        ((progress - from_at) / span).clamp(0.0, 1.0)
    } else {
        1.0
    };
    Some(from.mix(to, easing.at(local)))
}

/// The children of the box at `path` in `root`, a layer's root standing for
/// the [`Element::Deferred`] that lifts it.
fn siblings_at<'a>(root: &'a mut Element, path: &[PathSeg]) -> Option<&'a mut Vec<Element>> {
    let (_, rest) = path.split_first()?;
    let mut at = root;
    for seg in rest {
        at = child_mut(kids_mut(at)?, *seg)?;
    }
    boxed_mut(at)
}

/// The children of a box, to put one back among: a paragraph's atomic
/// inlines stand on its content, and one leaving goes with its place.
fn boxed_mut(el: &mut Element) -> Option<&mut Vec<Element>> {
    match el {
        Element::Div(d) => Some(&mut Rc::make_mut(d).kind),
        Element::Deferred(d) => boxed_mut(&mut Rc::make_mut(d).child),
        _ => None,
    }
}

fn kids_mut(el: &mut Element) -> Option<&mut [Element]> {
    match el {
        Element::Deferred(d) => kids_mut(&mut Rc::make_mut(d).child),
        _ => el.kids_mut(),
    }
}

fn child_mut(kids: &mut [Element], seg: PathSeg) -> Option<&mut Element> {
    match seg {
        PathSeg::Index(i) => kids.get_mut(i),
        PathSeg::Key(k) => kids.iter_mut().find(|c| c.key() == Some(k)),
        PathSeg::Component(..) => kids.iter_mut().find(|c| c.seg(0) == seg),
    }
}

/// The element in the slot at `path` in `root`, and its place among its siblings.
fn slot_at(root: &Element, path: &NodePath) -> Option<(Element, usize)> {
    let (last, parent) = path.0.split_last()?;
    let (_, rest) = parent.split_first()?;
    let mut at = root;
    for seg in rest {
        let kids = kids(at)?;
        at = kids.get(index_of(kids, *seg)?)?;
    }
    let kids = kids(at)?;
    let index = index_of(kids, *last)?;
    Some((kids.get(index)?.clone(), index))
}

/// Whether `root` has a node at `path`.
fn holds(root: &Element, path: &[PathSeg]) -> bool {
    let Some((_, rest)) = path.split_first() else {
        return false;
    };
    let mut at = root;
    for seg in rest {
        let Some(next) = kids(at).and_then(|k| k.get(index_of(k, *seg)?)) else {
            return false;
        };
        at = next;
    }
    !matches!(at, Element::Empty)
}

fn kids(el: &Element) -> Option<&[Element]> {
    match el {
        Element::Deferred(d) => kids(&d.child),
        _ => el.kids(),
    }
}

fn index_of(kids: &[Element], seg: PathSeg) -> Option<usize> {
    match seg {
        PathSeg::Index(i) => (i < kids.len()).then_some(i),
        PathSeg::Key(k) => kids.iter().position(|c| c.key() == Some(k)),
        PathSeg::Component(..) => kids.iter().position(|c| c.seg(0) == seg),
    }
}

/// Put `slot` back among `kids` at `seg`; false when something holds that
/// place now (the node came back).
fn put_back(kids: &mut Vec<Element>, seg: PathSeg, index: usize, slot: &Element) -> bool {
    match seg {
        PathSeg::Key(_) | PathSeg::Component(..) => {
            if kids.iter().enumerate().any(|(i, c)| c.seg(i) == seg) {
                return false;
            }
            kids.insert(index.min(kids.len()), slot.clone());
            true
        }
        PathSeg::Index(i) => match kids.get_mut(i) {
            // A layer shares its content's place: one that holds nothing
            // now is the place its leaving content goes back to.
            Some(at)
                if matches!(&*at, Element::Empty)
                    || matches!(&*at, Element::Deferred(d) if matches!(d.child, Element::Empty)) =>
            {
                *at = slot.clone();
                true
            }
            Some(_) => false,
            None => {
                kids.resize_with(i, Element::default);
                kids.push(slot.clone());
                true
            }
        },
    }
}

/// Make a leaving subtree inert: no handlers, no focus, and the pointer
/// passes through it (`pointer-events: none` while it animates out); it is
/// [`crate::Div::inert`] too, so it leaves the layer stack and the reader's
/// tree as it starts to leave.
fn inert(el: &mut Element) {
    if let Element::Deferred(d) = el {
        inert(&mut Rc::make_mut(d).child);
        return;
    }
    if let Some(c) = el.common_mut() {
        c.handlers = crate::element::Handlers::default();
        c.focus = None;
        c.focusable = false;
        c.pass_through = true;
        c.inert = true;
    }
    if let Some(kids) = el.kids_mut() {
        kids.iter_mut().for_each(inert);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyframes_ease_each_segment_and_fill_open_ends_with_the_own_value() {
        let pulse = Animation::pulse();
        let at = |p| keyframed(&pulse.keyframes, |k| k.opacity, 1.0, p, Easing::Linear);
        assert_eq!(at(0.0), Some(1.0));
        assert_eq!(at(0.25), Some(0.75));
        assert_eq!(at(0.5), Some(0.5));
        assert_eq!(at(1.0), Some(1.0));
        assert_eq!(keyframed(&pulse.keyframes, |k| k.scale, 1.0, 0.3, Easing::Linear), None);
    }

    #[test]
    fn a_reversed_transition_takes_as_long_as_the_way_back() {
        let (mut track, mut busy) = (None, Busy::default());
        let timing = Some(Timing {
            duration: 1.0,
            easing: Easing::Linear,
            delay: 0.0,
        });
        transition(&mut track, 0.0_f32, timing, 0.0, &mut busy);
        transition(&mut track, 1.0, timing, 0.0, &mut busy);
        assert_eq!(transition(&mut track, 1.0, timing, 0.25, &mut busy), 0.25);
        // Back to 0 from a quarter of the way: a quarter of the time.
        let mut busy = Busy::default();
        assert_eq!(transition(&mut track, 0.0, timing, 0.25, &mut busy), 0.25);
        assert_eq!(busy.until, Some(0.5));
        assert_eq!(transition(&mut track, 0.0, timing, 0.5, &mut busy), 0.0);
    }
}
