//! Carousel (`ui/carousel`): slides in a row (or column) shown a view at a
//! time, moved with the previous and next buttons, the arrow keys or autoplay.
//!
//! [`carousel`] keeps the snap shown: its own, or the caller's through
//! `.model(…)` (Embla's `selectedScrollSnap`, which shadcn-vue's API demo
//! reads). Its slides join it as they are laid out, so it counts them as
//! Embla does once mounted.
//!
//! Embla moves the track with a transform each animation frame; here the
//! track is translated by a fraction of its own size per snap, the move eased
//! over [`SLIDE_SECONDS`] as a transform transition. Embla measures each slide; `gui`
//! hands a render no sizes, so a slide's step is its declared basis
//! (`.flex_basis(…)`), which holds while no slide outgrows it.

use std::time::Duration;

use gui::clock;
use gui::prelude::*;
use gui::style::Calc;
use gui::{
    Bind, ComponentElement, Div, Easing, Icon, Length, Members, Model, NamedKey, Scope, Signal, UseModel, component,
    div, icon, on_mounted, on_updated, provide, use_context, use_model,
};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::part::{Part, PartProps};
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

fn float(n: usize) -> f32 {
    u16::try_from(n).map_or(f32::from(u16::MAX), f32::from)
}

/// How many places a track of `count` slides, each `basis` of the viewport,
/// stops at (Embla's `scrollSnapList().length`), snapping to the start
/// (`align: 'start'`) with the last snap trimmed so no space trails the last slide.
#[must_use]
pub fn carousel_snaps(count: usize, basis: f32) -> usize {
    let end = (float(count) * basis - 1.0).max(0.0);
    // Every slide start short of the end, then the end itself; the
    // epsilon keeps a start that lands on the end from counting twice.
    let short = (end / basis - 1e-3).ceil().max(0.0);
    (0..count)
        .take_while(|&i| float(i) < short)
        .count()
        .saturating_add(1)
        .min(count.max(1))
}

/// What the parts of a carousel read.
#[derive(Clone)]
struct CarouselContext {
    props: Signal<PartProps<CarouselProps>>,
    /// The snap shown, from 0 to the snaps less one.
    index: UseModel<PartProps<CarouselProps>, usize>,
    /// The slides laid out, counted after each layout that may change them.
    count: Signal<usize>,
    slides: Members<()>,
    /// The autoplay's next advance, while it plays: a scope owning its timer.
    autoplay: Signal<Option<Scope>>,
    owner: Scope,
}

impl CarouselContext {
    fn find() -> Option<Self> {
        use_context::<Self>()
    }

    fn orientation(&self) -> Orientation {
        self.props.with(|p| p.props.orientation)
    }

    fn basis(&self) -> f32 {
        self.props.with(|p| p.props.basis)
    }

    fn snaps(&self) -> usize {
        carousel_snaps(self.count.get(), self.basis())
    }

    fn can_scroll_prev(&self) -> bool {
        self.index.get() > 0
    }

    fn can_scroll_next(&self) -> bool {
        self.index.get().saturating_add(1) < self.snaps()
    }

    fn scroll_prev(&self) {
        self.scroll_to(self.index.get_untracked().saturating_sub(1));
    }

    fn scroll_next(&self) {
        self.scroll_to(self.index.get_untracked().saturating_add(1));
    }

    /// Move to snap `index`, from wherever the track is now.
    fn scroll_to(&self, index: usize) {
        let snaps = carousel_snaps(self.count.get_untracked(), self.props.with_untracked(|p| p.props.basis));
        self.index.set(index.min(snaps.saturating_sub(1)));
    }

    /// Advance a snap every `delay`, back to the first after the last.
    fn play(&self, delay: Duration) {
        self.stop();
        let me = self.clone();
        let tick = self.owner.run(Scope::new);
        tick.run(|| {
            clock::after(delay, move || {
                let snaps = carousel_snaps(me.count.get_untracked(), me.props.with_untracked(|p| p.props.basis));
                if me.index.get_untracked().saturating_add(1) < snaps {
                    me.scroll_next();
                } else {
                    me.scroll_to(0);
                }
                me.play(delay);
            })
        });
        self.autoplay.set(Some(tick));
    }

    /// Stop the autoplay; the slide shown stays.
    fn stop(&self) {
        if let Some(tick) = self.autoplay.get_untracked() {
            self.autoplay.set(None);
            tick.dispose();
        }
    }

    /// Where the track stops for the snap shown, in viewports from the start.
    fn position(&self) -> f32 {
        let end = (float(self.count.get()) * self.basis() - 1.0).max(0.0);
        (float(self.index.get()) * self.basis()).min(end)
    }
}

/// The carousel's region: focusable, arrow keys move it.
#[must_use]
pub fn carousel() -> Part<CarouselProps> {
    let root = div().named("carousel").position(Position::Relative).focusable();
    Part::new(CarouselProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = CarouselContext {
                props,
                index: use_model(props, |p| &p.props.index),
                count: Signal::labeled("carousel slides", 0),
                slides: Members::new(),
                autoplay: Signal::labeled("carousel autoplay", None),
                owner: Scope::new(),
            };
            provide(ctx.clone());
            // Count the slides once they are laid out, and after each render
            // that may have changed them.
            let count = {
                let ctx = ctx.clone();
                move || ctx.count.set(ctx.slides.len())
            };
            on_mounted(count.clone());
            on_updated(count);
            if let Some(delay) = props.with_untracked(|p| p.props.autoplay) {
                ctx.play(delay);
            }
            move || {
                let (orientation, autoplay, stop_on_enter, root) = props.with(|p| {
                    (
                        p.props.orientation,
                        p.props.autoplay,
                        p.props.stop_on_mouse_enter,
                        p.slot.0.clone(),
                    )
                });
                let (prev, next) = match orientation {
                    Orientation::Horizontal => (NamedKey::ArrowLeft, NamedKey::ArrowRight),
                    Orientation::Vertical => (NamedKey::ArrowUp, NamedKey::ArrowDown),
                };
                let keys = ctx.clone();
                let hover = ctx.clone();
                root.on_key_down(move |e| {
                    if e.is(prev) {
                        keys.scroll_prev();
                    } else if e.is(next) {
                        keys.scroll_next();
                    } else {
                        return false;
                    }
                    true
                })
                .when_some(autoplay.filter(|_| stop_on_enter), |d, delay| {
                    d.on_hover(move |inside| {
                        if inside {
                            hover.stop();
                        } else {
                            hover.play(delay);
                        }
                    })
                })
                .into_element()
            }
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
pub fn carousel_content() -> Part<()> {
    Part::new((), div().named("carousel-track").display(Display::Flex), |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = CarouselContext::find();
            move || {
                let mut track = props.with(|p| p.slot.0.clone()).into_element();
                let items = track.children_mut().map(std::mem::take).unwrap_or_default();
                let Some(ctx) = &ctx else {
                    return track;
                };
                let shift = Calc::fraction(-ctx.position());
                // The track moves by its own size per viewport: a box filling it,
                // translated by that fraction, as Embla's transform does, easing out
                // (`easeOutCubic`) from wherever it is.
                let slider = div()
                    .named("carousel-slider")
                    .display(Display::Flex)
                    .position(Position::Relative)
                    .flex_shrink(0.0)
                    .transition(Properties::TRANSFORM)
                    .duration_ms(SLIDE_SECONDS * 1000.0)
                    .ease(Easing::CubicBezier(0.33, 1.0, 0.68, 1.0))
                    .children(items);
                let slider = match ctx.orientation() {
                    Orientation::Horizontal => slider.width(pct(100.0)).translate_x(shift),
                    Orientation::Vertical => slider
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .height(pct(100.0))
                        .translate_y(shift),
                };
                if let Some(style) = track.style_mut() {
                    // `-ml-4` across, `-mt-4 flex-col` down: the caller's margin wins.
                    match ctx.orientation() {
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
                div()
                    .named("carousel-content")
                    .overflow(Overflow::Hidden)
                    .child(track)
                    .into_element()
            }
        })
    })
}

/// One slide, the carousel's basis of the track.
#[must_use]
pub fn carousel_item() -> Part<()> {
    let root = div().named("carousel-item").min_width(0.0).flex_shrink(0.0);
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = CarouselContext::find();
            move || {
                let mut item = props.with(|p| p.slot.0.clone());
                let Some(ctx) = &ctx else {
                    return item.into_element();
                };
                item = item.flex_basis(Length::Fraction(ctx.basis())).member(&ctx.slides, ());
                // `pl-4` across, `pt-4` down, unless the caller padded it
                // (its class wins in `cn`).
                let padding = item.style_mut().padding;
                match ctx.orientation() {
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
}

/// The button before the slides; disabled on the first snap.
#[must_use]
pub fn carousel_previous() -> ComponentElement {
    carousel_arrow(false)
}

/// The button after the slides; disabled on the last snap.
#[must_use]
pub fn carousel_next() -> ComponentElement {
    carousel_arrow(true)
}

fn carousel_arrow(next: bool) -> ComponentElement {
    component(CarouselArrowProps { next }, |cx| {
        let props = cx.props();
        let ctx = CarouselContext::find();
        move || {
            let next = props.with(|p| p.next);
            let Some(ctx) = ctx.clone() else {
                return gui::Element::Empty;
            };
            let orientation = ctx.orientation();
            if next {
                let d = arrow(orientation, ctx.can_scroll_next(), Icon::ArrowRight)
                    .named("carousel-next")
                    .on_click(move |_| ctx.scroll_next());
                match orientation {
                    Orientation::Horizontal => d.right(-48.0),
                    Orientation::Vertical => d.bottom(-48.0),
                }
            } else {
                let d = arrow(orientation, ctx.can_scroll_prev(), Icon::ArrowLeft)
                    .named("carousel-previous")
                    .on_click(move |_| ctx.scroll_prev());
                match orientation {
                    Orientation::Horizontal => d.left(-48.0),
                    Orientation::Vertical => d.top(-48.0),
                }
            }
            .into_element()
        }
    })
}
