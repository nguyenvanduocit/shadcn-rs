//! Card (`ui/card`): a bordered surface with header, title, description,
//! action, content and footer slots.

use gui::prelude::*;
use gui::{Color, Div, IntoElement, Track, div, text};

use crate::theme::theme;

#[must_use]
pub fn card() -> Div {
    let t = theme();
    div()
        .named("card")
        .background(t.card)
        .text_color(t.card_foreground)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(24.0)
        .radius(14.0)
        .border(1.0)
        .border_color(t.border)
        .padding_y(24.0)
        .shadow([
            Shadow {
                x: 0.0,
                y: 1.0,
                blur: 3.0,
                spread: 0.0,
                color: Color::BLACK.opacity(0.1),
            },
            Shadow {
                x: 0.0,
                y: 1.0,
                blur: 2.0,
                spread: -1.0,
                color: Color::BLACK.opacity(0.1),
            },
        ])
}

/// The header grid: title and description on the left, an optional
/// [`card_action`] in a column on the right.
#[must_use]
pub fn card_header() -> Div {
    div()
        .named("card-header")
        .display(Display::Grid)
        .grid_auto_rows(Track::MinContent)
        .grid_columns(vec![Track::Fr(1.0), Track::Auto])
        .align_items(Align::Start)
        .gap(6.0)
        .padding_x(24.0)
}

#[must_use]
pub fn card_title(title: impl Into<String>) -> impl IntoElement {
    text(title)
        .named("card-title")
        .line_height(LineHeight::Relative(1.0))
        .font_weight(600)
        .grid_column_start(1)
}

#[must_use]
pub fn card_description(description: impl Into<String>) -> impl IntoElement {
    let t = theme();
    text(description)
        .named("card-description")
        .text_color(t.muted_foreground)
        .text_size(14.0)
        .line_height(20.0)
        .grid_column_start(1)
}

/// Sits top-right in the header, beside title and description.
#[must_use]
pub fn card_action() -> Div {
    div()
        .named("card-action")
        .grid_column_start(2)
        .grid_row_span(2)
        .grid_row_start(1)
        .align_self(Align::Start)
        .justify_self(Align::End)
}

#[must_use]
pub fn card_content() -> Div {
    div().named("card-content").padding_x(24.0)
}

#[must_use]
pub fn card_footer() -> Div {
    div()
        .named("card-footer")
        .display(Display::Flex)
        .align_items(Align::Center)
        .padding_x(24.0)
}
