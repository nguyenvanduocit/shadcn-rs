//! Range Calendar (`ui/range-calendar`): one or more months to pick a start
//! and an end day from. The first click starts a range, the second closes it
//! (in either order), a third starts a new one, as reka-ui's
//! `RangeCalendarRoot` does. Dates, paging and keys come from `calendar`.

use gui::prelude::*;
use gui::{Div, IntoElement, Radius, Signal, div, text};

use crate::calendar::{CalendarDate, CalendarView, DaySelection, day_trigger, month_grid, page_button};
use crate::theme::theme;

/// The picked range (`v-model`): nothing, a start waiting for its end, or both
/// ends with `start <= end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RangeCalendarValue {
    #[default]
    Empty,
    Started(CalendarDate),
    Range {
        start: CalendarDate,
        end: CalendarDate,
    },
}

impl RangeCalendarValue {
    /// The range from `a` to `b`, in whichever order they come.
    #[must_use]
    pub fn between(a: CalendarDate, b: CalendarDate) -> Self {
        Self::Range {
            start: a.min(b),
            end: a.max(b),
        }
    }

    /// The value after a click on `day`.
    #[must_use]
    pub fn pick(self, day: CalendarDate) -> Self {
        match self {
            Self::Started(start) => Self::between(start, day),
            Self::Empty | Self::Range { .. } => Self::Started(day),
        }
    }

    #[must_use]
    pub fn start(self) -> Option<CalendarDate> {
        match self {
            Self::Empty => None,
            Self::Started(start) | Self::Range { start, .. } => Some(start),
        }
    }

    #[must_use]
    pub fn end(self) -> Option<CalendarDate> {
        match self {
            Self::Range { end, .. } => Some(end),
            Self::Empty | Self::Started(_) => None,
        }
    }

    /// Whether `day` is inside the range, its ends included.
    #[must_use]
    pub fn contains(self, day: CalendarDate) -> bool {
        match self {
            Self::Empty => false,
            Self::Started(start) => day == start,
            Self::Range { start, end } => (start..=end).contains(&day),
        }
    }
}

/// The value and view of one range calendar. Created once, outside the render closure.
#[derive(Clone)]
pub struct RangeCalendarState {
    pub value: Signal<RangeCalendarValue>,
    pub view: CalendarView,
}

impl RangeCalendarState {
    /// Showing the month of the range's start, or of today when it is empty.
    #[must_use]
    pub fn new(today: CalendarDate, value: RangeCalendarValue) -> Self {
        Self {
            value: Signal::labeled("range calendar value", value),
            view: CalendarView::new(today, value.start().unwrap_or(today)),
        }
    }

    /// `numberOfMonths`: how many months show side by side.
    #[must_use]
    pub fn number_of_months(mut self, months: u8) -> Self {
        self.view = self.view.number_of_months(months);
        self
    }

    /// `disableDaysOutsideCurrentView`: the padding days of other months take no clicks.
    #[must_use]
    pub fn disable_days_outside_current_view(mut self) -> Self {
        self.view = self.view.disable_days_outside_current_view();
        self
    }
}

/// The whole range calendar: heading between the paging arrows, then one grid per month.
#[must_use]
pub fn range_calendar(state: &RangeCalendarState) -> Div {
    let view = &state.view;
    div()
        .named("range-calendar")
        .p(3.0)
        .child(
            range_calendar_header()
                .child(range_calendar_heading(view.heading()))
                .child(range_calendar_prev_button(view))
                .child(range_calendar_next_button(view)),
        )
        .child(view.months_row(|month| {
            month_grid(view, month, range_calendar_head_cell, |day, col| {
                let value = state.value.get();
                let selected = view.in_month(day, month) && value.contains(day);
                range_calendar_cell(
                    selected,
                    col == 0 || value.start() == Some(day),
                    col == 6 || value.end() == Some(day),
                )
                .child(range_calendar_cell_trigger(state, day, month))
            })
        }))
}

#[must_use]
pub fn range_calendar_header() -> Div {
    div()
        .named("range-calendar-header")
        .flex()
        .justify_center()
        .pt(1.0)
        .relative()
        .items_center()
        .w_full()
}

#[must_use]
pub fn range_calendar_heading(heading: impl Into<String>) -> impl IntoElement {
    text(heading).named("range-calendar-heading").text_sm().font_medium()
}

#[must_use]
pub fn range_calendar_prev_button(view: &CalendarView) -> Div {
    page_button(view, -1).absolute().left(1.0)
}

#[must_use]
pub fn range_calendar_next_button(view: &CalendarView) -> Div {
    page_button(view, 1).absolute().right(1.0)
}

/// A weekday's name over its column (`w-8 text-[0.8rem]`).
#[must_use]
pub fn range_calendar_head_cell(name: &'static str) -> Div {
    let t = theme();
    div()
        .named("range-calendar-head-cell")
        .w(8.0)
        .flex()
        .justify_center()
        .rounded_md()
        .font_normal()
        .text_size(12.8, 19.2)
        .text_color(t.muted_foreground)
        .child(name)
}

/// A day's cell: the accent band of a selected range runs through it,
/// rounded at the range's ends and at the week's edges.
#[must_use]
pub fn range_calendar_cell(selected: bool, round_left: bool, round_right: bool) -> Div {
    let t = theme();
    div()
        .named("range-calendar-cell")
        .relative()
        .p(0.0)
        .text_sm()
        .when(selected, |d| {
            d.bg(t.accent)
                .when(round_left, |d| d.rounded_l(Radius::MD))
                .when(round_right, |d| d.rounded_r(Radius::MD))
        })
}

/// `day`'s button in `month`'s grid: starts or closes the range on click.
#[must_use]
pub fn range_calendar_cell_trigger(state: &RangeCalendarState, day: CalendarDate, month: CalendarDate) -> Div {
    let value = state.value.clone();
    let range = value.get();
    let outside = !state.view.in_month(day, month);
    let selection = if outside || !range.contains(day) {
        DaySelection::Unselected
    } else if range.start() == Some(day) || range.end() == Some(day) {
        DaySelection::Picked
    } else {
        DaySelection::Between
    };
    day_trigger(&state.view, day, month, selection, move |d| value.update(|v| v.pick(d)))
        .named("range-calendar-trigger")
}
