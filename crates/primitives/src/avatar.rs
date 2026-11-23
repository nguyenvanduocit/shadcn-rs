//! A picture of a person, or what stands in for it while there is none
//! (reka's `Avatar`: `AvatarRoot`, `AvatarImage`, `AvatarFallback`).
//!
//! **Status.** Where the image is ([`Status`]), held by the root
//! ([`Avatar`]): idle until an image part says otherwise, never back to
//! idle once it has (`Avatar/AvatarRoot.vue:21-29`,
//! `Avatar/AvatarImage.vue:36-44`). The image part ([`Avatar::image`])
//! works it out from what the caller holds ([`ImageLoad`]), and nobody
//! writes it (`Avatar/utils.ts:7-18`, `:40-66`): no source is an error; an
//! image decoded already is loaded at once, without a loading step
//! between, so a cached picture never flashes its fallback; one on its way
//! is loading, then loaded or an error; a new source is worked out again.
//! reka loads an `Image` outside the document to learn this and hides the
//! `<img>` until it has; here the caller decodes off the UI thread (a
//! [`gui::mailbox`] carries the result), and a part shows only what is
//! decoded, so a broken or half-drawn picture is never shown. The image
//! part tells its caller each status from the first
//! (`Avatar/AvatarImage.vue:36-44`).
//!
//! **Fallback.** Shown while the image is not loaded, once its delay, if
//! any, has passed since the image last stopped being loaded: from the
//! fallback's mount, and again each time a loaded image gives way to one
//! that is not (`Avatar/AvatarFallback.vue:24-36` counts from the mount
//! alone, so a second load flashes it). reka never shows a fallback whose
//! delay is 0 (its timer runs only for a delay other than 0, and "show at
//! once" only when none is set); here 0 shows it at once (research §4 item
//! 13).
//!
//! **To assistive technology.** The image is an image (`role=img`, which
//! `gui::Image` is); the root and fallback have no role.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gui::{ImageSource, Signal};

use crate::hover::Delay;

/// Where an avatar's image is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// No image part has said yet.
    Idle,
    /// On its way.
    Loading,
    /// Decoded: the image shows, the fallback goes.
    Loaded,
    /// There is none, or it could not be loaded: the fallback stays.
    Error,
}

impl Status {
    /// Whether the image shows.
    #[must_use]
    pub const fn loaded(self) -> bool {
        matches!(self, Self::Loaded)
    }
}

/// What the caller holds of an avatar's image.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ImageLoad {
    /// No source.
    #[default]
    Missing,
    /// Asked for, not decoded yet.
    Pending,
    /// Decoded.
    Ready(ImageSource),
    /// It could not be loaded or decoded.
    Failed,
}

impl ImageLoad {
    /// The status this makes.
    #[must_use]
    pub const fn status(&self) -> Status {
        match self {
            Self::Missing | Self::Failed => Status::Error,
            Self::Pending => Status::Loading,
            Self::Ready(_) => Status::Loaded,
        }
    }

    /// The image to show: only a decoded one.
    #[must_use]
    pub const fn image(&self) -> Option<&ImageSource> {
        match self {
            Self::Ready(source) => Some(source),
            Self::Missing | Self::Pending | Self::Failed => None,
        }
    }
}

/// A source the caller decoded already, or none.
impl From<Option<&ImageSource>> for ImageLoad {
    fn from(source: Option<&ImageSource>) -> Self {
        source.map_or(Self::Missing, |s| Self::Ready(s.clone()))
    }
}

/// An avatar's state: made once in its root's setup, `Copy`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Avatar {
    status: Signal<Status>,
}

impl Avatar {
    /// An avatar whose image is idle until an image part says. Called in
    /// the root's setup.
    #[must_use]
    #[track_caller]
    pub fn new() -> Self {
        Self {
            status: Signal::labeled("avatar status", Status::Idle),
        }
    }

    /// Where its image is (tracked): what the fallback and the look read.
    #[must_use]
    pub fn status(self) -> Status {
        self.status.get()
    }

    /// The image part of the image `load` gives (read tracked), telling
    /// `on_status` each status it reaches, the first included. Called in
    /// the image part's setup.
    #[must_use]
    #[track_caller]
    pub fn image(
        self,
        load: impl Fn() -> ImageLoad + 'static,
        on_status: impl Into<gui::Callback<Status>>,
    ) -> AvatarImage {
        let on_status = on_status.into();
        let load: Rc<dyn Fn() -> ImageLoad> = Rc::new(load);
        let me = AvatarImage {
            load: Signal::new(Rc::clone(&load)),
        };
        // From the first value on: the root learns it before the first
        // frame, so a decoded picture never shows its fallback.
        let told: Rc<std::cell::Cell<Option<Status>>> = Rc::default();
        gui::Effect::new(move || {
            let now = load().status();
            if told.replace(Some(now)) != Some(now) {
                gui::untracked(|| {
                    if self.status.is_alive() {
                        self.status.set(now);
                    }
                    on_status.call(now);
                });
            }
        });
        me
    }
}

impl Default for Avatar {
    fn default() -> Self {
        Self::new()
    }
}

/// An avatar's image part: made once in its setup, `Copy`.
#[derive(Clone, Copy, PartialEq)]
pub struct AvatarImage {
    load: Signal<Rc<dyn Fn() -> ImageLoad>>,
}

impl std::fmt::Debug for AvatarImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvatarImage").finish_non_exhaustive()
    }
}

impl AvatarImage {
    /// The image to show, once it is decoded (tracked): none hides the
    /// part, so a broken or half-drawn picture never shows.
    #[must_use]
    pub fn source(self) -> Option<ImageSource> {
        let load = self.load.get_untracked();
        load().image().cloned()
    }
}

/// A fallback's state: made once in the fallback's setup.
#[derive(Clone, Copy, PartialEq)]
pub struct Fallback {
    status: Signal<Rc<dyn Fn() -> Status>>,
    waited: Signal<bool>,
}

impl std::fmt::Debug for Fallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fallback").finish_non_exhaustive()
    }
}

impl Fallback {
    /// A fallback for the image whose status `status` reads (tracked: the
    /// root's), shown once `delay` has passed (none, or 0: at once) since
    /// the image last stopped being loaded: from the fallback's mount, and
    /// again each time a loaded image gives way to another that is not.
    /// reka counts only from the mount (its `canRender` stays once set), so
    /// a picture that loaded, then a new source on its way, flashes the
    /// fallback at once; the delay is there to keep a quick load from
    /// flashing it, so here it counts afresh (research §4 item 13).
    #[must_use]
    #[track_caller]
    pub fn new(delay: Option<Duration>, status: impl Fn() -> Status + 'static) -> Self {
        let wait = delay.filter(|d| !d.is_zero());
        let status: Rc<dyn Fn() -> Status> = Rc::new(status);
        let me = Self {
            status: Signal::new(Rc::clone(&status)),
            waited: Signal::labeled("avatar fallback waited", wait.is_none()),
        };
        let Some(delay) = wait else {
            return me;
        };
        let timer = Delay::new();
        let last: Rc<Cell<Option<bool>>> = Rc::default();
        gui::Effect::new(move || {
            let loaded = status().loaded();
            if last.replace(Some(loaded)) == Some(loaded) {
                return;
            }
            gui::untracked(|| {
                if me.waited.get() {
                    me.waited.set(false);
                }
                if loaded {
                    timer.cancel();
                } else {
                    timer.after(delay, move || {
                        if me.waited.is_alive() {
                            me.waited.set(true);
                        }
                    });
                }
            });
        });
        me
    }

    /// Whether it shows (tracked): the image is not loaded, and the delay
    /// has passed.
    #[must_use]
    pub fn shown(self) -> bool {
        let status = self.status.get_untracked();
        !status().loaded() && self.waited.get()
    }
}
