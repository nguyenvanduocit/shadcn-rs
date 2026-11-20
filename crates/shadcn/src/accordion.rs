//! Accordion (`ui/accordion`): stacked headings, each opening its own section.
//!
//! Keyboard as reka's: Up and Down (wrapping), Home and End move between the
//! triggers; Enter and Space open and close.

use std::cell::RefCell;
use std::rc::Rc;

use gui::prelude::*;
use gui::{Animation, Div, FocusHandle, Icon, KeyEvent, NamedKey, Signal, div, icon};

use crate::focus::focus_ring;
use crate::theme::theme;

/// How many sections may be open at once (reka's `type`, with `collapsible`
/// meaningful only for one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccordionKind {
    /// At most one; `collapsible` lets the open one close, leaving none.
    Single { collapsible: bool },
    /// Any number.
    Multiple,
}

/// Which sections are open, and the triggers in the order they render.
#[derive(Clone)]
pub struct AccordionState {
    kind: AccordionKind,
    open: Signal<Vec<String>>,
    /// One focus handle per item value, made the first time its trigger renders.
    triggers: Rc<RefCell<Vec<(String, FocusHandle)>>>,
}

impl AccordionState {
    /// One section open at a time, `default` first (reka's `type="single"`).
    #[must_use]
    pub fn single(default: Option<&str>, collapsible: bool) -> Self {
        Self::with(
            AccordionKind::Single { collapsible },
            default.into_iter().map(str::to_owned).collect(),
        )
    }

    /// Any number open, `defaults` first (reka's `type="multiple"`).
    #[must_use]
    pub fn multiple(defaults: &[&str]) -> Self {
        Self::with(
            AccordionKind::Multiple,
            defaults.iter().map(|v| (*v).to_owned()).collect(),
        )
    }

    fn with(kind: AccordionKind, open: Vec<String>) -> Self {
        Self {
            kind,
            open: Signal::labeled("accordion", open),
            triggers: Rc::default(),
        }
    }

    /// The open values (Vue's `v-model`).
    #[must_use]
    pub fn value(&self) -> Signal<Vec<String>> {
        self.open.clone()
    }

    /// Whether the section `value` is open (reactive).
    #[must_use]
    pub fn is_open(&self, value: &str) -> bool {
        self.open.get().iter().any(|v| v == value)
    }

    /// Open or close the section `value`, as pressing its trigger does.
    pub fn toggle(&self, value: &str) {
        let kind = self.kind;
        self.open.update(|open| toggled(kind, open, value));
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

    /// Move focus from the trigger of `from` as `key` asks; whether the key
    /// was one of the accordion's.
    fn navigate(&self, from: &str, key: &KeyEvent) -> bool {
        let triggers = self.triggers.borrow();
        let current = triggers.iter().position(|(v, _)| v == from);
        let Some(index) = roving_target(triggers.len(), current, key) else {
            return false;
        };
        if let Some((_, handle)) = triggers.get(index) {
            gui::focus::set_focus(Some(handle.id()), true);
        }
        true
    }
}

/// The open values after pressing the trigger of `value`.
fn toggled(kind: AccordionKind, open: Vec<String>, value: &str) -> Vec<String> {
    let was_open = open.iter().any(|v| v == value);
    match (kind, was_open) {
        (AccordionKind::Single { collapsible: false }, true) => open,
        (AccordionKind::Single { .. }, true) => Vec::new(),
        (AccordionKind::Single { .. }, false) => vec![value.to_owned()],
        (AccordionKind::Multiple, true) => open.into_iter().filter(|v| v != value).collect(),
        (AccordionKind::Multiple, false) => open.into_iter().chain([value.to_owned()]).collect(),
    }
}

/// Where a vertical, wrapping arrow navigation goes from `current` among `len`
/// triggers on `key` (reka's `useArrowNavigation`).
fn roving_target(len: usize, current: Option<usize>, key: &KeyEvent) -> Option<usize> {
    let last = len.checked_sub(1)?;
    let i = current?;
    if key.is(NamedKey::ArrowDown) {
        Some(if i >= last { 0 } else { i.saturating_add(1) })
    } else if key.is(NamedKey::ArrowUp) {
        Some(i.checked_sub(1).unwrap_or(last))
    } else if key.is(NamedKey::Home) {
        Some(0)
    } else if key.is(NamedKey::End) {
        Some(last)
    } else {
        None
    }
}

#[must_use]
pub fn accordion() -> Div {
    div().named("accordion")
}

/// One section: its [`accordion_trigger`] and [`accordion_content`].
/// `border-b last:border-b-0`: give the last item `.border_0()`.
#[must_use]
pub fn accordion_item() -> Div {
    let t = theme();
    div().named("accordion-item").border_b().border_color(t.border)
}

/// The heading that opens and closes `value`, showing `label` and a chevron
/// that turns over while open.
#[must_use]
pub fn accordion_trigger(state: &AccordionState, value: &str, label: impl IntoElement) -> Div {
    let t = theme();
    let open = state.is_open(value);
    let (on_click, on_key) = (state.clone(), state.clone());
    let (click_value, key_value) = (value.to_owned(), value.to_owned());
    let trigger = div()
        .named("accordion-trigger")
        .track_focus(state.focus_handle(value))
        .flex()
        .flex_1()
        .items_start()
        .justify_between()
        .gap(4.0)
        .rounded_md()
        .py(4.0)
        .text_left()
        .text_sm()
        .font_medium()
        .transition_all()
        .hover(|s| s.underline())
        .focus_visible(move |s| focus_ring(s, &t))
        .disabled_style(|s| s.opacity(0.5))
        .on_click(move |_| on_click.toggle(&click_value))
        .on_key_down(move |key| on_key.navigate(&key_value, key))
        .child(label)
        .child(
            icon(Icon::ChevronDown)
                .named("accordion-chevron")
                .text_color(t.muted_foreground)
                // `translate-y-0.5`: a 2px nudge that does not grow the row.
                .mt(0.5)
                .transition_transform()
                .duration_ms(200.0)
                .when(open, |i| i.rotate(180.0)),
        );
    div().named("accordion-header").flex().child(trigger)
}

/// The section's body, `None` while closed (reka unmounts it once it has
/// rolled up); `body` fills its inner `pt-0 pb-4` box. It rolls down from no
/// height as it opens and back up as it closes; one open as its item mounts
/// shows at once.
#[must_use]
pub fn accordion_content(state: &AccordionState, value: &str, body: impl FnOnce(Div) -> Div) -> Option<Div> {
    state.is_open(value).then(|| {
        div()
            .named("accordion-content")
            .overflow_hidden()
            .text_sm()
            .animate(Animation::accordion_down().after_mount())
            .animate_out(Animation::accordion_up())
            .child(body(div().pt(0.0).pb(4.0)))
    })
}
