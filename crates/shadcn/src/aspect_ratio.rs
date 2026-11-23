//! Aspect ratio (`ui/aspect-ratio`): a full-width box whose height follows a ratio.
//!
//! The behaviour is `primitives::aspect_ratio` (reka's `AspectRatio`): the
//! box is as wide as its container and its width over the ratio tall; what
//! goes in fills it and never sizes it.

use gui::{component, div};
use primitives::aspect_ratio::frame;

use crate::part::Part;

/// What [`aspect_ratio`] is given: width over height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AspectRatioProps {
    ratio: f32,
}

/// A box as wide as its container and `ratio` (width / height) as tall;
/// its content fills it with `.size(pct(100.0))`.
#[must_use]
pub fn aspect_ratio(ratio: f32) -> Part<AspectRatioProps> {
    Part::new(AspectRatioProps { ratio }, div().named("aspect-ratio"), |p| {
        component(p, |cx| {
            let props = cx.props();
            move || props.with(|p| frame(p.props.ratio, p.slot.0.clone()))
        })
    })
}
