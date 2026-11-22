//! Tabs (`ui/tabs`): a row of triggers, each showing its own panel.
//!
//! [`tabs`] keeps which tab is selected: its own, starting at
//! `default_value`, or the caller's through `.model(…)` (reka's `v-model`).
//! Triggers and panels below it read it.
//!
//! Keyboard as reka's: the arrow keys (left and right, wrapping), Home and End
//! move between triggers, and a trigger reached that way is selected
//! (`activationMode: 'automatic'`).

use gui::Properties;
use gui::prelude::*;
use gui::{
    Bind, Color, Div, FocusHandle, KeyEvent, Members, Model, NamedKey, UseModel, component, div, provide, use_context,
    use_model,
};

use crate::focus::focus_ring;
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`tabs`] is given.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TabsProps {
    value: Bind<String>,
}

/// What the parts below [`tabs`] read.
#[derive(Clone)]
struct TabsContext {
    value: UseModel<PartProps<TabsProps>, String>,
    /// The triggers in tree order, as last laid out.
    triggers: Members<(String, FocusHandle)>,
}

impl TabsContext {
    /// Move focus from the trigger of `from` as `key` asks, selecting where it
    /// lands; whether the key was one of the tab keys.
    fn navigate(&self, from: &str, key: &KeyEvent) -> bool {
        let m = key.modifiers;
        if m.shift || m.control || m.alt || m.command {
            return false;
        }
        let target = self.triggers.with(|triggers| {
            let current = triggers.iter().position(|(v, _)| v == from);
            roving_target(triggers.len(), current, key).map(|i| triggers.get(i).cloned())
        });
        let Some(target) = target else {
            return false;
        };
        if let Some((value, handle)) = target {
            gui::focus::set_focus(Some(handle.id()), true);
            self.value.set(value);
        }
        true
    }
}

/// Where a horizontal, wrapping roving focus goes from `current` among `len`
/// items on `key` (reka's `getFocusIntent` and `wrapArray`).
fn roving_target(len: usize, current: Option<usize>, key: &KeyEvent) -> Option<usize> {
    let last = len.checked_sub(1)?;
    let i = current?;
    if key.is(NamedKey::ArrowRight) {
        Some(if i >= last { 0 } else { i.saturating_add(1) })
    } else if key.is(NamedKey::ArrowLeft) {
        Some(i.checked_sub(1).unwrap_or(last))
    } else if key.is(NamedKey::Home) || key.is(NamedKey::PageUp) {
        Some(0)
    } else if key.is(NamedKey::End) || key.is(NamedKey::PageDown) {
        Some(last)
    } else {
        None
    }
}

/// The tabs: their triggers and panels go inside.
#[must_use]
pub fn tabs() -> Part<TabsProps> {
    Part::new(TabsProps::default(), div().named("tabs").flex_col().gap(2.0), |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(TabsContext {
                value: use_model(props, |p| &p.props.value),
                triggers: Members::new(),
            });
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
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
    pub fn on_change(self, f: impl Fn(String) + 'static) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }
}

/// The row of triggers. `w-fit` is `self_start`: in the column [`tabs`]
/// lays out, the list takes its content's width instead of stretching.
#[must_use]
pub fn tabs_list() -> Div {
    let t = theme();
    div()
        .named("tabs-list")
        .role(gui::Role::TabList)
        .bg(t.muted)
        .text_color(t.muted_foreground)
        .inline_flex()
        .self_start()
        .h(9.0)
        .items_center()
        .justify_center()
        .rounded_lg()
        .p(0.75)
}

/// What a trigger or a panel is given: the value it stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct TabProps {
    value: String,
}

/// The trigger selecting `value`; add its label with `.child`.
#[must_use]
pub fn tabs_trigger(value: impl Into<String>) -> Part<TabProps> {
    let t = theme();
    let root = div()
        .named("tabs-trigger")
        .role(gui::Role::Tab)
        .transition_props(Properties::COLOR | Properties::SHADOW)
        .text_color(if t.dark { t.muted_foreground } else { t.foreground })
        .inline_flex()
        // `h-[calc(100%-1px)]` of the list's 30px inside its 3px padding.
        .h_px(29.0)
        .flex_1()
        .items_center()
        .justify_center()
        .gap(1.5)
        .rounded_md()
        .border()
        .border_color(Color::TRANSPARENT)
        .px(2.0)
        .py(1.0)
        .text_sm()
        .font_medium()
        .whitespace_nowrap()
        .focus_visible(move |s| focus_ring(s, &t))
        .disabled_style(|s| s.opacity(0.5));
    Part::new(TabProps { value: value.into() }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let tabs = use_context::<TabsContext>();
            let handle = FocusHandle::new();
            move || {
                let (value, root) = props.with(|p| (p.props.value.clone(), p.slot.0.clone()));
                let Some(tabs) = tabs.clone() else {
                    return root.into_element();
                };
                let t = theme();
                let active = tabs.value.with(|v| *v == value);
                let (on_click, on_key) = (tabs.clone(), tabs.clone());
                let (click_value, key_value) = (value.clone(), value.clone());
                root.aria_selected(active)
                    .track_focus(handle)
                    .member(&tabs.triggers, (value, handle))
                    .when(active, |d| {
                        d.bg(t.background).shadow_sm().when(t.dark, |d| {
                            d.text_color(t.foreground)
                                .border_color(t.input)
                                .bg(t.input.opacity(0.3))
                        })
                    })
                    .on_click(move |_| on_click.value.set(click_value.clone()))
                    .on_key_down(move |key| on_key.navigate(&key_value, key))
                    .into_element()
            }
        })
    })
}

/// The panel of `value`, shown while its tab is selected (reka unmounts it
/// otherwise); fill it with `.child`.
#[must_use]
pub fn tabs_content(value: impl Into<String>) -> Part<TabProps> {
    let root = div().named("tabs-content").role(gui::Role::TabPanel).flex_1();
    Part::new(TabProps { value: value.into() }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let tabs = use_context::<TabsContext>();
            move || {
                props.with(|p| {
                    let shown = tabs
                        .as_ref()
                        .is_none_or(|tabs| tabs.value.with(|v| *v == p.props.value));
                    if shown {
                        p.slot.0.clone().into_element()
                    } else {
                        gui::Element::Empty
                    }
                })
            }
        })
    })
}
