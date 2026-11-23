//! Input (`ui/input`): a one-line text field, and the same frame around a
//! time input (`type="time"`) and a file picker (`type="file"`).
//!
//! As shadcn-vue's `Input` (`useVModel(props, 'modelValue', emits, {
//! passive: true, defaultValue })`), [`input`] keeps its own text
//! (`default_value`) or the caller's (`model`, `v-model`); `on_change`
//! hears each edit. What it edits with (the caret, the selection, an IME's
//! composition, focus) is the instance's own, unless the caller hands it a
//! [`TextField`] to share (`field`), as a form or an input group does.

use std::rc::Rc;

use gui::prelude::*;
use gui::value::Bindings;
use gui::{
    Bind, Color, Div, Element, InputType, Interaction, Model, Properties, Signal, Slot, Style, TextField, TextInput,
    component, div, text, text_input, use_model,
};
use primitives::Availability;
use primitives::date::DateStrings;
use primitives::date_field::{HourCycle, Piece, Precision};
use primitives::direction::ReadDirection;
use primitives::time_field::{Time, TimeField, TimeFieldOptions};

use crate::focus::StateLook;
use crate::focus::focus_ring;
use crate::theme::{Colors, theme};

/// A one-line text input (`<Input>`): empty, and keeping its own text, until
/// told otherwise. Style it as any node; its own classes come first.
#[must_use]
pub fn input() -> Input {
    let t = theme();
    Input::new("input", frame(Style::default(), &t))
}

/// A text input being built: [`input`] or [`crate::textarea`].
#[derive(Clone)]
pub struct Input {
    value: Bind<String>,
    field: Option<TextField>,
    name: &'static str,
    style: Style,
    /// The closures given to its setters, bound to the text input's node.
    bindings: Bindings,
    placeholder: String,
    input_type: InputType,
    read_only: bool,
    disabled: bool,
    multiline: bool,
    refine: Option<Slot<Refine>>,
}

/// A caller's change to the text input, after its own styles.
type Refine = Rc<dyn Fn(TextInput) -> TextInput>;

impl Input {
    pub(crate) fn new(name: &'static str, style: Style) -> Self {
        Self {
            value: Bind::default(),
            field: None,
            name,
            // A browser's own `input, textarea { text-align: start }`, and its text cursor.
            style: style.text_align(TextAlign::Left).cursor(Cursor::Text),
            bindings: Bindings::default(),
            placeholder: String::new(),
            input_type: InputType::Text,
            read_only: false,
            disabled: false,
            multiline: false,
            refine: None,
        }
    }

    pub(crate) fn multiline(mut self) -> Self {
        self.multiline = true;
        self
    }

    /// The text, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(mut self, model: impl Into<Model<String>>) -> Self {
        self.value = self.value.with_model(model);
        self
    }

    /// The text it starts with when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(mut self, value: impl Into<String>) -> Self {
        self.value = self.value.with_default(value.into());
        self
    }

    /// Call `f` with the text after each edit (`@update:model-value`).
    #[must_use]
    pub fn on_change(mut self, f: impl Into<gui::Callback<String>>) -> Self {
        self.value = self.value.on_change(f);
        self
    }

    /// Edit `field`, which the caller keeps: its text, caret and focus are
    /// the input's (a template ref on the element, as a form library or an
    /// input group takes one).
    #[must_use]
    pub fn field(mut self, field: TextField) -> Self {
        self.field = Some(field);
        self
    }

    #[must_use]
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }

    /// `type="password"`: a bullet per character.
    #[must_use]
    pub fn input_type(mut self, input_type: InputType) -> Self {
        self.input_type = input_type;
        self
    }

    /// `readonly`: selected and copied, not edited.
    #[must_use]
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The name the debug tools and scripts find it by.
    #[must_use]
    pub fn named(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    /// Change the input as a node after its own classes and states (a state
    /// style, a key handler), each render.
    #[must_use]
    pub fn map(mut self, f: impl Fn(TextInput) -> TextInput + 'static) -> Self {
        let f: Refine = match self.refine.take() {
            Some(before) => Rc::new(move |el| f(before(el))),
            None => Rc::new(f),
        };
        self.refine = Some(Slot(f));
        self
    }

    fn render(&self, field: &TextField) -> TextInput {
        let t = theme();
        let mut el = text_input(field);
        let (style, bindings) = el.style_and_bindings();
        *style = self.style.clone();
        if let Some(bindings) = bindings {
            *bindings = self.bindings.clone();
        }
        let el = el
            .named(self.name)
            .placeholder(self.placeholder.clone())
            .placeholder_color(t.muted_foreground)
            .selection_color(t.primary.opacity(0.35))
            .input_type(self.input_type)
            .disabled(self.disabled)
            .with_interaction(|d, i| d.focus_ring_on(i, &t).dim_disabled(i, 0.5).cursor_disabled(i))
            .when(self.multiline, TextInput::multiline)
            .when(self.read_only, TextInput::read_only);
        match &self.refine {
            Some(f) => f(el),
            None => el,
        }
    }
}

impl Fluent for Input {}

impl Styled for Input {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut Bindings>) {
        (&mut self.style, Some(&mut self.bindings))
    }
}

/// Props compare by value; bound properties cannot be compared, so an
/// input with any re-renders with its parent.
impl PartialEq for Input {
    fn eq(&self, other: &Self) -> bool {
        self.bindings.is_empty()
            && other.bindings.is_empty()
            && self.value == other.value
            && self.field == other.field
            && self.name == other.name
            && self.style == other.style
            && self.placeholder == other.placeholder
            && self.input_type == other.input_type
            && self.read_only == other.read_only
            && self.disabled == other.disabled
            && self.multiline == other.multiline
            && self.refine == other.refine
    }
}

impl IntoElement for Input {
    fn into_element(self) -> Element {
        let name = self.name;
        component(self, |cx| {
            let props = cx.props();
            let value = use_model(props, |p: &Input| &p.value);
            let field = props
                .with_untracked(|p| p.field)
                .unwrap_or_else(|| TextField::new(value.get_untracked()));
            crate::part::bind_field(field, value);
            move || props.with(|p| p.render(&field)).into_element()
        })
        .named(name)
        .into_element()
    }
}

/// A time input (`<Input type="time" step="1">`, its picker indicator
/// hidden as shadcn's demos hide it): the hour, minute and second fields in
/// the input's frame, the ring showing while one holds focus. Give it its
/// value as a [`TimeInput`]: `.model(…)` or `.default_value("10:30:00")`.
#[must_use]
pub fn input_time() -> TimeInput {
    let t = theme();
    // Chrome's `light-dark(highlight, …)` for the focused field, as it draws
    // it on the reference's macOS (measured: the light page's highlight, and
    // the dark page's).
    let highlight = if t.dark {
        Color::hex(0x0055_79a1)
    } else {
        Color::hex(0x00b3_d7fe)
    };
    frame(time_input(), &t)
        .map_field(move |d, i| {
            d.background_while(move || i.focused(), highlight)
                .text_color_while(move || i.focused(), Color::BLACK)
        })
        // `focus-visible:` matches the input while one of its fields holds focus.
        .map_root(|d| {
            d.with_interaction(|d, i| d.ring_while(move || i.focus_within(), &focus_ring(Style::default(), &t)))
        })
}

/// A file input showing the file in `chosen`: the browser's "Choose File"
/// button, styled by the `file:` classes (`inline-flex h-7 text-sm
/// font-medium text-foreground`, 4px before the name), then the file's name.
/// `gui` has no native file dialog yet, so a click picks nothing; whoever
/// opens one sets `chosen`.
#[must_use]
pub fn input_file(chosen: Signal<Option<String>>) -> Div {
    let t = theme();
    // One line from the content box's top: the 28px button overhangs the
    // 26px box below, and the name sits on the button's baseline.
    let line = || div().height(28.0).display(Display::Flex).align_items(Align::Center);
    frame(div().named("input-file"), &t)
        .display(Display::Flex)
        .align_items(Align::Start)
        .focusable()
        .with_interaction(|d, i| d.focus_ring_on(i, &t).dim_disabled(i, 0.5).cursor_disabled(i))
        .child(line().margin_right(4.0).font_weight(500).child("Choose File"))
        .child(line().child(text(chosen.get().unwrap_or_else(|| "No file chosen".to_owned()))))
}

/// The box all three draw: `border-input h-9 w-full min-w-0 rounded-md
/// border bg-transparent px-3 py-1 md:text-sm shadow-xs dark:bg-input/30`.
fn frame<S: Styled>(el: S, t: &Colors) -> S {
    el.transition(Properties::COLOR | Properties::SHADOW)
        .text_color(t.foreground)
        .border_color(t.input)
        .height(36.0)
        .width(pct(100.0))
        .min_width(0.0)
        .radius(8.0)
        .border(1.0)
        .background(if t.dark {
            t.input.opacity(0.3)
        } else {
            Color::TRANSPARENT
        })
        .padding_x(12.0)
        .padding_y(4.0)
        .text_size(14.0)
        .line_height(20.0)
        .shadow(Shadow {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: 0.0,
            color: Color::BLACK.opacity(0.05),
        })
}

/// A time input being built ([`input_time`]): Chrome's hour, minute and
/// second segments (`primitives::time_field`, which holds how they step and
/// are typed), drawn in a box the caller styles as a `Div`. Its value is a
/// [`Time`] to the second, the segments as they stand (`Time::value` is the
/// `HH:MM:SS` the input's `value` reads, empty until every segment has one),
/// its own (`default_value`) or the caller's (`model`, `v-model`). It tells
/// the time on 24 hours, as the reference's Chrome draws `type="time"`.
#[derive(Clone)]
pub struct TimeInput {
    value: Bind<Time>,
    root: Div,
    field: Option<SegmentLook>,
    strings: Option<DateStrings>,
    availability: Rc<dyn Fn() -> Availability>,
    read_only: bool,
}

/// What the caller adds to each segment's box, given its [`Interaction`].
type SegmentLook = Rc<dyn Fn(Div, Interaction) -> Div>;

/// Chrome sizes each segment 2 px × `devicePixelRatio` wider than the
/// widest value it can show (`DateTimeFieldElement::MaximumWidth`), while
/// drawing it with 1 px of padding a side: at the 2x a Retina screen and the
/// reference draw at, 2 px a side, the rest left empty after the segments.
const SIZING_PADDING: f32 = 2.0;

/// A time input: Chrome's segments, empty until given a value.
fn time_input() -> TimeInput {
    TimeInput {
        value: Bind::default().with_default(Time::empty(Precision::Second)),
        root: div()
            .named("time-input")
            .display(Display::Flex)
            .align_items(Align::Center),
        field: None,
        strings: None,
        availability: Rc::new(|| Availability::Enabled),
        read_only: false,
    }
}

/// Built anew by each render of its parent, it is never equal: the instance
/// renders again with what its parent passes now.
impl PartialEq for TimeInput {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl TimeInput {
    /// The value, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(mut self, model: impl Into<Model<Time>>) -> Self {
        self.value = self.value.with_model(model);
        self
    }

    /// The value it starts at when no model is given (`default-value`):
    /// a [`Time`], or its `HH:MM:SS`.
    #[must_use]
    pub fn default_value(mut self, value: impl Into<Time>) -> Self {
        self.value = self.value.with_default(value.into());
        self
    }

    /// Call `f` with each value written (`@update:model-value`), partly
    /// typed ones included.
    #[must_use]
    pub fn on_change(mut self, f: impl Into<gui::Callback<Time>>) -> Self {
        self.value = self.value.on_change(f);
        self
    }

    /// `disabled`: no focus, no keys, not sent with a form.
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.availability(Availability::from_disabled(disabled))
    }

    /// Whether it can be used ([`Availability`]), a value or a closure
    /// (read tracked, so it follows the signals it reads): an unavailable
    /// one keeps focus and moves between its segments, but is not changed.
    /// Give its `FormField` the same value or closure
    /// (`FormField::availability`), so the form and the input agree.
    #[must_use]
    pub fn availability<M>(mut self, availability: impl gui::value::IntoValue<Availability, M>) -> Self {
        self.availability = match availability.into_value() {
            gui::value::Value::Fixed(a) => Rc::new(move || a),
            gui::value::Value::Bound(f) => f,
        };
        self
    }

    /// `readonly`: focused and moved through, its value not changed.
    #[must_use]
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// What a screen reader calls each segment, and an empty one (the
    /// locale's, Chrome's English strings, by default).
    #[must_use]
    pub fn strings(mut self, strings: DateStrings) -> Self {
        self.strings = Some(strings);
        self
    }

    /// Add to each segment's box, given the [`Interaction`] its bindings
    /// read: the segment holding focus is the one whose `i.focused()` holds
    /// (`::-webkit-datetime-edit-hour-field:focus`).
    #[must_use]
    pub fn map_field(mut self, f: impl Fn(Div, Interaction) -> Div + 'static) -> Self {
        self.field = Some(Rc::new(f));
        self
    }

    /// Change the box as a `Div`.
    #[must_use]
    pub fn map_root(mut self, f: impl FnOnce(Div) -> Div) -> Self {
        self.root = f(self.root);
        self
    }
}

impl Fluent for TimeInput {}

impl Styled for TimeInput {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut Bindings>) {
        self.root.style_and_bindings()
    }
}

impl IntoElement for TimeInput {
    fn into_element(self) -> Element {
        component(self, |cx| {
            let props = cx.props();
            let value = use_model(props, |p: &TimeInput| &p.value);
            let segments = TimeField::new(
                value,
                move || {
                    props.with(|p| TimeFieldOptions {
                        availability: (p.availability)(),
                        readonly: p.read_only,
                        hour_cycle: Some(HourCycle::H23),
                        strings: p.strings.clone(),
                        ..TimeFieldOptions::default()
                    })
                },
                ReadDirection::fixed(gui::Direction::Ltr),
            );
            move || props.with(|p| render_time(segments, p)).into_element()
        })
        .named("time-input")
        .into_element()
    }
}

/// The segments in a box as wide as Chrome makes a time input's.
fn render_time(segments: TimeField<TimeInput>, p: &TimeInput) -> Div {
    let mut fields = div()
        .display(Display::Flex)
        .align_items(Align::Center)
        .grid_row_start(1)
        .grid_column_start(1);
    let mut sizer = div()
        .display(Display::Flex)
        .align_items(Align::Center)
        .grid_row_start(1)
        .grid_column_start(1)
        .invisible(true);
    for piece in segments.pieces() {
        let s = match piece {
            Piece::Literal(t) => {
                // Hidden from assistive technology, as the field's literals are.
                fields = fields.child(text(t).aria_hidden());
                sizer = sizer.child(text(t));
                continue;
            }
            Piece::Segment(s) => s,
        };
        let shown = segments.text(s);
        let it = Interaction::new();
        let one = segments.segment(s, div().interaction(it).padding(1.0).child(shown.clone()));
        fields = fields.child(match &p.field {
            Some(f) => f(one, it),
            None => one,
        });
        // The widest of what the segment can show, stacked in one cell.
        let max = segments.bounds(s).1;
        let widest = ["--".to_owned(), format!("{max:02}"), shown]
            .map(|t| text(t).grid_row_start(1).grid_column_start(1).into_element());
        sizer = sizer.child(div().display(Display::Grid).padding_x(SIZING_PADDING).children(widest));
    }
    // `disabled:opacity-50 disabled:cursor-not-allowed`, unavailable too.
    let root = if segments.availability().dimmed() {
        p.root.clone().opacity(0.5).cursor(Cursor::NotAllowed)
    } else {
        p.root.clone()
    };
    segments
        .root(root)
        .child(div().display(Display::Grid).child(sizer).child(fields))
}
