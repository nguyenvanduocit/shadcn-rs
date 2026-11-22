//! Pin input (`ui/pin-input`): a code typed one character per box.
//!
//! As reka-ui's, each slot is its own control: typing fills the focused slot
//! and moves on, Backspace clears it or, when it is empty, steps back and
//! clears the one before. Slots look like the OTP input's and join the same
//! way, so groups are [`crate::input_otp_group`]. The root keeps the
//! characters, its own (`default_value`) or the caller's (`model`, `v-model`).

use gui::prelude::*;
use gui::{
    Bind, Div, FocusHandle, Icon, Key, KeyEvent, Members, Model, NamedKey, Signal, UseModel, component, div, icon,
    provide, use_context, use_model,
};

use crate::input_otp::{OtpSlotProps, fake_caret, otp_slot, otp_slot_state};
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// Which characters a slot takes (reka's `type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PinInputType {
    #[default]
    Text,
    /// Digits only.
    Number,
}

/// What [`pin_input`] is given.
#[derive(Clone, PartialEq)]
pub struct PinInputProps {
    value: Bind<Vec<Option<char>>>,
    kind: PinInputType,
}

/// What the slots below a [`pin_input`] read.
#[derive(Clone)]
struct PinInputContext {
    value: UseModel<PartProps<PinInputProps>, Vec<Option<char>>>,
    props: Signal<PartProps<PinInputProps>>,
    /// Each slot's index and focus, in tree order as last laid out.
    slots: Members<(usize, FocusHandle)>,
}

impl PinInputContext {
    fn accepts(&self, c: char) -> bool {
        match self.props.with_untracked(|p| p.props.kind) {
            PinInputType::Text => !c.is_control(),
            PinInputType::Number => c.is_ascii_digit(),
        }
    }

    fn len(&self) -> usize {
        self.slots.len()
    }

    fn set(&self, at: usize, c: Option<char>) {
        let len = self.len();
        self.value.update(|v| {
            if v.len() < len {
                v.resize(len, None);
            }
            if let Some(slot) = v.get_mut(at) {
                *slot = c;
            }
        });
    }

    fn focus_at(&self, at: usize) {
        let handle = self
            .slots
            .with(|slots| slots.iter().find(|(i, _)| *i == at).map(|(_, f)| *f));
        if let Some(f) = handle {
            f.focus();
        }
    }

    /// Fill slots from `at` with what `typed` holds the type takes; focus the
    /// slot after the last one filled.
    fn fill(&self, at: usize, typed: &str) {
        let len = self.len();
        let mut next = at;
        for c in typed.chars().filter(|c| self.accepts(*c)) {
            if next >= len {
                break;
            }
            self.set(next, Some(c));
            next = next.saturating_add(1);
        }
        self.focus_at(next.min(len.saturating_sub(1)));
    }

    /// The keys of slot `at` (reka's keyboard table); whether it took `ev`.
    fn key(&self, at: usize, ev: &KeyEvent) -> bool {
        let last = self.len().saturating_sub(1);
        match &ev.key {
            Key::Named(NamedKey::Backspace) => {
                let filled = self.value.with(|v| v.get(at).copied().flatten().is_some());
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

/// The root (`<PinInput>`); its groups and separators go in with `.child`.
/// It keeps one character (or none) per slot: its own (`default_value`) or
/// the caller's (`model`, `v-model`).
#[must_use]
pub fn pin_input() -> Part<PinInputProps> {
    let props = PinInputProps {
        value: Bind::default(),
        kind: PinInputType::Text,
    };
    let root = div()
        .named("pin-input")
        .flex()
        .items_center()
        .gap(2.0)
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed());
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(PinInputContext {
                value: use_model(props, |p| &p.props.value),
                props,
                slots: Members::new(),
            });
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

impl Part<PinInputProps> {
    /// The characters, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<Vec<Option<char>>>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// The characters it starts with when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(self, value: Vec<Option<char>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_default(value))
    }

    /// Call `f` with the characters after each change (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(Vec<Option<char>>) + 'static) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    /// Which characters a slot takes (`type`; any by default).
    #[must_use]
    pub fn kind(self, kind: PinInputType) -> Self {
        self.with_props(|p| p.kind = kind)
    }
}

/// Slot `index`: focusable, taking the keys while focused.
#[must_use]
pub fn pin_input_slot(index: usize) -> Part<OtpSlotProps> {
    let t = theme();
    let root = otp_slot(&t)
        .named("pin-input-slot")
        .transition_all()
        .text_center()
        .cursor_text();
    Part::new(OtpSlotProps { index, invalid: false }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = use_context::<PinInputContext>();
            let focus = FocusHandle::new();
            move || {
                let (slot, root) = props.with(|p| (p.props, p.slot.0.clone()));
                let Some(ctx) = ctx.clone() else {
                    return root.into_element();
                };
                let t = theme();
                let focused = focus.is_focused();
                let char = ctx.value.with(|v| v.get(slot.index).copied().flatten());
                let members = ctx.slots.clone();
                otp_slot_state(root, &t, focused, slot.invalid)
                    .track_focus(focus)
                    .member(&members, (slot.index, focus))
                    .on_key_down(move |ev| ctx.key(slot.index, ev))
                    .when_some(char, |d, c| d.child(c.to_string()))
                    .when(focused && char.is_none(), |d| d.child(fake_caret(&t)))
                    .into_element()
            }
        })
    })
}

/// The mark between two groups.
#[must_use]
pub fn pin_input_separator() -> Div {
    // No `size-*` on it in shadcn: lucide's own 24px.
    div().named("pin-input-separator").child(icon(Icon::Minus).size(6.0))
}
