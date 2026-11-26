//! Scroll positions and motion as exact fixed-point numbers (ADR-0033,
//! "One exact motion account").
//!
//! A scroll container's position, what the compositor moves it by, what it
//! hands the UI, a gesture's resampled and flung positions: all whole
//! numbers of [`PER_PX`]ths of a pixel in an `i64`. Addition of whole
//! numbers associates, so the same motion summed in any grouping (cut at
//! frames by the compositor, at pulls by the UI, at commits) is the same
//! sum, and a gesture settles where its events put it whatever the frames
//! and faults did. An `f32` of pixels holds a position only where it is
//! drawn or laid out ([`px`]): deep in a long list (36,000,000 px, where an
//! `f32` steps by 4 px) a 1 px step still moves the list by 1 px here.
//!
//! The one place a pixel amount becomes units with a remainder is where an
//! event's delta is taken ([`Carry`], in the input router): what is finer
//! than a unit is carried to the next event, never dropped, so 1,024 steps
//! of 1/1,024 px move 1 px. A delta the router hands on is a whole number
//! of units an `f32` of pixels holds exactly (what the `f32` could not hold
//! is carried as well), so [`units`] takes it back exactly however it
//! crossed a channel or a log.

/// Units in a pixel: 2^20. A position up to 2^43 px (8.8e12) fits an
/// `i64`, and up to 2^33 px is exact in the `f64` it is drawn through.
pub(crate) const PER_PX: i64 = 1 << 20;

/// A position or a motion on each axis, in units.
pub type Exact = (i64, i64);

/// `v` pixels in units, to the nearest (exact for a delta the router
/// handed on). A NaN is 0.
#[must_use]
pub(crate) fn units(v: f32) -> i64 {
    crate::convert::f64_to_i64((f64::from(v) * PER_PX_F64).round())
}

/// [`units`] on each axis.
#[must_use]
pub(crate) fn units2(v: (f32, f32)) -> Exact {
    (units(v.0), units(v.1))
}

/// `u` units in pixels, where it is drawn or laid out: the one rounding.
#[must_use]
pub(crate) fn px(u: i64) -> f32 {
    crate::convert::f64_to_f32(crate::convert::i64_to_f64(u) / PER_PX_F64)
}

/// [`px`] on each axis.
#[must_use]
pub(crate) fn px2(u: Exact) -> (f32, f32) {
    (px(u.0), px(u.1))
}

/// `p` within `0..=end` on each axis.
#[must_use]
pub(crate) fn clamp2(p: Exact, end: Exact) -> Exact {
    (p.0.clamp(0, end.0.max(0)), p.1.clamp(0, end.1.max(0)))
}

/// `a + b` on each axis, saturating.
#[must_use]
pub(crate) fn add2(a: Exact, b: Exact) -> Exact {
    (a.0.saturating_add(b.0), a.1.saturating_add(b.1))
}

/// `a - b` on each axis, saturating.
#[must_use]
pub(crate) fn sub2(a: Exact, b: Exact) -> Exact {
    (a.0.saturating_sub(b.0), a.1.saturating_sub(b.1))
}

const PER_PX_F64: f64 = 1_048_576.0;

/// What of the deltas taken so far was finer than a unit, carried to the
/// next: an event's delta in pixels becomes whole units, its remainder
/// kept, so no motion is lost however fine the steps.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Carry((f64, f64));

impl Carry {
    /// `delta` pixels, with what was carried, in whole units an `f32` of
    /// pixels holds exactly (so [`units`] of its [`px`] is itself, as it
    /// crosses a channel or a log in pixels); the rest carried on.
    pub(crate) fn take(&mut self, delta: (f32, f32)) -> Exact {
        let one = |d: f32, carried: &mut f64| {
            let want = f64::from(d) * PER_PX_F64 + *carried;
            if !want.is_finite() {
                *carried = 0.0;
                return 0;
            }
            // The nearest whole number of units, then the nearest of
            // those an `f32` holds: both exact in an `f64`, the difference
            // carried.
            let whole = units(px(crate::convert::f64_to_i64(want.round())));
            *carried = want - crate::convert::i64_to_f64(whole);
            whole
        };
        let (cx, cy) = &mut self.0;
        (one(delta.0, cx), one(delta.1, cy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit taken to pixels and back is itself, on either side of the
    /// range a list reaches.
    #[test]
    fn a_delta_on_the_grid_goes_through_an_f32_exactly() {
        for d in [
            1.0 / 1024.0,
            0.1,
            3.75,
            7.999_999,
            40.0,
            100.000_01,
            123_456.79,
            -0.000_000_9,
        ] {
            let u = Carry::default().take((0.0, d)).1;
            assert_eq!(units(px(u)), u, "{d}");
        }
    }

    /// Steps finer than a unit are carried: a million steps of a tenth of
    /// a unit move 100,000 units.
    #[test]
    fn steps_finer_than_a_unit_are_carried() {
        let mut c = Carry::default();
        let step = px(1) / 10.0;
        let moved: i64 = (0..1_000_000).map(|_| c.take((0.0, step)).1).sum();
        let exact = f64::from(step) * PER_PX_F64 * 1e6;
        assert!(
            (crate::convert::i64_to_f64(moved) - exact).abs() <= 1.0,
            "{moved} for {exact}"
        );
    }
}
