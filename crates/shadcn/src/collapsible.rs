//! Collapsible (`ui/collapsible`): content shown and hidden by a trigger.
//!
//! The open state is the caller's `Signal<bool>` (Vue's `v-model:open`).

use gui::{ClickEvent, Div, Signal, div};

#[must_use]
pub fn collapsible() -> Div {
    div().named("collapsible")
}

/// What pressing the trigger does: flip `open`. Give it to your own control
/// (reka's `as-child`): `button(…).on_click(collapsible_toggle(&open))`.
pub fn collapsible_toggle(open: &Signal<bool>) -> impl Fn(&ClickEvent) + 'static {
    let open = open.clone();
    move |_| open.update(|o| !o)
}

/// An unstyled pressable trigger that flips `open`.
#[must_use]
pub fn collapsible_trigger(open: &Signal<bool>) -> Div {
    div()
        .named("collapsible-trigger")
        .focusable()
        .on_click(collapsible_toggle(open))
}

/// The content, `None` while closed (reka unmounts it); style and fill it
/// through `.map(|c| c.flex_col().child(…))`.
#[must_use]
pub fn collapsible_content(open: &Signal<bool>) -> Option<Div> {
    open.get().then(|| div().named("collapsible-content"))
}
