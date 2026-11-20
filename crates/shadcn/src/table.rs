//! Table (`ui/table`): rows of cells under a header, with a footer and a caption.
//!
//! `gui` has no table layout, where every row shares column widths found from
//! all cells; each row is a grid on the columns the caller states, so `Fr` and
//! `Px` tracks line up across rows and an `Auto` track does not.

use gui::prelude::*;
use gui::{Div, Track, div, text};

use crate::theme::theme;

/// The table (`<table>` in its `overflow-auto` container): `w-full text-sm`,
/// caption last as `caption-bottom` puts it.
#[must_use]
pub fn table() -> Div {
    div()
        .named("table")
        .relative()
        .w_full()
        .overflow_x_scroll()
        .flex_col()
        .text_sm()
}

/// `<thead>`: every row keeps its bottom border.
#[must_use]
pub fn table_header(rows: impl IntoIterator<Item = Div>) -> Div {
    div().named("table-header").flex_col().children(rows)
}

/// `<tbody>`: the last row has no border (`[&_tr:last-child]:border-0`).
#[must_use]
pub fn table_body(rows: impl IntoIterator<Item = Div>) -> Div {
    div().named("table-body").flex_col().children(without_last_border(rows))
}

/// `<tfoot>`: muted, bordered on top, its last row without a bottom border.
#[must_use]
pub fn table_footer(rows: impl IntoIterator<Item = Div>) -> Div {
    let t = theme();
    div()
        .named("table-footer")
        .flex_col()
        .bg(t.muted.opacity(0.5))
        .border_t()
        .border_color(t.border)
        .font_medium()
        .children(without_last_border(rows))
}

fn without_last_border(rows: impl IntoIterator<Item = Div>) -> Vec<Div> {
    let rows: Vec<Div> = rows.into_iter().collect();
    let last = rows.len().checked_sub(1);
    rows.into_iter()
        .enumerate()
        .map(|(i, row)| row.when(Some(i) == last, Styled::border_0))
        .collect()
}

/// `<tr>` on `columns`; `selected` is `data-state=selected`.
#[must_use]
pub fn table_row(columns: &[Track], selected: bool) -> Div {
    let t = theme();
    div()
        .named("table-row")
        .transition_colors()
        .grid()
        .grid_cols_tracks(columns.to_vec())
        .border_b()
        .border_color(t.border)
        .hover(move |s| s.bg(t.muted.opacity(0.5)))
        .when(selected, |d| d.bg(t.muted).hover(move |s| s.bg(t.muted)))
}

/// `<th>`: a column heading, left-aligned until a class says otherwise.
#[must_use]
pub fn table_head(content: impl IntoElement) -> Div {
    let t = theme();
    div()
        .named("table-head")
        .flex_col()
        .justify_center()
        .h(10.0)
        .px(2.0)
        .text_left()
        .text_color(t.foreground)
        .font_medium()
        .whitespace_nowrap()
        .child(content)
}

/// `<td>`.
#[must_use]
pub fn table_cell(content: impl IntoElement) -> Div {
    div()
        .named("table-cell")
        .flex_col()
        .justify_center()
        .p(2.0)
        .whitespace_nowrap()
        .child(content)
}

/// A row with one cell across every column, for a table with no rows.
#[must_use]
pub fn table_empty(columns: &[Track], content: impl IntoElement) -> Div {
    let t = theme();
    let span = u16::try_from(columns.len()).unwrap_or(u16::MAX);
    table_row(columns, false).child(
        table_cell(div().flex().items_center().justify_center().py(10.0).child(content))
            .col_span(span)
            .p(4.0)
            .text_sm()
            .text_color(t.foreground),
    )
}

/// `<caption>`: centred under the table.
#[must_use]
pub fn table_caption(caption: impl Into<String>) -> impl IntoElement {
    let t = theme();
    text(caption)
        .named("table-caption")
        .mt(4.0)
        .text_sm()
        .text_center()
        .text_color(t.muted_foreground)
}
