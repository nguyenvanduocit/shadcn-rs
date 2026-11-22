//! Separator (`ui/separator`): a 1px rule.

use gui::prelude::*;
use gui::{Div, div};

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
    let d = div().named("separator").background(t.border).flex_shrink(0.0);
    match orientation {
        Orientation::Horizontal => d.height(1.0).width(pct(100.0)),
        Orientation::Vertical => d.height(pct(100.0)).width(1.0),
    }
}
