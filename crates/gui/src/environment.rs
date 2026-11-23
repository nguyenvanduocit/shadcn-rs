//! What the platform says of where the app shows, as signals the UI thread
//! reads (tracked): whether its window can be seen (`visibilitychange`),
//! and whether the user asked for reduced motion
//! (`prefers-reduced-motion`). A carousel pauses its rotation by them.

use crate::reactive::{Signal, detached};

crate::engine::engine_local! {
    [Shared("a handle made live; the value is the reactive arena's, which forks")] static VISIBLE: Signal<bool> = detached(|| Signal::labeled("window visible", true));
    [Shared("a handle made live; the value is the reactive arena's, which forks")] static REDUCED: Signal<bool> = detached(|| Signal::labeled("reduced motion", false));
    [Shared("a handle made live; the value is the reactive arena's, which forks")] static SIZE: Signal<Option<(f32, f32)>> = detached(|| Signal::labeled("window size", None));
}

/// The window's size in logical pixels, once the platform has said
/// (tracked): what a node's box is on screen within.
#[must_use]
pub fn window_size() -> Option<(f32, f32)> {
    SIZE.with(Signal::get)
}

/// The window is `size` logical pixels.
pub fn set_window_size(size: (f32, f32)) {
    SIZE.with(|s| {
        if s.get_untracked() != Some(size) {
            s.set(Some(size));
        }
    });
}

/// Whether the window can be seen: not minimised or covered (tracked).
#[must_use]
pub fn window_visible() -> bool {
    VISIBLE.with(Signal::get)
}

/// Whether motion is reduced (tracked).
#[must_use]
pub fn reduced_motion() -> bool {
    REDUCED.with(Signal::get)
}

/// The platform told the window can (or cannot) be seen.
pub fn set_window_visible(visible: bool) {
    VISIBLE.with(|s| {
        if s.get_untracked() != visible {
            s.set(visible);
        }
    });
}

/// The app runs with motion reduced (or not).
pub fn set_reduced_motion(reduced: bool) {
    REDUCED.with(|s| {
        if s.get_untracked() != reduced {
            s.set(reduced);
        }
    });
}
