//! Slider (`ui/slider`): one value, or a range, picked along a track.
//!
//! As reka's `SliderRoot` it keeps its values itself (`default_value`) or
//! in the caller's model (`model`, `v-model`); `on_change` hears each move.

use std::cell::Cell;
use std::rc::Rc;

use gui::Color;
use gui::Properties;
use gui::prelude::*;
use gui::{
    Bind, Bounds, Div, DragEvent, DragPhase, FocusHandle, Key, KeyEvent, Model, NamedKey, UseModel, component, div, px,
    use_model,
};

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
    value: Bind<Vec<f32>>,
    range: Range,
    disabled: bool,
    orientation: Orientation,
}

/// A slider's bounds and step (reka's `min`, `max`, `step`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Range {
    min: f32,
    max: f32,
    step: f32,
}

impl Range {
    /// `v` clamped to the bounds and snapped to the nearest step, rounded to
    /// the step's decimals so `0.1 * 3` reads `0.3`.
    fn snap(self, v: f32) -> f32 {
        let steps = if self.step > 0.0 {
            ((v - self.min) / self.step).round()
        } else {
            0.0
        };
        let snapped = (self.min + steps * self.step).clamp(self.min, self.max);
        let decimals = self.step.to_string().split_once('.').map_or(0, |(_, d)| d.len());
        let scale = 10_f32.powi(i32::try_from(decimals).unwrap_or(0));
        (snapped * scale).round() / scale
    }

    /// Where `v` sits between the bounds, 0 to 1.
    fn fraction(self, v: f32) -> f32 {
        let span = self.max - self.min;
        if span > 0.0 {
            ((v - self.min) / span).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The value under the pointer at `(x, y)` on a slider laid out in `b`.
    fn value_at(self, orientation: Orientation, b: Bounds, x: f32, y: f32) -> f32 {
        let p = match orientation {
            Orientation::Horizontal if b.width > 0.0 => (x - b.x) / b.width,
            Orientation::Vertical if b.height > 0.0 => (b.y + b.height - y) / b.height,
            _ => 0.0,
        };
        self.min + p.clamp(0.0, 1.0) * (self.max - self.min)
    }
}

/// What a slider's handlers move: its values, the thumbs' focus, and the
/// thumb a drag or key moves.
#[derive(Clone)]
struct Thumbs {
    value: UseModel<PartProps<SliderProps>, Vec<f32>>,
    props: gui::Signal<PartProps<SliderProps>>,
    focus: Rc<[FocusHandle]>,
    active: Rc<Cell<usize>>,
}

impl Thumbs {
    fn range(&self) -> Range {
        self.props.with_untracked(|p| p.props.range)
    }

    /// Set the active thumb to `v`, keep the values ascending, and follow the
    /// thumb to its new place among them.
    fn set_active(&self, v: f32) {
        let v = self.range().snap(v);
        let mut values = self.value.get_untracked();
        let Some(slot) = values.get_mut(self.active.get()) else {
            return;
        };
        *slot = v;
        values.sort_by(f32::total_cmp);
        if let Some(i) = values.iter().position(|x| *x == v) {
            self.active.set(i);
        }
        self.value.set(values);
    }

    fn drag(&self, orientation: Orientation, ev: &DragEvent) {
        let v = self.range().value_at(orientation, ev.bounds, ev.x, ev.y);
        if ev.phase == DragPhase::Start {
            let values = self.value.get_untracked();
            let nearest = values
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| (*a - v).abs().total_cmp(&(*b - v).abs()))
                .map_or(0, |(i, _)| i);
            self.active.set(nearest);
        }
        self.set_active(v);
        if let Some(f) = self.focus.get(self.active.get()) {
            f.focus();
        }
    }

    /// Arrows step the thumb `at`, Page keys by ten steps, Home and End to the bounds.
    fn key(&self, at: usize, ev: &KeyEvent) -> bool {
        let Some(cur) = self.value.get_untracked().get(at).copied() else {
            return false;
        };
        let r = self.range();
        let big = r.step * 10.0;
        let next = match &ev.key {
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowUp) if ev.modifiers.shift => cur + big,
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowDown) if ev.modifiers.shift => cur - big,
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowUp) => cur + r.step,
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowDown) => cur - r.step,
            Key::Named(NamedKey::PageUp) => cur + big,
            Key::Named(NamedKey::PageDown) => cur - big,
            Key::Named(NamedKey::Home) => r.min,
            Key::Named(NamedKey::End) => r.max,
            _ => return false,
        };
        self.active.set(at);
        self.set_active(next);
        if let Some(f) = self.focus.get(self.active.get()) {
            gui::focus::set_focus(Some(f.id()), true);
        }
        true
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
            let value = use_model(props, |p| &p.props.value);
            let thumbs = Thumbs {
                value,
                props,
                focus: (0..value.get_untracked().len()).map(|_| FocusHandle::new()).collect(),
                active: Rc::new(Cell::new(0)),
            };
            move || {
                let values = value.get();
                props.with(|p| render(&p.props, p.slot.0.clone(), &values, &thumbs).into_element())
            }
        })
    })
}

impl Part<SliderProps> {
    /// The values, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<Vec<f32>>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// The thumbs' first values, when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(self, value: Vec<f32>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_default(value))
    }

    /// Call `f` with the values after each move (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(Vec<f32>) + 'static) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    #[must_use]
    pub fn min(self, min: f32) -> Self {
        self.with_props(|p| p.range.min = min)
    }

    #[must_use]
    pub fn max(self, max: f32) -> Self {
        self.with_props(|p| p.range.max = max)
    }

    #[must_use]
    pub fn step(self, step: f32) -> Self {
        self.with_props(|p| p.range.step = step)
    }

    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }
}

/// The slider's track, range and thumbs in `root`, the thumbs at `values`.
fn render(p: &SliderProps, root: Div, values: &[f32], thumbs: &Thumbs) -> Div {
    let t = theme();
    let (orientation, r, disabled) = (p.orientation, p.range, p.disabled);
    let (lo, hi) = match values {
        [one] => (0.0, r.fraction(*one)),
        many => (
            many.first().map_or(0.0, |v| r.fraction(*v)),
            many.last().map_or(0.0, |v| r.fraction(*v)),
        ),
    };
    let horizontal = orientation == Orientation::Horizontal;
    let range = div()
        .named("slider-range")
        .position(Position::Absolute)
        .background(t.primary);
    let range = if horizontal {
        range
            .top(0.0)
            .bottom(0.0)
            .left(Length::Fraction(lo))
            .right(Length::Fraction(1.0 - hi))
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
    let rail = if horizontal {
        rail.right(px(THUMB))
    } else {
        rail.top(px(THUMB))
    };
    let thumb_els = values.iter().enumerate().map(|(i, v)| {
        let f = r.fraction(*v);
        let keys = thumbs.clone();
        div()
            .named("slider-thumb")
            .transition(Properties::COLOR | Properties::SHADOW)
            .position(Position::Absolute)
            .when(horizontal, |d| d.left(Length::Fraction(f)).top(px(THUMB_OVERHANG)))
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
            .when_some(thumbs.focus.get(i).copied(), Div::track_focus)
            .disabled(disabled)
            .with_interaction(|d, it| {
                d.ring(move || (it.hovered() || it.focus_visible()).then(|| Ring::new(4.0, t.ring.opacity(0.5))))
            })
            .on_key_down(move |ev| keys.key(i, ev))
    });
    let drags = thumbs.clone();
    root.disabled(disabled)
        .on_drag(move |ev| drags.drag(orientation, ev))
        .child(track)
        .child(rail.children(thumb_els))
}
