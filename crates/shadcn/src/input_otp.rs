//! Input OTP (`ui/input-otp`): a one-time code typed into a row of slots.
//!
//! As `vue-input-otp`, it is one control: the root takes focus and keys, and
//! each slot shows one character of the value. The slot the next character
//! goes to is active, and shows a blinking caret while empty.

use gui::prelude::*;
use gui::{Div, FocusHandle, Icon, Key, KeyEvent, NamedKey, Radius, Signal, div, icon};

use crate::button_group::with_style;
use crate::focus::{destructive_ring, focus_ring};
use crate::theme::{Theme, theme};

/// Which characters a code takes (`vue-input-otp`'s `REGEXP_ONLY_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InputOtpPattern {
    /// Any printable character.
    #[default]
    Any,
    Digits,
    Chars,
    DigitsAndChars,
}

impl InputOtpPattern {
    fn accepts(self, c: char) -> bool {
        match self {
            Self::Any => !c.is_control(),
            Self::Digits => c.is_ascii_digit(),
            Self::Chars => c.is_ascii_alphabetic(),
            Self::DigitsAndChars => c.is_ascii_alphanumeric(),
        }
    }
}

/// An OTP input's state; create it once, outside the render closure.
#[derive(Clone)]
pub struct InputOtpState {
    /// The code typed so far (`v-model`).
    pub value: Signal<String>,
    pub focus: FocusHandle,
    max_length: usize,
    pattern: InputOtpPattern,
}

impl InputOtpState {
    #[must_use]
    pub fn new(max_length: usize, pattern: InputOtpPattern) -> Self {
        Self {
            value: Signal::new(String::new()),
            focus: FocusHandle::new(),
            max_length,
            pattern,
        }
    }

    /// Type, erase or paste; returns whether the key was the input's.
    fn key(&self, ev: &KeyEvent) -> bool {
        match &ev.key {
            Key::Named(NamedKey::Backspace) => {
                self.value.update(|mut v| {
                    v.pop();
                    v
                });
                true
            }
            Key::Character(c) if ev.modifiers.primary() && c.eq_ignore_ascii_case("v") => {
                let pasted = gui::clipboard::paste().unwrap_or_default();
                self.value.set(self.fit(&pasted, String::new()));
                true
            }
            _ if ev.modifiers.primary() || ev.modifiers.control => false,
            _ => match ev.text.as_deref() {
                Some(typed) if typed.chars().all(|c| self.pattern.accepts(c)) => {
                    let v = self.value.get();
                    let next = self.fit(typed, v.clone());
                    if next != v {
                        self.value.set(next);
                    }
                    true
                }
                _ => false,
            },
        }
    }

    /// `onto` with the characters of `typed` the pattern takes, up to the length.
    fn fit(&self, typed: &str, mut onto: String) -> String {
        for c in typed.chars().filter(|c| self.pattern.accepts(*c)) {
            if onto.chars().count() >= self.max_length {
                break;
            }
            onto.push(c);
        }
        onto
    }
}

/// The root: holds focus and takes the keys; its groups and separators go
/// in with `.child`.
#[must_use]
pub fn input_otp(state: &InputOtpState) -> Div {
    let keys = state.clone();
    div()
        .named("input-otp")
        .flex()
        .items_center()
        .gap(2.0)
        .cursor_text()
        .track_focus(state.focus)
        .on_key_down(move |ev| keys.key(ev))
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed())
}

/// Slots joined into one bordered strip: the first takes the left border
/// and corners, the last the right corners
/// (`first:rounded-l-md first:border-l last:rounded-r-md`).
#[must_use]
pub fn input_otp_group(slots: impl IntoIterator<Item = Div>) -> Div {
    let slots: Vec<Div> = slots.into_iter().collect();
    let last = slots.len().saturating_sub(1);
    div()
        .named("input-otp-group")
        .flex()
        .items_center()
        .children(slots.into_iter().enumerate().map(|(i, s)| {
            let s = if i == 0 { s.rounded_l(Radius::MD).border_l() } else { s };
            if i == last { s.rounded_r(Radius::MD) } else { s }
        }))
}

/// The slot showing character `index` of the code; `invalid` marks it
/// destructive (`aria-invalid`), as a form decides.
#[must_use]
pub fn input_otp_slot(state: &InputOtpState, index: usize, invalid: bool) -> Div {
    let t = theme();
    let value = state.value.get();
    let len = value.chars().count();
    let active = state.focus.is_focused() && index == len.min(state.max_length.saturating_sub(1));
    let char = value.chars().nth(index);
    otp_slot(&t, active, invalid)
        .named("input-otp-slot")
        .transition_all()
        .when_some(char, |d, c| d.child(c.to_string()))
        .when(active && char.is_none(), |d| d.child(fake_caret(&t)))
}

/// The look of one slot, `active` as `data-active` or `:focus` sets it;
/// shared with the pin input, whose slots look the same.
pub(crate) fn otp_slot(t: &Theme, active: bool, invalid: bool) -> Div {
    let t = *t;
    let d = div()
        .relative()
        .flex()
        .h(9.0)
        .w(9.0)
        .items_center()
        .justify_center()
        .border_t()
        .border_b()
        .border_r()
        .border_color(t.input)
        .text_sm()
        .shadow_xs()
        .when(t.dark, |d| d.bg(t.input.opacity(0.3)));
    match (active, invalid) {
        (false, false) => d,
        (false, true) => d.border_color(t.destructive),
        (true, false) => with_style(d, |s| focus_ring(s, &t)),
        (true, true) => with_style(d, |s| destructive_ring(s, &t).border_color(t.destructive)),
    }
}

/// The blinking caret an empty active slot shows (`animate-caret-blink`).
pub(crate) fn fake_caret(t: &Theme) -> Div {
    div()
        .absolute()
        .inset(0.0)
        .flex()
        .items_center()
        .justify_center()
        .pass_through()
        .child(
            div()
                .h(4.0)
                .w_px(1.0)
                .bg(t.foreground)
                .animate(gui::Animation::caret_blink()),
        )
}

/// The mark between two groups.
#[must_use]
pub fn input_otp_separator() -> Div {
    // No `size-*` on it in shadcn: lucide's own 24px.
    div()
        .named("input-otp-separator")
        .flex()
        .child(icon(Icon::Minus).size(6.0))
}
