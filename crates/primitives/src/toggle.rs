//! A two-state button, pressed or not (reka's `Toggle`; research §10
//! "Toggle").
//!
//! - **Value.** A `bool`, released by default; the user's click flips it
//!   (`Toggle/Toggle.vue:41-45,68-75`).
//! - **Keys.** Space and Enter flip it: the engine presses the focused
//!   control, so the part sets no key handler.
//! - **Root.** A button with `aria-pressed` (AccessKit's toggled button),
//!   one Tab stop.
//! - **Disabled.** Every way of flipping it is shut: the click handler
//!   checks the availability itself, and the engine runs no click on a
//!   disabled or unavailable node. reka's toggle relies on the native
//!   button's `disabled`, so one rendered as another element still flips
//!   while disabled (`Toggle.vue:73-75`; research §4 item 13); not here.
//! - **Form.** A toggle outside a group sends `name=true` while pressed
//!   (not `on`) and nothing while released or disabled
//!   (`Toggle.vue:105-114`): [`Pressed`].

use gui::{Div, Role};

use crate::availability::Availability;
use crate::form::{FormData, Submit};

/// `d` as a toggle that is `pressed` or not, usable as `availability`
/// says: a button, `aria-pressed`, focus, and the flip, which writes
/// `!pressed` through `set`. It sets no key handler.
#[must_use]
pub fn root(d: Div, pressed: bool, availability: Availability, set: impl Fn(bool) + 'static) -> Div {
    availability
        .apply(d.role(Role::Button).aria_checked(pressed).focusable())
        .on_click(move |_| {
            if availability.actionable() {
                set(!pressed);
            }
        })
}

/// A toggle's value as a form sends it: `true` while pressed, nothing
/// while released (`Toggle.vue:105-114`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Pressed(pub bool);

impl Submit for Pressed {
    fn form_data(&self) -> FormData {
        FormData::Toggle(self.0.then(|| "true".to_owned()))
    }
}
