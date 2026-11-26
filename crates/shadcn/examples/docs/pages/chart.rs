use gui::prelude::*;
use gui::{Icon, ScenarioError, div, icon, text};
use shadcn::{
    ChartCrosshair, ChartIndicator, ChartMark, ChartPie, ChartRow, ChartSeries, ChartXy, card, card_content,
    card_description, card_footer, card_header, card_title, chart_container, chart_legend_content, chart_pie,
    chart_summary, chart_xy, theme,
};

use crate::{Render, Shot, demo, page, section};

/// Inside a card of half the page beside the scrollbar: (896 - 12 - 16) / 2 wide, less its
/// border and `px-6` each side.
const CHART_W: f32 = 384.0;
/// `aspect-video`.
const CHART_H: f32 = CHART_W * 9.0 / 16.0;
/// A demo's `ChartContainer` at `w-full aspect-video` in the specimen.
const SPECIMEN_W: f32 = crate::verdict::SPECIMEN_WIDTH;
const SPECIMEN_H: f32 = SPECIMEN_W * 9.0 / 16.0;
/// `ChartDemo`'s chart: a `max-w-md` card less its border and `px-6` each side.
const CARD_CHART_W: f32 = 448.0 - 2.0 - 48.0;

/// The demos' `chartData`: each month's first day (days since 2024-01-01), tick, label and values.
const MONTHS: [(f32, &str, &str, f32, f32); 6] = [
    (0.0, "Jan", "January", 186.0, 80.0),
    (31.0, "Feb", "February", 305.0, 200.0),
    (60.0, "Mar", "March", 237.0, 120.0),
    (91.0, "Apr", "April", 73.0, 190.0),
    (121.0, "May", "May", 209.0, 130.0),
    (152.0, "Jun", "June", 214.0, 140.0),
];

/// The months' rows with the two series in `order` (`true` puts Desktop first).
fn rows(desktop_first: bool) -> Vec<ChartRow> {
    MONTHS
        .iter()
        .map(|(x, tick, label, desktop, mobile)| ChartRow {
            x: *x,
            tick: (*tick).to_owned(),
            label: (*label).to_owned(),
            values: if desktop_first {
                vec![*desktop, *mobile]
            } else {
                vec![*mobile, *desktop]
            },
        })
        .collect()
}

fn series(label: &str, color: gui::Color) -> ChartSeries {
    ChartSeries {
        label: label.to_owned(),
        color,
    }
}

fn chart_card(title: &str, description: &str, chart: impl IntoElement, footer: &str) -> gui::Div {
    let t = theme();
    card()
        .child(
            card_header()
                .child(card_title(title))
                .child(card_description(description)),
        )
        .child(card_content().child(chart))
        .child(
            card_footer()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .align_items(Align::Start)
                .gap(8.0)
                .text_size(14.0)
                .line_height(20.0)
                .child(
                    text("Trending up by 5.2% this month")
                        .font_weight(500)
                        .line_height(LineHeight::Relative(1.0)),
                )
                .child(
                    text(footer)
                        .line_height(LineHeight::Relative(1.0))
                        .text_color(t.muted_foreground),
                ),
        )
}

pub fn render() -> Render {
    let stacked = rows(false);
    let grouped = rows(true);
    let visitors = ChartRow {
        x: 0.0,
        tick: String::new(),
        label: "Visitors".to_owned(),
        values: vec![275.0, 200.0, 187.0, 173.0, 90.0],
    };
    Box::new(move || {
        let t = theme();
        let [c1, c2, c3, c4, c5] = t.chart;
        let desktop_mobile = [series("Desktop", c1), series("Mobile", c2)];
        let mobile_desktop = [series("Mobile", c2), series("Desktop", c1)];
        let desktop = [series("Desktop", c1)];
        let browsers = [
            series("Chrome", c1),
            series("Safari", c2),
            series("Firefox", c3),
            series("Edge", c4),
            series("Other", c5),
        ];
        let xy = |mark, config: &[ChartSeries], rows: &[ChartRow], indicator, hide_label| ChartXy {
            mark,
            config: config.to_vec(),
            rows: rows.to_vec(),
            width: CHART_W,
            height: CHART_H,
            axes: true,
            y_ticks: Some(3.0),
            crosshair: Some(ChartCrosshair { indicator, hide_label }),
        };
        let area_chart = xy(ChartMark::Area, &mobile_desktop, &stacked, ChartIndicator::Dot, false);
        let bar_chart = xy(
            ChartMark::Bar {
                group_padding: 0.0,
                bar_padding: 0.15,
            },
            &desktop_mobile,
            &grouped,
            ChartIndicator::Dot,
            false,
        );
        let line_chart = xy(ChartMark::Line, &desktop, &grouped, ChartIndicator::Line, true);
        let pie = ChartPie {
            config: &browsers,
            row: &visitors,
            size: 200.0,
            arc_width: Some(30.0),
        };
        page(
            "Chart",
            div()
                .flex(Flex::grow(1.0))
                .min_height(0.0)
                .overflow_y(Overflow::Scroll)
                .overflow_x(Overflow::Hidden)
                .child(
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(32.0)
                        .padding_right(12.0)
                        .child(
                            div()
                                .display(Display::Grid)
                                .grid_columns(Tracks::repeat(2, Track::Fr(1.0)))
                                .gap(16.0)
                                .child(chart_card(
                                    "Area Chart - Stacked",
                                    "Showing total visitors for the last 6 months",
                                    chart_container().child(chart_xy(area_chart)),
                                    "January - June 2024",
                                ))
                                .child(chart_card(
                                    "Bar Chart - Multiple",
                                    "January - June 2024",
                                    chart_container()
                                        .child(chart_xy(bar_chart))
                                        .child(chart_legend_content(&desktop_mobile)),
                                    "Showing total visitors for the last 6 months",
                                ))
                                .child(chart_card(
                                    "Line Chart",
                                    "January - June 2024",
                                    chart_container().child(chart_xy(line_chart)),
                                    "Showing total visitors for the last 6 months",
                                ))
                                .child(chart_card(
                                    "Pie Chart - Donut",
                                    "January - June 2024",
                                    chart_container()
                                        .align_items(Align::Center)
                                        .child(chart_pie(&pie))
                                        .child(chart_legend_content(&browsers))
                                        .child(
                                            text(chart_summary(&browsers, std::slice::from_ref(&visitors)))
                                                .padding_top(12.0)
                                                .text_color(t.muted_foreground),
                                        ),
                                    "Showing total visitors for the last 6 months",
                                )),
                        )
                        .child(section("shadcn-vue demos", demos())),
                ),
        )
    })
}

/// shadcn-vue's chart demos, one for one, for the verdict.
fn demos() -> gui::Div {
    let t = theme();
    let [c1, c2, ..] = t.chart;
    let months = rows(true);
    let themed = [series("Desktop", c1), series("Mobile", c2)];
    let fixed = [
        series("Desktop", gui::Color::hex(0x25_63_eb)),
        series("Mobile", gui::Color::hex(0x60_a5_fa)),
    ];
    // `bar-padding="0.1" group-padding="0"`, with or without the axes and the crosshair.
    let full = |height, axes, crosshair: bool| ChartXy {
        mark: ChartMark::Bar {
            group_padding: 0.0,
            bar_padding: 0.1,
        },
        config: fixed.to_vec(),
        rows: months.clone(),
        width: SPECIMEN_W,
        height,
        axes,
        y_ticks: None,
        crosshair: crosshair.then(ChartCrosshair::default),
    };
    // `VisGroupedBar`'s defaults: `group-padding` 0.05, `bar-padding` 0.
    let in_card = ChartXy {
        mark: ChartMark::Bar {
            group_padding: 0.05,
            bar_padding: 0.0,
        },
        config: themed.to_vec(),
        width: CARD_CHART_W,
        height: CARD_CHART_W * 9.0 / 16.0,
        ..full(0.0, true, true)
    };
    let plain = full(SPECIMEN_H, false, false);
    let axes = full(SPECIMEN_H, true, false);
    let tooltip = full(SPECIMEN_H, true, true);
    // The legend's `pt-3` and 16px line come out of the container's height.
    let legend = full(SPECIMEN_H - 28.0, true, true);
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(32.0)
        .child(demo(
            "ChartDemo.vue",
            card()
                .width(pct(100.0))
                .max_width(448.0)
                .child(
                    card_header()
                        .child(card_title("Bar Chart - Multiple"))
                        .child(card_description("January - June 2024")),
                )
                .child(card_content().child(chart_container().child(chart_xy(in_card))))
                .child(
                    card_footer()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .align_items(Align::Start)
                        .gap(8.0)
                        .text_size(14.0)
                        .line_height(20.0)
                        .child(
                            div()
                                .display(Display::Flex)
                                .gap(8.0)
                                .font_weight(500)
                                .line_height(LineHeight::Relative(1.0))
                                .child("Trending up by 5.2% this month")
                                .child(icon(Icon::TrendingUp).size(16.0)),
                        )
                        .child(
                            text("Showing total visitors for the last 6 months")
                                .line_height(LineHeight::Relative(1.0))
                                .text_color(t.muted_foreground),
                        ),
                ),
        ))
        .child(demo("ChartBarDemo.vue", chart_container().child(chart_xy(plain))))
        .child(demo("ChartBarDemoAxis.vue", chart_container().child(chart_xy(axes))))
        .child(demo(
            "ChartBarDemoLegend.vue",
            chart_container()
                .child(chart_xy(legend))
                .child(chart_legend_content(&fixed)),
        ))
        .child(demo(
            "ChartBarDemoTooltip.vue",
            chart_container().child(chart_xy(tooltip)),
        ))
}

/// The centre of the `n`th (from 0) node the debug tools name `name`, moved
/// `dy` down.
fn nth_named(s: &Shot, name: &str, n: usize, dy: f32) -> Result<(f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .filter(|node| node.name == Some(name))
        .nth(n)
        .map(|node| {
            let b = node.bounds(gui::inspect::OUTSIDE);
            (b.x + b.width / 2.0, b.y + b.height / 2.0 + dy)
        })
        .ok_or_else(|| ScenarioError::TextNotFound(name.to_owned()))
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    // Over a row's tick, up in its plot: the row the plot finds is hovered.
    let (x, y) = nth_named(s, "chart-tick", 1, -60.0)?;
    s.hover(x, y);
    s.frame("area hover February")?;
    let (x, y) = nth_named(s, "chart-tick", 9, -60.0)?;
    s.hover(x, y);
    s.frame("bar hover April")?;
    s.wheel(400.0);
    s.frame("scrolled to line and donut")?;
    let (x, y) = nth_named(s, "chart-tick", 14, -60.0)?;
    s.hover(x, y);
    s.frame("line hover March")?;
    Ok(())
}
