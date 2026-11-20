use gui::layout::Content;
use gui::prelude::*;
use gui::{ScenarioError, div, text};
use shadcn::{CalendarDate, RangeCalendarState, RangeCalendarValue, range_calendar, theme};

use crate::{Render, Shot, demo, page, section};

/// A fixed "today" so every run draws the same months; `CalendarRangeDemo.vue` uses
/// the real one, so the verdict matches only when run on this date.
fn today() -> CalendarDate {
    CalendarDate::new(2026, 11, 20).unwrap_or_else(CalendarDate::today_utc)
}

fn describe(value: RangeCalendarValue) -> String {
    let day = |d: CalendarDate| format!("{} {}", d.month_name(), d.day());
    match value {
        RangeCalendarValue::Empty => "No range picked".to_owned(),
        RangeCalendarValue::Started(start) => format!("From {}, pick the end", day(start)),
        RangeCalendarValue::Range { start, end } => format!("{} to {}", day(start), day(end)),
    }
}

pub fn render() -> Render {
    let state = RangeCalendarState::new(today(), RangeCalendarValue::between(today(), today().add_days(7)))
        .number_of_months(2)
        .disable_days_outside_current_view();
    Box::new(move || {
        let t = theme();
        page(
            "Range Calendar",
            section(
                "Two months, days outside them disabled",
                div()
                    .flex_col()
                    .items_start()
                    .gap(2.0)
                    .child(demo(
                        "CalendarRangeDemo.vue",
                        range_calendar(&state)
                            .rounded_md()
                            .border()
                            .border_color(t.border)
                            .shadow_sm(),
                    ))
                    .child(
                        text(describe(state.value.get()))
                            .text_sm()
                            .text_color(t.muted_foreground),
                    ),
            ),
        )
    })
}

/// The centre of the `n`th (from 0) node showing exactly `label`.
fn nth_text(s: &Shot, label: &str, n: usize) -> Result<(f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .filter(|node| matches!(&node.content, Content::Text { content, .. } if content == label))
        .nth(n)
        .map(|node| {
            (
                node.bounds.x + node.bounds.width / 2.0,
                node.bounds.y + node.bounds.height / 2.0,
            )
        })
        .ok_or_else(|| ScenarioError::TextNotFound(label.to_owned()))
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("a week from today")?;
    s.click_text("3")?;
    s.frame("started on Nov 3")?;
    let (x, y) = nth_text(s, "9", 1)?;
    s.click(x, y);
    s.frame("Nov 3 to Dec 9")?;
    let (x, y) = nth_text(s, "29", 1)?;
    s.click(x, y);
    s.frame("outside day takes no click")?;
    Ok(())
}
