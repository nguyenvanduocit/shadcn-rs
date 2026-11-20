//! Tags input (`ui/tags-input`): a list of tags and a field that adds them.
//!
//! As reka-ui's: Enter adds the typed text as a tag (once; a duplicate is
//! refused and stays typed), the delete button on a tag removes it, and a click anywhere in
//! the box focuses its field.

use gui::Properties;
use gui::prelude::*;
use gui::{Color, Div, Icon, NamedKey, Signal, TextField, TextInput, div, icon, text, text_input};

use crate::button_group::with_style;
use crate::focus::{destructive_ring, focus_ring};
use crate::theme::theme;

/// A tags input's state; create it once, outside the render closure.
#[derive(Clone)]
pub struct TagsInputState {
    /// The tags (`v-model`).
    pub tags: Signal<Vec<String>>,
    /// The text being typed.
    pub field: TextField,
}

impl TagsInputState {
    #[must_use]
    pub fn new<S: Into<String>>(tags: impl IntoIterator<Item = S>) -> Self {
        Self {
            tags: Signal::new(tags.into_iter().map(Into::into).collect()),
            field: TextField::new(""),
        }
    }

    /// Add the typed text as a tag and clear the field; a duplicate is
    /// refused and stays typed. Whether Enter was the input's.
    fn add(&self) -> bool {
        let typed = self.field.text();
        let tag = typed.trim();
        if tag.is_empty() {
            return false;
        }
        if !self.tags.get().iter().any(|t| t == tag) {
            let tag = tag.to_owned();
            self.tags.update(|mut tags| {
                tags.push(tag);
                tags
            });
            self.field.set_text("");
        }
        true
    }

    fn remove(&self, tag: &str) {
        self.tags.update(|mut tags| {
            tags.retain(|t| t != tag);
            tags
        });
    }
}

/// The box: its items and its [`tags_input_input`] go in with `.child`;
/// `invalid` marks it destructive (`aria-invalid`).
#[must_use]
pub fn tags_input(state: &TagsInputState, invalid: bool) -> Div {
    let t = theme();
    let focused = state.field.focus.is_focused();
    let keys = state.clone();
    let focus = state.field.focus;
    let d = div()
        .named("tags-input")
        .transition_props(Properties::COLOR | Properties::SHADOW)
        .flex()
        .flex_wrap()
        .gap(2.0)
        .items_center()
        .rounded_md()
        .border()
        .border_color(t.input)
        .bg(t.background)
        .px(2.0)
        .py(1.0)
        .text_sm()
        .shadow_xs()
        .cursor_text()
        .on_click(move |_| focus.focus())
        .on_key_down(move |ev| ev.is(NamedKey::Enter) && keys.add());
    // `focus-within:` and `aria-invalid:` are states of the box, not of the
    // field that holds focus, so they are read here.
    with_style(d, |s| match (focused, invalid) {
        (false, false) => s,
        (false, true) => s.border_color(t.destructive),
        (true, false) => focus_ring(s, &t),
        (true, true) => destructive_ring(s, &t).border_color(t.destructive),
    })
}

/// One tag; its text and delete button go in with `.child`.
#[must_use]
pub fn tags_input_item() -> Div {
    let t = theme();
    div()
        .named("tags-input-item")
        .flex()
        .h(5.0)
        .items_center()
        .rounded_md()
        .bg(t.secondary)
        .text_color(t.secondary_foreground)
}

/// A tag's text.
#[must_use]
pub fn tags_input_item_text(tag: impl Into<String>) -> Div {
    div()
        .named("tags-input-item-text")
        .py(0.5)
        .px(2.0)
        .text_sm()
        .rounded_sm()
        .child(text(tag))
}

/// The button that removes `tag`.
#[must_use]
pub fn tags_input_item_delete(state: &TagsInputState, tag: &str) -> Div {
    let remove = state.clone();
    let tag = tag.to_owned();
    div()
        .named("tags-input-item-delete")
        .flex()
        .rounded_sm()
        .bg(Color::TRANSPARENT)
        .mr(1.0)
        .cursor_pointer()
        .focusable()
        .on_click(move |_| remove.remove(&tag))
        .child(icon(Icon::X))
}

/// The field new tags are typed into.
#[must_use]
pub fn tags_input_input(state: &TagsInputState) -> TextInput {
    let t = theme();
    text_input(&state.field)
        .named("tags-input-input")
        .placeholder_color(t.muted_foreground)
        .selection_color(t.primary.opacity(0.35))
        .text_color(t.foreground)
        .text_sm()
        .min_h(5.0)
        .flex_1()
        .bg(Color::TRANSPARENT)
        .px(1.0)
}
