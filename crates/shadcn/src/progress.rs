//! Progress (`ui/progress`): a bar filled to a percentage.

use gui::style::Calc;
use gui::{Div, Styled, div};

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
        .bg(t.primary.opacity(0.2))
        .relative()
        .h(2.0)
        .w_full()
        .overflow_hidden()
        .rounded_full()
        .child(
            div()
                .named("progress-indicator")
                .bg(t.primary)
                .h_full()
                .w_full()
                .transition_all()
                .translate_x(Calc::fraction(filled - 1.0)),
        )
}
