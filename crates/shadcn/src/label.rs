//! Label (`ui/label`): the name of a form control.

use gui::{Div, Styled, div, text};

/// A label; `.disabled(true)` dims it beside a disabled control, as
/// `peer-disabled:opacity-50` and `group-data-[disabled=true]:opacity-50` do.
#[must_use]
pub fn label(name: impl Into<String>) -> Div {
    div()
        .named("label")
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed())
        .flex()
        .items_center()
        .gap(2.0)
        .text_sm()
        .leading_none()
        .font_medium()
        .child(text(name))
}
