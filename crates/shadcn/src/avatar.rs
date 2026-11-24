//! Avatar (`ui/avatar`): a round picture of a person, or their initials
//! when there is no picture.
//!
//! The behaviour is `primitives::avatar` (reka's `Avatar`): [`avatar`]
//! holds where its image is; [`avatar_image`] works that out from what the
//! caller holds (a decoded [`gui::ImageSource`], or an [`ImageLoad`] on its way)
//! and shows the picture only once it is decoded, telling the caller each
//! status (`.on_status(…)`, reka's `@loading-status-change`);
//! [`avatar_fallback`] shows while it is not, after its delay if one is
//! given (`.delay(…)`, reka's `delay-ms`; 0 shows it at once). This file
//! draws the parts.

use std::time::Duration;

use gui::prelude::*;
use gui::{Callback, component, div, image, provide};
use primitives::avatar::{Avatar, Fallback, ImageLoad, Status};

use crate::part::{Part, part_root};
use crate::theme::theme;

/// What the parts below [`avatar`] read: its primitive.
#[derive(Clone, Copy)]
struct AvatarContext(Avatar);

/// The avatar's frame; its [`avatar_image`] and [`avatar_fallback`] go inside.
#[must_use]
pub fn avatar() -> Part<()> {
    let root = div()
        .named("avatar")
        .position(Position::Relative)
        .display(Display::Flex)
        .size(32.0)
        .flex_shrink(0.0)
        .overflow(Overflow::Hidden)
        .radius(9999.0);
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(AvatarContext(Avatar::new()));
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

/// What [`avatar_image`] is given.
#[derive(Clone, Debug, PartialEq)]
pub struct AvatarImageProps {
    load: ImageLoad,
    on_status: Option<Callback<Status>>,
}

/// The picture: shown once `load` is decoded, nothing before. A decoded
/// source (`Some(&source)`) or none converts; a load on its way is
/// [`ImageLoad::Pending`].
#[must_use]
#[track_caller]
pub fn avatar_image(load: impl Into<ImageLoad>) -> Part<AvatarImageProps> {
    let props = AvatarImageProps {
        load: load.into(),
        on_status: None,
    };
    let root = div().aspect_ratio(1.0).size(pct(100.0));
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            // Outside an avatar (reported) the picture shows if it is decoded:
            // its look, without the status the avatar keeps.
            let part = part_root::<_, AvatarContext>(props, "avatar_image", "avatar").map(|AvatarContext(avatar)| {
                avatar.image(
                    move || props.with(|p| p.props.load.clone()),
                    move |status| {
                        if let Some(f) = props.with_untracked(|p| p.props.on_status.clone()) {
                            f.call(status);
                        }
                    },
                )
            });
            move || {
                let source = part.map_or_else(|| props.with(|p| p.props.load.image().cloned()), |i| i.source());
                let Some(source) = source else {
                    return gui::Element::Empty;
                };
                // The caller's box is the picture's: its size, corners and
                // filters land on the image node itself.
                let mut slot = props.with(|p| p.slot.0.clone());
                let mut picture = image(source).named("avatar-image");
                *picture.style_mut() = slot.style_mut().clone();
                picture.into_element()
            }
        })
    })
}

impl Part<AvatarImageProps> {
    /// Call `f` with each status the image reaches, the first included
    /// (reka's `@loading-status-change`).
    #[must_use]
    pub fn on_status(self, f: impl Into<Callback<Status>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.on_status = Some(f))
    }
}

/// What [`avatar_fallback`] is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AvatarFallbackProps {
    delay: Option<Duration>,
}

/// The `initials`, shown while the picture is not.
#[must_use]
#[track_caller]
pub fn avatar_fallback(initials: impl Into<String>) -> Part<AvatarFallbackProps> {
    let t = theme();
    let root = div()
        .named("avatar-fallback")
        .background(t.muted)
        .display(Display::Flex)
        .size(pct(100.0))
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .radius(9999.0)
        .child(initials.into());
    Part::new(AvatarFallbackProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            // Outside an avatar (reported) the fallback shows after its delay:
            // its look, without the avatar's picture status.
            let avatar = part_root::<_, AvatarContext>(props, "avatar_fallback", "avatar").map(|AvatarContext(a)| a);
            let fallback = Fallback::new(props.with_untracked(|p| p.props.delay), move || {
                avatar.map_or(Status::Idle, Avatar::status)
            });
            move || {
                if fallback.shown() {
                    props.with(|p| p.slot.0.clone()).into_element()
                } else {
                    gui::Element::Empty
                }
            }
        })
    })
}

impl Part<AvatarFallbackProps> {
    /// Wait `delay` from when the picture is missing (from the start, and
    /// each time a shown picture gives way to one still loading) before
    /// showing the fallback, so a picture that loads quickly never flashes
    /// it (reka's `delay-ms`).
    #[must_use]
    pub fn delay(self, delay: Duration) -> Self {
        self.with_props(|p| p.delay = Some(delay))
    }
}
