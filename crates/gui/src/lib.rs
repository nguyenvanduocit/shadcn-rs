//! A native GUI framework in Rust: fine-grained reactive state, an element tree
//! styled with Tailwind's vocabulary, Taffy layout, cosmic-text text, and a
//! tiny-skia CPU painter shown through softbuffer.

pub mod app;
mod cache;
pub mod clipboard;
pub mod clock;
pub mod color;
pub mod computed;
pub mod context;
mod convert;
pub mod edit;
mod effects;
pub mod element;
pub mod events;
pub mod focus;
pub mod geometry;
pub mod heap;
pub mod icon;
pub mod image;
pub mod inspect;
pub mod interact;
pub mod layout;
pub mod motion;
pub mod overlay;
pub mod paint;
mod placement;
pub mod reactive;
pub mod scroll;
pub mod shape;
pub mod style;
pub mod styled;
pub mod text;
pub mod time_input;
pub mod window;

pub use app::{DebugTools, RunOptions, Scenario, ScenarioError, run, snapshot};
pub use color::Color;
pub use computed::Computed;
pub use context::{provide, use_context};
pub use edit::{Preedit, TextEdit, TextField};
pub use element::{
    Deferred, Div, Element, ElementKey, IconElement, Image, InputMode, InputType, IntoElement, Placement, Shapes, Side,
    SideAlign, Span, Text, TextInput, View, deferred, div, icon, image, key_of, shapes, span, text, text_input,
};
pub use events::{ClickEvent, DragEvent, DragPhase, ImeEvent, Key, KeyEvent, Modifiers, NamedKey};
pub use focus::FocusHandle;
pub use geometry::Bounds;
pub use icon::Icon;
pub use image::ImageSource;
pub use motion::{Animation, Easing, Motion, Properties};
pub use reactive::{Effect, Signal, batch};
pub use scroll::{ScrollChild, ScrollHandle, ScrollLayout};
pub use shape::{PathCommand, Shape};
pub use style::{Cursor, Direction, Length, Radius, Shadow, Style, Track};
pub use styled::{Fluent, Styled, px, relative, units};
pub use time_input::{TimeField, time_input};

/// The traits a component file needs in scope.
pub mod prelude {
    pub use crate::element::IntoElement;
    pub use crate::styled::{Fluent, Styled};
}
