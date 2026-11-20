//! Pin input (`ui/pin-input`): a code typed one character per box.
//!
//! As reka-ui's, each slot is its own control: typing fills the focused slot
//! and moves on, Backspace clears it or, when it is empty, steps back and
//! clears the one before. Slots look like the OTP input's and join the same
//! way, so groups are [`crate::input_otp_group`].

use std::rc::Rc;

use gui::prelude::*;
use gui::{Div, FocusHandle, Icon, Key, KeyEvent, NamedKey, Signal, div, icon};

use crate::input_otp::{fake_caret, otp_slot};
use crate::theme::theme;

/// Which characters a slot takes (reka's `type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PinInputType {
    #[default]
    Text,
    /// Digits only.
    Number,
}

/// A pin input's state: one character (or none) per slot, and each slot's
/// focus. Create it once, outside the render closure.
#[derive(Clone)]
pub struct PinInputState {
    /// The slots' characters (`v-model`).
    pub value: Signal<Vec<Option<char>>>,
    kind: PinInputType,
    focus: Rc<[FocusHandle]>,
}

impl PinInputState {
    #[must_use]
    pub fn new(length: usize, kind: PinInputType) -> Self {
        Self {
            value: Signal::new(vec![None; length]),
            kind,
            focus: (0..length).map(|_| FocusHandle::new()).collect(),
        }
    }

    /// The code typed so far, empty slots skipped.
    #[must_use]
    pub fn code(&self) -> String {
        self.value.get().into_iter().flatten().collect()
    }

    fn accepts(&self, c: char) -> bool {
        match self.kind {
            PinInputType::Text => !c.is_control(),
            PinInputType::Number => c.is_ascii_digit(),
        }
    }

    fn set(&self, at: usize, c: Option<char>) {
        self.value.update(|mut v| {
            if let Some(slot) = v.get_mut(at) {
                *slot = c;
            }
            v
        });
    }

    fn focus_at(&self, at: usize) {
        if let Some(f) = self.focus.get(at) {
            f.focus();
        }
    }

    /// Fill slots from `at` with what `typed` holds the type takes; focus the
    /// slot after the last one filled.
    fn fill(&self, at: usize, typed: &str) {
        let mut next = at;
        for c in typed.chars().filter(|c| self.accepts(*c)) {
            if next >= self.focus.len() {
                break;
            }
            self.set(next, Some(c));
            next = next.saturating_add(1);
        }
        self.focus_at(next.min(self.focus.len().saturating_sub(1)));
    }

    /// The keys of slot `at` (reka's keyboard table); whether it took `ev`.
    fn key(&self, at: usize, ev: &KeyEvent) -> bool {
        let last = self.focus.len().saturating_sub(1);
        match &ev.key {
            Key::Named(NamedKey::Backspace) => {
                let filled = self.value.get().get(at).copied().flatten().is_some();
                if filled {
                    self.set(at, None);
                } else if let Some(prev) = at.checked_sub(1) {
                    self.set(prev, None);
                    self.focus_at(prev);
                }
                true
            }
            Key::Named(NamedKey::Delete) => {
                self.set(at, None);
                true
            }
            Key::Named(NamedKey::ArrowLeft) => {
                self.focus_at(at.saturating_sub(1));
                true
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.focus_at(at.saturating_add(1).min(last));
                true
            }
            Key::Named(NamedKey::Home) => {
                self.focus_at(0);
                true
            }
            Key::Named(NamedKey::End) => {
                self.focus_at(last);
                true
            }
            Key::Character(c) if ev.modifiers.primary() && c.eq_ignore_ascii_case("v") => {
                self.fill(at, &gui::clipboard::paste().unwrap_or_default());
                true
            }
            _ if ev.modifiers.primary() || ev.modifiers.control => false,
            _ => match ev.text.as_deref() {
                Some(typed) if typed.chars().all(|c| self.accepts(c)) => {
                    self.fill(at, typed);
                    true
                }
                _ => false,
            },
        }
    }
}

/// The root; its groups and separators go in with `.child`.
#[must_use]
pub fn pin_input() -> Div {
    div()
        .named("pin-input")
        .flex()
        .items_center()
        .gap(2.0)
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed())
}

/// Slot `index`: focusable, taking the keys while focused; `invalid` marks
/// it destructive (`aria-invalid`).
#[must_use]
pub fn pin_input_slot(state: &PinInputState, index: usize, invalid: bool) -> Div {
    let t = theme();
    let focus = state.focus.get(index).copied();
    let focused = focus.is_some_and(FocusHandle::is_focused);
    let char = state.value.get().get(index).copied().flatten();
    let keys = state.clone();
    otp_slot(&t, focused, invalid)
        .named("pin-input-slot")
        .transition_all()
        .text_center()
        .cursor_text()
        .when_some(focus, Div::track_focus)
        .on_key_down(move |ev| keys.key(index, ev))
        .when_some(char, |d, c| d.child(c.to_string()))
        .when(focused && char.is_none(), |d| d.child(fake_caret(&t)))
}

/// The mark between two groups.
#[must_use]
pub fn pin_input_separator() -> Div {
    // No `size-*` on it in shadcn: lucide's own 24px.
    div().named("pin-input-separator").child(icon(Icon::Minus).size(6.0))
}
