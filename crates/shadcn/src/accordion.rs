//! Accordion (`ui/accordion`): stacked headings, each opening its own section.
//!
//! Keyboard as reka's: Up and Down (wrapping), Home and End move between the
//! triggers; Enter and Space open and close.

use gui::prelude::*;
use gui::{
    Animation, Bind, Element, FocusHandle, Icon, KeyEvent, Members, Model, NamedKey, Signal, UseModel, component, div,
    icon, provide, use_context, use_model,
};

use crate::focus::StateLook;
use crate::part::{Part, PartProps};
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

/// What [`accordion`] is given.
#[derive(Clone, Debug, PartialEq)]
pub struct AccordionProps {
    kind: AccordionKind,
    value: Bind<Vec<String>>,
}

impl Default for AccordionProps {
    fn default() -> Self {
        Self {
            kind: AccordionKind::Single { collapsible: false },
            value: Bind::default(),
        }
    }
}

/// What the parts below [`accordion`] read.
#[derive(Clone)]
struct AccordionContext {
    props: Signal<PartProps<AccordionProps>>,
    open: UseModel<PartProps<AccordionProps>, Vec<String>>,
    /// The triggers in tree order, as last laid out.
    triggers: Members<FocusHandle>,
}

impl AccordionContext {
    fn is_open(&self, value: &str) -> bool {
        self.open.with(|open| open.iter().any(|v| v == value))
    }

    fn toggle(&self, value: &str) {
        let kind = self.props.with_untracked(|p| p.props.kind);
        self.open
            .update(|open| *open = toggled(kind, std::mem::take(open), value));
    }

    /// Move focus from the trigger `from` as `key` asks; whether the key was
    /// one of the accordion's.
    fn navigate(&self, from: FocusHandle, key: &KeyEvent) -> bool {
        let target = self.triggers.with(|triggers| {
            let current = triggers.iter().position(|h| *h == from);
            roving_target(triggers.len(), current, key).map(|i| triggers.get(i).copied())
        });
        let Some(target) = target else {
            return false;
        };
        if let Some(handle) = target {
            gui::focus::set_focus(Some(handle.id()), true);
        }
        true
    }
}

/// What an [`accordion_item`] gives its trigger and content: its value.
#[derive(Clone, Copy)]
struct ItemContext {
    props: Signal<PartProps<AccordionItemProps>>,
}

impl ItemContext {
    fn value(&self) -> String {
        self.props.with(|p| p.props.value.clone())
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

/// The accordion: its items go inside. One section opens at a time unless
/// it is made [`multiple`](Part::multiple).
#[must_use]
pub fn accordion() -> Part<AccordionProps> {
    Part::new(AccordionProps::default(), div().named("accordion"), |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(AccordionContext {
                props,
                open: use_model(props, |p| &p.props.value),
                triggers: Members::new(),
            });
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

impl Part<AccordionProps> {
    /// At most one section open (reka's `type="single"`); `collapsible` lets
    /// the open one close, leaving none.
    #[must_use]
    pub fn single(self, collapsible: bool) -> Self {
        self.with_props(|p| p.kind = AccordionKind::Single { collapsible })
    }

    /// Any number open (reka's `type="multiple"`).
    #[must_use]
    pub fn multiple(self) -> Self {
        self.with_props(|p| p.kind = AccordionKind::Multiple)
    }

    /// The sections open first, when no model is given (reka's `default-value`).
    #[must_use]
    pub fn default_value(self, values: &[&str]) -> Self {
        let values = values.iter().map(|v| (*v).to_owned()).collect();
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_default(values))
    }

    /// The open values, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<Vec<String>>>) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_model(model))
    }

    /// The open values as the caller shows them (`:model-value`); a toggle
    /// only calls `on_change`.
    #[must_use]
    pub fn value(self, values: &[&str]) -> Self {
        let values = values.iter().map(|v| (*v).to_owned()).collect();
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_value(values))
    }

    /// Called with the open values after each toggle (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(Vec<String>) + 'static) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).on_change(f))
    }
}

/// What an [`accordion_item`] is given: the value it opens as.
#[derive(Clone, Debug, PartialEq)]
pub struct AccordionItemProps {
    value: String,
}

/// One section, opening as `value`: its [`accordion_trigger`] and
/// [`accordion_content`] go inside. `border-b last:border-b-0`: give the last
/// item `.border(0.0)`.
#[must_use]
pub fn accordion_item(value: impl Into<String>) -> Part<AccordionItemProps> {
    let t = theme();
    let root = div().named("accordion-item").border_bottom(1.0).border_color(t.border);
    Part::new(AccordionItemProps { value: value.into() }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(ItemContext { props });
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

/// The heading that opens and closes its item, showing `label` and a
/// chevron that turns over while open.
#[must_use]
pub fn accordion_trigger(label: impl IntoElement) -> Part<()> {
    let t = theme();
    let trigger = div()
        .named("accordion-trigger")
        .display(Display::Flex)
        .flex(Flex::grow(1.0))
        .align_items(Align::Start)
        .justify_content(Justify::Between)
        .gap(16.0)
        .radius(8.0)
        .padding_y(16.0)
        .text_align(TextAlign::Left)
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500)
        .transition(Properties::ALL)
        .with_interaction(|d, i| {
            d.underline(move || i.hovered())
                .focus_ring_on(i, &t)
                .dim_disabled(i, 0.5)
        })
        .child(label);
    Part::new((), trigger, |p| {
        component(p, |cx| {
            let props = cx.props();
            let root = use_context::<AccordionContext>();
            let item = use_context::<ItemContext>();
            let handle = FocusHandle::new();
            move || {
                let trigger = props.with(|p| p.slot.0.clone());
                let (Some(root), Some(item)) = (root.clone(), item) else {
                    return trigger.into_element();
                };
                let t = theme();
                let value = item.value();
                let open = root.is_open(&value);
                let (on_click, on_key) = (root.clone(), root.clone());
                let trigger = trigger
                    .aria_expanded(open)
                    .track_focus(handle)
                    .member(&root.triggers, handle)
                    .on_click(move |_| on_click.toggle(&value))
                    .on_key_down(move |key| on_key.navigate(handle, key))
                    .child(
                        icon(Icon::ChevronDown)
                            .named("accordion-chevron")
                            .text_color(t.muted_foreground)
                            // `translate-y-0.5`: a 2px nudge that does not grow the row.
                            .margin_top(2.0)
                            .transition(Properties::TRANSFORM)
                            .duration_ms(200.0)
                            .when(open, |i| i.rotate(180.0)),
                    );
                div()
                    .named("accordion-header")
                    .display(Display::Flex)
                    .child(trigger)
                    .into_element()
            }
        })
    })
}

/// The section's body, shown while its item is open (reka unmounts it once it
/// has rolled up); what goes in fills its inner `pt-0 pb-4` box. It rolls down
/// from no height as it opens and back up as it closes; one open as its item
/// mounts shows at once.
#[must_use]
pub fn accordion_content() -> Part<()> {
    Part::new((), div().padding_top(0.0).padding_bottom(16.0), |p| {
        component(p, |cx| {
            let props = cx.props();
            let root = use_context::<AccordionContext>();
            let item = use_context::<ItemContext>();
            move || {
                let open = match (&root, item) {
                    (Some(root), Some(item)) => root.is_open(&item.value()),
                    _ => true,
                };
                if !open {
                    return Element::Empty;
                }
                div()
                    .named("accordion-content")
                    .overflow(Overflow::Hidden)
                    .text_size(14.0)
                    .line_height(20.0)
                    .animation(Animation::accordion_down().after_mount())
                    .exit_animation(Animation::accordion_up())
                    .child(props.with(|p| p.slot.0.clone()))
                    .into_element()
            }
        })
    })
}
