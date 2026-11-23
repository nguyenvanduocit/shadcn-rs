//! Progress (`ui/progress`): a bar filled to a percentage. Its semantics
//! (a `progressbar` with its value and percentage, an invalid value read as
//! indeterminate) are `primitives::progress`'s; this module draws it.

use gui::prelude::*;
use gui::style::Calc;
use gui::{Div, div};
use primitives::progress::{DEFAULT_MAX, Progress, ProgressText};

use crate::theme::theme;

/// A bar filled to `value` percent (0–100; reka's `modelValue`); a value
/// outside that is read as indeterminate, and reported.
#[must_use]
#[track_caller]
pub fn progress(value: f32) -> Div {
    let t = theme();
    let bar = Progress::new(Some(f64::from(value)), DEFAULT_MAX);
    // Full width, slid left by `100 - value` percent inside the clipping root,
    // so a new value slides it there (`transition-all`).
    let filled = fraction(bar.fraction());
    bar.root(div(), ProgressText::default())
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

/// A fraction of the bar as layout takes it.
#[expect(clippy::as_conversions, reason = "a fraction in 0..=1; `as` rounds to nearest")]
fn fraction(v: f64) -> f32 {
    v as f32
}
