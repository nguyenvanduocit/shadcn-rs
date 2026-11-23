//! Content for assistive technology that is not on screen (reka's
//! `VisuallyHidden`).
//!
//! - [`Hidden::Visually`] (reka's default): read, and it names its parent
//!   (an icon button's text); it keeps whatever focus it takes. It takes no
//!   room, so nothing around it moves or scrolls, and paints nothing.
//! - [`Hidden::Fully`]: neither shown nor read, and no Tab stop.
//!
//! reka clips a 1 px box at a negative margin to do this; here the node is
//! laid out out of the flow with no size and clips its content, which the
//! accessibility tree still reads.

use gui::Div;
use gui::prelude::*;

/// How far a node is hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Hidden {
    /// Not on screen, still read.
    #[default]
    Visually,
    /// Not on screen, not read, no Tab stop.
    Fully,
}

/// `d` hidden as `how` says.
#[must_use]
pub fn visually_hidden(d: Div, how: Hidden) -> Div {
    let d = d
        .position(Position::Absolute)
        .width(0.0)
        .height(0.0)
        .overflow(Overflow::Hidden)
        .nowrap(true);
    match how {
        Hidden::Visually => d,
        Hidden::Fully => d.aria_hidden().tab_stop(false),
    }
}
