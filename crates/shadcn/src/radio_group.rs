//! Radio group (`ui/radio-group`): a set of choices, one checked at a time.
//!
//! [`radio_group`] keeps the checked value: its own, from `default_value`,
//! or the caller's through `.model(…)` (reka's `v-model`); `on_change`
//! hears each check. Its items read it. The group is one Tab stop, and the
//! arrow keys move to the next or previous item and check it, wrapping;
//! Home and End only move focus, as reka's (`primitives::radio_group`).

use gui::Properties;
use gui::prelude::*;
use gui::{Bind, Color, FocusHandle, Model, component, div, provide, use_model};
use primitives::radio_group::RadioGroup;
use primitives::{Availability, use_direction};

use crate::focus::StateLook;
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`radio_group`] is given.
#[derive(Clone, PartialEq, Default)]
pub struct RadioGroupProps {
    value: Bind<Option<String>>,
    disabled: bool,
    required: bool,
}

/// What the items below a [`radio_group`] read.
#[derive(Clone, Copy)]
struct RadioGroupContext {
    group: RadioGroup<PartProps<RadioGroupProps>, String>,
    props: gui::Signal<PartProps<RadioGroupProps>>,
}

impl RadioGroupContext {
    fn disabled(&self) -> bool {
        self.props.with(|p| p.props.disabled)
    }
}

/// The group's grid; its items go in with `.child`, each beside its label.
/// Nothing is checked, and the group keeps its own value, until told otherwise.
#[must_use]
pub fn radio_group() -> Part<RadioGroupProps> {
    Part::new(
        RadioGroupProps::default(),
        div().named("radio-group").display(Display::Grid).gap(12.0),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let group = RadioGroup::new(
                    use_model(props, |p: &PartProps<RadioGroupProps>| &p.props.value),
                    use_direction(None),
                );
                provide(RadioGroupContext { group, props });
                move || {
                    let (slot, required) = props.with(|p| (p.slot.0.clone(), p.props.required));
                    group.root(slot, None, required).into_element()
                }
            })
        },
    )
}

impl Part<RadioGroupProps> {
    /// The checked value, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<Option<String>>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// The value checked first, when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.with_props(|p| p.value = p.value.clone().with_default(Some(value)))
    }

    /// Call `f` with the value each check makes (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Into<gui::Callback<Option<String>>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    /// Every item disabled.
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }

    /// A form needs one checked (`required`): told to assistive
    /// technology; the check itself is the form field's rule.
    #[must_use]
    pub fn required(self, required: bool) -> Self {
        self.with_props(|p| p.required = required)
    }
}

/// What a [`radio_group_item`] is given: the value it checks.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioGroupItemProps {
    value: String,
}

/// The radio of the item `value`; a click, or Space while focused, checks it.
#[must_use]
pub fn radio_group_item(value: impl Into<String>) -> Part<RadioGroupItemProps> {
    let t = theme();
    let i = Interaction::new();
    let root = div()
        .named("radio-group-item")
        .interaction(i)
        .transition(Properties::COLOR | Properties::SHADOW)
        .position(Position::Relative)
        .display(Display::Flex)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .aspect_ratio(1.0)
        .size(16.0)
        .flex_shrink(0.0)
        .radius(9999.0)
        .border(1.0)
        .border_color(t.input)
        .text_color(t.primary)
        .background(if t.dark {
            t.input.opacity(0.3)
        } else {
            Color::TRANSPARENT
        })
        .shadow(Shadow {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: 0.0,
            color: Color::BLACK.opacity(0.05),
        })
        .focus_ring_on(i, &t)
        .dim_disabled(i, 0.5)
        .cursor_disabled(i);
    Part::new(RadioGroupItemProps { value: value.into() }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let group = primitives::context::use_root::<RadioGroupContext>("radio_group_item", "radio_group");
            let focus = FocusHandle::new();
            move || {
                let (value, root) = props.with(|p| (p.props.value.clone(), p.slot.0.clone()));
                let Some(g) = group else {
                    return root.into_element();
                };
                let t = theme();
                let checked = g.group.checked(&value);
                let availability = Availability::from_disabled(g.disabled());
                g.group
                    .item(root, value, focus, availability)
                    .when(checked, |d| {
                        // lucide's circle (r=10 in 24) filled and stroked at `size-2`: a 22/3px disc.
                        d.child(
                            div()
                                .named("radio-group-indicator")
                                .pass_through()
                                .size(22.0 / 3.0)
                                .radius(9999.0)
                                .background(t.primary),
                        )
                    })
                    .into_element()
            }
        })
    })
}

impl crate::form::FormControl for Part<RadioGroupItemProps> {
    fn invalid(self, invalid: bool) -> Self {
        self.map_root(|d| crate::form::FormControl::invalid(d, invalid))
    }
}
