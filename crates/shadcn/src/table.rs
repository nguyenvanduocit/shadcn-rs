//! Table (`ui/table`): rows of cells under a header, with a footer and a caption.
//!
//! `gui` has no table layout, where every row shares column widths found from
//! all cells; each row is a grid on the columns the caller states, so `Fr` and
//! `Px` tracks line up across rows and an `Auto` track does not. A table of
//! many rows takes a [`Columns`] model instead (an `Auto` column as wide as
//! the widest cell seen, only growing) and a [`table_body_virtual`], which
//! builds only the rows in view (ADR-0014): a million rows scroll as ten do.
//!
//! A screen reader hears a table as `<table>` is heard: a table of row
//! groups, rows, column headers and cells; a virtual body says how many
//! rows it has in all and each row where it is among them (ADR-0021).

use gui::list::{Columns, ListState, virtual_list};
use gui::prelude::*;
use gui::{Div, ElementKey, Track, div, text};

use crate::focus::StateLook;
use crate::theme::theme;

/// The table (`<table>` in its `overflow-auto` container): `w-full text-sm`,
/// caption last as `caption-bottom` puts it.
#[must_use]
pub fn table() -> Div {
    div()
        .named("table")
        .role(gui::Role::Table)
        .position(Position::Relative)
        .width(pct(100.0))
        .overflow_x(Overflow::Scroll)
        .overflow_y(Overflow::Hidden)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .text_size(14.0)
        .line_height(20.0)
}

/// `<thead>`: every row keeps its bottom border.
#[must_use]
pub fn table_header(rows: impl IntoIterator<Item = Div>) -> Div {
    div()
        .named("table-header")
        .role(gui::Role::RowGroup)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .children(rows)
}

/// `<tbody>`: the last row has no border (`[&_tr:last-child]:border-0`).
#[must_use]
pub fn table_body(rows: impl IntoIterator<Item = Div>) -> Div {
    div()
        .named("table-body")
        .role(gui::Role::RowGroup)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .children(without_last_border(rows))
}

/// `<tbody>` of `rows` rows, `row(i)` built only for those in view and the
/// prefetch margin around them, each keyed `key(i)`: a scroll container of
/// its own, so give it a height (or `flex_1` in a column). The last row has
/// no border, as in [`table_body`]. Lay each row's cells on one
/// [`Columns`] ([`table_row_on`]) so they line up.
#[must_use]
pub fn table_body_virtual(
    state: &ListState,
    rows: u64,
    key: impl Fn(u64) -> ElementKey,
    row: impl Fn(u64) -> Div,
) -> Div {
    virtual_list(state, rows, key, move |i| {
        row(i)
            .when(i.saturating_add(1) == rows, |e| e.border(0.0))
            .into_element()
    })
    .named("table-body")
    .role(gui::Role::RowGroup)
}

/// `<tr>` of `cells` on the column model `columns`; `selected` is `data-state=selected`.
#[must_use]
pub fn table_row_on<E: IntoElement>(columns: &Columns, selected: bool, cells: impl IntoIterator<Item = E>) -> Div {
    table_row(&columns.tracks(), selected).children(columns.cells(cells))
}

/// `<tfoot>`: muted, bordered on top, its last row without a bottom border.
#[must_use]
pub fn table_footer(rows: impl IntoIterator<Item = Div>) -> Div {
    let t = theme();
    div()
        .named("table-footer")
        .role(gui::Role::RowGroup)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .background(t.muted.opacity(0.5))
        .border_top(1.0)
        .border_color(t.border)
        .font_weight(500)
        .children(without_last_border(rows))
}

fn without_last_border(rows: impl IntoIterator<Item = Div>) -> Vec<Div> {
    let rows: Vec<Div> = rows.into_iter().collect();
    let last = rows.len().checked_sub(1);
    rows.into_iter()
        .enumerate()
        .map(|(i, row)| row.when(Some(i) == last, |e| e.border(0.0)))
        .collect()
}

/// `<tr>` on `columns`; `selected` is `data-state=selected`.
#[must_use]
pub fn table_row(columns: &[Track], selected: bool) -> Div {
    let t = theme();
    div()
        .named("table-row")
        .role(gui::Role::Row)
        .when(selected, |d| d.aria_selected(true))
        .transition(Properties::COLORS)
        .display(Display::Grid)
        .grid_columns(columns.to_vec())
        .border_bottom(1.0)
        .border_color(t.border)
        .with_interaction(|d, i| {
            if selected {
                d.background(t.muted)
            } else {
                d.background_while(move || i.hovered(), t.muted.opacity(0.5))
            }
        })
}

/// `<th>`: a column heading, left-aligned until a class says otherwise.
#[must_use]
pub fn table_head(content: impl IntoElement) -> Div {
    let t = theme();
    div()
        .named("table-head")
        .role(gui::Role::ColumnHeader)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .justify_content(Justify::Center)
        .height(40.0)
        .padding_x(8.0)
        .text_align(TextAlign::Left)
        .text_color(t.foreground)
        .font_weight(500)
        .nowrap(true)
        .child(content)
}

/// `<td>`.
#[must_use]
pub fn table_cell(content: impl IntoElement) -> Div {
    div()
        .named("table-cell")
        .role(gui::Role::Cell)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .justify_content(Justify::Center)
        .padding(8.0)
        .nowrap(true)
        .child(content)
}

/// A row with one cell across every column, for a table with no rows.
#[must_use]
pub fn table_empty(columns: &[Track], content: impl IntoElement) -> Div {
    let t = theme();
    let span = u16::try_from(columns.len()).unwrap_or(u16::MAX);
    table_row(columns, false).child(
        table_cell(
            div()
                .display(Display::Flex)
                .align_items(Align::Center)
                .justify_content(Justify::Center)
                .padding_y(40.0)
                .child(content),
        )
        .grid_column_span(span)
        .padding(16.0)
        .text_size(14.0)
        .line_height(20.0)
        .text_color(t.foreground),
    )
}

/// `<caption>`: centred under the table.
#[must_use]
pub fn table_caption(caption: impl Into<String>) -> impl IntoElement {
    let t = theme();
    text(caption)
        .named("table-caption")
        .margin_top(16.0)
        .text_size(14.0)
        .line_height(20.0)
        .text_align(TextAlign::Center)
        .text_color(t.muted_foreground)
}
