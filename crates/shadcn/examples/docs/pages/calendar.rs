use gui::prelude::*;
use gui::{
    Effect, Icon, Modifiers, NamedKey, ScenarioError, Side, SideAlign, TextField, TimeField, div, icon, span, text,
};
use shadcn::{
    ButtonSize, ButtonVariant, CalendarDate, CalendarLayout, CalendarLocale, CalendarState, CalendarSystem, MonthWidth,
    PopoverState, SelectSize, SelectState, WeekdayFormat, button, calendar, calendar_month_select,
    calendar_with_heading, input, input_time, label, popover, popover_content_placed, popover_trigger, select_content,
    select_group, select_item, select_label, select_separator, select_trigger, theme,
};

use crate::{Render, Shot, demo, page, section};

/// A fixed "today" so every run draws the same month; the demos use the
/// real one, so the verdict matches only when run on this date.
fn today() -> CalendarDate {
    CalendarDate::new(2026, 11, 20).unwrap_or_else(CalendarDate::today_utc)
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
fn framed(d: gui::Div) -> gui::Div {
    d.rounded_md().border().border_color(theme().border).shadow_sm()
}

/// A labelled column (`flex flex-col gap-3`, the label `px-1`).
fn labelled(name: &str, control: impl IntoElement) -> gui::Div {
    div().flex_col().gap(3.0).child(label(name).px(1.0)).child(control)
}

/// A date picker's trigger and its popover calendar (`PopoverContent
/// class="w-auto overflow-hidden p-0" align="start|end"`).
fn picker(open: &PopoverState, trigger: gui::Div, align: SideAlign, cal: gui::Div) -> gui::Div {
    popover()
        .child(popover_trigger(open, trigger))
        .child(popover_content_placed(open, Side::Bottom, align, |c| {
            c.w_auto().overflow_hidden().p(0.0).child(cal)
        }))
}

#[expect(clippy::too_many_lines, reason = "one page, one demo per block")]
pub fn render() -> Render {
    let picked = CalendarState::new(today(), Some(today()))
        .min(CalendarDate::new(1925, 1, 1).unwrap_or_else(today))
        .max(CalendarDate::new(2035, 1, 1).unwrap_or_else(today))
        .layout(CalendarLayout::MonthAndYear);
    let bounded = CalendarState::new(today(), None)
        .min(today().add_days(-10))
        .max(today().add_days(10));
    let cell_size = CalendarState::new(today(), Some(today()))
        .weekday_format(WeekdayFormat::Short)
        .cell_size(12.0);

    let date_time_open = PopoverState::new();
    let date_time = {
        let open = date_time_open.clone();
        CalendarState::new(today(), Some(today())).on_update(move |_| open.close())
    };
    let time = TimeField::new("10:30:00");

    let birth_open = PopoverState::new();
    let birth = {
        let open = birth_open.clone();
        CalendarState::new(today(), Some(today()))
            .layout(CalendarLayout::MonthAndYear)
            .on_update(move |_| open.close())
    };

    let picker_open = PopoverState::new();
    let picker_date = {
        let open = picker_open.clone();
        CalendarState::new(today(), None)
            .layout(CalendarLayout::MonthAndYear)
            .on_update(move |_| open.close())
    };

    let phrase = TextField::new("In 2 days");
    let phrase_open = PopoverState::new();
    let phrase_date = {
        let (open, phrase) = (phrase_open.clone(), phrase.clone());
        CalendarState::new(today(), parse_date(&phrase.text(), today())).on_update(move |d| {
            phrase.set_text(two_digit_day(d));
            open.close();
        })
    };

    let persian = CalendarState::new(today(), Some(today()))
        .layout(CalendarLayout::MonthAndYear)
        .rtl()
        .calendar(CalendarSystem::Persian, CalendarLocale::FaIr);

    let year_and_month = CalendarState::new(today(), Some(today()))
        .layout(CalendarLayout::MonthAndYear)
        .disable_days_outside_current_view();
    let layout_select = SelectState::new(Some("month-and-year"));
    let follow_layout = {
        let (select, state) = (layout_select.clone(), year_and_month.clone());
        Effect::new(move || {
            state.layout.set(match select.value.get().as_deref() {
                Some("month-only") => CalendarLayout::MonthOnly,
                Some("year-only") => CalendarLayout::YearOnly,
                _ => CalendarLayout::MonthAndYear,
            });
        })
    };

    let systems = CalendarState::new(today(), Some(today()));
    let locale_select = SelectState::new(Some("en-US"));
    let system_select = SelectState::new(Some("gregory"));
    let locale_sets_calendar = {
        let (locale, system) = (locale_select.clone(), system_select.clone());
        // `updateLocale`: a new locale starts on the calendar it prefers.
        Effect::new(move || {
            let tag = locale.value.get().unwrap_or_default();
            let first = preferred(&tag).first().copied().unwrap_or_default();
            system.value.set(Some(first.identifier().to_owned()));
        })
    };
    let calendar_follows = {
        let (locale, system, state) = (locale_select.clone(), system_select.clone(), systems.clone());
        Effect::new(move || {
            let tag = locale.value.get().unwrap_or_default();
            let sys = system
                .value
                .get()
                .and_then(|id| CalendarSystem::from_identifier(&id))
                .unwrap_or_default();
            state
                .view
                .set_calendar(sys, CalendarLocale::from_tag(&tag).unwrap_or_default());
        })
    };

    Box::new(move || {
        let _effects = (&follow_layout, &locale_sets_calendar, &calendar_follows);
        let t = theme();
        let chosen = picked.selected.get().map_or_else(
            || "No day picked".to_owned(),
            |d| format!("Picked {} {}, {}", d.month_name(), d.day(), d.year()),
        );

        let custom_heading = div()
            .flex()
            .gap(2.0)
            .items_center()
            .child(div().child("Custom heading"))
            .child(calendar_month_select(&cell_size));

        let date_time_demo = div()
            .flex()
            .gap(4.0)
            .child(labelled(
                "Date",
                picker(
                    &date_time_open,
                    button(ButtonVariant::Outline, ButtonSize::Default)
                        .w(32.0)
                        .justify_between()
                        .font_normal()
                        .child(
                            date_time
                                .selected
                                .get()
                                .map_or_else(|| "Select date".to_owned(), numeric),
                        )
                        .child(icon(Icon::ChevronDown)),
                    SideAlign::Start,
                    calendar(&date_time),
                ),
            ))
            .child(labelled("Time", input_time(&time).bg(t.background)));

        let birth_demo = labelled(
            "Date of birth",
            picker(
                &birth_open,
                button(ButtonVariant::Outline, ButtonSize::Default)
                    .w(48.0)
                    .justify_between()
                    .font_normal()
                    .child(birth.selected.get().map_or_else(|| "Select date".to_owned(), numeric))
                    .child(icon(Icon::ChevronDown)),
                SideAlign::Start,
                calendar(&birth),
            ),
        );

        let picked_day = picker_date.selected.get();
        let picker_demo = picker(
            &picker_open,
            button(ButtonVariant::Outline, ButtonSize::Default)
                .w_px(240.0)
                .justify_start()
                .text_left()
                .font_normal()
                .when(picked_day.is_none(), |b| b.text_color(t.muted_foreground))
                .child(icon(Icon::Calendar))
                .child(picked_day.map_or_else(|| "Pick a date".to_owned(), long)),
            SideAlign::Start,
            calendar(&picker_date),
        );

        let parsed = parse_date(&phrase.text(), today());
        let sync = (phrase.clone(), phrase_date.clone());
        let phrase_demo = div()
            .flex_col()
            .gap(3.0)
            .child(label("Schedule Date").px(1.0))
            .child(
                div()
                    .relative()
                    .flex()
                    .gap(2.0)
                    .child(
                        input(&phrase)
                            .bg(t.background)
                            .pr(10.0)
                            .placeholder("Tomorrow or next week"),
                    )
                    .child(
                        picker(
                            &phrase_open,
                            button(ButtonVariant::Ghost, ButtonSize::Default)
                                .size(6.0)
                                .p(0.0)
                                .named("date-picker")
                                .child(icon(Icon::Calendar).size(3.5))
                                // The calendar opens on the day the phrase names.
                                .on_click(move |_| {
                                    let (phrase, state) = &sync;
                                    let day = parse_date(&phrase.text(), today());
                                    state.selected.set(day);
                                    if let Some(day) = day {
                                        state.view.focus_day(day, false);
                                    }
                                }),
                            SideAlign::End,
                            calendar(&phrase_date),
                        )
                        .absolute()
                        .top(1.5)
                        .right(2.0),
                    ),
            )
            .child(
                text("Your post will be published on ")
                    .span(span(parsed.map(two_digit_day).unwrap_or_default()).font_medium())
                    .span(".")
                    .px(1.0)
                    .text_sm()
                    .text_color(t.muted_foreground),
            );

        let persian_day = persian.selected.get().unwrap_or_else(today);
        let (system, locale) = (CalendarSystem::Persian, CalendarLocale::FaIr);
        let fields = system.fields(persian_day);
        let persian_demo = div().flex_col().child(framed(calendar(&persian))).child(
            div()
                .flex_col()
                .justify_center()
                .items_center()
                .gap(2.0)
                .child(text(format!("{}/{}/{}", fields.year, fields.month, fields.day)))
                .child(text(format!(
                    "{} {}",
                    locale.month(system, persian_day, MonthWidth::Short),
                    locale.year(system, persian_day)
                ))),
        );

        let layouts = select_content(&layout_select).child(
            select_group()
                .child(select_item(&layout_select, "month-and-year", "Month and Year", false))
                .child(select_item(&layout_select, "month-only", "Month Only", false))
                .child(select_item(&layout_select, "year-only", "Year Only", false)),
        );
        let year_and_month_demo = div()
            .flex_col()
            .gap(4.0)
            .child(framed(calendar(&year_and_month)))
            .child(labelled(
                "Dropdown",
                select_trigger(&layout_select, "Dropdown", SelectSize::Sm, layouts)
                    .bg(t.background)
                    .w_full(),
            ));

        let tag = locale_select.value.get().unwrap_or_default();
        let first = preferred(&tag);
        let others: Vec<CalendarSystem> = CalendarSystem::ALL.into_iter().filter(|c| !first.contains(c)).collect();
        let item = |select: &SelectState, value: &str, name: &str| {
            select_item(select, value, name, false)
                .text_xs()
                .leading_none()
                .rounded(3.0)
                .h_px(25.0)
                .pr(8.75)
                .pl(6.25)
        };
        let locales =
            select_content(&locale_select).children(LOCALES.map(|(name, tag, _)| item(&locale_select, tag, name)));
        let calendars = select_content(&system_select)
            .child(select_label(""))
            .child(
                select_group().children(
                    first
                        .iter()
                        .map(|c| item(&system_select, c.identifier(), calendar_name(*c))),
                ),
            )
            .child(select_separator())
            .child(select_label("Other"))
            .child(
                select_group().children(
                    others
                        .iter()
                        .map(|c| item(&system_select, c.identifier(), calendar_name(*c))),
                ),
            );
        let systems_demo = div()
            .flex_col()
            .gap(4.0)
            .child(label("Locale"))
            .child(select_trigger(&locale_select, "Select a fruit", SelectSize::Default, locales).w_full())
            .child(label("Calendar"))
            .child(select_trigger(&system_select, "Select a calendar", SelectSize::Default, calendars).w_full())
            .child(framed(calendar(&systems)));

        page(
            "Calendar",
            div()
                .flex_1()
                .min_h(0.0)
                .overflow_y_scroll()
                .flex_col()
                .gap(8.0)
                .child(section(
                    "Default",
                    div()
                        .flex_col()
                        .items_start()
                        .gap(2.0)
                        .child(demo("CalendarDemo.vue", framed(calendar(&picked))))
                        .child(text(chosen).text_sm().text_color(t.muted_foreground)),
                ))
                .child(demo("CalendarYearAndMonthSelector.vue", year_and_month_demo))
                .child(demo("CalendarPersianDemo.vue", persian_demo))
                .child(demo("CalendarSystems.vue", systems_demo))
                .child(demo(
                    "CalendarCustomCellSize.vue",
                    framed(calendar_with_heading(&cell_size, custom_heading)),
                ))
                .child(demo("CalendarDateAndTimePicker.vue", date_time_demo))
                .child(demo("CalendarDateBirth.vue", birth_demo))
                .child(demo("DatePickerDemo.vue", picker_demo))
                .child(demo("CalendarNaturalLanguagePicker.vue", phrase_demo))
                .child(section(
                    "Ten days either side of today",
                    div().flex().child(framed(calendar(&bounded))),
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
