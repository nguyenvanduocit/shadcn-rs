//! Keyboard focus: which focusable element receives keys.
//!
//! A component creates a [`FocusHandle`] once (it is part of the component's
//! state, like a signal) and attaches it to an element with `track_focus`.
//! Which handle holds focus is a signal, so reading it re-renders on change.

use std::cell::Cell;

use crate::reactive::Signal;

thread_local! {
    static NEXT: Cell<u64> = const { Cell::new(1) };
    static FOCUSED: Signal<Option<FocusId>> = Signal::labeled("focus", None);
    /// Whether the focus last moved by keyboard, which is when `focus-visible` shows.
    static VISIBLE: Signal<bool> = Signal::labeled("focus-visible", false);
}

/// The identity of one focusable element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FocusId(u64);

/// A focus target that stays the same across frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FocusHandle {
    id: FocusId,
}

impl Default for FocusHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl FocusHandle {
    #[must_use]
    pub fn new() -> Self {
        let id = NEXT.with(|n| {
            let id = n.get();
            n.set(id.wrapping_add(1));
            id
        });
        Self { id: FocusId(id) }
    }

    /// The handle of a node focusable by its place in the tree.
    #[must_use]
    pub fn for_path(path: &crate::geometry::NodePath) -> Self {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut h);
        // The top bit keeps these apart from counted handles.
        Self {
            id: FocusId(h.finish() | (1 << 63)),
        }
    }

    #[must_use]
    pub fn id(self) -> FocusId {
        self.id
    }

    /// Whether this handle holds focus (reactive).
    #[must_use]
    pub fn is_focused(self) -> bool {
        focused() == Some(self.id)
    }

    /// Give this handle focus, as a pointer would (no visible focus ring).
    pub fn focus(self) {
        set_focus(Some(self.id), false);
    }

    /// Drop focus if this handle holds it.
    pub fn blur(self) {
        if self.is_focused() {
            set_focus(None, false);
        }
    }
}

/// The focused element, if any (reactive).
#[must_use]
pub fn focused() -> Option<FocusId> {
    FOCUSED.with(Signal::get)
}

/// Whether the current focus was reached by keyboard (reactive).
#[must_use]
pub fn focus_visible() -> bool {
    VISIBLE.with(Signal::get)
}

/// Move focus to `id`; `by_keyboard` decides whether focus rings show.
pub fn set_focus(id: Option<FocusId>, by_keyboard: bool) {
    if focused() != id {
        FOCUSED.with(|f| f.set(id));
    }
    if focus_visible() != by_keyboard {
        VISIBLE.with(|v| v.set(by_keyboard));
    }
}
