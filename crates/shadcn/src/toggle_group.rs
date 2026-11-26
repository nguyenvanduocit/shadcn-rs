//! Toggle group (`ui/toggle-group`): a row of toggles, one or many pressed.
//!
//! [`toggle_group`] (reka's `type="single"`) or [`toggle_group_multiple`]
//! keeps which items are pressed: its own, from `default_value`, or the
//! caller's through `.model(…)` (`v-model`); `on_change` hears each press.
//! Its look (variant, size, spacing) and `disabled` go to every item, as
//! shadcn's `ToggleGroup` provides them. The group is one Tab stop, and the
//! arrow keys move focus between the items, wrapping, Home and End to the
//! ends (`primitives::toggle_group`, keymap `TOGGLE_GROUP`).

use std::rc::Rc;

use gui::Color;
use gui::prelude::*;
use gui::{
    Bind, Callback, Div, Element, FocusHandle, Model, Signal, Slot, UseModel, component, div, provide, use_model,
};
pub use primitives::toggle_group::Pressing;
use primitives::toggle_group::ToggleGroup;
use primitives::{Availability, use_direction};

use crate::checkbox::Class;
use crate::part::{Part, PartProps};
use crate::toggle::{ToggleSize, ToggleVariant, toggle_variants};

/// What a toggle group holds: one pressed value or none, or any number.
pub trait ToggleGroupModel: Pressing<String> + Default + std::fmt::Debug {}

impl<M: Pressing<String> + Default + std::fmt::Debug> ToggleGroupModel for M {}

/// What a toggle group is given: which items are pressed, and the look its
/// items share.
#[derive(Clone, PartialEq)]
pub struct ToggleGroupProps<M: ToggleGroupModel> {
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

/// The group's state its items reach, whichever it holds.
trait Group {
    /// Whether `value` is pressed (tracked).
    #[expect(
        clippy::ptr_arg,
        reason = "the primitive's values are `String`s, compared as they are"
    )]
    fn pressed(&self, value: &String) -> bool;
    /// `d` as the item of `value`, running `after` once a press it took
    /// has written the group.
    fn item(&self, d: Div, value: String, focus: FocusHandle, availability: Availability, after: Box<dyn Fn()>) -> Div;
    /// The items' values in tree order, as last laid out.
    fn order(&self) -> Vec<String>;
}

impl<M: ToggleGroupModel> Group for ToggleGroup<PartProps<ToggleGroupProps<M>>, M, String> {
    fn pressed(&self, value: &String) -> bool {
        ToggleGroup::pressed(*self, value)
    }

    fn item(&self, d: Div, value: String, focus: FocusHandle, availability: Availability, after: Box<dyn Fn()>) -> Div {
        ToggleGroup::item_then(*self, d, value, focus, availability, after)
    }

    fn order(&self) -> Vec<String> {
        self.items()
            .with(|items| items.iter().map(|i| i.value.clone()).collect())
    }
}

/// What the items below a group read.
#[derive(Clone)]
struct ToggleGroupContext {
    group: Rc<dyn Group>,
    look: Rc<dyn Fn() -> Look>,
    /// The values of the items in tree order (written after layout, read by
    /// the items for their joined corners).
    order: Signal<Vec<String>>,
    /// The group's scope: what an item's leaving queues the reading for.
    owner: Option<gui::Owner>,
}

impl ToggleGroupContext {
    /// Read the laid-out items' order into [`ToggleGroupContext::order`].
    fn refresh(&self) {
        if self.order.is_alive() {
            self.order.set(self.group.order());
        }
    }

    /// [`Self::refresh`] after the next layout, as the group's work (an
    /// item leaving may ask for it).
    fn refresh_later(&self) {
        let me = self.clone();
        match self.owner {
            Some(owner) => owner.after_layout(move || me.refresh()),
            None => gui::after_layout(move || me.refresh()),
        }
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

fn root<M: ToggleGroupModel>() -> Part<ToggleGroupProps<M>> {
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
                let value: UseModel<_, M> = use_model(props, |p: &PartProps<ToggleGroupProps<M>>| &p.props.value);
                let group = ToggleGroup::new(value, use_direction(None));
                let ctx = ToggleGroupContext {
                    group: Rc::new(group),
                    look: Rc::new(move || props.with(|p| p.props.look)),
                    order: Signal::new(Vec::new()),
                    owner: gui::Owner::current(),
                };
                provide(ctx);
                move || {
                    let (look, root) = props.with(|p| (p.props.look, p.slot.0.clone()));
                    group
                        .root(root)
                        .gap(look.spacing * 4.0)
                        .when(look.spacing == 0.0 && look.variant == ToggleVariant::Outline, |e| {
                            e.shadow(Shadow {
                                x: 0.0,
                                y: 1.0,
                                blur: 2.0,
                                spread: 0.0,
                                color: Color::BLACK.opacity(0.05),
                            })
                        })
                        .into_element()
                }
            })
        },
    )
}

impl<M: ToggleGroupModel> Part<ToggleGroupProps<M>> {
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
    pub fn on_change(self, f: impl Into<gui::Callback<M>>) -> Self {
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
#[track_caller]
pub fn toggle_group_item(value: impl Into<String>) -> ToggleGroupItem {
    ToggleGroupItem {
        value: value.into(),
        disabled: false,
        children: Slot(Vec::new()),
        class: None,
        on_click: None,
        at: std::panic::Location::caller(),
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
    /// Where the caller's code made it (a report of a missing group names it).
    at: &'static std::panic::Location<'static>,
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
    pub fn on_click(mut self, f: impl Into<Callback<()>>) -> Self {
        self.on_click = Some(f.into());
        self
    }
}

impl IntoElement for ToggleGroupItem {
    fn into_element(self) -> Element {
        component(self, |cx| {
            let props = cx.props();
            let group = primitives::context::use_root_at::<ToggleGroupContext>(
                "toggle_group_item",
                "toggle_group",
                props.with_untracked(|p| p.at),
            );
            let focus = FocusHandle::new();
            // The `on_click` passed last, after each press.
            let on_click = cx.event_opt(|p| p.on_click.as_ref());
            if let Some(g) = group.clone() {
                // Its place among the items is known once it is laid out.
                g.refresh_later();
                gui::on_unmounted(move || g.refresh_later());
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
                    let pressed = g.group.pressed(&p.value);
                    let availability = Availability::from_disabled(look.disabled || p.disabled);
                    let d = toggle_variants(look.variant, look.size, pressed)
                        .named("toggle-group-item")
                        .width(Length::Auto)
                        .min_width(0.0)
                        .flex_shrink(0.0)
                        .padding_x(12.0);
                    // The press, then the item's own `@click`, both the primitive's.
                    let after = on_click.clone();
                    let d = g
                        .group
                        .item(
                            d,
                            p.value.clone(),
                            focus,
                            availability,
                            Box::new(move || {
                                after.call(());
                            }),
                        )
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
