//! Slider (`ui/slider`): one value, or a range, picked along a track.
//!
//! As reka's `SliderRoot` it keeps its values itself (`default_value`) or
//! in the caller's model (`model`, `v-model`); `on_change` hears each move.
//! Its behaviour (keys, the pointer, snapping, the thumbs' semantics) is
//! `primitives::slider`'s; this module draws it.

use gui::Color;
use gui::Properties;
use gui::prelude::*;
use gui::{Axis, Bind, Div, Model, component, div, px, use_model, watch};
use primitives::slider::{Slide, Slider, SliderConfig};
use primitives::{Availability, use_direction};

use crate::focus::StateLook;
use crate::part::{Part, PartProps};
use crate::separator::Orientation;
use crate::theme::theme;

/// How far a thumb (`size-4`) reaches past the `h-1.5` track on each side.
const THUMB_OVERHANG: f32 = (6.0 - 16.0) / 2.0;
/// The thumb's size in pixels; thumbs travel the track less this.
const THUMB: f32 = 16.0;

/// What [`slider`] is given: its values (one per thumb, ascending), their
/// bounds and step, and its orientation.
#[derive(Clone, PartialEq)]
pub struct SliderProps {
    value: Bind<Vec<f64>>,
    range: Range,
    disabled: bool,
    orientation: Orientation,
}

/// A slider's bounds and step (reka's `min`, `max`, `step`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Range {
    min: f64,
    max: f64,
    step: f64,
}

impl SliderProps {
    /// What the primitive is told.
    fn config(&self) -> (SliderConfig, Availability) {
        let config = SliderConfig {
            min: self.range.min,
            max: self.range.max,
            step: self.range.step,
            orientation: match self.orientation {
                Orientation::Horizontal => Axis::Horizontal,
                Orientation::Vertical => Axis::Vertical,
            },
            ..SliderConfig::default()
        };
        (config, Availability::from_disabled(self.disabled))
    }
}

/// A slider laid out along `orientation` (`<Slider>`): one thumb at 0 on a
/// track from 0 to 100 in steps of 1, kept by itself, until told otherwise.
/// Drag anywhere on it, or focus a thumb and use the arrow keys.
#[must_use]
pub fn slider(orientation: Orientation) -> Part<SliderProps> {
    let horizontal = orientation == Orientation::Horizontal;
    let root = div()
        .named("slider")
        .position(Position::Relative)
        .display(Display::Flex)
        .align_items(Align::Center)
        .when(horizontal, |d| d.width(pct(100.0)))
        .when(!horizontal, |d| {
            d.display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .height(pct(100.0))
                .min_height(176.0)
                .width(Length::Auto)
        })
        .with_interaction(|d, i| d.dim_disabled(i, 0.5));
    let props = SliderProps {
        value: Bind::default_value(vec![0.0]),
        range: Range {
            min: 0.0,
            max: 100.0,
            step: 1.0,
        },
        disabled: false,
        orientation,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let slider = Slider::new(
                use_model(props, |p: &PartProps<SliderProps>| &p.props.value),
                use_direction(None),
            );
            let tell = move |(config, availability): (SliderConfig, Availability)| {
                slider.set_config(config);
                slider.set_availability(availability);
            };
            tell(props.with_untracked(|p| p.props.config()));
            slider.set_thumb(true, f64::from(THUMB));
            watch(move || props.with(|p| p.props.config()), move |c, _| tell(*c));
            move || {
                let values = slider.values();
                props.with(|p| render(&p.props, p.slot.0.clone(), &values, slider).into_element())
            }
        })
    })
}

impl Part<SliderProps> {
    /// The values, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<Vec<f64>>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// The thumbs' first values, when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(self, value: Vec<f64>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_default(value))
    }

    /// Call `f` with the values after each move (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Into<gui::Callback<Vec<f64>>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    #[must_use]
    pub fn min(self, min: f64) -> Self {
        self.with_props(|p| p.range.min = min)
    }

    #[must_use]
    pub fn max(self, max: f64) -> Self {
        self.with_props(|p| p.range.max = max)
    }

    #[must_use]
    pub fn step(self, step: f64) -> Self {
        self.with_props(|p| p.range.step = step)
    }

    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }
}

/// The slider's track, range and thumbs in `root`, the thumbs at `values`.
fn render(p: &SliderProps, root: Div, values: &[f64], slider: Slider<PartProps<SliderProps>>) -> Div {
    let t = theme();
    let (config, _) = p.config();
    let (lo, hi) = slider.range();
    let (lo, hi) = (f64_to_f32(lo), f64_to_f32(hi));
    let horizontal = p.orientation == Orientation::Horizontal;
    // Right to left, a horizontal slider runs from the right, as its keys do.
    let mirrored = horizontal && slider.slide() == Slide::FromRight;
    let range = div()
        .named("slider-range")
        .position(Position::Absolute)
        .background(t.primary);
    let range = if horizontal {
        let (start, end) = (Length::Fraction(lo), Length::Fraction(1.0 - hi));
        let range = range.top(0.0).bottom(0.0);
        if mirrored {
            range.right(start).left(end)
        } else {
            range.left(start).right(end)
        }
    } else {
        range
            .left(0.0)
            .right(0.0)
            .bottom(Length::Fraction(lo))
            .top(Length::Fraction(1.0 - hi))
    };
    let track = div()
        .named("slider-track")
        .position(Position::Relative)
        .flex_grow(1.0)
        .overflow(Overflow::Hidden)
        .radius(9999.0)
        .background(t.muted)
        .when(horizontal, |d| d.height(6.0).width(pct(100.0)))
        .when(!horizontal, |d| d.height(pct(100.0)).width(6.0))
        .child(range);
    // Thumbs stay inside the track: each travels its length less one thumb.
    let rail = div().named("slider-thumbs").position(Position::Absolute).inset(0.0);
    let rail = match (horizontal, mirrored) {
        (true, false) => rail.right(px(THUMB)),
        (true, true) => rail.left(px(THUMB)),
        (false, _) => rail.top(px(THUMB)),
    };
    let count = values.len();
    let thumb_els = values.iter().enumerate().map(|(i, v)| {
        let f = f64_to_f32(primitives::slider::fraction(*v, &config));
        let d = div()
            .named("slider-thumb")
            .transition(Properties::COLOR | Properties::SHADOW)
            .position(Position::Absolute)
            .when(horizontal && !mirrored, |d| {
                d.left(Length::Fraction(f)).top(px(THUMB_OVERHANG))
            })
            .when(mirrored, |d| d.right(Length::Fraction(f)).top(px(THUMB_OVERHANG)))
            .when(!horizontal, |d| d.bottom(Length::Fraction(f)).left(px(THUMB_OVERHANG)))
            .display(Display::Block)
            .size(16.0)
            .flex_shrink(0.0)
            .radius(9999.0)
            .border(1.0)
            .border_color(t.primary)
            .background(gui::Color::WHITE)
            .shadow([
                Shadow {
                    x: 0.0,
                    y: 1.0,
                    blur: 3.0,
                    spread: 0.0,
                    color: Color::BLACK.opacity(0.1),
                },
                Shadow {
                    x: 0.0,
                    y: 1.0,
                    blur: 2.0,
                    spread: -1.0,
                    color: Color::BLACK.opacity(0.1),
                },
            ])
            .with_interaction(|d, it| {
                d.ring(move || (it.hovered() || it.focus_visible()).then(|| Ring::new(4.0, t.ring.opacity(0.5))))
            });
        slider.thumb(d, i, count)
    });
    slider.root(root).child(track).child(rail.children(thumb_els))
}

/// A fraction of the track as layout takes it.
#[expect(clippy::as_conversions, reason = "a fraction in 0..=1; `as` rounds to nearest")]
fn f64_to_f32(v: f64) -> f32 {
    v as f32
}
