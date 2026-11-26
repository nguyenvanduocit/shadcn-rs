//! A month grid to pick dates from (reka's `Calendar/`), and the grid core
//! it shares with [`crate::range_calendar`] (research §11 "Lõi lưới").
//!
//! **The grid core** ([`CalendarGrid`]) is a calendar's view and keyboard:
//! the months shown, the anchor day (reka's `placeholder`: which months
//! show and which day holds the grid's one Tab stop, always set, whether
//! anything is picked or not), and the keys that move the anchor. Keys only
//! move the anchor; only a click, Enter or Space picks. Three kinds of day
//! are special, as the research's table has them:
//!
//! | Kind | Focus | Tab stop | Arrows | Home/End/PageUp/PageDown | Picked |
//! |---|---|---|---|---|---|
//! | blocked (before `min`, after `max`, [`CalendarOptions::is_blocked`], the calendar disabled) | no | no | skipped | skipped | no |
//! | unavailable ([`CalendarOptions::is_unavailable`]) | yes | no | stop on it | skipped | no |
//! | outside its grid's month | its own month's copy takes it, paging there | no | never stopped on | reached | yes, unless `disable_outside_view` |
//!
//! The keys (`Calendar/CalendarCellTrigger.vue:109-226`,
//! `shared/useCalendarKeyboardNavigation.ts`): ArrowLeft and ArrowRight a
//! day (swapped right to left), ArrowUp and ArrowDown a week, Home and End
//! the ends of the week (into the month beside it if that is where they
//! are), PageUp and PageDown a month (a year with Shift) keeping the day or
//! the month's last. An arrow past `min` or `max` stays; Home, End, PageUp
//! and PageDown are held to them. A day not shown pages the view there.
//! Enter and Space pick; with Control, Alt or Command they are left to the
//! page (a form's submit), as Escape and Tab are.
//!
//! **One Tab stop**: the anchor, when it is shown and neither blocked nor
//! unavailable; else, when nothing is picked or what is picked is blocked,
//! the first day of the view that is neither; else none
//! (`Calendar/CalendarCellTrigger.vue:82-90`). The focus handle follows the
//! anchor, so a key that moves the anchor moves focus with it.
//!
//! **Values.** [`Calendar`] picks one day (`Option<CalendarDate>`) or many
//! (`Vec<CalendarDate>`, in the order they were picked): two types, no
//! `multiple` flag (research §3.2). Picking the picked day again clears it,
//! unless `prevent_deselect`; a caller's value is shown as it is, never held
//! to `min` or `max`, and only makes [`Calendar::invalid`] true.
//!
//! What reka gets wrong and this does not (ADR-0025, group 7):
//!
//! - `disableDaysOutsideCurrentView` makes every gridcell `aria-disabled`,
//!   the month's own days too (`Calendar/CalendarCell.vue:25`): here only
//!   the days outside it are.
//! - The first focusable day compares the anchor, not the month's first
//!   day, with `min`, so an anchor before `min` in the same month skips the
//!   whole month and a one-month calendar loses its Tab stop
//!   (`Calendar/useCalendar.ts:361-363`): here a month is skipped only when
//!   it ends before `min`.
//! - Initial focus takes the first matching cell in the DOM, which may be
//!   an outside-the-month copy that cannot take focus
//!   (`shared/date/utils.ts:60-62`): here it is found by date, in its month.
//! - Today is the machine's (`Calendar/CalendarCellTrigger.vue:69-71`):
//!   here it is the caller's [`CalendarOptions::today`] (the day in any
//!   zone, `CalendarDate::at`), so a caller keeping it in a signal keeps it
//!   current past midnight.
//! - A focused Prev or Next that paging disables drops focus to the page
//!   (`Calendar/CalendarNext.vue:27,46`): here focus goes to the other
//!   button, which paging has just made usable.
//! - Numpad Enter is swallowed without picking, the keys being matched by
//!   `e.code` (`Calendar/CalendarCellTrigger.vue:116-121`): keys here are
//!   matched by what they mean, so every Enter picks.
//! - Prev and Next of a disabled calendar with no `min` or `max` stay
//!   usable (`Calendar/useCalendar.ts:155-186` returns before it looks at
//!   `disabled`): here a disabled calendar pages nowhere.
//! - The labels "Event Date", "Previous page" and "Next page" are English
//!   only: here they come from the locale ([`DateStrings`]).
//!
//! reka has no live region, and neither does this: a page change is heard
//! as focus moves to a day, whose name says its month and year.

use std::fmt;

use gui::events::{KeyEvent, NamedKey};
use gui::focus::{self, FocusHandle};
use gui::{Computed, Derive, Direction, Div, Role, Signal, UseModel, div, text, watch};

use crate::availability::Availability;
use crate::date::{
    CalendarDate, CalendarLocale, CalendarSystem, DateStrings, MonthWidth, WeekdayFormat, end_of_week, month_weeks,
    start_of_week,
};
use crate::direction::ReadDirection;
use crate::visually_hidden::{Hidden, visually_hidden};

/// How many days Home, End, PageUp and PageDown look on for one that can
/// take focus, before they give up (`MAX_FOCUS_RETRIES`).
pub const MAX_FOCUS_RETRIES: u32 = 24;

/// How many days an arrow key looks on, by its step, for one that is not
/// blocked. reka looks without end (`shiftFocus` recurses until a day is
/// found or paging stops), which never ends when every day ahead is
/// blocked and nothing bounds the calendar; a year of blocked days is
/// where this gives up and stays.
pub const MAX_ARROW_STEPS: u32 = 366;

/// What a calendar is told: reka's root props. Equal when every field is,
/// the predicates aside by pointer (a [`Derive`], called as a render reads
/// reads signals is read again when they change).
#[derive(Clone, Debug, PartialEq)]
pub struct CalendarOptions {
    /// The day counted as today (`data-today`): the caller's, so it is the
    /// day in the zone it chooses and stays current if it keeps it so.
    pub today: CalendarDate,
    /// `numberOfMonths`.
    pub months: u8,
    /// `pagedNavigation`: Prev and Next turn every month shown, not one.
    pub paged: bool,
    /// `nextPage`, `prevPage`: the day Prev and Next turn the view to, from
    /// the first month's day as the view was made; none for a month (every
    /// month shown, `paged`). The anchor goes to the day it gives, on the
    /// month's first when it turned a month or more, on the year's first
    /// month when a year or more (`Calendar/useCalendar.ts:212-302`). A
    /// button's own function ([`CalendarGrid::page_button_by`]) overrides it.
    pub next_page: Option<Derive<CalendarDate, CalendarDate>>,
    pub prev_page: Option<Derive<CalendarDate, CalendarDate>>,
    /// `fixedWeeks`: always six weeks.
    pub fixed_weeks: bool,
    /// `minValue`, `maxValue`: days outside are blocked.
    pub min: Option<CalendarDate>,
    pub max: Option<CalendarDate>,
    /// `weekStartsOn`, 0 for Sunday (absolute, not relative to the locale);
    /// none for the locale's.
    pub first_weekday: Option<u8>,
    pub weekday_format: WeekdayFormat,
    /// `disableDaysOutsideCurrentView`.
    pub disable_outside_view: bool,
    /// `disabled`: [`Availability::Disabled`] blocks every day and both
    /// page buttons. [`Availability::Unavailable`] reads as disabled and
    /// picks nothing, but keeps focus and the keys.
    pub availability: Availability,
    /// `readonly`: focus and keys as usual, nothing picked.
    pub readonly: bool,
    /// `preventDeselect`: picking a picked day keeps it.
    pub prevent_deselect: bool,
    /// The calendar the months are counted in.
    pub system: CalendarSystem,
    /// What names, digits and the first weekday are read in.
    pub locale: CalendarLocale,
    /// What assistive technology is told; none for the locale's.
    pub strings: Option<DateStrings>,
    /// `isDateDisabled`: days the caller blocks.
    pub is_blocked: Option<Derive<CalendarDate, bool>>,
    /// `isDateUnavailable`: days that take focus but cannot be picked
    /// (already booked).
    pub is_unavailable: Option<Derive<CalendarDate, bool>>,
}

impl Default for CalendarOptions {
    fn default() -> Self {
        Self {
            today: CalendarDate::today(),
            months: 1,
            paged: false,
            next_page: None,
            prev_page: None,
            fixed_weeks: false,
            min: None,
            max: None,
            first_weekday: None,
            weekday_format: WeekdayFormat::Narrow,
            disable_outside_view: false,
            availability: Availability::Enabled,
            readonly: false,
            prevent_deselect: false,
            system: CalendarSystem::Gregorian,
            locale: CalendarLocale::EnUs,
            strings: None,
            is_blocked: None,
            is_unavailable: None,
        }
    }
}

impl CalendarOptions {
    /// The weekday a week starts on, 0 for Sunday.
    #[must_use]
    pub fn first_weekday(&self) -> u8 {
        self.first_weekday
            .filter(|d| *d < 7)
            .unwrap_or_else(|| self.locale.first_weekday())
    }

    /// The strings assistive technology is told.
    #[must_use]
    pub fn strings(&self) -> DateStrings {
        self.strings.clone().unwrap_or_else(|| self.locale.strings())
    }

    /// Blocked at the calendar's level: before `min`, after `max`, by the
    /// caller, or the calendar disabled (`isDateDisabled`).
    #[must_use]
    pub fn blocks(&self, day: CalendarDate) -> bool {
        self.availability == Availability::Disabled
            || self.min.is_some_and(|m| day < m)
            || self.max.is_some_and(|m| day > m)
            || self.is_blocked.as_ref().is_some_and(|f| f.call(day))
    }

    /// Marked unavailable by the caller.
    #[must_use]
    pub fn unavailable(&self, day: CalendarDate) -> bool {
        self.is_unavailable.as_ref().is_some_and(|f| f.call(day))
    }

    /// `day` held to `min..=max`.
    #[must_use]
    pub fn clamp(&self, day: CalendarDate) -> CalendarDate {
        let day = self.min.map_or(day, |m| day.max(m));
        self.max.map_or(day, |m| day.min(m))
    }
}

/// What a day's cell is, for its look and its behaviour: what reka's cell
/// trigger reads into its `data-*` attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Day {
    pub date: CalendarDate,
    /// The first day of the month whose grid the cell is in.
    pub month: CalendarDate,
    /// `data-today`.
    pub today: bool,
    /// `data-outside-view`: a day of the month before or after, padding
    /// this grid's weeks.
    pub outside_month: bool,
    /// `data-outside-visible-view`: in no month shown.
    pub outside_view: bool,
    /// `data-disabled`: blocked, or outside the month with
    /// `disable_outside_view`.
    pub blocked: bool,
    /// `data-unavailable`.
    pub unavailable: bool,
}

impl Day {
    /// How usable the day's trigger is: a blocked day takes no focus and is
    /// not picked, an unavailable one takes focus and is not picked.
    #[must_use]
    pub fn availability(self) -> Availability {
        if self.blocked {
            Availability::Disabled
        } else if self.unavailable {
            Availability::Unavailable
        } else {
            Availability::Enabled
        }
    }
}

/// What a key on a day did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellKey {
    /// It moved the anchor, or was a navigation key that could not.
    Moved,
    /// Enter or Space: pick the day.
    Pick,
    /// Not the grid's: let it go on to the page.
    Ignored,
}

/// A calendar's view and keyboard, shared by [`Calendar`] and
/// [`crate::range_calendar::RangeCalendar`]. Made once in setup, `Copy`.
#[derive(Clone, Copy)]
pub struct CalendarGrid {
    options: Computed<CalendarOptions>,
    /// The first day of the first month shown.
    first: Signal<CalendarDate>,
    /// The day the view was made from (reka's `grid[0].value`): what a
    /// page function turns from.
    base: Signal<CalendarDate>,
    /// reka's `placeholder`.
    anchor: Signal<CalendarDate>,
    /// The anchor's cell; it moves with the anchor.
    focus: FocusHandle,
    prev: FocusHandle,
    next: FocusHandle,
    /// Read at each key: ArrowLeft and ArrowRight swap right to left.
    direction: ReadDirection,
}

impl PartialEq for CalendarGrid {
    fn eq(&self, other: &Self) -> bool {
        self.anchor == other.anchor
    }
}

impl fmt::Debug for CalendarGrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CalendarGrid").finish_non_exhaustive()
    }
}

impl CalendarGrid {
    /// The view of `options` on the month of `anchor`, its arrows by
    /// `direction`, made in setup. A new calendar system or locale makes the
    /// view again from the anchor, which stays (`useCalendar.ts:316-324`).
    #[must_use]
    #[track_caller]
    pub fn new(
        options: impl Fn() -> CalendarOptions + 'static,
        anchor: CalendarDate,
        direction: ReadDirection,
    ) -> Self {
        let options = Computed::new(options);
        let system = gui::untracked(|| options.with(|o| o.system));
        let grid = Self {
            options,
            first: Signal::labeled("calendar first month", system.start_of_month(anchor)),
            base: Signal::labeled("calendar view base", anchor),
            anchor: Signal::labeled("calendar anchor", anchor),
            focus: FocusHandle::new(),
            prev: FocusHandle::new(),
            next: FocusHandle::new(),
            direction,
        };
        watch(
            move || options.with(|o| (o.system, o.locale)),
            move |_, _| grid.show(grid.anchor.get_untracked()),
        );
        grid
    }

    /// The options (tracked).
    #[must_use]
    pub fn options(self) -> CalendarOptions {
        self.options.get()
    }

    fn with<R>(self, f: impl FnOnce(&CalendarOptions) -> R) -> R {
        self.options.with(f)
    }

    /// The options, untracked: what a key or a click reads.
    fn peek<R>(self, f: impl FnOnce(&CalendarOptions) -> R) -> R {
        gui::untracked(|| self.options.with(f))
    }

    /// The anchor (tracked).
    #[must_use]
    pub fn anchor(self) -> CalendarDate {
        self.anchor.get()
    }

    /// The focus handle of the anchor's cell.
    #[must_use]
    pub fn focus_handle(self) -> FocusHandle {
        self.focus
    }

    /// Make the view from `day`: its month first.
    fn show(self, day: CalendarDate) {
        let system = self.peek(|o| o.system);
        self.first.set(system.start_of_month(day));
        self.base.set(day);
    }

    /// Move the anchor to `day` and bring its month into view, as reka's
    /// `placeholder` watcher does: the first month shown becomes `day`'s
    /// when `day` is in none of them.
    pub fn set_anchor(self, day: CalendarDate) {
        if !self.in_view_untracked(day) {
            self.show(day);
        }
        self.anchor.set(day);
    }

    /// Move the anchor to `day` and give its cell focus.
    pub fn focus_day(self, day: CalendarDate, by_keyboard: bool) {
        self.set_anchor(day);
        focus::set_focus(Some(self.focus.id()), by_keyboard);
    }

    /// The first day of each month shown (tracked): reka's `grid` values.
    #[must_use]
    pub fn months(self) -> Vec<CalendarDate> {
        let first = self.first.get();
        self.with(|o| {
            (0..i64::from(o.months.max(1)))
                .map(|i| o.system.add_months(first, i))
                .collect()
        })
    }

    /// The weeks of the grid of `month` (tracked).
    #[must_use]
    pub fn weeks(self, month: CalendarDate) -> Vec<[CalendarDate; 7]> {
        self.with(|o| month_weeks(o.system, o.first_weekday(), month, o.fixed_weeks))
    }

    /// The weekdays' names over the columns, in order (tracked).
    #[must_use]
    pub fn weekdays(self) -> [&'static str; 7] {
        self.with(|o| {
            let first = o.first_weekday();
            std::array::from_fn(|i| {
                let day = u8::try_from(i)
                    .unwrap_or(0)
                    .saturating_add(first)
                    .checked_rem(7)
                    .unwrap_or(0);
                o.locale.weekday(day, o.weekday_format)
            })
        })
    }

    /// The months shown, as words (tracked): "January 1980", "January -
    /// February 1980", "December 1979 - January 1980"
    /// (`Calendar/useCalendar.ts:326-352`).
    #[must_use]
    pub fn heading(self) -> String {
        let first = self.first.get();
        self.with(|o| {
            let (system, locale) = (o.system, o.locale);
            let last = system.add_months(first, i64::from(o.months.max(1).saturating_sub(1)));
            if system.same_month(first, last) {
                return locale.month_and_year(system, first);
            }
            let month = |d| locale.month(system, d, MonthWidth::Long);
            let (first_year, last_year) = (locale.year(system, first), locale.year(system, last));
            if first_year == last_year {
                format!("{} - {} {last_year}", month(first), month(last))
            } else {
                format!("{} {first_year} - {} {last_year}", month(first), month(last))
            }
        })
    }

    /// The calendar's name to assistive technology: "Event Date, January
    /// 1980" (`fullCalendarLabel`, tracked).
    #[must_use]
    pub fn label(self) -> String {
        let name = self.with(|o| {
            o.strings
                .as_ref()
                .map_or_else(|| o.locale.strings().calendar, |s| s.calendar.clone())
        });
        format!("{name}, {}", self.heading())
    }

    /// `day`'s number as its cell shows it, in the locale's digits.
    #[must_use]
    pub fn number(self, day: CalendarDate) -> String {
        self.with(|o| o.locale.number(i64::from(o.system.fields(day).day)))
    }

    /// Whether `day` is in a month shown (tracked).
    #[must_use]
    pub fn in_view(self, day: CalendarDate) -> bool {
        let first = self.first.get();
        self.with(|o| Self::view_holds(o, first, day))
    }

    fn in_view_untracked(self, day: CalendarDate) -> bool {
        let first = self.first.get_untracked();
        self.peek(|o| Self::view_holds(o, first, day))
    }

    fn view_holds(o: &CalendarOptions, first: CalendarDate, day: CalendarDate) -> bool {
        let last = o
            .system
            .end_of_month(o.system.add_months(first, i64::from(o.months.max(1).saturating_sub(1))));
        (first..=last).contains(&day)
    }

    /// Blocked at the calendar's level (tracked).
    #[must_use]
    pub fn blocks(self, day: CalendarDate) -> bool {
        self.with(|o| o.blocks(day))
    }

    /// Marked unavailable (tracked).
    #[must_use]
    pub fn unavailable(self, day: CalendarDate) -> bool {
        self.with(|o| o.unavailable(day))
    }

    /// The day `date` as its cell in the grid of `month` is (tracked).
    #[must_use]
    pub fn day(self, date: CalendarDate, month: CalendarDate) -> Day {
        let in_view = self.in_view(date);
        self.with(|o| {
            let outside_month = !o.system.same_month(date, month);
            Day {
                date,
                month,
                today: date == o.today,
                outside_month,
                outside_view: !in_view,
                blocked: o.blocks(date) || (o.disable_outside_view && outside_month),
                unavailable: o.unavailable(date),
            }
        })
    }

    /// Whether the anchor can take the Tab stop: shown, not blocked, not
    /// unavailable (`isPlaceholderFocusable`).
    fn anchor_focusable(self) -> bool {
        let anchor = self.anchor.get_untracked();
        self.in_view_untracked(anchor) && self.peek(|o| !o.blocks(anchor) && !o.unavailable(anchor))
    }

    /// The first day of the view that is neither blocked nor unavailable,
    /// from `min` in `min`'s month (`firstFocusableDate`, its month test
    /// fixed: a month is skipped only when it ends before `min`).
    #[must_use]
    pub fn first_focusable(self) -> Option<CalendarDate> {
        let first = self.first.get_untracked();
        self.peek(|o| {
            (0..i64::from(o.months.max(1))).find_map(|i| {
                let month = o.system.add_months(first, i);
                let end = o.system.end_of_month(month);
                if o.min.is_some_and(|m| end < m) {
                    return None;
                }
                let start = o.min.filter(|m| o.system.same_month(*m, month)).unwrap_or(month);
                let days = start.days_until(end);
                (0..=days)
                    .map(|n| start.add_days(n))
                    .find(|d| !o.blocks(*d) && !o.unavailable(*d))
            })
        })
    }

    /// Whether `day`, in the grid of `month`, is the Tab stop, given the
    /// stop the owner falls back to when the anchor cannot take it
    /// (`isFocusedDate`). Read untracked, as Tab looks.
    #[must_use]
    pub fn is_stop(
        self,
        day: CalendarDate,
        month: CalendarDate,
        fallback: impl FnOnce() -> Option<CalendarDate>,
    ) -> bool {
        let (outside, blocked, disabled) = self.peek(|o| {
            let outside = !o.system.same_month(day, month);
            (
                outside,
                o.blocks(day) || (o.disable_outside_view && outside),
                o.availability == Availability::Disabled,
            )
        });
        if outside || blocked || disabled {
            return false;
        }
        if self.anchor_focusable() {
            return day == self.anchor.get_untracked();
        }
        fallback() == Some(day)
    }

    /// The months Prev (`step` −1) or Next (+1) turns by.
    fn page_months(self) -> i64 {
        self.peek(|o| if o.paged { i64::from(o.months.max(1)) } else { 1 })
    }

    /// Whether the view can turn `months` months (tracked): Next is
    /// blocked when the first day after the last month shown would be
    /// after `max`, Prev when the last day before the first would be before
    /// `min`; a disabled calendar turns nowhere (`isNextButtonDisabled`).
    #[must_use]
    pub fn can_turn(self, months: i64) -> bool {
        let first = self.first.get();
        self.with(|o| {
            if o.availability == Availability::Disabled {
                return false;
            }
            let shown = i64::from(o.months.max(1));
            if months > 0 {
                let after = o
                    .system
                    .add_months(first, shown.saturating_add(months).saturating_sub(1));
                o.max.is_none_or(|m| o.system.start_of_month(after) <= m)
            } else {
                let before = o.system.end_of_month(o.system.add_months(first, months));
                o.min.is_none_or(|m| before >= m)
            }
        })
    }

    /// Whether Prev (`step` < 0) or Next can page (tracked), by the root's
    /// page function or a month (every month shown, `paged`).
    #[must_use]
    pub fn can_page(self, step: i64) -> bool {
        self.can_page_by(step, None)
    }

    /// The page function Prev (`step` < 0) or Next turns by: the button's
    /// own, else the root's; none for months.
    fn page_fn(
        self,
        step: i64,
        own: Option<&Derive<CalendarDate, CalendarDate>>,
    ) -> Option<Derive<CalendarDate, CalendarDate>> {
        own.cloned().or_else(|| {
            self.peek(|o| {
                if step < 0 {
                    o.prev_page.clone()
                } else {
                    o.next_page.clone()
                }
            })
        })
    }

    /// Whether Prev (`step` < 0) or Next can page by `own`, else the root's
    /// function, else a month (tracked). With a function, Next is blocked
    /// when the day it gives from the last month shown, on its month's
    /// first (a turn of a week or more) and its year's first month (of a
    /// month or more), is after `max`; Prev when the day it gives from the
    /// first, on its month's last and its year's last month, is before `min`
    /// (`handleNextDisabled`, `handlePrevDisabled`).
    #[must_use]
    pub fn can_page_by(self, step: i64, own: Option<&Derive<CalendarDate, CalendarDate>>) -> bool {
        let Some(f) = self.page_fn(step, own) else {
            return self.can_turn(step.signum().saturating_mul(gui::untracked(|| self.page_months())));
        };
        let base = self.base.get();
        self.with(|o| {
            if o.availability == Availability::Disabled {
                return false;
            }
            let system = o.system;
            if step > 0 {
                let Some(max) = o.max else { return true };
                let last = system.add_months(base, i64::from(o.months.max(1).saturating_sub(1)));
                let mut to = f.call(last);
                let diff = last.days_until(to);
                if diff >= 7 {
                    to = system.start_of_month(to);
                }
                if diff >= i64::from(days_in_month(system, last)) {
                    to = system.with_month(to, 1);
                }
                to <= max
            } else {
                let Some(min) = o.min else { return true };
                let mut to = f.call(base);
                let diff = to.days_until(base);
                if diff >= i64::from(days_in_month(system, base)) {
                    let year = system.fields(to).year;
                    to = system.with_month(to, system.months_in_year(year));
                }
                if diff >= 7 {
                    to = system.end_of_month(to);
                }
                to >= min
            }
        })
    }

    /// Turn the view `months` months; the anchor goes to the first day of
    /// the first month shown.
    fn turn(self, months: i64) {
        let system = self.peek(|o| o.system);
        let base = system.add_months(self.base.get_untracked(), months);
        self.show(base);
        self.anchor.set(system.start_of_month(base));
    }

    /// Prev (`step` < 0) or Next: a page, when it can.
    pub fn page(self, step: i64) {
        self.page_by(step, None);
    }

    /// Prev (`step` < 0) or Next by `own`, else the root's page function,
    /// else a month: a page, when it can. By the button's own function the
    /// anchor is the day it gives; by the root's, that day on its month's
    /// first when the view turned a month or more, and on its year's first
    /// month when a year or more (`useCalendar.ts:212-302`).
    pub fn page_by(self, step: i64, own: Option<&Derive<CalendarDate, CalendarDate>>) {
        if !gui::untracked(|| self.can_page_by(step, own)) {
            return;
        }
        let Some(f) = self.page_fn(step, own) else {
            self.turn(step.signum().saturating_mul(self.page_months()));
            return;
        };
        let system = self.peek(|o| o.system);
        let base = self.base.get_untracked();
        let to = f.call(base);
        self.show(to);
        let mut anchor = to;
        if own.is_none() {
            let diff = base.days_until(to).abs();
            if diff >= i64::from(days_in_month(system, base)) {
                anchor = system.start_of_month(anchor);
            }
            if diff >= 365 {
                anchor = system.with_month(anchor, 1);
            }
        }
        self.anchor.set(anchor);
    }

    /// Show the month `month` (1-based) of the shown year, the anchor
    /// keeping its day where it can: a month select (shadcn's heading).
    pub fn set_month(self, month: u8) {
        let system = self.peek(|o| o.system);
        let anchor = system.with_month(self.anchor.get_untracked(), month);
        self.show(system.with_month(self.base.get_untracked(), month));
        self.set_anchor(anchor);
    }

    /// Show the shown month of `year`, the anchor keeping its month and
    /// day where that year has them: a year select.
    pub fn set_year(self, year: i32) {
        let system = self.peek(|o| o.system);
        let anchor = system.with_year(self.anchor.get_untracked(), year);
        self.show(system.with_year(self.base.get_untracked(), year));
        self.set_anchor(anchor);
    }

    /// The first month shown (tracked).
    #[must_use]
    pub fn first_month(self) -> CalendarDate {
        self.first.get()
    }

    /// An arrow key: `add` days on from `day`, past blocked days, paging
    /// when the day is not shown; staying when it would pass `min` or
    /// `max`, or the page cannot turn (`shiftFocus`).
    fn shift(self, day: CalendarDate, add: i64) {
        let mut at = day;
        for _ in 0..MAX_ARROW_STEPS {
            let next = at.add_days(add);
            let (out, blocked) = self.peek(|o| {
                (
                    o.min.is_some_and(|m| next < m) || o.max.is_some_and(|m| next > m),
                    o.blocks(next),
                )
            });
            if out || next == at {
                return;
            }
            if !self.in_view_untracked(next) {
                let months = add.signum().saturating_mul(self.page_months());
                if !gui::untracked(|| self.can_turn(months)) {
                    return;
                }
                self.turn(months);
                if !self.in_view_untracked(next) {
                    self.set_anchor(next);
                }
            }
            if blocked {
                at = next;
                continue;
            }
            self.focus_day(next, true);
            return;
        }
    }

    /// Home, End, PageUp, PageDown: `target`, reached going `outward`,
    /// held to `min..=max`; then, when it cannot take focus, the days on
    /// from it, at most [`MAX_FOCUS_RETRIES`] (`focusDate`, `tryFocusDate`).
    /// They run inward from a bound the target was held to (forward from
    /// `min` going back, back from `max` going forward); else back toward
    /// the day for a week's end (`week`), so focus stays in the week, and on
    /// outward for a page.
    fn seek(self, target: CalendarDate, outward: i64, week: bool) {
        let (clamped, min, max) = self.peek(|o| (o.clamp(target), o.min, o.max));
        let at_min = min == Some(clamped);
        let at_max = max == Some(clamped);
        let direction = if !(at_min || at_max) {
            if week { outward.saturating_neg() } else { outward }
        } else if at_min && outward < 0 {
            1
        } else if at_max && outward > 0 {
            -1
        } else {
            0
        };
        let mut at = clamped;
        self.set_anchor(at);
        for _ in 0..MAX_FOCUS_RETRIES {
            let usable = self.peek(|o| !o.blocks(at) && !o.unavailable(at));
            if usable {
                self.focus_day(at, true);
                return;
            }
            if direction == 0 {
                return;
            }
            let next = self.peek(|o| o.clamp(at.add_days(direction)));
            if next == at {
                return;
            }
            at = next;
            self.set_anchor(at);
        }
    }

    /// What a key does on the day `day` (in the grid of `month`). A
    /// blocked day takes no keys (it cannot hold focus anyway).
    pub fn key(self, day: Day, ev: &KeyEvent) -> CellKey {
        if day.blocked {
            return CellKey::Ignored;
        }
        let m = ev.modifiers;
        let (first_weekday, system) = self.peek(|o| (o.first_weekday(), o.system));
        let forward: i64 = if self.direction.get() == Direction::Rtl { -1 } else { 1 };
        if ev.is(NamedKey::Enter) || ev.is(NamedKey::Space) || ev.text.as_deref() == Some(" ") {
            return if m.control || m.alt || m.command {
                CellKey::Ignored
            } else {
                CellKey::Pick
            };
        }
        if m.control || m.alt || m.command {
            return CellKey::Ignored;
        }
        let d = day.date;
        if ev.is(NamedKey::ArrowRight) {
            self.shift(d, forward);
        } else if ev.is(NamedKey::ArrowLeft) {
            self.shift(d, forward.saturating_neg());
        } else if ev.is(NamedKey::ArrowUp) {
            self.shift(d, -7);
        } else if ev.is(NamedKey::ArrowDown) {
            self.shift(d, 7);
        } else if ev.is(NamedKey::Home) {
            // Toward the boundary; when it cannot take focus, back toward the day.
            self.seek(start_of_week(d, first_weekday), -1, true);
        } else if ev.is(NamedKey::End) {
            self.seek(end_of_week(d, first_weekday), 1, true);
        } else if ev.is(NamedKey::PageUp) || ev.is(NamedKey::PageDown) {
            let sign: i64 = if ev.is(NamedKey::PageDown) { 1 } else { -1 };
            let months = if m.shift { 12 } else { 1 };
            let target = system.add_months(d, sign.saturating_mul(months));
            let target = self.peek(|o| o.clamp(target));
            if !self.in_view_untracked(target) {
                let turn = sign.saturating_mul(months);
                if gui::untracked(|| self.can_turn(turn)) {
                    // reka's key page function: the month's first, a month or a year on.
                    self.show(system.start_of_month(system.add_months(self.base.get_untracked(), turn)));
                }
            }
            self.seek(target, sign, false);
        } else {
            return CellKey::Ignored;
        }
        CellKey::Moved
    }

    /// The day initial focus goes to (`handleCalendarInitialFocus`): the
    /// first of `picked` shown in its month and focusable, else today if it
    /// is, else the first day of the view not blocked.
    #[must_use]
    pub fn initial(self, picked: impl IntoIterator<Item = CalendarDate>) -> Option<CalendarDate> {
        let usable = |d: CalendarDate| self.in_view_untracked(d) && !self.peek(|o| o.blocks(d));
        picked
            .into_iter()
            .find(|d| usable(*d))
            .or_else(|| Some(self.peek(|o| o.today)).filter(|d| usable(*d)))
            .or_else(|| {
                let first = self.first.get_untracked();
                self.peek(|o| {
                    let end = o
                        .system
                        .end_of_month(o.system.add_months(first, i64::from(o.months.max(1).saturating_sub(1))));
                    (0..=first.days_until(end))
                        .map(|n| first.add_days(n))
                        .find(|d| !o.blocks(*d))
                })
            })
    }

    /// `d` as the calendar's root: a region named for the calendar and its
    /// heading, holding a hidden level-2 heading that repeats it
    /// (`Calendar/CalendarRoot.vue:349,364-373`). Called in the render.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        let label = self.label();
        d.role(Role::Group)
            .aria_label(label.clone())
            .child(visually_hidden(div().role(Role::Heading).aria_level(2), Hidden::Visually).child(text(label)))
    }

    /// `d` as a month's grid: an application to assistive technology, read
    /// as disabled when the calendar is and as read-only when it is
    /// (`Calendar/CalendarGrid.vue:22-25`).
    #[must_use]
    pub fn grid(self, d: Div) -> Div {
        let (unusable, readonly) = self.with(|o| (o.availability != Availability::Enabled, o.readonly));
        d.role(Role::Application).unavailable(unusable).aria_readonly(readonly)
    }

    /// `d` as the row of weekday names, hidden from assistive technology
    /// (`Calendar/CalendarGridHead.vue:16`).
    #[must_use]
    pub fn head(self, d: Div) -> Div {
        d.aria_hidden(true)
    }

    /// `d` as a week's row.
    #[must_use]
    pub fn row(self, d: Div) -> Div {
        d.role(Role::Row)
    }

    /// `d` as the cell of `day`: a grid cell, selected when `selected`, read
    /// as disabled when the day is blocked or unavailable. Unlike reka,
    /// `disable_outside_view` disables only the cells outside their month.
    #[must_use]
    pub fn cell(self, day: Day, selected: bool, d: Div) -> Div {
        d.role(Role::GridCell)
            .aria_selected(selected)
            .unavailable(day.blocked || day.unavailable)
    }

    /// `d` as the trigger of `day`: a button named by the full date
    /// ("Sunday, January 20, 1980") showing the day's number; blocked, it
    /// takes no focus; unavailable, it takes focus and is not picked. The
    /// anchor's cell takes the grid's focus handle, the Tab stop is the one
    /// `is_stop` names, and keys go to [`Self::key`], then `on_pick` on
    /// Enter or Space. A click calls `on_pick` and gives the day focus.
    /// Called in the render, it reads the grid (tracked).
    #[must_use]
    pub fn trigger(
        self,
        day: Day,
        d: Div,
        is_stop: impl Fn() -> bool + 'static,
        on_pick: impl Into<gui::Callback<CalendarDate>>,
    ) -> Div {
        let on_pick = on_pick.into();
        let label = self.with(|o| o.locale.day_label(o.system, day.date));
        let anchored = !day.outside_month && day.date == self.anchor.get();
        let d = day.availability().apply(d.role(Role::Button).aria_label(label));
        let d = if anchored { d.track_focus(self.focus) } else { d };
        let pick = on_pick.clone();
        d.tab_stop(is_stop)
            .on_click(move |_| {
                pick.call(day.date);
                self.focus_day(day.date, false);
            })
            .on_key_down(move |ev| match self.key(day, ev) {
                CellKey::Moved => true,
                CellKey::Pick => {
                    on_pick.call(day.date);
                    true
                }
                CellKey::Ignored => false,
            })
    }

    /// `d` as Prev (`step` −1) or Next (+1): named by the locale's "Previous
    /// page" or "Next page", disabled (out of Tab order) when the view
    /// cannot turn that way. A press turns it; when that leaves it unable
    /// to turn further while it holds focus, focus goes to the other
    /// button, which the turn made usable (reka lets focus drop to the
    /// page). Called in the render (tracked).
    #[must_use]
    pub fn page_button(self, step: i64, d: Div) -> Div {
        self.page_button_by(step, None, d)
    }

    /// `d` as Prev (`step` −1) or Next (+1) paging by its own function
    /// `page` (reka's `CalendarPrev`/`CalendarNext` `prevPage`/`nextPage`),
    /// over the root's; see [`Self::page_button`]. Called in the render
    /// (tracked).
    #[must_use]
    pub fn page_button_by(self, step: i64, page: Option<Derive<CalendarDate, CalendarDate>>, d: Div) -> Div {
        let (own, other) = if step < 0 {
            (self.prev, self.next)
        } else {
            (self.next, self.prev)
        };
        let label = self.with(|o| {
            let s = o.strings();
            if step < 0 { s.previous_page } else { s.next_page }
        });
        let usable = self.can_page_by(step, page.as_ref());
        d.role(Role::Button)
            .aria_label(label)
            .track_focus(own)
            .disabled(!usable)
            .on_click(move |_| {
                self.page_by(step, page.as_ref());
                if own.is_focused() && !gui::untracked(|| self.can_page_by(step, page.as_ref())) {
                    other.focus();
                }
            })
    }
}

/// The days in `day`'s month, in `system`.
fn days_in_month(system: CalendarSystem, day: CalendarDate) -> u8 {
    let f = system.fields(day);
    system.days_in_month(f.year, f.month)
}

/// What a calendar picks: one day or many. Two types, no `multiple` flag.
pub trait Picked: Clone + PartialEq + 'static {
    /// Whether `day` is picked.
    fn has(&self, day: CalendarDate) -> bool;
    /// The value after `day` is picked; none when it stays as it is.
    fn pick(&self, day: CalendarDate, prevent_deselect: bool) -> Option<Self>;
    /// The days picked, in the order they were.
    fn days(&self) -> Vec<CalendarDate>;
}

/// One day, or none: picking another replaces it, picking it again clears
/// it (`Calendar/CalendarRoot.vue:265-276`).
impl Picked for Option<CalendarDate> {
    fn has(&self, day: CalendarDate) -> bool {
        *self == Some(day)
    }

    fn pick(&self, day: CalendarDate, prevent_deselect: bool) -> Option<Self> {
        match self {
            Some(d) if *d == day && prevent_deselect => None,
            Some(d) if *d == day => Some(None),
            _ => Some(Some(day)),
        }
    }

    fn days(&self) -> Vec<CalendarDate> {
        self.iter().copied().collect()
    }
}

/// Many days, in the order they were picked: a new one goes last, a picked
/// one is taken out (`Calendar/CalendarRoot.vue:277-294`). The empty list
/// is "nothing picked".
impl Picked for Vec<CalendarDate> {
    fn has(&self, day: CalendarDate) -> bool {
        self.contains(&day)
    }

    fn pick(&self, day: CalendarDate, prevent_deselect: bool) -> Option<Self> {
        if !self.contains(&day) {
            let mut v = self.clone();
            v.push(day);
            return Some(v);
        }
        (!prevent_deselect).then(|| self.iter().copied().filter(|d| *d != day).collect())
    }

    fn days(&self) -> Vec<CalendarDate> {
        self.clone()
    }
}

/// A calendar's state: its grid and its value, read and written through
/// the part's [`UseModel`]. Made once in setup, `Copy`.
pub struct Calendar<P: 'static, V: 'static> {
    pub grid: CalendarGrid,
    value: UseModel<P, V>,
}

impl<P: 'static, V: 'static> Clone for Calendar<P, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static, V: 'static> Copy for Calendar<P, V> {}

impl<P: 'static, V: 'static> PartialEq for Calendar<P, V> {
    fn eq(&self, other: &Self) -> bool {
        self.grid == other.grid
    }
}

impl<P: 'static, V: 'static> fmt::Debug for Calendar<P, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Calendar").finish_non_exhaustive()
    }
}

impl<P: 'static, V: Picked> Calendar<P, V> {
    /// A calendar showing `value` (the part's `v-model`) with `options`,
    /// its arrows by `direction`, called in the setup that made it. The anchor starts at the last day
    /// picked, else `anchor` (reka's `defaultPlaceholder`), else today; it
    /// follows the value to each new last day picked, which brings its
    /// month into view (`Calendar/CalendarRoot.vue:254-262`).
    #[must_use]
    #[track_caller]
    pub fn new(
        value: UseModel<P, V>,
        options: impl Fn() -> CalendarOptions + 'static,
        anchor: Option<CalendarDate>,
        direction: ReadDirection,
    ) -> Self {
        let options = Computed::new(options);
        let today = gui::untracked(|| options.with(|o| o.today));
        let start = value.with_untracked_days().last().copied().or(anchor).unwrap_or(today);
        let grid = CalendarGrid::new(move || options.get(), start, direction);
        let calendar = Self { grid, value };
        watch(
            move || value.with(|v| v.days().last().copied()),
            move |last, _| {
                if let Some(day) = *last
                    && day != grid.anchor.get_untracked()
                {
                    grid.set_anchor(day);
                }
            },
        );
        calendar
    }

    /// The value (tracked).
    #[must_use]
    pub fn value(self) -> V {
        self.value.get()
    }

    /// Whether `day` is picked (tracked).
    #[must_use]
    pub fn has(self, day: CalendarDate) -> bool {
        self.value.with(|v| v.has(day))
    }

    /// Whether a picked day is blocked or unavailable (`data-invalid`,
    /// tracked): a caller's value is kept as it is and only marked so.
    #[must_use]
    pub fn invalid(self) -> bool {
        let days = self.value.with(V::days);
        self.grid
            .with(|o| days.iter().any(|d| o.blocks(*d) || o.unavailable(*d)))
    }

    /// Pick `day` as a click, Enter or Space does: nothing when the
    /// calendar is read-only or unavailable, or the day blocked or
    /// unavailable; else the value after it, and, when that clears the last
    /// day, the anchor to `day`.
    pub fn pick(self, day: CalendarDate) {
        let (blocked, readonly, prevent) = self.grid.peek(|o| {
            (
                o.blocks(day) || o.unavailable(day),
                o.readonly || o.availability != Availability::Enabled,
                o.prevent_deselect,
            )
        });
        if blocked || readonly {
            return;
        }
        let next = self.value.with_untracked_pick(day, prevent);
        if let Some(next) = next {
            if next.days().is_empty() {
                self.grid.set_anchor(day);
            }
            self.value.set(next);
        }
    }

    /// The Tab stop's fallback when the anchor cannot take it: the first
    /// focusable day, when nothing is picked or a picked day is blocked.
    fn fallback(self) -> Option<CalendarDate> {
        let days = self.value.get_untracked_days();
        let blocked = self.grid.peek(|o| days.iter().any(|d| o.blocks(*d)));
        (days.is_empty() || blocked)
            .then(|| self.grid.first_focusable())
            .flatten()
    }

    /// Give the day initial focus goes to focus, without a ring: the picked
    /// day, else today, else the first day not blocked (reka's
    /// `initialFocus`, and what a date picker does as it opens).
    pub fn focus_initial(self) {
        let days = self.value.get_untracked_days();
        if let Some(day) = self.grid.initial(days) {
            self.grid.focus_day(day, false);
        }
    }

    /// `d` as the cell of `day` (see [`CalendarGrid::cell`]).
    #[must_use]
    pub fn cell(self, day: Day, d: Div) -> Div {
        self.grid.cell(day, self.has(day.date), d)
    }

    /// `d` as the trigger of `day` (see [`CalendarGrid::trigger`]): it picks.
    #[must_use]
    pub fn trigger(self, day: Day, d: Div) -> Div {
        let month = day.month;
        self.grid.trigger(
            day,
            d,
            move || self.grid.is_stop(day.date, month, || self.fallback()),
            move |date| self.pick(date),
        )
    }
}

/// Reading a calendar's value without tracking.
trait ValueDays<V> {
    fn with_untracked_days(&self) -> Vec<CalendarDate>;
    fn get_untracked_days(&self) -> Vec<CalendarDate>;
    fn with_untracked_pick(&self, day: CalendarDate, prevent: bool) -> Option<V>;
}

impl<P: 'static, V: Picked> ValueDays<V> for UseModel<P, V> {
    fn with_untracked_days(&self) -> Vec<CalendarDate> {
        gui::untracked(|| self.with(V::days))
    }

    fn get_untracked_days(&self) -> Vec<CalendarDate> {
        self.with_untracked_days()
    }

    fn with_untracked_pick(&self, day: CalendarDate, prevent: bool) -> Option<V> {
        gui::untracked(|| self.with(|v| v.pick(day, prevent)))
    }
}
