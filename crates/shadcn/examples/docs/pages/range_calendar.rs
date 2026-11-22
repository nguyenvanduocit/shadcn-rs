use gui::layout::Content;
use gui::prelude::*;
use gui::{ScenarioError, Signal, div, text_bound};
use shadcn::{CalendarDate, RangeCalendarValue, range_calendar, theme};

use crate::{Render, Shot, demo, page, section};

/// The docs' fixed today (`verdict::CLOCK_MS`), so every run draws the same
/// month; the verdict pins Chrome's clock to the same instant.
fn today() -> CalendarDate {
    crate::verdict::today()
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
    let value = Signal::labeled(
        "range calendar value",
        RangeCalendarValue::between(today(), today().add_days(7)),
    );
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
                        range_calendar()
                            .model(value)
                            .today(today())
                            .number_of_months(2)
                            .disable_days_outside_current_view()
                            .rounded_md()
                            .border()
                            .border_color(t.border)
                            .shadow_sm(),
                    ))
                    .child(
                        text_bound(move || describe(value.get()))
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
