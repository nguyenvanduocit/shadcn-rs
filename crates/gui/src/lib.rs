//! A native GUI framework in Rust: fine-grained reactive state, an element tree
//! styled with Tailwind's vocabulary, Taffy layout, Parley text, and a
//! scene of primitives painted on the GPU (wgpu), in a window and headless.

pub mod access;
pub mod app;
mod cache;
pub mod canvas;
pub mod chunk;
pub mod clipboard;
pub mod clock;
pub mod color;
pub mod component;
pub mod compositor;
mod convert;
pub mod devtools;
pub mod edit;
mod effects;
pub mod element;
pub mod events;
pub mod focus;
pub mod geometry;
pub mod gpu;
pub mod heap;
pub mod icon;
pub mod image;
pub mod inspect;
pub mod instructions;
pub mod interact;
pub mod layout;
pub mod list;
pub mod mailbox;
pub mod members;
pub mod motion;
pub mod overlay;
pub mod paint;
mod placement;
pub mod priority;
pub mod process;
pub mod reactive;
pub mod scene;
pub mod scroll;
pub mod shape;
pub mod style;
pub mod styled;
pub mod sum_tree;
mod svg;
pub mod testing;
pub mod text;
pub mod time_input;
pub mod window;

pub use app::{DebugTools, RunOptions, Scenario, ScenarioError, offscreen, run, snapshot};
pub use color::Color;
pub use component::{
    Bind, Callback, ComponentElement, Ctx, Model, Priority, Root, Shared, Slot, UseModel, after_layout, component,
    idle, on_mounted, on_unmounted, on_updated, provide, use_context, use_model, with_priority,
};
pub use edit::{Preedit, TextEdit, TextField};
pub use element::{
    Deferred, Div, Drawn, Element, ElementKey, IconElement, Image, InputMode, InputType, IntoElement, Placement, Role,
    Semantics, Shapes, Side, SideAlign, Span, Text, TextInput, Toggled, View, deferred, div, drawing, icon, image,
    key_of, shapes, span, text, text_bound, text_input,
};
pub use events::{
    ClickEvent, DragEvent, DragPhase, ImeEvent, Key, KeyEvent, Modifiers, NamedKey, PointerEvent, WheelEvent,
};
pub use focus::FocusHandle;
pub use geometry::Bounds;
pub use icon::Icon;
pub use image::{ImageSource, Pixels, PixelsError, demultiply};
pub use list::{ListState, virtual_list};
pub use mailbox::{Sender, mailbox};
pub use members::Members;
pub use motion::{Animation, Easing, Motion, Properties};
pub use reactive::{
    Collection, Computed, Effect, Scope, Signal, batch, detached, on_cleanup, trigger, untracked, watch,
};
pub use scroll::{ScrollChild, ScrollHandle, ScrollLayout};
pub use shape::{Cap, Dash, Draw, FillRule, Join, Path, PathBuilder, PathStyle, Shape, Stroke};
pub use style::{Cursor, Direction, Length, Radius, Shadow, Style, Track};
pub use styled::{Fluent, Styled, px, relative, units};
pub use time_input::{TimeInput, time_input};

/// The traits a component file needs in scope.
pub mod prelude {
    pub use crate::element::IntoElement;
    pub use crate::styled::{Fluent, Styled};
}
