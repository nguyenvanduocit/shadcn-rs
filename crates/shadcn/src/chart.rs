//! Chart (`ui/chart`): the container, tooltip and legend shadcn wraps around
//! its charting library, and the marks that library draws (area, bar, line,
//! pie and donut), built on `gui::shapes` and laid out as unovis lays out
//! shadcn-vue's: grid lines, x-axis ticks and a hover tooltip per category.
//! Charts carry a text equivalent ([`chart_summary`]).

use std::f32::consts::{FRAC_PI_2, TAU};

use gui::prelude::*;
use gui::{Color, Div, Placement, Shape, Side, SideAlign, Signal, deferred, div, px, shapes, text};

use crate::theme::theme;

/// One entry of shadcn's `ChartConfig`: a series' name and colour. A chart's
/// config lists its series in the order of each row's values.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartSeries {
    pub label: String,
    pub color: Color,
}

/// One category of data: where it sits, its x-axis tick, its tooltip label,
/// and one value per series.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartRow {
    /// Its place on the x axis, a continuous scale as unovis' over a date:
    /// days since the first row, or the row's index to space rows evenly.
    pub x: f32,
    /// Under the x axis ("Jan").
    pub tick: String,
    /// Atop the tooltip ("January").
    pub label: String,
    pub values: Vec<f32>,
}

/// What an x-y chart draws for its series.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChartMark {
    /// Areas stacked in config order, the first at the bottom, each edged by a line.
    Area,
    /// Bars grouped side by side at each row's x (`VisGroupedBar`): the share
    /// of each group's slot left empty (`group-padding`), and the d3 band
    /// padding between and around a group's bars (`bar-padding`).
    Bar { group_padding: f32, bar_padding: f32 },
    /// One line per series.
    Line,
}

/// The swatch beside each tooltip row (`indicator`, or `hideIndicator`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ChartIndicator {
    #[default]
    Dot,
    Line,
    Hidden,
}

/// An x-y chart: its marks, data and size. The size is in pixels because a
/// shape is drawn in coordinates fixed when the tree is built.
#[derive(Clone, Copy, Debug)]
pub struct ChartXy<'a> {
    pub mark: ChartMark,
    pub config: &'a [ChartSeries],
    pub rows: &'a [ChartRow],
    pub width: f32,
    pub height: f32,
    /// `VisAxis` x and y: the rows' ticks under the plot and a y grid. Without
    /// them the marks fill the box.
    pub axes: bool,
    /// The y axis' `num-ticks`; `None` takes unovis' default for the plot's
    /// height. The grid draws twice as many lines.
    pub y_ticks: Option<f32>,
    /// What hovering a row shows; `None` for a chart that only draws.
    pub crosshair: Option<ChartCrosshair>,
}

/// `ChartTooltip` and `ChartCrosshair`: hovering a row shows a dot on each
/// area or line, and a tooltip of the row's values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ChartCrosshair {
    pub indicator: ChartIndicator,
    /// `hideLabel`: the tooltip shows no title row.
    pub hide_label: bool,
}

/// A pie, or a donut when `arc_width` is set: one slice per series of `row`.
#[derive(Clone, Copy, Debug)]
pub struct ChartPie<'a> {
    pub config: &'a [ChartSeries],
    pub row: &'a ChartRow,
    /// Width and height of the square it fills.
    pub size: f32,
    pub arc_width: Option<f32>,
}

/// Which category the pointer is over. Created once, outside the render closure.
#[derive(Clone)]
pub struct ChartState {
    pub active: Signal<Option<usize>>,
}

impl Default for ChartState {
    fn default() -> Self {
        Self::new()
    }
}

impl ChartState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: Signal::labeled("chart active", None),
        }
    }
}

/// One row of a tooltip.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartTooltipItem {
    pub label: String,
    pub value: String,
    pub color: Color,
}

/// The x axis under the plot, as tall as unovis measures it: a 6px tick, 8px
/// padding and the 12px label's box.
const X_AXIS: f32 = 27.0;
/// Where the tick labels' 16px line starts under the plot, so its middle sits
/// where unovis centres the label: tick, padding and half the font (20px).
const TICK_TOP: f32 = 12.0;
/// The y axis left of the plot: its 6px tick, drawn transparent, and half its stroke.
const Y_AXIS: f32 = 6.5;
/// How far the first and last points of an area or line sit from the sides, so their ticks fit.
const EDGE: f32 = 16.0;
/// `rounded-corners="4"`.
const BAR_RADIUS: f32 = 4.0;
/// The crosshair's dot on each series at the hovered row (`circleRadius`); the
/// plot keeps this much room above and beside it so a dot is never cut.
const DOT: f32 = 4.0;

/// The chart's frame (`ChartContainer`): a column, centred, in `text-xs`.
#[must_use]
pub fn chart_container() -> Div {
    div().named("chart").flex_col().justify_center().text_xs()
}

/// An area, bar or line chart, with a y grid and x ticks when it has axes, and a
/// tooltip for the hovered category when it has a crosshair.
#[must_use]
pub fn chart_xy(chart: &ChartXy<'_>, state: &ChartState) -> Div {
    let t = theme();
    let plot = Plot::new(chart);
    let active = state.active.get();
    let mut drawn: Vec<Shape> = plot
        .grid()
        .into_iter()
        .map(|y| {
            Shape::path()
                .move_to(plot.left, y)
                .line_to(chart.width, y)
                .stroke(t.border, 1.0)
        })
        .collect();
    drawn.extend(plot.marks());
    if !matches!(chart.mark, ChartMark::Bar { .. })
        && chart.crosshair.is_some()
        && let Some(i) = active
    {
        drawn.extend(plot.dots(i));
    }
    div()
        .named("chart-xy")
        .relative()
        .w_px(chart.width)
        .h_px(chart.height)
        .child(
            shapes(drawn)
                .absolute()
                .top(0.0)
                .left(0.0)
                .w_px(chart.width)
                .h_px(chart.height),
        )
        .children(chart.rows.iter().zip(plot.xs()).filter(|_| chart.axes).map(|(row, x)| {
            div()
                .absolute()
                .left_len(px(x - plot.band / 2.0))
                .top_len(px(plot.base + TICK_TOP))
                .w_px(plot.band)
                .flex()
                .justify_center()
                .whitespace_nowrap()
                .text_color(t.muted_foreground)
                .child(row.tick.clone())
        }))
        .children(chart.crosshair.into_iter().flat_map(|crosshair| {
            chart
                .rows
                .iter()
                .zip(plot.columns())
                .enumerate()
                .map(move |(i, (row, (left, right)))| {
                    let hovered = state.active.clone();
                    let column = div()
                        .named("chart-category")
                        .absolute()
                        .left_len(px(left))
                        .top(0.0)
                        .w_px((right - left).max(0.0))
                        .h_px(plot.base)
                        .on_hover(move |inside| {
                            if inside {
                                hovered.set(Some(i));
                            } else if hovered.get() == Some(i) {
                                hovered.set(None);
                            }
                        });
                    column.when(active == Some(i), |c| {
                        let items: Vec<ChartTooltipItem> = chart
                            .config
                            .iter()
                            .zip(&row.values)
                            .map(|(s, v)| ChartTooltipItem {
                                label: s.label.clone(),
                                value: chart_value(*v),
                                color: s.color,
                            })
                            .collect();
                        let label = (!crosshair.hide_label).then_some(row.label.as_str());
                        c.child(deferred(
                            Placement::Anchored {
                                side: Side::Right,
                                align: SideAlign::Center,
                                offset: 8.0,
                                match_width: false,
                                fit_height: false,
                            },
                            chart_tooltip_content(label, &items, crosshair.indicator),
                        ))
                    })
                })
        }))
}

/// A pie or donut, slices clockwise from twelve o'clock in config order.
#[must_use]
pub fn chart_pie(pie: &ChartPie<'_>) -> Div {
    let total: f32 = pie.row.values.iter().map(|v| v.max(0.0)).sum();
    let r = pie.size / 2.0;
    let inner = pie.arc_width.map_or(0.0, |w| (r - w).max(0.0));
    let mut start = 0.0_f32;
    let slices: Vec<Shape> = pie
        .config
        .iter()
        .zip(&pie.row.values)
        .filter(|_| total > 0.0)
        .map(|(series, v)| {
            let end = start + v.max(0.0) / total * TAU;
            let outer = arc(
                Shape::path().move_to(r + r * start.sin(), r - r * start.cos()),
                r,
                r,
                start,
                end,
            );
            let slice = if inner > 0.0 {
                arc(
                    outer.line_to(r + inner * end.sin(), r - inner * end.cos()),
                    r,
                    inner,
                    end,
                    start,
                )
            } else {
                outer.line_to(r, r)
            };
            start = end;
            slice.close().fill(series.color)
        })
        .collect();
    div()
        .named("chart-pie")
        .w_px(pie.size)
        .h_px(pie.size)
        .child(shapes(slices).w_px(pie.size).h_px(pie.size))
}

/// The tooltip's card (`ChartTooltipContent`): an optional title, then one
/// row per series with its swatch, name and value.
#[must_use]
pub fn chart_tooltip_content(label: Option<&str>, items: &[ChartTooltipItem], indicator: ChartIndicator) -> Div {
    let t = theme();
    let nest_label = items.len() == 1 && indicator != ChartIndicator::Dot;
    div()
        .named("chart-tooltip")
        .grid()
        .min_w_px(128.0)
        .items_start()
        .gap(1.5)
        .rounded_lg()
        .border()
        .border_color(t.border.opacity(0.5))
        .bg(t.background)
        .text_color(t.foreground)
        .px(2.5)
        .py(1.5)
        .text_xs()
        .shadow_xl()
        .when_some(label.filter(|_| !nest_label), |d, l| d.child(text(l).font_medium()))
        .child(div().grid().gap(1.5).children(items.iter().map(|item| {
            let swatch = match indicator {
                ChartIndicator::Dot => Some(div().size(2.5)),
                ChartIndicator::Line => Some(div().w(1.0)),
                ChartIndicator::Hidden => None,
            };
            div()
                .flex()
                .w_full()
                .flex_wrap()
                .gap(2.0)
                .items_stretch()
                .when(indicator == ChartIndicator::Dot, |d| d.items_center())
                .when_some(swatch, |d, s| d.child(s.shrink_0().rounded_xs().bg(item.color)))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .justify_between()
                        .leading_none()
                        .when(nest_label, |d| d.items_end())
                        .when(!nest_label, |d| d.items_center())
                        .child(
                            div()
                                .grid()
                                .gap(1.5)
                                .when_some(label.filter(|_| nest_label), |d, l| d.child(text(l).font_medium()))
                                .child(text(item.label.clone()).text_color(t.muted_foreground)),
                        )
                        .child(
                            text(item.value.clone())
                                .font_mono()
                                .font_medium()
                                .text_color(t.foreground),
                        ),
                )
        })))
}

/// The legend (`ChartLegendContent`): each series' swatch and name, centred under the chart.
#[must_use]
pub fn chart_legend_content(config: &[ChartSeries]) -> Div {
    div()
        .named("chart-legend")
        .flex()
        .items_center()
        .justify_center()
        .gap(4.0)
        .pt(3.0)
        .children(config.iter().map(|s| {
            div()
                .flex()
                .items_center()
                .gap(1.5)
                .child(div().size(2.0).shrink_0().rounded_xs().bg(s.color))
                .child(s.label.clone())
        }))
}

/// What the chart shows, in words: one sentence per row.
#[must_use]
pub fn chart_summary(config: &[ChartSeries], rows: &[ChartRow]) -> String {
    rows.iter()
        .map(|row| {
            let values: Vec<String> = config
                .iter()
                .zip(&row.values)
                .map(|(s, v)| format!("{} {}", s.label, chart_value(*v)))
                .collect();
            format!("{}: {}.", row.label, values.join(", "))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A value as `toLocaleString` writes it in English: thousands grouped, up to three decimals.
#[must_use]
pub fn chart_value(v: f32) -> String {
    let plain = format!("{}", (v * 1000.0).round() / 1000.0);
    let (sign, digits) = plain.strip_prefix('-').map_or(("", plain.as_str()), |d| ("-", d));
    let (int, frac) = digits.split_once('.').map_or((digits, None), |(i, f)| (i, Some(f)));
    let mut grouped = String::from(sign);
    for (i, c) in int.chars().enumerate() {
        if i > 0 && int.len().saturating_sub(i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    if let Some(f) = frac {
        grouped.push('.');
        grouped.push_str(f);
    }
    grouped
}

/// An x-y chart's geometry as unovis' `VisXYContainer` lays it out: where
/// each row and value lands. Rows are in x order.
struct Plot<'a> {
    chart: &'a ChartXy<'a>,
    /// Where the plot starts, right of the y axis.
    left: f32,
    /// The y of value 0.
    base: f32,
    /// From the plot's top to `base`.
    height: f32,
    /// Pixels per unit of value.
    unit: f32,
    /// The largest value the y axis reaches.
    max: f32,
    /// A bar group's width (`_getGroupWidth`); 0 for areas and lines.
    group: f32,
    /// The x of each row.
    xs: Vec<f32>,
    /// Each tick label's box: as wide as the two closest rows are apart.
    band: f32,
}

impl<'a> Plot<'a> {
    fn new(chart: &'a ChartXy<'a>) -> Self {
        let left = if chart.axes { Y_AXIS } else { 0.0 };
        let base = chart.height - if chart.axes { X_AXIS } else { 0.0 };
        let bars = matches!(chart.mark, ChartMark::Bar { .. });
        // The crosshair's bleed: room for its dot past the axes' margins.
        let dot = if chart.crosshair.is_some() { DOT } else { 0.0 };
        let height = (base - dot).max(0.0);
        let width = (chart.width - left).max(0.0);
        let max = chart
            .rows
            .iter()
            .map(|row| match chart.mark {
                ChartMark::Area => row.values.iter().map(|v| v.max(0.0)).sum(),
                ChartMark::Bar { .. } | ChartMark::Line => row.values.iter().copied().fold(0.0, f32::max),
            })
            .fold(0.0, f32::max);
        let max = if max > 0.0 { max } else { 1.0 };
        let group = match chart.mark {
            // A continuous x scale keeps room for one group more than it has rows.
            ChartMark::Bar { group_padding, .. } if chart.rows.len() >= 2 => {
                (1.0 - group_padding) * width / (count(chart.rows.len()) + 1.0)
            }
            ChartMark::Bar { .. } => width,
            ChartMark::Area | ChartMark::Line => 0.0,
        };
        // The scale's range, inset so the outer groups (or points' ticks) and dots fit.
        let inset = if bars { group / 2.0 } else { EDGE };
        let (from, to) = (left + inset.max(dot - left), left + width - inset.max(dot));
        let first = chart.rows.first().map_or(0.0, |r| r.x);
        let span = chart.rows.last().map_or(0.0, |r| r.x) - first;
        let xs: Vec<f32> = chart
            .rows
            .iter()
            .map(|row| {
                if span > 0.0 {
                    from + (row.x - first) / span * (to - from)
                } else {
                    left + width / 2.0
                }
            })
            .collect();
        let band = xs.windows(2).map(|w| match *w {
            [a, b] => b - a,
            _ => width,
        });
        let band = band.fold(width, f32::min);
        Self {
            chart,
            left,
            base,
            height,
            unit: height / max,
            max,
            group,
            xs,
            band,
        }
    }

    fn y(&self, value: f32) -> f32 {
        self.base - value.max(0.0) * self.unit
    }

    /// The x of each row.
    fn xs(&self) -> impl Iterator<Item = f32> + '_ {
        self.xs.iter().copied()
    }

    /// The span the pointer picks each row in: halfway to its neighbours, as
    /// the crosshair snaps to the nearest row.
    fn columns(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        let mids: Vec<f32> = self
            .xs
            .windows(2)
            .filter_map(|w| match *w {
                [a, b] => Some(f32::midpoint(a, b)),
                _ => None,
            })
            .collect();
        let lefts = std::iter::once(self.left).chain(mids.clone());
        let rights = mids.into_iter().chain(std::iter::once(self.chart.width));
        lefts.zip(rights)
    }

    /// The y of each grid line: d3's ticks from 0 to the top value, twice the
    /// y axis' `num-ticks` (unovis' default: `height^0.85 / 25`, at least 1).
    fn grid(&self) -> Vec<f32> {
        if !self.chart.axes {
            return Vec::new();
        }
        let ticks = self
            .chart
            .y_ticks
            .unwrap_or_else(|| (self.height.powf(0.85) / 25.0).max(1.0));
        let raw = self.max / (2.0 * ticks);
        let power = 10_f32.powf(raw.log10().floor());
        let error = raw / power;
        let factor = if error >= 50_f32.sqrt() {
            10.0
        } else if error >= 10_f32.sqrt() {
            5.0
        } else if error >= 2_f32.sqrt() {
            2.0
        } else {
            1.0
        };
        let step = power * factor;
        std::iter::successors(Some(0.0_f32), |v| Some(v + step))
            .take_while(|v| *v <= self.max * 1.000_001)
            .map(|v| self.y(v))
            .collect()
    }

    /// Each series' value per row, stacked for areas.
    fn series(&self) -> Vec<Vec<(f32, f32)>> {
        let mut floor: Vec<f32> = vec![0.0; self.chart.rows.len()];
        (0..self.chart.config.len())
            .map(|s| {
                self.chart
                    .rows
                    .iter()
                    .zip(floor.iter_mut())
                    .map(|(row, below)| {
                        let v = row.values.get(s).copied().unwrap_or(0.0).max(0.0);
                        let low = *below;
                        if self.chart.mark == ChartMark::Area {
                            *below += v;
                            (low, low + v)
                        } else {
                            (0.0, v)
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn marks(&self) -> Vec<Shape> {
        let series = self.series();
        match self.chart.mark {
            ChartMark::Bar { bar_padding, .. } => self.bars(&series, bar_padding),
            ChartMark::Area | ChartMark::Line => self
                .chart
                .config
                .iter()
                .zip(&series)
                .flat_map(|(s, values)| {
                    let top: Vec<(f32, f32)> = self.xs().zip(values).map(|(x, (_, hi))| (x, self.y(*hi))).collect();
                    let line = curve(
                        top.first().map_or(Shape::path(), |p| Shape::path().move_to(p.0, p.1)),
                        &top,
                    )
                    .stroke(s.color, if self.chart.mark == ChartMark::Area { 1.0 } else { 2.0 });
                    let area = (self.chart.mark == ChartMark::Area).then(|| {
                        let bottom: Vec<(f32, f32)> = self
                            .xs
                            .iter()
                            .zip(values)
                            .rev()
                            .map(|(x, (lo, _))| (*x, self.y(*lo)))
                            .collect();
                        let upper = curve(
                            top.first().map_or(Shape::path(), |p| Shape::path().move_to(p.0, p.1)),
                            &top,
                        );
                        let lower = bottom
                            .first()
                            .map_or(upper.clone(), |p| upper.clone().line_to(p.0, p.1));
                        curve(lower, &bottom).close().fill(s.color.opacity(0.4))
                    });
                    area.into_iter().chain(std::iter::once(line))
                })
                .collect(),
        }
    }

    /// Each group's bars across its width, on a d3 band scale whose inner and
    /// outer padding are both `padding`.
    fn bars(&self, series: &[Vec<(f32, f32)>], padding: f32) -> Vec<Shape> {
        let n = count(series.len());
        let step = self.group / (n - padding + 2.0 * padding).max(1.0);
        let width = step * (1.0 - padding);
        let start = (self.group - step * (n - padding)) / 2.0 - self.group / 2.0;
        self.chart
            .config
            .iter()
            .zip(series)
            .enumerate()
            .flat_map(|(j, (s, values))| {
                self.xs().zip(values).map(move |(x, (_, v))| {
                    bar(x + start + count(j) * step, self.y(*v), width, self.base).fill(s.color)
                })
            })
            .collect()
    }

    /// The crosshair's dots at row `i`.
    fn dots(&self, i: usize) -> Vec<Shape> {
        let x = self.xs().nth(i).unwrap_or(0.0);
        self.chart
            .config
            .iter()
            .zip(self.series())
            .filter_map(|(s, values)| {
                values
                    .get(i)
                    .map(|(_, hi)| Shape::circle(x, self.y(*hi), DOT).fill(s.color))
            })
            .collect()
    }
}

/// A count as a coordinate; past 65535 it stops growing, far beyond any chart's rows.
fn count(n: usize) -> f32 {
    u16::try_from(n).map_or(f32::from(u16::MAX), f32::from)
}

/// A bar from `top` down to `base`, its top corners rounded as unovis'
/// `roundedRectPath`: at most half the bar's width or height.
fn bar(left: f32, top: f32, width: f32, base: f32) -> Shape {
    /// A quarter circle's cubic control distance, per unit of radius.
    const K: f32 = 0.552_284_8;
    let r = BAR_RADIUS.min(width.min(base - top) / 2.0).max(0.0);
    let k = r * (1.0 - K);
    Shape::path()
        .move_to(left, base)
        .line_to(left, top + r)
        .cubic_to(left, top + k, left + k, top, left + r, top)
        .line_to(left + width - r, top)
        .cubic_to(left + width - k, top, left + width, top + k, left + width, top + r)
        .line_to(left + width, base)
        .close()
}

/// `shape`, whose pen is at `points[0]`, continued through the rest of
/// `points` along d3's natural cubic spline (`CurveType.Natural`).
fn curve(shape: Shape, points: &[(f32, f32)]) -> Shape {
    let xs: Vec<f32> = points.iter().map(|p| p.0).collect();
    let ys: Vec<f32> = points.iter().map(|p| p.1).collect();
    match (natural_controls(&xs), natural_controls(&ys)) {
        (Some((x1, x2)), Some((y1, y2))) => points
            .iter()
            .skip(1)
            .zip(x1.iter().zip(&x2).zip(y1.iter().zip(&y2)))
            .fold(shape, |s, (p, ((c1x, c2x), (c1y, c2y)))| {
                s.cubic_to(*c1x, *c1y, *c2x, *c2y, p.0, p.1)
            }),
        _ => points.iter().skip(1).fold(shape, |s, p| s.line_to(p.0, p.1)),
    }
}

/// The two control points of each segment of a natural cubic spline through
/// `k`, on one axis: d3's `controlPoints`, solved with the Thomas algorithm.
/// `None` under three points, where the curve is a line.
fn natural_controls(k: &[f32]) -> Option<(Vec<f32>, Vec<f32>)> {
    let segments = k.len().checked_sub(1).filter(|n| *n >= 2)?;
    let last = segments.saturating_sub(1);
    let rows = k.windows(2).enumerate().filter_map(|(i, w)| match *w {
        [x0, x1] if i == 0 => Some((0.0, 2.0, x0 + 2.0 * x1)),
        [x0, x1] if i == last => Some((2.0, 7.0, 8.0 * x0 + x1)),
        [x0, x1] => Some((1.0, 4.0, 4.0 * x0 + 2.0 * x1)),
        _ => None,
    });
    let mut previous: Option<(f32, f32)> = None;
    let swept: Vec<(f32, f32)> = rows
        .map(|(a, b, r)| {
            let row = previous.map_or((b, r), |(pb, pr)| {
                let m = a / pb;
                (b - m, r - m * pr)
            });
            previous = Some(row);
            row
        })
        .collect();
    let mut next: Option<f32> = None;
    let mut first: Vec<f32> = swept
        .iter()
        .rev()
        .map(|(b, r)| {
            let a = next.map_or(r / b, |n| (r - n) / b);
            next = Some(a);
            a
        })
        .collect();
    first.reverse();
    let mut second: Vec<f32> = k
        .iter()
        .skip(1)
        .zip(first.iter().skip(1))
        .map(|(x, a)| 2.0 * x - a)
        .collect();
    second.push((k.last()? + first.last()?) / 2.0);
    Some((first, second))
}

/// `shape` continued along the circle of radius `r` about `(c, c)` from angle
/// `from` to `to` (radians clockwise from twelve o'clock), in cubic pieces of
/// at most a quarter turn.
fn arc(mut shape: Shape, c: f32, r: f32, from: f32, to: f32) -> Shape {
    let point = |a: f32| (c + r * a.sin(), c - r * a.cos());
    let mut at = from;
    for _ in 0..8 {
        let step = (to - at).clamp(-FRAC_PI_2, FRAC_PI_2);
        if step.abs() < 1e-4 {
            break;
        }
        let k = 4.0 / 3.0 * (step / 4.0).tan() * r;
        let (p0, p1) = (point(at), point(at + step));
        shape = shape.cubic_to(
            p0.0 + k * at.cos(),
            p0.1 + k * at.sin(),
            p1.0 - k * (at + step).cos(),
            p1.1 - k * (at + step).sin(),
            p1.0,
            p1.1,
        );
        at += step;
    }
    shape
}
