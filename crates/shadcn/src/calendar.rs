//! Calendar (`ui/calendar`): a month of days to pick one date from, paged
//! with the header's arrows. Its behaviour (the view, the anchor and the
//! grid's one Tab stop, the keys, picking, what assistive technology is
//! told) is `primitives::calendar`'s; this file draws it: the header, the
//! day cells and triggers, the arrows and the month and year selects. The
//! look and the dates are shared with `range_calendar`.
//!
//! [`calendar`] is an instance keeping its view and, unless the caller
//! passes `.model(…)` (reka's `v-model`), the picked day, starting at
//! `default_value`. Each day is an instance of its own, so a pick renders
//! the two days it changes.

use gui::prelude::*;
use gui::{
    Bind, Computed, Derive, Direction, Div, Element, FocusHandle, Icon, Length, Model, Signal, component, div, icon,
    on_mounted, provide, text,
};
use primitives::Availability;
use primitives::calendar::{Calendar, CalendarGrid};
pub use primitives::calendar::{CalendarOptions, Day};
use primitives::context::use_root;
pub use primitives::date::{
    CalendarDate, CalendarLocale, CalendarSystem, DatePart, DateStrings, Era, MonthWidth, SystemDate, WeekdayFormat,
};
use primitives::direction::use_direction;

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::focus::StateLook;
use crate::native_select::{NativeSelectEntry, native_select, native_select_option};
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What only the look of both calendars reads: the day triggers' size and
/// the direction the rows run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalendarLook {
    /// A day trigger's side, in pixels (`size-8`).
    pub cell_size: f32,
    /// `dir="rtl"`: rows run right to left, the arrows and arrow keys mirrored.
    pub rtl: bool,
}

impl Default for CalendarLook {
    fn default() -> Self {
        Self {
            cell_size: 32.0,
            rtl: false,
        }
    }
}

/// The props of a calendar ([`calendar`], [`crate::range_calendar`]): what
/// their shared builder methods set.
pub trait HasOptions {
    fn options_mut(&mut self) -> &mut CalendarOptions;
    fn look_mut(&mut self) -> &mut CalendarLook;
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

/// A row laid out in the calendar's direction.
pub(crate) fn row(rtl: bool) -> Div {
    if rtl {
        div().display(Display::Flex).flex_direction(FlexDirection::RowReverse)
    } else {
        div().display(Display::Flex).flex_direction(FlexDirection::Row)
    }
}

/// The months' grids side by side.
pub(crate) fn months_row(grid: CalendarGrid, rtl: bool, month: impl Fn(CalendarDate) -> Div) -> Div {
    row(rtl)
        .margin_top(16.0)
        .column_gap(16.0)
        .children(grid.months().into_iter().map(month))
}

/// The look of the day button both calendars put in a cell
/// (`CalendarCellTrigger`); the primitive's trigger part adds what it does.
pub(crate) fn day_trigger(grid: CalendarGrid, look: CalendarLook, day: Day, selection: DaySelection) -> Div {
    let t = theme();
    let cell = button(ButtonVariant::Ghost, ButtonSize::IconSm);
    let i = cell.interaction();
    let lit = if t.dark { t.accent.opacity(0.5) } else { t.accent };
    cell.named("calendar-cell-trigger")
        .size(look.cell_size)
        .padding(0.0)
        .font_weight(400)
        .cursor(Cursor::Default)
        .when(day.today && selection == DaySelection::Unselected, |b| {
            b.background(t.accent).text_color(t.accent_foreground)
        })
        .when(day.outside_month, |b| b.text_color(t.muted_foreground))
        // A picked day keeps its fill under the pointer; any other takes
        // the ghost button's hover over what it shows.
        .when(selection == DaySelection::Picked, |b| {
            b.background(t.primary).text_color(t.primary_foreground)
        })
        .when(selection != DaySelection::Picked, |b| {
            b.background_while(move || i.hovered(), lit)
                .text_color_while(move || i.hovered(), t.accent_foreground)
        })
        .when(day.blocked, |b| b.text_color(t.muted_foreground))
        .child(grid.number(day.date))
        .into_div()
}

/// The look of the prev and next arrows both calendars use: an outline
/// icon button, see-through until hovered, its chevron pointing the way it
/// pages; the primitive's part adds what it does.
pub(crate) fn page_button(grid: CalendarGrid, rtl: bool, step: i64) -> Div {
    let t = theme();
    let back = (step < 0) != rtl;
    let arrow = button(ButtonVariant::Outline, ButtonSize::Default);
    let i = arrow.interaction();
    let d = arrow
        .named(if step < 0 {
            "calendar-prev-button"
        } else {
            "calendar-next-button"
        })
        .size(28.0)
        .padding(0.0)
        // Over the outline button's own hover (its fill in dark, its text).
        .when(!t.dark, |b| {
            b.background(gui::Color::TRANSPARENT)
                .background_while(move || i.hovered(), t.accent)
        })
        .opacity(move || if i.hovered() { 1.0 } else { 0.5 })
        .child(icon(if back { Icon::ChevronLeft } else { Icon::ChevronRight }))
        .into_div();
    grid.page_button(step, d)
}

/// One month's grid: weekday names over its weeks; `cell` gives each day's
/// cell given its column.
pub(crate) fn month_grid(
    grid: CalendarGrid,
    rtl: bool,
    month: CalendarDate,
    head_cell: impl Fn(&'static str) -> Div,
    cell: impl Fn(CalendarDate, usize) -> Element,
) -> Div {
    let names = grid.weekdays();
    grid.grid(
        div()
            .named("calendar-grid")
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .width(pct(100.0)),
    )
    .child(grid.head(row(rtl).named("calendar-grid-row").children(names.map(&head_cell))))
    .children(grid.weeks(month).into_iter().map(|week| {
        grid.row(row(rtl).named("calendar-grid-row").margin_top(8.0).width(pct(100.0)))
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
    look: CalendarLook,
    layout: CalendarLayout,
    /// `initialFocus`: the picked day (else today) takes focus as the calendar mounts.
    initial_focus: bool,
    /// The `#calendar-heading` slot, in place of the layout's.
    heading: Option<Element>,
}

impl HasOptions for CalendarProps {
    fn options_mut(&mut self) -> &mut CalendarOptions {
        &mut self.options
    }

    fn look_mut(&mut self) -> &mut CalendarLook {
        &mut self.look
    }
}

type Picker = Calendar<PartProps<CalendarProps>, Option<CalendarDate>>;

/// What a calendar's parts read.
#[derive(Clone)]
struct CalendarContext {
    calendar: Picker,
    rtl: bool,
    look: Computed<CalendarLook>,
}

/// A calendar: its `layout`'s heading between paging arrows, then the month
/// grids. Style it as a box (`rounded-md border`).
#[must_use]
pub fn calendar() -> Part<CalendarProps> {
    let props = CalendarProps {
        value: Bind::default(),
        options: CalendarOptions::default(),
        look: CalendarLook::default(),
        layout: CalendarLayout::Heading,
        heading: None,
        initial_focus: false,
    };
    Part::new(props, div().named("calendar").padding(12.0), |p| {
        component(p, |cx| {
            let props = cx.props();
            let value = gui::use_model(props, |p| &p.props.value);
            let rtl = props.with_untracked(|p| p.props.look.rtl);
            let direction = use_direction(Some(if rtl { Direction::Rtl } else { Direction::Ltr }));
            let calendar = Calendar::new(value, move || props.with(|p| p.props.options.clone()), None, direction);
            if props.with_untracked(|p| p.props.initial_focus) {
                // As reka-ui's `initialFocus`: once laid out, the picked day
                // (else today) takes focus, with no ring.
                on_mounted(move || calendar.focus_initial());
            }
            let look = Computed::new(move || props.with(|p| p.props.look));
            provide(CalendarContext { calendar, rtl, look });
            move || calendar_render(props, calendar, rtl)
        })
    })
}

fn calendar_render(props: Signal<PartProps<CalendarProps>>, calendar: Picker, rtl: bool) -> Element {
    let grid = calendar.grid;
    let (root, layout, heading) = props.with(|p| (p.slot.0.clone(), p.props.layout, p.props.heading.clone()));
    let heading = heading.unwrap_or_else(|| match layout {
        CalendarLayout::Heading => calendar_heading(grid.heading()).into_element(),
        CalendarLayout::MonthAndYear => row(rtl)
            .align_items(Align::Center)
            .justify_content(Justify::Center)
            .gap(4.0)
            .child(calendar_month_select())
            .child(calendar_year_select())
            .into_element(),
        CalendarLayout::MonthOnly => {
            let o = grid.options();
            let year = o.locale.year(o.system, grid.first_month());
            row(rtl)
                .align_items(Align::Center)
                .justify_content(Justify::Center)
                .gap(4.0)
                .child(calendar_month_select())
                .child(text(year))
                .into_element()
        }
        CalendarLayout::YearOnly => {
            let o = grid.options();
            let month = o.locale.month(o.system, grid.first_month(), MonthWidth::Short);
            row(rtl)
                .align_items(Align::Center)
                .justify_content(Justify::Center)
                .gap(4.0)
                .child(text(month))
                .child(calendar_year_select())
                .into_element()
        }
    });
    grid.root(
        root.child(
            calendar_header().padding_top(0.0).child(heading).child(
                row(rtl)
                    .align_items(Align::Center)
                    .gap(4.0)
                    .position(Position::Absolute)
                    .top(0.0)
                    .left(0.0)
                    .right(0.0)
                    .justify_content(Justify::Between)
                    .pass_through()
                    .child(page_button(grid, rtl, -1))
                    .child(page_button(grid, rtl, 1)),
            ),
        )
        .child(months_row(grid, rtl, |month| {
            month_grid(grid, rtl, month, calendar_head_cell, |day, _| calendar_day(day, month))
        })),
    )
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

    /// `pagedNavigation`: the arrows turn every month shown.
    #[must_use]
    pub fn paged_navigation(self) -> Self {
        self.with_props(|p| p.options_mut().paged = true)
    }

    /// `fixedWeeks`: always six weeks.
    #[must_use]
    pub fn fixed_weeks(self) -> Self {
        self.with_props(|p| p.options_mut().fixed_weeks = true)
    }

    /// `weekStartsOn`: the weekday a week starts on, 0 for Sunday.
    #[must_use]
    pub fn week_starts_on(self, weekday: u8) -> Self {
        self.with_props(|p| p.options_mut().first_weekday = Some(weekday))
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

    /// `isDateDisabled`: days that cannot be focused or picked.
    #[must_use]
    pub fn is_date_disabled(self, f: impl Into<Derive<CalendarDate, bool>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.options_mut().is_blocked = Some(f))
    }

    /// `isDateUnavailable`: days that take focus but cannot be picked.
    #[must_use]
    pub fn is_date_unavailable(self, f: impl Into<Derive<CalendarDate, bool>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.options_mut().is_unavailable = Some(f))
    }

    /// `disabled`: no day takes focus or a pick, the arrows are disabled.
    #[must_use]
    pub fn disabled(self) -> Self {
        self.with_props(|p| p.options_mut().availability = Availability::Disabled)
    }

    /// `readonly`: focus and keys as usual, nothing picked.
    #[must_use]
    pub fn readonly(self) -> Self {
        self.with_props(|p| p.options_mut().readonly = true)
    }

    /// `preventDeselect`: picking the picked day keeps it.
    #[must_use]
    pub fn prevent_deselect(self) -> Self {
        self.with_props(|p| p.options_mut().prevent_deselect = true)
    }

    /// `weekdayFormat`: how the weekdays over the columns are named.
    #[must_use]
    pub fn weekday_format(self, format: WeekdayFormat) -> Self {
        self.with_props(|p| p.options_mut().weekday_format = format)
    }

    /// A day trigger's side in pixels (`**:data-[slot=calendar-cell-trigger]:size-12!`).
    #[must_use]
    pub fn cell_size(self, size: f32) -> Self {
        self.with_props(|p| p.look_mut().cell_size = size)
    }

    /// `disableDaysOutsideCurrentView`: the padding days of other months are disabled.
    #[must_use]
    pub fn disable_days_outside_current_view(self) -> Self {
        self.with_props(|p| p.options_mut().disable_outside_view = true)
    }

    /// `dir="rtl"`: laid out right to left, ArrowLeft and ArrowRight swapped.
    #[must_use]
    pub fn rtl(self) -> Self {
        self.with_props(|p| p.look_mut().rtl = true)
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

    /// What assistive technology is told, in place of the locale's strings
    /// (`calendarLabel` among them).
    #[must_use]
    pub fn strings(self, strings: DateStrings) -> Self {
        self.with_props(|p| p.options_mut().strings = Some(strings))
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
    pub fn on_change(self, f: impl Into<gui::Callback<Option<CalendarDate>>>) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).on_change(f))
    }

    /// `initialFocus`: as it mounts, the picked day (else today) takes
    /// focus, without a ring. For a calendar in a popover.
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
        let ctx = use_root::<CalendarContext>("calendar day", "calendar");
        move || {
            let (date, month) = props.get();
            let Some(ctx) = &ctx else {
                return Element::Empty;
            };
            let calendar = ctx.calendar;
            let look = ctx.look.get();
            let day = calendar.grid.day(date, month);
            let selected = calendar.has(date);
            let selection = if selected {
                DaySelection::Picked
            } else {
                DaySelection::Unselected
            };
            calendar
                .cell(day, calendar_cell(selected))
                .child(calendar.trigger(day, day_trigger(calendar.grid, look, day, selection)))
                .into_element()
        }
    })
    .into_element()
}

#[must_use]
pub fn calendar_header() -> Div {
    div()
        .named("calendar-header")
        .display(Display::Flex)
        .justify_content(Justify::Center)
        .padding_top(4.0)
        .position(Position::Relative)
        .align_items(Align::Center)
        .width(pct(100.0))
        .padding_x(32.0)
}

#[must_use]
pub fn calendar_heading(heading: impl Into<String>) -> impl IntoElement {
    text(heading)
        .named("calendar-heading")
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500)
}

/// The shown month's select, its months by short name: a part of the
/// calendar it is in.
#[must_use]
pub fn calendar_month_select() -> Element {
    component((), |_| {
        let ctx = use_root::<CalendarContext>("calendar select", "calendar");
        let focus = FocusHandle::new();
        move || {
            let Some(ctx) = &ctx else {
                return Element::Empty;
            };
            let grid = ctx.calendar.grid;
            let o = grid.options();
            let (system, locale) = (o.system, o.locale);
            let shown = grid.first_month();
            let year = system.fields(shown).year;
            let months: Vec<NativeSelectEntry> = (1..=system.months_in_year(year))
                .map(|m| {
                    let name = locale.month(system, system.date(year, m, 1), MonthWidth::Short);
                    native_select_option(&m.to_string(), name)
                })
                .collect();
            heading_select(
                ctx.rtl,
                locale.month(system, shown, MonthWidth::Short),
                focus,
                &system.fields(shown).month.to_string(),
                &months,
                move |v| {
                    if let Ok(m) = v.parse::<u8>() {
                        grid.set_month(m);
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
        let ctx = use_root::<CalendarContext>("calendar select", "calendar");
        let focus = FocusHandle::new();
        move || {
            let Some(ctx) = &ctx else {
                return Element::Empty;
            };
            let grid = ctx.calendar.grid;
            let o = grid.options();
            let (system, locale) = (o.system, o.locale);
            let shown = grid.first_month();
            let year_of = |d: CalendarDate| system.fields(d).year;
            let first = o.min.map_or(year_of(o.today).saturating_sub(100), year_of);
            let last = o.max.map_or(year_of(o.today).saturating_add(10), year_of);
            let years: Vec<NativeSelectEntry> = (first..=last)
                .map(|y| native_select_option(&y.to_string(), &locale.year(system, system.with_year(shown, y))))
                .collect();
            heading_select(
                ctx.rtl,
                &locale.year(system, shown),
                focus,
                &year_of(shown).to_string(),
                &years,
                move |v| {
                    if let Ok(y) = v.parse::<i32>() {
                        grid.set_year(y);
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
    rtl: bool,
    shown: &str,
    focus: FocusHandle,
    value: &str,
    entries: &[NativeSelectEntry],
    on_change: impl Fn(String) + 'static,
) -> Div {
    div()
        .position(Position::Relative)
        .child(
            div()
                .position(Position::Absolute)
                .inset(0.0)
                .display(Display::Flex)
                .align_items(Align::Center)
                .when(rtl, |d| d.justify_content(Justify::End))
                .text_size(14.0)
                .line_height(20.0)
                .padding_left(8.0)
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
                        .text_size(12.0)
                        .line_height(16.0)
                        .height(32.0)
                        .padding_right(24.0)
                        .padding_left(8.0)
                        .text_color(gui::Color::TRANSPARENT);
                    let chevron = if rtl {
                        chevron.right(Length::Auto).left(14.0)
                    } else {
                        chevron.right(4.0)
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
        .flex(Flex::grow(1.0))
        .display(Display::Flex)
        .justify_content(Justify::Center)
        .radius(8.0)
        .font_weight(400)
        .text_size(12.8)
        .line_height(19.2)
        .text_color(t.muted_foreground)
        .child(name)
}

/// A day's cell; a selected day's cell takes the accent behind its trigger.
#[must_use]
pub fn calendar_cell(selected: bool) -> Div {
    let t = theme();
    div()
        .named("calendar-cell")
        .position(Position::Relative)
        .padding(0.0)
        .display(Display::Flex)
        .justify_content(Justify::Center)
        .text_size(14.0)
        .line_height(20.0)
        .flex(Flex::grow(1.0))
        .when(selected, |d| d.radius(8.0).background(t.accent))
}
