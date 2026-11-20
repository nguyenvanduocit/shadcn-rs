//! Card (`ui/card`): a bordered surface with header, title, description,
//! action, content and footer slots.

use gui::{Div, IntoElement, Styled, Track, div, text};

use crate::theme::theme;

#[must_use]
pub fn card() -> Div {
    let t = theme();
    div()
        .named("card")
        .bg(t.card)
        .text_color(t.card_foreground)
        .flex_col()
        .gap(6.0)
        .rounded_xl()
        .border()
        .border_color(t.border)
        .py(6.0)
        .shadow_sm()
}

/// The header grid: title and description on the left, an optional
/// [`card_action`] in a column on the right.
#[must_use]
pub fn card_header() -> Div {
    div()
        .named("card-header")
        .grid()
        .auto_rows_min()
        .grid_cols_tracks(vec![Track::Fr(1.0), Track::Auto])
        .items_start()
        .gap(1.5)
        .px(6.0)
}

#[must_use]
pub fn card_title(title: impl Into<String>) -> impl IntoElement {
    text(title)
        .named("card-title")
        .leading_none()
        .font_semibold()
        .col_start(1)
}

#[must_use]
pub fn card_description(description: impl Into<String>) -> impl IntoElement {
    let t = theme();
    text(description)
        .named("card-description")
        .text_color(t.muted_foreground)
        .text_sm()
        .col_start(1)
}

/// Sits top-right in the header, beside title and description.
#[must_use]
pub fn card_action() -> Div {
    div()
        .named("card-action")
        .col_start(2)
        .row_span(2)
        .row_start(1)
        .self_start()
        .justify_self_end()
}

#[must_use]
pub fn card_content() -> Div {
    div().named("card-content").px(6.0)
}

#[must_use]
pub fn card_footer() -> Div {
    div().named("card-footer").flex().items_center().px(6.0)
}
