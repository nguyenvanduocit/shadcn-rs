//! A date typed segment by segment (reka's `DateField/`), and the segment
//! core every date and time field types by (research §11 "Lõi segment").
//!
//! **The value is the segments** ([`DateEntry`]): year, month and day, and,
//! to the entry's [`Precision`], hour, minute and second, each empty until
//! it is given one, in the entry's [`CalendarSystem`]. A date partly typed
//! is a value of its own, so a caller's write (a form reset's empty entry)
//! always puts the segments back, as ADR-0025 §12 has the time field do.
//! [`DateEntry::date`] is the day once the date segments are whole, and
//! what a form sends ([`DateEntry::iso`]) is empty until every segment is
//! set: the "value written only when complete" of reka, read off the one
//! value. The entry's precision decides which segments there are, as
//! reka's granularity follows its placeholder's type; so no option can
//! disagree with the value.
//!
//! **Segments** run in the locale's order (`formatToParts`): month, day,
//! year in `en-US`; day, month, year in `vi-VN`; then hour, minute, second
//! and, on a 12-hour clock, the day period ([`DateField::parts`]). Each is a
//! Tab stop and a spin button to assistive technology, named in the
//! locale ([`DateStrings`]); literals between them are hidden from it.
//!
//! **Keys** (Chrome's date input, which the verdict compares against, and
//! reka's where Chrome has nothing to say):
//!
//! | Key | What it does |
//! |---|---|
//! | 0–9 (and the locale's digits) | [`type_digit`]: joins the digit typed before it, held to the segment's range; moves on once no further digit could follow |
//! | ArrowUp / ArrowDown | ±the unit's step, wrapping; an empty segment takes its lowest (highest) value, the year the anchor's |
//! | Backspace, Delete | empty the segment |
//! | ArrowLeft / ArrowRight | the segment before or after (swapped right to left); not wrapping; the last hands on to what follows (a picker's trigger) |
//! | A, P (and the locale's first letters) on the day period | AM, PM |
//! | ArrowUp / ArrowDown on the day period | the other half of the day |
//!
//! **The typing rule** is Chrome's (`DateTimeNumericFieldElement`, ADR-0025
//! §12): digits join while they fit the segment's digits, the value is held
//! to the segment's largest ("2" then "5" in a 24-hour hour gives 23; "1",
//! "3" in the month 12); a number below the segment's least (a leading 0 in
//! a month) leaves the segment empty, waiting for the next digit; and the
//! segment moves on once it has all its digits or a further digit could
//! only pass its largest. One rule for every segment, date and time alike.
//! reka starts again from the new digit (2, 5 gives 5) and deletes one
//! digit at a time.
//!
//! What reka gets wrong and this does not (ADR-0025, group 7):
//!
//! - The largest day takes the anchor's year, not the year typed: in a
//!   month-day-year order with a common anchor year, 29 February cannot be
//!   typed (`shared/date/useDateField.ts:652-653`). Here a day is held to
//!   its month in the year typed, or, with no year yet, in any year (29 for
//!   February), and held again once the year is typed.
//! - A 24-hour hour types up to 24 while `aria-valuemax` is 23; a 12-hour
//!   hour reports `aria-valuenow` 13–23 above its `aria-valuemax` 12, and
//!   midnight shows "0" (`useDateField.ts:128-132,756`; `utils.ts:73-79`):
//!   here a 24-hour hour is 0–23 and a 12-hour one is 1–12, value and
//!   range alike.
//! - A value of 0 reads as empty to `aria-valuenow`, which then reads the
//!   anchor's (`useDateField.ts:128,150-152`): here 0 is 0.
//! - The day period on an empty hour writes the hour 12 or −12
//!   (`useDateField.ts:855-871`): here it is kept until an hour is typed.
//! - The era is a spin button whose keys call a handler that does not
//!   exist (`useDateField.ts:232-248,895-906`): here the era is not a
//!   segment; the year is the system's year as the locale shows it.
//! - A form gets the fields of the value's own calendar as if they were
//!   Gregorian (`shared/date/utils.ts:99-101`): here it gets the ISO day.
//! - Digits show in the locale's script while only ASCII ones type: here
//!   both do ([`digit_value`]).
//! - The strings for assistive technology are English only: here they are
//!   the locale's ([`DateStrings`]).
//!
//! Enter is left to the page, so it submits a form, as Chrome's date input
//! does (reka swallows it).

use std::fmt;

use gui::events::{KeyEvent, NamedKey};
use gui::focus::FocusHandle;
use gui::{Computed, Derive, Direction, Div, Role, Signal, UseModel, ValueRange, watch};

use crate::time_field::Time;

use crate::availability::Availability;
use crate::date::{CalendarDate, CalendarLocale, CalendarSystem, DatePart, DateStrings, digit_value};
use crate::direction::ReadDirection;
use crate::form::{FormData, Submit};

/// One editable part of a date or time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Segment {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    /// AM or PM, on a 12-hour clock.
    DayPeriod,
}

impl Segment {
    const ALL: [Self; 7] = [
        Self::Year,
        Self::Month,
        Self::Day,
        Self::Hour,
        Self::Minute,
        Self::Second,
        Self::DayPeriod,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

/// How far below the day an entry goes (reka's `granularity`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Precision {
    /// A date alone.
    #[default]
    Day,
    Hour,
    Minute,
    Second,
}

/// A 12- or 24-hour clock (`hourCycle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HourCycle {
    /// 1–12 with AM and PM.
    H12,
    /// 0–23.
    H23,
}

/// A date and time as a field's segments hold it, each empty until given
/// one, counted in `system`; `precision` says which time segments it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DateEntry {
    pub system: CalendarSystem,
    pub precision: Precision,
    pub year: Option<i32>,
    pub month: Option<u8>,
    pub day: Option<u8>,
    /// 0–23, whatever the clock shows.
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    pub second: Option<u8>,
}

impl Default for DateEntry {
    fn default() -> Self {
        Self::empty(CalendarSystem::Gregorian, Precision::Day)
    }
}

impl DateEntry {
    /// No segment set.
    #[must_use]
    pub const fn empty(system: CalendarSystem, precision: Precision) -> Self {
        Self {
            system,
            precision,
            year: None,
            month: None,
            day: None,
            hour: None,
            minute: None,
            second: None,
        }
    }

    /// `date`'s segments in `system`, the time segments of `precision` empty.
    #[must_use]
    pub fn of(system: CalendarSystem, precision: Precision, date: CalendarDate) -> Self {
        Self::empty(system, precision).with_date(Some(date))
    }

    /// The date segments set to `date` (emptied by none), the time kept.
    #[must_use]
    pub fn with_date(self, date: Option<CalendarDate>) -> Self {
        let f = date.map(|d| self.system.fields(d));
        Self {
            year: f.map(|f| f.year),
            month: f.map(|f| f.month),
            day: f.map(|f| f.day),
            ..self
        }
    }

    /// The time segments set (to the entry's precision).
    #[must_use]
    pub fn with_time(self, hour: Option<u8>, minute: Option<u8>, second: Option<u8>) -> Self {
        Self {
            hour: hour.filter(|h| *h < 24),
            minute: minute.filter(|m| *m < 60),
            second: second.filter(|s| *s < 60),
            ..self
        }
    }

    /// The segments the entry has, in no particular order.
    pub fn segments(self) -> impl Iterator<Item = Segment> {
        let p = self.precision;
        [
            Segment::Year,
            Segment::Month,
            Segment::Day,
            Segment::Hour,
            Segment::Minute,
            Segment::Second,
        ]
        .into_iter()
        .filter(move |s| match s {
            Segment::Hour => p >= Precision::Hour,
            Segment::Minute => p >= Precision::Minute,
            Segment::Second => p >= Precision::Second,
            _ => true,
        })
    }

    /// Segment `s`'s value (the hour 0–23; the day period 0 for AM, 1 for PM).
    #[must_use]
    pub fn get(self, s: Segment) -> Option<i32> {
        match s {
            Segment::Year => self.year,
            Segment::Month => self.month.map(i32::from),
            Segment::Day => self.day.map(i32::from),
            Segment::Hour => self.hour.map(i32::from),
            Segment::Minute => self.minute.map(i32::from),
            Segment::Second => self.second.map(i32::from),
            Segment::DayPeriod => self.hour.map(|h| i32::from(h >= 12)),
        }
    }

    /// Segment `s` set to `v` (no check of its range: see [`DateField`]),
    /// the others as they are: a day past its month is kept until
    /// [`held`](Self::held), so 29 February typed before its year (en-US
    /// types the year last) survives the year's first digits.
    #[must_use]
    pub fn set(self, s: Segment, v: Option<i32>) -> Self {
        let byte = v.and_then(|v| u8::try_from(v).ok());
        let mut e = self;
        match s {
            Segment::Year => e.year = v,
            Segment::Month => e.month = byte,
            Segment::Day => e.day = byte,
            Segment::Hour => e.hour = byte,
            Segment::Minute => e.minute = byte,
            Segment::Second => e.second = byte,
            Segment::DayPeriod => {
                e.hour = e.hour.zip(byte).map(|(h, pm)| {
                    h.checked_rem(12)
                        .unwrap_or(0)
                        .saturating_add(if pm > 0 { 12 } else { 0 })
                });
            }
        }
        e
    }

    /// The day held to its month in its year, once all three are set. A
    /// field holds when a typing ends, on a step, and when focus leaves.
    #[must_use]
    pub fn held(self) -> Self {
        match (self.year, self.month, self.day) {
            (Some(y), Some(m), Some(d)) => {
                let m = m.min(self.system.months_in_year(y));
                Self {
                    month: Some(m),
                    day: Some(d.min(self.system.days_in_month(y, m))),
                    ..self
                }
            }
            _ => self,
        }
    }

    /// The day, once the year, month and day are set.
    #[must_use]
    pub fn date(self) -> Option<CalendarDate> {
        let (y, m, d) = (self.year?, self.month?, self.day?);
        (m >= 1 && m <= self.system.months_in_year(y) && d >= 1 && d <= self.system.days_in_month(y, m))
            .then(|| self.system.date(y, m, d))
    }

    /// Whether every segment it has is set.
    #[must_use]
    pub fn is_complete(self) -> bool {
        self.segments().all(|s| self.get(s).is_some()) && self.date().is_some()
    }

    /// Whether no segment is set.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.segments().all(|s| self.get(s).is_none())
    }

    /// What a form sends, as `<input type="date">` or
    /// `<input type="datetime-local">`: `YYYY-MM-DD`, or
    /// `YYYY-MM-DDTHH:mm`, with `:ss` to the second, the day ISO (Gregorian)
    /// whatever the entry's system; empty while a segment is.
    #[must_use]
    pub fn iso(self) -> String {
        if !self.is_complete() {
            return String::new();
        }
        let Some(date) = self.date() else {
            return String::new();
        };
        let two = |v: Option<u8>| format!("{:02}", v.unwrap_or(0));
        match self.precision {
            Precision::Day => date.iso(),
            Precision::Hour | Precision::Minute => format!("{}T{}:{}", date.iso(), two(self.hour), two(self.minute)),
            Precision::Second => format!(
                "{}T{}:{}:{}",
                date.iso(),
                two(self.hour),
                two(self.minute),
                two(self.second)
            ),
        }
    }
}

/// Sent as a native date input is: its ISO `value`, empty while partly typed.
impl Submit for DateEntry {
    fn form_data(&self) -> FormData {
        FormData::Text(self.iso())
    }
}

/// What a typed digit made of a segment ([`type_digit`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Typed {
    /// The segment's value: none while what was typed is below its least.
    pub value: Option<i32>,
    /// The digits typed so far, and the number they spell.
    pub buffer: (u8, i32),
    /// Whether the segment is done and focus moves on.
    pub done: bool,
}

/// Chrome's type-ahead (`DateTimeNumericFieldElement::HandleKeyboardEvent`),
/// the one typing rule of every date and time segment: `digit` joins the
/// digits typed before it (`buffer`, none on entering the segment), of
/// which a segment of `digits` digits keeps the last `digits − 1` once
/// full; the number is held to `max`, and one below `min` leaves the
/// segment empty, waiting. The segment is done once it has `digits` digits
/// or a further one could only pass `max`.
#[must_use]
pub fn type_digit(buffer: Option<(u8, i32)>, digit: u8, min: i32, max: i32, digits: u8) -> Typed {
    let keep = 10_i32.saturating_pow(u32::from(digits.saturating_sub(1)));
    let (before, count) = match buffer {
        Some((count, spelled)) if count >= digits => (spelled.checked_rem(keep).unwrap_or(0), digits),
        Some((count, spelled)) => (spelled, count.saturating_add(1)),
        None => (0, 1),
    };
    let spelled = before.saturating_mul(10).saturating_add(i32::from(digit));
    let value = (spelled >= min).then(|| spelled.min(max));
    Typed {
        value,
        buffer: (count, spelled),
        done: count >= digits || spelled.saturating_mul(10) > max,
    }
}

/// One step of `step` up (or down) from `value` within `min..=max`,
/// wrapping; an empty segment takes `empty_up` (or `empty_down`)
/// (Chrome's `stepUp`, `stepDown`).
#[must_use]
pub fn step_value(value: Option<i32>, up: bool, step: i32, min: i32, max: i32, empty: (i32, i32)) -> i32 {
    let Some(v) = value else {
        return if up { empty.0 } else { empty.1 };
    };
    let span = max.saturating_sub(min).saturating_add(1).max(1);
    let moved = if up {
        v.saturating_add(step)
    } else {
        v.saturating_sub(step)
    };
    moved.saturating_sub(min).rem_euclid(span).saturating_add(min)
}

/// The step of each unit for the arrow keys (reka's `step`, 1 each by default).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Steps {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Default for Steps {
    fn default() -> Self {
        Self {
            year: 1,
            month: 1,
            day: 1,
            hour: 1,
            minute: 1,
            second: 1,
        }
    }
}

/// What a date field is told.
#[derive(Clone, Debug, PartialEq)]
pub struct DateFieldOptions {
    pub availability: Availability,
    pub readonly: bool,
    /// Days before `min` or after `max` make the field invalid (not held).
    pub min: Option<CalendarDate>,
    pub max: Option<CalendarDate>,
    /// `isDateUnavailable`: days that make the field invalid.
    pub is_unavailable: Option<Derive<CalendarDate, bool>>,
    pub locale: CalendarLocale,
    /// The clock; none for the locale's.
    pub hour_cycle: Option<HourCycle>,
    pub steps: Steps,
    /// `stepSnapping`: an hour, minute or second whose unit steps by more
    /// than 1 is rounded to its nearest multiple when focus leaves it, held
    /// below the unit's largest (a minute stepping by 15: 58 gives 45).
    /// The arrows always step by the step.
    pub round_to_step: bool,
    /// What assistive technology is told; none for the locale's.
    pub strings: Option<DateStrings>,
    /// The day an empty year steps from (reka's `placeholder`), today by default.
    pub anchor: CalendarDate,
}

impl Default for DateFieldOptions {
    fn default() -> Self {
        Self {
            availability: Availability::Enabled,
            readonly: false,
            min: None,
            max: None,
            is_unavailable: None,
            locale: CalendarLocale::EnUs,
            hour_cycle: None,
            steps: Steps::default(),
            round_to_step: false,
            strings: None,
            anchor: CalendarDate::today(),
        }
    }
}

impl DateFieldOptions {
    /// Whether `day` is one the field takes: in `min..=max`, not unavailable.
    fn bad(&self, day: CalendarDate) -> bool {
        self.min.is_some_and(|m| day < m)
            || self.max.is_some_and(|m| day > m)
            || self.is_unavailable.as_ref().is_some_and(|f| f.call(day))
    }

    /// Whether `range` is invalid (`DateRangeFieldRoot.vue:166-219`): an end
    /// unavailable or outside `min..=max`, the start after the end, or an
    /// unavailable day between them (reka checks between always, whatever
    /// the calendar beside it allows).
    #[must_use]
    pub fn range_invalid(&self, range: DateRangeEntry) -> bool {
        let (start, end) = (range.start.date(), range.end.date());
        if start.is_some_and(|d| self.bad(d)) || end.is_some_and(|d| self.bad(d)) {
            return true;
        }
        match (start, end) {
            (Some(s), Some(e)) if e < s => true,
            (Some(s), Some(e)) => self
                .is_unavailable
                .as_ref()
                .is_some_and(|f| (1..s.days_until(e)).any(|n| f.call(s.add_days(n)))),
            _ => false,
        }
    }

    fn cycle(&self) -> HourCycle {
        self.hour_cycle.unwrap_or(if self.locale.twelve_hour() {
            HourCycle::H12
        } else {
            HourCycle::H23
        })
    }
}

/// A range as two fields hold it (reka's `DateRangeField`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DateRangeEntry {
    pub start: DateEntry,
    pub end: DateEntry,
}

impl DateRangeEntry {
    /// The days, as far as each field has a whole one.
    #[must_use]
    pub fn range(self) -> crate::range_calendar::DateRange {
        crate::range_calendar::DateRange {
            start: self.start.date(),
            end: self.end.date(),
        }
    }

    /// What a form sends: "`<start>` - `<end>`", empty while either is
    /// (`DateRangeFieldRoot.vue:276-281`), so `required` reports it.
    #[must_use]
    pub fn value(self) -> String {
        let (a, b) = (self.start.iso(), self.end.iso());
        if a.is_empty() || b.is_empty() {
            return String::new();
        }
        format!("{a} - {b}")
    }
}

impl Submit for DateRangeEntry {
    fn form_data(&self) -> FormData {
        FormData::Text(self.value())
    }
}

/// Where a field's entry is kept: a value of its own, one end of a range's
/// (the two fields of a range read and write one value, so they never
/// disagree), or a time's ([`crate::time_field`]). Each reads and writes
/// through its part's [`UseModel`] and nowhere else.
enum Source<P: 'static> {
    Whole(UseModel<P, DateEntry>),
    Start(UseModel<P, DateRangeEntry>),
    End(UseModel<P, DateRangeEntry>),
    Time(UseModel<P, Time>),
}

impl<P: 'static> Clone for Source<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Source<P> {}

impl<P: 'static> Source<P> {
    /// The entry (tracked).
    fn get(self) -> DateEntry {
        match self {
            Self::Whole(m) => m.get(),
            Self::Start(m) => m.with(|r| r.start),
            Self::End(m) => m.with(|r| r.end),
            Self::Time(m) => m.with(|t| t.entry()),
        }
    }

    fn with<R>(self, f: impl FnOnce(&DateEntry) -> R) -> R {
        f(&self.get())
    }

    fn get_untracked(self) -> DateEntry {
        gui::untracked(|| self.get())
    }

    fn set(self, v: DateEntry) {
        match self {
            Self::Whole(m) => m.set(v),
            Self::Start(m) => m.update(|r| r.start = v),
            Self::End(m) => m.update(|r| r.end = v),
            Self::Time(m) => m.set(Time::of_entry(v)),
        }
    }
}

/// A piece of a field as it is laid out: a segment, or the text between two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    Segment(Segment),
    Literal(&'static str),
}

/// What a key or a digit did in a segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Moved {
    Stay,
    Next,
}

/// A date field's state: its value, read and written through the part's
/// [`UseModel`] and nowhere else, the digits being typed, the day period
/// waiting for an hour, and each segment's focus. Made once in setup, `Copy`.
pub struct DateField<P: 'static> {
    value: Source<P>,
    options: Computed<DateFieldOptions>,
    /// `aria-invalid`: what the field's kind makes invalid of its value.
    invalid: Computed<bool>,
    /// A time field: the time segments alone.
    time_only: bool,
    /// The digits typed into a segment since it took focus, while it still
    /// shows what they spelled.
    typed: Signal<Option<(Segment, u8, i32)>>,
    /// PM (true) or AM chosen while the hour is empty.
    period: Signal<Option<bool>>,
    focus: [FocusHandle; 7],
    /// What ArrowRight from the last segment reaches (a picker's trigger,
    /// a range's end field), and ArrowLeft from the first (a range's start).
    after: Signal<Option<FocusHandle>>,
    before: Signal<Option<FocusHandle>>,
    direction: ReadDirection,
}

impl<P: 'static> Clone for DateField<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for DateField<P> {}

impl<P: 'static> PartialEq for DateField<P> {
    fn eq(&self, other: &Self) -> bool {
        self.typed == other.typed
    }
}

impl<P: 'static> fmt::Debug for DateField<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DateField").finish_non_exhaustive()
    }
}

impl<P: 'static> DateField<P> {
    /// Segments showing `value` (the part's `v-model`) with `options`, its
    /// arrows by `direction`, called in the setup that made it. What was
    /// typed is forgotten whenever focus moves.
    #[must_use]
    #[track_caller]
    pub fn new(
        value: UseModel<P, DateEntry>,
        options: impl Fn() -> DateFieldOptions + 'static,
        direction: ReadDirection,
    ) -> Self {
        let options = Computed::new(options);
        let invalid = Computed::new(move || value.with(|v| v.date()).is_some_and(|d| options.with(|o| o.bad(d))));
        Self::build(Source::Whole(value), options, invalid, false, direction)
    }

    /// The start and end fields of a range kept in `value` (reka's
    /// `DateRangeField`), with one set of `options`, called in setup. Both
    /// are invalid when the range is ([`DateFieldOptions::range_invalid`]);
    /// ArrowRight from the start's last segment reaches the end's first, and
    /// ArrowLeft back, in whatever order the locale has them at the time.
    #[must_use]
    #[track_caller]
    pub fn range(
        value: UseModel<P, DateRangeEntry>,
        options: impl Fn() -> DateFieldOptions + 'static,
        direction: ReadDirection,
    ) -> (Self, Self) {
        let options = Computed::new(options);
        let invalid = Computed::new(move || {
            let r = value.get();
            options.with(|o| o.range_invalid(r))
        });
        let start = Self::build(Source::Start(value), options, invalid, false, direction);
        let end = Self::build(Source::End(value), options, invalid, false, direction);
        let link = move || (end.edge(false), start.edge(true));
        let (first, last) = gui::untracked(link);
        start.set_after(first);
        end.set_before(last);
        watch(link, move |(first, last), _| {
            start.set_after(*first);
            end.set_before(*last);
        });
        (start, end)
    }

    /// The time segments of `value` with `options` and `invalid` (a time
    /// field's own bounds): [`crate::time_field::TimeField`]'s engine.
    pub(crate) fn time(
        value: UseModel<P, Time>,
        options: Computed<DateFieldOptions>,
        invalid: Computed<bool>,
        direction: ReadDirection,
    ) -> Self {
        Self::build(Source::Time(value), options, invalid, true, direction)
    }

    /// The time segments of a dated `value` (a date-time whose day stays as
    /// it is): [`crate::time_field::TimeField::over`]'s engine.
    pub(crate) fn time_of(
        value: UseModel<P, DateEntry>,
        options: Computed<DateFieldOptions>,
        invalid: Computed<bool>,
        direction: ReadDirection,
    ) -> Self {
        Self::build(Source::Whole(value), options, invalid, true, direction)
    }

    fn build(
        value: Source<P>,
        options: Computed<DateFieldOptions>,
        invalid: Computed<bool>,
        time_only: bool,
        direction: ReadDirection,
    ) -> Self {
        let field = Self {
            value,
            options,
            invalid,
            time_only,
            typed: Signal::labeled("date typed", None),
            period: Signal::labeled("date period", None),
            focus: std::array::from_fn(|_| FocusHandle::new()),
            after: Signal::labeled("date field after", None),
            before: Signal::labeled("date field before", None),
            direction,
        };
        watch(gui::focus::focused, move |_, left| {
            field.typed.set(None);
            let left = Segment::ALL
                .into_iter()
                .find(|s| left.is_some() && field.focus.get(s.index()).map(|f| f.id()) == *left);
            let v = field.value.get_untracked();
            let next = left.map_or(v, |s| field.rounded(s, v)).held();
            if next != v {
                field.write(next);
            }
        });
        field
    }

    /// `v` with segment `s` rounded to its unit's step, when the options
    /// round and the step is above 1 (`snapValueToStep`).
    pub(crate) fn rounded(self, s: Segment, v: DateEntry) -> DateEntry {
        let (round, steps) = gui::untracked(|| self.options.with(|o| (o.round_to_step, o.steps)));
        let (step, max) = match s {
            Segment::Hour => (steps.hour, 23),
            Segment::Minute => (steps.minute, 59),
            Segment::Second => (steps.second, 59),
            _ => return v,
        };
        let Some(value) = v.get(s) else { return v };
        if !round || step <= 1 {
            return v;
        }
        v.set(s, Some(snap(value, i32::from(step), max)))
    }

    /// Whether it can be used (tracked): what a picker's trigger and a
    /// [`crate::form::FormField`] of it read, so they agree with it.
    #[must_use]
    pub fn availability(self) -> Availability {
        self.options.with(|o| o.availability)
    }

    /// Whether it is read-only (tracked).
    #[must_use]
    pub fn readonly(self) -> bool {
        self.options.with(|o| o.readonly)
    }

    /// The options (tracked).
    #[must_use]
    pub fn options(self) -> DateFieldOptions {
        self.options.get()
    }

    /// The value (tracked).
    #[must_use]
    pub fn value(self) -> DateEntry {
        self.value.get()
    }

    /// Hand ArrowRight on the last segment to `handle` (a date picker's
    /// trigger, which ends the field's arrow chain).
    pub fn set_after(self, handle: Option<FocusHandle>) {
        self.after.set(handle);
    }

    /// Hand ArrowLeft on the first segment to `handle` (a range's start
    /// field, before its end field).
    pub fn set_before(self, handle: Option<FocusHandle>) {
        self.before.set(handle);
    }

    /// The focus handle of the first (or last) segment (tracked).
    #[must_use]
    pub fn edge(self, last: bool) -> Option<FocusHandle> {
        let order = self.segments();
        let s = if last { order.last() } else { order.first() };
        s.and_then(|s| self.focus.get(s.index())).copied()
    }

    /// The pieces in the locale's order (tracked).
    #[must_use]
    pub fn pieces(self) -> Vec<Piece> {
        let (precision, (locale, cycle)) = (
            self.value.with(|v| v.precision),
            self.options.with(|o| (o.locale, o.cycle())),
        );
        let mut out = Vec::with_capacity(14);
        if !self.time_only {
            for (i, part) in locale.date_order().into_iter().enumerate() {
                if i > 0 {
                    out.push(Piece::Literal(locale.date_separator()));
                }
                out.push(Piece::Segment(match part {
                    DatePart::Year => Segment::Year,
                    DatePart::Month => Segment::Month,
                    DatePart::Day => Segment::Day,
                }));
            }
        }
        let period_first = cycle == HourCycle::H12 && locale.day_period_first();
        if self.time_only || precision >= Precision::Hour {
            if !self.time_only {
                out.push(Piece::Literal(", "));
            }
            if period_first {
                out.push(Piece::Segment(Segment::DayPeriod));
                out.push(Piece::Literal(" "));
            }
            out.push(Piece::Segment(Segment::Hour));
            for (p, s) in [
                (Precision::Minute, Segment::Minute),
                (Precision::Second, Segment::Second),
            ] {
                if precision >= p {
                    out.push(Piece::Literal(":"));
                    out.push(Piece::Segment(s));
                }
            }
            if cycle == HourCycle::H12 && !period_first {
                out.push(Piece::Literal(" "));
                out.push(Piece::Segment(Segment::DayPeriod));
            }
        }
        out
    }

    /// The segments in order (tracked).
    #[must_use]
    pub fn segments(self) -> Vec<Segment> {
        self.pieces()
            .into_iter()
            .filter_map(|p| match p {
                Piece::Segment(s) => Some(s),
                Piece::Literal(_) => None,
            })
            .collect()
    }

    fn order(self) -> Vec<Segment> {
        gui::untracked(|| self.segments())
    }

    /// Whether one of its segments holds focus (tracked).
    #[must_use]
    pub fn focused(self) -> bool {
        self.focus.iter().any(|f| f.is_focused())
    }

    /// Focus the first segment (a label pressed, a form's error).
    pub fn focus_first(self) -> bool {
        self.focus_segment(self.order().first().copied())
    }

    /// Focus `s`; false when there is none.
    pub fn focus_segment(self, s: Option<Segment>) -> bool {
        s.and_then(|s| self.focus.get(s.index())).map(|f| f.focus()).is_some()
    }

    /// The segment after (or before) `s` in the locale's order.
    fn beside(self, s: Segment, after: bool) -> Option<Segment> {
        let order = self.order();
        let i = order.iter().position(|x| *x == s)?;
        if after {
            order.get(i.checked_add(1)?).copied()
        } else {
            order.get(i.checked_sub(1)?).copied()
        }
    }

    /// Whether the day period reads PM: the hour's, else the one chosen.
    fn pm(self) -> Option<bool> {
        let v = self.value.get_untracked();
        v.hour.map(|h| h >= 12).or_else(|| self.period.get_untracked())
    }

    /// Segment `s`'s range as it is typed and read (the hour as shown).
    fn span(self, s: Segment, v: DateEntry) -> (i32, i32) {
        let (cycle, anchor) = self.options.with(|o| (o.cycle(), o.anchor));
        let system = v.system;
        match s {
            Segment::Year => (1, 9999),
            Segment::Month => (
                1,
                i32::from(system.months_in_year(v.year.unwrap_or_else(|| system.fields(anchor).year))),
            ),
            Segment::Day => {
                let days = match (v.year, v.month) {
                    (Some(y), Some(m)) => system.days_in_month(y, m),
                    // No year yet: the longest that month runs in any year.
                    (None, Some(m)) => {
                        let y = system.fields(anchor).year;
                        (0..8)
                            .map(|n| system.days_in_month(y.saturating_add(n), m))
                            .max()
                            .unwrap_or(31)
                    }
                    _ => 31,
                };
                (1, i32::from(days))
            }
            Segment::Hour if cycle == HourCycle::H12 => (1, 12),
            Segment::Hour => (0, 23),
            Segment::Minute | Segment::Second => (0, 59),
            Segment::DayPeriod => (0, 1),
        }
    }

    /// Segment `s`'s value as it is shown and read: the hour on its clock.
    fn shown_value(self, s: Segment, v: DateEntry) -> Option<i32> {
        let cycle = self.options.with(|o| o.cycle());
        match s {
            Segment::Hour if cycle == HourCycle::H12 => v.hour.map(|h| {
                let h = i32::from(h).rem_euclid(12);
                if h == 0 { 12 } else { h }
            }),
            Segment::DayPeriod => v.get(Segment::DayPeriod).or_else(|| self.period.get().map(i32::from)),
            _ => v.get(s),
        }
    }

    /// What segment `s` shows (tracked): its value in the locale's digits,
    /// two of them for a month, a day and a time unit, four for a year; the
    /// day period's word; else the placeholder (`mm`, `dd`, `yyyy`; `--`).
    #[must_use]
    pub fn text(self, s: Segment) -> String {
        let v = self.value.get();
        let locale = self.options.with(|o| o.locale);
        match (s, self.shown_value(s, v)) {
            (Segment::DayPeriod, Some(pm)) => locale
                .day_periods()
                .get(usize::from(pm == 1))
                .copied()
                .unwrap_or("AM")
                .to_owned(),
            (Segment::Year, Some(y)) => localized(locale, &format!("{y:04}")),
            (_, Some(n)) => localized(locale, &format!("{n:02}")),
            (Segment::Year, None) => locale.placeholder(DatePart::Year).to_owned(),
            (Segment::Month, None) => locale.placeholder(DatePart::Month).to_owned(),
            (Segment::Day, None) => locale.placeholder(DatePart::Day).to_owned(),
            (_, None) => "--".to_owned(),
        }
    }

    /// Whether segment `s` is empty (tracked): its look shows the
    /// placeholder colour.
    #[must_use]
    pub fn is_empty(self, s: Segment) -> bool {
        let v = self.value.get();
        self.shown_value(s, v).is_none()
    }

    /// Whether the value is one the field does not take (`aria-invalid`,
    /// tracked): a day unavailable, before `min` or after `max`; a range
    /// as [`DateFieldOptions::range_invalid`] says; a time outside its
    /// bounds. Never held: the value stays as typed.
    #[must_use]
    pub fn invalid(self) -> bool {
        self.invalid.get()
    }

    fn write(self, v: DateEntry) {
        self.value.set(v);
    }

    /// A digit in `s` (Chrome's type-ahead); whether focus moved on.
    fn digit(self, s: Segment, digit: u8) -> Moved {
        let v = self.value.get_untracked();
        let (min, max) = gui::untracked(|| self.span(s, v));
        let digits = if s == Segment::Year { 4 } else { 2 };
        let buffer = match self.typed.get_untracked() {
            Some((at, count, spelled))
                if at == s
                    && gui::untracked(|| self.shown_value(s, v)) == (spelled >= min).then(|| spelled.min(max)) =>
            {
                Some((count, spelled))
            }
            _ => None,
        };
        let typed = type_digit(buffer, digit, min, max, digits);
        let next = if s == Segment::Hour && gui::untracked(|| self.options.with(|o| o.cycle())) == HourCycle::H12 {
            let pm = self.pm().unwrap_or(false);
            v.set(
                s,
                typed
                    .value
                    .map(|h| h.rem_euclid(12).saturating_add(if pm { 12 } else { 0 })),
            )
        } else {
            v.set(s, typed.value)
        };
        self.write(if typed.done { next.held() } else { next });
        let moved = typed.done && self.focus_segment(self.beside(s, true));
        if !moved {
            self.typed.set(Some((s, typed.buffer.0, typed.buffer.1)));
            return Moved::Stay;
        }
        Moved::Next
    }

    /// ArrowUp or ArrowDown in `s`.
    fn step(self, s: Segment, up: bool) {
        let v = self.value.get_untracked();
        self.typed.set(None);
        if s == Segment::DayPeriod {
            let pm = !self.pm().unwrap_or(!up);
            self.set_period(pm);
            return;
        }
        let (min, max) = gui::untracked(|| self.span(s, v));
        let (steps, anchor, cycle) = gui::untracked(|| self.options.with(|o| (o.steps, o.anchor, o.cycle())));
        let step = match s {
            Segment::Year => i32::from(steps.year),
            Segment::Month => i32::from(steps.month),
            Segment::Day => i32::from(steps.day),
            Segment::Hour => i32::from(steps.hour),
            Segment::Minute => i32::from(steps.minute),
            Segment::Second => i32::from(steps.second),
            Segment::DayPeriod => 1,
        };
        let empty = if s == Segment::Year {
            let y = v.system.fields(anchor).year;
            (y, y)
        } else {
            (min, max)
        };
        let shown = gui::untracked(|| self.shown_value(s, v));
        let next = step_value(shown, up, step.max(1), min, max, empty);
        let next = if s == Segment::Hour && cycle == HourCycle::H12 {
            let pm = self.pm().unwrap_or(false);
            next.rem_euclid(12).saturating_add(if pm { 12 } else { 0 })
        } else {
            next
        };
        self.write(v.set(s, Some(next)).held());
    }

    /// AM (false) or PM: the hour's half of the day, or kept for the hour
    /// to come while it is empty.
    fn set_period(self, pm: bool) {
        let v = self.value.get_untracked();
        if v.hour.is_some() {
            self.write(v.set(Segment::DayPeriod, Some(i32::from(pm))));
        }
        self.period.set(Some(pm));
    }

    /// What a key does in segment `s`; whether it took it. A disabled
    /// field takes none; a read-only one moves between segments only.
    pub fn key(self, s: Segment, ev: &KeyEvent) -> bool {
        let (availability, readonly) = gui::untracked(|| self.options.with(|o| (o.availability, o.readonly)));
        if availability == Availability::Disabled {
            return false;
        }
        let rtl = self.direction.get() == Direction::Rtl;
        let (left, right) = (ev.is(NamedKey::ArrowLeft), ev.is(NamedKey::ArrowRight));
        if left || right {
            let forward = right != rtl;
            let to = self.beside(s, forward);
            if to.is_none() {
                let edge = if forward { self.after } else { self.before };
                return gui::untracked(|| edge.get()).map(FocusHandle::focus).is_some();
            }
            return self.focus_segment(to);
        }
        if readonly || availability != Availability::Enabled {
            return false;
        }
        let m = ev.modifiers;
        if m.control || m.alt || m.command {
            return false;
        }
        if ev.is(NamedKey::ArrowUp) || ev.is(NamedKey::ArrowDown) {
            self.step(s, ev.is(NamedKey::ArrowUp));
            return true;
        }
        if ev.is(NamedKey::Backspace) || ev.is(NamedKey::Delete) {
            self.typed.set(None);
            if s == Segment::DayPeriod {
                self.period.set(None);
            } else {
                let v = self.value.get_untracked();
                self.write(v.set(s, None));
            }
            return true;
        }
        let Some(text) = ev.text.as_deref() else {
            return false;
        };
        if s == Segment::DayPeriod {
            let locale = gui::untracked(|| self.options.with(|o| o.locale));
            let [am, pm] = locale.day_periods();
            let first = |w: &str| w.chars().next().map(|c| c.to_lowercase().collect::<String>());
            let typed = text.to_lowercase();
            if Some(typed.clone()) == first(am) || typed == "a" {
                self.set_period(false);
            } else if Some(typed.clone()) == first(pm) || typed == "p" {
                self.set_period(true);
            }
            return !text.trim().is_empty();
        }
        // Each digit in turn, an input method's whole text included: one
        // that moves focus on hands the rest to the next segment.
        let mut at = s;
        let mut took = false;
        for c in text.chars() {
            let Some(d) = digit_value(c) else { continue };
            took = true;
            if self.digit(at, d) == Moved::Next {
                match self.beside(at, true) {
                    Some(next) => at = next,
                    None => break,
                }
            }
        }
        took || !text.trim().is_empty()
    }

    /// `d` as the box around the segments: a group, read as disabled when
    /// the field is (`DateFieldRoot.vue:362-374`); a press beside the
    /// segments focuses the first unless one has focus, an unavailable
    /// field's too, as Chrome's `aria-disabled` date input does (it is
    /// focus, not an action: `gui::Div::press_focuses`). A disabled field's
    /// segments take no focus, so it stays where it is.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        let availability = self.options.with(|o| o.availability);
        d.role(Role::Group)
            .unavailable(availability != Availability::Enabled)
            .aria_invalid(self.invalid())
            .press_focuses(move || {
                if self.focused() {
                    return None;
                }
                self.order().first().and_then(|s| self.focus.get(s.index())).copied()
            })
    }

    /// `d` as a literal between segments: hidden from assistive technology.
    #[must_use]
    pub fn literal(self, d: Div) -> Div {
        d.aria_hidden(true)
    }

    /// The strings it names its parts with: the options' own, else its
    /// locale's (tracked).
    #[must_use]
    pub fn strings(self) -> DateStrings {
        self.options
            .with(|o| o.strings.clone().unwrap_or_else(|| o.locale.strings()))
    }

    /// `d` as segment `s`: a Tab stop taking the segment's keys, and a spin
    /// button named in the locale with the segment's range, its value as
    /// shown (the hour on its clock), its text as shown or blank while
    /// empty. Called in the render (tracked).
    #[must_use]
    pub fn segment(self, s: Segment, d: Div) -> Div {
        let handle = self.focus.get(s.index()).copied().unwrap_or_default();
        let v = self.value.get();
        let (strings, availability, readonly) = self.options.with(|o| {
            (
                o.strings.clone().unwrap_or_else(|| o.locale.strings()),
                o.availability,
                o.readonly,
            )
        });
        let name = match s {
            Segment::Year => strings.year,
            Segment::Month => strings.month,
            Segment::Day => strings.day,
            Segment::Hour => strings.hour,
            Segment::Minute => strings.minute,
            Segment::Second => strings.second,
            Segment::DayPeriod => strings.day_period,
        };
        let (min, max) = self.span(s, v);
        let now = self.shown_value(s, v);
        let text = if now.is_some() { self.text(s) } else { strings.blank };
        availability
            .apply(d.track_focus(handle))
            .role(Role::SpinButton)
            .aria_label(name)
            .aria_value_range(ValueRange {
                now: now.map(f64::from),
                min: f64::from(min),
                max: f64::from(max),
                text: Some(text),
            })
            .aria_invalid(self.invalid())
            .aria_readonly(readonly)
            .on_key_down(move |ev| self.key(s, ev))
    }

    /// Segment `s`'s range as it is typed and read (the hour on its clock;
    /// tracked): what a look sizes a segment by.
    #[must_use]
    pub fn bounds(self, s: Segment) -> (i32, i32) {
        let v = self.value.get();
        self.span(s, v)
    }
}

/// `value` rounded to the nearest multiple of `step` in `0..=max`, below
/// `max` when that passes it (`snapValueToStep` with no minimum).
fn snap(value: i32, step: i32, max: i32) -> i32 {
    let rem = value.rem_euclid(step);
    let up = rem.saturating_mul(2) >= step;
    let snapped = if up {
        value.saturating_add(step.saturating_sub(rem))
    } else {
        value.saturating_sub(rem)
    };
    if snapped > max {
        max.checked_div(step).unwrap_or(0).saturating_mul(step)
    } else {
        snapped
    }
}

/// Latin digits in `latin` written in `locale`'s.
fn localized(locale: CalendarLocale, latin: &str) -> String {
    latin
        .chars()
        .map(|c| {
            c.to_digit(10)
                .map_or_else(|| c.to_string(), |d| locale.number(i64::from(d)))
        })
        .collect()
}
