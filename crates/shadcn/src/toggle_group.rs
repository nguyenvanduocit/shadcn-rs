//! Toggle group (`ui/toggle-group`): a row of toggles, one or many pressed.

use std::rc::Rc;

use gui::prelude::*;
use gui::{Div, FocusHandle, NamedKey, Radius, Signal, div};

use crate::toggle::{ToggleSize, ToggleVariant, toggle_variants};

/// Which items are pressed: at most one (`type="single"`) or any number (`type="multiple"`).
#[derive(Clone)]
pub enum ToggleGroupSelection {
    Single(Signal<Option<String>>),
    Multiple(Signal<Vec<String>>),
}

impl ToggleGroupSelection {
    /// Whether the item `value` is pressed (reactive).
    #[must_use]
    pub fn is_on(&self, value: &str) -> bool {
        match self {
            Self::Single(s) => s.get().as_deref() == Some(value),
            Self::Multiple(s) => s.get().iter().any(|v| v == value),
        }
    }

    /// Press `value`, or release it when pressed.
    pub fn flip(&self, value: &str) {
        match self {
            Self::Single(s) => s.update(|cur| (cur.as_deref() != Some(value)).then(|| value.to_owned())),
            Self::Multiple(s) => s.update(|mut cur| {
                if let Some(i) = cur.iter().position(|v| v == value) {
                    cur.remove(i);
                } else {
                    cur.push(value.to_owned());
                }
                cur
            }),
        }
    }
}

/// A toggle group's state and the look its items share (the Vue `provide`).
/// Create it once, outside the render closure.
#[derive(Clone)]
pub struct ToggleGroupState {
    pub selection: ToggleGroupSelection,
    pub variant: ToggleVariant,
    pub size: ToggleSize,
    /// Gap between items on Tailwind's scale; 0 joins them into one bar.
    pub spacing: f32,
    pub disabled: Signal<bool>,
    /// The items in order, each with the focus handle arrow keys move between.
    items: Rc<[(String, FocusHandle)]>,
}

impl ToggleGroupState {
    /// A group of the items `values`, in order, pressed as `selection` holds.
    #[must_use]
    pub fn new(selection: ToggleGroupSelection, values: &[&str]) -> Self {
        Self {
            selection,
            variant: ToggleVariant::Default,
            size: ToggleSize::Default,
            spacing: 0.0,
            disabled: Signal::new(false),
            items: values.iter().map(|v| ((*v).to_owned(), FocusHandle::new())).collect(),
        }
    }

    #[must_use]
    pub fn variant(mut self, variant: ToggleVariant) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub fn size(mut self, size: ToggleSize) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Move focus `by` items from the focused one, wrapping (roving focus).
    fn move_focus(&self, by: isize) -> bool {
        let len = self.items.len();
        let Some(at) = self.items.iter().position(|(_, f)| f.is_focused()) else {
            return false;
        };
        let Some(next) = isize::try_from(at)
            .ok()
            .and_then(|a| a.checked_add(by))
            .and_then(|n| n.checked_rem_euclid(isize::try_from(len).ok()?))
            .and_then(|n| usize::try_from(n).ok())
        else {
            return false;
        };
        self.focus_item(next)
    }

    fn focus_item(&self, at: usize) -> bool {
        self.items.get(at).is_some_and(|(_, f)| {
            gui::focus::set_focus(Some(f.id()), true);
            true
        })
    }
}

/// The group's row; its items go in with `.child(toggle_group_item(...))`.
/// Arrow keys move focus between the items, Home and End to the ends.
#[must_use]
pub fn toggle_group(state: &ToggleGroupState) -> Div {
    let keys = state.clone();
    div()
        .named("toggle-group")
        .flex()
        .items_center()
        .gap(state.spacing)
        .rounded_md()
        .when(
            state.spacing == 0.0 && state.variant == ToggleVariant::Outline,
            Styled::shadow_xs,
        )
        .on_key_down(move |ev| match &ev.key {
            gui::Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown) => keys.move_focus(1),
            gui::Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => keys.move_focus(-1),
            gui::Key::Named(NamedKey::Home) => keys.focus_item(0),
            gui::Key::Named(NamedKey::End) => keys.focus_item(keys.items.len().saturating_sub(1)),
            _ => false,
        })
}

/// The item `value` of `state`'s group; add its icon or label with `.child`.
#[must_use]
pub fn toggle_group_item(state: &ToggleGroupState, value: &str) -> Div {
    let at = state.items.iter().position(|(v, _)| v == value);
    let first = at == Some(0);
    let last = at.is_some_and(|i| i.saturating_add(1) == state.items.len());
    let focus = at.and_then(|i| state.items.get(i)).map(|(_, f)| *f);
    let selection = state.selection.clone();
    let owned = value.to_owned();
    let joined = state.spacing == 0.0;
    let outline = state.variant == ToggleVariant::Outline;
    toggle_variants(state.variant, state.size, state.selection.is_on(value))
        .named("toggle-group-item")
        .w_auto()
        .min_w(0.0)
        .shrink_0()
        .px(3.0)
        .when_some(focus, Div::track_focus)
        .disabled(state.disabled.get())
        .when(joined, |d| {
            d.rounded_none()
                .shadow_none()
                .when(first, |d| d.rounded_l(Radius::MD))
                .when(last, |d| d.rounded_r(Radius::MD))
                // `border-l-0` except the first: neighbours share one rule.
                .when(outline && !first, |d| d.border_0().border_t().border_r().border_b())
        })
        .on_click(move |_| selection.flip(&owned))
}
