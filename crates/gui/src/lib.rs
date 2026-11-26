//! A native GUI framework in Rust: fine-grained reactive state, an element tree
//! whose properties take values or closures, Taffy layout, Parley text, and a
//! scene of primitives painted on the GPU (wgpu), in a window and headless.

pub mod access;
pub mod app;
mod cache;
pub mod canvas;
pub mod chunk;
mod clip;
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
#[doc(hidden)]
pub mod engine;
pub mod environment;
pub mod events;
pub mod figure;
pub mod focus;
pub mod geometry;
pub mod gesture;
pub mod gpu;
pub mod heap;
pub mod holder;
pub mod icon;
pub mod image;
pub mod inspect;
pub mod instructions;
pub mod interact;
pub mod layer;
pub mod layout;
pub mod list;
pub mod mailbox;
pub mod members;
pub mod motion;
pub mod overlay;
pub mod paint;
pub mod place;
pub mod placement;
pub mod port;
pub mod priority;
pub mod process;
pub mod random;
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
pub mod value;
pub mod window;

pub use access::NodeRef;
pub use app::{DebugTools, RunOptions, Scenario, ScenarioError, offscreen, run, snapshot};
pub use color::Color;
pub use component::{
    Bind, Callback, ComponentElement, Ctx, Derive, Model, Owner, Priority, Root, Shared, Slot, UseModel, after_layout,
    component, idle, on_mounted, on_unmounted, on_updated, provide, use_context, use_model, with_priority,
};
pub use edit::{Preedit, TextEdit, TextField};
pub use element::{
    Announce, AriaCurrent, AutoComplete, Deferred, Div, Drawn, Element, ElementKey, Figures, HasPopup, IconElement,
    Image, InputMode, InputType, IntoElement, Live, Positioning, Relations, Role, Semantics, Side, SideAlign, Span,
    Text, TextInput, Toggled, ValueRange, View, deferred, div, drawing, figures, icon, image, key_of, span, text,
    text_bound, text_input,
};
pub use events::{
    CaptureGroup, ClickEvent, DragEvent, DragPhase, ImeEvent, Key, KeyEvent, Modifiers, NamedKey, PointerEvent,
    PointerId, TouchEvent, TouchPhase, WheelEvent,
};
pub use figure::{Cap, Dash, Draw, Figure, FillRule, Join, Path, PathBuilder, PathStyle, Stroke};
pub use focus::{AutoFocus, FocusHandle, FocusScope, Modality};
pub use geometry::Bounds;
pub use holder::{Holder, Keyed, Registration, Registry, while_mounted};
pub use icon::{Icon, SvgIcon};
pub use image::{ImageSource, Pixels, PixelsError, demultiply};
pub use interact::Interaction;
pub use layer::{
    Dismiss, DismissPolicy, DismissReason, Layer, LayerHandle, PointerButton, PointerKind, PointerOutside,
};
pub use list::{ListState, virtual_list};
pub use mailbox::{Closed, Sender, mailbox};
pub use members::Members;
pub use motion::{Animation, Easing, Motion, Properties, Slide, Stage};
pub use place::Place;
pub use placement::Placement;
pub use port::{Cancel, Port, Task, port, spawn};
pub use reactive::{
    Collection, Computed, Effect, Scope, Signal, batch, detached, on_cleanup, trigger, untracked, watch,
};
pub use scroll::{ScrollChild, ScrollHandle, ScrollLayout};
pub use shape::{Backdrop, Blend, Decoration, EdgeBar, InsetLayer, OverlayBorder, Shape};
pub use style::{
    Align, Axis, BackgroundImage, Calc, ContentStyle, Corners, Cursor, Direction, Display, Edges, Filter, Filters,
    Flex, FlexDirection, FontFamily, GenericFamily, GridAutoFlow, GridSpan, Justify, Length, LineHeight,
    LinearGradient, MaskImage, Origin, Overflow, Position, Ring, RingCorners, ScrollFade, Shadow, Shadows, Stop,
    StopColor, Stops, Style, TextAlign, TextStyle, Track, Tracks, WrapStyle, pct, px,
};
pub use styled::{Fluent, Styled};

/// The traits a component file needs in scope, and the values its setters take.
pub mod prelude {
    pub use crate::element::IntoElement;
    pub use crate::interact::Interaction;
    pub use crate::motion::{Animation, Easing, Properties, Slide};
    pub use crate::shape::{Backdrop, Blend, EdgeBar, InsetLayer, OverlayBorder, Shape};
    pub use crate::style::{
        Align, Axis, BackgroundImage, Calc, ContentStyle, Corners, Cursor, Direction, Display, Edges, Filter, Flex,
        FlexDirection, FontFamily, GridAutoFlow, GridSpan, Justify, Length, LineHeight, LinearGradient, MaskImage,
        Origin, Overflow, Position, Ring, RingCorners, ScrollFade, Shadow, Shadows, Stop, StopColor, Stops, TextAlign,
        TextStyle, Track, Tracks, WrapStyle, pct, px,
    };
    pub use crate::styled::{Fluent, Styled};
}
