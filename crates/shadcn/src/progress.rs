//! Progress (`ui/progress`): a bar filled to a percentage.

use gui::prelude::*;
use gui::style::Calc;
use gui::{Div, div};

use crate::theme::theme;

/// A bar filled to `value` percent (0–100; reka's `modelValue`).
#[must_use]
pub fn progress(value: f32) -> Div {
    let t = theme();
    // Full width, slid left by `100 - value` percent inside the clipping root,
    // so a new value slides it there (`transition-all`).
    let filled = if value.is_finite() {
        (value / 100.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    div()
        .named("progress")
        .background(t.primary.opacity(0.2))
        .position(Position::Relative)
        .height(8.0)
        .width(pct(100.0))
        .overflow(Overflow::Hidden)
        .radius(9999.0)
        .child(
            div()
                .named("progress-indicator")
                .background(t.primary)
                .height(pct(100.0))
                .width(pct(100.0))
                .transition(Properties::ALL)
                .translate_x(Calc::fraction(filled - 1.0)),
        )
}
