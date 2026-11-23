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

/// A wheel handler: returns whether it consumed the wheel.
pub type WheelHandler = Rc<dyn Fn(&crate::events::WheelEvent) -> bool>;

/// Whether a press with these modifiers focuses a node.
pub type PressFocus = Rc<dyn Fn(&crate::events::Modifiers) -> bool>;

/// Which focus a press on a node hands to another node's, when it is
/// pressed itself rather than on a node inside it that takes focus.
pub type PressFocuses = Rc<dyn Fn() -> Option<crate::focus::FocusHandle>>;

/// What a node does when the pointer or keyboard acts on it.
#[derive(Clone, Default)]
pub struct Handlers {
    pub(crate) click: Option<Handler<ClickEvent>>,
    pub(crate) mouse_down: Option<Handler<ClickEvent>>,
    pub(crate) drag: Option<Handler<DragEvent>>,
    pub(crate) key_down: Option<KeyHandler>,
    /// Told each key nothing on the focused node's way up took, wherever
    /// focus is ([`Div::on_window_key`]).
    pub(crate) window_key: Option<KeyHandler>,
    pub(crate) secondary_click: Option<Handler<ClickEvent>>,
    pub(crate) hover: Option<Rc<dyn Fn(bool)>>,
    /// Told each move of the pointer over the node, where it is.
    pub(crate) pointer: Option<Handler<crate::events::PointerEvent>>,
    /// Takes the wheel over it before any scroll container above it;
    /// returns whether it consumed it.
    pub(crate) wheel: Option<WheelHandler>,
    /// Whether a primary press with these modifiers focuses the node
    /// (none: every press does); asked of the node a press would focus.
    pub(crate) press_focus: Option<PressFocus>,
    /// The focus a press on the node itself hands on ([`Div::press_focuses`]).
    pub(crate) press_focuses: Option<PressFocuses>,
    /// A touch or pen held on it ([`crate::gesture`]).
    pub(crate) long_press: Option<Handler<ClickEvent>>,
    /// How long a touch or pen is held before `long_press` runs (none:
    /// [`crate::gesture::LONG_PRESS`]).
    pub(crate) long_press_after: Option<std::time::Duration>,
    /// A press that went down outside it released over it.
    pub(crate) release: Option<Handler<ClickEvent>>,
    /// The raw contact of a touch or pen that went down on it.
    pub(crate) touch: Option<Handler<crate::events::TouchEvent>>,
    /// What an input method does while focus is on it (or inside it) and
    /// it is not a text input.
    pub(crate) ime: Option<Handler<crate::events::ImeEvent>>,
    /// The floating layer whose region it is ([`crate::layer`]).
    pub(crate) layer: Option<Rc<crate::layer::Layer>>,
    /// Told the node's box in window pixels after each layout.
    pub(crate) measured: Option<Handler<Bounds>>,
    /// The nodes captured with it ([`Div::capture_group`]).
    pub(crate) capture_group: Option<crate::events::CaptureGroup>,
    /// Each pointer pressing it drags and touches it, not only the first
    /// ([`Div::multi_pointer`]).
    pub(crate) multi_pointer: bool,
    /// The component instance whose render made it: its handlers run
    /// there, and a node another instance makes at its path is another
    /// node (a drag, a press, a click it held does not pass to it).
    pub(crate) owner: Option<crate::reactive::OwnerId>,
    /// The component instance whose output it is the root of (its scope):
    /// each node is made by the nearest such above it (itself included).
    pub(crate) instance: Option<crate::reactive::OwnerId>,
}

/// Handlers a node is given add up, as the DOM's `addEventListener`:
/// each one registered runs, in the order registered. A primitive that
/// adds its own to a node a caller made (a tooltip's trigger around a
/// button) keeps the caller's.
fn then<E: ?Sized + 'static>(before: Option<Handler<E>>, f: Handler<E>) -> Handler<E> {
    match before {
        Some(b) => Rc::new(move |e: &E| {
            b(e);
            f(e);
        }),
        None => f,
    }
}

/// A handler that returns whether it consumed its event.
type Consumes<E> = Rc<dyn Fn(&E) -> bool>;

/// Handlers that consume or pass add up as [`then`] does, stopping at
/// the first that consumes: a later one is asked only what the earlier
/// ones passed.
fn or_consumed<E: ?Sized + 'static>(before: Option<Consumes<E>>, f: Consumes<E>) -> Consumes<E> {
    match before {
        Some(b) => Rc::new(move |e: &E| b(e) || f(e)),
        None => f,
    }
}

/// `f`, run inside `owner` (the component instance whose render made it,
/// [`crate::component::handler_owner`]): a timer or effect it makes is
/// owned there, and goes with the instance. The engine cancels what a node
/// captured before its instance is disposed; a call that still comes after
/// (a stray one) runs nothing.
fn in_owner<E: ?Sized + 'static, R: Default + 'static>(
    owner: Option<crate::reactive::OwnerId>,
    f: impl Fn(&E) -> R + 'static,
) -> Rc<dyn Fn(&E) -> R> {
    match owner {
        Some(owner) => Rc::new(move |e| {
            if crate::reactive::is_live(owner) {
                crate::reactive::run_owned(Some(owner), || f(e))
            } else {
                R::default()
            }
        }),
        None => Rc::new(f),
    }
}

/// Whether a focusable node is a Tab stop while it does not hold focus
/// ([`Div::tab_stop`]): fixed, or a closure read when Tab looks.
#[derive(Clone, Default)]
pub(crate) enum TabStop {
    #[default]
    Always,
    /// `tabindex="-1"`.
    Never,
    Bound(Rc<dyn Fn() -> bool>),
}

impl TabStop {
    /// Whether it is a stop now; a closure runs untracked.
    pub(crate) fn get(&self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Bound(f) => crate::untracked(|| f()),
        }
    }
}

/// What every node kind has.
#[derive(Clone, Default)]
pub struct Common {
    pub(crate) key: Option<ElementKey>,
    /// The key of the component whose output's root it is, when that
    /// component is keyed: its path segment is the component's
    /// ([`crate::geometry::PathSeg::Component`]).
    pub(crate) component_key: Option<ElementKey>,
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
    /// Whether it is a Tab stop while it does not hold focus
    /// ([`Div::tab_stop`]).
    pub(crate) tab_stop: TabStop,
    /// Tab visits its children last first ([`Div::tab_children_reversed`]).
    pub(crate) tab_reversed: bool,
    pub(crate) disabled: bool,
    /// Clicks pass through this node to whatever is below it.
    pub(crate) pass_through: bool,
    /// Clicks pass through it and its subtree ([`Div::pass_through_all`]).
    pub(crate) pass_through_all: bool,
    /// It and its subtree take no input, no focus and no screen reader.
    pub(crate) inert: bool,
    /// It and its subtree are dormant ([`Div::dormant`]).
    pub(crate) dormant: bool,
    /// Holds this scroll container's offset.
    pub(crate) scroll: Option<ScrollHandle>,
    /// What the render built the container's children from, handed to its
    /// handle's hook after layout ([`Div::track_scroll_built`]).
    pub(crate) scroll_built: Option<Rc<dyn std::any::Any>>,
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
    /// Which way it runs (`aria-orientation`): a separator, a tab list, a
    /// slider.
    pub orientation: Option<crate::style::Axis>,
    /// The value it holds in a range (`aria-valuenow`, `aria-valuemin`,
    /// `aria-valuemax`, `aria-valuetext`): a spin button, a slider.
    pub range: Option<ValueRange>,
    /// Read as disabled, with what it holds, but still focusable
    /// (`aria-disabled`); its clicks do not run ([`Div::unavailable`]).
    pub unavailable: bool,
    /// Left out of what a screen reader reads, with its subtree.
    pub hidden: bool,
    /// Its name for other nodes' relations.
    pub node: Option<crate::access::NodeRef>,
    /// What it points to (labels, descriptions, what it controls…).
    pub relations: Relations,
    /// What else it states that changes nothing but what is read.
    pub announce: Announce,
}

/// What a node states to a screen reader beyond its role, name and state,
/// changing nothing else about it: a heading's level, the current item of
/// a set, a role's own name, a live region.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Announce {
    /// Its level as a heading, from 1 (`aria-level`; `h3` is 3).
    pub level: Option<usize>,
    /// It is the current one of its set (`aria-current`): a pagination's
    /// page, a stepper's step.
    pub current: Option<AriaCurrent>,
    /// What its role is called here (`aria-roledescription`): "carousel",
    /// "slide".
    pub role_description: Option<String>,
    /// What is read after its name (`aria-description`): an option's group,
    /// in a list whose groups are not nodes around their options.
    pub description: Option<String>,
    /// How changes inside it are told (`aria-live`).
    pub live: Option<Live>,
    /// A list or tree whose user may choose several (`aria-multiselectable`).
    pub multiselectable: bool,
    /// The text it holds, for a node that holds text without being a text
    /// input (a pin's slot, a one-time code): an input's `value`.
    pub value: Option<String>,
    /// A control a form needs filled (`aria-required`).
    pub required: bool,
    /// The popup it opens (`aria-haspopup`): a combobox's listbox.
    pub has_popup: Option<HasPopup>,
    /// How typing in it completes (`aria-autocomplete`).
    pub auto_complete: Option<AutoComplete>,
    /// Its value is not one it accepts (`aria-invalid`).
    pub invalid: bool,
    /// Its value is shown and moved through, not changed (`aria-readonly`):
    /// a read-only date segment, a calendar's grid.
    pub read_only: bool,
}

/// A value in a range, as a screen reader is told it: a spin button's or a
/// slider's. A node holding one takes AccessKit's `Increment` and
/// `Decrement`, which reach its key handler as ArrowUp and ArrowDown, as a
/// press of those keys steps it.
#[derive(Clone, Debug, Default)]
pub struct ValueRange {
    /// The value; none while it is empty (a time segment showing `--`).
    pub now: Option<f64>,
    /// The ends; an infinite one (a number field with no bound) is no end
    /// and is not told.
    pub min: f64,
    pub max: f64,
    /// What is read for the value when its number is not what it means
    /// (`aria-valuetext`): "Blank" for an empty segment.
    pub text: Option<String>,
}

/// Equal when every number has the same bits, so the relation is an
/// equivalence (NaN equals itself) and the tree's diff stays exact.
impl PartialEq for ValueRange {
    fn eq(&self, other: &Self) -> bool {
        self.now.map(f64::to_bits) == other.now.map(f64::to_bits)
            && self.min.to_bits() == other.min.to_bits()
            && self.max.to_bits() == other.max.to_bits()
            && self.text == other.text
    }
}

impl Eq for ValueRange {}

/// A node's relations to others, by [`crate::access::NodeRef`]: told to a
/// screen reader only towards nodes it is shown, so none points at
/// something that is not there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Relations {
    pub labelled_by: Vec<crate::access::NodeRef>,
    pub described_by: Vec<crate::access::NodeRef>,
    pub controls: Vec<crate::access::NodeRef>,
    pub owns: Vec<crate::access::NodeRef>,
    pub active_descendant: Option<crate::access::NodeRef>,
}

impl Relations {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.labelled_by.is_empty()
            && self.described_by.is_empty()
            && self.controls.is_empty()
            && self.owns.is_empty()
            && self.active_descendant.is_none()
    }
}

pub use accesskit::{AriaCurrent, AutoComplete, HasPopup, Live, Role, Toggled};

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
        let f = in_owner(crate::component::handler_owner(), f);
        self.click = Some(then(self.click.take(), f));
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
    /// it no input method, as a browser does. Its text is `last`'s (what it
    /// showed before) when the same ([`crate::edit::Shown`]).
    pub(crate) fn shown(
        self,
        edit: &TextEdit,
        last: Option<&crate::edit::Shown>,
    ) -> (crate::edit::Shown, Option<(usize, usize)>) {
        match self.input_type {
            InputType::Text => edit.composed(last),
            InputType::Password => (edit.masked(last), None),
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
    /// Right to left (`direction`) the right edges line up instead. It is at
    /// least `min_height` tall (or all of itself), and `part` shows whole:
    /// what the window's bottom edge would cut is made up above.
    Over {
        part: ElementKey,
        anchor_part: ElementKey,
        direction: crate::Direction,
        min_height: f32,
    },
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
    /// The keyed node inside the declaring node it is placed against.
    pub(crate) anchor_part: Option<ElementKey>,
    /// The keyed node inside the declaring node its Tab stops follow.
    pub(crate) tab_after: Option<ElementKey>,
}

impl Deferred {
    /// Place it against the node keyed `part` inside the node that declares
    /// it (reka's `PopoverAnchor`), not against that node; while no such
    /// node is laid out, against the declaring node.
    #[must_use]
    pub fn anchor_part(self, part: ElementKey) -> Self {
        Self {
            anchor_part: Some(part),
            ..self
        }
    }

    /// Put its Tab stops right after the subtree of the node keyed `part`
    /// inside the node that declares it, not after the declaring node's
    /// subtree: one surface shared by several triggers (a navigation menu's
    /// viewport) follows the trigger it shows for, as reka's focus proxy
    /// makes it. While no such node is laid out, after the declaring node.
    #[must_use]
    pub fn tab_after(self, part: ElementKey) -> Self {
        Self {
            tab_after: Some(part),
            ..self
        }
    }

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

    /// Its path segment as child `index` of its parent: the keyed
    /// component's whose root it is, else its key, else its place.
    #[must_use]
    pub(crate) fn seg(&self, index: usize) -> crate::geometry::PathSeg {
        use crate::geometry::PathSeg;
        // A layer at a keyed component's root is placed as its content.
        if let Self::Deferred(d) = self
            && d.child.common().is_some_and(|c| c.component_key.is_some())
        {
            return d.child.seg(index);
        }
        match self.common() {
            Some(Common {
                component_key: Some(k),
                key,
                ..
            }) => PathSeg::Component(*k, *key),
            _ => self.key().map_or(PathSeg::Index(index), PathSeg::Key),
        }
    }

    /// Whether another holds this very node (it is shared, not this one's
    /// alone to write in place).
    pub(crate) fn is_shared(&self) -> bool {
        match self {
            Self::Div(n) => Rc::strong_count(n) > 1,
            Self::Text(n) => Rc::strong_count(n) > 1,
            Self::Icon(n) => Rc::strong_count(n) > 1,
            Self::Input(n) => Rc::strong_count(n) > 1,
            Self::Figures(n) => Rc::strong_count(n) > 1,
            Self::Image(n) => Rc::strong_count(n) > 1,
            Self::Drawn(n) => Rc::strong_count(n) > 1,
            Self::Deferred(n) => Rc::strong_count(n) > 1,
            Self::Component(n) => Rc::strong_count(n) > 1,
            Self::Empty => false,
        }
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

    /// Whether this focusable node is a Tab stop (default true). One that
    /// is not still takes focus from code and the pointer (`tabindex="-1"`),
    /// and while it holds focus Tab goes on from it: a roving group's items
    /// other than its one stop.
    ///
    /// A value or a closure. Nothing is laid out, painted or announced from
    /// it, so it belongs to no phase: a closure is run, untracked, only when
    /// Tab looks for the next stop. A signal it reads changing (a roving
    /// group's stop moving) renders nothing and dirties nothing.
    #[must_use]
    pub fn tab_stop<M>(mut self, stop: impl crate::value::IntoValue<bool, M>) -> Self {
        self.common.tab_stop = match stop.into_value() {
            crate::value::Value::Fixed(true) => TabStop::Always,
            crate::value::Value::Fixed(false) => TabStop::Never,
            crate::value::Value::Bound(f) => TabStop::Bound(f),
        };
        self
    }

    /// Tab visits this node's children last first, each child's own
    /// subtree (and the layers declared in it) still in tree order: a
    /// stack of toasts declared oldest first, so the newest paints on top,
    /// whose newest Tab reaches first (reka's `ToastViewport`). Shift+Tab
    /// goes the other way, so from outside it reaches the oldest child's
    /// last stop.
    #[must_use]
    pub fn tab_children_reversed(mut self, reversed: bool) -> Self {
        self.common.tab_reversed = reversed;
        self
    }

    /// Keep this scroll container's offset in `handle`, which reads its
    /// layout and moves it.
    #[must_use]
    pub fn track_scroll(mut self, handle: &ScrollHandle) -> Self {
        self.common.scroll = Some(handle.clone());
        self
    }

    /// [`Self::track_scroll`], with what this render built the children
    /// from (the rows a virtual list chose): handed to the handle's hook
    /// after each layout of the frame, where state is written, so the
    /// render that chose it writes nothing (a render is pure, ADR-0013).
    #[must_use]
    pub(crate) fn track_scroll_built(mut self, handle: &ScrollHandle, built: Rc<dyn std::any::Any>) -> Self {
        self.common.scroll = Some(handle.clone());
        self.common.scroll_built = Some(built);
        self
    }

    /// Disable this node and its subtree, the deferred subtrees declared in
    /// it included, as [`Self::inert`] is inherited and as a disabled
    /// `<fieldset>` disables what it holds: every node in it takes no
    /// focus (Tab passes it by), no keys and no activation (no click from
    /// the pointer, Enter or Space, or a screen reader), and a screen
    /// reader reads it as disabled. No handler of a node in it that acts
    /// runs, for the pointer or the keyboard; what only follows the
    /// pointer still does, as Chrome sends `mouseover`, `mouseenter` and
    /// `mousemove` to a disabled control: [`Self::on_hover`],
    /// [`Self::on_pointer_move`] and [`Self::on_wheel`] (its hovered flag
    /// stays off). Gestures an enabled ancestor owns
    /// still reach that ancestor through it, as Chrome sends pointer events
    /// on past a disabled form control (a drawer swipes from a disabled
    /// button in it, a context menu around it opens): a press, a drag, a
    /// touch, a long press and a secondary click; a click never does. A
    /// press in it moves no focus. A drag or a touch a node in it holds
    /// when it is disabled is cancelled: its handler gets
    /// [`crate::DragPhase::Cancel`] or [`crate::TouchPhase::Cancelled`]
    /// once, its one call while disabled, and nothing after. Unlike
    /// [`Self::unavailable`], which keeps focus and keys.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.common.disabled = disabled;
        self
    }

    /// An unavailable node still takes focus and keys, and a screen reader
    /// reads it, as disabled (`aria-disabled`); but it is not activated:
    /// no click runs on it or through it on its ancestors, whether from the
    /// pointer, Enter or Space, or a screen reader. Gestures an ancestor
    /// owns still reach the ancestor through it, as pointer events bubble
    /// past an `aria-disabled` element on the web (a drawer swipes from it,
    /// a context menu around it opens): a press, a drag, a touch, a long
    /// press and a secondary click. Its own handlers of those do not run,
    /// as they would act on it; a press still focuses it
    /// ([`Self::focus_on_press`], [`Self::press_focuses`]). Keys reach its
    /// handlers, which decide what an unavailable node does with them, and
    /// hover, pointer moves and the wheel reach it as they reach any node.
    /// A drag or a touch it holds when it is made unavailable is cancelled
    /// once, as on a disabled node. It applies to its own node for input:
    /// a node inside it keeps its own clicks (their click does not pass it
    /// to its ancestors) and its focus, but a screen reader reads that node
    /// as disabled too, as Chrome reads what an `aria-disabled` element
    /// holds.
    #[must_use]
    pub fn unavailable(mut self, unavailable: bool) -> Self {
        self.common.semantics.unavailable = unavailable;
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

    /// Which way it runs, to a screen reader (`aria-orientation`).
    #[must_use]
    pub fn aria_orientation(mut self, orientation: crate::style::Axis) -> Self {
        self.common.semantics.orientation = Some(orientation);
        self
    }

    /// The value it holds in a range, to a screen reader (`aria-valuenow`,
    /// `aria-valuemin`, `aria-valuemax`, `aria-valuetext`).
    #[must_use]
    pub fn aria_value_range(mut self, range: ValueRange) -> Self {
        self.common.semantics.range = Some(range);
        self
    }

    /// Its level as a heading, from 1 (`aria-level`; an `h3` is 3).
    #[must_use]
    pub fn aria_level(mut self, level: usize) -> Self {
        self.common.semantics.announce.level = Some(level);
        self
    }

    /// It is the current one of its set (`aria-current`).
    #[must_use]
    pub fn aria_current(mut self, current: AriaCurrent) -> Self {
        self.common.semantics.announce.current = Some(current);
        self
    }

    /// The text it holds, to a screen reader, when it holds text without
    /// being a text input (a pin's slot): an input's `value`.
    #[must_use]
    pub fn aria_value(mut self, value: impl Into<String>) -> Self {
        self.common.semantics.announce.value = Some(value.into());
        self
    }

    /// What its role is called here (`aria-roledescription`).
    #[must_use]
    pub fn aria_role_description(mut self, description: impl Into<String>) -> Self {
        self.common.semantics.announce.role_description = Some(description.into());
        self
    }

    /// What is read after its name (`aria-description`).
    #[must_use]
    pub fn aria_description(mut self, description: impl Into<String>) -> Self {
        self.common.semantics.announce.description = Some(description.into());
        self
    }

    /// How changes inside it are told (`aria-live`).
    #[must_use]
    pub fn aria_live(mut self, live: Live) -> Self {
        self.common.semantics.announce.live = Some(live);
        self
    }

    /// Its user may choose several of its options (`aria-multiselectable`).
    #[must_use]
    pub fn aria_multiselectable(mut self, multiple: bool) -> Self {
        self.common.semantics.announce.multiselectable = multiple;
        self
    }

    /// A form needs it filled (`aria-required`).
    #[must_use]
    pub fn aria_required(mut self, required: bool) -> Self {
        self.common.semantics.announce.required = required;
        self
    }

    /// Its value is not one it accepts (`aria-invalid`).
    #[must_use]
    pub fn aria_invalid(mut self, invalid: bool) -> Self {
        self.common.semantics.announce.invalid = invalid;
        self
    }

    /// Its value is not changed by its user (`aria-readonly`).
    #[must_use]
    pub fn aria_readonly(mut self, read_only: bool) -> Self {
        self.common.semantics.announce.read_only = read_only;
        self
    }

    /// The popup it opens (`aria-haspopup`).
    #[must_use]
    pub fn aria_has_popup(mut self, popup: HasPopup) -> Self {
        self.common.semantics.announce.has_popup = Some(popup);
        self
    }

    /// How typing in it completes (`aria-autocomplete`).
    #[must_use]
    pub fn aria_auto_complete(mut self, complete: AutoComplete) -> Self {
        self.common.semantics.announce.auto_complete = Some(complete);
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

    /// While `on`, let the pointer through this node and everything in it
    /// to what is below (`pointer-events: none` as CSS inherits it): none of
    /// it is hit or hovered. Focus and the screen reader are unchanged. A
    /// splitter's panels while a drag runs.
    #[must_use]
    pub fn pass_through_all(mut self, on: bool) -> Self {
        self.common.pass_through_all = on;
        self
    }

    /// `f` as a handler of this node: run inside the component instance
    /// rendering now, which the node records as its own ([`in_owner`]).
    fn owned<E: ?Sized + 'static, R: Default + 'static>(
        &mut self,
        f: impl Fn(&E) -> R + 'static,
    ) -> Rc<dyn Fn(&E) -> R> {
        let owner = crate::component::handler_owner();
        self.common.handlers.owner = owner;
        in_owner(owner, f)
    }

    /// Called on a click (press and release on this node); bubbles to the
    /// nearest ancestor that has one.
    #[must_use]
    pub fn on_click(mut self, f: impl Fn(&ClickEvent) + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.click = Some(then(self.common.handlers.click.take(), f));
        self
    }

    /// Called when the primary button goes down on this node.
    #[must_use]
    pub fn on_mouse_down(mut self, f: impl Fn(&ClickEvent) + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.mouse_down = Some(then(self.common.handlers.mouse_down.take(), f));
        self
    }

    /// Run `f` with each [`crate::motion::Stage`] this node reaches, after
    /// the layout pass that reached it: laid out (its entry animation
    /// starts), its entry animation ended (at once with none), left the tree
    /// (its exit animation starts), its exit ended (at once with none). The
    /// engine knows when each animation ends, the node's own and not its
    /// children's, under reduced motion too, so the four are exact (reka's
    /// `Presence` events). Leaving, the node keeps reporting, inert. Each
    /// given runs, in the order given.
    #[must_use]
    pub fn on_stage(mut self, f: impl Fn(crate::motion::Stage) + 'static) -> Self {
        let motion = Rc::make_mut(self.common.style.motion.get_or_insert_default());
        let f: Rc<dyn Fn(crate::motion::Stage)> = match motion.report.take() {
            Some(crate::motion::StageReport(before, _)) => Rc::new(move |stage| {
                before(stage);
                f(stage);
            }),
            None => Rc::new(f),
        };
        motion.report = Some(crate::motion::StageReport(f, crate::component::Owner::current()));
        self
    }

    /// Called as the pointer is pressed on this node, moved, and released.
    /// Run `f` with the node's border box in window pixels after each
    /// layout (a table's `Auto` columns learn their cells' widths so).
    #[must_use]
    pub fn on_measured(mut self, f: impl Fn(&Bounds) + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.measured = Some(then(self.common.handlers.measured.take(), f));
        self
    }

    /// Called as the pointer or a finger presses on this node (or a child),
    /// moves and lifts: `Start`, `Move`s, then `End`, or `Cancel` once the
    /// node has been disabled ([`Self::disabled`]), made unavailable
    /// ([`Self::unavailable`]) or gone meanwhile. The node that started a
    /// drag gets its later phases wherever the pointer goes, followed by
    /// its path in the tree: in a list whose items come, go or move, key
    /// each item that takes drags ([`Self::key`]), as an unkeyed rebuild
    /// matches nodes by position (as Vue does) and the drag would follow
    /// whichever item came to its place.
    #[must_use]
    pub fn on_drag(mut self, f: impl Fn(&DragEvent) + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.drag = Some(then(self.common.handlers.drag.take(), f));
        self
    }

    /// Captured with the other nodes of `group`: a pointer that starts
    /// dragging or touching one of them captures them all, as one pointer
    /// captures one node, and another pointer pressing any of them
    /// meanwhile drags or touches nothing there.
    #[must_use]
    pub fn capture_group(mut self, group: crate::events::CaptureGroup) -> Self {
        self.common.handlers.capture_group = Some(group);
        self
    }

    /// Each pointer that presses it drags it and gets its raw touch, each
    /// its own from its start to its end (multi-touch). Without it, as in
    /// Chrome, the pointer that started a drag or a touch of the node has
    /// captured it: another pointer pressing it meanwhile gets nothing of
    /// it, so a control keeps one drag's state with no ownership of its own.
    #[must_use]
    pub fn multi_pointer(mut self) -> Self {
        self.common.handlers.multi_pointer = true;
        self
    }

    /// Called with each key pressed while focus is inside this node; return
    /// `true` when handled.
    #[must_use]
    pub fn on_key_down(mut self, f: impl Fn(&KeyEvent) -> bool + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.key_down = Some(or_consumed(self.common.handlers.key_down.take(), f));
        self
    }

    /// Called with each key pressed in the window that the focused node
    /// and its ancestors did not take, wherever focus is (a document's
    /// `keydown` listener): a shortcut to a region (the toasts' F8). Return
    /// `true` when handled. A node below a modal layer is not told.
    #[must_use]
    pub fn on_window_key(mut self, f: impl Fn(&KeyEvent) -> bool + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.window_key = Some(or_consumed(self.common.handlers.window_key.take(), f));
        self
    }

    /// Called when the secondary (right) button goes down on this node; bubbles.
    #[must_use]
    pub fn on_secondary_click(mut self, f: impl Fn(&ClickEvent) + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.secondary_click = Some(then(self.common.handlers.secondary_click.take(), f));
        self
    }

    /// Called with `true` when the pointer enters this node and `false` when it leaves.
    #[must_use]
    pub fn on_hover(mut self, f: impl Fn(bool) + 'static) -> Self {
        let f = self.owned(move |on: &bool| f(*on));
        let before = self.common.handlers.hover.take();
        self.common.handlers.hover = Some(Rc::new(move |on| {
            if let Some(b) = &before {
                b(on);
            }
            f(&on);
        }));
        self
    }

    /// Called on each move of the pointer over this node (or a child), with
    /// where it is; [`Self::on_hover`] says when it leaves.
    #[must_use]
    pub fn on_pointer_move(mut self, f: impl Fn(&crate::events::PointerEvent) + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.pointer = Some(then(self.common.handlers.pointer.take(), f));
        self
    }

    /// Called with the wheel over this node (or a child); returns whether
    /// it consumed it. Consumed, it scrolls nothing above; declined, it goes
    /// on to the next handler or scroll container up (a field that steps
    /// only while focused). Over a node with one, the compositor hands the
    /// wheel to the UI thread instead of scrolling it itself (Chromium's
    /// wheel event handler region).
    #[must_use]
    pub fn on_wheel(mut self, f: impl Fn(&crate::events::WheelEvent) -> bool + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.wheel = Some(or_consumed(self.common.handlers.wheel.take(), f));
        self
    }

    /// Whether a primary press with the modifiers held focuses this node,
    /// asked when the press would focus it; refused, focus stays where it
    /// is (a `mousedown` whose default is prevented). Every press focuses
    /// a node without one.
    #[must_use]
    pub fn focus_on_press(mut self, f: impl Fn(&crate::events::Modifiers) -> bool + 'static) -> Self {
        let f = self.owned(f);
        // Each one registered may refuse: it focuses when all allow it.
        self.common.handlers.press_focus = Some(match self.common.handlers.press_focus.take() {
            Some(before) => Rc::new(move |m| before(m) && f(m)),
            None => f,
        });
        self
    }

    /// A primary press on this node, not on a node inside it that takes
    /// focus, focuses what `f` returns, as a press on a node that takes
    /// focus focuses it; none (or a handle no node holds) leaves focus where
    /// it is. A box around segments focuses its first segment, as Chrome's
    /// date and time inputs do. It is focus, not an action: it runs on an
    /// unavailable node ([`Self::unavailable`]), whose own press handlers do
    /// not, and not on a disabled one.
    #[must_use]
    pub fn press_focuses(mut self, f: impl Fn() -> Option<crate::focus::FocusHandle> + 'static) -> Self {
        let f = self.owned(move |(): &()| f());
        // The first registered that names a focus has it.
        let before = self.common.handlers.press_focuses.take();
        self.common.handlers.press_focuses =
            Some(Rc::new(move || before.as_ref().and_then(|b| b()).or_else(|| f(&()))));
        self
    }

    /// Called when a touch or pen is held on this node (or a child) for
    /// [`crate::gesture::LONG_PRESS`] without moving past
    /// [`crate::gesture::SLOP`]; bubbles. With none, a long press runs the
    /// nearest [`Self::on_secondary_click`], as a platform opens a context
    /// menu on it.
    #[must_use]
    pub fn on_long_press(mut self, f: impl Fn(&ClickEvent) + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.long_press = Some(then(self.common.handlers.long_press.take(), f));
        self
    }

    /// Hold a touch or pen `after` (not [`crate::gesture::LONG_PRESS`])
    /// before this node's [`Self::on_long_press`] runs: reka's context
    /// menu's `pressOpenDelay`. Read as the contact goes down.
    #[must_use]
    pub fn long_press_after(mut self, after: std::time::Duration) -> Self {
        self.common.handlers.long_press_after = Some(after);
        self
    }

    /// Called when a primary press (a mouse's, or a touch or pen's after its
    /// long press) that went down outside this node is released over it
    /// (or a child); bubbles to the nearest node with one. A press and
    /// release on the node itself is its click instead. A menu item picks
    /// on it: pressing a menubar trigger, dragging to an item and
    /// releasing picks it (reka's `MenuItem` `pointerup`).
    #[must_use]
    pub fn on_release(mut self, f: impl Fn(&ClickEvent) + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.release = Some(then(self.common.handlers.release.take(), f));
        self
    }

    /// Called with each phase of the touch or pen contact that went down
    /// on this node (or a child), the first finger only, from `Started` to
    /// `Ended` or `Cancelled`, wherever it moves; bubbles to the nearest
    /// node with one, found as the contact goes down and kept until it
    /// lifts (a captured pointer). Disabled meanwhile ([`Self::disabled`]),
    /// the node gets `Cancelled` once and nothing after. The engine's
    /// gestures run as well: `Ended` is told
    /// after what the contact turned out to be (a tap's press and click)
    /// has run. A held button that repeats reads it (reka's
    /// `usePressedHold`).
    #[must_use]
    pub fn on_touch(mut self, f: impl Fn(&crate::events::TouchEvent) + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.touch = Some(then(self.common.handlers.touch.take(), f));
        self
    }

    /// Called with what an input method composes and commits while focus
    /// is on this node (or inside it, on a node that is not a text input):
    /// an input method may type into it (a pin's slot, a one-time code), its
    /// candidates placed at the focused node. A text input composes by
    /// itself and does not call it.
    #[must_use]
    pub fn on_ime(mut self, f: impl Fn(&crate::events::ImeEvent) + 'static) -> Self {
        let f = self.owned(f);
        self.common.handlers.ime = Some(then(self.common.handlers.ime.take(), f));
        self
    }

    /// Make this node the region of a floating layer the engine keeps
    /// (ADR-0024): presses and focus outside it, Escape and scrolls ask
    /// `layer`'s owner to close it, a modal one blocks what is below, and
    /// its focus scope keeps focus. Give it to the node a [`deferred`]
    /// subtree dismisses as (a dialog's content, not its overlay). With a
    /// focus scope it takes focus itself as a last resort (no Tab stop).
    #[must_use]
    pub fn layer(mut self, layer: crate::layer::Layer) -> Self {
        if layer.scope.is_some() && self.common.focus.is_none() {
            self.common.focusable = true;
        }
        self.common.handlers.layer = Some(Rc::new(layer));
        self
    }

    /// Make this node and its subtree inert (HTML's `inert`), the deferred
    /// subtrees declared in it included: nothing in it is hit by the
    /// pointer, takes focus, is a layer, or is read by a screen reader. It
    /// still lays out and paints.
    #[must_use]
    pub fn inert(mut self, inert: bool) -> Self {
        self.common.inert = inert;
        self
    }

    /// Make this node and its subtree dormant: laid out but not shown
    /// (not painted, hit, focused, given keys or read by a screen reader),
    /// and still members of their lists ([`crate::Members`]), which their
    /// owner walks while they sleep: a closed select's options (typeahead
    /// and the arrows choose among them), a carousel's slides out of view.
    /// A member hidden for any other reason (`invisible`, inert, an inline
    /// a clamp hides) is in no list, its owner's included.
    #[must_use]
    pub fn dormant(mut self, dormant: bool) -> Self {
        self.common.dormant = dormant;
        self
    }

    /// Name this node with `node`, for other nodes' relations to it.
    #[must_use]
    pub fn node_ref(mut self, node: crate::access::NodeRef) -> Self {
        self.common.semantics.node = Some(node);
        self
    }

    /// The node whose text names this one (`aria-labelledby`); several add up.
    #[must_use]
    pub fn labelled_by(mut self, node: crate::access::NodeRef) -> Self {
        self.common.semantics.relations.labelled_by.push(node);
        self
    }

    /// The node whose text describes this one (`aria-describedby`); several add up.
    #[must_use]
    pub fn described_by(mut self, node: crate::access::NodeRef) -> Self {
        self.common.semantics.relations.described_by.push(node);
        self
    }

    /// The node this one controls (`aria-controls`: a trigger's popup).
    #[must_use]
    pub fn controls(mut self, node: crate::access::NodeRef) -> Self {
        self.common.semantics.relations.controls.push(node);
        self
    }

    /// The node this one owns though it is elsewhere in the tree (`aria-owns`).
    #[must_use]
    pub fn owns(mut self, node: crate::access::NodeRef) -> Self {
        self.common.semantics.relations.owns.push(node);
        self
    }

    /// The descendant that is active while this node keeps focus
    /// (`aria-activedescendant`: a listbox's highlighted option).
    #[must_use]
    pub fn active_descendant(mut self, node: Option<crate::access::NodeRef>) -> Self {
        self.common.semantics.relations.active_descendant = node;
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
        anchor_part: None,
        tab_after: None,
        child: child.into_element(),
    }
}

/// Anything that renders to an element; implement it for a component struct.
pub trait View {
    fn render(&self) -> Element;
}
