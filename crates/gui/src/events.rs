//! What a handler receives: where the pointer was, which key, which modifiers.

pub use winit::keyboard::{Key, NamedKey};

use crate::geometry::Bounds;

/// Modifier keys held during an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    /// ⌘ on macOS, the Windows key elsewhere.
    pub command: bool,
}

impl Modifiers {
    /// The platform's shortcut modifier: ⌘ on macOS, Ctrl elsewhere.
    #[must_use]
    pub fn primary(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.command
        } else {
            self.control
        }
    }
}

/// A press or click of the primary button.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClickEvent {
    pub x: f32,
    pub y: f32,
    /// The box of the node whose handler runs.
    pub bounds: Bounds,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragPhase {
    Start,
    Move,
    End,
}

/// The pointer pressed on a node and moving until release.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragEvent {
    pub x: f32,
    pub y: f32,
    /// The box of the dragged node, as of the last frame.
    pub bounds: Bounds,
    pub phase: DragPhase,
}

/// A key press.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyEvent {
    pub key: Key,
    /// The text the key types, if any.
    pub text: Option<String>,
    pub modifiers: Modifiers,
}

/// What an input method does to the focused text input (winit's `Ime`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeEvent {
    /// Composing: show this text at the caret; empty ends the composition
    /// with nothing typed.
    Preedit(crate::edit::Preedit),
    /// Composed: type this text.
    Commit(String),
}

impl KeyEvent {
    /// Whether this is the named key `k`.
    #[must_use]
    pub fn is(&self, k: NamedKey) -> bool {
        self.key == Key::Named(k)
    }
}
