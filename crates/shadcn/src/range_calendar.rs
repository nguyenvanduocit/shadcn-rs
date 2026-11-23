//! Range Calendar (`ui/range-calendar`): one or more months to pick a start
//! and an end day from. How presses build the range, the preview, the
//! keys, Escape and what assistive technology is told are
//! `primitives::range_calendar`'s; this file draws it, with the day look,
//! arrows and dates of `calendar`.

use gui::prelude::*;
use gui::{Bind, Computed, Direction, Div, Element, IntoElement, Model, component, div, provide, text};
use primitives::context::use_root;
use primitives::direction::use_direction;
use primitives::range_calendar::RangeCalendar;
pub use primitives::range_calendar::{DateRange, RangeDay, RangeEnd, RangeOptions};

use crate::calendar::{
    CalendarDate, CalendarLook, CalendarOptions, DaySelection, HasOptions, day_trigger, month_grid, months_row,
    page_button,
};
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`range_calendar`] is given.
#[derive(Clone)]
pub struct RangeCalendarProps {
    value: Bind<DateRange>,
    options: CalendarOptions,
    range: RangeOptions,
    look: CalendarLook,
}

impl HasOptions for RangeCalendarProps {
    fn options_mut(&mut self) -> &mut CalendarOptions {
        &mut self.options
    }

    fn look_mut(&mut self) -> &mut CalendarLook {
        &mut self.look
    }
}

type Ranger = RangeCalendar<PartProps<RangeCalendarProps>>;

/// What a range calendar's days read.
#[derive(Clone)]
struct RangeCalendarContext {
    calendar: Ranger,
    look: Computed<CalendarLook>,
}

/// A range calendar: its heading between the paging arrows, then one grid
/// per month. It keeps its view and, unless given `.model(…)`, the range,
/// starting at `default_value`; the options are [`crate::calendar`]'s.
#[must_use]
pub fn range_calendar() -> Part<RangeCalendarProps> {
    let props = RangeCalendarProps {
        value: Bind::default(),
        options: CalendarOptions::default(),
        range: RangeOptions::default(),
        look: CalendarLook::default(),
    };
    Part::new(props, div().named("range-calendar").padding(12.0), |p| {
        component(p, |cx| {
            let props = cx.props();
            let value = gui::use_model(props, |p| &p.props.value);
            let rtl = props.with_untracked(|p| p.props.look.rtl);
            let direction = use_direction(Some(if rtl { Direction::Rtl } else { Direction::Ltr }));
            let calendar = RangeCalendar::new(
                value,
                move || props.with(|p| p.props.options.clone()),
                move || props.with(|p| p.props.range.clone()),
                None,
                direction,
            );
            let look = Computed::new(move || props.with(|p| p.props.look));
            provide(RangeCalendarContext { calendar, look });
            move || {
                let grid = calendar.grid;
                calendar
                    .root(
                        props
                            .with(|p| p.slot.0.clone())
                            .child(
                                range_calendar_header()
                                    .child(range_calendar_heading(grid.heading()))
                                    .child(page_button(grid, rtl, -1).position(Position::Absolute).left(4.0))
                                    .child(page_button(grid, rtl, 1).position(Position::Absolute).right(4.0)),
                            )
                            .child(months_row(grid, rtl, |month| {
                                calendar.grid(month_grid(grid, rtl, month, range_calendar_head_cell, |day, col| {
                                    range_day(day, month, col)
                                }))
                            })),
                    )
                    .into_element()
            }
        })
    })
}

impl Part<RangeCalendarProps> {
    /// The range, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<DateRange>>) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_model(model))
    }

    /// The range shown, kept by the caller as a plain value (`:model-value`):
    /// a pick only calls `on_change`.
    #[must_use]
    pub fn value(self, value: DateRange) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_value(value))
    }

    /// The range first, kept by the calendar (`default-value`).
    #[must_use]
    pub fn default_value(self, value: DateRange) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_default(value))
    }

    /// Called with each value picked (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Into<gui::Callback<DateRange>>) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).on_change(f))
    }

    /// `allowNonContiguousRanges`: a range may run over unavailable days.
    #[must_use]
    pub fn allow_non_contiguous_ranges(self) -> Self {
        self.with_props(|p| p.range.non_contiguous = true)
    }

    /// `maximumDays`: the longest range, its ends included.
    #[must_use]
    pub fn maximum_days(self, days: u32) -> Self {
        self.with_props(|p| p.range.maximum_days = Some(days))
    }

    /// `fixedDate`: which end a press does not move by default.
    #[must_use]
    pub fn fixed_date(self, end: RangeEnd) -> Self {
        self.with_props(|p| p.range.fixed = Some(end))
    }
}

/// One day of the grid at column `col`: its cell and trigger, an instance
/// of its own.
fn range_day(day: CalendarDate, month: CalendarDate, col: usize) -> Element {
    component((day, month, col), |cx| {
        let props = cx.props();
        let ctx = use_root::<RangeCalendarContext>("range calendar day", "range_calendar");
        move || {
            let (date, month, col) = props.get();
            let Some(ctx) = &ctx else {
                return Element::Empty;
            };
            let calendar = ctx.calendar;
            let day = calendar.grid.day(date, month);
            let range = calendar.day(date);
            let shown = !day.outside_month && range.selected;
            let selection = if !shown {
                DaySelection::Unselected
            } else if range.start || range.end {
                DaySelection::Picked
            } else {
                DaySelection::Between
            };
            calendar
                .cell(
                    day,
                    range_calendar_cell(shown, col == 0 || range.start, col == 6 || range.end),
                )
                .child(calendar.trigger(
                    day,
                    day_trigger(calendar.grid, ctx.look.get(), day, selection).named("range-calendar-trigger"),
                ))
                .into_element()
        }
    })
    .into_element()
}

#[must_use]
pub fn range_calendar_header() -> Div {
    div()
        .named("range-calendar-header")
        .display(Display::Flex)
        .justify_content(Justify::Center)
        .padding_top(4.0)
        .position(Position::Relative)
        .align_items(Align::Center)
        .width(pct(100.0))
}

#[must_use]
pub fn range_calendar_heading(heading: impl Into<String>) -> impl IntoElement {
    text(heading)
        .named("range-calendar-heading")
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500)
}

/// A weekday's name over its column (`w-8 text-[0.8rem]`).
#[must_use]
pub fn range_calendar_head_cell(name: &'static str) -> Div {
    let t = theme();
    div()
        .named("range-calendar-head-cell")
        .width(32.0)
        .display(Display::Flex)
        .justify_content(Justify::Center)
        .radius(8.0)
        .font_weight(400)
        .text_size(12.8)
        .line_height(19.2)
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
        .position(Position::Relative)
        .padding(0.0)
        .text_size(14.0)
        .line_height(20.0)
        .when(selected, |d| {
            d.background(t.accent)
                .when(round_left, |d| d.radius_left(8.0))
                .when(round_right, |d| d.radius_right(8.0))
        })
}
