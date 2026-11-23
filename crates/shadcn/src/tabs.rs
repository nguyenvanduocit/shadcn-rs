//! Tabs (`ui/tabs`): a row of triggers, each showing its own panel.
//!
//! The behaviour is `primitives::tabs` (reka's `Tabs`): [`tabs`] keeps
//! which tab is selected, its own from `default_value` or the caller's
//! through `.model(…)` (reka's `v-model`); the triggers are one Tab stop,
//! the arrow keys (wrapping, swapped right to left), Home and End move
//! between them, and a trigger reached so is selected unless the tabs are
//! [`manual`](Part::manual); a press selects as it goes down. This file
//! draws the parts.

use gui::Properties;
use gui::prelude::*;
use gui::{Axis, Bind, Color, FocusHandle, Model, component, div, provide, use_model};
use primitives::context::use_root;
use primitives::tabs::{Activation, Tabs, TabsOptions};
use primitives::{Availability, Wrap, use_direction};

use crate::focus::StateLook;
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`tabs`] is given.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TabsProps {
    value: Bind<String>,
    orientation: Option<Axis>,
    manual: bool,
    keep_mounted: bool,
}

impl TabsProps {
    fn options(&self) -> TabsOptions {
        TabsOptions {
            orientation: self.orientation.unwrap_or(Axis::Horizontal),
            activation: if self.manual {
                Activation::Manual
            } else {
                Activation::Automatic
            },
            wrap: Wrap::Around,
            keep_mounted: self.keep_mounted,
        }
    }
}

/// What the parts below [`tabs`] read: its primitive.
#[derive(Clone, Copy)]
struct TabsContext(Tabs<PartProps<TabsProps>, String>);

/// The tabs: their triggers and panels go inside.
#[must_use]
pub fn tabs() -> Part<TabsProps> {
    Part::new(
        TabsProps::default(),
        div()
            .named("tabs")
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(8.0),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                provide(TabsContext(Tabs::new(
                    use_model(props, |p| &p.props.value),
                    move || props.with(|p| p.props.options()),
                    use_direction(None),
                )));
                move || props.with(|p| p.slot.0.clone()).into_element()
            })
        },
    )
}

impl Part<TabsProps> {
    /// The tab selected first, when no model is given (reka's `default-value`).
    #[must_use]
    pub fn default_value(self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.with_props(|p| p.value = p.value.clone().with_default(value))
    }

    /// The selected tab, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: Model<String>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// Call `f` with each tab selected (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Into<gui::Callback<String>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    /// The axis the list's arrow keys run along (horizontal unless set).
    #[must_use]
    pub fn orientation(self, orientation: Axis) -> Self {
        self.with_props(|p| p.orientation = Some(orientation))
    }

    /// A tab focused from the keyboard is selected only by Enter or Space
    /// (reka's `activation-mode="manual"`).
    #[must_use]
    pub fn manual(self, manual: bool) -> Self {
        self.with_props(|p| p.manual = manual)
    }

    /// Inactive panels stay mounted, hidden and inert (reka's
    /// `unmount-on-hide="false"`).
    #[must_use]
    pub fn keep_mounted(self, keep: bool) -> Self {
        self.with_props(|p| p.keep_mounted = keep)
    }
}

/// The row of triggers. `w-fit` is `self_start`: in the column [`tabs`]
/// lays out, the list takes its content's width instead of stretching.
#[must_use]
pub fn tabs_list() -> Part<()> {
    let t = theme();
    let root = div()
        .named("tabs-list")
        .background(t.muted)
        .text_color(t.muted_foreground)
        .display(Display::Flex)
        .inline(true)
        .align_self(Align::Start)
        .height(36.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .radius(10.0)
        .padding(3.0);
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let tabs = use_root::<TabsContext>("tabs_list", "tabs");
            move || {
                let list = props.with(|p| p.slot.0.clone());
                match tabs {
                    Some(TabsContext(tabs)) => tabs.list(list).into_element(),
                    None => list.into_element(),
                }
            }
        })
    })
}

/// What a panel is given: the value it stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct TabProps {
    value: String,
}

/// What a trigger is given: the value it selects, and whether it can be.
#[derive(Clone, Debug, PartialEq)]
pub struct TabsTriggerProps {
    value: String,
    disabled: bool,
}

impl Part<TabsTriggerProps> {
    /// It takes no focus and no press, and the arrow keys, Home and End
    /// pass it by (reka's `disabled` on a trigger).
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }
}

/// The trigger selecting `value`; add its label with `.child`.
#[must_use]
pub fn tabs_trigger(value: impl Into<String>) -> Part<TabsTriggerProps> {
    let t = theme();
    let root = div()
        .named("tabs-trigger")
        .transition(Properties::COLOR | Properties::SHADOW)
        .text_color(if t.dark { t.muted_foreground } else { t.foreground })
        .display(Display::Flex)
        .inline(true)
        // `h-[calc(100%-1px)]` of the list's 30px inside its 3px padding.
        .height(29.0)
        .flex(Flex::grow(1.0))
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .gap(6.0)
        .radius(8.0)
        .border(1.0)
        .border_color(Color::TRANSPARENT)
        .padding_x(8.0)
        .padding_y(4.0)
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500)
        .nowrap(true)
        .with_interaction(|d, i| d.dim_disabled(i, 0.5));
    let props = TabsTriggerProps {
        value: value.into(),
        disabled: false,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let tabs = use_root::<TabsContext>("tabs_trigger", "tabs");
            let handle = FocusHandle::new();
            move || {
                let (value, disabled, root) =
                    props.with(|p| (p.props.value.clone(), p.props.disabled, p.slot.0.clone()));
                let Some(TabsContext(tabs)) = tabs else {
                    return root.into_element();
                };
                let t = theme();
                let active = tabs.is_selected(&value);
                tabs.trigger(root, value, Availability::from_disabled(disabled), handle)
                    .when(active, |d| {
                        d.background(t.background)
                            .shadow([
                                Shadow {
                                    x: 0.0,
                                    y: 1.0,
                                    blur: 3.0,
                                    spread: 0.0,
                                    color: Color::BLACK.opacity(0.1),
                                },
                                Shadow {
                                    x: 0.0,
                                    y: 1.0,
                                    blur: 2.0,
                                    spread: -1.0,
                                    color: Color::BLACK.opacity(0.1),
                                },
                            ])
                            .when(t.dark, |d| {
                                d.text_color(t.foreground)
                                    .border_color(t.input)
                                    .background(t.input.opacity(0.3))
                            })
                    })
                    // Over the border the active tab has (`dark:data-[state=active]:border-input`).
                    .with_interaction(|d, i| d.focus_ring_on(i, &t))
                    .into_element()
            }
        })
    })
}

/// The panel of `value`, shown while its tab is selected (reka unmounts it
/// otherwise); fill it with `.child`.
#[must_use]
pub fn tabs_content(value: impl Into<String>) -> Part<TabProps> {
    let root = div().named("tabs-content").flex(Flex::grow(1.0));
    Part::new(TabProps { value: value.into() }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let tabs = use_root::<TabsContext>("tabs_content", "tabs");
            move || {
                let (value, panel) = props.with(|p| (p.props.value.clone(), p.slot.0.clone()));
                match tabs {
                    Some(TabsContext(tabs)) => tabs.panel(panel, &value),
                    None => panel.into_element(),
                }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gui::{Modifiers, NamedKey, Scenario, Signal};

    #[test]
    fn a_disabled_trigger_is_passed_by_the_keys_and_takes_no_press() {
        let value = Signal::new("a".to_owned());
        let mut s = Scenario::measuring(
            move || {
                tabs()
                    .model(value.into())
                    .child(
                        tabs_list()
                            .child(tabs_trigger("a").child("A"))
                            .child(tabs_trigger("b").disabled(true).child("B"))
                            .child(tabs_trigger("c").child("C")),
                    )
                    .into_element()
            },
            400,
            300,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("settled");
        s.press(NamedKey::Tab, Modifiers::default());
        s.settle().expect("settled");
        s.press(NamedKey::ArrowRight, Modifiers::default());
        s.settle().expect("settled");
        assert_eq!(value.get_untracked(), "c", "ArrowRight passes the disabled B");
        s.press(NamedKey::ArrowLeft, Modifiers::default());
        s.settle().expect("settled");
        assert_eq!(value.get_untracked(), "a");
        assert!(s.click_text("B").is_err(), "B takes no press");
        assert_eq!(value.get_untracked(), "a");
    }
}
