//! Calendar (`ui/calendar`): a month of days to pick one date from, paged with
//! the header's arrows and walked with the keyboard as reka-ui's
//! `CalendarRoot` does. The date type, the view (which months show, in which
//! calendar system and locale, which day holds the grid's tab stop) and the
//! day trigger are shared with `range_calendar`.

use std::num::Saturating;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gui::prelude::*;
use gui::{Div, FocusHandle, Icon, KeyEvent, Length, NamedKey, Signal, div, icon, text};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::calendar_locale::{CalendarLocale, MonthWidth, WeekdayFormat};
use crate::calendar_system::CalendarSystem;
use crate::native_select::{NativeSelectEntry, NativeSelectLook, native_select_option, native_select_with};
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

    /// Today in UTC: the standard library knows no local time zone.
    #[must_use]
    pub fn today_utc() -> Self {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Self::from_days(i64::try_from(secs.div_euclid(86_400)).unwrap_or(0))
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

/// Which months show, in which calendar system and locale, and which day
/// holds the grid's one tab stop: what the calendar and the range calendar
/// share. Created once, outside the render closure.
#[derive(Clone)]
pub struct CalendarView {
    /// The first month shown (reka-ui's `placeholder`), as its first day in [`Self::system`].
    pub month: Signal<CalendarDate>,
    /// The day the arrow keys move and the grid's tab stop sits on.
    pub focused: Signal<CalendarDate>,
    pub focus: FocusHandle,
    pub today: CalendarDate,
    /// The calendar the months are counted in (the placeholder's calendar).
    pub system: Signal<CalendarSystem>,
    /// What the names, digits, week and direction are read in (`locale`).
    pub locale: Signal<CalendarLocale>,
    months: u8,
    min: Option<CalendarDate>,
    max: Option<CalendarDate>,
    weekday_format: WeekdayFormat,
    /// A day trigger's side, in spacing units (`size-8`).
    cell_size: f32,
    disable_outside_view: bool,
    /// `dir="rtl"`: rows run right to left, the arrows and arrow keys mirrored.
    rtl: bool,
}

impl CalendarView {
    /// One month, the one holding `focused`, Gregorian in `en-US`.
    #[must_use]
    pub fn new(today: CalendarDate, focused: CalendarDate) -> Self {
        Self {
            month: Signal::labeled("calendar month", focused.start_of_month()),
            focused: Signal::labeled("calendar focused", focused),
            focus: FocusHandle::new(),
            today,
            system: Signal::labeled("calendar system", CalendarSystem::Gregorian),
            locale: Signal::labeled("calendar locale", CalendarLocale::EnUs),
            months: 1,
            min: None,
            max: None,
            weekday_format: WeekdayFormat::Narrow,
            cell_size: 8.0,
            disable_outside_view: false,
            rtl: false,
        }
    }

    /// `numberOfMonths`: how many months show side by side.
    #[must_use]
    pub fn number_of_months(mut self, months: u8) -> Self {
        self.months = months.max(1);
        self
    }

    /// `minValue`: days before it are disabled.
    #[must_use]
    pub fn min(mut self, date: CalendarDate) -> Self {
        self.min = Some(date);
        self
    }

    /// `maxValue`: days after it are disabled.
    #[must_use]
    pub fn max(mut self, date: CalendarDate) -> Self {
        self.max = Some(date);
        self
    }

    /// `weekdayFormat`: how the weekdays over the columns are named.
    #[must_use]
    pub fn weekday_format(mut self, format: WeekdayFormat) -> Self {
        self.weekday_format = format;
        self
    }

    /// A day trigger's side in spacing units (`**:data-[slot=calendar-cell-trigger]:size-12!`).
    #[must_use]
    pub fn cell_size(mut self, size: f32) -> Self {
        self.cell_size = size;
        self
    }

    /// `disableDaysOutsideCurrentView`: the padding days of other months are disabled.
    #[must_use]
    pub fn disable_days_outside_current_view(mut self) -> Self {
        self.disable_outside_view = true;
        self
    }

    /// `dir="rtl"`: laid out right to left.
    #[must_use]
    pub fn rtl(mut self) -> Self {
        self.rtl = true;
        self
    }

    /// Count the months in `system` and read them out in `locale` (the
    /// placeholder's calendar and `locale`), back on today's month.
    pub fn set_calendar(&self, system: CalendarSystem, locale: CalendarLocale) {
        self.system.set(system);
        self.locale.set(locale);
        self.month.set(system.start_of_month(self.today));
        self.focused.set(self.today);
    }

    /// The first day of each month shown.
    #[must_use]
    pub fn shown(&self) -> Vec<CalendarDate> {
        let system = self.system.get();
        let first = self.month.get();
        (0..i64::from(self.months))
            .map(|i| system.add_months(first, i))
            .collect()
    }

    /// Whether `day` is a day of the month `month` starts.
    #[must_use]
    pub fn in_month(&self, day: CalendarDate, month: CalendarDate) -> bool {
        self.system.get().same_month(day, month)
    }

    /// `day`'s number as the grid shows it.
    #[must_use]
    pub fn day_label(&self, day: CalendarDate) -> String {
        self.locale.get().number(i64::from(self.system.get().fields(day).day))
    }

    /// "November 2026", or "November - December 2026" over several months.
    #[must_use]
    pub fn heading(&self) -> String {
        let (system, locale) = (self.system.get(), self.locale.get());
        let first = self.month.get();
        let last = system.add_months(first, i64::from(self.months.saturating_sub(1)));
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
    #[must_use]
    pub fn is_disabled(&self, date: CalendarDate, month: CalendarDate) -> bool {
        self.min.is_some_and(|m| date < m)
            || self.max.is_some_and(|m| date > m)
            || (self.disable_outside_view && !self.in_month(date, month))
    }

    /// The last day shown when the first month shown starts on `first`.
    fn last_shown(&self, first: CalendarDate) -> CalendarDate {
        let system = self.system.get();
        system.end_of_month(system.add_months(first, i64::from(self.months.saturating_sub(1))))
    }

    /// Whether paging `step` pages (of `numberOfMonths`) still shows a day in bounds.
    #[must_use]
    pub fn can_page(&self, step: i64) -> bool {
        let first = self
            .system
            .get()
            .add_months(self.month.get(), step.saturating_mul(i64::from(self.months)));
        let last = self.last_shown(first);
        !(self.max.is_some_and(|m| first > m) || self.min.is_some_and(|m| last < m))
    }

    /// Show the months `step` pages away; the focused day moves with them.
    pub fn page(&self, step: i64) {
        self.shift(step.saturating_mul(i64::from(self.months)));
    }

    /// Show the months `months` later (earlier when negative); the focused
    /// day moves with them, as reka-ui's `placeholder` does.
    pub fn shift(&self, months: i64) {
        let system = self.system.get();
        self.month.update(|m| system.add_months(m, months));
        self.focused.update(|d| system.add_months(d, months));
    }

    /// Show `year`'s month of the same name (the year select); the focused day moves with it.
    pub fn set_year(&self, year: i32) {
        let system = self.system.get();
        self.month.update(|m| system.with_year(m, year));
        self.focused.update(|d| system.with_year(d, year));
    }

    /// Show `month` of the shown year (the month select); the focused day moves with it.
    pub fn set_month(&self, month: u8) {
        let system = self.system.get();
        let by = i64::from(month).saturating_sub(i64::from(system.fields(self.month.get()).month));
        self.shift(by);
    }

    /// Move the tab stop to `date`, bring its month into view and focus it.
    pub fn focus_day(&self, date: CalendarDate, by_keyboard: bool) {
        let system = self.system.get();
        let date = self.min.map_or(date, |m| date.max(m));
        let date = self.max.map_or(date, |m| date.min(m));
        let first = self.month.get();
        if date < first {
            self.month.set(system.start_of_month(date));
        } else if date > self.last_shown(first) {
            self.month.set(system.add_months(
                system.start_of_month(date),
                i64::from(self.months.saturating_sub(1)).saturating_neg(),
            ));
        }
        self.focused.set(date);
        gui::focus::set_focus(Some(self.focus.id()), by_keyboard);
    }

    /// The grid's keys: arrows by day and week (mirrored right to left),
    /// Home and End to the week's ends, Page Up and Down by month (by year
    /// with Shift).
    pub fn on_key(&self, e: &KeyEvent) -> bool {
        let (system, locale) = (self.system.get(), self.locale.get());
        let d = self.focused.get();
        let page: i64 = if e.modifiers.shift { 12 } else { 1 };
        let forward: i64 = if self.rtl { -1 } else { 1 };
        let into_week = i64::from(d.weekday())
            .saturating_sub(i64::from(locale.first_weekday()))
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
            system.add_months(d, page.saturating_neg())
        } else if e.is(NamedKey::PageDown) {
            system.add_months(d, page)
        } else {
            return false;
        };
        self.focus_day(to, true);
        true
    }

    /// A row laid out in the locale's direction.
    pub(crate) fn row(&self) -> Div {
        if self.rtl {
            div().flex_row_reverse()
        } else {
            div().flex_row()
        }
    }

    /// The months' grids side by side, walked with [`Self::on_key`].
    pub(crate) fn months_row(&self, grid: impl Fn(CalendarDate) -> Div) -> Div {
        let view = self.clone();
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
    view: &CalendarView,
    day: CalendarDate,
    month: CalendarDate,
    selection: DaySelection,
    on_pick: impl Fn(CalendarDate) + 'static,
) -> Div {
    let t = theme();
    let outside = !view.in_month(day, month);
    let disabled = view.is_disabled(day, month);
    let view_c = view.clone();
    button(ButtonVariant::Ghost, ButtonSize::IconSm)
        .named("calendar-cell-trigger")
        .size(view.cell_size)
        .p(0.0)
        .font_normal()
        .cursor_default()
        .when(day == view.today && selection == DaySelection::Unselected, |b| {
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
            view_c.focus_day(day, false);
        })
        .child(view.day_label(day))
}

/// The prev and next arrows both calendars use: an outline icon button,
/// see-through until hovered, its chevron pointing the way it pages.
pub(crate) fn page_button(view: &CalendarView, step: i64) -> Div {
    let t = theme();
    let view_c = view.clone();
    let back = (step < 0) != view.rtl;
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
        .on_click(move |_| view_c.page(step))
        .child(icon(if back { Icon::ChevronLeft } else { Icon::ChevronRight }))
}

/// One month's grid: weekday names over its weeks; `cell` builds each day's
/// cell given its column.
pub(crate) fn month_grid(
    view: &CalendarView,
    month: CalendarDate,
    head_cell: impl Fn(&'static str) -> Div,
    cell: impl Fn(CalendarDate, usize) -> Div,
) -> Div {
    let (system, locale) = (view.system.get(), view.locale.get());
    let first = locale.first_weekday();
    let names =
        (0..7_u8).map(|i| locale.weekday(first.saturating_add(i).checked_rem(7).unwrap_or(0), view.weekday_format));
    div()
        .named("calendar-grid")
        .flex_col()
        .w_full()
        .child(view.row().named("calendar-grid-row").children(names.map(&head_cell)))
        .children(calendar_weeks(system, first, month).into_iter().map(|week| {
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

/// The value and view of one calendar. Created once, outside the render closure.
#[derive(Clone)]
pub struct CalendarState {
    /// The picked day (`v-model`).
    pub selected: Signal<Option<CalendarDate>>,
    pub view: CalendarView,
    /// What the header shows (`layout`).
    pub layout: Signal<CalendarLayout>,
    /// `@update:model-value`: told the day a click picks.
    on_update: Option<Rc<dyn Fn(CalendarDate)>>,
    month_select: FocusHandle,
    year_select: FocusHandle,
}

impl CalendarState {
    /// Showing the month of `selected`, or of today when nothing is picked.
    #[must_use]
    pub fn new(today: CalendarDate, selected: Option<CalendarDate>) -> Self {
        Self {
            selected: Signal::labeled("calendar selected", selected),
            view: CalendarView::new(today, selected.unwrap_or(today)),
            layout: Signal::labeled("calendar layout", CalendarLayout::Heading),
            on_update: None,
            month_select: FocusHandle::new(),
            year_select: FocusHandle::new(),
        }
    }

    /// `layout`: what the header shows.
    #[must_use]
    pub fn layout(self, layout: CalendarLayout) -> Self {
        self.layout.set(layout);
        self
    }

    /// `minValue`: days before it are disabled.
    #[must_use]
    pub fn min(mut self, date: CalendarDate) -> Self {
        self.view = self.view.min(date);
        self
    }

    /// `maxValue`: days after it are disabled.
    #[must_use]
    pub fn max(mut self, date: CalendarDate) -> Self {
        self.view = self.view.max(date);
        self
    }

    /// `weekdayFormat`: how the weekdays over the columns are named.
    #[must_use]
    pub fn weekday_format(mut self, format: WeekdayFormat) -> Self {
        self.view = self.view.weekday_format(format);
        self
    }

    /// A day trigger's side in spacing units (`**:data-[slot=calendar-cell-trigger]:size-12!`).
    #[must_use]
    pub fn cell_size(mut self, size: f32) -> Self {
        self.view = self.view.cell_size(size);
        self
    }

    /// `disableDaysOutsideCurrentView`: the padding days of other months are disabled.
    #[must_use]
    pub fn disable_days_outside_current_view(mut self) -> Self {
        self.view = self.view.disable_days_outside_current_view();
        self
    }

    /// `dir="rtl"`: laid out right to left.
    #[must_use]
    pub fn rtl(mut self) -> Self {
        self.view = self.view.rtl();
        self
    }

    /// `@update:model-value`: `f` is told the day a click picks, after it is picked.
    #[must_use]
    pub fn on_update(mut self, f: impl Fn(CalendarDate) + 'static) -> Self {
        self.on_update = Some(Rc::new(f));
        self
    }

    /// Counted in `system` and read out in `locale` (`placeholder` in that
    /// calendar, `locale`).
    #[must_use]
    pub fn calendar(self, system: CalendarSystem, locale: CalendarLocale) -> Self {
        self.view.set_calendar(system, locale);
        self
    }
}

/// The whole calendar: its `layout`'s heading, paging arrows and the month grid.
#[must_use]
pub fn calendar(state: &CalendarState) -> Div {
    let heading = match state.layout.get() {
        CalendarLayout::Heading => calendar_heading(state.view.heading()).into_element(),
        CalendarLayout::MonthAndYear => state
            .view
            .row()
            .items_center()
            .justify_center()
            .gap(1.0)
            .child(calendar_month_select(state))
            .child(calendar_year_select(state))
            .into_element(),
        CalendarLayout::MonthOnly => {
            let year = state
                .view
                .locale
                .get()
                .year(state.view.system.get(), state.view.month.get());
            state
                .view
                .row()
                .items_center()
                .justify_center()
                .gap(1.0)
                .child(calendar_month_select(state))
                .child(text(year))
                .into_element()
        }
        CalendarLayout::YearOnly => {
            let view = &state.view;
            let month = view
                .locale
                .get()
                .month(view.system.get(), view.month.get(), MonthWidth::Short);
            view.row()
                .items_center()
                .justify_center()
                .gap(1.0)
                .child(text(month))
                .child(calendar_year_select(state))
                .into_element()
        }
    };
    calendar_with_heading(state, heading)
}

/// The calendar with `heading` between the arrows in place of its
/// `layout`'s (the `#calendar-heading` slot; build it from
/// [`calendar_month_select`] and [`calendar_year_select`]).
#[must_use]
pub fn calendar_with_heading(state: &CalendarState, heading: impl IntoElement) -> Div {
    let view = &state.view;
    div()
        .named("calendar")
        .p(3.0)
        .child(
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
                    .child(calendar_prev_button(view))
                    .child(calendar_next_button(view)),
            ),
        )
        .child(view.months_row(|month| {
            month_grid(view, month, calendar_head_cell, |day, _| {
                calendar_cell(state.selected.get() == Some(day)).child(calendar_cell_trigger(state, day, month))
            })
        }))
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

/// The shown month's select: its months by short name.
#[must_use]
pub fn calendar_month_select(state: &CalendarState) -> Div {
    let view = &state.view;
    let (system, locale) = (view.system.get(), view.locale.get());
    let shown = view.month.get();
    let year = system.fields(shown).year;
    let months: Vec<NativeSelectEntry> = (1..=system.months_in_year(year))
        .map(|m| {
            let name = locale.month(system, system.date(year, m, 1), MonthWidth::Short);
            native_select_option(&m.to_string(), name)
        })
        .collect();
    let to_month = view.clone();
    heading_select(
        view,
        locale.month(system, shown, MonthWidth::Short),
        state.month_select,
        &system.fields(shown).month.to_string(),
        &months,
        move |v| {
            if let Ok(m) = v.parse::<u8>() {
                to_month.set_month(m);
            }
        },
    )
}

/// The shown year's select: reka-ui's `yearRange`, `minValue` to `maxValue`,
/// else a hundred years back and ten ahead.
#[must_use]
pub fn calendar_year_select(state: &CalendarState) -> Div {
    let view = &state.view;
    let (system, locale) = (view.system.get(), view.locale.get());
    let shown = view.month.get();
    let year_of = |d: CalendarDate| system.fields(d).year;
    let first = view.min.map_or(year_of(view.today).saturating_sub(100), year_of);
    let last = view.max.map_or(year_of(view.today).saturating_add(10), year_of);
    let years: Vec<NativeSelectEntry> = (first..=last)
        .map(|y| native_select_option(&y.to_string(), &locale.year(system, system.with_year(shown, y))))
        .collect();
    let to_year = view.clone();
    heading_select(
        view,
        &locale.year(system, shown),
        state.year_select,
        &year_of(shown).to_string(),
        &years,
        move |v| {
            if let Ok(y) = v.parse::<i32>() {
                to_year.set_year(y);
            }
        },
    )
}

/// A select whose own text is hidden under `shown` in the header's size: the
/// Vue calendar's month and year templates. Right to left, `shown` sits at
/// the right and the chevron at the left (the Persian demo's
/// `**:data-[slot=native-select-icon]:left-3.5`).
fn heading_select(
    view: &CalendarView,
    shown: &str,
    focus: FocusHandle,
    value: &str,
    entries: &[NativeSelectEntry],
    on_change: impl Fn(String) + 'static,
) -> Div {
    let rtl = view.rtl;
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
        .child(native_select_with(
            focus,
            value,
            on_change,
            entries,
            NativeSelectLook::default(),
            |field, chevron| {
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
            },
        ))
}

#[must_use]
pub fn calendar_prev_button(view: &CalendarView) -> Div {
    page_button(view, -1)
}

#[must_use]
pub fn calendar_next_button(view: &CalendarView) -> Div {
    page_button(view, 1)
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

/// `day`'s button in `month`'s grid: picks it on click.
#[must_use]
pub fn calendar_cell_trigger(state: &CalendarState, day: CalendarDate, month: CalendarDate) -> Div {
    let selected = state.selected.clone();
    let selection = if selected.get() == Some(day) {
        DaySelection::Picked
    } else {
        DaySelection::Unselected
    };
    let on_update = state.on_update.clone();
    day_trigger(&state.view, day, month, selection, move |d| {
        selected.set(Some(d));
        if let Some(f) = &on_update {
            f(d);
        }
    })
}
