//! Radio group (`ui/radio-group`): a set of choices, one checked at a time.

use std::rc::Rc;

use gui::Properties;
use gui::prelude::*;
use gui::{Color, Div, FocusHandle, Key, NamedKey, Signal, div};

use crate::focus::focus_ring;
use crate::theme::theme;

/// A radio group's state: the checked value and its items in order, each with
/// the focus handle arrow keys move between. Create it once, outside the render closure.
#[derive(Clone)]
pub struct RadioGroupState {
    pub value: Signal<Option<String>>,
    pub disabled: Signal<bool>,
    items: Rc<[(String, FocusHandle)]>,
}

impl RadioGroupState {
    /// A group of the items `values`, in order, with `checked` checked.
    #[must_use]
    pub fn new(values: &[&str], checked: Option<&str>) -> Self {
        Self {
            value: Signal::new(checked.map(str::to_owned)),
            disabled: Signal::new(false),
            items: values.iter().map(|v| ((*v).to_owned(), FocusHandle::new())).collect(),
        }
    }

    /// Focus and check the item `by` places from the focused one, wrapping,
    /// as WAI-ARIA radio groups do.
    fn step(&self, by: isize) -> bool {
        let len = self.items.len();
        let Some(at) = self.items.iter().position(|(_, f)| f.is_focused()) else {
            return false;
        };
        let next = isize::try_from(at)
            .ok()
            .and_then(|a| a.checked_add(by))
            .and_then(|n| n.checked_rem_euclid(isize::try_from(len).ok()?))
            .and_then(|n| usize::try_from(n).ok());
        next.is_some_and(|n| self.choose(n))
    }

    fn choose(&self, at: usize) -> bool {
        self.items.get(at).is_some_and(|(v, f)| {
            gui::focus::set_focus(Some(f.id()), true);
            self.value.set(Some(v.clone()));
            true
        })
    }
}

/// The group's grid; its items go in with `.child`, each beside its label.
/// Arrow keys move to the next or previous item and check it.
#[must_use]
pub fn radio_group(state: &RadioGroupState) -> Div {
    let keys = state.clone();
    div()
        .named("radio-group")
        .grid()
        .gap(3.0)
        .on_key_down(move |ev| match &ev.key {
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowRight) => keys.step(1),
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowLeft) => keys.step(-1),
            _ => false,
        })
}

/// The radio of the item `value`; a click, or Space while focused, checks it.
#[must_use]
pub fn radio_group_item(state: &RadioGroupState, value: &str) -> Div {
    let t = theme();
    let checked = state.value.get().as_deref() == Some(value);
    let focus = state.items.iter().find(|(v, _)| v == value).map(|(_, f)| *f);
    let pick = state.value.clone();
    let owned = value.to_owned();
    div()
        .named("radio-group-item")
        .role(gui::Role::RadioButton)
        .aria_checked(checked)
        .transition_props(Properties::COLOR | Properties::SHADOW)
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .aspect_square()
        .size(4.0)
        .shrink_0()
        .rounded_full()
        .border()
        .border_color(t.input)
        .text_color(t.primary)
        .bg(if t.dark {
            t.input.opacity(0.3)
        } else {
            Color::TRANSPARENT
        })
        .shadow_xs()
        .when_some(focus, Div::track_focus)
        .disabled(state.disabled.get())
        .focus_visible(move |s| focus_ring(s, &t))
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed())
        // WAI-ARIA: Enter does not check a radio, only Space does.
        .on_key_down(|ev| ev.is(NamedKey::Enter))
        .on_click(move |_| pick.set(Some(owned.clone())))
        .when(checked, |d| {
            // lucide's circle (r=10 in 24) filled and stroked at `size-2`: a 22/3px disc.
            d.child(
                div()
                    .named("radio-group-indicator")
                    .pass_through()
                    .size_px(22.0 / 3.0)
                    .rounded_full()
                    .bg(t.primary),
            )
        })
}
