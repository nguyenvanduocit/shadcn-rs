//! How far a task has come (reka's `ProgressRoot`, `ProgressIndicator`;
//! research §10 "Progress").
//!
//! - **Value.** A number in `0..=max`, or none: indeterminate. The user
//!   writes nothing. A value that is NaN, below 0 or above `max` is read as
//!   indeterminate; a `max` that is not above 0 (or NaN) is read as 100
//!   (`Progress/ProgressRoot.vue:45-72`). Each is reported to the developer,
//!   once per place in the source, as reka's `console.error`; reka also
//!   writes the corrected value back to the caller, which a value the
//!   caller passes here has no channel for, and needs none: what is shown
//!   follows the rule.
//! - **State** ([`ProgressState`]): indeterminate (no value), complete (the
//!   value is `max`), loading otherwise (`ProgressRoot.vue:110-116`). The
//!   value is not rounded; the minimum is 0.
//! - **ARIA.** Role `progressbar` with `aria-valuemin` 0, `aria-valuemax`,
//!   `aria-valuenow` (none while indeterminate) and `aria-valuetext`. reka
//!   names the bar by its rounded percentage ("80%") and gives no
//!   `aria-valuetext` unless asked (`ProgressRoot.vue:82-83,126-134`); a
//!   percentage is what the value reads as, not what the bar is, so here it
//!   is the `aria-valuetext` (research §10, "Dễ sót") and the bar keeps the
//!   name its label gives it. [`ProgressText`] words it for another
//!   language.
//! - **Indicator.** Decoration; it carries no ARIA (`ProgressIndicator.vue:18-26`).

#[expect(
    clippy::disallowed_types,
    reason = "diagnostics reported once per call site, per thread: no part's state"
)]
use std::cell::RefCell;
use std::collections::HashSet;
use std::panic::Location;

use gui::{Div, Role, ValueRange};

gui::engine_local! {
    /// The places an invalid value or max was already reported from.
    #[expect(clippy::disallowed_types, reason = "diagnostics reported once per call site, per thread: no part's state")]
    [Shared("diagnostics, reported once per call site: no frame's result reads them")]
    static REPORTED: RefCell<HashSet<(&'static Location<'static>, bool)>> = RefCell::new(HashSet::new());
}

/// The max a bar has unless told otherwise.
pub const DEFAULT_MAX: f64 = 100.0;

/// Where a bar stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProgressState {
    /// No value: how far is not known.
    Indeterminate,
    /// Under way.
    Loading,
    /// At its max.
    Complete,
}

/// How a bar's value is read: its percentage of the max, rounded.
#[derive(Clone, Copy, Debug)]
pub struct ProgressText {
    /// The value text for a rounded percentage.
    pub percent: fn(i64) -> String,
}

impl PartialEq for ProgressText {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::fn_addr_eq(self.percent, other.percent)
    }
}

impl Default for ProgressText {
    fn default() -> Self {
        Self {
            percent: |p| format!("{p}%"),
        }
    }
}

/// A bar's value and max, as they are read: an invalid value is
/// indeterminate, an invalid max is [`DEFAULT_MAX`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    value: Option<f64>,
    max: f64,
}

impl Progress {
    /// `value` of `max`, each read as above; an invalid one is reported to
    /// the developer, once per place in the source.
    #[must_use]
    #[track_caller]
    pub fn new(value: Option<f64>, max: f64) -> Self {
        let at = Location::caller();
        let max = if max.is_finite() && max > 0.0 {
            max
        } else {
            report(
                at,
                false,
                &format!("progress max {max} is not above 0; read as {DEFAULT_MAX}"),
            );
            DEFAULT_MAX
        };
        let value = match value {
            Some(v) if (0.0..=max).contains(&v) => Some(v),
            Some(v) => {
                report(
                    at,
                    true,
                    &format!("progress value {v} is not in 0..={max}; read as indeterminate"),
                );
                None
            }
            None => None,
        };
        Self { value, max }
    }

    /// The value, none while indeterminate.
    #[must_use]
    pub const fn value(self) -> Option<f64> {
        self.value
    }

    #[must_use]
    pub const fn max(self) -> f64 {
        self.max
    }

    #[must_use]
    pub fn state(self) -> ProgressState {
        match self.value {
            None => ProgressState::Indeterminate,
            Some(v) if v == self.max => ProgressState::Complete,
            Some(_) => ProgressState::Loading,
        }
    }

    /// How much of the bar is filled, 0 to 1; 0 while indeterminate. The
    /// look reads it (shadcn-vue's indicator reads the raw value as a
    /// percentage, so it assumes a max of 100; this does not).
    #[must_use]
    pub fn fraction(self) -> f64 {
        self.value.map_or(0.0, |v| v / self.max)
    }

    /// `d` as the bar: role, range, and the percentage as its value text.
    #[must_use]
    pub fn root(self, d: Div, text: ProgressText) -> Div {
        let percent = self.value.map(|_| {
            let rounded = (self.fraction() * 100.0).round();
            (text.percent)(f64_to_i64(rounded))
        });
        d.role(Role::ProgressIndicator).aria_value_range(ValueRange {
            now: self.value,
            min: 0.0,
            max: self.max,
            text: percent,
        })
    }
}

/// A whole percentage as an integer (it is in `0..=100`).
#[expect(clippy::as_conversions, reason = "a rounded value in 0..=100; `as` saturates")]
fn f64_to_i64(v: f64) -> i64 {
    v as i64
}

fn report(at: &'static Location<'static>, value: bool, message: &str) {
    if REPORTED.with(|r| r.borrow_mut().insert((at, value))) {
        eprintln!("primitives: {message} (at {at})");
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_MAX, Progress, ProgressState};

    #[test]
    fn invalid_values_are_indeterminate_and_an_invalid_max_is_100() {
        // (value, max, read value, read max, state)
        let table = [
            (Some(30.0), 100.0, Some(30.0), 100.0, ProgressState::Loading),
            (Some(100.0), 100.0, Some(100.0), 100.0, ProgressState::Complete),
            (Some(0.0), 100.0, Some(0.0), 100.0, ProgressState::Loading),
            (None, 100.0, None, 100.0, ProgressState::Indeterminate),
            (Some(f64::NAN), 100.0, None, 100.0, ProgressState::Indeterminate),
            (Some(-1.0), 100.0, None, 100.0, ProgressState::Indeterminate),
            (Some(101.0), 100.0, None, 100.0, ProgressState::Indeterminate),
            (Some(5.0), 0.0, Some(5.0), DEFAULT_MAX, ProgressState::Loading),
            (Some(5.0), f64::NAN, Some(5.0), DEFAULT_MAX, ProgressState::Loading),
            (Some(10.0), 10.0, Some(10.0), 10.0, ProgressState::Complete),
        ];
        for (value, max, read, read_max, state) in table {
            let p = Progress::new(value, max);
            assert_eq!(p.value(), read, "{value:?} of {max}");
            assert_eq!(p.max(), read_max, "{value:?} of {max}");
            assert_eq!(p.state(), state, "{value:?} of {max}");
        }
        assert_eq!(
            Progress::new(Some(5.0), 10.0).fraction(),
            0.5,
            "not assuming a max of 100"
        );
    }
}
