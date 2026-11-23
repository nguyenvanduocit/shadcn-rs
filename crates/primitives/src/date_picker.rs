//! A date field with a calendar in a popup (reka's `DatePicker/`), and its
//! range twin (`DateRangePicker/`). Neither holds a rule of its own for
//! typing, the grid or the popup: they join a [`DateField`] (or two), a
//! calendar and an engine layer (ADR-0024) over one value, and add what
//! reka's pickers add (research §11):
//!
//! - One value for the field and the calendar. The calendar's day is the
//!   entry's date ([`DatePicker::date`]); picking one keeps the time the
//!   entry has, and an empty time starts at midnight; the calendar clearing
//!   the day clears the time too, so the next pick starts at midnight, while
//!   a time typed after that is kept (`DatePickerRoot.vue:147-165`).
//! - Closed at first, not modal; Escape, a press outside and focus outside
//!   close it, and closing gives focus back to the trigger (the layer's).
//! - `close_on_select` (off by default): a pick closes it only when the day
//!   changes; a range closes once both ends are set, whoever set them
//!   (`DatePickerRoot.vue:166-170`; `DateRangePickerRoot.vue:162-166`).
//! - The trigger ends the field's arrow chain: ArrowRight from the last
//!   segment reaches it, ArrowLeft from it the last segment (swapped right
//!   to left); Tab goes through every segment, then to it.
//! - Opening gives the calendar focus, on the picked day, else today, else
//!   the first day not blocked: the calendar inside does it as it mounts
//!   ([`crate::calendar::Calendar::focus_initial`]); the layer keeps focus
//!   where that puts it.
//! - Disabled: it neither opens nor edits.
//! - The field is told only `min`, `max` and the unavailable days; the
//!   days the calendar blocks can still be typed (as reka's).
//! - The fields own what they are told (availability, `min`, `max`, the
//!   unavailable days): the trigger reads the field's availability, and the
//!   range picker's two fields share one set of options, so nothing is told
//!   twice.
//! - A range's two fields are made by the picker over its one value
//!   ([`DateField::range`]): invalid with an end unavailable or outside
//!   `min..=max`, the start after the end, or an unavailable day between;
//!   it sends "`<start>` - `<end>`", empty while an end is missing, so
//!   `required` reports it (`DateRangeFieldRoot.vue`).
//! - Escape while a range is half made puts back the last whole range and
//!   also reaches the popup, which closes.

use std::fmt;

use gui::events::{KeyEvent, NamedKey};
use gui::focus::{AutoFocus, FocusHandle, FocusScope};
use gui::layer::{Dismiss, DismissPolicy, Layer};
use gui::{Computed, Direction, Div, Model, Role, Signal, UseModel, watch};

use crate::availability::Availability;
use crate::date::CalendarDate;
use crate::date_field::{DateEntry, DateField, DateFieldOptions, DateRangeEntry};
use crate::direction::ReadDirection;
use crate::range_calendar::DateRange;

/// The open state, the trigger and the layer both pickers share.
struct Popup<P: 'static> {
    open: UseModel<P, bool>,
    trigger: FocusHandle,
    /// The field's (it owns it).
    availability: Computed<Availability>,
    direction: ReadDirection,
}

impl<P: 'static> Clone for Popup<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Popup<P> {}

impl<P: 'static> Popup<P> {
    fn set_open(self, open: bool) {
        if open && gui::untracked(|| self.availability.get()) != Availability::Enabled {
            return;
        }
        self.open.set(open);
    }

    fn trigger(self, d: Div, label: String, last: impl Fn() -> Option<FocusHandle> + 'static) -> Div {
        let open = self.open.get();
        let usable = self.availability.get();
        usable
            .apply(
                d.role(Role::Button)
                    .aria_label(label)
                    .aria_expanded(open)
                    .track_focus(self.trigger),
            )
            .on_click(move |_| self.set_open(!self.open.get_untracked()))
            .on_key_down(move |ev: &KeyEvent| {
                let back = if self.direction.get() == Direction::Rtl {
                    NamedKey::ArrowRight
                } else {
                    NamedKey::ArrowLeft
                };
                if ev.is(back) {
                    return last().map(FocusHandle::focus).is_some();
                }
                false
            })
    }

    fn content(self, d: Div) -> Div {
        let open = self.open;
        d.layer(
            Layer::new()
                .dismiss(DismissPolicy::DEFAULT)
                .on_dismiss(move |_| {
                    open.set(false);
                    Dismiss::Close
                })
                .focus_scope(FocusScope::new().auto_focus(AutoFocus::Keep).return_to(self.trigger)),
        )
    }
}

/// A date picker's state: the entry its field types, the day its calendar
/// shows, the open state, and the trigger. Made once in setup, `Copy`.
pub struct DatePicker<P: 'static> {
    pub field: DateField<P>,
    value: UseModel<P, DateEntry>,
    date: Signal<Option<CalendarDate>>,
    popup: Popup<P>,
}

impl<P: 'static> Clone for DatePicker<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for DatePicker<P> {}

impl<P: 'static> fmt::Debug for DatePicker<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatePicker").finish_non_exhaustive()
    }
}

impl<P: 'static> DatePicker<P> {
    /// A picker over `field` (made on `value`, the part's `v-model`) and
    /// `open` (its open state), `close_on_select` read at each pick, called
    /// in setup. A picker whose field cannot be used does not open.
    #[must_use]
    #[track_caller]
    pub fn new(
        field: DateField<P>,
        value: UseModel<P, DateEntry>,
        open: UseModel<P, bool>,
        close_on_select: impl Fn() -> bool + 'static,
        direction: ReadDirection,
    ) -> Self {
        let date = Signal::labeled("picker date", gui::untracked(|| value.with(|e| e.date())));
        let popup = Popup {
            open,
            trigger: FocusHandle::new(),
            availability: Computed::new(move || field.availability()),
            direction,
        };
        field.set_after(Some(popup.trigger));
        let picker = Self {
            field,
            value,
            date,
            popup,
        };
        // The entry's day is the calendar's.
        watch(move || value.with(|e| e.date()), move |d, _| date.set(*d));
        // The calendar's day goes into the entry, keeping its time.
        watch(
            move || date.get(),
            move |d, _| {
                let entry = value.get_untracked();
                if entry.date() == *d {
                    return;
                }
                let next = match d {
                    Some(_) => {
                        let e = entry.with_date(*d);
                        e.with_time(e.hour.or(Some(0)), e.minute.or(Some(0)), e.second.or(Some(0)))
                    }
                    None => entry.with_date(None).with_time(None, None, None),
                };
                value.set(next);
                if d.is_some() && close_on_select() {
                    popup.set_open(false);
                }
            },
        );
        picker
    }

    /// The entry (tracked).
    #[must_use]
    pub fn value(self) -> DateEntry {
        self.value.get()
    }

    /// Whether the popup is open (tracked).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.popup.open.get()
    }

    /// Open or close it; a picker that cannot be used does not open.
    pub fn set_open(self, open: bool) {
        self.popup.set_open(open);
    }

    /// The calendar's value: the entry's day, both ways. Give it to the
    /// calendar inside the popup (`.model(picker.date())`).
    #[must_use]
    pub fn date(self) -> Model<Option<CalendarDate>> {
        Model::bind(self.date)
    }

    /// The trigger's focus handle.
    #[must_use]
    pub fn trigger_handle(self) -> FocusHandle {
        self.popup.trigger
    }

    /// `d` as the trigger: a button telling whether the popup is open, that
    /// toggles it; ArrowLeft goes back to the field's last segment.
    #[must_use]
    pub fn trigger(self, d: Div) -> Div {
        let field = self.field;
        self.popup
            .trigger(d, field.strings().show_picker, move || field.edge(true))
    }

    /// `d` as the popup's content: an engine layer that Escape, a press or
    /// focus outside close, giving focus back to the trigger.
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        self.popup.content(d)
    }
}

/// A date range picker's state: two fields over one entry, the range its
/// calendar shows, the open state, and the trigger. Made once in setup,
/// `Copy`.
pub struct DateRangePicker<P: 'static> {
    pub start: DateField<P>,
    pub end: DateField<P>,
    value: UseModel<P, DateRangeEntry>,
    range: Signal<DateRange>,
    popup: Popup<P>,
}

impl<P: 'static> Clone for DateRangePicker<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for DateRangePicker<P> {}

impl<P: 'static> fmt::Debug for DateRangePicker<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DateRangePicker").finish_non_exhaustive()
    }
}

impl<P: 'static> DateRangePicker<P> {
    /// A range picker over `value` (the part's `v-model`), its two fields
    /// made here with `options` ([`DateField::range`]), and `open`, called
    /// in setup. The arrows run through the start's segments, the end's,
    /// then the trigger (swapped right to left), in the locale's order at
    /// the time.
    #[must_use]
    #[track_caller]
    pub fn new(
        value: UseModel<P, DateRangeEntry>,
        options: impl Fn() -> DateFieldOptions + 'static,
        open: UseModel<P, bool>,
        close_on_select: impl Fn() -> bool + 'static,
        direction: ReadDirection,
    ) -> Self {
        let (start, end) = DateField::range(value, options, direction);
        let range = Signal::labeled("picker range", gui::untracked(|| value.with(|e| e.range())));
        let popup = Popup {
            open,
            trigger: FocusHandle::new(),
            availability: Computed::new(move || start.availability()),
            direction,
        };
        end.set_after(Some(popup.trigger));
        let picker = Self {
            start,
            end,
            value,
            range,
            popup,
        };
        watch(move || value.with(|e| e.range()), move |r, _| range.set(*r));
        watch(
            move || range.get(),
            move |r, _| {
                let entry = value.get_untracked();
                if entry.range() != *r {
                    value.set(DateRangeEntry {
                        start: entry.start.with_date(r.start),
                        end: entry.end.with_date(r.end),
                    });
                }
                if r.is_complete() && close_on_select() {
                    popup.set_open(false);
                }
            },
        );
        picker
    }

    /// The entry (tracked).
    #[must_use]
    pub fn value(self) -> DateRangeEntry {
        self.value.get()
    }

    /// Whether the range is invalid (tracked), by the fields' options
    /// ([`DateFieldOptions::range_invalid`]).
    #[must_use]
    pub fn invalid(self) -> bool {
        self.start.invalid()
    }

    /// Whether the popup is open (tracked).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.popup.open.get()
    }

    /// Open or close it.
    pub fn set_open(self, open: bool) {
        self.popup.set_open(open);
    }

    /// The trigger's focus handle.
    #[must_use]
    pub fn trigger_handle(self) -> FocusHandle {
        self.popup.trigger
    }

    /// The range calendar's value, both ways (`.model(picker.range())`).
    #[must_use]
    pub fn range(self) -> Model<DateRange> {
        Model::bind(self.range)
    }

    /// `d` as the trigger (see [`DatePicker::trigger`]); ArrowLeft goes
    /// back to the end field's last segment.
    #[must_use]
    pub fn trigger(self, d: Div) -> Div {
        let end = self.end;
        self.popup.trigger(d, end.strings().show_picker, move || end.edge(true))
    }

    /// `d` as the popup's content (see [`DatePicker::content`]).
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        self.popup.content(d)
    }
}
