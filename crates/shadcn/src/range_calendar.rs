//! Range Calendar (`ui/range-calendar`): one or more months to pick a start
//! and an end day from. The first click starts a range, the second closes it
//! (in either order), a third starts a new one, as reka-ui's
//! `RangeCalendarRoot` does. Dates, paging and keys come from `calendar`.

use gui::prelude::*;
use gui::{
    Bind, Div, Element, IntoElement, Model, Radius, UseModel, component, div, provide, text, use_context, use_model,
};

use crate::calendar::{
    CalendarDate, CalendarOptions, CalendarView, DaySelection, HasOptions, day_trigger, month_grid, page_button,
};
use crate::part::{Part, PartProps};
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

/// What [`range_calendar`] is given.
#[derive(Clone)]
pub struct RangeCalendarProps {
    value: Bind<RangeCalendarValue>,
    options: CalendarOptions,
}

impl HasOptions for RangeCalendarProps {
    fn options_mut(&mut self) -> &mut CalendarOptions {
        &mut self.options
    }
}

/// What a range calendar's days read.
#[derive(Clone)]
struct RangeCalendarContext {
    view: CalendarView,
    value: UseModel<PartProps<RangeCalendarProps>, RangeCalendarValue>,
}

/// A range calendar: its heading between the paging arrows, then one grid
/// per month. It keeps its view and, unless given `.model(…)`, the range,
/// starting at `default_value`; the options are [`crate::calendar`]'s.
#[must_use]
pub fn range_calendar() -> Part<RangeCalendarProps> {
    let props = RangeCalendarProps {
        value: Bind::default(),
        options: CalendarOptions::default(),
    };
    Part::new(props, div().named("range-calendar").p(3.0), |p| {
        component(p, |cx| {
            let props = cx.props();
            let value = use_model(props, |p| &p.props.value);
            let today = props.with_untracked(|p| p.props.options.today);
            let start = value.get_untracked().start().unwrap_or(today);
            let view = CalendarView::new(move || props.with(|p| p.props.options), start);
            provide(RangeCalendarContext { view, value });
            move || {
                props
                    .with(|p| p.slot.0.clone())
                    .child(
                        range_calendar_header()
                            .child(range_calendar_heading(view.heading()))
                            .child(page_button(view, -1).absolute().left(1.0))
                            .child(page_button(view, 1).absolute().right(1.0)),
                    )
                    .child(view.months_row(|month| {
                        month_grid(view, month, range_calendar_head_cell, |day, col| {
                            range_day(day, month, col)
                        })
                    }))
                    .into_element()
            }
        })
    })
}

impl Part<RangeCalendarProps> {
    /// The range, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<RangeCalendarValue>>) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_model(model))
    }

    /// The range shown, kept by the caller as a plain value (`:model-value`):
    /// a pick only calls `on_change`.
    #[must_use]
    pub fn value(self, value: RangeCalendarValue) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_value(value))
    }

    /// The range first, kept by the calendar (`default-value`).
    #[must_use]
    pub fn default_value(self, value: RangeCalendarValue) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_default(value))
    }

    /// Called with each value picked (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(RangeCalendarValue) + 'static) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).on_change(f))
    }
}

/// One day of the grid at column `col`: its cell and trigger, an instance
/// of its own. The first click starts a range, the next closes it.
fn range_day(day: CalendarDate, month: CalendarDate, col: usize) -> Element {
    component((day, month, col), |cx| {
        let props = cx.props();
        let ctx = use_context::<RangeCalendarContext>();
        move || {
            let (day, month, col) = props.get();
            let Some(ctx) = &ctx else {
                return Element::Empty;
            };
            let range = ctx.value.get();
            let inside = ctx.view.in_month(day, month);
            let selection = if !inside || !range.contains(day) {
                DaySelection::Unselected
            } else if range.start() == Some(day) || range.end() == Some(day) {
                DaySelection::Picked
            } else {
                DaySelection::Between
            };
            let value = ctx.value;
            range_calendar_cell(
                inside && range.contains(day),
                col == 0 || range.start() == Some(day),
                col == 6 || range.end() == Some(day),
            )
            .child(
                day_trigger(ctx.view, day, month, selection, move |d| {
                    value.update(|v| *v = v.pick(d))
                })
                .named("range-calendar-trigger"),
            )
            .into_element()
        }
    })
    .into_element()
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
