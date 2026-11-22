//! Tags input (`ui/tags-input`): a list of tags and a field that adds them.
//!
//! As reka-ui's: Enter adds the typed text as a tag (once; a duplicate is
//! refused and stays typed), the delete button on a tag removes it, and a click anywhere in
//! the box focuses its field. The root keeps the tags, its own
//! (`default_value`) or the caller's (`model`, `v-model`).

use std::rc::Rc;

use gui::Properties;
use gui::prelude::*;
use gui::{
    Bind, Color, Div, Element, Icon, KeyEvent, Model, NamedKey, Slot, Style, TextField, UseModel, component, div, icon,
    provide, text, text_input, use_context, use_model,
};

use crate::focus::{StateLook, destructive_ring, focus_ring};
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`tags_input`] is given.
#[derive(Clone, PartialEq)]
pub struct TagsInputProps {
    tags: Bind<Vec<String>>,
    text: Bind<String>,
    invalid: bool,
    add_on_enter: bool,
}

impl Default for TagsInputProps {
    fn default() -> Self {
        Self {
            tags: Bind::default(),
            text: Bind::default(),
            invalid: false,
            add_on_enter: true,
        }
    }
}

/// What the parts below a [`tags_input`] read.
#[derive(Clone, Copy)]
struct TagsInputContext {
    tags: UseModel<PartProps<TagsInputProps>, Vec<String>>,
    /// The text being typed.
    field: TextField,
}

impl TagsInputContext {
    /// Add the typed text as a tag and clear the field; a duplicate is
    /// refused and stays typed. Whether Enter was the input's.
    fn add(self) -> bool {
        let typed = self.field.edit.with_untracked(|e| e.text.clone());
        let tag = typed.trim();
        if tag.is_empty() {
            return false;
        }
        if !self.tags.with(|tags| tags.iter().any(|t| t == tag)) {
            let tag = tag.to_owned();
            self.tags.update(|tags| tags.push(tag));
            self.field.set_text("");
        }
        true
    }

    fn remove(self, tag: &str) {
        self.tags.update(|tags| tags.retain(|t| t != tag));
    }
}

/// The box (`<TagsInput>`): its items and its [`tags_input_input`] go in
/// with `.child`. Its tags are its own (`default_value`) or the caller's
/// (`model`, `v-model`, which a caller listing them binds).
#[must_use]
pub fn tags_input() -> Part<TagsInputProps> {
    let t = theme();
    let root = div()
        .named("tags-input")
        .transition(Properties::COLOR | Properties::SHADOW)
        .display(Display::Flex)
        .flex_wrap(true)
        .gap(8.0)
        .align_items(Align::Center)
        .radius(8.0)
        .border(1.0)
        .border_color(t.input)
        .background(t.background)
        .padding_x(8.0)
        .padding_y(4.0)
        .text_size(14.0)
        .line_height(20.0)
        .shadow(Shadow {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: 0.0,
            color: Color::BLACK.opacity(0.05),
        })
        .cursor(Cursor::Text);
    Part::new(TagsInputProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let text = use_model(props, |p| &p.props.text);
            let field = TextField::new(text.get_untracked());
            // The typed text up to the caller's model, and back.
            crate::part::bind_field(field, text);
            let ctx = TagsInputContext {
                tags: use_model(props, |p| &p.props.tags),
                field,
            };
            provide(ctx);
            move || {
                let t = theme();
                let (invalid, adds, root) = props.with(|p| (p.props.invalid, p.props.add_on_enter, p.slot.0.clone()));
                let d = root
                    .on_click(move |_| field.focus.focus())
                    .when(adds, |d| d.on_key_down(move |ev| ev.is(NamedKey::Enter) && ctx.add()));
                // `focus-within:` and `aria-invalid:` are states of the box,
                // not of the field that holds focus: the box's own
                // `focus_within`, bound to its border and ring.
                d.with_interaction(|d, i| {
                    if invalid {
                        let ring = destructive_ring(Style::default(), &t).border_color(t.destructive);
                        d.border_color(t.destructive)
                            .ring_while(move || i.focus_within(), &ring)
                    } else {
                        d.ring_while(move || i.focus_within(), &focus_ring(Style::default(), &t))
                    }
                })
                .into_element()
            }
        })
    })
}

impl Part<TagsInputProps> {
    /// The tags, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<Vec<String>>>) -> Self {
        self.with_props(|p| p.tags = p.tags.clone().with_model(model))
    }

    /// The tags it starts with when no model is given (`default-value`).
    #[must_use]
    pub fn default_value<S: Into<String>>(self, tags: impl IntoIterator<Item = S>) -> Self {
        let tags: Vec<String> = tags.into_iter().map(Into::into).collect();
        self.with_props(|p| p.tags = p.tags.clone().with_default(tags))
    }

    /// Call `f` with the tags after each change (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(Vec<String>) + 'static) -> Self {
        self.with_props(|p| p.tags = p.tags.clone().on_change(f))
    }

    /// The text typed in its field, kept by the caller. A listbox's filter
    /// (`ListboxFilter v-model="searchTerm" as-child`) binds the same model
    /// as its [`command`](crate::command)'s [`search`](Part::search), so
    /// what is typed filters that list.
    #[must_use]
    pub fn text(self, text: impl Into<Model<String>>) -> Self {
        self.with_props(|p| p.text = p.text.clone().with_model(text))
    }

    /// Destructive (`aria-invalid`), as a form decides.
    #[must_use]
    pub fn invalid(self, invalid: bool) -> Self {
        self.with_props(|p| p.invalid = invalid)
    }

    /// Whether Enter adds the typed text as a tag (`true`; a field whose
    /// Enter picks from a listbox, `@keydown.enter.prevent`, says `false`).
    #[must_use]
    pub fn add_on_enter(self, adds: bool) -> Self {
        self.with_props(|p| p.add_on_enter = adds)
    }
}

/// One tag; its text and delete button go in with `.child`.
#[must_use]
pub fn tags_input_item() -> Div {
    let t = theme();
    div()
        .named("tags-input-item")
        .display(Display::Flex)
        .height(20.0)
        .align_items(Align::Center)
        .radius(8.0)
        .background(t.secondary)
        .text_color(t.secondary_foreground)
}

/// A tag's text.
#[must_use]
pub fn tags_input_item_text(tag: impl Into<String>) -> Div {
    div()
        .named("tags-input-item-text")
        .padding_y(2.0)
        .padding_x(8.0)
        .text_size(14.0)
        .line_height(20.0)
        .radius(6.0)
        .child(text(tag))
}

/// The button that removes `tag`.
#[must_use]
pub fn tags_input_item_delete(tag: impl Into<String>) -> Part<String> {
    let root = div()
        .named("tags-input-item-delete")
        .display(Display::Flex)
        .radius(6.0)
        .background(Color::TRANSPARENT)
        .margin_right(4.0)
        .cursor(Cursor::Pointer)
        .focusable()
        .aria_label("Remove")
        .child(icon(Icon::X));
    Part::new(tag.into(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = use_context::<TagsInputContext>();
            move || {
                let (tag, root) = props.with(|p| (p.props.clone(), p.slot.0.clone()));
                match ctx {
                    Some(ctx) => root.on_click(move |_| ctx.remove(&tag)),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// The field new tags are typed into (`TagsInputInput`).
#[must_use]
pub fn tags_input_input() -> TagsInputInput {
    TagsInputInput {
        placeholder: String::new(),
        keys: None,
    }
}

/// A key handler that returns whether it took the key.
type KeyHandler = Rc<dyn Fn(&KeyEvent) -> bool>;

/// A [`tags_input_input`] being built.
#[derive(Clone, PartialEq)]
pub struct TagsInputInput {
    placeholder: String,
    keys: Option<Slot<KeyHandler>>,
}

impl TagsInputInput {
    #[must_use]
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }

    /// Keys it takes before the box does (`@keydown`); whether `f` took one.
    #[must_use]
    pub fn on_key_down(mut self, f: impl Fn(&KeyEvent) -> bool + 'static) -> Self {
        self.keys = Some(Slot(Rc::new(f)));
        self
    }
}

impl IntoElement for TagsInputInput {
    fn into_element(self) -> Element {
        component(self, |cx| {
            let props = cx.props();
            let ctx = use_context::<TagsInputContext>();
            move || {
                let Some(ctx) = ctx else {
                    return Element::Empty;
                };
                let t = theme();
                props.with(|p| {
                    text_input(&ctx.field)
                        .named("tags-input-input")
                        .placeholder(p.placeholder.clone())
                        .placeholder_color(t.muted_foreground)
                        .selection_color(t.primary.opacity(0.35))
                        .text_color(t.foreground)
                        .text_size(14.0)
                        .line_height(20.0)
                        .min_height(20.0)
                        .flex(Flex::grow(1.0))
                        .background(Color::TRANSPARENT)
                        .padding_x(4.0)
                        .when_some(p.keys.clone(), |i, keys| i.on_key_down(move |ev| keys(ev)))
                        .into_element()
                })
            }
        })
        .named("tags-input-input")
        .into_element()
    }
}
