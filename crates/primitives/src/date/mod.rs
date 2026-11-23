//! The date utilities every date primitive shares (reka's `date/` and
//! `shared/date/`, `@internationalized/date` underneath): a day
//! ([`CalendarDate`]), the calendar systems it is counted in
//! ([`system`]), what a locale reads it out as ([`locale`]), a month's grid
//! of weeks ([`month_weeks`]), week numbers, and the ISO strings a form
//! sends.
//!
//! A day is one day whatever the system: a [`CalendarDate`] is a day of the
//! proleptic Gregorian calendar, and a [`CalendarSystem`] reads its year,
//! month and day in that system and finds the day that has them. reka's
//! dates carry their calendar and, for a field, a time and a zone; here the
//! calendar is the caller's option and a time is the field's segments
//! ([`crate::date_field::DateEntry`]), so comparing two days never depends
//! on their type (reka's "same day" and "same value" disagree on it,
//! `date/comparators.ts:193-225`).

use std::num::Saturating;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod locale;
pub mod system;

pub use locale::{CalendarLocale, DatePart, DateStrings, MonthWidth, WeekdayFormat, digit_value};
pub use system::{CalendarSystem, Era, SystemDate};

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

impl CalendarDate {
    /// The ISO 8601 day a form sends (`YYYY-MM-DD`, the year padded to four
    /// digits), always in the Gregorian calendar, as `<input type="date">`'s
    /// `value` is. reka writes the fields of the value's own calendar
    /// (`shared/date/utils.ts:99-101`), so a Persian date posts a Persian
    /// year as if it were Gregorian; that is not kept (ADR-0025, group 7).
    #[must_use]
    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// The day an ISO 8601 date names (`2026-11-22`), if it is one.
    #[must_use]
    pub fn parse_iso(value: &str) -> Option<Self> {
        let mut parts = value.splitn(3, '-');
        let year = parts.next()?.parse().ok()?;
        let month = parts.next()?.parse().ok()?;
        let day = parts.next()?.get(..2)?.parse().ok()?;
        Self::new(year, month, day)
    }

    /// Whole days from `self` to `other` (negative when `other` is earlier).
    #[must_use]
    pub fn days_until(self, other: Self) -> i64 {
        other.days().saturating_sub(self.days())
    }
}

/// The weeks a month's grid shows (reka's `createMonth`): the days of the
/// month `month` is in (counted in `system`), the days of the month before
/// from the start of its first week, the days of the month after to the end
/// of its last; with `fixed_weeks`, more of the month after until there are
/// six weeks (42 days). Weeks start on `first_weekday`, 0 for Sunday.
#[must_use]
pub fn month_weeks(
    system: CalendarSystem,
    first_weekday: u8,
    month: CalendarDate,
    fixed_weeks: bool,
) -> Vec<[CalendarDate; 7]> {
    let first = system.start_of_month(month);
    let last = system.end_of_month(month);
    let lead = i64::from(first.weekday())
        .saturating_sub(i64::from(first_weekday))
        .rem_euclid(7);
    let mut start = first.add_days(lead.saturating_neg());
    let mut weeks = Vec::with_capacity(6);
    while start <= last || (fixed_weeks && weeks.len() < 6) {
        weeks.push(std::array::from_fn(|d| start.add_days(i64::try_from(d).unwrap_or(0))));
        start = start.add_days(7);
    }
    weeks
}

/// The first day of the week holding `date`, weeks starting on `first_weekday`.
#[must_use]
pub fn start_of_week(date: CalendarDate, first_weekday: u8) -> CalendarDate {
    let into = i64::from(date.weekday())
        .saturating_sub(i64::from(first_weekday))
        .rem_euclid(7);
    date.add_days(into.saturating_neg())
}

/// The last day of the week holding `date`.
#[must_use]
pub fn end_of_week(date: CalendarDate, first_weekday: u8) -> CalendarDate {
    start_of_week(date, first_weekday).add_days(6)
}

/// The number of the week holding `date` (reka's `getWeekNumber`): by ISO
/// 8601 when `iso` (weeks from Monday, week 1 holding January 4), else
/// weeks from `first_weekday` with week 1 holding January 1 (the US rule).
#[must_use]
pub fn week_number(date: CalendarDate, first_weekday: u8, iso: bool) -> u32 {
    let (week_start, anchor_day) = if iso { (1, 4) } else { (first_weekday, 1) };
    let start = start_of_week(date, week_start);
    // The year a week belongs to: the one its anchor day (Thursday for ISO) falls in.
    let year = if iso { start.add_days(3).year() } else { date.year() };
    let first = CalendarDate::new(year, 1, anchor_day).map_or(start, |d| start_of_week(d, week_start));
    let weeks = first.days_until(start).div_euclid(7).saturating_add(1);
    if weeks < 1 && !iso {
        // Before week 1 of its own year: the last week of the year before.
        return CalendarDate::new(year.saturating_sub(1), 12, 31).map_or(1, |d| week_number(d, first_weekday, iso));
    }
    u32::try_from(weeks.max(1)).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::{CalendarDate, CalendarSystem, month_weeks, week_number};

    fn day(y: i32, m: u8, d: u8) -> CalendarDate {
        CalendarDate::new(y, m, d).unwrap_or(CalendarDate::from_days(0))
    }

    #[test]
    fn a_month_grid_pads_its_weeks_and_six_when_fixed() {
        // February 2026 starts on a Sunday and has exactly four weeks.
        let weeks = month_weeks(CalendarSystem::Gregorian, 0, day(2026, 2, 10), false);
        assert_eq!(weeks.len(), 4);
        assert_eq!(weeks.first().map(|w| w[0]), Some(day(2026, 2, 1)));
        let fixed = month_weeks(CalendarSystem::Gregorian, 0, day(2026, 2, 10), true);
        assert_eq!(fixed.len(), 6, "fixedWeeks: 42 days");
        assert_eq!(fixed.last().map(|w| w[6]), Some(day(2026, 3, 14)));
        // Monday first: January 2021 starts with Dec 28.
        let weeks = month_weeks(CalendarSystem::Gregorian, 1, day(2021, 1, 1), false);
        assert_eq!(weeks.first().map(|w| w[0]), Some(day(2020, 12, 28)));
    }

    #[test]
    fn weeks_are_numbered_by_iso_or_from_january_first() {
        // reka's date/calendar.test.ts: 2021-01-01 is ISO week 53 of 2020, US week 1.
        assert_eq!(week_number(day(2021, 1, 1), 1, true), 53);
        assert_eq!(week_number(day(2021, 1, 4), 1, true), 1);
        assert_eq!(week_number(day(2021, 1, 1), 0, false), 1);
        assert_eq!(week_number(day(2021, 1, 3), 0, false), 2);
        assert_eq!(week_number(day(2026, 12, 31), 1, true), 53);
    }

    #[test]
    fn a_form_sends_gregorian_iso_days() {
        assert_eq!(day(987, 3, 4).iso(), "0987-03-04");
        assert_eq!(CalendarDate::parse_iso("2026-11-22"), Some(day(2026, 11, 22)));
        assert_eq!(CalendarDate::parse_iso("2026-02-30"), None);
    }
}

#[cfg(test)]
mod zone_tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::CalendarDate;

    /// Set in the child process `today_follows_the_local_time_zone` runs.
    const PROBE: &str = "PRIMITIVES_TODAY_PROBE";

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
                    "date::zone_tests::today_follows_the_local_time_zone",
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
