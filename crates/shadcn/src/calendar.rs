//! Calendar (`ui/calendar`): a month of days to pick one date from, paged with
//! the header's arrows and walked with the keyboard as reka-ui's
//! `CalendarRoot` does. The date type, the view (which months show, in which
//! calendar system and locale, which day holds the grid's tab stop) and the
//! day trigger are shared with `range_calendar`.
//!
//! [`calendar`] is an instance keeping its view and, unless the caller
//! passes `.model(…)` (reka's `v-model`), the picked day, starting at
//! `default_value`. Each day is an instance of its own, so a pick renders
//! the two days it changes.

use std::num::Saturating;
use std::time::{SystemTime, UNIX_EPOCH};

use gui::prelude::*;
use gui::{
    Bind, Computed, Div, Element, FocusHandle, Icon, KeyEvent, Length, Model, NamedKey, Signal, UseModel, component,
    div, icon, on_mounted, provide, text, use_context, use_model, watch,
};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::calendar_locale::{CalendarLocale, MonthWidth, WeekdayFormat};
use crate::calendar_system::CalendarSystem;
use crate::native_select::{NativeSelectEntry, native_select, native_select_option};
use crate::part::{Part, PartProps};
use crate::theme::theme;

const FIRST_YEAR: i32 = 1;
const LAST_YEAR: i32 = 9999;

/// A day of the proleptic Gregorian calendar, in years 1 to 9999.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CalendarDate {
    year: i32,
    month: u8,
    day: u8,
}

impl CalendarDate {
    /// `None` when that day does not exist or its year is outside 1..=9999.
    #[must_use]
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        let valid = (FIRST_YEAR..=LAST_YEAR).contains(&year)
            && (1..=12).contains(&month)
            && (1..=CalendarSystem::Gregorian.days_in_month(year, month)).contains(&day);
        valid.then_some(Self { year, month, day })
    }

    /// Today where the machine is: the day it is now in the system's time
    /// zone (`TZ` when set, else the OS's), as reka's
    /// `today(getLocalTimeZone())` is the day in the browser's.
    #[must_use]
    pub fn today() -> Self {
        Self::in_zone(SystemTime::now(), &jiff::tz::TimeZone::system())
    }

    /// The day `instant` falls on in the IANA time zone `zone` ("UTC",
    /// "Asia/Tehran"); `None` when the time zone database has no such zone.
    #[must_use]
    pub fn at(instant: SystemTime, zone: &str) -> Option<Self> {
        jiff::tz::TimeZone::get(zone).ok().map(|tz| Self::in_zone(instant, &tz))
    }

    /// The day `instant` falls on in `tz`, clamped into years 1..=9999.
    fn in_zone(instant: SystemTime, tz: &jiff::tz::TimeZone) -> Self {
        let Ok(at) = jiff::Timestamp::try_from(instant) else {
            let after = instant.duration_since(UNIX_EPOCH).is_ok();
            return Self::from_days(if after { i64::MAX } else { i64::MIN });
        };
        let date = tz.to_datetime(at).date();
        Self::new(
            i32::from(date.year()),
            date.month().unsigned_abs(),
            date.day().unsigned_abs(),
        )
        .unwrap_or_else(|| Self::from_days(if date.year() < 1 { i64::MIN } else { i64::MAX }))
    }

    #[must_use]
    pub fn year(self) -> i32 {
        self.year
    }

    /// 1 for January.
    #[must_use]
    pub fn month(self) -> u8 {
        self.month
    }

    #[must_use]
    pub fn day(self) -> u8 {
        self.day
    }

    /// The month's English name.
    #[must_use]
    pub fn month_name(self) -> &'static str {
        CalendarLocale::EnUs.month(CalendarSystem::Gregorian, self, MonthWidth::Long)
    }

    /// "Nov": the name `Intl`'s `{ month: 'short' }` gives in English.
    #[must_use]
    pub fn month_short_name(self) -> &'static str {
        CalendarLocale::EnUs.month(CalendarSystem::Gregorian, self, MonthWidth::Short)
    }

    /// 0 for Sunday.
    #[must_use]
    pub fn weekday(self) -> u8 {
        u8::try_from(self.days().saturating_add(4).rem_euclid(7)).unwrap_or(0)
    }

    /// `n` days later (earlier when negative), held inside years 1..=9999.
    #[must_use]
    pub fn add_days(self, n: i64) -> Self {
        Self::from_days(self.days().saturating_add(n))
    }

    /// `n` months later, its day cut to the length of that month.
    #[must_use]
    pub fn add_months(self, n: i64) -> Self {
        CalendarSystem::Gregorian.add_months(self, n)
    }

    #[must_use]
    pub fn start_of_month(self) -> Self {
        CalendarSystem::Gregorian.start_of_month(self)
    }

    #[must_use]
    pub fn end_of_month(self) -> Self {
        CalendarSystem::Gregorian.end_of_month(self)
    }

    /// Whether `other` falls in the same month of the same year.
    #[must_use]
    pub fn same_month(self, other: Self) -> bool {
        CalendarSystem::Gregorian.same_month(self, other)
    }

    /// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`). Every
    /// intermediate is non-negative because years start at 1.
    pub(crate) fn days(self) -> i64 {
        let y = s(i64::from(self.year)) - s(i64::from(self.month <= 2));
        let era = quot(y, 400);
        let yoe = y - era * s(400);
        let mp = rem(s(i64::from(self.month)) + s(9), 12);
        let doy = quot(s(153) * mp + s(2), 5) + s(i64::from(self.day)) - s(1);
        let doe = yoe * s(365) + quot(yoe, 4) - quot(yoe, 100) + doy;
        (era * s(146_097) + doe - s(719_468)).0
    }

    /// The day `days` after 1970-01-01 (`civil_from_days`), clamped into years 1..=9999.
    pub(crate) fn from_days(days: i64) -> Self {
        let first = Self {
            year: FIRST_YEAR,
            month: 1,
            day: 1,
        }
        .days();
        let last = Self {
            year: LAST_YEAR,
            month: 12,
            day: 31,
        }
        .days();
        let z = s(days.clamp(first, last)) + s(719_468);
        let era = quot(z, 146_097);
        let doe = z - era * s(146_097);
        let yoe = quot(doe - quot(doe, 1460) + quot(doe, 36_524) - quot(doe, 146_096), 365);
        let doy = doe - (s(365) * yoe + quot(yoe, 4) - quot(yoe, 100));
        let mp = quot(s(5) * doy + s(2), 153);
        let day = doy - quot(s(153) * mp + s(2), 5) + s(1);
        let month = if mp < s(10) { mp + s(3) } else { mp - s(9) };
        let year = yoe + era * s(400) + s(i64::from(month <= s(2)));
        Self {
            year: i32::try_from(year.0).unwrap_or(LAST_YEAR),
            month: u8::try_from(month.0).unwrap_or(1),
            day: u8::try_from(day.0).unwrap_or(1),
        }
    }
}

/// The civil-date arithmetic runs on non-negative day counts far from `i64`'s
/// ends: it saturates rather than overflow, and divides only by the positive
/// constants of the algorithm.
pub(crate) fn s(v: i64) -> Saturating<i64> {
    Saturating(v)
}

pub(crate) fn quot(a: Saturating<i64>, by: i64) -> Saturating<i64> {
    Saturating(a.0.checked_div_euclid(by).unwrap_or_default())
}

pub(crate) fn rem(a: Saturating<i64>, by: i64) -> Saturating<i64> {
    Saturating(a.0.checked_rem_euclid(by).unwrap_or_default())
}

/// The weeks a month's grid shows, starting on `first_weekday` (0 for
/// Sunday), padded with the days of the months around it (reka-ui's grid
/// without `fixedWeeks`). `month` is any day of the month, in `system`.
#[must_use]
pub fn calendar_weeks(system: CalendarSystem, first_weekday: u8, month: CalendarDate) -> Vec<Vec<CalendarDate>> {
    let first = system.start_of_month(month);
    let last = system.end_of_month(month);
    let lead = i64::from(first.weekday())
        .saturating_sub(i64::from(first_weekday))
        .rem_euclid(7);
    let mut start = first.add_days(lead.saturating_neg());
    let mut weeks = Vec::new();
    while start <= last {
        weeks.push((0..7).map(|d| start.add_days(d)).collect());
        start = start.add_days(7);
    }
    weeks
}

/// What both calendars are told about their view: reka-ui's `numberOfMonths`,
/// `minValue`, `maxValue`, `weekdayFormat`, `disableDaysOutsideCurrentView`,
/// `dir`, `locale` and the placeholder's calendar, with the day counted as
/// today and the day triggers' size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalendarOptions {
    pub today: CalendarDate,
    pub months: u8,
    pub min: Option<CalendarDate>,
    pub max: Option<CalendarDate>,
    pub weekday_format: WeekdayFormat,
    /// A day trigger's side, in spacing units (`size-8`).
    pub cell_size: f32,
    pub disable_outside_view: bool,
    /// `dir="rtl"`: rows run right to left, the arrows and arrow keys mirrored.
    pub rtl: bool,
    /// The calendar the months are counted in (the placeholder's calendar).
    pub system: CalendarSystem,
    /// What the names, digits, week and direction are read in (`locale`).
    pub locale: CalendarLocale,
}

impl Default for CalendarOptions {
    fn default() -> Self {
        Self {
            today: CalendarDate::today(),
            months: 1,
            min: None,
            max: None,
            weekday_format: WeekdayFormat::Narrow,
            cell_size: 8.0,
            disable_outside_view: false,
            rtl: false,
            system: CalendarSystem::Gregorian,
            locale: CalendarLocale::EnUs,
        }
    }
}

/// The props of a calendar ([`calendar`], [`crate::range_calendar`]): what
/// their shared builder methods set.
pub trait HasOptions {
    fn options_mut(&mut self) -> &mut CalendarOptions;
}

/// Which months show and which day holds the grid's one tab stop: the state
/// a calendar's instance keeps, shared with its days.
#[derive(Clone, Copy)]
pub(crate) struct CalendarView {
    /// The first month shown (reka-ui's `placeholder`), as its first day in the system.
    month: Signal<CalendarDate>,
    /// The day the arrow keys move and the grid's tab stop sits on.
    focused: Signal<CalendarDate>,
    focus: FocusHandle,
    options: Computed<CalendarOptions>,
}

impl CalendarView {
    /// In a calendar's setup: the view of `options`, on the month of
    /// `focused`. A new calendar system or locale goes back to today.
    #[track_caller]
    pub(crate) fn new(options: impl Fn() -> CalendarOptions + 'static, focused: CalendarDate) -> Self {
        let options = Computed::new(options);
        let system = options.get_untracked().system;
        let view = Self {
            month: Signal::labeled("calendar month", system.start_of_month(focused)),
            focused: Signal::labeled("calendar focused", focused),
            focus: FocusHandle::new(),
            options,
        };
        let _calendar = watch(
            move || options.with(|o| (o.system, o.locale)),
            move |(system, _), _| {
                let today = options.get_untracked().today;
                view.month.set(system.start_of_month(today));
                view.focused.set(today);
            },
        );
        view
    }

    fn opt(&self) -> CalendarOptions {
        self.options.get()
    }

    pub(crate) fn rtl(&self) -> bool {
        self.opt().rtl
    }

    /// The first day of each month shown.
    pub(crate) fn shown(&self) -> Vec<CalendarDate> {
        let o = self.opt();
        let first = self.month.get();
        (0..i64::from(o.months))
            .map(|i| o.system.add_months(first, i))
            .collect()
    }

    /// Whether `day` is a day of the month `month` starts.
    pub(crate) fn in_month(&self, day: CalendarDate, month: CalendarDate) -> bool {
        self.opt().system.same_month(day, month)
    }

    /// `day`'s number as the grid shows it.
    fn day_label(&self, day: CalendarDate) -> String {
        let o = self.opt();
        o.locale.number(i64::from(o.system.fields(day).day))
    }

    /// "November 2026", or "November - December 2026" over several months.
    pub(crate) fn heading(&self) -> String {
        let o = self.opt();
        let (system, locale) = (o.system, o.locale);
        let first = self.month.get();
        let last = system.add_months(first, i64::from(o.months.saturating_sub(1)));
        if first == last {
            return locale.month_and_year(system, first);
        }
        let month = |d| locale.month(system, d, MonthWidth::Long);
        let (first_year, last_year) = (locale.year(system, first), locale.year(system, last));
        if first_year == last_year {
            format!("{} - {} {last_year}", month(first), month(last))
        } else {
            format!("{} {first_year} - {} {last_year}", month(first), month(last))
        }
    }

    /// Outside `minValue..=maxValue`, or outside `month` when those are disabled.
    fn is_disabled(&self, date: CalendarDate, month: CalendarDate) -> bool {
        let o = self.opt();
        o.min.is_some_and(|m| date < m)
            || o.max.is_some_and(|m| date > m)
            || (o.disable_outside_view && !self.in_month(date, month))
    }

    /// The last day shown when the first month shown starts on `first`.
    fn last_shown(&self, first: CalendarDate) -> CalendarDate {
        let o = self.opt();
        o.system
            .end_of_month(o.system.add_months(first, i64::from(o.months.saturating_sub(1))))
    }

    /// Whether paging `step` pages (of `numberOfMonths`) still shows a day in bounds.
    fn can_page(&self, step: i64) -> bool {
        let o = self.opt();
        let first = o
            .system
            .add_months(self.month.get(), step.saturating_mul(i64::from(o.months)));
        let last = self.last_shown(first);
        !(o.max.is_some_and(|m| first > m) || o.min.is_some_and(|m| last < m))
    }

    /// Show the months `step` pages away; the focused day moves with them.
    fn page(&self, step: i64) {
        self.shift(step.saturating_mul(i64::from(self.opt().months)));
    }

    /// Show the months `months` later (earlier when negative); the focused
    /// day moves with them, as reka-ui's `placeholder` does.
    fn shift(&self, months: i64) {
        let system = self.opt().system;
        self.month.update(|m| *m = system.add_months(*m, months));
        self.focused.update(|d| *d = system.add_months(*d, months));
    }

    /// Show `year`'s month of the same name (the year select); the focused day moves with it.
    fn set_year(&self, year: i32) {
        let system = self.opt().system;
        self.month.update(|m| *m = system.with_year(*m, year));
        self.focused.update(|d| *d = system.with_year(*d, year));
    }

    /// Show `month` of the shown year (the month select); the focused day moves with it.
    fn set_month(&self, month: u8) {
        let system = self.opt().system;
        let by = i64::from(month).saturating_sub(i64::from(system.fields(self.month.get_untracked()).month));
        self.shift(by);
    }

    /// Move the tab stop to `date` and bring its month into view.
    pub(crate) fn reveal(&self, date: CalendarDate) {
        let o = self.opt();
        let date = o.min.map_or(date, |m| date.max(m));
        let date = o.max.map_or(date, |m| date.min(m));
        let first = self.month.get_untracked();
        if date < first {
            self.month.set(o.system.start_of_month(date));
        } else if date > self.last_shown(first) {
            self.month.set(o.system.add_months(
                o.system.start_of_month(date),
                i64::from(o.months.saturating_sub(1)).saturating_neg(),
            ));
        }
        self.focused.set(date);
    }

    /// Move the tab stop to `date`, bring its month into view and focus it.
    fn focus_day(&self, date: CalendarDate, by_keyboard: bool) {
        self.reveal(date);
        gui::focus::set_focus(Some(self.focus.id()), by_keyboard);
    }

    /// The grid's keys: arrows by day and week (mirrored right to left),
    /// Home and End to the week's ends, Page Up and Down by month (by year
    /// with Shift).
    fn on_key(&self, e: &KeyEvent) -> bool {
        let o = self.opt();
        let d = self.focused.get_untracked();
        let page: i64 = if e.modifiers.shift { 12 } else { 1 };
        let forward: i64 = if o.rtl { -1 } else { 1 };
        let into_week = i64::from(d.weekday())
            .saturating_sub(i64::from(o.locale.first_weekday()))
            .rem_euclid(7);
        let to = if e.is(NamedKey::ArrowLeft) {
            d.add_days(forward.saturating_neg())
        } else if e.is(NamedKey::ArrowRight) {
            d.add_days(forward)
        } else if e.is(NamedKey::ArrowUp) {
            d.add_days(-7)
        } else if e.is(NamedKey::ArrowDown) {
            d.add_days(7)
        } else if e.is(NamedKey::Home) {
            d.add_days(into_week.saturating_neg())
        } else if e.is(NamedKey::End) {
            d.add_days(6_i64.saturating_sub(into_week))
        } else if e.is(NamedKey::PageUp) {
            o.system.add_months(d, page.saturating_neg())
        } else if e.is(NamedKey::PageDown) {
            o.system.add_months(d, page)
        } else {
            return false;
        };
        self.focus_day(to, true);
        true
    }

    /// A row laid out in the locale's direction.
    pub(crate) fn row(&self) -> Div {
        if self.rtl() {
            div().flex_row_reverse()
        } else {
            div().flex_row()
        }
    }

    /// The months' grids side by side, walked with [`Self::on_key`].
    pub(crate) fn months_row(&self, grid: impl Fn(CalendarDate) -> Div) -> Div {
        let view = *self;
        self.row()
            .mt(4.0)
            .gap_x(4.0)
            .on_key_down(move |e| view.on_key(e))
            .children(self.shown().into_iter().map(grid))
    }
}

/// How a day stands in the selection; both calendars draw it the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DaySelection {
    Unselected,
    /// Inside a range, between its ends: the cell carries the colour.
    Between,
    /// The picked day, or an end of a range.
    Picked,
}

/// The day button both calendars put in a cell (`CalendarCellTrigger`).
pub(crate) fn day_trigger(
    view: CalendarView,
    day: CalendarDate,
    month: CalendarDate,
    selection: DaySelection,
    on_pick: impl Fn(CalendarDate) + 'static,
) -> Div {
    let t = theme();
    let outside = !view.in_month(day, month);
    let disabled = view.is_disabled(day, month);
    let today = view.opt().today;
    button(ButtonVariant::Ghost, ButtonSize::IconSm)
        .named("calendar-cell-trigger")
        .size(view.opt().cell_size)
        .p(0.0)
        .font_normal()
        .cursor_default()
        .when(day == today && selection == DaySelection::Unselected, |b| {
            b.bg(t.accent).text_color(t.accent_foreground)
        })
        .when(outside, |b| b.text_color(t.muted_foreground))
        .when(selection == DaySelection::Picked, |b| {
            b.bg(t.primary)
                .text_color(t.primary_foreground)
                .hover(move |s| s.bg(t.primary).text_color(t.primary_foreground))
        })
        .when(disabled, |b| b.text_color(t.muted_foreground).disabled(true))
        .when(!outside && day == view.focused.get(), |b| b.track_focus(view.focus))
        .on_click(move |_| {
            on_pick(day);
            view.focus_day(day, false);
        })
        .child(view.day_label(day))
}

/// The prev and next arrows both calendars use: an outline icon button,
/// see-through until hovered, its chevron pointing the way it pages.
pub(crate) fn page_button(view: CalendarView, step: i64) -> Div {
    let t = theme();
    let back = (step < 0) != view.rtl();
    button(ButtonVariant::Outline, ButtonSize::Default)
        .named(if step < 0 {
            "calendar-prev-button"
        } else {
            "calendar-next-button"
        })
        .size(7.0)
        .p(0.0)
        .when(!t.dark, |b| b.bg(gui::Color::TRANSPARENT))
        .opacity(0.5)
        .hover(move |s| {
            let bg = if t.dark { t.input.opacity(0.5) } else { t.accent };
            s.bg(bg).text_color(t.accent_foreground).opacity(1.0)
        })
        .disabled(!view.can_page(step))
        .on_click(move |_| view.page(step))
        .child(icon(if back { Icon::ChevronLeft } else { Icon::ChevronRight }))
}

/// One month's grid: weekday names over its weeks; `cell` gives each day's
/// cell given its column.
pub(crate) fn month_grid(
    view: CalendarView,
    month: CalendarDate,
    head_cell: impl Fn(&'static str) -> Div,
    cell: impl Fn(CalendarDate, usize) -> Element,
) -> Div {
    let o = view.opt();
    let first = o.locale.first_weekday();
    let names = (0..7_u8).map(|i| {
        o.locale
            .weekday(first.saturating_add(i).checked_rem(7).unwrap_or(0), o.weekday_format)
    });
    div()
        .named("calendar-grid")
        .flex_col()
        .w_full()
        .child(view.row().named("calendar-grid-row").children(names.map(&head_cell)))
        .children(calendar_weeks(o.system, first, month).into_iter().map(|week| {
            view.row()
                .named("calendar-grid-row")
                .mt(2.0)
                .w_full()
                .children(week.into_iter().enumerate().map(|(col, day)| cell(day, col)))
        }))
}

/// What the header shows between the arrows (the Vue calendar's `layout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CalendarLayout {
    /// "November 2026" (`layout` unset).
    #[default]
    Heading,
    /// `month-and-year`: a month and a year select that jump the view.
    MonthAndYear,
    /// `month-only`: the month select, then the year.
    MonthOnly,
    /// `year-only`: the month, then the year select.
    YearOnly,
}

/// What [`calendar`] is given.
#[derive(Clone)]
pub struct CalendarProps {
    value: Bind<Option<CalendarDate>>,
    options: CalendarOptions,
    layout: CalendarLayout,
    /// `initialFocus`: the tab-stop day takes focus as the calendar mounts.
    initial_focus: bool,
    /// The `#calendar-heading` slot, in place of the layout's.
    heading: Option<Element>,
}

impl HasOptions for CalendarProps {
    fn options_mut(&mut self) -> &mut CalendarOptions {
        &mut self.options
    }
}

/// What a calendar's parts read.
#[derive(Clone)]
struct CalendarContext {
    view: CalendarView,
    value: UseModel<PartProps<CalendarProps>, Option<CalendarDate>>,
}

/// A calendar: its `layout`'s heading between paging arrows, then the month
/// grids. Style it as a box (`rounded-md border`).
#[must_use]
pub fn calendar() -> Part<CalendarProps> {
    let props = CalendarProps {
        value: Bind::default(),
        options: CalendarOptions::default(),
        layout: CalendarLayout::Heading,
        heading: None,
        initial_focus: false,
    };
    Part::new(props, div().named("calendar").p(3.0), |p| {
        component(p, |cx| {
            let props = cx.props();
            let value = use_model(props, |p| &p.props.value);
            let start = value.get_untracked();
            let today = props.with_untracked(|p| p.props.options.today);
            let view = CalendarView::new(move || props.with(|p| p.props.options), start.unwrap_or(today));
            // A day picked from outside comes into view (reka-ui's placeholder
            // follows the model); with `initialFocus` it takes focus too.
            let focuses = props.with_untracked(|p| p.props.initial_focus);
            if focuses {
                // As reka-ui's `initialFocus`: once laid out, focus the tab
                // stop (the picked day, else today), with no ring.
                on_mounted(move || view.focus_day(view.focused.get_untracked(), false));
            }
            let picked = value;
            let _follow = watch(
                move || picked.get(),
                move |new, _| {
                    if let Some(day) = *new {
                        if focuses {
                            view.focus_day(day, false);
                        } else {
                            view.reveal(day);
                        }
                    }
                },
            );
            provide(CalendarContext { view, value });
            move || calendar_render(props, view)
        })
    })
}

fn calendar_render(props: Signal<PartProps<CalendarProps>>, view: CalendarView) -> Element {
    let (root, layout, heading) = props.with(|p| (p.slot.0.clone(), p.props.layout, p.props.heading.clone()));
    let heading = heading.unwrap_or_else(|| match layout {
        CalendarLayout::Heading => calendar_heading(view.heading()).into_element(),
        CalendarLayout::MonthAndYear => view
            .row()
            .items_center()
            .justify_center()
            .gap(1.0)
            .child(calendar_month_select())
            .child(calendar_year_select())
            .into_element(),
        CalendarLayout::MonthOnly => {
            let o = view.opt();
            let year = o.locale.year(o.system, view.month.get());
            view.row()
                .items_center()
                .justify_center()
                .gap(1.0)
                .child(calendar_month_select())
                .child(text(year))
                .into_element()
        }
        CalendarLayout::YearOnly => {
            let o = view.opt();
            let month = o.locale.month(o.system, view.month.get(), MonthWidth::Short);
            view.row()
                .items_center()
                .justify_center()
                .gap(1.0)
                .child(text(month))
                .child(calendar_year_select())
                .into_element()
        }
    });
    root.child(
        calendar_header().pt(0.0).child(heading).child(
            view.row()
                .items_center()
                .gap(1.0)
                .absolute()
                .top(0.0)
                .left(0.0)
                .right(0.0)
                .justify_between()
                .pass_through()
                .child(page_button(view, -1))
                .child(page_button(view, 1)),
        ),
    )
    .child(view.months_row(|month| month_grid(view, month, calendar_head_cell, |day, _| calendar_day(day, month))))
    .into_element()
}

/// The builder methods of [`calendar`] and [`crate::range_calendar`].
impl<P: HasOptions> Part<P> {
    /// The day counted as today (the accent day; where a new calendar
    /// system opens).
    #[must_use]
    pub fn today(self, today: CalendarDate) -> Self {
        self.with_props(|p| p.options_mut().today = today)
    }

    /// `numberOfMonths`: how many months show side by side.
    #[must_use]
    pub fn number_of_months(self, months: u8) -> Self {
        self.with_props(|p| p.options_mut().months = months.max(1))
    }

    /// `minValue`: days before it are disabled.
    #[must_use]
    pub fn min(self, date: CalendarDate) -> Self {
        self.with_props(|p| p.options_mut().min = Some(date))
    }

    /// `maxValue`: days after it are disabled.
    #[must_use]
    pub fn max(self, date: CalendarDate) -> Self {
        self.with_props(|p| p.options_mut().max = Some(date))
    }

    /// `weekdayFormat`: how the weekdays over the columns are named.
    #[must_use]
    pub fn weekday_format(self, format: WeekdayFormat) -> Self {
        self.with_props(|p| p.options_mut().weekday_format = format)
    }

    /// A day trigger's side in spacing units (`**:data-[slot=calendar-cell-trigger]:size-12!`).
    #[must_use]
    pub fn cell_size(self, size: f32) -> Self {
        self.with_props(|p| p.options_mut().cell_size = size)
    }

    /// `disableDaysOutsideCurrentView`: the padding days of other months are disabled.
    #[must_use]
    pub fn disable_days_outside_current_view(self) -> Self {
        self.with_props(|p| p.options_mut().disable_outside_view = true)
    }

    /// `dir="rtl"`: laid out right to left.
    #[must_use]
    pub fn rtl(self) -> Self {
        self.with_props(|p| p.options_mut().rtl = true)
    }

    /// Counted in `system` and read out in `locale` (`placeholder` in that
    /// calendar, `locale`); changing either goes back to today's month.
    #[must_use]
    pub fn calendar(self, system: CalendarSystem, locale: CalendarLocale) -> Self {
        self.with_props(|p| {
            let o = p.options_mut();
            o.system = system;
            o.locale = locale;
        })
    }
}

impl Part<CalendarProps> {
    /// The picked day, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<Option<CalendarDate>>>) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_model(model))
    }

    /// The picked day shown, kept by the caller as a plain value (`:model-value`):
    /// a pick only calls `on_change`.
    #[must_use]
    pub fn value(self, value: Option<CalendarDate>) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_value(value))
    }

    /// The picked day first, kept by the calendar (`default-value`).
    #[must_use]
    pub fn default_value(self, value: Option<CalendarDate>) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_default(value))
    }

    /// Called with each value picked (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(Option<CalendarDate>) + 'static) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).on_change(f))
    }

    /// `initialFocus`: as it mounts, the picked day (else today) takes
    /// focus, without a ring; a day the model moves to later takes it too.
    /// For a calendar in a popover.
    #[must_use]
    pub fn initial_focus(self) -> Self {
        self.with_props(|p| p.initial_focus = true)
    }

    /// `layout`: what the header shows.
    #[must_use]
    pub fn layout(self, layout: CalendarLayout) -> Self {
        self.with_props(|p| p.layout = layout)
    }

    /// The `#calendar-heading` slot: `heading` between the arrows in place
    /// of the layout's (build it from [`calendar_month_select`] and
    /// [`calendar_year_select`]).
    #[must_use]
    pub fn heading(self, heading: impl IntoElement) -> Self {
        let heading = heading.into_element();
        self.with_props(|p| p.heading = Some(heading))
    }
}

/// One day of a calendar's grid: its cell and trigger, an instance of its
/// own (it renders when its selection, focus or view changes).
fn calendar_day(day: CalendarDate, month: CalendarDate) -> Element {
    component((day, month), |cx| {
        let props = cx.props();
        let ctx = use_context::<CalendarContext>();
        move || {
            let (day, month) = props.get();
            let Some(ctx) = &ctx else {
                return Element::Empty;
            };
            let selected = ctx.value.with(|v| *v == Some(day));
            let selection = if selected {
                DaySelection::Picked
            } else {
                DaySelection::Unselected
            };
            let value = ctx.value;
            calendar_cell(selected)
                .child(day_trigger(ctx.view, day, month, selection, move |d| {
                    value.set(Some(d))
                }))
                .into_element()
        }
    })
    .into_element()
}

#[must_use]
pub fn calendar_header() -> Div {
    div()
        .named("calendar-header")
        .flex()
        .justify_center()
        .pt(1.0)
        .relative()
        .items_center()
        .w_full()
        .px(8.0)
}

#[must_use]
pub fn calendar_heading(heading: impl Into<String>) -> impl IntoElement {
    text(heading).named("calendar-heading").text_sm().font_medium()
}

/// The shown month's select, its months by short name: a part of the
/// calendar it is in.
#[must_use]
pub fn calendar_month_select() -> Element {
    component((), |_| {
        let ctx = use_context::<CalendarContext>();
        let focus = FocusHandle::new();
        move || {
            let Some(ctx) = &ctx else {
                return Element::Empty;
            };
            let view = ctx.view;
            let o = view.opt();
            let (system, locale) = (o.system, o.locale);
            let shown = view.month.get();
            let year = system.fields(shown).year;
            let months: Vec<NativeSelectEntry> = (1..=system.months_in_year(year))
                .map(|m| {
                    let name = locale.month(system, system.date(year, m, 1), MonthWidth::Short);
                    native_select_option(&m.to_string(), name)
                })
                .collect();
            heading_select(
                view,
                locale.month(system, shown, MonthWidth::Short),
                focus,
                &system.fields(shown).month.to_string(),
                &months,
                move |v| {
                    if let Ok(m) = v.parse::<u8>() {
                        view.set_month(m);
                    }
                },
            )
            .into_element()
        }
    })
    .into_element()
}

/// The shown year's select: reka-ui's `yearRange`, `minValue` to `maxValue`,
/// else a hundred years back and ten ahead; a part of the calendar it is in.
#[must_use]
pub fn calendar_year_select() -> Element {
    component((), |_| {
        let ctx = use_context::<CalendarContext>();
        let focus = FocusHandle::new();
        move || {
            let Some(ctx) = &ctx else {
                return Element::Empty;
            };
            let view = ctx.view;
            let o = view.opt();
            let (system, locale) = (o.system, o.locale);
            let shown = view.month.get();
            let year_of = |d: CalendarDate| system.fields(d).year;
            let first = o.min.map_or(year_of(o.today).saturating_sub(100), year_of);
            let last = o.max.map_or(year_of(o.today).saturating_add(10), year_of);
            let years: Vec<NativeSelectEntry> = (first..=last)
                .map(|y| native_select_option(&y.to_string(), &locale.year(system, system.with_year(shown, y))))
                .collect();
            heading_select(
                view,
                &locale.year(system, shown),
                focus,
                &year_of(shown).to_string(),
                &years,
                move |v| {
                    if let Ok(y) = v.parse::<i32>() {
                        view.set_year(y);
                    }
                },
            )
            .into_element()
        }
    })
    .into_element()
}

/// A select whose own text is hidden under `shown` in the header's size: the
/// Vue calendar's month and year templates. Right to left, `shown` sits at
/// the right and the chevron at the left (the Persian demo's
/// `**:data-[slot=native-select-icon]:left-3.5`).
fn heading_select(
    view: CalendarView,
    shown: &str,
    focus: FocusHandle,
    value: &str,
    entries: &[NativeSelectEntry],
    on_change: impl Fn(String) + 'static,
) -> Div {
    let rtl = view.rtl();
    div()
        .relative()
        .child(
            div()
                .absolute()
                .inset(0.0)
                .flex()
                .items_center()
                .when(rtl, |d| d.justify_end())
                .text_sm()
                .pl(2.0)
                .pass_through()
                .child(shown.to_owned()),
        )
        .child(
            native_select(entries.iter().cloned())
                .value(value)
                .on_change(on_change)
                .focus(focus)
                .class(move |field, chevron| {
                    let field = field
                        .text_xs()
                        .h(8.0)
                        .pr(6.0)
                        .pl(2.0)
                        .text_color(gui::Color::TRANSPARENT);
                    let chevron = if rtl {
                        chevron.right_len(Length::Auto).left(3.5)
                    } else {
                        chevron.right(1.0)
                    };
                    (field, chevron)
                }),
        )
}

/// A weekday's name over its column (`text-[0.8rem]`).
#[must_use]
pub fn calendar_head_cell(name: &'static str) -> Div {
    let t = theme();
    div()
        .named("calendar-head-cell")
        .flex_1()
        .flex()
        .justify_center()
        .rounded_md()
        .font_normal()
        .text_size(12.8, 19.2)
        .text_color(t.muted_foreground)
        .child(name)
}

/// A day's cell; a selected day's cell takes the accent behind its trigger.
#[must_use]
pub fn calendar_cell(selected: bool) -> Div {
    let t = theme();
    div()
        .named("calendar-cell")
        .relative()
        .p(0.0)
        .flex()
        .justify_center()
        .text_sm()
        .flex_1()
        .when(selected, |d| d.rounded_md().bg(t.accent))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::CalendarDate;

    /// Set in the child process `today_follows_the_local_time_zone` runs.
    const PROBE: &str = "SHADCN_CALENDAR_TODAY_PROBE";

    fn utc(secs: u64) -> SystemTime {
        UNIX_EPOCH.checked_add(Duration::from_secs(secs)).unwrap()
    }

    fn day(year: i32, month: u8, day: u8) -> Option<CalendarDate> {
        CalendarDate::new(year, month, day)
    }

    #[test]
    fn an_instant_falls_on_its_zones_day() {
        // 2026-11-20T20:00:00Z: still the 20th in London, already the 21st in Saigon.
        let evening = utc(1_795_204_800);
        assert_eq!(CalendarDate::at(evening, "UTC"), day(2026, 11, 20));
        assert_eq!(CalendarDate::at(evening, "Europe/London"), day(2026, 11, 20));
        assert_eq!(CalendarDate::at(evening, "Asia/Ho_Chi_Minh"), day(2026, 11, 21));
        // 2026-11-21T05:00:00Z: the 21st in UTC, the 20th in Los Angeles.
        let dawn = utc(1_795_230_000);
        assert_eq!(CalendarDate::at(dawn, "UTC"), day(2026, 11, 21));
        assert_eq!(CalendarDate::at(dawn, "America/Los_Angeles"), day(2026, 11, 20));
        assert_eq!(CalendarDate::at(dawn, "Nowhere/Atlantis"), None);
    }

    /// `today()` is the day in the process's time zone, not in UTC: run in
    /// zones 25 hours apart (`TZ`), it names two different days, each the
    /// one it is now there.
    #[test]
    fn today_follows_the_local_time_zone() {
        if std::env::var_os(PROBE).is_some() {
            let d = CalendarDate::today();
            println!("today={}-{}-{}", d.year(), d.month(), d.day());
            return;
        }
        let today_in = |zone: &str| {
            let before = CalendarDate::at(SystemTime::now(), zone);
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "calendar::tests::today_follows_the_local_time_zone",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(PROBE, "1")
                .env("TZ", zone)
                .output()
                .unwrap();
            let after = CalendarDate::at(SystemTime::now(), zone);
            let out = String::from_utf8_lossy(&out.stdout).into_owned();
            // The harness prints the test's name on the same line.
            let line = out
                .split("today=")
                .nth(1)
                .and_then(|t| t.split_whitespace().next())
                .unwrap_or_else(|| panic!("no today in {out}"));
            let mut parts = line.split('-').map(|p| p.parse::<i32>().unwrap());
            let (y, m, d) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
            let got = day(y, u8::try_from(m).unwrap(), u8::try_from(d).unwrap());
            assert!(
                got == before || got == after,
                "TZ={zone}: today {got:?}, the day there {before:?}"
            );
            got
        };
        assert_ne!(today_in("Pacific/Kiritimati"), today_in("Pacific/Pago_Pago"));
    }
}
