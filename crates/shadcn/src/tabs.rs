//! Tabs (`ui/tabs`): a row of triggers, each showing its own panel.
//!
//! Keyboard as reka's: the arrow keys (left and right, wrapping), Home and End
//! move between triggers, and a trigger reached that way is selected
//! (`activationMode: 'automatic'`).

use std::cell::RefCell;
use std::rc::Rc;

use gui::Properties;
use gui::prelude::*;
use gui::{Color, Div, FocusHandle, KeyEvent, NamedKey, Signal, div};

use crate::focus::focus_ring;
use crate::theme::theme;

/// Which tab is selected, and the triggers in the order they render.
#[derive(Clone)]
pub struct TabsState {
    value: Signal<String>,
    /// One focus handle per trigger value, made the first time it renders.
    triggers: Rc<RefCell<Vec<(String, FocusHandle)>>>,
}

impl TabsState {
    /// Tabs opening on `default` (reka's `default-value`).
    #[must_use]
    pub fn new(default: impl Into<String>) -> Self {
        Self {
            value: Signal::labeled("tabs", default.into()),
            triggers: Rc::default(),
        }
    }

    /// The selected value (Vue's `v-model`).
    #[must_use]
    pub fn value(&self) -> Signal<String> {
        self.value.clone()
    }

    /// Whether `value` is the selected tab (reactive).
    #[must_use]
    pub fn is_active(&self, value: &str) -> bool {
        self.value.get() == value
    }

    fn select(&self, value: &str) {
        if !self.is_active(value) {
            self.value.set(value.to_owned());
        }
    }

    fn focus_handle(&self, value: &str) -> FocusHandle {
        let mut triggers = self.triggers.borrow_mut();
        if let Some((_, handle)) = triggers.iter().find(|(v, _)| v == value) {
            return *handle;
        }
        let handle = FocusHandle::new();
        triggers.push((value.to_owned(), handle));
        handle
    }

    /// Move focus from the trigger of `from` as `key` asks, selecting where it
    /// lands; whether the key was one of the tab keys.
    fn navigate(&self, from: &str, key: &KeyEvent) -> bool {
        let m = key.modifiers;
        if m.shift || m.control || m.alt || m.command {
            return false;
        }
        let target = {
            let triggers = self.triggers.borrow();
            let current = triggers.iter().position(|(v, _)| v == from);
            let Some(index) = roving_target(triggers.len(), current, key) else {
                return false;
            };
            triggers.get(index).cloned()
        };
        if let Some((value, handle)) = target {
            gui::focus::set_focus(Some(handle.id()), true);
            self.select(&value);
        }
        true
    }
}

/// Where a horizontal, wrapping roving focus goes from `current` among `len`
/// items on `key` (reka's `getFocusIntent` and `wrapArray`).
fn roving_target(len: usize, current: Option<usize>, key: &KeyEvent) -> Option<usize> {
    let last = len.checked_sub(1)?;
    let i = current?;
    if key.is(NamedKey::ArrowRight) {
        Some(if i >= last { 0 } else { i.saturating_add(1) })
    } else if key.is(NamedKey::ArrowLeft) {
        Some(i.checked_sub(1).unwrap_or(last))
    } else if key.is(NamedKey::Home) || key.is(NamedKey::PageUp) {
        Some(0)
    } else if key.is(NamedKey::End) || key.is(NamedKey::PageDown) {
        Some(last)
    } else {
        None
    }
}

#[must_use]
pub fn tabs() -> Div {
    div().named("tabs").flex_col().gap(2.0)
}

/// The row of triggers. `w-fit` is `self_start`: in the column [`tabs`]
/// lays out, the list takes its content's width instead of stretching.
#[must_use]
pub fn tabs_list() -> Div {
    let t = theme();
    div()
        .named("tabs-list")
        .role(gui::Role::TabList)
        .bg(t.muted)
        .text_color(t.muted_foreground)
        .inline_flex()
        .self_start()
        .h(9.0)
        .items_center()
        .justify_center()
        .rounded_lg()
        .p(0.75)
}

/// The trigger selecting `value`; add its label with `.child`.
#[must_use]
pub fn tabs_trigger(state: &TabsState, value: &str) -> Div {
    let t = theme();
    let active = state.is_active(value);
    let (on_click, on_key) = (state.clone(), state.clone());
    let (click_value, key_value) = (value.to_owned(), value.to_owned());
    div()
        .named("tabs-trigger")
        .role(gui::Role::Tab)
        .aria_selected(active)
        .transition_props(Properties::COLOR | Properties::SHADOW)
        .track_focus(state.focus_handle(value))
        .text_color(if t.dark { t.muted_foreground } else { t.foreground })
        .inline_flex()
        // `h-[calc(100%-1px)]` of the list's 30px inside its 3px padding.
        .h_px(29.0)
        .flex_1()
        .items_center()
        .justify_center()
        .gap(1.5)
        .rounded_md()
        .border()
        .border_color(Color::TRANSPARENT)
        .px(2.0)
        .py(1.0)
        .text_sm()
        .font_medium()
        .whitespace_nowrap()
        .when(active, |d| {
            d.bg(t.background).shadow_sm().when(t.dark, |d| {
                d.text_color(t.foreground)
                    .border_color(t.input)
                    .bg(t.input.opacity(0.3))
            })
        })
        .focus_visible(move |s| focus_ring(s, &t))
        .disabled_style(|s| s.opacity(0.5))
        .on_click(move |_| on_click.select(&click_value))
        .on_key_down(move |key| on_key.navigate(&key_value, key))
}

/// The panel of `value`, `None` while another tab is selected (reka unmounts
/// it); fill it through `.map(|c| c.child(…))`.
#[must_use]
pub fn tabs_content(state: &TabsState, value: &str) -> Option<Div> {
    state
        .is_active(value)
        .then(|| div().named("tabs-content").role(gui::Role::TabPanel).flex_1())
}
