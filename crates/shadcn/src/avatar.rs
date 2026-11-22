//! Avatar (`ui/avatar`): a round picture of a person, or their initials
//! when there is no picture.
//!
//! reka shows `AvatarImage` once the image has loaded and `AvatarFallback`
//! until then. Here an image is loaded when the caller holds a decoded
//! [`ImageSource`], so both parts take that `Option`: exactly one renders.

use gui::prelude::*;
use gui::{Div, Image, ImageSource, div, image};

use crate::theme::theme;

#[must_use]
pub fn avatar() -> Div {
    div()
        .named("avatar")
        .position(Position::Relative)
        .display(Display::Flex)
        .size(32.0)
        .flex_shrink(0.0)
        .overflow(Overflow::Hidden)
        .radius(9999.0)
}

/// The picture, when there is one.
#[must_use]
pub fn avatar_image(source: Option<&ImageSource>) -> Option<Image> {
    source.map(|s| {
        image(s.clone())
            .named("avatar-image")
            .aspect_ratio(1.0)
            .size(pct(100.0))
    })
}

/// The `initials`, when there is no picture.
#[must_use]
pub fn avatar_fallback(source: Option<&ImageSource>, initials: impl Into<String>) -> Option<Div> {
    let t = theme();
    source.is_none().then(|| {
        div()
            .named("avatar-fallback")
            .background(t.muted)
            .display(Display::Flex)
            .size(pct(100.0))
            .align_items(Align::Center)
            .justify_content(Justify::Center)
            .radius(9999.0)
            .child(initials.into())
    })
}
