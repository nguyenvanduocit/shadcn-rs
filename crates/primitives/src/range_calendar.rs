//! A month grid to pick a range of days from (reka's `RangeCalendar/`), on
//! the grid core of [`crate::calendar`]: the same view, keys, Tab stop and
//! kinds of day. What it adds is a range and how presses build it.
//!
//! **Picking** (a click, Enter or Space; `RangeCalendarCellTrigger.vue:106-171`),
//! nothing when read-only or the day is blocked or unavailable; in order:
//!
//! 1. A start and no end, and the days from the start to the pressed day do
//!    not make a range (a blocked or, for a contiguous range, an unavailable
//!    day between): the start pressed again clears it (unless
//!    `prevent_deselect`); another day is a first press ignored, and a second
//!    press of that same day makes it the new start.
//! 2. A one-day range pressed on its day clears (unless `prevent_deselect`).
//! 3. No start: the day starts the range.
//! 4. No end: the day ends it; before the start, the ends swap.
//! 5. Both ends and nothing fixed: the day starts a new range.
//! 6. The start fixed: a day before it moves the start, any other the end;
//!    the end fixed, the other way about. A fixed end is not locked.
//!
//! **The preview** (`useRangeCalendar.ts:107-143`) runs from the start to the
//! pointed day (the one last focused or under the pointer; leaving the grid
//! clears it), while the range is open or an end is fixed; with
//! `maximum_days` and no end, it is the window of that many days from the
//! start toward the pointed day; otherwise only while every day between is
//! usable. **`maximum_days` N** blocks, while only a start is set, the days N
//! or more from it either way; with both ends and an end fixed, the days that
//! would make the range longer than N.
//!
//! **Escape** while a range is half made puts back the last whole range and
//! goes on to the page (a popover around it closes too:
//! `RangeCalendarRoot.vue:381-388`).
//!
//! What reka gets wrong and this does not (ADR-0025, group 7):
//!
//! - A whole range the caller writes does not become the last valid one, so
//!   Escape afterwards puts back an older range
//!   (`RangeCalendarRoot.vue:349-362`): here every whole range, whoever
//!   wrote it, is the one Escape returns to.
//! - With `maximum_days` and no end, the preview window skips the contiguity
//!   test, so the end pressed can make a range across an unavailable day
//!   (`useRangeCalendar.ts:124-135`): here a press tests the days between
//!   whatever the preview shows.
//! - `aria-selected` and `aria-pressed` disagree on an unavailable day
//!   inside a contiguous range (`RangeCalendarCell.vue:23`,
//!   `RangeCalendarCellTrigger.vue:313`): here both say the day is in the
//!   range exactly when it is, and an unavailable day is never in a
//!   contiguous one.
//! - Every gridcell is `aria-disabled` with `disableDaysOutsideCurrentView`,
//!   as in Calendar: here only the days outside their month.

use std::fmt;

use gui::events::{KeyEvent, NamedKey};
use gui::{Computed, Derive, Div, Signal, UseModel, watch};

use crate::availability::Availability;
use crate::calendar::{CalendarGrid, CalendarOptions, Day};
use crate::date::CalendarDate;
use crate::direction::ReadDirection;

/// A range of days: each end may be unset; with both, `start <= end`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DateRange {
    pub start: Option<CalendarDate>,
    pub end: Option<CalendarDate>,
}

impl DateRange {
    /// No day.
    pub const EMPTY: Self = Self { start: None, end: None };

    /// The range from `a` to `b`, in whichever order they come.
    #[must_use]
    pub fn between(a: CalendarDate, b: CalendarDate) -> Self {
        Self {
            start: Some(a.min(b)),
            end: Some(a.max(b)),
        }
    }

    /// A start and no end yet.
    #[must_use]
    pub const fn started(start: CalendarDate) -> Self {
        Self {
            start: Some(start),
            end: None,
        }
    }

    /// `start` and `end` as given, put in order when both are set.
    #[must_use]
    pub fn of(start: Option<CalendarDate>, end: Option<CalendarDate>) -> Self {
        match (start, end) {
            (Some(a), Some(b)) => Self::between(a, b),
            _ => Self { start, end },
        }
    }

    /// Both ends set.
    #[must_use]
    pub fn is_complete(self) -> bool {
        self.start.is_some() && self.end.is_some()
    }

    /// Whether `day` is in it: an end, or between them
    /// (`useRangeCalendar.ts:63-72`).
    #[must_use]
    pub fn contains(self, day: CalendarDate) -> bool {
        match (self.start, self.end) {
            (Some(s), Some(e)) => (s.min(e)..=s.max(e)).contains(&day),
            (Some(d), None) | (None, Some(d)) => d == day,
            (None, None) => false,
        }
    }
}

/// Which end of a range a press does not move by default (`fixedDate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RangeEnd {
    Start,
    End,
}

/// What a range calendar adds to [`CalendarOptions`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RangeOptions {
    /// `allowNonContiguousRanges`: a range may hold unavailable days.
    pub non_contiguous: bool,
    /// `maximumDays`: the longest range, its ends included.
    pub maximum_days: Option<u32>,
    /// `fixedDate`.
    pub fixed: Option<RangeEnd>,
    /// `isDateHighlightable`: blocked or unavailable days a range may still
    /// run over.
    pub is_highlightable: Option<Derive<CalendarDate, bool>>,
}

/// How a day stands in the range, for its look: what reka's range cell
/// trigger reads into its `data-*` attributes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RangeDay {
    /// In the range (`data-selected`, `aria-pressed`, `aria-selected`).
    pub selected: bool,
    pub start: bool,
    pub end: bool,
    /// In the preview (`data-highlighted`), and its ends.
    pub highlighted: bool,
    pub highlight_start: bool,
    pub highlight_end: bool,
}

/// A range calendar's state: its grid, its range (read and written through
/// the part's [`UseModel`]), the pointed day, the last day pressed and the
/// last whole range. Made once in setup, `Copy`.
pub struct RangeCalendar<P: 'static> {
    pub grid: CalendarGrid,
    value: UseModel<P, DateRange>,
    calendar: Computed<CalendarOptions>,
    range: Computed<RangeOptions>,
    /// reka's `focusedValue`: what the preview runs to.
    pointed: Signal<Option<CalendarDate>>,
    /// reka's `lastPressedDateValue`, for a press that must be made twice.
    pressed: Signal<Option<CalendarDate>>,
    /// reka's `validModelValue`: the last whole range.
    valid: Signal<DateRange>,
}

impl<P: 'static> Clone for RangeCalendar<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for RangeCalendar<P> {}

impl<P: 'static> PartialEq for RangeCalendar<P> {
    fn eq(&self, other: &Self) -> bool {
        self.grid == other.grid
    }
}

impl<P: 'static> fmt::Debug for RangeCalendar<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RangeCalendar").finish_non_exhaustive()
    }
}

impl<P: 'static> RangeCalendar<P> {
    /// A range calendar showing `value` (the part's `v-model`) with
    /// `calendar` and `range` options, its arrows by `direction`, called in
    /// the setup that made it. The anchor starts at the start, else
    /// `anchor`, else today, and follows the start to each new day.
    #[must_use]
    #[track_caller]
    pub fn new(
        value: UseModel<P, DateRange>,
        calendar: impl Fn() -> CalendarOptions + 'static,
        range: impl Fn() -> RangeOptions + 'static,
        anchor: Option<CalendarDate>,
        direction: ReadDirection,
    ) -> Self {
        let calendar = Computed::new(calendar);
        let range = Computed::new(range);
        let first = value.get_untracked();
        let today = gui::untracked(|| calendar.with(|o| o.today));
        let pointed = Signal::labeled("range pointed", None);
        let this_value = value;
        // The grid blocks what the calendar blocks and what the range's
        // `maximum_days` does, so its keys skip those days too.
        let grid = CalendarGrid::new(
            move || {
                let mut o = calendar.get();
                let own = o.is_blocked.take();
                // Read when called (a cell's render, a key), so a new range or
                // rule reaches it; the options it sits in compare equal.
                o.is_blocked = Some(Derive::new(move |d: CalendarDate| {
                    own.as_ref().is_some_and(|f| f.call(d))
                        || range.with(|rule| this_value.with(|v| blocked_by_length(rule, *v, d)))
                }));
                o
            },
            first.start.or(anchor).unwrap_or(today),
            direction,
        );
        let state = Self {
            grid,
            value,
            calendar,
            range,
            pointed,
            pressed: Signal::labeled("range pressed", None),
            valid: Signal::labeled(
                "range last valid",
                if first.is_complete() { first } else { DateRange::EMPTY },
            ),
        };
        watch(
            move || value.get(),
            move |v, old| {
                if v.is_complete() {
                    state.valid.set(*v);
                }
                if let Some(start) = v.start
                    && old.start != Some(start)
                    && start != gui::untracked(|| grid.anchor())
                {
                    grid.set_anchor(start);
                }
            },
        );
        // Focus on a day points at it, as reka's `focusin` does.
        watch(
            move || grid.focus_handle().is_focused().then(|| grid.anchor()),
            move |at, _| {
                if let Some(day) = *at
                    && !gui::untracked(|| grid.blocks(day) || grid.unavailable(day))
                {
                    pointed.set(Some(day));
                }
            },
        );
        state
    }

    /// The range (tracked).
    #[must_use]
    pub fn value(self) -> DateRange {
        self.value.get()
    }

    /// The day the preview runs to (tracked).
    #[must_use]
    pub fn pointed(self) -> Option<CalendarDate> {
        self.pointed.get()
    }

    /// Point at `day` (the pointer over its cell), or at nothing (the
    /// pointer leaving the grid); a blocked or unavailable day is not
    /// pointed at.
    pub fn point(self, day: Option<CalendarDate>) {
        match day {
            Some(d) if gui::untracked(|| self.grid.blocks(d) || self.grid.unavailable(d)) => {}
            _ => self.pointed.set(day),
        }
    }

    /// Whether the days after `start` up to `end` (either order) may be one
    /// range: none blocked, none unavailable when it must be contiguous,
    /// highlightable ones excepted (`areAllDaysBetweenValid`). Tracked, as
    /// the predicates are ([`gui::Derive`]): a preview through a day the
    /// caller blocks since is drawn again.
    fn spans(self, a: CalendarDate, b: CalendarDate) -> bool {
        let (start, end) = (a.min(b), a.max(b));
        let rule = self.range.get();
        let span = start.days_until(end);
        (1..=span).map(|n| start.add_days(n)).all(|d| {
            rule.is_highlightable.as_ref().is_some_and(|f| f.call(d))
                || !(self.grid.blocks(d) || (!rule.non_contiguous && self.grid.unavailable(d)))
        })
    }

    fn grid_blocks(self, d: CalendarDate) -> bool {
        gui::untracked(|| self.grid.blocks(d))
    }

    /// The preview (`highlightedRange`, tracked): from the start to the
    /// pointed day while the range is open or an end is fixed.
    #[must_use]
    pub fn preview(self) -> Option<(CalendarDate, CalendarDate)> {
        let v = self.value.get();
        let rule = self.range.get();
        if v.is_complete() && rule.fixed.is_none() {
            return None;
        }
        let (start, pointed) = (v.start?, self.pointed.get()?);
        let (a, b) = (start.min(pointed), start.max(pointed));
        if a == b {
            return Some((a, b));
        }
        if let Some(n) = rule.maximum_days.filter(|_| v.end.is_none()) {
            let reach = i64::from(n.saturating_sub(1));
            return Some(if pointed >= start {
                (start, start.add_days(reach))
            } else {
                (start.add_days(reach.saturating_neg()), start)
            });
        }
        self.spans(a, b).then_some((a, b))
    }

    /// How `day` stands in the range and the preview (tracked).
    #[must_use]
    pub fn day(self, day: CalendarDate) -> RangeDay {
        let v = self.value.get();
        let contiguous = !self.range.with(|r| r.non_contiguous);
        let end = v.start == Some(day) || v.end == Some(day);
        let selected = v.contains(day) && (end || !(contiguous && self.grid.unavailable(day)));
        let preview = self.preview();
        RangeDay {
            selected,
            start: v.start == Some(day),
            end: v.end == Some(day),
            highlighted: preview.is_some_and(|(a, b)| (a..=b).contains(&day)),
            highlight_start: preview.is_some_and(|(a, _)| a == day),
            highlight_end: preview.is_some_and(|(_, b)| b == day),
        }
    }

    /// Whether the range is invalid (`data-invalid`, tracked): an end
    /// blocked, or the end before the start. Unavailable days do not make it
    /// invalid (unlike Calendar).
    #[must_use]
    pub fn invalid(self) -> bool {
        let v = self.value.get();
        v.start.is_some_and(|d| self.grid.blocks(d))
            || v.end.is_some_and(|d| self.grid.blocks(d))
            || matches!((v.start, v.end), (Some(s), Some(e)) if e < s)
    }

    /// Whether a range is being made: not whole, and not the last whole
    /// range (reka's `isEditing`, set by any change that leaves it so).
    #[must_use]
    pub fn editing(self) -> bool {
        let v = self.value.get_untracked();
        !v.is_complete() && v != self.valid.get_untracked()
    }

    /// Press `day`, as a click, Enter or Space does (the table in the
    /// module's docs).
    pub fn press(self, day: CalendarDate) {
        let (stop, prevent) = gui::untracked(|| {
            self.calendar.with(|o| {
                (
                    o.readonly || o.availability != Availability::Enabled || o.unavailable(day),
                    o.prevent_deselect,
                )
            })
        });
        if stop || self.grid_blocks(day) {
            return;
        }
        let v = self.value.get_untracked();
        let fixed = gui::untracked(|| self.range.with(|r| r.fixed));
        // No preview to the day (none pointed at, or the days between do not
        // make a range; the window of `maximum_days` is tested too).
        if let (Some(start), None) = (v.start, v.end)
            && (gui::untracked(|| self.preview()).is_none() || !self.spans(start, day))
        {
            if day == start && !prevent {
                self.grid.set_anchor(day);
                self.pressed.set(Some(day));
                self.value.set(DateRange::EMPTY);
            } else if day != start {
                if self.pressed.get_untracked() == Some(day) {
                    self.value.set(DateRange::started(day));
                }
                self.pressed.set(Some(day));
            }
            return;
        }
        self.pressed.set(Some(day));
        let next = match (v.start, v.end) {
            (Some(s), Some(e)) if s == e && s == day && !prevent => {
                self.grid.set_anchor(day);
                DateRange::EMPTY
            }
            (None, None) => DateRange::started(day),
            (None, Some(e)) => DateRange::between(e, day),
            (Some(s), None) => DateRange::between(s, day),
            (Some(s), Some(e)) => match fixed {
                None => DateRange::started(day),
                Some(RangeEnd::Start) if day < s => DateRange::of(Some(day), Some(e)),
                Some(RangeEnd::Start) => DateRange::of(Some(s), Some(day)),
                Some(RangeEnd::End) if day > e => DateRange::of(Some(s), Some(day)),
                Some(RangeEnd::End) => DateRange::of(Some(day), Some(e)),
            },
        };
        self.value.set(next);
    }

    /// Escape: a half-made range goes back to the last whole one; the key
    /// goes on to the page either way (false).
    pub fn escape(self, ev: &KeyEvent) -> bool {
        if ev.is(NamedKey::Escape) && self.editing() {
            self.value.set(self.valid.get_untracked());
        }
        false
    }

    /// The Tab stop's fallback when the anchor cannot take it
    /// (`RangeCalendarCellTrigger.vue:94-104`): the start, else the end,
    /// when not blocked; else, with nothing picked or both ends blocked, the
    /// first focusable day.
    fn fallback(self) -> Option<CalendarDate> {
        let v = self.value.get_untracked();
        let usable = |d: &CalendarDate| !self.grid_blocks(*d);
        if let Some(d) = v.start.filter(usable).or_else(|| v.end.filter(usable)) {
            return Some(d);
        }
        let none = v.start.is_none() && v.end.is_none();
        let blocked = match (v.start, v.end) {
            (Some(s), Some(e)) => self.grid_blocks(s) && self.grid_blocks(e),
            (Some(d), None) | (None, Some(d)) => self.grid_blocks(d),
            (None, None) => false,
        };
        (none || blocked).then(|| self.grid.first_focusable()).flatten()
    }

    /// Give initial focus: a picked end, else today, else the first day
    /// not blocked.
    pub fn focus_initial(self) {
        let v = self.value.get_untracked();
        if let Some(day) = self.grid.initial(v.start.into_iter().chain(v.end)) {
            self.grid.focus_day(day, false);
        }
    }

    /// `d` as the calendar's root (see [`CalendarGrid::root`]), taking
    /// Escape for a half-made range.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        self.grid.root(d).on_key_down(move |ev| self.escape(ev))
    }

    /// `d` as a month's grid; the pointer leaving it points at nothing.
    #[must_use]
    pub fn grid(self, d: Div) -> Div {
        self.grid.grid(d).on_hover(move |inside| {
            if !inside {
                self.point(None);
            }
        })
    }

    /// `d` as the cell of `day`: selected while in the range.
    #[must_use]
    pub fn cell(self, day: Day, d: Div) -> Div {
        let selected = self.day(day.date).selected;
        self.grid.cell(day, selected, d)
    }

    /// `d` as the trigger of `day`: a toggle button, pressed while in the
    /// range; it points at its day while under the pointer, and presses.
    #[must_use]
    pub fn trigger(self, day: Day, d: Div) -> Div {
        let month = day.month;
        let pressed = self.day(day.date).selected;
        self.grid
            .trigger(
                day,
                d.aria_checked(pressed),
                move || self.grid.is_stop(day.date, month, || self.fallback()),
                move |date| {
                    self.point(Some(date));
                    self.press(date);
                },
            )
            .on_hover(move |inside| {
                if inside {
                    self.point(Some(day.date));
                }
            })
    }
}

/// Whether `rule`'s `maximum_days` blocks `day` given `range`
/// (`useRangeCalendar.ts:74-99`).
fn blocked_by_length(rule: &RangeOptions, range: DateRange, day: CalendarDate) -> bool {
    let Some(n) = rule.maximum_days.map(i64::from) else {
        return false;
    };
    match (range.start, range.end) {
        (Some(s), Some(e)) => {
            if rule.fixed.is_none() {
                return false;
            }
            // Room left for the range to grow by, either way, staying within N days.
            let left = n.saturating_sub(s.days_until(e).saturating_add(1));
            left >= 0 && !(s.add_days(left.saturating_neg())..=e.add_days(left)).contains(&day)
        }
        (Some(s), None) => s.days_until(day).abs() >= n,
        _ => false,
    }
}
