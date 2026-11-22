//! Radio group (`ui/radio-group`): a set of choices, one checked at a time.
//!
//! [`radio_group`] keeps the checked value: its own, from `default_value`,
//! or the caller's through `.model(…)` (reka's `v-model`); `on_change`
//! hears each check. Its items read it. Arrow keys move to the next or
//! previous item and check it, wrapping, as WAI-ARIA radio groups do.

use gui::Properties;
use gui::prelude::*;
use gui::{
    Bind, Color, FocusHandle, Key, Members, Model, NamedKey, UseModel, component, div, provide, use_context, use_model,
};

use crate::focus::StateLook;
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`radio_group`] is given.
#[derive(Clone, PartialEq, Default)]
pub struct RadioGroupProps {
    value: Bind<Option<String>>,
    disabled: bool,
}

/// What the items below a [`radio_group`] read.
#[derive(Clone)]
struct RadioGroupContext {
    value: UseModel<PartProps<RadioGroupProps>, Option<String>>,
    props: gui::Signal<PartProps<RadioGroupProps>>,
    /// The items in tree order, as last laid out, each with its focus.
    items: Members<(String, FocusHandle)>,
}

impl RadioGroupContext {
    fn disabled(&self) -> bool {
        self.props.with(|p| p.props.disabled)
    }

    /// Focus and check the item `by` places from the focused one, wrapping.
    fn step(&self, by: isize) -> bool {
        let target = self.items.with(|items| {
            let len = items.len();
            let at = items.iter().position(|(_, f)| f.is_focused())?;
            let next = isize::try_from(at)
                .ok()
                .and_then(|a| a.checked_add(by))
                .and_then(|n| n.checked_rem_euclid(isize::try_from(len).ok()?))
                .and_then(|n| usize::try_from(n).ok())?;
            items.get(next).cloned()
        });
        target.is_some_and(|(v, f)| {
            gui::focus::set_focus(Some(f.id()), true);
            self.value.set(Some(v));
            true
        })
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
                let ctx = RadioGroupContext {
                    value: use_model(props, |p| &p.props.value),
                    props,
                    items: Members::new(),
                };
                provide(ctx.clone());
                move || {
                    let keys = ctx.clone();
                    props
                        .with(|p| p.slot.0.clone())
                        .role(gui::Role::RadioGroup)
                        .on_key_down(move |ev| match &ev.key {
                            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowRight) => keys.step(1),
                            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowLeft) => keys.step(-1),
                            _ => false,
                        })
                        .into_element()
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
    pub fn on_change(self, f: impl Fn(Option<String>) + 'static) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    /// Every item disabled.
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
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
        .role(gui::Role::RadioButton)
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
        .cursor_disabled(i)
        // WAI-ARIA: Enter does not check a radio, only Space does.
        .on_key_down(|ev| ev.is(NamedKey::Enter));
    Part::new(RadioGroupItemProps { value: value.into() }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let group = use_context::<RadioGroupContext>();
            let focus = FocusHandle::new();
            move || {
                let (value, root) = props.with(|p| (p.props.value.clone(), p.slot.0.clone()));
                let Some(g) = group.clone() else {
                    return root.into_element();
                };
                let t = theme();
                let checked = g.value.with(|v| v.as_deref() == Some(value.as_str()));
                let pick = g.value;
                root.aria_checked(checked)
                    .track_focus(focus)
                    .member(&g.items, (value.clone(), focus))
                    .disabled(g.disabled())
                    .on_click(move |_| pick.set(Some(value.clone())))
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
