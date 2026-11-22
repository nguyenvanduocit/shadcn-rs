//! Aspect ratio (`ui/aspect-ratio`): a full-width box whose height follows a ratio.

use gui::prelude::*;
use gui::{Div, div};

/// A box as wide as its container and `ratio` (width / height) as tall;
/// its content fills it with `.size(pct(100.0))`.
#[must_use]
pub fn aspect_ratio(ratio: f32) -> Div {
    div()
        .named("aspect-ratio")
        .position(Position::Relative)
        .width(pct(100.0))
        .aspect_ratio(ratio)
}
