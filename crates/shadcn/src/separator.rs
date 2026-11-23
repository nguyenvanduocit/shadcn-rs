//! Separator (`ui/separator`): a 1px rule, decorative as shadcn-vue's is
//! by default (`primitives::separator`: nothing to a screen reader).

use gui::prelude::*;
use gui::{Axis, Div, div};
use primitives::separator::Separator;

use crate::theme::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

impl Orientation {
    /// The axis it runs along.
    #[must_use]
    pub const fn axis(self) -> Axis {
        match self {
            Self::Horizontal => Axis::Horizontal,
            Self::Vertical => Axis::Vertical,
        }
    }
}

#[must_use]
pub fn separator(orientation: Orientation) -> Div {
    let t = theme();
    let d = Separator::new(orientation.axis(), true).apply(div().named("separator"));
    let d = d.background(t.border).flex_shrink(0.0);
    match orientation {
        Orientation::Horizontal => d.height(1.0).width(pct(100.0)),
        Orientation::Vertical => d.height(pct(100.0)).width(1.0),
    }
}
