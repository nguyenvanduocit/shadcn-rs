//! Aspect ratio (`ui/aspect-ratio`): a full-width box whose height follows a ratio.

use gui::{Div, Styled, div};

/// A box as wide as its container and `ratio` (width / height) as tall;
/// its content fills it with `.size_full()`.
#[must_use]
pub fn aspect_ratio(ratio: f32) -> Div {
    div().named("aspect-ratio").relative().w_full().aspect(ratio)
}
