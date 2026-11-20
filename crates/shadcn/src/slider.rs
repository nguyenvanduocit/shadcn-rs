//! Slider (`ui/slider`): one value, or a range, picked along a track.

use std::cell::Cell;
use std::rc::Rc;

use gui::Properties;
use gui::prelude::*;
use gui::{Bounds, Div, DragEvent, DragPhase, FocusHandle, Key, KeyEvent, NamedKey, Signal, div, px, relative};

use crate::separator::Orientation;
use crate::theme::theme;

/// How far a thumb (`size-4`) reaches past the `h-1.5` track on each side.
const THUMB_OVERHANG: f32 = (1.5 - 4.0) / 2.0 * gui::styled::UNIT;
/// The thumb's size in pixels; thumbs travel the track less this.
const THUMB: f32 = 4.0 * gui::styled::UNIT;

/// A slider's state: its values (one per thumb, ascending) and their bounds.
/// Create it once, outside the render closure.
#[derive(Clone)]
pub struct SliderState {
    pub value: Signal<Vec<f32>>,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub disabled: Signal<bool>,
    thumbs: Rc<[FocusHandle]>,
    /// The thumb a drag moves.
    active: Rc<Cell<usize>>,
}

impl SliderState {
    /// A slider from `min` to `max` in steps of `step`, its thumbs at `value`.
    #[must_use]
    pub fn new(value: Vec<f32>, min: f32, max: f32, step: f32) -> Self {
        let thumbs = value.iter().map(|_| FocusHandle::new()).collect();
        Self {
            value: Signal::new(value),
            min,
            max,
            step,
            disabled: Signal::new(false),
            thumbs,
            active: Rc::new(Cell::new(0)),
        }
    }

    /// `v` clamped to the bounds and snapped to the nearest step, rounded to
    /// the step's decimals so `0.1 * 3` reads `0.3`.
    fn snap(&self, v: f32) -> f32 {
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
    fn fraction(&self, v: f32) -> f32 {
        let span = self.max - self.min;
        if span > 0.0 {
            ((v - self.min) / span).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Set the active thumb to `v`, keep the values ascending, and follow the
    /// thumb to its new place among them.
    fn set_active(&self, v: f32) {
        let v = self.snap(v);
        let mut values = self.value.get();
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

    /// The value under the pointer at `(x, y)` on a slider laid out in `b`.
    fn value_at(&self, orientation: Orientation, b: Bounds, x: f32, y: f32) -> f32 {
        let p = match orientation {
            Orientation::Horizontal if b.width > 0.0 => (x - b.x) / b.width,
            Orientation::Vertical if b.height > 0.0 => (b.y + b.height - y) / b.height,
            _ => 0.0,
        };
        self.min + p.clamp(0.0, 1.0) * (self.max - self.min)
    }

    fn drag(&self, orientation: Orientation, ev: &DragEvent) {
        let v = self.value_at(orientation, ev.bounds, ev.x, ev.y);
        if ev.phase == DragPhase::Start {
            let values = self.value.get();
            let nearest = values
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| (*a - v).abs().total_cmp(&(*b - v).abs()))
                .map_or(0, |(i, _)| i);
            self.active.set(nearest);
        }
        self.set_active(v);
        if let Some(f) = self.thumbs.get(self.active.get()) {
            f.focus();
        }
    }

    /// Arrows step the thumb `at`, Page keys by ten steps, Home and End to the bounds.
    fn key(&self, at: usize, ev: &KeyEvent) -> bool {
        let Some(cur) = self.value.get().get(at).copied() else {
            return false;
        };
        let big = self.step * 10.0;
        let next = match &ev.key {
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowUp) if ev.modifiers.shift => cur + big,
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowDown) if ev.modifiers.shift => cur - big,
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowUp) => cur + self.step,
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowDown) => cur - self.step,
            Key::Named(NamedKey::PageUp) => cur + big,
            Key::Named(NamedKey::PageDown) => cur - big,
            Key::Named(NamedKey::Home) => self.min,
            Key::Named(NamedKey::End) => self.max,
            _ => return false,
        };
        self.active.set(at);
        self.set_active(next);
        if let Some(f) = self.thumbs.get(self.active.get()) {
            gui::focus::set_focus(Some(f.id()), true);
        }
        true
    }
}

/// A slider laid out along `orientation`; drag anywhere on it, or focus a
/// thumb and use the arrow keys.
#[must_use]
pub fn slider(state: &SliderState, orientation: Orientation) -> Div {
    let t = theme();
    let values = state.value.get();
    let disabled = state.disabled.get();
    let (lo, hi) = match values.as_slice() {
        [one] => (0.0, state.fraction(*one)),
        many => (
            many.first().map_or(0.0, |v| state.fraction(*v)),
            many.last().map_or(0.0, |v| state.fraction(*v)),
        ),
    };
    let horizontal = orientation == Orientation::Horizontal;
    let range = div().named("slider-range").absolute().bg(t.primary);
    let range = if horizontal {
        range
            .top(0.0)
            .bottom(0.0)
            .left_len(relative(lo))
            .right_len(relative(1.0 - hi))
    } else {
        range
            .left(0.0)
            .right(0.0)
            .bottom_len(relative(lo))
            .top_len(relative(1.0 - hi))
    };
    let track = div()
        .named("slider-track")
        .relative()
        .grow()
        .overflow_hidden()
        .rounded_full()
        .bg(t.muted)
        .when(horizontal, |d| d.h(1.5).w_full())
        .when(!horizontal, |d| d.h_full().w(1.5))
        .child(range);
    // Thumbs stay inside the track: each travels its length less one thumb.
    let rail = div().named("slider-thumbs").absolute().inset(0.0);
    let rail = if horizontal {
        rail.right_len(px(THUMB))
    } else {
        rail.top_len(px(THUMB))
    };
    let thumbs = values.iter().enumerate().map(|(i, v)| {
        let p = state.fraction(*v);
        let keys = state.clone();
        div()
            .named("slider-thumb")
            .transition_props(Properties::COLOR | Properties::SHADOW)
            .absolute()
            .when(horizontal, |d| d.left_len(relative(p)).top_len(px(THUMB_OVERHANG)))
            .when(!horizontal, |d| d.bottom_len(relative(p)).left_len(px(THUMB_OVERHANG)))
            .block()
            .size(4.0)
            .shrink_0()
            .rounded_full()
            .border()
            .border_color(t.primary)
            .bg(gui::Color::WHITE)
            .shadow_sm()
            .when_some(state.thumbs.get(i).copied(), Div::track_focus)
            .disabled(disabled)
            .hover(move |s| s.ring(4.0, t.ring.opacity(0.5)))
            .focus_visible(move |s| s.ring(4.0, t.ring.opacity(0.5)))
            .on_key_down(move |ev| keys.key(i, ev))
    });
    let drags = state.clone();
    div()
        .named("slider")
        .relative()
        .flex()
        .items_center()
        .when(horizontal, |d| d.w_full())
        .when(!horizontal, |d| d.flex_col().h_full().min_h(44.0).w_auto())
        .disabled(disabled)
        .disabled_style(|s| s.opacity(0.5))
        .on_drag(move |ev| drags.drag(orientation, ev))
        .child(track)
        .child(rail.children(thumbs))
}
