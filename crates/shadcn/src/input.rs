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
use gui::{
    Bind, Color, Div, Element, InputType, Model, Properties, Signal, Slot, Style, TextField, TextInput, TimeInput,
    component, div, text, text_input, time_input, use_model,
};

use crate::button_group::with_style;
use crate::focus::focus_ring;
use crate::theme::{Theme, theme};

/// A one-line text input (`<Input>`): empty, and keeping its own text, until
/// told otherwise. Style it as any node; its own classes come first.
#[must_use]
pub fn input() -> Input {
    let t = theme();
    Input::new("input", frame(Style::default(), &t))
}

/// A text input being built: [`input`] or [`crate::textarea`].
#[derive(Clone, PartialEq)]
pub struct Input {
    value: Bind<String>,
    field: Option<TextField>,
    name: &'static str,
    style: Style,
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
            style: style.text_left().cursor_text(),
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
    pub fn on_change(mut self, f: impl Fn(String) + 'static) -> Self {
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
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
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
        *el.style_mut() = self.style.clone();
        let el = el
            .named(self.name)
            .placeholder(self.placeholder.clone())
            .placeholder_color(t.muted_foreground)
            .selection_color(t.primary.opacity(0.35))
            .input_type(self.input_type)
            .disabled(self.disabled)
            .focus_visible(move |s| focus_ring(s, &t))
            .disabled_style(|s| s.opacity(0.5).cursor_not_allowed())
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
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
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
        .focused_field(move |s| s.bg(highlight).text_color(Color::BLACK))
        // `focus-visible:` matches the input while one of its fields holds focus.
        .focus_within(move |d| with_style(d, |s| focus_ring(s, &t)))
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
    let line = || div().h(7.0).flex().items_center();
    frame(div().named("input-file"), &t)
        .flex()
        .items_start()
        .focusable()
        .focus_visible(move |s| focus_ring(s, &t))
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed())
        .child(line().mr(1.0).font_medium().child("Choose File"))
        .child(line().child(text(chosen.get().unwrap_or_else(|| "No file chosen".to_owned()))))
}

/// The box all three draw: `border-input h-9 w-full min-w-0 rounded-md
/// border bg-transparent px-3 py-1 md:text-sm shadow-xs dark:bg-input/30`.
fn frame<S: Styled>(el: S, t: &Theme) -> S {
    el.transition_props(Properties::COLOR | Properties::SHADOW)
        .text_color(t.foreground)
        .border_color(t.input)
        .h(9.0)
        .w_full()
        .min_w(0.0)
        .rounded_md()
        .border()
        .bg(if t.dark {
            t.input.opacity(0.3)
        } else {
            Color::TRANSPARENT
        })
        .px(3.0)
        .py(1.0)
        .text_sm()
        .shadow_xs()
}
