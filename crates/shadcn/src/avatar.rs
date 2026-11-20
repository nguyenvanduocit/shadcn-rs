//! Avatar (`ui/avatar`): a round picture of a person, or their initials
//! when there is no picture.
//!
//! reka shows `AvatarImage` once the image has loaded and `AvatarFallback`
//! until then. Here an image is loaded when the caller holds a decoded
//! [`ImageSource`], so both parts take that `Option`: exactly one renders.

use gui::{Div, Image, ImageSource, Styled, div, image};

use crate::theme::theme;

#[must_use]
pub fn avatar() -> Div {
    div()
        .named("avatar")
        .relative()
        .flex()
        .size(8.0)
        .shrink_0()
        .overflow_hidden()
        .rounded_full()
}

/// The picture, when there is one.
#[must_use]
pub fn avatar_image(source: Option<&ImageSource>) -> Option<Image> {
    source.map(|s| image(s.clone()).named("avatar-image").aspect_square().size_full())
}

/// The `initials`, when there is no picture.
#[must_use]
pub fn avatar_fallback(source: Option<&ImageSource>, initials: impl Into<String>) -> Option<Div> {
    let t = theme();
    source.is_none().then(|| {
        div()
            .named("avatar-fallback")
            .bg(t.muted)
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .rounded_full()
            .child(initials.into())
    })
}
