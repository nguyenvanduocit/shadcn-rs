//! A time input (`<input type="time" step="1">`) built as Chrome builds its
//! own: an hour, a minute and a second field, each focusable (Tab and the
//! arrow keys move between them), stepped with ArrowUp and ArrowDown and
//! typed digit by digit; a 24-hour clock, as Chrome shows it on the machine
//! the reference is drawn on.

use std::rc::Rc;

use crate::element::{Div, IntoElement, Refine, div, text};
use crate::events::{KeyEvent, NamedKey};
use crate::focus::{self, FocusHandle};
use crate::reactive::{Effect, Signal};
use crate::style::Style;
use crate::styled::Styled;

/// The largest value of each field: hour, minute, second.
const MAX: [u8; 3] = [23, 59, 59];

/// What an empty field shows.
const PLACEHOLDER: &str = "--";

/// Chrome sizes each field 2 px × `devicePixelRatio` wider than the widest
/// value it can show (`DateTimeFieldElement::MaximumWidth`), while drawing
/// it with 1 px of padding a side: at the 2x a Retina screen and the
/// reference draw at, 2 px a side, the rest left empty after the fields.
const SIZING_PADDING: f32 = 0.5;

/// The state behind one time input: each field's value and focus.
#[derive(Clone)]
pub struct TimeField {
    /// Hour, minute and second; `None` shows `--`.
    values: Signal<[Option<u8>; 3]>,
    /// The digits typed into a field since it took focus (Chrome's
    /// type-ahead): the field, their number, and the number they spell.
    typed: Signal<Option<(usize, u8, u8)>>,
    focus: [FocusHandle; 3],
    /// Forgets `typed` whenever focus moves, as Chrome does on blur.
    _forget: Rc<Effect>,
}

impl TimeField {
    /// A field holding `value` (`HH:MM:SS`); a part missing, out of range or
    /// `--` leaves its field empty.
    #[must_use]
    pub fn new(value: &str) -> Self {
        let mut parts = value.split(':');
        let values = MAX.map(|max| parts.next().and_then(|p| p.parse::<u8>().ok()).filter(|v| *v <= max));
        let typed = Signal::labeled("time typed", None);
        let forget = typed.clone();
        Self {
            values: Signal::labeled("time", values),
            typed,
            focus: [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()],
            _forget: Rc::new(Effect::new(move || {
                // Read to re-run on every move of focus.
                let _moved = focus::focused();
                forget.update(|_| None);
            })),
        }
    }

    /// The value (`HH:MM:SS`) once every field has one, as the input's `value` is.
    #[must_use]
    pub fn value(&self) -> Option<String> {
        let [h, m, s] = self.values.get();
        Some(format!("{:02}:{:02}:{:02}", h?, m?, s?))
    }

    /// Whether one of its fields holds focus (reactive), for the box's
    /// `:focus-within` look.
    #[must_use]
    pub fn focused(&self) -> bool {
        self.focus.iter().any(|f| f.is_focused())
    }

    fn get(&self, i: usize) -> Option<u8> {
        self.values.get().get(i).copied().flatten()
    }

    fn set(&self, i: usize, value: Option<u8>) {
        self.values.update(|mut v| {
            if let Some(slot) = v.get_mut(i) {
                *slot = value;
            }
            v
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

/// The fields of `field` in a box as wide as Chrome makes a time input's;
/// the caller gives it its frame (border, padding, height). `focused` styles
/// the field that holds focus (`::-webkit-datetime-edit-hour-field:focus`).
#[must_use]
pub fn time_input(field: &TimeField, focused: impl Fn(Style) -> Style + 'static) -> Div {
    let focused: Refine = Rc::new(focused);
    let values = field.values.get();
    let between = || text(":");
    let mut fields = div().flex().items_center().row_start(1).col_start(1);
    let mut sizer = div().flex().items_center().row_start(1).col_start(1).invisible();
    for (i, (value, max)) in values.into_iter().zip(MAX).enumerate() {
        if i > 0 {
            fields = fields.child(between());
            sizer = sizer.child(between());
        }
        let (me, look) = (field.clone(), Rc::clone(&focused));
        let handle = field.focus.get(i).copied().unwrap_or_default();
        fields = fields.child(
            div()
                .p(0.25)
                .track_focus(handle)
                .focus(move |s| look(s))
                .on_key_down(move |ev| me.key(i, ev))
                .child(shown(value)),
        );
        // The widest of what the field can show, stacked in one cell.
        let widest = [PLACEHOLDER.to_owned(), format!("{max:02}"), shown(value)]
            .map(|t| text(t).row_start(1).col_start(1).into_element());
        sizer = sizer.child(div().grid().px(SIZING_PADDING).children(widest));
    }
    let first = field.clone();
    div()
        .named("time-input")
        .flex()
        .items_center()
        .on_mouse_down(move |_| {
            // A press beside the fields focuses the first, as Chrome's does.
            if !first.focused() {
                first.focus_field(Some(0));
            }
        })
        .child(div().grid().child(sizer).child(fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Key, Modifiers};

    fn press(f: &TimeField, i: usize, key: NamedKey) -> bool {
        f.key(
            i,
            &KeyEvent {
                key: Key::Named(key),
                text: None,
                modifiers: Modifiers::default(),
            },
        )
    }

    fn type_in(f: &TimeField, i: usize, digits: &str) {
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
        let f = TimeField::new("23:30:--");
        assert!(press(&f, 0, NamedKey::ArrowUp));
        assert_eq!(f.get(0), Some(0), "23 wraps to 00");
        press(&f, 2, NamedKey::ArrowDown);
        assert_eq!(
            f.value().as_deref(),
            Some("00:30:59"),
            "an empty field steps down to its highest"
        );
        type_in(&f, 1, "4");
        assert_eq!(f.get(1), Some(4), "4 waits for a second digit");
        type_in(&f, 1, "5");
        assert_eq!(f.get(1), Some(45));
        type_in(&f, 0, "7");
        assert_eq!(f.get(0), Some(7), "no hour starts with 7 and has two digits");
        assert!(press(&f, 2, NamedKey::Backspace));
        assert_eq!(f.value(), None);
        assert!(
            !press(&f, 0, NamedKey::ArrowLeft),
            "the first field leaves ArrowLeft to the page"
        );
    }
}
