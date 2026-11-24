//! The date primitives (ADR-0025, group 7), by behaviour: the calendar's
//! keyboard table, its one Tab stop and the kinds of day it skips; picking
//! one day, many, and a range; a field typed segment by segment; a picker
//! joining a field and a calendar over one value; and what a screen reader
//! is told of each. The cases and their numbers are reka's tests
//! (`Calendar.test.ts`, `RangeCalendar.test.ts`, `DateField.test.ts`,
//! `DatePicker.test.ts`) where they agree with the research's notes, and
//! the fixes ADR-0025 records where reka's code is wrong.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "a scenario that cannot run, or a node not found, is a failed test"
)]

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use accesskit::{Node, NodeId, TreeUpdate};
use gui::app::Scenario;
use gui::events::{Key, KeyEvent, Modifiers, NamedKey};
use gui::prelude::*;
use gui::{Bind, Derive, Direction, Element, Signal, component, div, text, use_model};
use primitives::Availability;
use primitives::calendar::{Calendar, CalendarGrid, CalendarOptions, CellKey};
use primitives::date::{CalendarDate, CalendarLocale, CalendarSystem};
use primitives::date_field::{
    DateEntry, DateField, DateFieldOptions, DateRangeEntry, HourCycle, Piece, Precision, Segment,
};
use primitives::date_picker::{DatePicker, DateRangePicker};
use primitives::direction::ReadDirection;
use primitives::form::{FormData, Submit};
use primitives::range_calendar::{DateRange, RangeCalendar, RangeEnd, RangeOptions};
use primitives::time_field::{Time, TimeField, TimeFieldOptions};

fn day(y: i32, m: u8, d: u8) -> CalendarDate {
    CalendarDate::new(y, m, d).expect("a real day")
}

fn ltr() -> ReadDirection {
    ReadDirection::fixed(Direction::Ltr)
}

fn named(k: NamedKey, modifiers: Modifiers) -> KeyEvent {
    KeyEvent {
        key: Key::Named(k),
        text: None,
        modifiers,
    }
}

fn plain(k: NamedKey) -> KeyEvent {
    named(k, Modifiers::default())
}

fn shift(k: NamedKey) -> KeyEvent {
    named(
        k,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    )
}

fn typed(c: char) -> KeyEvent {
    let t = c.to_string();
    KeyEvent {
        key: Key::Character(t.as_str().into()),
        text: Some(t),
        modifiers: Modifiers::default(),
    }
}

fn options(today: CalendarDate) -> CalendarOptions {
    CalendarOptions {
        today,
        ..CalendarOptions::default()
    }
}

type Single = Calendar<Bind<Option<CalendarDate>>, Option<CalendarDate>>;

/// A calendar keeping its own value, starting at `value`, on `options`.
fn single(value: Option<CalendarDate>, o: CalendarOptions, direction: ReadDirection) -> Single {
    let model = use_model(Signal::new(Bind::default().with_default(value)), |b| b);
    Calendar::new(model, move || o.clone(), None, direction)
}

/// A key on the anchor's cell; what it did.
fn key_at(grid: CalendarGrid, ev: &KeyEvent) -> CellKey {
    let anchor = gui::untracked(|| grid.anchor());
    let month = gui::untracked(|| grid.months()[0]);
    let d = gui::untracked(|| grid.day(anchor, month));
    grid.key(d, ev)
}

fn anchor(grid: CalendarGrid) -> CalendarDate {
    gui::untracked(|| grid.anchor())
}

#[test]
fn the_arrows_step_days_and_weeks_and_mirror_right_to_left() {
    let c = single(Some(day(2024, 1, 10)), options(day(2024, 1, 10)), ltr());
    let g = c.grid;
    for (k, to) in [
        (NamedKey::ArrowRight, day(2024, 1, 11)),
        (NamedKey::ArrowLeft, day(2024, 1, 10)),
        (NamedKey::ArrowDown, day(2024, 1, 17)),
        (NamedKey::ArrowUp, day(2024, 1, 10)),
    ] {
        assert_eq!(key_at(g, &plain(k)), CellKey::Moved);
        assert_eq!(anchor(g), to, "{k:?}");
    }
    // Past the month: the page turns with it.
    for _ in 0..4 {
        key_at(g, &plain(NamedKey::ArrowDown));
    }
    assert_eq!(anchor(g), day(2024, 2, 7));
    assert_eq!(gui::untracked(|| g.first_month()), day(2024, 2, 1));
    let rtl = single(
        Some(day(2024, 1, 10)),
        options(day(2024, 1, 10)),
        ReadDirection::fixed(Direction::Rtl),
    );
    key_at(rtl.grid, &plain(NamedKey::ArrowRight));
    assert_eq!(anchor(rtl.grid), day(2024, 1, 9), "right to left, ArrowRight goes back");
}

#[test]
fn home_end_and_the_page_keys_keep_the_day_or_the_months_last() {
    // Home from 1 January 2021 (a Friday) is 27 December 2020, its month shown.
    let c = single(Some(day(2021, 1, 1)), options(day(2021, 1, 1)), ltr());
    key_at(c.grid, &plain(NamedKey::Home));
    assert_eq!(anchor(c.grid), day(2020, 12, 27));
    assert_eq!(gui::untracked(|| c.grid.first_month()), day(2020, 12, 1));
    key_at(c.grid, &plain(NamedKey::End));
    assert_eq!(anchor(c.grid), day(2021, 1, 2));
    // 31 March 2024 PageUp: 29 February; Shift+PageUp: 28 February 2023.
    let c = single(Some(day(2024, 3, 31)), options(day(2024, 3, 31)), ltr());
    key_at(c.grid, &plain(NamedKey::PageUp));
    assert_eq!(anchor(c.grid), day(2024, 2, 29));
    key_at(c.grid, &shift(NamedKey::PageUp));
    assert_eq!(anchor(c.grid), day(2023, 2, 28));
    key_at(c.grid, &shift(NamedKey::PageDown));
    assert_eq!(anchor(c.grid), day(2024, 2, 28));
}

#[test]
fn an_arrow_past_the_bounds_stays_and_the_page_keys_are_held_to_them() {
    let o = CalendarOptions {
        min: Some(day(2024, 1, 5)),
        max: Some(day(2024, 1, 25)),
        ..options(day(2024, 1, 10))
    };
    let c = single(Some(day(2024, 1, 6)), o, ltr());
    key_at(c.grid, &plain(NamedKey::ArrowUp));
    assert_eq!(anchor(c.grid), day(2024, 1, 6), "6 − 7 is before min: stays, not held");
    key_at(c.grid, &plain(NamedKey::PageDown));
    assert_eq!(anchor(c.grid), day(2024, 1, 25), "held to max");
    key_at(c.grid, &plain(NamedKey::PageUp));
    assert_eq!(anchor(c.grid), day(2024, 1, 5), "held to min");
    assert!(!gui::untracked(|| c.grid.can_page(-1)), "Prev blocked by min");
    assert!(!gui::untracked(|| c.grid.can_page(1)), "Next blocked by max");
}

#[test]
fn arrows_skip_blocked_days_and_stop_on_unavailable_ones() {
    let blocked = day(2024, 1, 3);
    let booked = day(2024, 1, 11);
    let o = CalendarOptions {
        is_blocked: Some(Derive::new(move |d| d == blocked)),
        is_unavailable: Some(Derive::new(move |d| d == booked)),
        ..options(day(2024, 1, 10))
    };
    let c = single(Some(day(2024, 1, 10)), o, ltr());
    key_at(c.grid, &plain(NamedKey::ArrowUp));
    assert_eq!(anchor(c.grid), day(2023, 12, 27), "3 January blocked: on by a week");
    let o = CalendarOptions {
        is_blocked: Some(Derive::new(move |d| d == blocked)),
        is_unavailable: Some(Derive::new(move |d| d == booked)),
        ..options(day(2024, 1, 10))
    };
    let c = single(Some(day(2024, 1, 10)), o, ltr());
    key_at(c.grid, &plain(NamedKey::ArrowRight));
    assert_eq!(anchor(c.grid), booked, "an arrow stops on an unavailable day");
    // Home and End pass an unavailable day: from 11 January (Thursday),
    // End reaches Saturday 13, Home Sunday 7.
    key_at(c.grid, &plain(NamedKey::Home));
    assert_eq!(anchor(c.grid), day(2024, 1, 7));
}

#[test]
fn the_page_keys_look_on_at_most_24_days() {
    // February blocked: PageDown lands on 10 February and looks on to 1 March.
    let o = CalendarOptions {
        is_blocked: Some(Derive::new(|d: CalendarDate| d.month() == 2)),
        ..options(day(2024, 1, 10))
    };
    let c = single(Some(day(2024, 1, 10)), o, ltr());
    key_at(c.grid, &plain(NamedKey::PageDown));
    assert_eq!(anchor(c.grid), day(2024, 3, 1));
    // Blocked to 10 March: it gives up 24 days on, on 5 March.
    let o = CalendarOptions {
        is_blocked: Some(Derive::new(|d: CalendarDate| {
            d.month() == 2 || (d.month() == 3 && d.day() <= 10)
        })),
        ..options(day(2024, 1, 10))
    };
    let c = single(Some(day(2024, 1, 10)), o, ltr());
    key_at(c.grid, &plain(NamedKey::PageDown));
    assert_eq!(anchor(c.grid), day(2024, 3, 5), "10 February + 24 days");
}

#[test]
fn enter_and_space_pick_and_pick_again_clears_unless_kept() {
    let c = single(None, options(day(2024, 1, 10)), ltr());
    assert_eq!(key_at(c.grid, &plain(NamedKey::Enter)), CellKey::Pick);
    c.pick(day(2024, 1, 10));
    assert_eq!(gui::untracked(|| c.value()), Some(day(2024, 1, 10)));
    c.pick(day(2024, 1, 10));
    assert_eq!(gui::untracked(|| c.value()), None, "the picked day again clears it");
    let ctrl = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    assert_eq!(
        key_at(c.grid, &named(NamedKey::Enter, ctrl)),
        CellKey::Ignored,
        "Ctrl+Enter goes on to the page (a form's submit)"
    );
    let kept = single(
        Some(day(2024, 1, 10)),
        CalendarOptions {
            prevent_deselect: true,
            ..options(day(2024, 1, 10))
        },
        ltr(),
    );
    kept.pick(day(2024, 1, 10));
    assert_eq!(gui::untracked(|| kept.value()), Some(day(2024, 1, 10)));
}

#[test]
fn many_days_go_in_the_order_picked_and_none_left_is_empty() {
    let model = use_model(
        Signal::new(Bind::default().with_default(Vec::<CalendarDate>::new())),
        |b| b,
    );
    let o = options(day(2024, 1, 10));
    let c = Calendar::new(model, move || o.clone(), None, ltr());
    for d in [5, 2, 9] {
        c.pick(day(2024, 1, d));
    }
    assert_eq!(
        gui::untracked(|| c.value()),
        vec![day(2024, 1, 5), day(2024, 1, 2), day(2024, 1, 9)]
    );
    assert_eq!(
        anchor(c.grid),
        day(2024, 1, 9),
        "the anchor follows the last day picked"
    );
    for d in [2, 5, 9] {
        c.pick(day(2024, 1, d));
    }
    assert!(gui::untracked(|| c.value()).is_empty());
}

#[test]
fn readonly_and_disabled_calendars() {
    let ro = single(
        None,
        CalendarOptions {
            readonly: true,
            ..options(day(2024, 1, 10))
        },
        ltr(),
    );
    key_at(ro.grid, &plain(NamedKey::ArrowRight));
    assert_eq!(anchor(ro.grid), day(2024, 1, 11), "read-only keeps the keys");
    ro.pick(day(2024, 1, 11));
    assert_eq!(gui::untracked(|| ro.value()), None, "and picks nothing");
    let off = single(
        None,
        CalendarOptions {
            availability: Availability::Disabled,
            ..options(day(2024, 1, 10))
        },
        ltr(),
    );
    let d = gui::untracked(|| off.grid.day(day(2024, 1, 10), day(2024, 1, 1)));
    assert!(d.blocked, "every day is blocked");
    assert!(
        !gui::untracked(|| off.grid.can_page(1)),
        "and the page goes nowhere, with no max either"
    );
}

#[test]
fn a_caller_value_is_shown_as_it_is() {
    // A plain value the caller does not write back: a pick changes nothing shown.
    let props = Signal::new(Bind::value(Some(day(2024, 1, 3))));
    let model = use_model(props, |b| b);
    let o = CalendarOptions {
        min: Some(day(2024, 1, 5)),
        ..options(day(2024, 1, 10))
    };
    let c = Calendar::new(model, move || o.clone(), None, ltr());
    assert!(gui::untracked(|| c.invalid()), "before min: kept, and invalid");
    c.pick(day(2024, 1, 8));
    assert_eq!(gui::untracked(|| c.value()), Some(day(2024, 1, 3)));
    // The caller writing a day in another month brings it into view.
    props.set(Bind::value(Some(day(2024, 4, 2))));
    assert_eq!(anchor(c.grid), day(2024, 4, 2));
    assert_eq!(gui::untracked(|| c.grid.first_month()), day(2024, 4, 1));
}

#[test]
fn the_tab_stop_is_the_anchor_or_the_first_usable_day_from_min() {
    // The anchor before min in min's month: reka skips the whole month and
    // loses the stop; here the stop is min.
    let o = CalendarOptions {
        min: Some(day(2024, 1, 20)),
        ..options(day(2024, 1, 10))
    };
    let c = single(None, o, ltr());
    let jan = day(2024, 1, 1);
    let stops: Vec<CalendarDate> = (1..=31)
        .map(|d| day(2024, 1, d))
        .filter(|d| c.grid.is_stop(*d, jan, || c.grid.first_focusable()))
        .collect();
    assert_eq!(stops, [day(2024, 1, 20)]);
}

#[test]
fn a_range_is_built_by_presses_as_reka_tests_it() {
    let model = use_model(Signal::new(Bind::default().with_default(DateRange::EMPTY)), |b| b);
    let o = options(day(2024, 1, 10));
    let r = RangeCalendar::new(model, move || o.clone(), RangeOptions::default, None, ltr());
    let press = |d: u8| {
        r.point(Some(day(2024, 1, d)));
        r.press(day(2024, 1, d));
        gui::untracked(|| r.value())
    };
    assert_eq!(press(5), DateRange::started(day(2024, 1, 5)));
    assert_eq!(press(7), DateRange::between(day(2024, 1, 5), day(2024, 1, 7)));
    assert_eq!(
        press(8),
        DateRange::started(day(2024, 1, 8)),
        "a third press starts again"
    );
    assert_eq!(
        press(8),
        DateRange::between(day(2024, 1, 8), day(2024, 1, 8)),
        "a one-day range"
    );
    assert_eq!(press(8), DateRange::EMPTY, "pressed again, cleared");
    assert_eq!(press(9), DateRange::started(day(2024, 1, 9)));
    assert_eq!(
        press(3),
        DateRange::between(day(2024, 1, 3), day(2024, 1, 9)),
        "the ends swap"
    );
}

#[test]
fn a_range_over_a_blocked_day_takes_a_second_press_and_escape_puts_back_the_last() {
    let blocked = day(2024, 1, 6);
    let model = use_model(
        Signal::new(Bind::default().with_default(DateRange::between(day(2024, 1, 1), day(2024, 1, 2)))),
        |b| b,
    );
    let o = CalendarOptions {
        is_blocked: Some(Derive::new(move |d| d == blocked)),
        ..options(day(2024, 1, 10))
    };
    let r = RangeCalendar::new(model, move || o.clone(), RangeOptions::default, None, ltr());
    let press = |d: u8| {
        r.point(Some(day(2024, 1, d)));
        r.press(day(2024, 1, d));
        gui::untracked(|| r.value())
    };
    assert_eq!(press(4), DateRange::started(day(2024, 1, 4)));
    assert_eq!(gui::untracked(|| r.preview()), Some((day(2024, 1, 4), day(2024, 1, 4))));
    r.point(Some(day(2024, 1, 8)));
    assert_eq!(gui::untracked(|| r.preview()), None, "no preview across a blocked day");
    assert_eq!(
        press(8),
        DateRange::started(day(2024, 1, 4)),
        "a first press is ignored"
    );
    assert_eq!(press(8), DateRange::started(day(2024, 1, 8)), "the second starts there");
    assert!(r.editing());
    assert!(!r.escape(&plain(NamedKey::Escape)), "Escape goes on to the page");
    assert_eq!(
        gui::untracked(|| r.value()),
        DateRange::between(day(2024, 1, 1), day(2024, 1, 2)),
        "the last whole range is back"
    );
}

/// The preview reads the days inside it tracked: a day the caller's
/// `is_blocked` or `is_unavailable` (each reading a signal) closes inside
/// it since draws it again, without one.
#[test]
fn a_range_preview_is_drawn_again_when_a_day_inside_it_is_closed_since() {
    let blocked = Signal::new(None::<u8>);
    let unavailable = Signal::new(None::<u8>);
    let model = use_model(Signal::new(Bind::default().with_default(DateRange::EMPTY)), |b| b);
    let o = CalendarOptions {
        is_blocked: Some(Derive::new(move |d: CalendarDate| blocked.get() == Some(d.day()))),
        is_unavailable: Some(Derive::new(move |d: CalendarDate| unavailable.get() == Some(d.day()))),
        ..options(day(2024, 1, 10))
    };
    let r = RangeCalendar::new(model, move || o.clone(), RangeOptions::default, None, ltr());
    r.point(Some(day(2024, 1, 4)));
    r.press(day(2024, 1, 4));
    r.point(Some(day(2024, 1, 8)));
    let preview = gui::Computed::new(move || r.preview());
    assert_eq!(preview.get_untracked(), Some((day(2024, 1, 4), day(2024, 1, 8))));
    blocked.set(Some(6));
    assert_eq!(preview.get_untracked(), None, "a day blocked inside it");
    blocked.set(None);
    assert_eq!(preview.get_untracked(), Some((day(2024, 1, 4), day(2024, 1, 8))));
    unavailable.set(Some(7));
    assert_eq!(preview.get_untracked(), None, "a day unavailable inside it");
}

#[test]
fn maximum_days_and_a_fixed_start() {
    let model = use_model(Signal::new(Bind::default().with_default(DateRange::EMPTY)), |b| b);
    let o = options(day(2024, 1, 10));
    let rule = RangeOptions {
        maximum_days: Some(3),
        ..RangeOptions::default()
    };
    let r = RangeCalendar::new(model, move || o.clone(), move || rule.clone(), None, ltr());
    r.press(day(2024, 1, 10));
    assert!(
        gui::untracked(|| r.grid.blocks(day(2024, 1, 13))),
        "three days from the start is too far"
    );
    assert!(!gui::untracked(|| r.grid.blocks(day(2024, 1, 12))));
    assert!(gui::untracked(|| r.grid.blocks(day(2024, 1, 7))), "either way");
    r.point(Some(day(2024, 1, 11)));
    assert_eq!(
        gui::untracked(|| r.preview()),
        Some((day(2024, 1, 10), day(2024, 1, 12))),
        "the window of three days toward the pointed one"
    );

    let model = use_model(
        Signal::new(Bind::default().with_default(DateRange::between(day(2024, 1, 10), day(2024, 1, 12)))),
        |b| b,
    );
    let o = options(day(2024, 1, 10));
    let fixed = RangeOptions {
        fixed: Some(RangeEnd::Start),
        ..RangeOptions::default()
    };
    let r = RangeCalendar::new(model, move || o.clone(), move || fixed.clone(), None, ltr());
    r.press(day(2024, 1, 15));
    assert_eq!(
        gui::untracked(|| r.value()),
        DateRange::between(day(2024, 1, 10), day(2024, 1, 15))
    );
    r.press(day(2024, 1, 4));
    assert_eq!(
        gui::untracked(|| r.value()),
        DateRange::between(day(2024, 1, 4), day(2024, 1, 15)),
        "before a fixed start, the start moves"
    );
}

type Field = DateField<Bind<DateEntry>>;

fn field(entry: DateEntry, o: DateFieldOptions) -> Field {
    let model = use_model(Signal::new(Bind::default().with_default(entry)), |b| b);
    DateField::new(model, move || o.clone(), ltr())
}

fn type_in(f: Field, s: Segment, text: &str) {
    f.key(
        s,
        &KeyEvent {
            key: Key::Character(text.into()),
            text: Some(text.to_owned()),
            modifiers: Modifiers::default(),
        },
    );
}

fn en() -> DateFieldOptions {
    DateFieldOptions {
        anchor: day(2026, 11, 22),
        ..DateFieldOptions::default()
    }
}

#[test]
fn a_date_is_typed_segment_by_segment_in_the_locales_order() {
    let f = field(DateEntry::empty(CalendarSystem::Gregorian, Precision::Day), en());
    assert_eq!(
        gui::untracked(|| f.segments()),
        [Segment::Month, Segment::Day, Segment::Year]
    );
    type_in(f, Segment::Month, "1");
    type_in(f, Segment::Month, "3");
    let v = gui::untracked(|| f.value());
    assert_eq!(v.month, Some(12), "13 is held to 12, as Chrome holds it");
    type_in(f, Segment::Day, "0");
    assert_eq!(gui::untracked(|| f.value()).day, None, "a leading 0 waits, empty");
    type_in(f, Segment::Day, "5");
    type_in(f, Segment::Year, "2026");
    let v = gui::untracked(|| f.value());
    assert_eq!(v.date(), Some(day(2026, 12, 5)));
    assert_eq!(v.form_data(), FormData::Text("2026-12-05".to_owned()));
    f.key(Segment::Day, &plain(NamedKey::Backspace));
    let v = gui::untracked(|| f.value());
    assert_eq!(
        (v.month, v.day, v.year),
        (Some(12), None, Some(2026)),
        "Backspace empties one segment"
    );
    assert_eq!(v.iso(), "", "and the form gets nothing while one is empty");

    let vi = field(
        DateEntry::empty(CalendarSystem::Gregorian, Precision::Day),
        DateFieldOptions {
            locale: CalendarLocale::Vi,
            ..en()
        },
    );
    assert_eq!(
        gui::untracked(|| vi.segments()),
        [Segment::Day, Segment::Month, Segment::Year]
    );
    assert_eq!(gui::untracked(|| vi.text(Segment::Day)), "dd");
    assert_eq!(gui::untracked(|| vi.text(Segment::Year)), "yyyy");
}

#[test]
fn the_29th_of_february_types_before_its_year() {
    // en-US types the month and day before the year: reka holds the day to
    // the anchor's year (a common year here) and refuses 29.
    let f = field(
        DateEntry::empty(CalendarSystem::Gregorian, Precision::Day),
        DateFieldOptions {
            anchor: day(2026, 1, 1),
            ..en()
        },
    );
    type_in(f, Segment::Month, "2");
    type_in(f, Segment::Day, "29");
    type_in(f, Segment::Year, "2024");
    assert_eq!(gui::untracked(|| f.value()).date(), Some(day(2024, 2, 29)));
    // A common year typed then holds the day to 28.
    let f = field(gui::untracked(|| f.value()), en());
    f.key(Segment::Year, &plain(NamedKey::Backspace));
    type_in(f, Segment::Year, "2025");
    assert_eq!(gui::untracked(|| f.value()).date(), Some(day(2025, 2, 28)));
}

#[test]
fn arrows_step_and_wrap_and_an_empty_year_starts_at_the_anchors() {
    let f = field(
        DateEntry::of(CalendarSystem::Gregorian, Precision::Day, day(2026, 12, 31)),
        en(),
    );
    f.key(Segment::Month, &plain(NamedKey::ArrowUp));
    assert_eq!(gui::untracked(|| f.value()).month, Some(1), "12 wraps to 1");
    f.key(Segment::Day, &plain(NamedKey::ArrowUp));
    assert_eq!(gui::untracked(|| f.value()).day, Some(1), "31 wraps to 1");
    let f = field(DateEntry::empty(CalendarSystem::Gregorian, Precision::Day), en());
    f.key(Segment::Year, &plain(NamedKey::ArrowDown));
    assert_eq!(gui::untracked(|| f.value()).year, Some(2026));
    f.key(Segment::Month, &plain(NamedKey::ArrowDown));
    assert_eq!(gui::untracked(|| f.value()).month, Some(12), "empty, down: the highest");
}

#[test]
fn a_twelve_hour_clock_types_one_to_twelve_and_keeps_the_period() {
    let f = field(
        DateEntry::of(CalendarSystem::Gregorian, Precision::Minute, day(2026, 1, 1)),
        en(),
    );
    let order = gui::untracked(|| f.segments());
    assert_eq!(
        order.last(),
        Some(&Segment::DayPeriod),
        "en-US tells time on a 12-hour clock"
    );
    f.key(Segment::DayPeriod, &typed('p'));
    type_in(f, Segment::Hour, "0");
    assert_eq!(gui::untracked(|| f.value()).hour, None, "0 is below 1: waits");
    type_in(f, Segment::Hour, "3");
    assert_eq!(gui::untracked(|| f.value()).hour, Some(15), "3 PM");
    type_in(f, Segment::Minute, "7");
    assert_eq!(gui::untracked(|| f.value()).minute, Some(7));
    assert_eq!(gui::untracked(|| f.text(Segment::Hour)), "03");
    assert_eq!(gui::untracked(|| f.text(Segment::DayPeriod)), "PM");
    f.key(Segment::DayPeriod, &plain(NamedKey::ArrowUp));
    assert_eq!(gui::untracked(|| f.value()).hour, Some(3), "the other half of the day");
    assert_eq!(gui::untracked(|| f.value()).iso(), "2026-01-01T03:07");
    let h23 = field(
        DateEntry::of(CalendarSystem::Gregorian, Precision::Hour, day(2026, 1, 1)),
        DateFieldOptions {
            hour_cycle: Some(HourCycle::H23),
            ..en()
        },
    );
    type_in(h23, Segment::Hour, "25");
    assert_eq!(
        gui::untracked(|| h23.value()).hour,
        Some(23),
        "25 is held to 23, never 24"
    );
}

#[test]
fn persian_digits_type_as_latin_ones() {
    let f = field(DateEntry::empty(CalendarSystem::Gregorian, Precision::Day), en());
    type_in(f, Segment::Month, "۰۴");
    assert_eq!(gui::untracked(|| f.value()).month, Some(4));
}

/// The pieces of a field, as a test look lays them out.
fn field_look<P: Clone + PartialEq + 'static>(f: DateField<P>) -> gui::Div {
    let mut row = f.root(div().display(Display::Flex));
    for piece in f.pieces() {
        row = match piece {
            Piece::Segment(s) => row.child(f.segment(s, div().width(30.0).height(20.0).child(f.text(s)))),
            Piece::Literal(t) => row.child(f.literal(div().child(text(t)))),
        };
    }
    row
}

fn calendar_look(c: Single) -> Element {
    let g = c.grid;
    let mut root = div().display(Display::Flex);
    for month in g.months() {
        let mut grid = g.grid(div().display(Display::Flex).flex_direction(FlexDirection::Column));
        grid = grid.child(
            g.head(
                div()
                    .display(Display::Flex)
                    .children(g.weekdays().map(|w| div().width(30.0).child(w))),
            ),
        );
        for week in g.weeks(month) {
            let mut row = g.row(div().display(Display::Flex));
            for d in week {
                let cell = g.day(d, month);
                row = row.child(
                    c.cell(cell, div())
                        .child(c.trigger(cell, div().width(30.0).height(30.0).focusable().child(g.number(d)))),
                );
            }
            grid = grid.child(row);
        }
        root = root.child(grid);
    }
    g.root(root).into_element()
}

fn dump(update: &TreeUpdate) -> String {
    let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let mut out = String::new();
    let mut stack = vec![(NodeId(0), 0_usize)];
    while let Some((id, depth)) = stack.pop() {
        let Some(n) = nodes.get(&id) else { continue };
        let mut line = format!("{}{:?}", "  ".repeat(depth), n.role());
        if let Some(nm) = n.label().or_else(|| n.value()) {
            line.push_str(&format!(" {nm:?}"));
        }
        if let (Some(min), Some(max)) = (n.min_numeric_value(), n.max_numeric_value()) {
            let now = n.numeric_value().map_or_else(|| "none".to_owned(), |v| v.to_string());
            line.push_str(&format!(" value={now} in {min}..={max}"));
            if let Some(t) = n.value() {
                line.push_str(&format!(" text={t:?}"));
            }
        }
        if n.is_selected() == Some(true) {
            line.push_str(" selected");
        }
        if n.toggled().is_some() {
            line.push_str(&format!(" toggled={:?}", n.toggled()));
        }
        if n.is_disabled() {
            line.push_str(" disabled");
        }
        if n.is_hidden() {
            line.push_str(" hidden");
        }
        if n.invalid().is_some() {
            line.push_str(" invalid");
        }
        if n.is_read_only() {
            line.push_str(" readonly");
        }
        if let Some(level) = n.level() {
            line.push_str(&format!(" level={}", level.saturating_add(1)));
        }
        out.push_str(&line);
        out.push('\n');
        for c in n.children().iter().rev() {
            stack.push((*c, depth.saturating_add(1)));
        }
    }
    out
}

fn scenario(f: impl Fn() -> Element + 'static) -> Scenario {
    let mut s = Scenario::measuring(f, 800, 600, 1.0).expect("a scenario");
    s.settle().expect("settled");
    s
}

/// Every Scenario starts on the fixed default day, whatever an earlier one
/// on the thread did to the clock: `today()` names the same day in each,
/// and moves with a scenario's own clock.
#[test]
fn every_scenario_starts_on_the_fixed_day_and_moves_with_its_own_clock() {
    let mut first = scenario(|| div().into_element());
    let fixed = CalendarDate::today();
    first.wait(std::time::Duration::from_secs(3 * 86_400));
    assert_eq!(fixed.days_until(CalendarDate::today()), 3, "three days on in the first");
    let _second = scenario(|| div().into_element());
    assert_eq!(
        CalendarDate::today(),
        fixed,
        "the next scenario starts from the default day again"
    );
}

#[test]
fn a_calendar_in_a_window_one_tab_stop_and_what_a_screen_reader_is_told() {
    let made = Rc::new(Cell::new(None::<Single>));
    let probe = Rc::clone(&made);
    let blocked = day(2024, 2, 14);
    let mut s = scenario(move || {
        let probe = Rc::clone(&probe);
        component(Bind::default().with_default(Some(day(2024, 2, 9))), move |cx| {
            let value = use_model(cx.props(), |b: &Bind<Option<CalendarDate>>| b);
            let c = Calendar::new(
                value,
                move || CalendarOptions {
                    is_blocked: Some(Derive::new(move |d| d == blocked)),
                    is_unavailable: Some(Derive::new(|d: CalendarDate| d.day() == 15)),
                    ..options(day(2024, 2, 1))
                },
                None,
                ltr(),
            );
            probe.set(Some(c));
            move || {
                div()
                    .child(calendar_look(c))
                    .child(
                        div()
                            .focusable()
                            .tab_stop(true)
                            .aria_label("after")
                            .width(10.0)
                            .height(10.0),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    let c = made.get().expect("the calendar");
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("calendar_a11y", dump(&tree));
    // Tab enters on the anchor, the picked day.
    s.press(NamedKey::Tab, Modifiers::default());
    s.settle().expect("settled");
    assert!(c.grid.focus_handle().is_focused());
    assert_eq!(anchor(c.grid), day(2024, 2, 9));
    // Keys move focus with the anchor: down a week, back onto the
    // unavailable 15th (an arrow stops there).
    for k in [NamedKey::ArrowDown, NamedKey::ArrowLeft] {
        s.press(k, Modifiers::default());
        s.settle().expect("settled");
    }
    assert_eq!(anchor(c.grid), day(2024, 2, 15));
    assert!(c.grid.focus_handle().is_focused());
    s.press(NamedKey::Enter, Modifiers::default());
    s.settle().expect("settled");
    assert_eq!(
        gui::untracked(|| c.value()),
        Some(day(2024, 2, 9)),
        "an unavailable day is not picked"
    );
    s.press(NamedKey::ArrowRight, Modifiers::default());
    s.press(NamedKey::Space, Modifiers::default());
    s.settle().expect("settled");
    assert_eq!(gui::untracked(|| c.value()), Some(day(2024, 2, 16)));
    // Tab leaves the grid in one press, and Shift+Tab comes back to the
    // picked day.
    s.press(NamedKey::Tab, Modifiers::default());
    s.settle().expect("settled");
    assert!(!c.grid.focus_handle().is_focused());
    s.press(
        NamedKey::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    s.settle().expect("settled");
    assert!(c.grid.focus_handle().is_focused());
    assert_eq!(anchor(c.grid), day(2024, 2, 16));
}

/// `isDateDisabled` is called as the day renders, tracked: one that reads
/// a signal, made once, disables the day the signal names when it changes.
#[test]
fn a_calendars_is_date_disabled_that_reads_a_signal_renders_the_day_again() {
    let closed = Signal::new(14_u8);
    let mut s = scenario(move || {
        component(Bind::default().with_default(Some(day(2024, 2, 9))), move |cx| {
            let value = use_model(cx.props(), |b: &Bind<Option<CalendarDate>>| b);
            let c = Calendar::new(
                value,
                move || CalendarOptions {
                    is_blocked: Some(Derive::new(move |d: CalendarDate| d.day() == closed.get())),
                    ..options(day(2024, 2, 1))
                },
                None,
                ltr(),
            );
            move || calendar_look(c)
        })
        .into_element()
    });
    let disabled = |s: &Scenario, name: &str| {
        let mut ts = gui::text::TextSystem::new();
        let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
        tree.nodes
            .iter()
            .find(|(_, n)| n.label() == Some(name))
            .is_some_and(|(_, n)| n.is_disabled())
    };
    let (d14, d20) = ("Wednesday, February 14, 2024", "Tuesday, February 20, 2024");
    assert!(disabled(&s, d14) && !disabled(&s, d20), "the 14th closed");
    closed.set(20);
    s.settle().expect("settled");
    assert!(!disabled(&s, d14) && disabled(&s, d20), "the 20th closed instead");
}

#[test]
fn a_date_field_in_a_window_and_what_a_screen_reader_is_told() {
    let made = Rc::new(Cell::new(None::<Field>));
    let probe = Rc::clone(&made);
    let mut s = scenario(move || {
        let probe = Rc::clone(&probe);
        component(
            Bind::default().with_default(DateEntry::empty(CalendarSystem::Gregorian, Precision::Minute)),
            move |cx| {
                let value = use_model(cx.props(), |b: &Bind<DateEntry>| b);
                let f = DateField::new(value, en, ltr());
                probe.set(Some(f));
                move || field_look(f).into_element()
            },
        )
        .into_element()
    });
    let f = made.get().expect("the field");
    s.act("focus", || {
        f.focus_first();
    });
    s.settle().expect("settled");
    // A run of 3s by Chrome's rule (ADR-0025 §12): month 3 (no second digit
    // fits), day 33 held to 31, year 3333, hour 3 (12-hour), minute 3 waiting
    // for its second digit. reka reads 33 as 3 then 3 and drifts a segment.
    s.type_text("333333333");
    s.settle().expect("settled");
    let v = gui::untracked(|| f.value());
    assert_eq!(
        (v.month, v.day, v.year, v.hour, v.minute),
        (Some(3), Some(31), Some(3333), Some(3), Some(3))
    );
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("date_field_a11y", dump(&tree));
}

#[derive(Clone, PartialEq)]
struct PickerProps {
    value: Bind<DateEntry>,
    open: Bind<bool>,
}

#[test]
fn a_picker_joins_the_field_and_the_calendar_over_one_value() {
    let made = Rc::new(Cell::new(None::<DatePicker<PickerProps>>));
    let probe = Rc::clone(&made);
    let close = Rc::new(Cell::new(true));
    let closes = Rc::clone(&close);
    let open = Signal::new(true);
    let mut s = scenario(move || {
        let probe = Rc::clone(&probe);
        let closes = Rc::clone(&closes);
        component(
            PickerProps {
                value: Bind::default().with_default(
                    DateEntry::of(CalendarSystem::Gregorian, Precision::Minute, day(2024, 1, 5)).with_time(
                        Some(9),
                        Some(30),
                        None,
                    ),
                ),
                open: Bind::model(open),
            },
            move |cx| {
                let value = use_model(cx.props(), |p: &PickerProps| &p.value);
                let field = DateField::new(value, en, ltr());
                let opened = use_model(cx.props(), |p: &PickerProps| &p.open);
                let picker = DatePicker::new(field, value, opened, move || closes.get(), ltr());
                probe.set(Some(picker));
                move || {
                    let mut row = field_look(field).child(picker.trigger(div().width(20.0).height(20.0)));
                    if picker.is_open() {
                        row = row.child(picker.content(div().width(100.0).height(100.0)));
                    }
                    row.into_element()
                }
            },
        )
        .into_element()
    });
    let p = made.get().expect("the picker");
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("date_picker_a11y", dump(&tree));
    let date = p.date();
    assert_eq!(date.get_untracked(), Some(day(2024, 1, 5)));
    date.set(Some(day(2024, 1, 9)));
    let v = gui::untracked(|| p.value());
    assert_eq!(
        (v.date(), v.hour, v.minute),
        (Some(day(2024, 1, 9)), Some(9), Some(30)),
        "a pick keeps the time"
    );
    assert!(!open.get_untracked(), "and, closing on select, closes");
    date.set(None);
    let v = gui::untracked(|| p.value());
    assert_eq!((v.date(), v.hour), (None, None), "cleared, the time goes too");
    date.set(Some(day(2024, 1, 2)));
    let v = gui::untracked(|| p.value());
    assert_eq!(
        (v.hour, v.minute),
        (Some(0), Some(0)),
        "the next pick starts at midnight"
    );
    // The trigger ends the field's arrow chain.
    let last = p.field.edge(true).expect("a last segment");
    last.focus();
    p.field.key(Segment::DayPeriod, &plain(NamedKey::ArrowRight));
    assert!(p.trigger_handle().is_focused());
    // Escape closes the popup and gives focus back to the trigger.
    s.act("open", || p.set_open(true));
    s.settle().expect("settled");
    s.press(NamedKey::Escape, Modifiers::default());
    s.settle().expect("settled");
    assert!(!open.get_untracked(), "Escape dismisses");
    assert!(p.trigger_handle().is_focused());
}

type Range = RangeCalendar<Bind<DateRange>>;

#[test]
fn a_range_calendar_and_what_a_screen_reader_is_told() {
    let made = Rc::new(Cell::new(None::<Range>));
    let probe = Rc::clone(&made);
    let s = scenario(move || {
        let probe = Rc::clone(&probe);
        component(
            Bind::default().with_default(DateRange::between(day(2024, 2, 5), day(2024, 2, 7))),
            move |cx| {
                let value = use_model(cx.props(), |b: &Bind<DateRange>| b);
                let r = RangeCalendar::new(value, || options(day(2024, 2, 1)), RangeOptions::default, None, ltr());
                probe.set(Some(r));
                move || {
                    let g = r.grid;
                    let month = g.months()[0];
                    let mut grid = r.grid(div().display(Display::Flex).flex_direction(FlexDirection::Column));
                    for week in g.weeks(month).into_iter().skip(1).take(1) {
                        let mut row = g.row(div().display(Display::Flex));
                        for d in week {
                            let cell = g.day(d, month);
                            row =
                                row.child(r.cell(cell, div()).child(
                                    r.trigger(cell, div().width(30.0).height(30.0).focusable().child(g.number(d))),
                                ));
                        }
                        grid = grid.child(row);
                    }
                    r.root(div().child(grid)).into_element()
                }
            },
        )
        .into_element()
    });
    let r = made.get().expect("the range calendar");
    let picked = |d: u8| gui::untracked(|| r.day(day(2024, 2, d)));
    assert!(picked(5).start && picked(7).end && picked(6).selected && !picked(8).selected);
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("range_calendar_a11y", dump(&tree));
}

#[test]
fn home_and_end_held_to_a_bound_probe_inward() {
    // min is Friday 5 January 2024 and unavailable: Home from Saturday the
    // 6th is held to it, and probes on to the 6th (reka passes the outward
    // sign and turns it inward at the bound).
    let o = CalendarOptions {
        min: Some(day(2024, 1, 5)),
        max: Some(day(2024, 1, 10)),
        is_unavailable: Some(Derive::new(|d: CalendarDate| {
            d == day(2024, 1, 5) || d == day(2024, 1, 10)
        })),
        ..options(day(2024, 1, 6))
    };
    let c = single(None, o.clone(), ltr());
    c.grid.set_anchor(day(2024, 1, 6));
    key_at(c.grid, &plain(NamedKey::Home));
    assert_eq!(anchor(c.grid), day(2024, 1, 6), "held to min, then forward off it");
    // End from Monday the 8th: Saturday the 13th, held to max the 10th,
    // unavailable, then back to the 9th.
    c.grid.set_anchor(day(2024, 1, 8));
    key_at(c.grid, &plain(NamedKey::End));
    assert_eq!(anchor(c.grid), day(2024, 1, 9), "held to max, then back off it");
    // Blocked at the bound alike.
    let o = CalendarOptions {
        is_unavailable: None,
        is_blocked: Some(Derive::new(|d: CalendarDate| d == day(2024, 1, 5))),
        ..o
    };
    let c = single(None, o, ltr());
    c.grid.set_anchor(day(2024, 1, 6));
    key_at(c.grid, &plain(NamedKey::Home));
    assert_eq!(anchor(c.grid), day(2024, 1, 6));
    // Unbounded, Home from Wednesday the 10th on a blocked Sunday 7th looks
    // back toward the day, staying in the week.
    let o = CalendarOptions {
        is_blocked: Some(Derive::new(|d: CalendarDate| d == day(2024, 1, 7))),
        ..options(day(2024, 1, 10))
    };
    let c = single(Some(day(2024, 1, 10)), o, ltr());
    key_at(c.grid, &plain(NamedKey::Home));
    assert_eq!(anchor(c.grid), day(2024, 1, 8));
}

#[test]
fn a_page_function_turns_the_view_and_places_the_anchor_as_reka_tests_it() {
    let years = |n: i32| Derive::new(move |d: CalendarDate| d.add_months(i64::from(n) * 12));
    // The root's function: ten years on from January 1980; the anchor goes
    // to the month's first and the year's first month.
    let o = CalendarOptions {
        next_page: Some(years(10)),
        prev_page: Some(years(-10)),
        ..options(day(1980, 1, 20))
    };
    let c = single(Some(day(1980, 1, 20)), o, ltr());
    c.grid.page(1);
    assert_eq!(gui::untracked(|| c.grid.heading()), "January 1990");
    assert_eq!(anchor(c.grid), day(1990, 1, 1));
    c.grid.page(-1);
    c.grid.page(-1);
    assert_eq!(gui::untracked(|| c.grid.heading()), "January 1970");
    // A button's own function overrides the root's, and the anchor is
    // exactly what it gives.
    let c = single(Some(day(1980, 3, 20)), options(day(1980, 3, 20)), ltr());
    let one = years(1);
    c.grid.page_by(1, Some(&one));
    assert_eq!(gui::untracked(|| c.grid.heading()), "March 1981");
    assert_eq!(anchor(c.grid), day(1981, 3, 20));
    // Next by a year is blocked when a year on, on its first month's first,
    // passes max (`handleNextDisabled`).
    let o = CalendarOptions {
        max: Some(day(1980, 12, 31)),
        ..options(day(1980, 3, 20))
    };
    let c = single(Some(day(1980, 3, 20)), o, ltr());
    assert!(!gui::untracked(|| c.grid.can_page_by(1, Some(&one))));
    assert!(gui::untracked(|| c.grid.can_page(1)), "a month is still allowed");
    let o = CalendarOptions {
        max: Some(day(1981, 1, 1)),
        ..options(day(1980, 3, 20))
    };
    let c = single(Some(day(1980, 3, 20)), o, ltr());
    assert!(
        gui::untracked(|| c.grid.can_page_by(1, Some(&one))),
        "1 January 1981 is not after max"
    );
}

#[test]
fn a_new_locale_or_system_keeps_the_anchor() {
    let locale = Signal::new(CalendarLocale::EnUs);
    let model = use_model(Signal::new(Bind::default().with_default(Some(day(2024, 5, 9)))), |b| b);
    let c: Single = Calendar::new(
        model,
        move || CalendarOptions {
            locale: locale.get(),
            ..options(day(2026, 11, 22))
        },
        None,
        ltr(),
    );
    c.grid.set_anchor(day(2024, 7, 3));
    locale.set(CalendarLocale::Vi);
    assert_eq!(
        anchor(c.grid),
        day(2024, 7, 3),
        "the view is made again from the anchor"
    );
    assert_eq!(gui::untracked(|| c.grid.first_month()), day(2024, 7, 1));
}

fn time_look<P: Clone + PartialEq + 'static>(f: TimeField<P>) -> gui::Div {
    let mut row = f.root(div().display(Display::Flex));
    for piece in f.pieces() {
        row = match piece {
            Piece::Segment(s) => row.child(f.segment(s, div().width(30.0).height(20.0).child(f.text(s)))),
            Piece::Literal(t) => row.child(f.literal(div().child(text(t)))),
        };
    }
    row
}

#[test]
fn a_time_field_and_a_read_only_invalid_date_field_as_a_screen_reader_hears_them() {
    let s = scenario(move || {
        div()
            .child(component(
                Bind::default().with_default(Time::parse("07:05")),
                move |cx| {
                    let value = use_model(cx.props(), |b: &Bind<Time>| b);
                    let f = TimeField::new(
                        value,
                        || TimeFieldOptions {
                            min: Some(Time::parse("09:00")),
                            ..TimeFieldOptions::default()
                        },
                        ltr(),
                    );
                    move || time_look(f).into_element()
                },
            ))
            .child(component(
                Bind::default().with_default(DateEntry::of(
                    CalendarSystem::Gregorian,
                    Precision::Day,
                    day(2024, 1, 2),
                )),
                move |cx| {
                    let value = use_model(cx.props(), |b: &Bind<DateEntry>| b);
                    let f = DateField::new(
                        value,
                        || DateFieldOptions {
                            readonly: true,
                            min: Some(day(2024, 2, 1)),
                            ..en()
                        },
                        ltr(),
                    );
                    move || field_look(f).into_element()
                },
            ))
            .into_element()
    });
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("time_and_invalid_field_a11y", dump(&tree));
}

#[derive(Clone, PartialEq)]
struct RangeProps {
    value: Bind<DateRangeEntry>,
    open: Bind<bool>,
}

type RangePicker = DateRangePicker<RangeProps>;

fn range_entry(a: Option<CalendarDate>, b: Option<CalendarDate>) -> DateRangeEntry {
    let e = |d| DateEntry::empty(CalendarSystem::Gregorian, Precision::Day).with_date(d);
    DateRangeEntry { start: e(a), end: e(b) }
}

/// A range picker in a window over `value`, its popup's calendar a range
/// calendar on the picker's range, `options` the fields', `direction` both.
fn range_picker(
    value: Signal<DateRangeEntry>,
    open: Signal<bool>,
    field_options: Signal<DateFieldOptions>,
    direction: Direction,
) -> (Scenario, RangePicker, Rc<Cell<Option<Range>>>) {
    let made = Rc::new(Cell::new(None::<RangePicker>));
    let probe = Rc::clone(&made);
    let calendar = Rc::new(Cell::new(None::<Range>));
    let inner = Rc::clone(&calendar);
    let s = scenario(move || {
        let probe = Rc::clone(&probe);
        let inner = Rc::clone(&inner);
        component(
            RangeProps {
                value: Bind::model(value),
                open: Bind::model(open),
            },
            move |cx| {
                let value = use_model(cx.props(), |p: &RangeProps| &p.value);
                let opened = use_model(cx.props(), |p: &RangeProps| &p.open);
                let dir = ReadDirection::fixed(direction);
                let picker = DateRangePicker::new(value, move || field_options.get(), opened, || true, dir);
                probe.set(Some(picker));
                let inner = Rc::clone(&inner);
                move || {
                    let mut row = div()
                        .display(Display::Flex)
                        .child(field_look(picker.start))
                        .child(field_look(picker.end))
                        .child(picker.trigger(div().width(20.0).height(20.0)));
                    if picker.is_open() {
                        let inner = Rc::clone(&inner);
                        let calendar = component(Bind::model(picker.range()), move |cx| {
                            let value = use_model(cx.props(), |b: &Bind<DateRange>| b);
                            let r = RangeCalendar::new(
                                value,
                                || options(day(2024, 2, 1)),
                                RangeOptions::default,
                                None,
                                ltr(),
                            );
                            inner.set(Some(r));
                            move || {
                                let g = r.grid;
                                let month = g.months()[0];
                                let mut grid =
                                    r.grid(div().display(Display::Flex).flex_direction(FlexDirection::Column));
                                for week in g.weeks(month) {
                                    let mut w = g.row(div().display(Display::Flex));
                                    for d in week {
                                        let cell = g.day(d, month);
                                        w = w.child(r.cell(cell, div()).child(r.trigger(
                                            cell,
                                            div().width(20.0).height(20.0).focusable().child(g.number(d)),
                                        )));
                                    }
                                    grid = grid.child(w);
                                }
                                r.root(div().child(grid)).into_element()
                            }
                        });
                        row = row.child(picker.content(div().child(calendar)));
                    }
                    row.into_element()
                }
            },
        )
        .into_element()
    });
    let picker = made.get().expect("the picker");
    (s, picker, calendar)
}

#[test]
fn a_range_picker_runs_its_arrows_through_both_fields_to_the_trigger() {
    for direction in [Direction::Ltr, Direction::Rtl] {
        let value = Signal::new(range_entry(Some(day(2024, 2, 5)), Some(day(2024, 2, 9))));
        let options = Signal::new(en());
        let (mut s, p, _) = range_picker(value, Signal::new(false), options, direction);
        let (on, back) = if direction == Direction::Rtl {
            (NamedKey::ArrowLeft, NamedKey::ArrowRight)
        } else {
            (NamedKey::ArrowRight, NamedKey::ArrowLeft)
        };
        s.act("focus", || {
            p.start.focus_first();
        });
        s.settle().expect("settled");
        let mut seen = Vec::new();
        for _ in 0..6 {
            s.press(on, Modifiers::default());
            s.settle().expect("settled");
            seen.push(p.trigger_handle().is_focused());
        }
        assert_eq!(
            seen,
            [false, false, false, false, false, true],
            "{direction:?}: start, end, trigger"
        );
        s.press(back, Modifiers::default());
        s.settle().expect("settled");
        assert_eq!(gui::untracked(|| p.end.edge(true)).map(|h| h.is_focused()), Some(true));
        // A new locale reorders the segments; the chain follows.
        s.act("vi", || {
            options.set(DateFieldOptions {
                locale: CalendarLocale::Vi,
                ..en()
            });
        });
        s.settle().expect("settled");
        s.act("start's last", || {
            gui::untracked(|| p.start.edge(true)).expect("a segment").focus();
        });
        s.press(on, Modifiers::default());
        s.settle().expect("settled");
        let first = gui::untracked(|| p.end.edge(false)).expect("a segment");
        assert!(first.is_focused(), "{direction:?}: onto the end's first, the day in vi");
        assert_eq!(gui::untracked(|| p.end.segments())[0], Segment::Day);
    }
}

#[test]
fn a_range_picker_closes_once_whole_and_reports_its_value_and_validity() {
    let value = Signal::new(range_entry(None, None));
    let open = Signal::new(true);
    let unavailable = day(2024, 2, 14);
    let options = Signal::new(DateFieldOptions {
        min: Some(day(2024, 2, 2)),
        max: Some(day(2024, 2, 20)),
        is_unavailable: Some(Derive::new(move |d| d == unavailable)),
        ..en()
    });
    let (mut s, p, _) = range_picker(value, open, options, Direction::Ltr);
    let range = p.range();
    s.act("start", || range.set(DateRange::started(day(2024, 2, 5))));
    s.settle().expect("settled");
    assert!(open.get_untracked(), "a half range keeps it open");
    assert_eq!(
        gui::untracked(|| p.value()).value(),
        "",
        "nothing sent while an end is missing"
    );
    s.act("end", || {
        range.set(DateRange::between(day(2024, 2, 5), day(2024, 2, 9)))
    });
    s.settle().expect("settled");
    assert!(!open.get_untracked(), "whole, it closes");
    assert_eq!(gui::untracked(|| p.value()).value(), "2024-02-05 - 2024-02-09");
    assert_eq!(
        gui::untracked(|| p.value()).form_data(),
        FormData::Text("2024-02-05 - 2024-02-09".to_owned())
    );
    assert!(!gui::untracked(|| p.invalid()));
    // A caller writing a whole range closes it too.
    s.act("open", || p.set_open(true));
    s.act("caller", || {
        value.set(range_entry(Some(day(2024, 2, 3)), Some(day(2024, 2, 4))))
    });
    s.settle().expect("settled");
    assert!(!open.get_untracked(), "the caller's whole range closes it");
    // Invalid, by the fields' own options.
    for (a, b, why) in [
        (day(2024, 2, 5), day(2024, 2, 21), "the end after max"),
        (day(2024, 2, 9), day(2024, 2, 5), "the start after the end"),
        (day(2024, 2, 14), day(2024, 2, 15), "an unavailable end"),
        (day(2024, 2, 10), day(2024, 2, 18), "an unavailable day between"),
    ] {
        s.act("write", || value.set(range_entry(Some(a), Some(b))));
        assert!(gui::untracked(|| p.invalid()), "{why}");
        assert!(gui::untracked(|| p.end.invalid()), "{why}: both fields read it");
    }
    s.settle().expect("settled");
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("date_range_picker_a11y", dump(&tree));
}

#[test]
fn escape_while_a_range_is_half_made_puts_it_back_and_closes_the_popup() {
    let value = Signal::new(range_entry(Some(day(2024, 2, 5)), Some(day(2024, 2, 7))));
    let open = Signal::new(true);
    let (mut s, p, calendar) = range_picker(value, open, Signal::new(en()), Direction::Ltr);
    let r = calendar.get().expect("the range calendar");
    s.act("half", || {
        r.press(day(2024, 2, 12));
        r.grid.focus_day(day(2024, 2, 12), true);
    });
    s.settle().expect("settled");
    assert!(gui::untracked(|| r.editing()));
    assert!(open.get_untracked());
    s.press(NamedKey::Escape, Modifiers::default());
    s.settle().expect("settled");
    assert_eq!(
        value.get_untracked().range(),
        DateRange::between(day(2024, 2, 5), day(2024, 2, 7)),
        "the last whole range is back"
    );
    assert!(!open.get_untracked(), "and the popup closed");
    assert!(p.trigger_handle().is_focused(), "focus back on the trigger");
}
