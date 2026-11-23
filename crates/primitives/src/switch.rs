//! An on/off control (reka's `SwitchRoot`, `SwitchThumb`; research §10
//! "Switch").
//!
//! - **Value.** On or off; the user's flip turns one into the other
//!   (`Switch/SwitchRoot.vue:79-84`). There is no indeterminate.
//! - **Keys.** Space and Enter flip it (`SwitchRoot.vue:117`): the engine
//!   presses the focused control on both, so the part sets no key handler,
//!   and a component may set its own.
//! - **Root.** Role `switch`, `aria-checked` true or false, `aria-required`
//!   when required (`SwitchRoot.vue:108-109`), one Tab stop. A
//!   disabled switch takes no focus and no flip; an unavailable one keeps
//!   focus and is read as disabled.
//! - **Thumb.** Decoration that reflects the same state and takes no
//!   pointer (`SwitchThumb.vue:19-27`).
//! - **Form.** `name=on` while on, nothing while off or disabled; the
//!   required check is the form's rule ([`required`], `SwitchRoot.vue:125-134`). `bool`'s
//!   [`crate::form::Submit`] sends it so.
//!
//! reka compares the value with its `trueValue` by `===`, Checkbox
//! structurally; here a switch holds a `bool`, and every primitive compares
//! by `PartialEq` (ADR-0025 §1).

use gui::{Div, Role};

use crate::availability::Availability;

/// `d` as a switch that is `on` or not, usable as `availability` says and
/// `required` or not: role, `aria-checked`, `aria-required`, focus, and the
/// flip, which writes `!on` through `set`. It sets no key handler: Enter
/// and Space press it.
#[must_use]
pub fn root(d: Div, on: bool, availability: Availability, required: bool, set: impl Fn(bool) + 'static) -> Div {
    availability
        .apply(
            d.role(Role::Switch)
                .aria_checked(on)
                .aria_required(required)
                .focusable(),
        )
        .on_click(move |_| {
            if availability.actionable() {
                set(!on);
            }
        })
}

/// `d` as the thumb: decoration that takes no pointer. The look moves it
/// by the same `on`.
#[must_use]
pub fn thumb(d: Div) -> Div {
    d.pass_through()
}

/// The message a required switch fails while off.
#[must_use]
pub fn required(on: &bool, message: &str) -> Vec<String> {
    if *on { Vec::new() } else { vec![message.to_owned()] }
}
