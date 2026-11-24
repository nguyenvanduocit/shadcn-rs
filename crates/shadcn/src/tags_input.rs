//! Tags input (`ui/tags-input`): a list of tags and a field that adds them.
//!
//! The behaviour is `primitives::tags_input`'s: Enter (or the `,`
//! delimiter) adds the typed text, trimmed, as a tag (a duplicate is
//! refused and stays typed), the keys select and remove tags from the
//! field's start, the delete button on a tag removes it, and a click
//! anywhere in the box focuses its field. The root keeps the tags, its own
//! (`default_value`) or the caller's (`model`, `v-model`); this module draws
//! it.

use gui::Properties;
use gui::prelude::*;
use gui::{
    Bind, Color, Div, Element, Icon, KeyEvent, Model, Style, TextField, component, div, icon, provide, text,
    text_input, use_model,
};

use primitives::Availability;
use primitives::listbox::Bound;
pub use primitives::tags_input::Delimiter;
use primitives::tags_input::{TagsConfig, TagsInput};

use crate::focus::{StateLook, destructive_ring, focus_ring};
use crate::part::{Part, PartProps, part_root};
use crate::theme::theme;

/// What [`tags_input`] is given.
#[derive(Clone, Default, PartialEq)]
pub struct TagsInputProps {
    tags: Bind<Vec<String>>,
    text: Bind<String>,
    invalid: bool,
    disabled: bool,
    /// How it adds (reka's `delimiter`, `max`, `addOnPaste`, `addOnTab`,
    /// `addOnBlur`, `duplicate`, and whether Enter adds).
    config: TagsConfig,
}

/// What the parts below a [`tags_input`] read.
#[derive(Clone, Copy)]
struct TagsInputContext {
    tags: TagsInput<String>,
}

impl TagsInputContext {
    /// The field's keys: the tags input's table.
    fn key(self, ev: &KeyEvent) -> bool {
        self.tags.key(ev)
    }

    /// The place of `tag` among the tags (tracked).
    fn index_of(self, tag: &str) -> Option<usize> {
        self.tags.tags().iter().position(|t| t == tag)
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
            let text = use_model(props, |p: &PartProps<TagsInputProps>| &p.props.text);
            let field = TextField::new(text.get_untracked());
            // The typed text up to the caller's model, and back.
            crate::part::bind_field(field, text);
            let tags = TagsInput::strings(
                Bound::new(use_model(props, |p: &PartProps<TagsInputProps>| &p.props.tags)),
                field,
                props.with_untracked(|p| p.props.config.clone()),
                primitives::use_direction(None),
            );
            let disabled = move || Availability::from_disabled(props.with(|p| p.props.disabled));
            tags.set_availability(gui::untracked(disabled));
            gui::watch(disabled, move |now, _| tags.set_availability(*now));
            gui::watch(
                move || props.with(|p| p.props.config.clone()),
                move |now, _| tags.set_config(now.clone()),
            );
            provide(TagsInputContext { tags });
            move || {
                let t = theme();
                let (invalid, root) = props.with(|p| (p.props.invalid, p.slot.0.clone()));
                let d = tags.root(root);
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
    pub fn on_change(self, f: impl Into<gui::Callback<Vec<String>>>) -> Self {
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
        self.with_props(|p| p.config.add_on_enter = adds)
    }

    /// What separates tags typed or pasted at once (`delimiter`; `,`).
    #[must_use]
    pub fn delimiter(self, delimiter: Delimiter) -> Self {
        self.with_props(|p| p.config.delimiter = delimiter)
    }

    /// The most tags it holds (`max`; 0, the default, is no limit).
    #[must_use]
    pub fn max(self, max: usize) -> Self {
        self.with_props(|p| p.config.max = max)
    }

    /// A paste adds its pieces as tags (`add-on-paste`).
    #[must_use]
    pub fn add_on_paste(self, adds: bool) -> Self {
        self.with_props(|p| p.config.add_on_paste = adds)
    }

    /// Tab adds the typed text (`add-on-tab`).
    #[must_use]
    pub fn add_on_tab(self, adds: bool) -> Self {
        self.with_props(|p| p.config.add_on_tab = adds)
    }

    /// The field losing focus adds the typed text (`add-on-blur`).
    #[must_use]
    pub fn add_on_blur(self, adds: bool) -> Self {
        self.with_props(|p| p.config.add_on_blur = adds)
    }

    /// The same tag may be added twice (`duplicate`).
    #[must_use]
    pub fn duplicate(self, duplicate: bool) -> Self {
        self.with_props(|p| p.config.duplicates = duplicate)
    }

    /// Not usable (`disabled`): nothing adds, removes or selects a tag.
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }
}

/// The tag `tag`; its text and delete button go in with `.child`. While the
/// keyboard selects it, it carries the ring (`data-[state=active]:ring-2
/// ring-ring ring-offset-2`).
#[must_use]
#[track_caller]
pub fn tags_input_item(tag: impl Into<String>) -> Part<String> {
    let t = theme();
    let root = div()
        .named("tags-input-item")
        .display(Display::Flex)
        .height(20.0)
        .align_items(Align::Center)
        .radius(8.0)
        .background(t.secondary)
        .text_color(t.secondary_foreground);
    Part::new(tag.into(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = part_root::<_, TagsInputContext>(props, "tags_input_item", "tags_input");
            move || {
                let (tag, root) = props.with(|p| (p.props.clone(), p.slot.0.clone()));
                let Some(ctx) = ctx else {
                    return root.into_element();
                };
                let Some(index) = ctx.index_of(&tag) else {
                    return root.into_element();
                };
                let t = theme();
                let selected = ctx.tags.selected() == Some(index);
                ctx.tags
                    .item(root, index)
                    .when(selected, |d| d.ring(Ring::new(2.0, t.ring).offset(2.0)))
                    .into_element()
            }
        })
    })
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
#[track_caller]
pub fn tags_input_item_delete(tag: impl Into<String>) -> Part<String> {
    let root = div()
        .named("tags-input-item-delete")
        .display(Display::Flex)
        .radius(6.0)
        .background(Color::TRANSPARENT)
        .margin_right(4.0)
        .cursor(Cursor::Pointer)
        .focusable()
        .child(icon(Icon::X));
    Part::new(tag.into(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = part_root::<_, TagsInputContext>(props, "tags_input_item_delete", "tags_input");
            move || {
                let (tag, root) = props.with(|p| (p.props.clone(), p.slot.0.clone()));
                match ctx.and_then(|c| c.index_of(&tag).map(|i| (c, i))) {
                    Some((ctx, index)) => ctx.tags.delete(root, index),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// The field new tags are typed into (`TagsInputInput`).
#[must_use]
#[track_caller]
pub fn tags_input_input() -> TagsInputInput {
    TagsInputInput {
        placeholder: String::new(),
        keys: None,
        at: std::panic::Location::caller(),
    }
}

/// A key handler that returns whether it took the key.
type KeyHandler = gui::Callback<KeyEvent, bool>;

/// A [`tags_input_input`] being built.
#[derive(Clone, PartialEq)]
pub struct TagsInputInput {
    placeholder: String,
    keys: Option<KeyHandler>,
    /// Where the caller's code made it (a report of a missing tags input names it).
    at: &'static std::panic::Location<'static>,
}

impl TagsInputInput {
    #[must_use]
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }

    /// Keys it takes before the box does (`@keydown`); whether `f` took one.
    #[must_use]
    pub fn on_key_down(mut self, f: impl Into<gui::Callback<KeyEvent, bool>>) -> Self {
        // Made here, so it runs as the caller's, not as the input's.
        self.keys = Some(f.into());
        self
    }
}

impl IntoElement for TagsInputInput {
    fn into_element(self) -> Element {
        component(self, |cx| {
            let props = cx.props();
            let ctx = primitives::context::use_root_at::<TagsInputContext>(
                "tags_input_input",
                "tags_input",
                props.with_untracked(|p| p.at),
            );
            // The caller's keys as passed last (a handle: the field keeps
            // its handler while new ones are passed).
            let keys = cx.event_opt(|p: &TagsInputInput| p.keys.as_ref());
            move || {
                let Some(ctx) = ctx else {
                    return Element::Empty;
                };
                let t = theme();
                props.with(|p| {
                    text_input(&ctx.tags.field())
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
                        .on_key_down({
                            // The caller's keys first, then the tags input's
                            // (before the field's own editing).
                            let keys = keys.clone();
                            move |ev| keys.call(ev.clone()) || ctx.key(ev)
                        })
                        .into_element()
                })
            }
        })
        .named("tags-input-input")
        .into_element()
    }
}
