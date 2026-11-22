//! Scroll Area (`ui/scroll-area`): a box that scrolls its content, with a
//! thin overlay scrollbar.
//!
//! `gui` scrolls any `overflow-*-scroll` node and paints its thumb, so the
//! root and its viewport are one node here. The thumb's look (width, colour,
//! shown only on hover) and a horizontal bar are `gui`'s, not reka's.

use gui::prelude::*;
use gui::{Div, Properties, div};

/// A vertically scrolling area; give it a height and put the content in with `.child`.
#[must_use]
pub fn scroll_area() -> Div {
    div()
        .named("scroll-area")
        .transition(Properties::COLOR | Properties::SHADOW)
        .position(Position::Relative)
        .overflow_y(Overflow::Scroll)
        .overflow_x(Overflow::Hidden)
}

/// An area with a horizontal scrollbar too (`<ScrollBar orientation="horizontal" />`).
#[must_use]
pub fn scroll_area_horizontal() -> Div {
    div()
        .named("scroll-area")
        .transition(Properties::COLOR | Properties::SHADOW)
        .position(Position::Relative)
        .overflow(Overflow::Scroll)
}
