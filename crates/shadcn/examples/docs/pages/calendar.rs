use gui::prelude::*;
use gui::{
    Effect, Icon, Modifiers, NamedKey, ScenarioError, Side, SideAlign, Signal, TextField, div, icon, span, text,
    text_bound,
};
use shadcn::focus::StateLook;
use shadcn::{
    ButtonSize, ButtonVariant, CalendarDate, CalendarLayout, CalendarLocale, CalendarProps, CalendarSystem, MonthWidth,
    Part, PopoverProps, SelectSize, WeekdayFormat, button, calendar, calendar_month_select, input, input_time, label,
    popover, popover_content, popover_trigger, select, select_content, select_group, select_item, select_label,
    select_separator, select_value, theme,
};

use crate::{Render, Shot, demo, page, section};

/// The docs' fixed today (`verdict::CLOCK_MS`), so every run draws the same
/// month; the verdict pins Chrome's clock to the same instant.
fn today() -> CalendarDate {
    crate::verdict::today()
}

/// `toLocaleDateString()` in `en-US`: "11/20/2026".
fn numeric(d: CalendarDate) -> String {
    format!("{}/{}/{}", d.month(), d.day(), d.year())
}

/// `dateStyle: 'long'` in `en-US`: "November 20, 2026".
fn long(d: CalendarDate) -> String {
    format!("{} {}, {}", d.month_name(), d.day(), d.year())
}

/// `CalendarNaturalLanguagePicker`'s `formatDate`: "November 05, 2026".
fn two_digit_day(d: CalendarDate) -> String {
    format!("{} {:02}, {}", d.month_name(), d.day(), d.year())
}

/// The few phrases of `chrono-node`'s `parseDate` the natural language demo
/// is typed with: "today", "tomorrow", "yesterday", "in 2 days", "3 weeks
/// from now", "next week|month|year", "next friday" and the dates its own
/// `formatDate` writes ("November 22, 2026", "Nov 22").
fn parse_date(input: &str, today: CalendarDate) -> Option<CalendarDate> {
    let lower = input.trim().to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let by = |n: i64, unit: &str| -> Option<CalendarDate> {
        match unit.trim_end_matches('s') {
            "day" => Some(today.add_days(n)),
            "week" => Some(today.add_days(n.saturating_mul(7))),
            "month" => Some(today.add_months(n)),
            "year" => Some(today.add_months(n.saturating_mul(12))),
            _ => None,
        }
    };
    let count = |w: &str| match w {
        "a" | "an" | "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        _ => w.parse::<i64>().ok(),
    };
    match words.as_slice() {
        ["today" | "now"] => Some(today),
        ["tomorrow"] => Some(today.add_days(1)),
        ["yesterday"] => Some(today.add_days(-1)),
        ["in", n, unit] | [n, unit, "from", "now"] => by(count(n)?, unit),
        [n, unit, "ago"] => by(count(n)?.saturating_neg(), unit),
        ["next", unit] => by(1, unit).or_else(|| {
            let weekday = weekday_named(unit)?;
            let ahead = i64::from(weekday)
                .saturating_sub(i64::from(today.weekday()))
                .rem_euclid(7);
            Some(today.add_days(if ahead == 0 { 7 } else { ahead }))
        }),
        ["last", unit] => by(-1, unit),
        [month, day] | [month, day, _] => {
            let month = (1..=12_u8).find(|&m| {
                CalendarDate::new(2000, m, 1).is_some_and(|d| {
                    d.month_name()
                        .to_lowercase()
                        .starts_with(month.get(..3).unwrap_or(month))
                })
            })?;
            let day = day.trim_end_matches(',').parse().ok()?;
            let year = match words.get(2) {
                Some(y) => y.parse().ok()?,
                None => today.year(),
            };
            CalendarDate::new(year, month, day)
        }
        _ => None,
    }
}

fn weekday_named(word: &str) -> Option<u8> {
    ["sun", "mon", "tue", "wed", "thu", "fri", "sat"]
        .iter()
        .position(|w| word.starts_with(w))
        .and_then(|i| u8::try_from(i).ok())
}

/// `CalendarSystems.vue`'s locales: label, tag, the calendars it prefers first.
const LOCALES: [(&str, &str, &str); 13] = [
    ("Default", "en-US", "gregory"),
    (
        "Arabic (Algeria)",
        "ar-DZ",
        "gregory islamic islamic-civil islamic-tbla",
    ),
    (
        "Arabic (United Arab Emirates)",
        "ar-AE",
        "gregory islamic-umalqura islamic islamic-civil islamic-tbla",
    ),
    (
        "Arabic (Egypt)",
        "AR-EG",
        "gregory coptic islamic islamic-civil islamic-tbla",
    ),
    (
        "Arabic (Saudi Arabia)",
        "ar-SA",
        "islamic-umalqura gregory islamic islamic-rgsa",
    ),
    (
        "Farsi (Iran)",
        "fa-IR",
        "persian gregory islamic islamic-civil islamic-tbla",
    ),
    (
        "Farsi (Afghanistan)",
        "fa-AF",
        "persian gregory islamic islamic-civil islamic-tbla",
    ),
    ("Amharic (Ethiopia)", "am-ET", "gregory ethiopic ethioaa"),
    (
        "Hebrew (Israel)",
        "he-IL",
        "gregory hebrew islamic islamic-civil islamic-tbla",
    ),
    ("Hindi (India)", "hi-IN", "gregory indian"),
    ("Japanese (Japan)", "ja-JP", "gregory japanese"),
    ("Thai (Thailand)", "th-TH", "buddhist gregory"),
    ("Chinese (Taiwan)", "zh-TW", "gregory roc chinese"),
];

/// `CalendarSystems.vue`'s calendars, by name.
fn calendar_name(system: CalendarSystem) -> &'static str {
    match system {
        CalendarSystem::Gregorian => "Gregorian",
        CalendarSystem::Japanese => "Japanese",
        CalendarSystem::Buddhist => "Buddhist",
        CalendarSystem::Taiwan => "Taiwan",
        CalendarSystem::Persian => "Persian",
        CalendarSystem::Indian => "Indian",
        CalendarSystem::IslamicUmalqura => "Islamic (Umm al-Qura)",
        CalendarSystem::IslamicCivil => "Islamic Civil",
        CalendarSystem::IslamicTabular => "Islamic Tabular",
        CalendarSystem::Hebrew => "Hebrew",
        CalendarSystem::Coptic => "Coptic",
        CalendarSystem::Ethiopic => "Ethiopic",
        CalendarSystem::EthiopicAmeteAlem => "Ethiopic (Amete Alem)",
    }
}

/// The calendars a locale prefers, in its order (those `CalendarSystem` knows).
fn preferred(tag: &str) -> Vec<CalendarSystem> {
    LOCALES.iter().find(|(_, t, _)| *t == tag).map_or_else(
        || vec![CalendarSystem::Gregorian],
        |(_, _, order)| order.split(' ').filter_map(CalendarSystem::from_identifier).collect(),
    )
}

/// The demos' calendar look: `rounded-md border shadow-sm`.
fn framed<S: Styled>(d: S) -> S {
    d.radius(8.0).border(1.0).border_color(theme().border).shadow([
        Shadow {
            x: 0.0,
            y: 1.0,
            blur: 3.0,
            spread: 0.0,
            color: gui::Color::BLACK.opacity(0.1),
        },
        Shadow {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: -1.0,
            color: gui::Color::BLACK.opacity(0.1),
        },
    ])
}

/// A labelled column (`flex flex-col gap-3`, the label `px-1`).
fn labelled(name: &str, control: impl IntoElement) -> gui::Div {
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(12.0)
        .child(label(name).padding_x(4.0))
        .child(control)
}

/// A date picker's trigger and its popover calendar (`PopoverContent
/// class="w-auto overflow-hidden p-0" align="start|end"`).
fn picker(
    open: Signal<bool>,
    trigger: impl Into<gui::Div>,
    align: SideAlign,
    cal: impl IntoElement,
) -> Part<PopoverProps> {
    popover().open(open).child(popover_trigger(trigger)).child(
        popover_content()
            .side(Side::Bottom)
            .align(align)
            .width(Length::Auto)
            .overflow(Overflow::Hidden)
            .padding(0.0)
            .child(cal),
    )
}

/// A calendar on the page's fixed today.
fn cal() -> Part<CalendarProps> {
    calendar().today(today())
}

#[expect(clippy::too_many_lines, reason = "one page, one demo per block")]
pub fn render() -> Render {
    let picked = Signal::labeled("calendar picked", Some(today()));

    let date_time_open = Signal::labeled("date time open", false);
    let date_time = Signal::labeled("date and time", Some(today()));

    let birth_open = Signal::labeled("birth open", false);
    let birth = Signal::labeled("date of birth", Some(today()));

    let picker_open = Signal::labeled("picker open", false);
    let picker_date = Signal::labeled("picked date", None);

    let phrase = TextField::new("In 2 days");
    let phrase_open = Signal::labeled("phrase open", false);
    let phrase_date = Signal::labeled("phrase date", parse_date(&phrase.text(), today()));

    let persian = Signal::labeled("persian date", Some(today()));

    let layout_select = Signal::labeled("calendar layout", Some("month-and-year".to_owned()));

    let locale_select = Signal::labeled("calendar locale", Some("en-US".to_owned()));
    let system_select = Signal::labeled("calendar system", Some("gregory".to_owned()));
    let locale_sets_calendar = {
        let (locale, system) = (locale_select, system_select);
        // `updateLocale`: a new locale starts on the calendar it prefers.
        Effect::new(move || {
            let tag = locale.get().unwrap_or_default();
            let first = preferred(&tag).first().copied().unwrap_or_default();
            system.set(Some(first.identifier().to_owned()));
        })
    };

    Box::new(move || {
        let _effect = &locale_sets_calendar;
        let t = theme();
        let chosen = text_bound(move || {
            picked.get().map_or_else(
                || "No day picked".to_owned(),
                |d| format!("Picked {} {}, {}", d.month_name(), d.day(), d.year()),
            )
        });

        let custom_heading = div()
            .display(Display::Flex)
            .gap(8.0)
            .align_items(Align::Center)
            .child(div().child("Custom heading"))
            .child(calendar_month_select());

        let date_time_demo = div()
            .display(Display::Flex)
            .gap(16.0)
            .child(labelled(
                "Date",
                picker(
                    date_time_open,
                    button(ButtonVariant::Outline, ButtonSize::Default)
                        .width(128.0)
                        .justify_content(Justify::Between)
                        .font_weight(400)
                        .child(date_time.get().map_or_else(|| "Select date".to_owned(), numeric))
                        .child(icon(Icon::ChevronDown)),
                    SideAlign::Start,
                    cal().model(date_time).on_change(move |_| date_time_open.set(false)),
                ),
            ))
            .child(labelled(
                "Time",
                input_time().default_value("10:30:00").background(t.background),
            ));

        let birth_demo = labelled(
            "Date of birth",
            picker(
                birth_open,
                button(ButtonVariant::Outline, ButtonSize::Default)
                    .width(192.0)
                    .justify_content(Justify::Between)
                    .font_weight(400)
                    .child(birth.get().map_or_else(|| "Select date".to_owned(), numeric))
                    .child(icon(Icon::ChevronDown)),
                SideAlign::Start,
                cal()
                    .model(birth)
                    .on_change(move |_| birth_open.set(false))
                    .layout(CalendarLayout::MonthAndYear),
            ),
        );

        let picked_day = picker_date.get();
        let picker_demo = picker(
            picker_open,
            button(ButtonVariant::Outline, ButtonSize::Default)
                .width(240.0)
                .justify_content(Justify::Start)
                .text_align(TextAlign::Left)
                .font_weight(400)
                .when(picked_day.is_none(), |b| {
                    // Over the outline button's rest: its hover text stays over it.
                    let i = b.interaction();
                    b.text_color(t.muted_foreground)
                        .text_color_while(move || i.hovered(), t.accent_foreground)
                })
                .child(icon(Icon::Calendar))
                .child(picked_day.map_or_else(|| "Pick a date".to_owned(), long)),
            SideAlign::Start,
            cal()
                .model(picker_date)
                .on_change(move |_| picker_open.set(false))
                .layout(CalendarLayout::MonthAndYear),
        );

        let parsed = parse_date(&phrase.text(), today());
        let sync = phrase;
        let phrase_demo = div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(12.0)
            .child(label("Schedule Date").padding_x(4.0))
            .child(
                div()
                    .position(Position::Relative)
                    .display(Display::Flex)
                    .gap(8.0)
                    .child(
                        input()
                            .field(phrase)
                            .background(t.background)
                            .padding_right(40.0)
                            .placeholder("Tomorrow or next week"),
                    )
                    .child(
                        picker(
                            phrase_open,
                            button(ButtonVariant::Ghost, ButtonSize::Default)
                                .size(24.0)
                                .padding(0.0)
                                .named("date-picker")
                                .child(icon(Icon::Calendar).size(14.0))
                                // The calendar opens on the day the phrase names.
                                .on_click(move |_| phrase_date.set(parse_date(&sync.text(), today()))),
                            SideAlign::End,
                            {
                                let phrase = phrase;
                                cal()
                                    .model(phrase_date)
                                    .on_change(move |d| {
                                        if let Some(d) = d {
                                            phrase.set_text(two_digit_day(d));
                                        }
                                        phrase_open.set(false);
                                    })
                                    .initial_focus()
                            },
                        )
                        .position(Position::Absolute)
                        .top(6.0)
                        .right(8.0),
                    ),
            )
            .child(
                text("Your post will be published on ")
                    .span(span(parsed.map(two_digit_day).unwrap_or_default()).font_weight(500))
                    .span(".")
                    .padding_x(4.0)
                    .text_size(14.0)
                    .line_height(20.0)
                    .text_color(t.muted_foreground),
            );

        let persian_day = persian.get().unwrap_or_else(today);
        let (system, locale) = (CalendarSystem::Persian, CalendarLocale::FaIr);
        let fields = system.fields(persian_day);
        let persian_demo = div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .child(framed(
                cal()
                    .model(persian)
                    .layout(CalendarLayout::MonthAndYear)
                    .rtl()
                    .calendar(system, locale),
            ))
            .child(
                div()
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .justify_content(Justify::Center)
                    .align_items(Align::Center)
                    .gap(8.0)
                    .child(text(format!("{}/{}/{}", fields.year, fields.month, fields.day)))
                    .child(text(format!(
                        "{} {}",
                        locale.month(system, persian_day, MonthWidth::Short),
                        locale.year(system, persian_day)
                    ))),
            );

        let layouts = select_content().child(
            select_group()
                .child(select_item("month-and-year", "Month and Year"))
                .child(select_item("month-only", "Month Only"))
                .child(select_item("year-only", "Year Only")),
        );
        let layout = match layout_select.get().as_deref() {
            Some("month-only") => CalendarLayout::MonthOnly,
            Some("year-only") => CalendarLayout::YearOnly,
            _ => CalendarLayout::MonthAndYear,
        };
        let year_and_month_demo = div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(framed(
                cal()
                    .default_value(Some(today()))
                    .layout(layout)
                    .disable_days_outside_current_view(),
            ))
            .child(labelled(
                "Dropdown",
                select()
                    .model(layout_select)
                    .size(SelectSize::Sm)
                    .when(true, |s| {
                        // Over the trigger's rest: its dark hover fill stays over it.
                        let i = s.node_interaction();
                        let s = s.background(t.background);
                        match i {
                            Some(i) if t.dark => s.background_while(move || i.hovered(), t.input.opacity(0.5)),
                            _ => s,
                        }
                    })
                    .width(pct(100.0))
                    .child(select_value("Dropdown"))
                    .child(layouts),
            ));

        let tag = locale_select.get().unwrap_or_default();
        let first = preferred(&tag);
        let shown_system = system_select
            .get()
            .and_then(|id| CalendarSystem::from_identifier(&id))
            .unwrap_or_default();
        let others: Vec<CalendarSystem> = CalendarSystem::ALL.into_iter().filter(|c| !first.contains(c)).collect();
        let item = |value: &str, name: &str| {
            select_item(value, name).class(|d| {
                d.text_size(12.0)
                    .line_height(16.0)
                    .line_height(LineHeight::Relative(1.0))
                    .radius(3.0)
                    .height(25.0)
                    .padding_right(35.0)
                    .padding_left(25.0)
            })
        };
        let locales = select_content().children(LOCALES.map(|(name, tag, _)| item(tag, name)));
        let calendars = select_content()
            .child(select_label(""))
            .child(select_group().children(first.iter().map(|c| item(c.identifier(), calendar_name(*c)))))
            .child(select_separator())
            .child(select_label("Other"))
            .child(select_group().children(others.iter().map(|c| item(c.identifier(), calendar_name(*c)))));
        let systems_demo = div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(16.0)
            .child(label("Locale"))
            .child(
                select()
                    .model(locale_select)
                    .width(pct(100.0))
                    .child(select_value("Select a fruit"))
                    .child(locales),
            )
            .child(label("Calendar"))
            .child(
                select()
                    .model(system_select)
                    .width(pct(100.0))
                    .child(select_value("Select a calendar"))
                    .child(calendars),
            )
            .child(framed(cal().default_value(Some(today())).calendar(
                shown_system,
                CalendarLocale::from_tag(&tag).unwrap_or_default(),
            )));

        page(
            "Calendar",
            div()
                .flex(Flex::grow(1.0))
                .min_height(0.0)
                .overflow_y(Overflow::Scroll)
                .overflow_x(Overflow::Hidden)
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(section(
                    "Default",
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .align_items(Align::Start)
                        .gap(8.0)
                        .child(demo(
                            "CalendarDemo.vue",
                            framed(
                                cal()
                                    .model(picked)
                                    .min(CalendarDate::new(1925, 1, 1).unwrap_or_else(today))
                                    .max(CalendarDate::new(2035, 1, 1).unwrap_or_else(today))
                                    .layout(CalendarLayout::MonthAndYear),
                            ),
                        ))
                        .child(chosen.text_size(14.0).line_height(20.0).text_color(t.muted_foreground)),
                ))
                .child(demo("CalendarYearAndMonthSelector.vue", year_and_month_demo))
                .child(demo("CalendarPersianDemo.vue", persian_demo))
                .child(demo("CalendarSystems.vue", systems_demo))
                .child(demo(
                    "CalendarCustomCellSize.vue",
                    framed(
                        cal()
                            .default_value(Some(today()))
                            .weekday_format(WeekdayFormat::Short)
                            .cell_size(48.0)
                            .heading(custom_heading),
                    ),
                ))
                .child(demo("CalendarDateAndTimePicker.vue", date_time_demo))
                .child(demo("CalendarDateBirth.vue", birth_demo))
                .child(demo("DatePickerDemo.vue", picker_demo))
                .child(demo("CalendarNaturalLanguagePicker.vue", phrase_demo))
                .child(section(
                    "Ten days either side of today",
                    div()
                        .display(Display::Flex)
                        .child(framed(cal().min(today().add_days(-10)).max(today().add_days(10)))),
                )),
        )
    })
}

/// The centre of the first node the debug tools name `name`.
fn named(s: &Shot, name: &str) -> Result<(f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .find(|n| n.name == Some(name))
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound(name.to_owned()))
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("today selected")?;
    let (x, y) = s.locate("25")?;
    s.hover(x, y);
    s.frame("hover 25")?;
    s.click_text("25")?;
    s.frame("picked 25, today accent")?;
    s.press(NamedKey::ArrowRight, Modifiers::default());
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.frame("keys moved focus to Dec 3")?;
    s.press(NamedKey::Enter, Modifiers::default());
    s.frame("enter picked Dec 3")?;
    let (x, y) = named(s, "calendar-prev-button")?;
    s.click(x, y);
    s.frame("back to November")?;
    let (x, y) = named(s, "native-select")?;
    s.click(x, y);
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.frame("month select to December")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.press(NamedKey::ArrowUp, Modifiers::default());
    s.frame("year select to December 2025")?;
    pickers(s)?;
    time_field(s)?;
    layouts(s)?;
    systems(s)
}

/// CalendarDateAndTimePicker's time input, driven as in Chrome: a click
/// focuses the hour, the arrows step it and move between fields, digits
/// type ahead into the next field.
fn time_field(s: &mut Shot) -> Result<(), ScenarioError> {
    let (_, y) = reveal(s, "Time")?;
    let (x, _) = named(s, "time-input")?;
    // The hour, at the start of the fields (the box is ~98px wide).
    s.click(x - 30.0, y + 30.0);
    s.frame("time hour focused")?;
    s.press(NamedKey::ArrowUp, Modifiers::default());
    s.press(NamedKey::ArrowRight, Modifiers::default());
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.frame("time 11:29:00, minute focused")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.type_text("7");
    s.frame("time seconds typed 07")?;
    s.press(NamedKey::Backspace, Modifiers::default());
    s.frame("time seconds cleared")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("time blurred")?;
    Ok(())
}

/// Scroll the page until the node showing `label` is in view; its centre.
fn reveal(s: &mut Shot, label: &str) -> Result<(f32, f32), ScenarioError> {
    let (_, y) = s.locate(label)?;
    if (80.0..560.0).contains(&y) {
        return s.locate(label);
    }
    place(s, label, 240.0)
}

/// Scroll the page until the node showing `label` sits at `y`; its centre.
fn place(s: &mut Shot, label: &str, y: f32) -> Result<(f32, f32), ScenarioError> {
    let (_, at) = s.locate(label)?;
    s.hover(480.0, 320.0);
    s.wheel(at - y);
    s.settle()?;
    s.locate(label)
}

/// The centre of the last node showing exactly `label`: in an open popover,
/// which is laid out after the page.
fn last_text(s: &Shot, label: &str) -> Result<(f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .rev()
        .find(|n| matches!(&n.content, gui::layout::Content::Text { content, .. } if content == label))
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound(label.to_owned()))
}

fn pickers(s: &mut Shot) -> Result<(), ScenarioError> {
    let (x, y) = reveal(s, "Pick a date")?;
    s.click(x, y);
    s.frame("date picker open")?;
    let (x, y) = last_text(s, "12")?;
    s.click(x, y);
    s.frame("date picker picked Nov 12, closed")?;
    let (x, y) = reveal(s, "Schedule Date")?;
    s.hover(x, y);
    let (x, y) = named(s, "date-picker")?;
    s.click(x, y);
    s.frame("natural language calendar on Nov 22")?;
    let (x, y) = last_text(s, "3")?;
    s.click(x, y);
    s.frame("phrase is November 03, 2026")?;
    Ok(())
}

fn layouts(s: &mut Shot) -> Result<(), ScenarioError> {
    let (x, y) = reveal(s, "Month and Year")?;
    s.click(x, y);
    s.frame("layout select open")?;
    s.click_text("Month Only")?;
    place(s, "Dropdown", 560.0)?;
    s.frame("month only")?;
    s.click_text("Month Only")?;
    s.frame("layout select open again")?;
    let (x, y) = last_text(s, "Year Only")?;
    s.click(x, y);
    s.frame("year only")?;
    Ok(())
}

fn systems(s: &mut Shot) -> Result<(), ScenarioError> {
    let mut shown = "Gregorian";
    for name in ["Persian", "Hebrew", "Islamic (Umm al-Qura)", "Japanese", "Ethiopic"] {
        place(s, "Locale", 100.0)?;
        s.click_text(shown)?;
        s.frame(&format!("calendar select open on {shown}"))?;
        let (x, y) = last_text(s, name)?;
        s.click(x, y);
        s.frame(&format!("{name} calendar"))?;
        shown = name;
    }
    let (x, y) = last_text(s, "Default")?;
    s.click(x, y);
    s.frame("locale select open")?;
    let (x, y) = last_text(s, "Farsi (Iran)")?;
    s.click(x, y);
    s.frame("Farsi (Iran): Persian, Saturday first")?;
    Ok(())
}
