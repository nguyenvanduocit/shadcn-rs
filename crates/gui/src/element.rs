//! The element tree a component returns each frame.
//!
//! Every node kind shares [`Common`]: its style, the style refinements for
//! hover, press and focus, its event handlers and its focus handle. What
//! differs is what it holds: children ([`Div`]), a paragraph ([`Text`],
//! whose [`Span`]s style parts of it inline), an
//! [`Icon`], or an editable [`TextField`] ([`TextInput`]). [`Deferred`]
//! lifts a subtree onto a layer above the rest (popovers, dialogs, menus).

use std::ops::Range;
use std::rc::Rc;

use crate::color::Color;
use crate::edit::{TextEdit, TextField};
use crate::events::{ClickEvent, DragEvent, KeyEvent};
use crate::focus::FocusHandle;
use crate::icon::Icon;
use crate::image::ImageSource;
use crate::scroll::ScrollHandle;
use crate::shape::Shape;
use crate::style::Style;
use crate::styled::Styled;

/// Stable identity for a node inside a dynamic sibling list.
pub type ElementKey = u64;

/// A key from any string, for lists keyed by name.
#[must_use]
pub fn key_of(s: &str) -> ElementKey {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// A style change applied while a node is in some state.
pub type Refine = Rc<dyn Fn(Style) -> Style>;

/// A handler for an event of type `E`.
pub type Handler<E> = Rc<dyn Fn(&E)>;

/// A key handler: returns whether it handled the key; unhandled keys go to the parent.
pub type KeyHandler = Rc<dyn Fn(&KeyEvent) -> bool>;

/// What a node does when the pointer or keyboard acts on it.
#[derive(Clone, Default)]
pub struct Handlers {
    pub(crate) click: Option<Handler<ClickEvent>>,
    pub(crate) mouse_down: Option<Handler<ClickEvent>>,
    pub(crate) drag: Option<Handler<DragEvent>>,
    pub(crate) key_down: Option<KeyHandler>,
    pub(crate) secondary_click: Option<Handler<ClickEvent>>,
    pub(crate) hover: Option<Rc<dyn Fn(bool)>>,
    pub(crate) click_outside: Option<Rc<dyn Fn()>>,
}

/// The style refinements for each interactive state.
#[derive(Clone, Default)]
pub struct States {
    pub(crate) has_icon: Option<Refine>,
    pub(crate) hover: Option<Refine>,
    pub(crate) active: Option<Refine>,
    pub(crate) focus: Option<Refine>,
    pub(crate) focus_visible: Option<Refine>,
    pub(crate) disabled: Option<Refine>,
}

/// What every node kind has.
#[derive(Clone, Default)]
pub struct Common {
    pub(crate) key: Option<ElementKey>,
    /// A name the debug tools show for this node.
    pub(crate) name: Option<&'static str>,
    pub(crate) style: Style,
    pub(crate) states: States,
    pub(crate) handlers: Handlers,
    pub(crate) focus: Option<FocusHandle>,
    /// Focusable by its place in the tree, as a `<button>` is.
    pub(crate) focusable: bool,
    pub(crate) disabled: bool,
    /// Clicks pass through this node to whatever is below it.
    pub(crate) pass_through: bool,
    /// Holds this scroll container's offset.
    pub(crate) scroll: Option<ScrollHandle>,
}

/// One node kind plus what all kinds share.
#[derive(Clone)]
pub struct Node<K> {
    pub(crate) common: Common,
    pub(crate) kind: K,
}

/// A box of children.
pub type Div = Node<Vec<Element>>;
/// A paragraph: text wrapped as one, parts of it styled by [`Span`]s.
pub type Text = Node<TextSpec>;
/// A lucide icon, drawn in the inherited text colour.
pub type IconElement = Node<IconSpec>;
/// An editable text box.
pub type TextInput = Node<InputSpec>;
/// Vector shapes drawn in the node's box.
pub type Shapes = Node<Vec<Shape>>;
/// A raster image covering the node's box.
pub type Image = Node<ImageSource>;

/// A paragraph's text and the spans styled inside it.
#[derive(Clone, Default)]
pub struct TextSpec {
    pub(crate) content: String,
    pub(crate) spans: Vec<SpanSpec>,
}

/// A [`Span`] placed in its paragraph.
#[derive(Clone)]
pub(crate) struct SpanSpec {
    pub(crate) range: Range<usize>,
    pub(crate) style: Style,
    pub(crate) click: Option<Handler<ClickEvent>>,
}

/// Text inside a paragraph in an inline style of its own, wrapping with the
/// rest of it (`<a>`, `<code>`, `<strong>`, a highlighted token). Of its style
/// it honours the inherited text properties (colour, size, weight, family,
/// italic, underline, line-through, tracking), a background with its corner
/// radius and padding, and the cursor; layout properties have no effect.
#[derive(Clone)]
pub struct Span {
    content: String,
    style: Style,
    click: Option<Handler<ClickEvent>>,
}

impl crate::styled::Fluent for Span {}

impl Styled for Span {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Span {
    /// Called on a click on the span's text (a link).
    #[must_use]
    pub fn on_click(mut self, f: impl Fn(&ClickEvent) + 'static) -> Self {
        self.click = Some(Rc::new(f));
        self
    }
}

impl From<&str> for Span {
    fn from(content: &str) -> Self {
        span(content)
    }
}

impl From<String> for Span {
    fn from(content: String) -> Self {
        span(content)
    }
}

/// Text to append to a paragraph in a style of its own; see [`Text::span`].
#[must_use]
pub fn span(content: impl Into<String>) -> Span {
    Span {
        content: content.into(),
        style: Style::default(),
        click: None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconSpec {
    pub icon: Icon,
    pub stroke_width: f32,
}

#[derive(Clone)]
pub struct InputSpec {
    pub(crate) field: TextField,
    pub(crate) placeholder: String,
    pub(crate) placeholder_color: Option<Color>,
    pub(crate) selection_color: Option<Color>,
    pub(crate) multiline: bool,
    pub(crate) mode: InputMode,
}

/// What a text input holds (`<input type>`), which decides how its value shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputType {
    #[default]
    Text,
    /// A bullet per character (`type="password"`); its value is never copied
    /// out, and the frame, the inspector and the debug dumps hold only bullets.
    Password,
}

/// How a text input shows and takes its value: its `type` and `readonly`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputMode {
    pub input_type: InputType,
    /// Focusable, selectable and copyable, but never edited, and no caret shows (`readonly`).
    pub read_only: bool,
}

impl InputMode {
    /// `edit` as the input shows it.
    pub(crate) fn shown(self, edit: TextEdit) -> TextEdit {
        match self.input_type {
            InputType::Text => edit,
            InputType::Password => edit.masked(),
        }
    }

    /// Where an offset `at` in what the input shows of `edit` falls in its value.
    pub(crate) fn value_offset(self, edit: &TextEdit, at: usize) -> usize {
        match self.input_type {
            InputType::Text => at,
            InputType::Password => edit.unmasked(at),
        }
    }
}

/// Which side of its anchor a deferred layer opens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// How a deferred layer lines up with its anchor along the other axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideAlign {
    Start,
    Center,
    End,
}

/// Where a deferred layer goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    /// Next to the parent of the [`Deferred`] element, flipped and kept on screen.
    Anchored {
        side: Side,
        align: SideAlign,
        /// Gap between anchor and layer.
        offset: f32,
        /// Make the layer at least as wide as the anchor.
        match_width: bool,
        /// Make the layer at most as tall as the room on its side, to the
        /// window's edge (reka's `--reka-*-content-available-height`, which
        /// select and menu contents take as their `max-h`).
        fit_height: bool,
    },
    /// Over the anchor, moved so the keyed node `part` inside the layer lies
    /// on the keyed node `anchor_part` inside the anchor: their left edges
    /// line up, and `part`'s middle sits on the anchor's middle. At least as
    /// wide as the anchor to its right edge, kept on screen, cut to the
    /// window's height; a layer whose root keeps its offset in a
    /// [`crate::ScrollHandle`] opens scrolled so `part` stays over the anchor
    /// when the room above runs out (reka's select `position="item-aligned"`).
    Over { part: ElementKey, anchor_part: ElementKey },
    /// At a point in the window (a context menu at the pointer).
    At { x: f32, y: f32 },
    /// Covering the whole window (a dialog and its scrim, a sheet, toasts).
    Fill,
}

/// A subtree drawn on its own layer above everything laid out before it.
#[derive(Clone)]
pub struct Deferred {
    pub(crate) placement: Placement,
    pub(crate) child: Element,
}

/// A node in the UI tree.
#[derive(Clone, Default)]
pub enum Element {
    Div(Box<Div>),
    Text(Box<Text>),
    Icon(Box<IconElement>),
    Input(Box<TextInput>),
    Shapes(Box<Shapes>),
    Image(Box<Image>),
    Deferred(Box<Deferred>),
    #[default]
    Empty,
}

impl Element {
    pub(crate) fn common(&self) -> Option<&Common> {
        match self {
            Self::Div(n) => Some(&n.common),
            Self::Text(n) => Some(&n.common),
            Self::Icon(n) => Some(&n.common),
            Self::Input(n) => Some(&n.common),
            Self::Shapes(n) => Some(&n.common),
            Self::Image(n) => Some(&n.common),
            Self::Deferred(_) | Self::Empty => None,
        }
    }

    pub(crate) fn common_mut(&mut self) -> Option<&mut Common> {
        match self {
            Self::Div(n) => Some(&mut n.common),
            Self::Text(n) => Some(&mut n.common),
            Self::Icon(n) => Some(&mut n.common),
            Self::Input(n) => Some(&mut n.common),
            Self::Shapes(n) => Some(&mut n.common),
            Self::Image(n) => Some(&mut n.common),
            Self::Deferred(_) | Self::Empty => None,
        }
    }

    #[must_use]
    pub fn key(&self) -> Option<ElementKey> {
        self.common().and_then(|c| c.key)
    }

    /// The node's own style, for a parent that styles its children by
    /// position (CSS `& > *`); `None` for a layer or nothing.
    pub fn style_mut(&mut self) -> Option<&mut Style> {
        match self {
            Self::Div(n) => Some(&mut n.common.style),
            Self::Text(n) => Some(&mut n.common.style),
            Self::Icon(n) => Some(&mut n.common.style),
            Self::Input(n) => Some(&mut n.common.style),
            Self::Shapes(n) => Some(&mut n.common.style),
            Self::Image(n) => Some(&mut n.common.style),
            Self::Deferred(_) | Self::Empty => None,
        }
    }

    /// The name [`Node::named`] gave the node.
    #[must_use]
    pub fn name(&self) -> Option<&'static str> {
        self.common().and_then(|c| c.name)
    }

    /// A box's children, for a caller that reshapes a tree a component built
    /// (a docs page lifting each named demo out of its page); `None` for every
    /// other kind.
    pub fn children_mut(&mut self) -> Option<&mut Vec<Self>> {
        match self {
            Self::Div(n) => Some(&mut n.kind),
            _ => None,
        }
    }
}

/// Anything that can stand where an element goes.
pub trait IntoElement {
    fn into_element(self) -> Element;
}

impl IntoElement for Element {
    fn into_element(self) -> Element {
        self
    }
}

impl IntoElement for Div {
    fn into_element(self) -> Element {
        Element::Div(Box::new(self))
    }
}

impl IntoElement for Text {
    fn into_element(self) -> Element {
        Element::Text(Box::new(self))
    }
}

impl IntoElement for IconElement {
    fn into_element(self) -> Element {
        Element::Icon(Box::new(self))
    }
}

impl IntoElement for TextInput {
    fn into_element(self) -> Element {
        Element::Input(Box::new(self))
    }
}

impl IntoElement for Shapes {
    fn into_element(self) -> Element {
        Element::Shapes(Box::new(self))
    }
}

impl IntoElement for Image {
    fn into_element(self) -> Element {
        Element::Image(Box::new(self))
    }
}

impl IntoElement for Deferred {
    fn into_element(self) -> Element {
        Element::Deferred(Box::new(self))
    }
}

impl IntoElement for &str {
    fn into_element(self) -> Element {
        text(self).into_element()
    }
}

impl IntoElement for String {
    fn into_element(self) -> Element {
        text(self).into_element()
    }
}

impl<T: IntoElement> IntoElement for Option<T> {
    fn into_element(self) -> Element {
        self.map_or(Element::Empty, IntoElement::into_element)
    }
}

impl<K> crate::styled::Fluent for Node<K> {}

impl<K> Styled for Node<K> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.common.style
    }
}

impl<K> Node<K> {
    fn new(kind: K, style: Style) -> Self {
        Self {
            common: Common {
                style,
                ..Common::default()
            },
            kind,
        }
    }

    /// A stable identity among dynamic siblings.
    #[must_use]
    pub fn key(mut self, key: ElementKey) -> Self {
        self.common.key = Some(key);
        self
    }

    /// A name the debug tools show for this node.
    #[must_use]
    pub fn named(mut self, name: &'static str) -> Self {
        self.common.name = Some(name);
        self
    }

    /// Style while the pointer is over this node or a child.
    #[must_use]
    pub fn hover(mut self, f: impl Fn(Style) -> Style + 'static) -> Self {
        self.common.states.hover = Some(Rc::new(f));
        self
    }

    /// Style while the primary button is held down on this node.
    #[must_use]
    pub fn active(mut self, f: impl Fn(Style) -> Style + 'static) -> Self {
        self.common.states.active = Some(Rc::new(f));
        self
    }

    /// Style while this node's focus handle holds focus.
    #[must_use]
    pub fn focus(mut self, f: impl Fn(Style) -> Style + 'static) -> Self {
        self.common.states.focus = Some(Rc::new(f));
        self
    }

    /// Style while focused by keyboard (and, for text inputs, by any means), as
    /// CSS `:focus-visible`.
    #[must_use]
    pub fn focus_visible(mut self, f: impl Fn(Style) -> Style + 'static) -> Self {
        self.common.states.focus_visible = Some(Rc::new(f));
        self
    }

    /// Style while a direct child is an icon (CSS `:has(> svg)`, shadcn's
    /// `has-[>svg]:`): known only once the caller has added the children.
    #[must_use]
    pub fn has_icon(mut self, f: impl Fn(Style) -> Style + 'static) -> Self {
        self.common.states.has_icon = Some(Rc::new(f));
        self
    }

    /// Style while disabled (CSS `:disabled`).
    #[must_use]
    pub fn disabled_style(mut self, f: impl Fn(Style) -> Style + 'static) -> Self {
        self.common.states.disabled = Some(Rc::new(f));
        self
    }

    /// Make this node focusable by its place in the tree, in Tab order; for
    /// focus that must survive the node moving, use [`Self::track_focus`].
    #[must_use]
    pub fn focusable(mut self) -> Self {
        self.common.focusable = true;
        self
    }

    /// Make this node focusable through `handle`, in Tab order.
    #[must_use]
    pub fn track_focus(mut self, handle: FocusHandle) -> Self {
        self.common.focus = Some(handle);
        self
    }

    /// Keep this scroll container's offset in `handle`, which reads its
    /// layout and moves it.
    #[must_use]
    pub fn track_scroll(mut self, handle: &ScrollHandle) -> Self {
        self.common.scroll = Some(handle.clone());
        self
    }

    /// A disabled node takes no clicks, keys or focus.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.common.disabled = disabled;
        self
    }

    /// Let clicks through to what is below (`pointer-events: none`).
    #[must_use]
    pub fn pass_through(mut self) -> Self {
        self.common.pass_through = true;
        self
    }

    /// Called on a click (press and release on this node); bubbles to the
    /// nearest ancestor that has one.
    #[must_use]
    pub fn on_click(mut self, f: impl Fn(&ClickEvent) + 'static) -> Self {
        self.common.handlers.click = Some(Rc::new(f));
        self
    }

    /// Called when the primary button goes down on this node.
    #[must_use]
    pub fn on_mouse_down(mut self, f: impl Fn(&ClickEvent) + 'static) -> Self {
        self.common.handlers.mouse_down = Some(Rc::new(f));
        self
    }

    /// Called as the pointer is pressed on this node, moved, and released.
    #[must_use]
    pub fn on_drag(mut self, f: impl Fn(&DragEvent) + 'static) -> Self {
        self.common.handlers.drag = Some(Rc::new(f));
        self
    }

    /// Called with each key pressed while focus is inside this node; return
    /// `true` when handled.
    #[must_use]
    pub fn on_key_down(mut self, f: impl Fn(&KeyEvent) -> bool + 'static) -> Self {
        self.common.handlers.key_down = Some(Rc::new(f));
        self
    }

    /// Called when the secondary (right) button goes down on this node; bubbles.
    #[must_use]
    pub fn on_secondary_click(mut self, f: impl Fn(&ClickEvent) + 'static) -> Self {
        self.common.handlers.secondary_click = Some(Rc::new(f));
        self
    }

    /// Called with `true` when the pointer enters this node and `false` when it leaves.
    #[must_use]
    pub fn on_hover(mut self, f: impl Fn(bool) + 'static) -> Self {
        self.common.handlers.hover = Some(Rc::new(f));
        self
    }

    /// Called when the primary button goes down outside this node and outside
    /// its layer's anchor (dismissing a popover).
    #[must_use]
    pub fn on_click_outside(mut self, f: impl Fn() + 'static) -> Self {
        self.common.handlers.click_outside = Some(Rc::new(f));
        self
    }
}

impl Div {
    #[must_use]
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.kind.push(child.into_element());
        self
    }

    #[must_use]
    pub fn children<E: IntoElement>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.kind.extend(children.into_iter().map(IntoElement::into_element));
        self
    }
}

impl Text {
    /// Append `span` to the paragraph: plain text, or a [`span`] styled on its own.
    #[must_use]
    pub fn span(mut self, span: impl Into<Span>) -> Self {
        let span = span.into();
        let start = self.kind.content.len();
        self.kind.content.push_str(&span.content);
        if span.style != Style::default() || span.click.is_some() {
            self.kind.spans.push(SpanSpec {
                range: start..self.kind.content.len(),
                style: span.style,
                click: span.click,
            });
        }
        self
    }

    /// Append each of `spans`, in order.
    #[must_use]
    pub fn spans<S: Into<Span>>(self, spans: impl IntoIterator<Item = S>) -> Self {
        spans.into_iter().fold(self, Self::span)
    }
}

impl IconElement {
    /// Line width in the icon's 24-unit grid (lucide's default is 2).
    #[must_use]
    pub fn stroke_width(mut self, width: f32) -> Self {
        self.kind.stroke_width = width;
        self
    }
}

impl TextInput {
    #[must_use]
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.kind.placeholder = text.into();
        self
    }

    #[must_use]
    pub fn placeholder_color(mut self, color: Color) -> Self {
        self.kind.placeholder_color = Some(color);
        self
    }

    #[must_use]
    pub fn selection_color(mut self, color: Color) -> Self {
        self.kind.selection_color = Some(color);
        self
    }

    /// Wrap lines and take Enter as a newline (a textarea).
    #[must_use]
    pub fn multiline(mut self) -> Self {
        self.kind.multiline = true;
        self
    }

    /// What the input holds (`type`); a [`InputType::Password`] shows bullets.
    #[must_use]
    pub fn input_type(mut self, input_type: InputType) -> Self {
        self.kind.mode.input_type = input_type;
        self
    }

    /// Let the value be selected and copied but not edited (`readonly`).
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.kind.mode.read_only = true;
        self
    }
}

/// A box (`<div>`): block layout until `.flex()` or `.grid()`.
#[must_use]
pub fn div() -> Div {
    Node::new(Vec::new(), Style::default())
}

/// A paragraph in the inherited text style; [`Text::span`] adds more to it.
#[must_use]
pub fn text(content: impl Into<String>) -> Text {
    Node::new(
        TextSpec {
            content: content.into(),
            spans: Vec::new(),
        },
        Style::default(),
    )
}

/// A lucide icon, 16px (`size-4`) and not shrinking, as shadcn sizes them.
#[must_use]
pub fn icon(icon: Icon) -> IconElement {
    Node::new(
        IconSpec {
            icon,
            stroke_width: 2.0,
        },
        Style::default(),
    )
    .shrink_0()
}

/// A text box editing `field`, focusable through the field's handle.
#[must_use]
pub fn text_input(field: &TextField) -> TextInput {
    let focus = field.focus;
    Node::new(
        InputSpec {
            field: field.clone(),
            placeholder: String::new(),
            placeholder_color: None,
            selection_color: None,
            multiline: false,
            mode: InputMode::default(),
        },
        Style::default(),
    )
    .track_focus(focus)
    .cursor_text()
    // A browser's own `input, textarea { text-align: start }`: a field
    // starts its text at its start edge in a centred parent too.
    .text_left()
}

/// Vector `shapes` drawn in the node's box (give it a size).
#[must_use]
pub fn shapes(shapes: Vec<Shape>) -> Shapes {
    Node::new(shapes, Style::default())
}

/// `source` scaled to cover the node's box, clipped to its corners.
#[must_use]
pub fn image(source: ImageSource) -> Image {
    Node::new(source, Style::default())
}

/// Lift `child` onto a layer placed by `placement`.
#[must_use]
pub fn deferred(placement: Placement, child: impl IntoElement) -> Deferred {
    Deferred {
        placement,
        child: child.into_element(),
    }
}

/// Anything that renders to an element; implement it for a component struct.
pub trait View {
    fn render(&self) -> Element;
}
