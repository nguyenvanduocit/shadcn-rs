//! A time typed segment by segment (reka's `TimeField/`): the time segments
//! of the segment core in [`crate::date_field`], which types every date and
//! time field by one rule.
//!
//! **The value** is a [`Time`]: the hour, minute and second as the
//! segments hold them, each empty until given one, to the time's
//! [`Precision`] (reka's `granularity`: an hour, a minute — the default — or
//! a second), so a time partly typed is a value of its own and a caller's
//! write (a form reset's empty time) always puts the segments back. What a
//! form sends is [`Time::value`]: `HH:MM`, or `HH:MM:SS` to the second, empty
//! until every segment has one (Chrome's time input; reka's `Time` always
//! writes the seconds). [`TimeField::over`] edits the time of a date and
//! time instead, its day kept as it is (reka takes a date-time and keeps
//! its type).
//!
//! **Segments** run in the locale's order (`formatToParts`): the hour, the
//! minute, the second and, on a 12-hour clock (the locale's, or
//! [`TimeFieldOptions::hour_cycle`]), the day period, before the hour
//! where the locale writes it first ([`CalendarLocale::day_period_first`]).
//! Each is a Tab stop and a spin button named in the locale
//! ([`DateStrings`]), carrying the field's `aria-invalid` and
//! `aria-readonly`; the literals between them are hidden from it.
//!
//! **Keys** are the segment core's ([`crate::date_field`]): digits by
//! Chrome's typing rule ("2" then "5" in a 24-hour hour gives 23; reka's
//! gives 5), ArrowUp and ArrowDown by the unit's step
//! ([`TimeFieldOptions::steps`]) wrapping, Backspace and Delete emptying the
//! segment, ArrowLeft and ArrowRight to the segment before or after,
//! swapped right to left, A and P (and the locale's first letters) on the
//! day period. With [`TimeFieldOptions::round_to_step`] a unit stepping by
//! more than 1 is rounded to its step as focus leaves it (a minute by 15:
//! 58 gives 45).
//!
//! **Bounds.** A time before [`TimeFieldOptions::min`] or after
//! [`TimeFieldOptions::max`] makes the field invalid ([`TimeField::invalid`])
//! and is kept as typed. reka compares the bounds on today's date, so a
//! date and time of another day is compared wrongly
//! (`TimeField/TimeFieldRoot.vue:86-92,195-206`): here a bound is a time of
//! day, compared with the value's time of day whatever its day.

use std::fmt;

use gui::events::KeyEvent;
use gui::focus::FocusHandle;
use gui::{Computed, Div, UseModel};

use crate::availability::Availability;
use crate::date::{CalendarLocale, CalendarSystem, DateStrings};
use crate::date_field::{DateEntry, DateField, DateFieldOptions, HourCycle, Piece, Precision, Segment, Steps};
use crate::direction::ReadDirection;
use crate::form::{FormData, Submit};

/// A time as a field's segments hold it: the hour (0–23, whatever the
/// clock shows), the minute and the second, each empty until given one, to
/// `precision` (an hour at least).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Time {
    pub precision: Precision,
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    pub second: Option<u8>,
}

/// An empty time to the minute (reka's default granularity).
impl Default for Time {
    fn default() -> Self {
        Self::empty(Precision::Minute)
    }
}

impl Time {
    /// No segment set, to `precision` (a [`Precision::Day`] reads as an hour).
    #[must_use]
    pub fn empty(precision: Precision) -> Self {
        Self {
            precision: if precision == Precision::Day {
                Precision::Hour
            } else {
                precision
            },
            hour: None,
            minute: None,
            second: None,
        }
    }

    /// `hour`, `minute` and `second` to `precision`; a value out of its
    /// range, or below the precision, is left empty.
    #[must_use]
    pub fn at(precision: Precision, hour: Option<u8>, minute: Option<u8>, second: Option<u8>) -> Self {
        let t = Self::empty(precision);
        Self {
            hour: hour.filter(|h| *h < 24),
            minute: minute.filter(|m| *m < 60 && t.precision >= Precision::Minute),
            second: second.filter(|s| *s < 60 && t.precision >= Precision::Second),
            ..t
        }
    }

    /// `value` (`HH`, `HH:MM` or `HH:MM:SS`, its parts deciding the
    /// precision); a part out of range or not a number (`--`) is empty.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        let parts: Vec<Option<u8>> = value.split(':').take(3).map(|p| p.trim().parse::<u8>().ok()).collect();
        let precision = match parts.len() {
            0 | 1 => Precision::Hour,
            2 => Precision::Minute,
            _ => Precision::Second,
        };
        let part = |i: usize| parts.get(i).copied().flatten();
        Self::at(precision, part(0), part(1), part(2))
    }

    /// Whether every segment of its precision is set.
    #[must_use]
    pub fn is_complete(self) -> bool {
        self.hour.is_some()
            && (self.precision < Precision::Minute || self.minute.is_some())
            && (self.precision < Precision::Second || self.second.is_some())
    }

    /// Whether no segment is set.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.hour.is_none() && self.minute.is_none() && self.second.is_none()
    }

    /// What a form sends, as Chrome's `<input type="time">`: `HH:MM`, or
    /// `HH:MM:SS` to the second; empty while a segment is.
    #[must_use]
    pub fn value(self) -> String {
        if !self.is_complete() {
            return String::new();
        }
        let two = |v: Option<u8>| format!("{:02}", v.unwrap_or(0));
        if self.precision == Precision::Second {
            format!("{}:{}:{}", two(self.hour), two(self.minute), two(self.second))
        } else {
            format!("{}:{}", two(self.hour), two(self.minute))
        }
    }

    /// The seconds since midnight, once complete (a unit below the
    /// precision counts 0).
    #[must_use]
    pub fn seconds(self) -> Option<u32> {
        if !self.is_complete() {
            return None;
        }
        let n = |v: Option<u8>| u32::from(v.unwrap_or(0));
        Some(
            n(self.hour)
                .saturating_mul(3600)
                .saturating_add(n(self.minute).saturating_mul(60))
                .saturating_add(n(self.second)),
        )
    }

    /// The time as the segment core's entry: no date, its time segments.
    #[must_use]
    pub fn entry(self) -> DateEntry {
        DateEntry::empty(CalendarSystem::Gregorian, self.precision).with_time(self.hour, self.minute, self.second)
    }

    /// The time segments of `entry`.
    #[must_use]
    pub fn of_entry(entry: DateEntry) -> Self {
        Self::at(entry.precision, entry.hour, entry.minute, entry.second)
    }
}

impl From<&str> for Time {
    fn from(value: &str) -> Self {
        Self::parse(value)
    }
}

/// As the input's `value` reads: empty while a segment is.
impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value())
    }
}

/// Sent as a native time input is: its `value`, empty while partly typed.
impl Submit for Time {
    fn form_data(&self) -> FormData {
        FormData::Text(self.value())
    }
}

/// What a time field is told (reka's `TimeFieldRoot` props).
#[derive(Clone, Debug, PartialEq)]
pub struct TimeFieldOptions {
    pub availability: Availability,
    pub readonly: bool,
    /// Times before `min` or after `max` make the field invalid (not held);
    /// a bound is a time of day.
    pub min: Option<Time>,
    pub max: Option<Time>,
    pub locale: CalendarLocale,
    /// The clock; none for the locale's.
    pub hour_cycle: Option<HourCycle>,
    /// The step of each unit for the arrow keys (the date units unused).
    pub steps: Steps,
    /// `stepSnapping` (see [`DateFieldOptions::round_to_step`]).
    pub round_to_step: bool,
    /// What assistive technology is told; none for the locale's.
    pub strings: Option<DateStrings>,
}

impl Default for TimeFieldOptions {
    fn default() -> Self {
        Self {
            availability: Availability::Enabled,
            readonly: false,
            min: None,
            max: None,
            locale: CalendarLocale::EnUs,
            hour_cycle: None,
            steps: Steps::default(),
            round_to_step: false,
            strings: None,
        }
    }
}

impl TimeFieldOptions {
    /// The segment core's options.
    fn engine(&self) -> DateFieldOptions {
        DateFieldOptions {
            availability: self.availability,
            readonly: self.readonly,
            locale: self.locale,
            hour_cycle: self.hour_cycle,
            steps: self.steps,
            round_to_step: self.round_to_step,
            strings: self.strings.clone(),
            ..DateFieldOptions::default()
        }
    }

    /// Whether `time` is outside the bounds.
    #[must_use]
    pub fn out_of_bounds(&self, time: Time) -> bool {
        let Some(at) = time.seconds() else {
            return false;
        };
        self.min.and_then(Time::seconds).is_some_and(|m| at < m)
            || self.max.and_then(Time::seconds).is_some_and(|m| at > m)
    }
}

/// A time field's state: the segment core over its time. Made once in
/// setup, `Copy`.
pub struct TimeField<P: 'static> {
    field: DateField<P>,
}

impl<P: 'static> Clone for TimeField<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for TimeField<P> {}

impl<P: 'static> PartialEq for TimeField<P> {
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field
    }
}

impl<P: 'static> fmt::Debug for TimeField<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TimeField").finish_non_exhaustive()
    }
}

impl<P: 'static> TimeField<P> {
    /// Segments showing `value` (the part's `v-model`) with `options`, its
    /// arrows by `direction`, called in the setup that made it.
    #[must_use]
    #[track_caller]
    pub fn new(
        value: UseModel<P, Time>,
        options: impl Fn() -> TimeFieldOptions + 'static,
        direction: ReadDirection,
    ) -> Self {
        let options = Computed::new(options);
        let engine = Computed::new(move || options.with(TimeFieldOptions::engine));
        let invalid = Computed::new(move || {
            let t = value.get();
            options.with(|o| o.out_of_bounds(t))
        });
        Self {
            field: DateField::time(value, engine, invalid, direction),
        }
    }

    /// The time segments of the date and time `value`, its day kept as it
    /// is, compared with the bounds by its time of day (reka's TimeField on
    /// a `CalendarDateTime`).
    #[must_use]
    #[track_caller]
    pub fn over(
        value: UseModel<P, DateEntry>,
        options: impl Fn() -> TimeFieldOptions + 'static,
        direction: ReadDirection,
    ) -> Self {
        let options = Computed::new(options);
        let engine = Computed::new(move || options.with(TimeFieldOptions::engine));
        let invalid = Computed::new(move || {
            let t = Time::of_entry(value.get());
            options.with(|o| o.out_of_bounds(t))
        });
        Self {
            field: DateField::time_of(value, engine, invalid, direction),
        }
    }

    /// The time (tracked).
    #[must_use]
    pub fn value(self) -> Time {
        Time::of_entry(self.field.value())
    }

    /// Whether the time is outside its bounds (`aria-invalid`, tracked).
    #[must_use]
    pub fn invalid(self) -> bool {
        self.field.invalid()
    }

    /// Whether it can be used (tracked): what a [`crate::form::FormField`]
    /// of it reads, so the form and the control agree.
    #[must_use]
    pub fn availability(self) -> Availability {
        self.field.availability()
    }

    /// Whether it is read-only (tracked).
    #[must_use]
    pub fn readonly(self) -> bool {
        self.field.readonly()
    }

    /// The pieces in the locale's order (tracked).
    #[must_use]
    pub fn pieces(self) -> Vec<Piece> {
        self.field.pieces()
    }

    /// The segments in order (tracked).
    #[must_use]
    pub fn segments(self) -> Vec<Segment> {
        self.field.segments()
    }

    /// What segment `s` shows (tracked): see [`DateField::text`].
    #[must_use]
    pub fn text(self, s: Segment) -> String {
        self.field.text(s)
    }

    /// Whether segment `s` is empty (tracked).
    #[must_use]
    pub fn is_empty(self, s: Segment) -> bool {
        self.field.is_empty(s)
    }

    /// Segment `s`'s range as typed and read (tracked).
    #[must_use]
    pub fn bounds(self, s: Segment) -> (i32, i32) {
        self.field.bounds(s)
    }

    /// The strings it names its parts with (tracked).
    #[must_use]
    pub fn strings(self) -> DateStrings {
        self.field.strings()
    }

    /// Whether one of its segments holds focus (tracked).
    #[must_use]
    pub fn focused(self) -> bool {
        self.field.focused()
    }

    /// Focus the first segment (a label pressed, a form's error).
    pub fn focus_first(self) -> bool {
        self.field.focus_first()
    }

    /// Focus `s`; false when there is none.
    pub fn focus_segment(self, s: Option<Segment>) -> bool {
        self.field.focus_segment(s)
    }

    /// The focus handle of the first (or last) segment (tracked).
    #[must_use]
    pub fn edge(self, last: bool) -> Option<FocusHandle> {
        self.field.edge(last)
    }

    /// Hand ArrowRight on the last segment to `handle`.
    pub fn set_after(self, handle: Option<FocusHandle>) {
        self.field.set_after(handle);
    }

    /// Hand ArrowLeft on the first segment to `handle`.
    pub fn set_before(self, handle: Option<FocusHandle>) {
        self.field.set_before(handle);
    }

    /// What a key does in segment `s`; whether it took it.
    pub fn key(self, s: Segment, ev: &KeyEvent) -> bool {
        self.field.key(s, ev)
    }

    /// `d` as the box around the segments (see [`DateField::root`]).
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        self.field.root(d)
    }

    /// `d` as a literal between segments: hidden from assistive technology.
    #[must_use]
    pub fn literal(self, d: Div) -> Div {
        self.field.literal(d)
    }

    /// `d` as segment `s` (see [`DateField::segment`]). Called in the
    /// render (tracked).
    #[must_use]
    pub fn segment(self, s: Segment, d: Div) -> Div {
        self.field.segment(s, d)
    }
}

#[cfg(test)]
mod tests {
    use gui::events::{Key, KeyEvent, Modifiers, NamedKey};
    use gui::{Bind, Direction, Signal, use_model};

    use super::{Time, TimeField, TimeFieldOptions};
    use crate::availability::Availability;
    use crate::date::CalendarLocale;
    use crate::date_field::{HourCycle, Piece, Precision, Segment, Steps};
    use crate::direction::ReadDirection;

    type Field = TimeField<Bind<Time>>;

    fn make(t: Time, o: TimeFieldOptions, direction: Direction) -> Field {
        TimeField::new(
            use_model(Signal::new(Bind::default().with_default(t)), |b| b),
            move || o.clone(),
            ReadDirection::fixed(direction),
        )
    }

    /// A 24-hour field keeping its own value, starting at `t`.
    fn field(t: Time) -> Field {
        make(
            t,
            TimeFieldOptions {
                hour_cycle: Some(HourCycle::H23),
                ..TimeFieldOptions::default()
            },
            Direction::Ltr,
        )
    }

    fn press(f: Field, s: Segment, key: NamedKey) -> bool {
        f.key(
            s,
            &KeyEvent {
                key: Key::Named(key),
                text: None,
                modifiers: Modifiers::default(),
            },
        )
    }

    fn type_in(f: Field, s: Segment, digits: &str) -> bool {
        f.key(
            s,
            &KeyEvent {
                key: Key::Character(digits.into()),
                text: Some(digits.to_owned()),
                modifiers: Modifiers::default(),
            },
        )
    }

    fn now(f: Field) -> Time {
        gui::untracked(|| f.value())
    }

    #[test]
    fn a_time_parses_and_sends_to_its_precision() {
        assert_eq!(Time::parse("10:30").value(), "10:30");
        assert_eq!(Time::parse("10:30:05").value(), "10:30:05");
        assert_eq!(Time::parse("10").precision, Precision::Hour);
        assert_eq!(Time::parse("10").value(), "10:00");
        assert_eq!(Time::parse("09:--:--").value(), "", "partly typed, nothing is sent");
        assert_eq!(Time::parse("24:61").hour, None, "out of range is empty");
    }

    #[test]
    fn arrows_step_and_wrap_and_digits_type_by_chromes_rule() {
        let f = field(Time::parse("23:30:--"));
        assert!(press(f, Segment::Hour, NamedKey::ArrowUp));
        assert_eq!(now(f).hour, Some(0), "23 wraps to 00");
        press(f, Segment::Second, NamedKey::ArrowDown);
        assert_eq!(now(f).value(), "00:30:59", "an empty segment steps down to its highest");
        type_in(f, Segment::Minute, "4");
        assert_eq!(now(f).minute, Some(4), "4 waits for a second digit");
        type_in(f, Segment::Minute, "5");
        assert_eq!(now(f).minute, Some(45));
        let f = field(Time::empty(Precision::Second));
        type_in(f, Segment::Hour, "25");
        assert_eq!(now(f).hour, Some(23), "2 then 5 gives 23, not reka's 5");
        assert!(press(f, Segment::Hour, NamedKey::Backspace));
        assert_eq!(now(f).hour, None, "Backspace empties the whole segment");
    }

    #[test]
    fn the_granularity_and_the_clock_decide_the_segments() {
        let minute = make(
            Time::empty(Precision::Minute),
            TimeFieldOptions::default(),
            Direction::Ltr,
        );
        assert_eq!(
            gui::untracked(|| minute.pieces()),
            vec![
                Piece::Segment(Segment::Hour),
                Piece::Literal(":"),
                Piece::Segment(Segment::Minute),
                Piece::Literal(" "),
                Piece::Segment(Segment::DayPeriod),
            ],
            "en-US tells the time on 12 hours, to the minute by default"
        );
        let vi = make(
            Time::empty(Precision::Hour),
            TimeFieldOptions {
                locale: CalendarLocale::Vi,
                ..TimeFieldOptions::default()
            },
            Direction::Ltr,
        );
        assert_eq!(gui::untracked(|| vi.segments()), vec![Segment::Hour], "vi: 24 hours");
        assert_eq!(gui::untracked(|| vi.bounds(Segment::Hour)), (0, 23));
        assert_eq!(gui::untracked(|| minute.bounds(Segment::Hour)), (1, 12));
    }

    #[test]
    fn a_twelve_hour_clock_types_one_to_twelve_and_the_period_moves_the_hour() {
        let f = make(
            Time::empty(Precision::Minute),
            TimeFieldOptions::default(),
            Direction::Ltr,
        );
        type_in(f, Segment::DayPeriod, "p");
        type_in(f, Segment::Hour, "3");
        assert_eq!(now(f).hour, Some(15), "3 PM, the period chosen first kept");
        assert_eq!(gui::untracked(|| f.text(Segment::Hour)), "03");
        press(f, Segment::DayPeriod, NamedKey::ArrowUp);
        assert_eq!(now(f).hour, Some(3), "the other half of the day");
        type_in(f, Segment::Hour, "12");
        assert_eq!(now(f).hour, Some(0), "12 AM is midnight");
    }

    #[test]
    fn the_arrows_step_by_the_units_step_and_leaving_rounds_to_it() {
        let f = make(
            Time::parse("10:58"),
            TimeFieldOptions {
                hour_cycle: Some(HourCycle::H23),
                steps: Steps {
                    minute: 15,
                    ..Steps::default()
                },
                round_to_step: true,
                ..TimeFieldOptions::default()
            },
            Direction::Ltr,
        );
        press(f, Segment::Minute, NamedKey::ArrowUp);
        assert_eq!(
            now(f).minute,
            Some(13),
            "58 + 15 wraps to 13: the arrows step, not round"
        );
        let r = f.field.rounded(Segment::Minute, Time::parse("10:58").entry());
        assert_eq!(r.minute, Some(45), "58 rounds to 45: 60 passes the largest");
        let r = f.field.rounded(Segment::Minute, Time::parse("10:07").entry());
        assert_eq!(r.minute, Some(0));
        let r = f.field.rounded(Segment::Minute, Time::parse("10:08").entry());
        assert_eq!(r.minute, Some(15));
    }

    #[test]
    fn a_time_outside_its_bounds_is_invalid_and_kept() {
        let f = make(
            Time::parse("08:00"),
            TimeFieldOptions {
                hour_cycle: Some(HourCycle::H23),
                min: Some(Time::parse("09:00")),
                max: Some(Time::parse("17:30")),
                ..TimeFieldOptions::default()
            },
            Direction::Ltr,
        );
        assert!(gui::untracked(|| f.invalid()), "08:00 is before 09:00");
        type_in(f, Segment::Hour, "12");
        assert!(!gui::untracked(|| f.invalid()));
        type_in(f, Segment::Hour, "18");
        assert_eq!(now(f).hour, Some(18), "kept as typed, not held");
        assert!(gui::untracked(|| f.invalid()));
    }

    #[test]
    fn the_arrows_between_segments_mirror_right_to_left() {
        let f = make(
            Time::parse("10:20:30"),
            TimeFieldOptions {
                hour_cycle: Some(HourCycle::H23),
                ..TimeFieldOptions::default()
            },
            Direction::Rtl,
        );
        assert!(
            !press(f, Segment::Hour, NamedKey::ArrowRight),
            "right to left, ArrowRight goes back"
        );
        assert!(press(f, Segment::Hour, NamedKey::ArrowLeft));
    }

    #[test]
    fn a_field_that_cannot_be_used_keeps_its_value() {
        // (availability, readonly, whether ArrowRight still moves)
        let table = [
            (Availability::Disabled, false, false),
            (Availability::Unavailable, false, true),
            (Availability::Enabled, true, true),
        ];
        for (availability, readonly, moves) in table {
            let f = make(
                Time::parse("10:20:30"),
                TimeFieldOptions {
                    availability,
                    readonly,
                    hour_cycle: Some(HourCycle::H23),
                    ..TimeFieldOptions::default()
                },
                Direction::Ltr,
            );
            assert!(
                !press(f, Segment::Hour, NamedKey::ArrowUp),
                "{availability:?} {readonly}"
            );
            assert!(!press(f, Segment::Hour, NamedKey::Backspace));
            type_in(f, Segment::Hour, "7");
            assert_eq!(
                now(f),
                Time::parse("10:20:30"),
                "{availability:?} {readonly}: unchanged"
            );
            assert_eq!(press(f, Segment::Hour, NamedKey::ArrowRight), moves);
        }
    }

    #[test]
    fn a_shortcut_is_left_to_the_page() {
        let f = field(Time::parse("10:20:30"));
        for modifiers in [
            Modifiers {
                control: true,
                ..Modifiers::default()
            },
            Modifiers {
                command: true,
                ..Modifiers::default()
            },
        ] {
            let took = f.key(
                Segment::Hour,
                &KeyEvent {
                    key: Key::Named(NamedKey::ArrowUp),
                    text: None,
                    modifiers,
                },
            );
            assert!(!took);
            assert_eq!(now(f), Time::parse("10:20:30"));
        }
    }

    #[test]
    fn a_caller_keeping_the_value_decides_what_is_shown() {
        let props = Signal::new(Bind::value(Time::parse("10:--:--")));
        let f = TimeField::new(
            use_model(props, |b| b),
            || TimeFieldOptions {
                hour_cycle: Some(HourCycle::H23),
                ..TimeFieldOptions::default()
            },
            ReadDirection::fixed(Direction::Ltr),
        );
        type_in(f, Segment::Hour, "2");
        assert_eq!(now(f).hour, Some(10), "the caller did not take the 2");
        props.set(Bind::value(Time::parse("02:--:--")));
        assert_eq!(now(f).hour, Some(2));
    }
}
