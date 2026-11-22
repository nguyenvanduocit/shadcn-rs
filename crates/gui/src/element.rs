//! The element tree a component returns each frame.
//!
//! Every node kind shares [`Common`]: its style and bound properties, the
//! [`Interaction`] its bindings read, its event handlers and its focus
//! handle. What
//! differs is what it holds: children ([`Div`]), a paragraph ([`Text`],
//! whose [`Span`]s style parts of it inline), an
//! [`Icon`], or an editable [`TextField`] ([`TextInput`]). [`Deferred`]
//! lifts a subtree onto a layer above the rest (popovers, dialogs, menus).

use std::ops::Range;
use std::rc::Rc;

use crate::color::Color;
use crate::edit::{TextEdit, TextField};
use crate::events::{ClickEvent, DragEvent, KeyEvent};
use crate::figure::Figure;
use crate::focus::FocusHandle;
use crate::geometry::Bounds;
use crate::icon::Icon;
use crate::image::ImageSource;
use crate::interact::Interaction;
use crate::scroll::ScrollHandle;
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
    /// Told each move of the pointer over the node, where it is.
    pub(crate) pointer: Option<Handler<crate::events::PointerEvent>>,
    /// Takes the wheel over it before any scroll container above it.
    pub(crate) wheel: Option<Handler<crate::events::WheelEvent>>,
    pub(crate) click_outside: Option<Rc<dyn Fn()>>,
    /// Told the node's box in window pixels after each layout.
    pub(crate) measured: Option<Handler<Bounds>>,
}

/// What every node kind has.
#[derive(Clone, Default)]
pub struct Common {
    pub(crate) key: Option<ElementKey>,
    /// A name the debug tools show for this node.
    pub(crate) name: Option<&'static str>,
    pub(crate) style: Style,
    /// Its bound properties (ADR-0013's bindings by phase): replayed over
    /// `style` where the node resolves, under a tracker of the node's own,
    /// so a signal they read resolves the node again without rendering the
    /// instance that built it.
    pub(crate) bindings: crate::value::Bindings,
    /// Where hit testing and focus write what they do to the node, for
    /// its bindings (and its children's) to read.
    pub(crate) interaction: Option<Interaction>,
    pub(crate) handlers: Handlers,
    pub(crate) focus: Option<FocusHandle>,
    /// Focusable by its place in the tree, as a `<button>` is.
    pub(crate) focusable: bool,
    pub(crate) disabled: bool,
    /// Clicks pass through this node to whatever is below it.
    pub(crate) pass_through: bool,
    /// Holds this scroll container's offset.
    pub(crate) scroll: Option<ScrollHandle>,
    /// What a screen reader is told this node is.
    pub(crate) semantics: Semantics,
    /// The lists it joins when laid out ([`crate::Members`]).
    pub(crate) joins: Vec<crate::members::Join>,
}

/// What a node is to assistive technology, as the ARIA attributes say it on
/// the web: its role, its name, and its state. A node that sets no role
/// takes the one its content implies (`crate::access`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Semantics {
    pub role: Option<Role>,
    pub label: Option<String>,
    pub checked: Option<Toggled>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    /// Rows in all, of which only some are built (`aria-rowcount`, `aria-setsize`).
    pub row_count: Option<usize>,
    /// This row's place among them, from 0 (`aria-rowindex`, and
    /// `aria-posinset` unless [`Self::set`] says otherwise).
    pub row_index: Option<usize>,
    /// Its place in its set, from 1, and the set's size (`aria-posinset`,
    /// `aria-setsize`): an option among the options of a list that holds
    /// headings too.
    pub set: Option<(usize, usize)>,
    /// Left out of what a screen reader reads, with its subtree.
    pub hidden: bool,
}

pub use accesskit::{Role, Toggled};

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
/// A lucide icon, drawn in the content colour provided to it.
pub type IconElement = Node<IconSpec>;
/// An editable text box.
pub type TextInput = Node<InputSpec>;
/// Vector figures drawn in the node's box.
pub type Figures = Node<Vec<Figure>>;
/// A raster image covering the node's box.
pub type Image = Node<ImageSource>;
/// A [`crate::chunk::Picture`] laid into the node's box (see [`drawing`]).
pub type Drawn = Node<DrawSpec>;

/// What a [`drawing`] draws: made by a closure run where the node is laid
/// out, under a tracker of its own.
#[derive(Clone)]
pub struct DrawSpec {
    pub(crate) make: Rc<dyn Fn() -> crate::chunk::Drawing>,
}

/// A paragraph's text and the spans styled inside it.
#[derive(Clone, Default)]
pub struct TextSpec {
    pub(crate) content: String,
    /// Content bound to signals, read when laid out (see [`text_bound`]).
    pub(crate) bound: Option<Rc<dyn Fn() -> String>>,
    pub(crate) spans: Vec<SpanSpec>,
    /// Elements laid in its lines as atomic inlines ([`Text::child`]), in
    /// order, child `i` on the [`crate::text::OBJECT`] at byte `objects[i]`.
    pub(crate) children: Vec<Element>,
    pub(crate) objects: Vec<usize>,
}

/// A [`Span`] placed in its paragraph.
#[derive(Clone)]
pub(crate) struct SpanSpec {
    pub(crate) range: Range<usize>,
    pub(crate) style: Style,
    /// Its bound properties, replayed where its paragraph resolves.
    pub(crate) bindings: crate::value::Bindings,
    pub(crate) click: Option<Handler<ClickEvent>>,
}

/// Text inside a paragraph in an inline style of its own, wrapping with the
/// rest of it (`<a>`, `<code>`, `<strong>`, a highlighted token). Of its style
/// it honours its content ([`crate::style::ContentStyle`]: colour, size,
/// weight, family, italic, underline, line-through, tracking), each field
/// it does not set taken from its paragraph (ADR-0001, ADR-0022), a
/// background with its corner radius and padding, and the cursor; layout
/// properties have no effect.
/// A closure given to a setter is bound to its paragraph's node, as an
/// element's is: a signal it reads resolves the paragraph again (a colour
/// repaints it, a size lays it out), not the instance that built it.
#[derive(Clone)]
pub struct Span {
    content: String,
    style: Style,
    bindings: crate::value::Bindings,
    click: Option<Handler<ClickEvent>>,
}

impl crate::styled::Fluent for Span {}

impl Styled for Span {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut crate::value::Bindings>) {
        (&mut self.style, Some(&mut self.bindings))
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
        bindings: crate::value::Bindings::default(),
        click: None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconSpec {
    pub icon: Icon,
}

#[derive(Clone)]
pub struct InputSpec {
    pub(crate) field: TextField,
    pub(crate) placeholder: String,
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
    /// `edit` as the input shows it, and the range of what an input method
    /// is composing there. A password takes no composition: the window offers
    /// it no input method, as a browser does.
    pub(crate) fn shown(self, edit: &TextEdit) -> (TextEdit, Option<(usize, usize)>) {
        match self.input_type {
            InputType::Text => edit.composed(),
            InputType::Password => (edit.masked(), None),
        }
    }

    /// Where an offset `at` in what the input shows of `edit` falls in its value.
    pub(crate) fn value_offset(self, edit: &TextEdit, at: usize) -> usize {
        match self.input_type {
            InputType::Text => edit.uncomposed(at),
            InputType::Password => edit.unmasked(at),
        }
    }

    /// Whether the input takes text from an input method: it is edited and
    /// its value shows as typed.
    pub(crate) fn composes(self) -> bool {
        !self.read_only && self.input_type == InputType::Text
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

/// Where a deferred layer is asked to go; [`crate::Placement`] is where
/// layout put it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Positioning {
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
    pub(crate) positioning: Positioning,
    pub(crate) child: Element,
    /// Where layout reports how the layer was placed.
    pub(crate) placed: Option<crate::reactive::Signal<Option<crate::placement::Placement>>>,
}

impl Deferred {
    /// Report how the layer is placed into `placed` each time layout
    /// places it (`None` while it is not anchored): the side it opened on,
    /// its alignment, the room it has, its anchor and the origin facing
    /// it, which a look reads as reka's content reads `data-side`.
    #[must_use]
    pub fn placement(self, placed: crate::reactive::Signal<Option<crate::placement::Placement>>) -> Self {
        Self {
            placed: Some(placed),
            ..self
        }
    }
}

/// A node in the UI tree.
#[derive(Clone, Default)]
pub enum Element {
    Div(Rc<Div>),
    Text(Rc<Text>),
    Icon(Rc<IconElement>),
    Input(Rc<TextInput>),
    Figures(Rc<Figures>),
    Image(Rc<Image>),
    Drawn(Rc<Drawn>),
    Deferred(Rc<Deferred>),
    /// A component instance's place (ADR-0013), replaced by its output
    /// when the frame resolves the tree.
    Component(Rc<crate::component::ComponentElement>),
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
            Self::Figures(n) => Some(&n.common),
            Self::Image(n) => Some(&n.common),
            Self::Drawn(n) => Some(&n.common),
            Self::Deferred(_) | Self::Component(_) | Self::Empty => None,
        }
    }

    pub(crate) fn common_mut(&mut self) -> Option<&mut Common> {
        match self {
            Self::Div(n) => Some(&mut Rc::make_mut(n).common),
            Self::Text(n) => Some(&mut Rc::make_mut(n).common),
            Self::Icon(n) => Some(&mut Rc::make_mut(n).common),
            Self::Input(n) => Some(&mut Rc::make_mut(n).common),
            Self::Figures(n) => Some(&mut Rc::make_mut(n).common),
            Self::Image(n) => Some(&mut Rc::make_mut(n).common),
            Self::Drawn(n) => Some(&mut Rc::make_mut(n).common),
            Self::Deferred(_) | Self::Component(_) | Self::Empty => None,
        }
    }

    #[must_use]
    pub fn key(&self) -> Option<ElementKey> {
        self.common().and_then(|c| c.key)
    }

    /// Whether `other` is this very element (the same shared node): a
    /// subtree a clean instance gave back again is known by pointer.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Div(a), Self::Div(b)) => Rc::ptr_eq(a, b),
            (Self::Text(a), Self::Text(b)) => Rc::ptr_eq(a, b),
            (Self::Icon(a), Self::Icon(b)) => Rc::ptr_eq(a, b),
            (Self::Input(a), Self::Input(b)) => Rc::ptr_eq(a, b),
            (Self::Figures(a), Self::Figures(b)) => Rc::ptr_eq(a, b),
            (Self::Image(a), Self::Image(b)) => Rc::ptr_eq(a, b),
            (Self::Drawn(a), Self::Drawn(b)) => Rc::ptr_eq(a, b),
            (Self::Deferred(a), Self::Deferred(b)) => Rc::ptr_eq(a, b),
            (Self::Component(a), Self::Component(b)) => Rc::ptr_eq(a, b),
            (Self::Empty, Self::Empty) => true,
            _ => false,
        }
    }

    /// The node's own style, for a parent that styles its children by
    /// position (CSS `& > *`); `None` for a layer or nothing.
    pub fn style_mut(&mut self) -> Option<&mut Style> {
        match self {
            Self::Div(n) => Some(&mut Rc::make_mut(n).common.style),
            Self::Text(n) => Some(&mut Rc::make_mut(n).common.style),
            Self::Icon(n) => Some(&mut Rc::make_mut(n).common.style),
            Self::Input(n) => Some(&mut Rc::make_mut(n).common.style),
            Self::Figures(n) => Some(&mut Rc::make_mut(n).common.style),
            Self::Image(n) => Some(&mut Rc::make_mut(n).common.style),
            Self::Drawn(n) => Some(&mut Rc::make_mut(n).common.style),
            Self::Deferred(_) | Self::Component(_) | Self::Empty => None,
        }
    }

    /// The element with `f` applied to its root's style after its own, for
    /// a parent that joins or sizes its children by position (CSS
    /// `& > *`). A component's place keeps `f` and applies it to the root
    /// of every output the component gives (Vue's attribute fallthrough);
    /// a layer or nothing is returned as it is.
    #[must_use]
    pub fn restyle(mut self, f: impl Fn(Style) -> Style + 'static) -> Self {
        if let Self::Component(c) = &mut self {
            let Some(c) = Rc::get_mut(c) else {
                eprintln!(
                    "gui: `{}` is placed twice; only its first place can be restyled",
                    c.name
                );
                return self;
            };
            let f: crate::component::Fallthrough = match c.fallthrough.take() {
                Some(before) => Rc::new(move |s| f(before(s))),
                None => Rc::new(f),
            };
            c.fallthrough = Some(f);
            return self;
        }
        if let Some(common) = self.common_mut() {
            // Over the node's bindings when it has any, so a property the
            // node bound is the parent's when the parent writes it too.
            let f: Rc<dyn Fn(Style) -> Style> = Rc::new(f);
            if !common.bindings.restyle(Rc::clone(&f)) {
                common.style = f(std::mem::take(&mut common.style));
            }
        }
        self
    }

    /// The name [`Node::named`] gave the node, or a component's place its
    /// component's name.
    #[must_use]
    pub fn name(&self) -> Option<&'static str> {
        match self {
            Self::Component(c) => Some(c.name),
            _ => self.common().and_then(|c| c.name),
        }
    }

    /// The elements below this one in the tree: a box's children, or the
    /// atomic inlines of a paragraph ([`Text::child`]); `None` for every
    /// other kind. Whatever walks the element tree walks them alike.
    pub(crate) fn kids(&self) -> Option<&[Self]> {
        match self {
            Self::Div(n) => Some(&n.kind),
            Self::Text(n) => Some(&n.kind.children),
            _ => None,
        }
    }

    /// [`Self::kids`], to change them in place (not to add or take any: a
    /// paragraph's stand on its content's [`crate::text::OBJECT`]s).
    pub(crate) fn kids_mut(&mut self) -> Option<&mut [Self]> {
        match self {
            Self::Div(n) => Some(&mut Rc::make_mut(n).kind),
            Self::Text(n) => Some(&mut Rc::make_mut(n).kind.children),
            _ => None,
        }
    }

    /// This element with `kids` in place of [`Self::kids`] (as many, in
    /// the same order), sharing everything else.
    pub(crate) fn with_kids(&self, kids: Vec<Self>) -> Self {
        match self {
            Self::Div(n) => Self::Div(Rc::new(Node {
                common: n.common.clone(),
                kind: kids,
            })),
            Self::Text(n) => Self::Text(Rc::new(Node {
                common: n.common.clone(),
                kind: TextSpec {
                    content: n.kind.content.clone(),
                    bound: n.kind.bound.clone(),
                    spans: n.kind.spans.clone(),
                    children: kids,
                    objects: n.kind.objects.clone(),
                },
            })),
            other => other.clone(),
        }
    }

    /// A box's children, for a caller that reshapes a tree a component built
    /// (a docs page lifting each named demo out of its page); `None` for every
    /// other kind.
    pub fn children_mut(&mut self) -> Option<&mut Vec<Self>> {
        match self {
            Self::Div(n) => Some(&mut Rc::make_mut(n).kind),
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
        Element::Div(Rc::new(self))
    }
}

impl IntoElement for Text {
    fn into_element(self) -> Element {
        Element::Text(Rc::new(self))
    }
}

impl IntoElement for IconElement {
    fn into_element(self) -> Element {
        Element::Icon(Rc::new(self))
    }
}

impl IntoElement for TextInput {
    fn into_element(self) -> Element {
        Element::Input(Rc::new(self))
    }
}

impl IntoElement for Figures {
    fn into_element(self) -> Element {
        Element::Figures(Rc::new(self))
    }
}

impl IntoElement for Image {
    fn into_element(self) -> Element {
        Element::Image(Rc::new(self))
    }
}

impl IntoElement for Drawn {
    fn into_element(self) -> Element {
        Element::Drawn(Rc::new(self))
    }
}

impl IntoElement for Deferred {
    fn into_element(self) -> Element {
        Element::Deferred(Rc::new(self))
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
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut crate::value::Bindings>) {
        (&mut self.common.style, Some(&mut self.common.bindings))
    }

    fn node_interaction(&self) -> Option<Interaction> {
        self.common.interaction
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

    /// Join `list` with `value` while laid out: the list holds its members'
    /// values in tree order as last laid out ([`crate::Members`]).
    #[must_use]
    pub fn member<T: Clone + 'static>(mut self, list: &crate::Members<T>, value: T) -> Self {
        self.common.joins.push(crate::members::join(list, value));
        self
    }

    /// Write what the pointer and the keyboard do to this node into `i`
    /// (hovered, pressed, focused, focus-visible, focus-within, disabled),
    /// for bindings to read: `.background(move || if i.hovered() { a } else { b })`.
    #[must_use]
    pub fn interaction(mut self, i: Interaction) -> Self {
        self.common.interaction = Some(i);
        self
    }

    /// Build on with this node's [`Interaction`], made and given to it when
    /// it has none: `.with_interaction(|d, i| d.border_color(move || …))`.
    #[must_use]
    #[track_caller]
    pub fn with_interaction(mut self, f: impl FnOnce(Self, Interaction) -> Self) -> Self {
        let i = match self.common.interaction {
            Some(i) => i,
            None => *self.common.interaction.insert(Interaction::new()),
        };
        f(self, i)
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

    /// What this node is to a screen reader (`role="…"`).
    #[must_use]
    pub fn role(mut self, role: Role) -> Self {
        self.common.semantics.role = Some(role);
        self
    }

    /// Its name to a screen reader when its content does not say it (`aria-label`).
    #[must_use]
    pub fn aria_label(mut self, label: impl Into<String>) -> Self {
        self.common.semantics.label = Some(label.into());
        self
    }

    /// Checked, unchecked or mixed, for a checkbox, switch or radio
    /// (`aria-checked`), and pressed or not for a toggle button (`aria-pressed`).
    #[must_use]
    pub fn aria_checked(mut self, checked: impl Into<Toggled>) -> Self {
        self.common.semantics.checked = Some(checked.into());
        self
    }

    /// The chosen one of its set, for a tab or an option (`aria-selected`).
    #[must_use]
    pub fn aria_selected(mut self, selected: bool) -> Self {
        self.common.semantics.selected = Some(selected);
        self
    }

    /// Whether what it controls is open, for a disclosure or menu trigger (`aria-expanded`).
    #[must_use]
    pub fn aria_expanded(mut self, expanded: bool) -> Self {
        self.common.semantics.expanded = Some(expanded);
        self
    }

    /// How many rows a virtual list or table has in all, built or not
    /// (`aria-rowcount`; each row's `aria-setsize`).
    #[must_use]
    pub fn aria_row_count(mut self, rows: usize) -> Self {
        self.common.semantics.row_count = Some(rows);
        self
    }

    /// This row's index among all of them, from 0 (`aria-rowindex`, `aria-posinset`).
    #[must_use]
    pub fn aria_row_index(mut self, row: usize) -> Self {
        self.common.semantics.row_index = Some(row);
        self
    }

    /// Its place in its set, from 1, of `size` (`aria-posinset`, `aria-setsize`).
    #[must_use]
    pub fn aria_pos_in_set(mut self, position: usize, size: usize) -> Self {
        self.common.semantics.set = Some((position, size));
        self
    }

    /// Leave this node and its subtree out of what a screen reader reads (`aria-hidden`).
    #[must_use]
    pub fn aria_hidden(mut self) -> Self {
        self.common.semantics.hidden = true;
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
    /// Run `f` with the node's border box in window pixels after each
    /// layout (a table's `Auto` columns learn their cells' widths so).
    #[must_use]
    pub fn on_measured(mut self, f: impl Fn(&Bounds) + 'static) -> Self {
        self.common.handlers.measured = Some(Rc::new(f));
        self
    }

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

    /// Called on each move of the pointer over this node (or a child), with
    /// where it is; [`Self::on_hover`] says when it leaves.
    #[must_use]
    pub fn on_pointer_move(mut self, f: impl Fn(&crate::events::PointerEvent) + 'static) -> Self {
        self.common.handlers.pointer = Some(Rc::new(f));
        self
    }

    /// Called with the wheel over this node (or a child), which then
    /// scrolls nothing above it.
    #[must_use]
    pub fn on_wheel(mut self, f: impl Fn(&crate::events::WheelEvent) + 'static) -> Self {
        self.common.handlers.wheel = Some(Rc::new(f));
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
        if span.style != Style::default() || !span.bindings.is_empty() || span.click.is_some() {
            self.kind.spans.push(SpanSpec {
                range: start..self.kind.content.len(),
                style: span.style,
                bindings: span.bindings,
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

    /// Append `child` to the paragraph as an atomic inline (CSS
    /// `inline-flex`, `inline-block`: a button in a `<p>`): laid out at its
    /// shrink-to-fit width, it takes its size in the line, wraps with the
    /// words around it, and sits on the baseline by its own first baseline
    /// (its bottom margin edge when it has none); the line grows to hold
    /// it. It is a node of its own below the paragraph, hit, focused,
    /// painted and read as any child is. It stands on a
    /// [`crate::text::OBJECT`] in the content, which no reader or copy
    /// sees; a paragraph whose content is bound ([`text_bound`]) holds none.
    #[must_use]
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.kind.objects.push(self.kind.content.len());
        self.kind.content.push(crate::text::OBJECT);
        self.kind.children.push(child.into_element());
        self
    }

    /// Append each of `children` as [`Self::child`] does, in order.
    #[must_use]
    pub fn children<E: IntoElement>(self, children: impl IntoIterator<Item = E>) -> Self {
        children.into_iter().fold(self, Self::child)
    }
}

impl IconElement {
    /// Line width in the icon's 24-unit grid (lucide's default is 2): a
    /// value, or a closure bound to the node's paint.
    #[must_use]
    pub fn stroke_width<M>(mut self, width: impl crate::value::IntoValue<f32, M>) -> Self {
        let (style, bindings) = self.style_and_bindings();
        crate::value::set(
            style,
            bindings,
            crate::value::Fields::of(&[crate::value::Field::StrokeWidth]),
            width,
            |s, v| s.stroke_width = v,
        );
        self
    }
}

impl TextInput {
    #[must_use]
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.kind.placeholder = text.into();
        self
    }

    /// The placeholder's colour (`::placeholder { color }`): a value, or
    /// a closure bound to the node's paint.
    #[must_use]
    pub fn placeholder_color<M>(mut self, color: impl crate::value::IntoValue<Option<Color>, M>) -> Self {
        let (style, bindings) = self.style_and_bindings();
        crate::value::set(
            style,
            bindings,
            crate::value::Fields::of(&[crate::value::Field::PlaceholderColor]),
            color,
            |s, v| s.placeholder_color = v,
        );
        self
    }

    /// What is painted behind selected text (`::selection { background }`):
    /// a value, or a closure bound to the node's paint.
    #[must_use]
    pub fn selection_color<M>(mut self, color: impl crate::value::IntoValue<Option<Color>, M>) -> Self {
        let (style, bindings) = self.style_and_bindings();
        crate::value::set(
            style,
            bindings,
            crate::value::Fields::of(&[crate::value::Field::SelectionColor]),
            color,
            |s, v| s.selection_color = v,
        );
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

/// A box (`<div>`): block layout until `.display(Display::Flex)` or `.display(Display::Grid)`.
#[must_use]
pub fn div() -> Div {
    Node::new(Vec::new(), Style::default())
}

/// A paragraph in the content style provided to it ([`crate::style::ContentStyle`]);
/// [`Text::span`] adds more to it.
#[must_use]
pub fn text(content: impl Into<String>) -> Text {
    Node::new(
        TextSpec {
            content: content.into(),
            bound: None,
            spans: Vec::new(),
            children: Vec::new(),
            objects: Vec::new(),
        },
        Style::default(),
    )
}

/// A paragraph whose content `f` gives when it is laid out: a signal `f`
/// reads changes the text without rendering the instance that built it
/// (Vue Vapor's and Leptos' text bindings).
#[must_use]
pub fn text_bound(f: impl Fn() -> String + 'static) -> Text {
    Node::new(
        TextSpec {
            content: String::new(),
            bound: Some(Rc::new(f)),
            spans: Vec::new(),
            children: Vec::new(),
            objects: Vec::new(),
        },
        Style::default(),
    )
}

/// A lucide icon, as large as the `icon_size` provided to it (16px at the
/// root), and not shrinking.
#[must_use]
pub fn icon(icon: Icon) -> IconElement {
    Node::new(IconSpec { icon }, Style::default()).flex_shrink(0.0)
}

/// A text box editing `field`, focusable through the field's handle.
#[must_use]
pub fn text_input(field: &TextField) -> TextInput {
    let focus = field.focus;
    Node::new(
        InputSpec {
            field: *field,
            placeholder: String::new(),
            multiline: false,
            mode: InputMode::default(),
        },
        Style::default(),
    )
    .track_focus(focus)
    .cursor(crate::style::Cursor::Text)
    // A browser's own `input, textarea { text-align: start }`: a field
    // starts its text at its start edge in a centred parent too.
    .text_align(crate::style::TextAlign::Left)
}

/// Vector `figures` drawn in the node's box (give it a size).
#[must_use]
pub fn figures(figures: Vec<Figure>) -> Figures {
    Node::new(figures, Style::default())
}

/// What `make` returns, drawn in the node's box (give it a size). `make`
/// runs where the node is laid out, tracked as a bound style or text is
/// (ADR-0013): a signal it read changing makes it run again and records
/// the node's chunk alone, without rendering the instance that built it.
#[must_use]
pub fn drawing(make: impl Fn() -> crate::chunk::Drawing + 'static) -> Drawn {
    Node::new(DrawSpec { make: Rc::new(make) }, Style::default())
}

/// `source` scaled to cover the node's box, clipped to its corners.
#[must_use]
pub fn image(source: ImageSource) -> Image {
    Node::new(source, Style::default())
}

/// Lift `child` onto a layer placed as `positioning` asks.
#[must_use]
pub fn deferred(positioning: Positioning, child: impl IntoElement) -> Deferred {
    Deferred {
        positioning,
        placed: None,
        child: child.into_element(),
    }
}

/// Anything that renders to an element; implement it for a component struct.
pub trait View {
    fn render(&self) -> Element;
}
