//! Label (`ui/label`): the name of a form control.

use gui::prelude::*;
use gui::{Div, Styled, div, text};

use crate::focus::StateLook;

/// A label; `.disabled(true)` dims it beside a disabled control, as
/// `peer-disabled:opacity-50` and `group-data-[disabled=true]:opacity-50` do.
#[must_use]
pub fn label(name: impl Into<String>) -> Div {
    div()
        .named("label")
        .with_interaction(|d, i| d.dim_disabled(i, 0.5).cursor_disabled(i))
        .display(Display::Flex)
        .align_items(Align::Center)
        .gap(8.0)
        .text_size(14.0)
        .line_height(20.0)
        .line_height(LineHeight::Relative(1.0))
        .font_weight(500)
        .child(text(name))
}
