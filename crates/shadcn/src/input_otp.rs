//! Input OTP (`ui/input-otp`): a one-time code typed into a row of slots.
//!
//! As `vue-input-otp`, it is one control: the root takes focus and keys, and
//! each slot shows one character of the value. The slot the next character
//! goes to is active, and shows a blinking caret while empty. The root keeps
//! the code, its own (`default_value`) or the caller's (`model`, `v-model`).

use gui::prelude::*;
use gui::{
    Bind, Div, FocusHandle, Icon, Key, KeyEvent, Model, NamedKey, Radius, Signal, UseModel, component, div, icon,
    provide, use_context, use_model,
};

use crate::button_group::with_style;
use crate::focus::{destructive_ring, focus_ring};
use crate::part::{Part, PartProps};
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

/// What [`input_otp`] is given.
#[derive(Clone, PartialEq)]
pub struct InputOtpProps {
    value: Bind<String>,
    max_length: usize,
    pattern: InputOtpPattern,
}

/// What the slots below an [`input_otp`] read.
#[derive(Clone, Copy)]
struct InputOtpContext {
    value: UseModel<PartProps<InputOtpProps>, String>,
    focus: FocusHandle,
    props: Signal<PartProps<InputOtpProps>>,
}

impl InputOtpContext {
    fn rules(self) -> (usize, InputOtpPattern) {
        self.props.with_untracked(|p| (p.props.max_length, p.props.pattern))
    }

    /// Type, erase or paste; returns whether the key was the input's.
    fn key(self, ev: &KeyEvent) -> bool {
        match &ev.key {
            Key::Named(NamedKey::Backspace) => {
                self.value.update(|v| {
                    v.pop();
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
                Some(typed) if typed.chars().all(|c| self.rules().1.accepts(c)) => {
                    let v = self.value.get_untracked();
                    self.value.set(self.fit(typed, v));
                    true
                }
                _ => false,
            },
        }
    }

    /// `onto` with the characters of `typed` the pattern takes, up to the length.
    fn fit(self, typed: &str, mut onto: String) -> String {
        let (max_length, pattern) = self.rules();
        for c in typed.chars().filter(|c| pattern.accepts(*c)) {
            if onto.chars().count() >= max_length {
                break;
            }
            onto.push(c);
        }
        onto
    }
}

/// The root (`<InputOTP>`): holds focus and takes the keys; its groups and
/// separators go in with `.child`. Its code is its own (`default_value`) or
/// the caller's (`model`, `v-model`); six characters of any kind by default.
#[must_use]
pub fn input_otp() -> Part<InputOtpProps> {
    let props = InputOtpProps {
        value: Bind::default(),
        max_length: 6,
        pattern: InputOtpPattern::Any,
    };
    let root = div()
        .named("input-otp")
        .flex()
        .items_center()
        .gap(2.0)
        .cursor_text()
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed());
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = InputOtpContext {
                value: use_model(props, |p| &p.props.value),
                focus: FocusHandle::new(),
                props,
            };
            provide(ctx);
            move || {
                props
                    .with(|p| p.slot.0.clone())
                    .track_focus(ctx.focus)
                    .on_key_down(move |ev| ctx.key(ev))
                    .into_element()
            }
        })
    })
}

impl Part<InputOtpProps> {
    /// The code, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<String>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// The code it starts with when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.with_props(|p| p.value = p.value.clone().with_default(value))
    }

    /// Call `f` with the code after each change (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(String) + 'static) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    /// How many characters the code has (`maxlength`).
    #[must_use]
    pub fn max_length(self, max_length: usize) -> Self {
        self.with_props(|p| p.max_length = max_length)
    }

    /// Which characters it takes (`pattern`).
    #[must_use]
    pub fn pattern(self, pattern: InputOtpPattern) -> Self {
        self.with_props(|p| p.pattern = pattern)
    }
}

/// Slots joined into one bordered strip: the first takes the left border
/// and corners, the last the right corners
/// (`first:rounded-l-md first:border-l last:rounded-r-md`). The pin
/// input's slots join the same way.
#[must_use]
pub fn input_otp_group(slots: impl IntoIterator<Item = Part<OtpSlotProps>>) -> Div {
    let slots: Vec<Part<OtpSlotProps>> = slots.into_iter().collect();
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

/// What a slot (of an OTP or a pin input) is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OtpSlotProps {
    pub(crate) index: usize,
    pub(crate) invalid: bool,
}

impl Part<OtpSlotProps> {
    /// Destructive (`aria-invalid`), as a form decides.
    #[must_use]
    pub fn invalid(self, invalid: bool) -> Self {
        self.with_props(|p| p.invalid = invalid)
    }
}

/// The slot showing character `index` of the code.
#[must_use]
pub fn input_otp_slot(index: usize) -> Part<OtpSlotProps> {
    let t = theme();
    let root = otp_slot(&t).named("input-otp-slot").transition_all();
    Part::new(OtpSlotProps { index, invalid: false }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = use_context::<InputOtpContext>();
            move || {
                let (slot, root) = props.with(|p| (p.props, p.slot.0.clone()));
                let Some(ctx) = ctx else {
                    return root.into_element();
                };
                let t = theme();
                let max_length = ctx.props.with(|p| p.props.max_length);
                let (len, char) = ctx.value.with(|v| (v.chars().count(), v.chars().nth(slot.index)));
                let active = ctx.focus.is_focused() && slot.index == len.min(max_length.saturating_sub(1));
                otp_slot_state(root, &t, active, slot.invalid)
                    .when_some(char, |d, c| d.child(c.to_string()))
                    .when(active && char.is_none(), |d| d.child(fake_caret(&t)))
                    .into_element()
            }
        })
    })
}

/// The look of one slot; shared with the pin input, whose slots look the same.
pub(crate) fn otp_slot(t: &Theme) -> Div {
    div()
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
        .when(t.dark, |d| d.bg(t.input.opacity(0.3)))
}

/// A slot `active` as `data-active` or `:focus` sets it, and `invalid`.
pub(crate) fn otp_slot_state(d: Div, t: &Theme, active: bool, invalid: bool) -> Div {
    let t = *t;
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
