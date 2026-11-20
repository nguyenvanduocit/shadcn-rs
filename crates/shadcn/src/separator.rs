//! Separator (`ui/separator`): a 1px rule.

use gui::{Div, Styled, div};

use crate::theme::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

#[must_use]
pub fn separator(orientation: Orientation) -> Div {
    let t = theme();
    let d = div().named("separator").bg(t.border).shrink_0();
    match orientation {
        Orientation::Horizontal => d.h_px(1.0).w_full(),
        Orientation::Vertical => d.h_full().w_px(1.0),
    }
}
