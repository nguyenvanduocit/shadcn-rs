//! What the platform says of where the app shows, as signals the UI thread
//! reads (tracked): whether its window can be seen (`visibilitychange`),
//! its size, and whether the user asked for reduced motion
//! (`prefers-reduced-motion`). A carousel pauses its rotation by them.
//! A window's size and visibility are its own (the current window's,
//! [`crate::focus::window_signals`]); reduced motion is the user's, one
//! for the app. The engine owns them: each is written in a commit step
//! (`engine::commit`), never by hand, and a step is part of a turn
//! (`engine::Turn`): the platform's writes go through its window's turn
//! (`Pipeline::told`); the public setters, a test's, open one of the
//! current window when none runs.

use crate::engine::{Owned, Turn, commit, in_turn};

crate::engine::engine_local! {
    [Shared("a handle made live; the value is the reactive arena's, which forks")] static REDUCED: Owned<bool> = Owned::labeled("reduced motion", false);
}

/// The current window's size in logical pixels, once the platform has
/// said (tracked): what a node's box is on screen within.
#[must_use]
pub fn window_size() -> Option<(f32, f32)> {
    crate::focus::window_signals().size.get()
}

/// The current window is `size` logical pixels: part of the turn under
/// way, or a turn of its own.
pub fn set_window_size(size: (f32, f32)) {
    in_turn(|t| put_window_size(t, size));
}

/// The current window is `size` logical pixels, in the turn `t`.
pub(crate) fn put_window_size(t: &Turn, size: (f32, f32)) {
    let s = crate::focus::window_signals().size;
    commit(t, |c| s.put(Some(size), c));
}

/// Whether the current window can be seen: not minimised or covered
/// (tracked).
#[must_use]
pub fn window_visible() -> bool {
    crate::focus::window_signals().visible.get()
}

/// Whether motion is reduced (tracked).
#[must_use]
pub fn reduced_motion() -> bool {
    REDUCED.with(|r| r.get())
}

/// The platform told the current window can (or cannot) be seen: part of
/// the turn under way, or a turn of its own.
pub fn set_window_visible(visible: bool) {
    in_turn(|t| put_window_visible(t, visible));
}

/// The current window can (or cannot) be seen, in the turn `t`.
pub(crate) fn put_window_visible(t: &Turn, visible: bool) {
    let s = crate::focus::window_signals().visible;
    commit(t, |c| s.put(visible, c));
}

/// The app runs with motion reduced (or not): part of the turn under way,
/// or a turn of its own.
pub fn set_reduced_motion(reduced: bool) {
    in_turn(|t| put_reduced_motion(t, reduced));
}

/// The app runs with motion reduced (or not), in the turn `t`.
pub(crate) fn put_reduced_motion(t: &Turn, reduced: bool) {
    let r = REDUCED.with(|r| *r);
    commit(t, |c| r.put(reduced, c));
}
