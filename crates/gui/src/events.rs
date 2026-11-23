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

    /// The modifier that moves the caret and deletes by words: ⌥ on macOS,
    /// Ctrl elsewhere.
    #[must_use]
    pub fn word(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.alt
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

/// The pointer moving over a node: where (window pixels, as a click's) and
/// the node's box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    pub x: f32,
    pub y: f32,
    pub bounds: Bounds,
}

/// The wheel (or a trackpad's scroll) over a node: where the pointer is
/// (window pixels), how far (pixels, down and right positive), the node's box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelEvent {
    pub x: f32,
    pub y: f32,
    pub dx: f32,
    pub dy: f32,
    pub bounds: Bounds,
}

/// Where a drag is: it starts, moves, and then ends or is cancelled, once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragPhase {
    Start,
    Move,
    /// The button or the finger was released: the drag does what it was for.
    End,
    /// The drag stopped without a release (`pointercancel`): the dragged
    /// node became disabled, by itself or by an ancestor. Nothing follows;
    /// the handler settles without acting on the release (no fling, no
    /// dismiss, no commit beyond what moves already wrote).
    Cancel,
}

impl DragPhase {
    /// It is the drag's last phase: [`Self::End`] or [`Self::Cancel`].
    #[must_use]
    pub const fn is_last(self) -> bool {
        matches!(self, Self::End | Self::Cancel)
    }
}

/// Which pointer an input comes from, as Chrome's `pointerId`: the mouse,
/// or one touch or pen contact (the id its [`TouchEvent`]s carry).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointerId {
    Mouse,
    Contact(u64),
}

/// The pointer pressed on a node and moving until release.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragEvent {
    pub x: f32,
    pub y: f32,
    /// The box of the dragged node, as of the last frame.
    pub bounds: Bounds,
    pub phase: DragPhase,
    /// Which pointer drags: a node is captured by one pointer at a time,
    /// from its `Start` to its `End` or `Cancel` (another pointer pressing
    /// it meanwhile drags nothing), unless it takes several
    /// ([`crate::Div::multi_pointer`]).
    pub pointer: PointerId,
    /// What pressed: a mouse, a pen or a finger.
    pub kind: crate::layer::PointerKind,
    /// The press went down on a control inside the dragged node (a node
    /// that takes clicks, a text input, or a button, link, checkbox, radio,
    /// switch, slider, combobox or label), not on the node itself: a
    /// drawer's swipe leaves such a press to it when it is a mouse's.
    pub on_control: bool,
}

/// Nodes a pointer captures together ([`crate::Div::capture_group`]): the
/// pointer that starts dragging or touching one holds them all until it
/// lets go, and another pointer pressing any of them meanwhile drags or
/// touches nothing there (a splitter family's groups, which one crossing
/// drags together).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CaptureGroup(u64);

impl Default for CaptureGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureGroup {
    /// A group of its own.
    #[must_use]
    pub fn new() -> Self {
        crate::engine::engine_local! {
            [Fork] static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
        }
        Self(NEXT.with(|n| {
            let id = n.get();
            n.set(id.wrapping_add(1));
            id
        }))
    }
}

/// Where a finger or a pen is in its contact with the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    /// The platform took it away (a system gesture began).
    Cancelled,
}

/// A finger or a pen on the screen (winit's `Touch`): which contact, what
/// it is, where (window pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchEvent {
    pub id: u64,
    pub kind: crate::layer::PointerKind,
    pub phase: TouchPhase,
    pub x: f32,
    pub y: f32,
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
