//! Pin input (`ui/pin-input`): a code typed one character per box.
//!
//! As reka-ui's, each slot is its own control: typing fills the focused slot
//! and moves on, Backspace clears it or, when it is empty, steps back and
//! clears the one before; a paste spreads over the slots. As shadcn-vue's,
//! the slots fill in order (`otp`, on by default). Slots look like the OTP
//! input's and join the same way, so groups are [`crate::input_otp_group`].
//! The root keeps the characters, its own (`default_value`) or the caller's
//! (`model`, `v-model`). The behaviour is `primitives::pin_input`'s; this
//! module draws it.

use gui::prelude::*;
use gui::{Bind, Div, FocusHandle, Icon, Model, Signal, component, div, icon, provide, use_model, watch};
pub use primitives::pin_input::PinKind;
use primitives::pin_input::{PinConfig, PinInput};
use primitives::{Availability, use_direction};

use crate::focus::StateLook;
use crate::input_otp::{OtpSlotProps, fake_caret, otp_slot, otp_slot_state};
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`pin_input`] is given.
#[derive(Clone, PartialEq)]
pub struct PinInputProps {
    value: Bind<Vec<Option<String>>>,
    config: PinConfig,
}

/// What the slots below a [`pin_input`] read.
#[derive(Clone, Copy)]
struct PinInputContext {
    pin: PinInput<PartProps<PinInputProps>>,
    props: Signal<PartProps<PinInputProps>>,
}

/// The root (`<PinInput>`); its groups and separators go in with `.child`.
/// It keeps one character (or none) per slot: its own (`default_value`) or
/// the caller's (`model`, `v-model`).
#[must_use]
pub fn pin_input() -> Part<PinInputProps> {
    let props = PinInputProps {
        value: Bind::default(),
        config: PinConfig {
            otp: true,
            ..PinConfig::default()
        },
    };
    let root = div()
        .named("pin-input")
        .display(Display::Flex)
        .align_items(Align::Center)
        .gap(8.0)
        .with_interaction(|d, i| d.dim_disabled(i, 0.5).cursor_disabled(i));
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let config = props.with_untracked(|p| p.props.config);
            let pin = PinInput::new(
                use_model(props, |p: &PartProps<PinInputProps>| &p.props.value),
                config,
                use_direction(None),
            );
            watch(move || props.with(|p| p.props.config), move |c, _| pin.set_config(*c));
            provide(PinInputContext { pin, props });
            move || {
                let (root, config) = props.with(|p| (p.slot.0.clone(), p.props.config));
                config.availability.apply(root).into_element()
            }
        })
    })
}

impl Part<PinInputProps> {
    /// The characters, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<Vec<Option<String>>>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// The characters it starts with when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(self, value: Vec<Option<String>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_default(value))
    }

    /// Call `f` with the characters after each change (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Into<gui::Callback<Vec<Option<String>>>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    /// Which characters a slot takes (`type`; any by default).
    #[must_use]
    pub fn kind(self, kind: PinKind) -> Self {
        self.with_props(|p| p.config.kind = kind)
    }

    /// Whether the slots fill in order (`otp`; on, as shadcn-vue's).
    #[must_use]
    pub fn otp(self, otp: bool) -> Self {
        self.with_props(|p| p.config.otp = otp)
    }

    /// Whether the characters are masked (`mask`).
    #[must_use]
    pub fn mask(self, mask: bool) -> Self {
        self.with_props(|p| p.config.mask = mask)
    }

    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.config.availability = Availability::from_disabled(disabled))
    }
}

/// Slot `index`: focusable, taking the keys while focused.
#[must_use]
pub fn pin_input_slot(index: usize) -> Part<OtpSlotProps> {
    let t = theme();
    let root = otp_slot(&t)
        .named("pin-input-slot")
        .transition(Properties::ALL)
        .text_align(TextAlign::Center)
        .cursor(Cursor::Text);
    Part::new(OtpSlotProps { index, invalid: false }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = primitives::context::use_root::<PinInputContext>("pin_input_slot", "pin_input");
            let focus = FocusHandle::new();
            move || {
                let (slot, root) = props.with(|p| (p.props, p.slot.0.clone()));
                let Some(ctx) = ctx else {
                    return root.into_element();
                };
                let t = theme();
                // Bound: focus renders nothing.
                let focused = move || focus.is_focused();
                let config = ctx.props.with(|p| p.props.config);
                let char = ctx.pin.char_at(slot.index);
                let shown = char.map(|c| if config.mask { "\u{2022}".to_owned() } else { c });
                // An input method's composition shows in place of the character.
                let shown = ctx.pin.composing_at(slot.index).or(shown);
                let empty = shown.is_none();
                let d = otp_slot_state(root, &t, focused, slot.invalid)
                    .when_some(shown, |d, c| d.child(c))
                    .when(empty, |d| d.child(fake_caret(&t, focused)));
                ctx.pin.slot(d, slot.index, focus, config.availability).into_element()
            }
        })
    })
}

/// The mark between two groups.
#[must_use]
pub fn pin_input_separator() -> Div {
    // No `size-*` on it in shadcn: lucide's own 24px.
    div().named("pin-input-separator").child(icon(Icon::Minus).size(24.0))
}
