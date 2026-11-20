//! Checkbox (`ui/checkbox`): a box that is checked, not, or indeterminate.

use gui::prelude::*;
use gui::{Div, Icon, Signal, div, icon};

use crate::focus::focus_ring;
use crate::theme::theme;

/// What a checkbox shows: reka's `boolean | 'indeterminate'`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedState {
    Unchecked,
    Checked,
    /// Some of what it stands for is checked (a "select all" over a part).
    Indeterminate,
}

impl From<bool> for CheckedState {
    fn from(checked: bool) -> Self {
        if checked { Self::Checked } else { Self::Unchecked }
    }
}

/// A checkbox bound to `checked`; a click, or Space while focused, flips it.
/// Disable it with `.disabled(true)`.
#[must_use]
pub fn checkbox(checked: &Signal<bool>) -> Div {
    let set = checked.clone();
    checkbox_state(checked.get().into(), move |on| set.set(on))
}

/// A checkbox showing `state`, whose owner keeps it: a click, or Space while
/// focused, calls `on_change` with the value it turns to (checked, unless it
/// was; reka checks an indeterminate one).
#[must_use]
pub fn checkbox_state(state: CheckedState, on_change: impl Fn(bool) + 'static) -> Div {
    let t = theme();
    let base = div()
        .named("checkbox")
        .transition_shadow()
        .flex()
        .items_center()
        .justify_center()
        .size(4.0)
        .shrink_0()
        .rounded(4.0)
        .border()
        .border_color(t.input)
        .shadow_xs()
        .focusable()
        .on_click(move |_| on_change(state != CheckedState::Checked))
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed());
    let indicator = || icon(Icon::Check).named("checkbox-indicator").size(3.5);
    match state {
        // `data-[state=checked]:border-primary` outranks `focus-visible:border-ring`.
        CheckedState::Checked => base
            .bg(t.primary)
            .text_color(t.primary_foreground)
            .border_color(t.primary)
            .focus_visible(move |s| s.ring(3.0, t.ring.opacity(0.5)))
            .child(indicator()),
        // The indicator shows, but the `checked` fill and colour do not.
        CheckedState::Indeterminate => base.focus_visible(move |s| focus_ring(s, &t)).child(indicator()),
        CheckedState::Unchecked => base.focus_visible(move |s| focus_ring(s, &t)),
    }
}
