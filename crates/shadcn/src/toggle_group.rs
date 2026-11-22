//! Toggle group (`ui/toggle-group`): a row of toggles, one or many pressed.
//!
//! [`toggle_group`] (reka's `type="single"`) or [`toggle_group_multiple`]
//! keeps which items are pressed: its own, from `default_value`, or the
//! caller's through `.model(…)` (`v-model`); `on_change` hears each press.
//! Its look (variant, size, spacing) and `disabled` go to every item, as
//! shadcn's `ToggleGroup` provides them. Arrow keys move focus between the
//! items, wrapping, Home and End to the ends.

use std::rc::Rc;

use gui::Color;
use gui::prelude::*;
use gui::{
    Bind, Callback, Div, Element, FocusHandle, Members, Model, NamedKey, Signal, Slot, component, div, provide,
    use_context, use_model,
};

use crate::checkbox::Class;
use crate::part::Part;
use crate::select::{Chosen, SelectModel};
use crate::toggle::{ToggleSize, ToggleVariant, toggle_variants};

/// What a toggle group is given: which items are pressed, and the look its
/// items share.
#[derive(Clone, PartialEq)]
pub struct ToggleGroupProps<M: SelectModel> {
    value: Bind<M>,
    look: Look,
}

/// The look and state every item of a group shares.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
struct Look {
    variant: ToggleVariant,
    size: ToggleSize,
    /// Gap between items on Tailwind's scale; 0 joins them into one bar.
    spacing: f32,
    disabled: bool,
}

/// What the items below a group read.
#[derive(Clone)]
struct ToggleGroupContext {
    value: Rc<dyn Chosen>,
    look: Rc<dyn Fn() -> Look>,
    /// The items in tree order, as last laid out, each with its focus.
    items: Members<(String, FocusHandle)>,
    /// The values of the items in tree order (written after layout, read by
    /// the items for their joined corners).
    order: Signal<Vec<String>>,
}

impl ToggleGroupContext {
    /// Read the laid-out items' order into [`ToggleGroupContext::order`].
    fn refresh(&self) {
        if self.order.is_alive() {
            let values: Vec<String> = self.items.with(|items| items.iter().map(|(v, _)| v.clone()).collect());
            self.order.set(values);
        }
    }

    /// Focus the item `at`, or `by` items from the focused one, wrapping.
    fn move_focus(&self, by: isize) -> bool {
        self.items.with(|items| {
            let len = items.len();
            let Some(at) = items.iter().position(|(_, f)| f.is_focused()) else {
                return false;
            };
            let next = isize::try_from(at)
                .ok()
                .and_then(|a| a.checked_add(by))
                .and_then(|n| n.checked_rem_euclid(isize::try_from(len).ok()?))
                .and_then(|n| usize::try_from(n).ok());
            next.and_then(|n| items.get(n)).is_some_and(|(_, f)| {
                gui::focus::set_focus(Some(f.id()), true);
                true
            })
        })
    }

    fn focus_end(&self, last: bool) -> bool {
        self.items.with(|items| {
            let item = if last { items.last() } else { items.first() };
            item.is_some_and(|(_, f)| {
                gui::focus::set_focus(Some(f.id()), true);
                true
            })
        })
    }
}

/// A group where at most one item is pressed (`type="single"`); its items
/// go in with `.child(toggle_group_item(…))`.
#[must_use]
pub fn toggle_group() -> Part<ToggleGroupProps<Option<String>>> {
    root()
}

/// A group where any number of items are pressed (`type="multiple"`).
#[must_use]
pub fn toggle_group_multiple() -> Part<ToggleGroupProps<Vec<String>>> {
    root()
}

fn root<M: SelectModel>() -> Part<ToggleGroupProps<M>> {
    let props = ToggleGroupProps {
        value: Bind::default(),
        look: Look::default(),
    };
    Part::new(
        props,
        div()
            .named("toggle-group")
            .display(Display::Flex)
            .align_items(Align::Center)
            .radius(8.0),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let value = use_model(props, |p| &p.props.value);
                let ctx = ToggleGroupContext {
                    value: Rc::new(value),
                    look: Rc::new(move || props.with(|p| p.props.look)),
                    items: Members::new(),
                    order: Signal::new(Vec::new()),
                };
                provide(ctx.clone());
                move || {
                    let (look, root) = props.with(|p| (p.props.look, p.slot.0.clone()));
                    let keys = ctx.clone();
                    root.gap(look.spacing * 4.0)
                        .when(look.spacing == 0.0 && look.variant == ToggleVariant::Outline, |e| {
                            e.shadow(Shadow {
                                x: 0.0,
                                y: 1.0,
                                blur: 2.0,
                                spread: 0.0,
                                color: Color::BLACK.opacity(0.05),
                            })
                        })
                        .on_key_down(move |ev| match &ev.key {
                            gui::Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown) => keys.move_focus(1),
                            gui::Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => keys.move_focus(-1),
                            gui::Key::Named(NamedKey::Home) => keys.focus_end(false),
                            gui::Key::Named(NamedKey::End) => keys.focus_end(true),
                            _ => false,
                        })
                        .into_element()
                }
            })
        },
    )
}

impl<M: SelectModel> Part<ToggleGroupProps<M>> {
    /// The pressed items, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<M>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// The items pressed first, when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(self, value: M) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_default(value))
    }

    /// Call `f` with what is pressed after each press (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(M) + 'static) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    #[must_use]
    pub fn variant(self, variant: ToggleVariant) -> Self {
        self.with_props(|p| p.look.variant = variant)
    }

    #[must_use]
    pub fn size(self, size: ToggleSize) -> Self {
        self.with_props(|p| p.look.size = size)
    }

    /// The gap between items on Tailwind's scale; 0 (the default) joins them.
    #[must_use]
    pub fn spacing(self, spacing: f32) -> Self {
        self.with_props(|p| p.look.spacing = spacing)
    }

    /// Every item disabled.
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.look.disabled = disabled)
    }
}

/// The item `value` of the group around it; add its icon or label with
/// `.child`.
#[must_use]
pub fn toggle_group_item(value: impl Into<String>) -> ToggleGroupItem {
    ToggleGroupItem {
        value: value.into(),
        disabled: false,
        children: Slot(Vec::new()),
        class: None,
        on_click: None,
    }
}

/// A [`toggle_group_item`] being built.
#[derive(Clone, PartialEq)]
pub struct ToggleGroupItem {
    value: String,
    disabled: bool,
    children: Slot<Vec<Element>>,
    class: Option<Class>,
    on_click: Option<Callback<()>>,
}

impl ToggleGroupItem {
    #[must_use]
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.0.push(child.into_element());
        self
    }

    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Classes after the item's own, its state's included.
    #[must_use]
    pub fn class(mut self, f: impl Fn(Div) -> Div + 'static) -> Self {
        self.class = Some(Slot(Rc::new(f)));
        self
    }

    /// Call `f` after each press of this item (`@click`).
    #[must_use]
    pub fn on_click(mut self, f: impl Fn() + 'static) -> Self {
        self.on_click = Some(Callback::new(move |()| f()));
        self
    }
}

impl IntoElement for ToggleGroupItem {
    fn into_element(self) -> Element {
        component(self, |cx| {
            let props = cx.props();
            let group = use_context::<ToggleGroupContext>();
            let focus = FocusHandle::new();
            if let Some(g) = group.clone() {
                // Its place among the items is known once it is laid out.
                let mounted = g.clone();
                gui::after_layout(move || mounted.refresh());
                gui::on_unmounted(move || gui::after_layout(move || g.refresh()));
            }
            move || {
                let Some(g) = group.clone() else {
                    return Element::Empty;
                };
                let look = (g.look)();
                props.with(|p| {
                    let order = g.order.get();
                    let at = order.iter().position(|v| *v == p.value);
                    let first = at == Some(0);
                    let last = at.is_some_and(|i| i.saturating_add(1) == order.len());
                    let joined = look.spacing == 0.0;
                    let outline = look.variant == ToggleVariant::Outline;
                    let (value, chosen, after) = (p.value.clone(), Rc::clone(&g.value), p.on_click.clone());
                    let d = toggle_variants(look.variant, look.size, g.value.holds(&p.value))
                        .named("toggle-group-item")
                        .width(Length::Auto)
                        .min_width(0.0)
                        .flex_shrink(0.0)
                        .padding_x(12.0)
                        .track_focus(focus)
                        .member(&g.items, (p.value.clone(), focus))
                        .disabled(look.disabled || p.disabled)
                        .when(joined, |d| {
                            d.radius(0.0)
                                .shadow(Shadows::new())
                                .when(first, |d| d.radius_left(8.0))
                                .when(last, |d| d.radius_right(8.0))
                                // `border-l-0` except the first: neighbours share one rule.
                                .when(outline && !first, |d| {
                                    d.border(0.0).border_top(1.0).border_right(1.0).border_bottom(1.0)
                                })
                        })
                        .on_click(move |_| {
                            chosen.flip(&value);
                            if let Some(f) = &after {
                                f.call(());
                            }
                        })
                        .children(p.children.0.iter().cloned());
                    match &p.class {
                        Some(f) => f(d),
                        None => d,
                    }
                    .into_element()
                })
            }
        })
        .named("toggle_group_item")
        .into_element()
    }
}
