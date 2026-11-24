//! Carousel (`ui/carousel`): slides in a row (or column) shown a view at a
//! time, moved with the previous and next buttons, the arrow keys or autoplay.
//!
//! The behaviour is `primitives::carousel` (shadcn-vue's Carousel over
//! Embla): [`carousel`] keeps the snap shown, its own or the caller's
//! through `.model(…)` (Embla's `selectedScrollSnap`, which shadcn-vue's API
//! demo reads); its slides join it as they are laid out, so it counts them
//! as Embla does once mounted; the keys, the rotation and the semantics are
//! the primitive's. This file draws the track, the slides' gutters and the
//! arrows.
//!
//! Embla moves the track with a transform each animation frame; here the
//! track is translated by what the primitive says (a fraction of its own
//! size per snap, plus what a drag holds it at), bound to the node so a move
//! renders nothing, and eased over [`SLIDE_SECONDS`] to a snap (`easeOutCubic`),
//! 250 ms back from an overscroll, at once while dragged. Embla measures each
//! slide; `gui` hands a render no sizes, so a slide's step is its declared
//! basis (`.flex_basis(…)`), which holds while no slide outgrows it.

use std::time::Duration;

use gui::prelude::*;
use gui::style::Calc;
use gui::{Bind, ComponentElement, Div, Easing, Icon, Length, Model, component, div, icon, provide, use_model};
use primitives::Availability;
use primitives::carousel::{Carousel, CarouselOptions, Motion, Pace, Rotation};
use primitives::direction::use_direction;

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::part::{Part, PartProps, part_root};
use crate::separator::Orientation;

/// How long a move to the next slide takes.
pub const SLIDE_SECONDS: f32 = 0.4;

/// What [`carousel`] is given.
#[derive(Clone, Debug, PartialEq)]
pub struct CarouselProps {
    orientation: Orientation,
    /// The share of the viewport one slide takes (`basis-1/3` → 1/3).
    basis: f32,
    index: Bind<usize>,
    /// Advance a snap this often (Embla's Autoplay plugin).
    autoplay: Option<Duration>,
    /// Stop the autoplay while the pointer is over it, and play again once it leaves.
    stop_on_mouse_enter: bool,
}

impl Default for CarouselProps {
    fn default() -> Self {
        Self {
            orientation: Orientation::Horizontal,
            basis: 1.0,
            index: Bind::default(),
            autoplay: None,
            stop_on_mouse_enter: false,
        }
    }
}

/// How many places a track of `count` slides, each `basis` of the viewport,
/// stops at (Embla's `scrollSnapList().length`), snapping to the start
/// (`align: 'start'`) with the last snap trimmed so no space trails the last slide.
#[must_use]
pub fn carousel_snaps(count: usize, basis: f32) -> usize {
    primitives::carousel::snaps(count, basis)
}

/// What the parts of a carousel read: its primitive.
#[derive(Clone, Copy)]
struct CarouselContext(Carousel<PartProps<CarouselProps>>);

impl CarouselContext {
    fn find<P: 'static>(props: gui::Signal<PartProps<P>>, part: &'static str) -> Option<Self> {
        part_root(props, part, "carousel")
    }
}

fn axis(orientation: Orientation) -> Axis {
    match orientation {
        Orientation::Horizontal => Axis::Horizontal,
        Orientation::Vertical => Axis::Vertical,
    }
}

fn orientation(axis: Axis) -> Orientation {
    match axis {
        Axis::Horizontal => Orientation::Horizontal,
        Axis::Vertical => Orientation::Vertical,
    }
}

/// Embla's Autoplay plugin as shadcn-vue's demo sets it: it plays on at
/// the last slide and through presses (`stopOnInteraction: false` is the
/// demo's; Embla's `stopOnLastSnap` is off by default).
fn rotation(p: &CarouselProps) -> Option<Rotation> {
    p.autoplay.map(|delay| Rotation {
        pace: Pace::Step {
            delay,
            stop_on_last: false,
        },
        pause_on_hover: p.stop_on_mouse_enter,
        stop_on_interaction: false,
    })
}

/// The carousel's region: focusable, arrow keys move it.
#[must_use]
pub fn carousel() -> Part<CarouselProps> {
    let root = div().named("carousel").position(Position::Relative).focusable();
    Part::new(CarouselProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let carousel = Carousel::new(
                use_model(props, |p: &PartProps<CarouselProps>| &p.props.index),
                move || {
                    props.with(|p| CarouselOptions {
                        orientation: axis(p.props.orientation),
                        per_view: p.props.basis,
                        wrap: false,
                        // Embla's buttons are `disabled` at the ends.
                        at_end: Availability::Disabled,
                        duration: Duration::from_secs_f32(SLIDE_SECONDS),
                        ..CarouselOptions::default()
                    })
                },
                props.with_untracked(|p| rotation(&p.props)),
                use_direction(None),
            );
            gui::watch(
                move || props.with(|p| rotation(&p.props)),
                move |_, r| carousel.set_rotation(*r),
            );
            provide(CarouselContext(carousel));
            move || carousel.root(props.with(|p| p.slot.0.clone())).into_element()
        })
    })
}

impl Part<CarouselProps> {
    /// Slides along `orientation` (a row unless set).
    #[must_use]
    pub fn orientation(self, orientation: Orientation) -> Self {
        self.with_props(|p| p.orientation = orientation)
    }

    /// Each slide takes `fraction` of the viewport (1 unless set).
    #[must_use]
    pub fn basis(self, fraction: f32) -> Self {
        self.with_props(|p| p.basis = fraction)
    }

    /// The snap shown, kept by the caller.
    #[must_use]
    pub fn model(self, index: impl Into<Model<usize>>) -> Self {
        self.with_props(|p| p.index = Bind::model(index))
    }

    /// Advance a snap every `delay`, back to the first after the last
    /// (Embla's Autoplay plugin); `stop_on_mouse_enter` stops it while the
    /// pointer is over the carousel and plays it again once it leaves.
    #[must_use]
    pub fn autoplay(self, delay: Duration, stop_on_mouse_enter: bool) -> Self {
        self.with_props(|p| {
            p.autoplay = Some(delay);
            p.stop_on_mouse_enter = stop_on_mouse_enter;
        })
    }
}

/// The viewport and its track; style the track here (where shadcn's
/// `CarouselContent` puts its class) and put its [`carousel_item`]s in.
#[must_use]
#[track_caller]
pub fn carousel_content() -> Part<()> {
    Part::new((), div().named("carousel-track").display(Display::Flex), |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = CarouselContext::find(props, "carousel_content");
            move || {
                let mut track = props.with(|p| p.slot.0.clone()).into_element();
                let items = track.children_mut().map(std::mem::take).unwrap_or_default();
                let Some(CarouselContext(carousel)) = ctx else {
                    return track;
                };
                let orientation = orientation(carousel.options().orientation);
                let shift = move || -> Calc { carousel.offset() };
                // The track moves by its own size per viewport: a box filling it,
                // translated as Embla's transform does, easing out from wherever
                // it is (`easeOutCubic` to a snap, `ease-out` back from an
                // overscroll).
                let slider = div()
                    .named("carousel-slider")
                    .display(Display::Flex)
                    .position(Position::Relative)
                    .flex_shrink(0.0)
                    .transition(Properties::TRANSFORM)
                    .duration_ms(move || carousel.move_duration().as_secs_f32() * 1000.0)
                    .ease(move || match carousel.motion() {
                        Motion::Bounce => Easing::CubicBezier(0.0, 0.0, 0.58, 1.0),
                        Motion::Snap | Motion::Follow => Easing::CubicBezier(0.33, 1.0, 0.68, 1.0),
                    })
                    .children(items);
                let slider = match orientation {
                    Orientation::Horizontal => slider.width(pct(100.0)).translate_x(shift),
                    Orientation::Vertical => slider
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .height(pct(100.0))
                        .translate_y(shift),
                };
                if let Some(style) = track.style_mut() {
                    // `-ml-4` across, `-mt-4 flex-col` down: the caller's margin wins.
                    match orientation {
                        Orientation::Horizontal if style.margin.left == Length::Px(0.0) => {
                            style.margin.left = Length::Px(-16.0);
                        }
                        Orientation::Vertical => {
                            if style.margin.top == Length::Px(0.0) {
                                style.margin.top = Length::Px(-16.0);
                            }
                            style.flex_direction = gui::style::FlexDirection::Column;
                            if style.height == Length::Auto {
                                style.height = Length::Fraction(1.0);
                            }
                        }
                        Orientation::Horizontal => {}
                    }
                }
                if let Some(kids) = track.children_mut() {
                    kids.push(slider.into_element());
                }
                carousel
                    .content(div().named("carousel-content").overflow(Overflow::Hidden))
                    .child(track)
                    .into_element()
            }
        })
    })
}

/// One slide, the carousel's basis of the track.
#[must_use]
#[track_caller]
pub fn carousel_item() -> Part<()> {
    let root = div().named("carousel-item").min_width(0.0).flex_shrink(0.0);
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = CarouselContext::find(props, "carousel_item");
            let key = ctx.map(|CarouselContext(c)| c.slide_key());
            move || {
                let mut item = props.with(|p| p.slot.0.clone());
                let (Some(CarouselContext(carousel)), Some(key)) = (ctx, key) else {
                    return item.into_element();
                };
                let options = carousel.options();
                item = carousel.slide(item.flex_basis(Length::Fraction(options.per_view)), key);
                // `pl-4` across, `pt-4` down, unless the caller padded it
                // (its class wins in `cn`).
                let padding = item.style_mut().padding;
                match orientation(options.orientation) {
                    Orientation::Horizontal if padding.left == 0.0 => item = item.padding_left(16.0),
                    Orientation::Vertical if padding.top == 0.0 => item = item.padding_top(16.0),
                    _ => {}
                }
                item.into_element()
            }
        })
    })
}

fn arrow(orientation: Orientation, enabled: bool, glyph: Icon) -> Div {
    let d = button(ButtonVariant::Outline, ButtonSize::Icon)
        .position(Position::Absolute)
        .size(32.0)
        .radius(9999.0)
        .disabled(!enabled);
    // `-translate-1/2` of its own 32px. Vertical turns the button a quarter
    // (`rotate-90`); it is a circle, so only its arrow shows the turn.
    match orientation {
        Orientation::Horizontal => d.top(Length::Fraction(0.5)).margin_top(-16.0).child(icon(glyph)),
        Orientation::Vertical => d
            .left(Length::Fraction(0.5))
            .margin_left(-16.0)
            .child(icon(glyph).rotate(90.0)),
    }
    .into_div()
}

/// Which arrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarouselArrowProps {
    next: bool,
    /// Where the caller's code made it (a report of a missing carousel names it).
    at: &'static std::panic::Location<'static>,
}

/// The button before the slides; disabled on the first snap.
#[must_use]
#[track_caller]
pub fn carousel_previous() -> ComponentElement {
    carousel_arrow(false)
}

/// The button after the slides; disabled on the last snap.
#[must_use]
#[track_caller]
pub fn carousel_next() -> ComponentElement {
    carousel_arrow(true)
}

#[track_caller]
fn carousel_arrow(next: bool) -> ComponentElement {
    let at = std::panic::Location::caller();
    component(CarouselArrowProps { next, at }, |cx| {
        let props = cx.props();
        let (part, at) = props.with_untracked(|p| (if p.next { "carousel_next" } else { "carousel_previous" }, p.at));
        let ctx = primitives::context::use_root_at::<CarouselContext>(part, "carousel", at);
        move || {
            let next = props.with(|p| p.next);
            let Some(CarouselContext(carousel)) = ctx else {
                return gui::Element::Empty;
            };
            let orientation = orientation(carousel.options().orientation);
            let enabled = carousel.step_availability(next).actionable();
            if next {
                let d = arrow(orientation, enabled, Icon::ArrowRight).named("carousel-next");
                let d = carousel.step_button(d, true);
                match orientation {
                    Orientation::Horizontal => d.right(-48.0),
                    Orientation::Vertical => d.bottom(-48.0),
                }
            } else {
                let d = arrow(orientation, enabled, Icon::ArrowLeft).named("carousel-previous");
                let d = carousel.step_button(d, false);
                match orientation {
                    Orientation::Horizontal => d.left(-48.0),
                    Orientation::Vertical => d.top(-48.0),
                }
            }
            .into_element()
        }
    })
}
