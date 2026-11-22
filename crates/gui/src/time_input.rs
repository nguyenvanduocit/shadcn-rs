//! A time input (`<input type="time" step="1">`) built as Chrome builds its
//! own: an hour, a minute and a second field, each focusable (Tab and the
//! arrow keys move between them), stepped with ArrowUp and ArrowDown and
//! typed digit by digit; a 24-hour clock, as Chrome shows it on the machine
//! the reference is drawn on.
//!
//! [`time_input`] is a component instance holding its fields; its value
//! (`HH:MM:SS`, empty until every field has one, as the input's `value` is)
//! is its own (`default_value`) or the caller's (`model`, `v-model`).

use std::rc::Rc;

use crate::component::{Bind, Model, component, use_model};
use crate::element::{Div, Element, IntoElement, div, text};
use crate::events::{KeyEvent, NamedKey};
use crate::focus::{self, FocusHandle};
use crate::interact::Interaction;
use crate::prelude::*;
use crate::reactive::{Signal, watch};
use crate::style::Style;

/// The largest value of each field: hour, minute, second.
const MAX: [u8; 3] = [23, 59, 59];

/// What an empty field shows.
const PLACEHOLDER: &str = "--";

/// Chrome sizes each field 2 px × `devicePixelRatio` wider than the widest
/// value it can show (`DateTimeFieldElement::MaximumWidth`), while drawing
/// it with 1 px of padding a side: at the 2x a Retina screen and the
/// reference draw at, 2 px a side, the rest left empty after the fields.
const SIZING_PADDING: f32 = 2.0;

/// What an instance holds: each field's value and focus.
#[derive(Clone, Copy)]
struct Fields {
    /// Hour, minute and second; `None` shows `--`.
    values: Signal<[Option<u8>; 3]>,
    /// The digits typed into a field since it took focus (Chrome's
    /// type-ahead): the field, their number, and the number they spell.
    typed: Signal<Option<(usize, u8, u8)>>,
    focus: [FocusHandle; 3],
}

/// `value` (`HH:MM:SS`) as the fields' values; a part missing, out of range
/// or `--` leaves its field empty.
fn parse(value: &str) -> [Option<u8>; 3] {
    let mut parts = value.split(':');
    MAX.map(|max| parts.next().and_then(|p| p.parse::<u8>().ok()).filter(|v| *v <= max))
}

/// The value of `values` once every field has one, else empty.
fn written(values: [Option<u8>; 3]) -> String {
    match values {
        [Some(h), Some(m), Some(s)] => format!("{h:02}:{m:02}:{s:02}"),
        _ => String::new(),
    }
}

impl Fields {
    /// Fields holding `value` (`HH:MM:SS`).
    fn new(value: &str) -> Self {
        Self {
            values: Signal::labeled("time", parse(value)),
            typed: Signal::labeled("time typed", None),
            focus: [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()],
        }
    }

    /// Whether one of its fields holds focus (reactive).
    fn focused(&self) -> bool {
        self.focus.iter().any(|f| f.is_focused())
    }

    fn get(&self, i: usize) -> Option<u8> {
        self.values.get().get(i).copied().flatten()
    }

    fn set(&self, i: usize, value: Option<u8>) {
        self.values.update(|v| {
            if let Some(slot) = v.get_mut(i) {
                *slot = value;
            }
        });
    }

    /// Focus field `i`; false when there is none.
    fn focus_field(&self, i: Option<usize>) -> bool {
        i.and_then(|i| self.focus.get(i)).map(|f| f.focus()).is_some()
    }

    /// One up (or down) from the value, wrapping; an empty field takes its
    /// lowest (or highest) value.
    fn step(&self, i: usize, up: bool) {
        let max = MAX.get(i).copied().unwrap_or(0);
        let next = match (self.get(i), up) {
            (None, true) => 0,
            (None, false) => max,
            (Some(v), true) if v >= max => 0,
            (Some(v), true) => v.saturating_add(1),
            (Some(v), false) => v.checked_sub(1).unwrap_or(max),
        };
        self.typed.set(None);
        self.set(i, Some(next));
    }

    /// Chrome's type-ahead: a digit joins the one typed before it, and the
    /// field moves on once it has two or no third could follow.
    fn type_digit(&self, i: usize, digit: u8) {
        let max = MAX.get(i).copied().unwrap_or(0);
        // Of a full buffer only the last digit stays; of one digit, that digit.
        let (before, count) = match self.typed.get() {
            Some((field, count, spelled)) if field == i => (
                if count >= 2 {
                    spelled.checked_rem(10).unwrap_or(0)
                } else {
                    spelled
                },
                2,
            ),
            _ => (0, 1),
        };
        let spelled = before.saturating_mul(10).saturating_add(digit);
        self.set(i, Some(spelled.min(max)));
        let done = count >= 2 || u16::from(spelled).saturating_mul(10) > u16::from(max);
        if !(done && self.focus_field(i.checked_add(1).filter(|n| *n < MAX.len()))) {
            self.typed.set(Some((i, count, spelled)));
        }
    }

    /// What a key does in field `i`; whether it took it.
    fn key(&self, i: usize, ev: &KeyEvent) -> bool {
        if ev.is(NamedKey::ArrowLeft) {
            return self.focus_field(i.checked_sub(1));
        }
        if ev.is(NamedKey::ArrowRight) {
            return self.focus_field(i.checked_add(1).filter(|n| *n < MAX.len()));
        }
        if ev.is(NamedKey::ArrowUp) || ev.is(NamedKey::ArrowDown) {
            self.step(i, ev.is(NamedKey::ArrowUp));
            return true;
        }
        if ev.is(NamedKey::Backspace) || ev.is(NamedKey::Delete) {
            self.typed.set(None);
            self.set(i, None);
            return true;
        }
        let digit = ev
            .text
            .as_deref()
            .and_then(|t| t.parse::<u8>().ok())
            .filter(|d| *d <= 9);
        match digit {
            Some(d) if !ev.modifiers.primary() => {
                self.type_digit(i, d);
                true
            }
            _ => false,
        }
    }
}

fn shown(value: Option<u8>) -> String {
    value.map_or_else(|| PLACEHOLDER.to_owned(), |v| format!("{v:02}"))
}

/// A time input: Chrome's fields, empty until given a value. Style the box
/// as a `Div` (the caller gives it its frame: border, padding, height).
#[must_use]
pub fn time_input() -> TimeInput {
    TimeInput {
        value: Bind::default(),
        root: div()
            .named("time-input")
            .display(Display::Flex)
            .align_items(Align::Center),
        field: None,
    }
}

/// A [`time_input`] being built.
#[derive(Clone)]
pub struct TimeInput {
    value: Bind<String>,
    root: Div,
    field: Option<FieldLook>,
}

/// What the caller adds to each field's box, given the field's [`Interaction`].
type FieldLook = Rc<dyn Fn(Div, Interaction) -> Div>;

/// Built anew by each render of its parent, it is never equal: the instance
/// renders again with what its parent passes now.
impl PartialEq for TimeInput {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl TimeInput {
    /// The value (`HH:MM:SS`, empty while a field is), kept by the caller
    /// (`v-model`).
    #[must_use]
    pub fn model(mut self, model: impl Into<Model<String>>) -> Self {
        self.value = self.value.with_model(model);
        self
    }

    /// The value it starts at when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(mut self, value: impl Into<String>) -> Self {
        self.value = self.value.with_default(value.into());
        self
    }

    /// Call `f` with each value written (`@update:model-value`).
    #[must_use]
    pub fn on_change(mut self, f: impl Fn(String) + 'static) -> Self {
        self.value = self.value.on_change(f);
        self
    }

    /// Add to each field's box, given the [`Interaction`] its bindings
    /// read: the field holding focus is the one whose `i.focused()` holds
    /// (`::-webkit-datetime-edit-hour-field:focus`). The box holds focus
    /// within while a field does: give it an [`Interaction`] of its own
    /// with [`Self::map_root`] (`:focus-within`).
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
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut crate::value::Bindings>) {
        self.root.style_and_bindings()
    }
}

impl IntoElement for TimeInput {
    fn into_element(self) -> Element {
        component(self, |cx| {
            let props = cx.props();
            let value = use_model(props, |p: &TimeInput| &p.value);
            let fields = Fields::new(&value.get_untracked());
            // Forget what was typed whenever focus moves, as Chrome does on blur.
            watch(focus::focused, move |_, _| fields.typed.set(None));
            // The fields into the value: empty until each has one.
            watch(
                move || written(fields.values.get()),
                move |now, _| value.set(now.clone()),
            );
            // A value written from outside into the fields.
            watch(
                move || value.get(),
                move |v, _| {
                    if written(fields.values.get_untracked()) != *v {
                        fields.typed.set(None);
                        fields.values.set(parse(v));
                    }
                },
            );
            move || props.with(|p| render(&fields, p)).into_element()
        })
        .named("time-input")
        .into_element()
    }
}

/// The fields in a box as wide as Chrome makes a time input's.
fn render(field: &Fields, p: &TimeInput) -> Div {
    let values = field.values.get();
    let between = || text(":");
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
    for (i, (value, max)) in values.into_iter().zip(MAX).enumerate() {
        if i > 0 {
            fields = fields.child(between());
            sizer = sizer.child(between());
        }
        let me = *field;
        let handle = field.focus.get(i).copied().unwrap_or_default();
        let it = Interaction::new();
        let one = div()
            .interaction(it)
            .padding(1.0)
            .track_focus(handle)
            .on_key_down(move |ev| me.key(i, ev))
            .child(shown(value));
        fields = fields.child(match &p.field {
            Some(f) => f(one, it),
            None => one,
        });
        // The widest of what the field can show, stacked in one cell.
        let widest = [PLACEHOLDER.to_owned(), format!("{max:02}"), shown(value)]
            .map(|t| text(t).grid_row_start(1).grid_column_start(1).into_element());
        sizer = sizer.child(div().display(Display::Grid).padding_x(SIZING_PADDING).children(widest));
    }
    let first = *field;
    p.root
        .clone()
        .on_mouse_down(move |_| {
            // A press beside the fields focuses the first, as Chrome's does.
            if !first.focused() {
                first.focus_field(Some(0));
            }
        })
        .child(div().display(Display::Grid).child(sizer).child(fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Key, Modifiers};

    fn press(f: &Fields, i: usize, key: NamedKey) -> bool {
        f.key(
            i,
            &KeyEvent {
                key: Key::Named(key),
                text: None,
                modifiers: Modifiers::default(),
            },
        )
    }

    fn type_in(f: &Fields, i: usize, digits: &str) {
        for c in digits.chars() {
            let t = c.to_string();
            f.key(
                i,
                &KeyEvent {
                    key: Key::Character(t.as_str().into()),
                    text: Some(t),
                    modifiers: Modifiers::default(),
                },
            );
        }
    }

    #[test]
    fn arrows_step_and_wrap_and_digits_type_ahead_as_chrome_does() {
        let f = Fields::new("23:30:--");
        assert!(press(&f, 0, NamedKey::ArrowUp));
        assert_eq!(f.get(0), Some(0), "23 wraps to 00");
        press(&f, 2, NamedKey::ArrowDown);
        assert_eq!(
            written(f.values.get()),
            "00:30:59",
            "an empty field steps down to its highest"
        );
        type_in(&f, 1, "4");
        assert_eq!(f.get(1), Some(4), "4 waits for a second digit");
        type_in(&f, 1, "5");
        assert_eq!(f.get(1), Some(45));
        type_in(&f, 0, "7");
        assert_eq!(f.get(0), Some(7), "no hour starts with 7 and has two digits");
        assert!(press(&f, 2, NamedKey::Backspace));
        assert_eq!(written(f.values.get()), "");
        assert!(
            !press(&f, 0, NamedKey::ArrowLeft),
            "the first field leaves ArrowLeft to the page"
        );
    }
}
