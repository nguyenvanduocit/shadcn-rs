//! A box whose height follows its width (reka's `AspectRatio`).
//!
//! The box is as wide as its container and as tall as that width over the
//! ratio, 1 unless the caller says otherwise (`AspectRatio/AspectRatio.vue:22-24`,
//! `:40-41`). Its content fills it and does not size it: content larger
//! than the box overflows it, neither pushing it taller nor clipped
//! (`:40-49`). reka gets there with a `padding-bottom` percentage and an
//! absolutely placed child; here the box takes Taffy's `aspect-ratio` and
//! its content goes in one absolutely placed layer filling it, so it lays
//! out as one box whatever is inside (research §12, "Dễ sót": CSS's
//! `aspect-ratio` alone lets content grow a box with `min-height: auto`).
//! It has nothing to tell assistive technology.

use gui::prelude::*;
use gui::{Div, Element, IntoElement, div};

/// The ratio of width to height unless the caller says otherwise.
pub const RATIO: f32 = 1.0;

/// `d`, the look's box, framed to `ratio` (width over height): its children
/// move into a layer filling it, so they never size it.
#[must_use]
pub fn frame(ratio: f32, d: Div) -> Element {
    let mut outer = d
        .position(Position::Relative)
        .width(pct(100.0))
        .aspect_ratio(ratio)
        .into_element();
    let content = outer.children_mut().map(std::mem::take).unwrap_or_default();
    let fill = div()
        .named("aspect-ratio-content")
        .position(Position::Absolute)
        .top(0.0)
        .right(0.0)
        .bottom(0.0)
        .left(0.0)
        .children(content);
    if let Some(kids) = outer.children_mut() {
        kids.push(fill.into_element());
    }
    outer
}
