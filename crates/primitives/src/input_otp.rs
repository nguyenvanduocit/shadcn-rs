//! A one-time code typed into a row of slots as one control (shadcn's
//! `input-otp`, which shadcn-vue builds on vue-input-otp; research §10
//! "input_otp của shadcn-rs").
//!
//! It is one focus stop holding the whole code as a string; each slot shows
//! one of its characters. It keeps the [`crate::pin_input`] guarantees a
//! code field needs:
//!
//! - **Fill order.** Characters go in order, so no slot past the first
//!   empty one is reached: the slot the next character goes to is the
//!   active one ([`Otp::active`]), and a press anywhere focuses the control.
//!   Tab is not trapped: the control is one stop.
//! - **One character per slot**, a grapheme ([`crate::pin_input::graphemes`]);
//!   on the last slot a new one replaces the last character, as a pin's
//!   last slot does (`PinInputInput.vue:89-92`).
//! - **Several at once** (a paste, an autofill): as many as the slots or
//!   more fill from the first slot, fewer go on from the last one typed,
//!   the rest cut; characters the pattern refuses are dropped first
//!   (`PinInputInput.vue:203-239`).
//! - **Input methods** ([`Otp::composing`]): the control takes an input
//!   method's composition, shown in the next slot while it lasts; what it
//!   commits is entered as typing is, spread over the slots.
//! - **Backspace** removes the last character; there is nothing after the
//!   caret for Delete to remove.
//! - **Full** ([`Otp::set_on_complete`]): heard at each change while every
//!   slot has a character.
//! - **ARIA.** One text input, the code its value, named by its label
//!   ([`OtpConfig::label`]) or, without one, by [`OtpNames`] ("One-time
//!   code"): never both, as a name of its own would hide the label's.
//! - **Form.** The code as typed (`String`'s [`crate::form::Submit`]);
//!   nothing while disabled.

use gui::{Div, FocusHandle, Key, KeyEvent, NamedKey, Role, Signal, UseModel, watch};
use unicode_segmentation::UnicodeSegmentation;

use crate::availability::Availability;
use crate::label::Label;

/// Which characters a code takes (vue-input-otp's `REGEXP_ONLY_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum OtpPattern {
    /// Any printable character.
    #[default]
    Any,
    Digits,
    Chars,
    DigitsAndChars,
}

impl OtpPattern {
    /// Whether the pattern takes `grapheme`.
    #[must_use]
    pub fn takes(self, grapheme: &str) -> bool {
        let one = |f: fn(&char) -> bool| grapheme.chars().count() == 1 && grapheme.chars().all(|c| f(&c));
        match self {
            Self::Any => !grapheme.is_empty() && !grapheme.chars().all(char::is_control),
            Self::Digits => one(char::is_ascii_digit),
            Self::Chars => one(char::is_ascii_alphabetic),
            Self::DigitsAndChars => one(char::is_ascii_alphanumeric),
        }
    }
}

/// A code's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OtpConfig {
    /// How many characters the code has (`maxlength`).
    pub max_length: usize,
    pub pattern: OtpPattern,
    pub availability: Availability,
    /// The label naming it (its [`Label::control`]); none, it is named by
    /// [`OtpNames`].
    pub label: Option<Label>,
}

impl Default for OtpConfig {
    /// Six characters of any kind.
    fn default() -> Self {
        Self {
            max_length: 6,
            pattern: OtpPattern::Any,
            availability: Availability::Enabled,
            label: None,
        }
    }
}

/// What a screen reader is told the control is when no label names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OtpNames {
    pub control: String,
}

impl Default for OtpNames {
    fn default() -> Self {
        Self {
            control: "One-time code".to_owned(),
        }
    }
}

/// `code` with `typed` entered after it, as the table above has it.
#[must_use]
pub fn entered(code: &str, typed: &str, config: &OtpConfig) -> String {
    let chars: Vec<&str> = typed.graphemes(true).filter(|g| config.pattern.takes(g)).collect();
    let max = config.max_length;
    if chars.is_empty() || max == 0 {
        return code.to_owned();
    }
    if chars.len() >= max && chars.len() > 1 {
        return chars.iter().take(max).copied().collect();
    }
    let mut now: Vec<&str> = code.graphemes(true).collect();
    if let [one] = chars.as_slice() {
        if now.len() >= max {
            // The last slot takes the new character in place of its own.
            now.truncate(max.saturating_sub(1));
        }
        now.push(one);
    } else {
        // Several go on from the last one typed; the rest is cut.
        let room = max.saturating_sub(now.len());
        now.extend(chars.into_iter().take(room));
    }
    now.concat()
}

type OnComplete = Option<gui::Callback<String>>;

/// An OTP input: made once in its root's setup, `Copy`.
pub struct Otp<P: 'static> {
    value: UseModel<P, String>,
    config: Signal<OtpConfig>,
    focus: FocusHandle,
    on_complete: Signal<OnComplete>,
    names: Signal<OtpNames>,
    /// What an input method composes, shown in the next slot.
    composing: Signal<Option<String>>,
}

impl<P: 'static> Clone for Otp<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Otp<P> {}

impl<P: 'static> std::fmt::Debug for Otp<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Otp").finish_non_exhaustive()
    }
}

impl<P: 'static> Otp<P> {
    /// A code writing `value` (its `v-model`) by `config`; called in setup.
    #[must_use]
    #[track_caller]
    pub fn new(value: UseModel<P, String>, config: OtpConfig) -> Self {
        let otp = Self {
            value,
            config: Signal::labeled("otp config", config),
            focus: FocusHandle::new(),
            on_complete: Signal::new(None),
            names: Signal::labeled("otp names", OtpNames::default()),
            composing: Signal::labeled("otp composition", None),
        };
        watch(
            move || value.get(),
            move |code, _| {
                let max = otp.config.get_untracked().max_length;
                if max > 0
                    && code.graphemes(true).count() >= max
                    && let Some(f) = otp.on_complete.get_untracked()
                {
                    f.call(code.clone());
                }
            },
        );
        otp
    }

    /// Its settings from now on.
    pub fn set_config(self, config: OtpConfig) {
        self.config.set(config);
    }

    pub fn set_names(self, names: OtpNames) {
        self.names.set(names);
    }

    /// Call `f` with the code at each change while every slot has a
    /// character (`@complete`).
    pub fn set_on_complete(self, f: impl Into<gui::Callback<String>>) {
        let f: gui::Callback<String> = f.into();
        self.on_complete.update(|c| *c = Some(f));
    }

    /// Its settings (tracked).
    #[must_use]
    pub fn config(self) -> OtpConfig {
        self.config.get()
    }

    /// The focus of the control.
    #[must_use]
    pub fn focus(self) -> FocusHandle {
        self.focus
    }

    /// The character slot `i` shows (tracked).
    #[must_use]
    pub fn char_at(self, i: usize) -> Option<String> {
        self.value.with(|v| v.graphemes(true).nth(i).map(str::to_owned))
    }

    /// What an input method composes, while it does (tracked): the look
    /// shows it in the next slot ([`Self::is_next`]).
    #[must_use]
    pub fn composing(self) -> Option<String> {
        self.composing.get()
    }

    /// What an input method does: its composition is shown; what it
    /// commits is entered, as typing and a paste are.
    fn ime(self, ev: &gui::ImeEvent) {
        let actionable = self.config.get_untracked().availability.actionable();
        match ev {
            gui::ImeEvent::Preedit(p) if p.text.is_empty() || !actionable => self.composing.set(None),
            gui::ImeEvent::Preedit(p) => self.composing.set(Some(p.text.clone())),
            gui::ImeEvent::Commit(text) => {
                self.composing.set(None);
                if actionable {
                    self.type_text(text);
                }
            }
        }
    }

    /// Whether slot `i` is the one the next character goes to (tracked:
    /// the code, not focus; [`Self::active`] adds focus).
    #[must_use]
    pub fn is_next(self, i: usize) -> bool {
        let max = self.config.get().max_length;
        let len = self.value.with(|v| v.graphemes(true).count());
        i == len.min(max.saturating_sub(1))
    }

    /// Whether slot `i` is active: the next one, while the control has
    /// focus. Bind it to the node, so focus renders nothing.
    pub fn active(self, i: usize) -> impl Fn() -> bool + Clone + 'static {
        let next = gui::untracked(|| self.is_next(i));
        let focus = self.focus;
        move || next && focus.is_focused()
    }

    /// Enter `typed`.
    pub fn type_text(self, typed: &str) {
        let config = self.config.get_untracked();
        let next = self.value.with(|v| entered(v, typed, &config));
        self.value.set(next);
    }

    /// What a key does; whether it was the control's.
    fn key(self, ev: &KeyEvent) -> bool {
        if !self.config.get_untracked().availability.actionable() {
            return false;
        }
        // While an input method composes, its keys are its own.
        if self.composing.with_untracked(Option::is_some) {
            return true;
        }
        let m = ev.modifiers;
        match &ev.key {
            Key::Named(NamedKey::Backspace) => {
                self.value.update(|v| {
                    let keep = v.graphemes(true).count().saturating_sub(1);
                    *v = v.graphemes(true).take(keep).collect();
                });
                true
            }
            Key::Character(c) if m.primary() && c.eq_ignore_ascii_case("v") => {
                self.type_text(&gui::clipboard::paste().unwrap_or_default());
                true
            }
            _ if m.primary() || m.control || m.alt => false,
            _ => match ev.text.as_deref() {
                Some(typed) if !typed.chars().all(char::is_control) => {
                    self.type_text(typed);
                    true
                }
                _ => false,
            },
        }
    }

    /// `d` as the control: one focus stop, a text input named by its label
    /// (or, without one, [`OtpNames`]), taking the keys. It sets the node's
    /// key handler.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        let config = self.config.get();
        let code = self.value.get();
        let d = d.role(Role::TextInput).aria_value(code).track_focus(self.focus);
        // A name of its own would win over the label's (`aria-label` is
        // read before `aria-labelledby` here), so it is the fallback only.
        let d = match config.label {
            Some(label) => label.control(d),
            None => d.aria_label(self.names.with(|n| n.control.clone())),
        };
        config
            .availability
            .apply(d)
            .on_key_down(move |ev| self.key(ev))
            .on_ime(move |ev| self.ime(ev))
    }
}

#[cfg(test)]
mod tests {
    use super::{OtpConfig, OtpPattern, entered};

    #[test]
    fn characters_go_in_order_and_a_paste_fills_from_the_first_slot() {
        let digits = OtpConfig {
            max_length: 4,
            pattern: OtpPattern::Digits,
            ..OtpConfig::default()
        };
        assert_eq!(entered("12", "3", &digits), "123");
        assert_eq!(entered("12", "a", &digits), "12", "refused by the pattern");
        assert_eq!(entered("1234", "5", &digits), "1235", "the last slot takes the new one");
        assert_eq!(
            entered("12", "a9b8c7d6", &digits),
            "9876",
            "long enough: from the first slot"
        );
        assert_eq!(entered("1", "56", &digits), "156", "shorter: on from the last");
        assert_eq!(entered("123", "56", &digits), "1235", "the rest is cut");
        let any = OtpConfig::default();
        assert_eq!(entered("", "e\u{301}", &any), "e\u{301}", "a grapheme is one character");
    }
}
