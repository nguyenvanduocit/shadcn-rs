//! Collapsible (`ui/collapsible`): content shown and hidden by a trigger.
//!
//! [`collapsible`] keeps whether it is open: its own, starting at
//! `default_open`, the caller's through `.model(…)` (reka's
//! `v-model:open`), or a value the caller keeps (`.open(…)` with
//! `.on_change(…)`). Its trigger and content read it.

use gui::prelude::*;
use gui::{Bind, Div, Element, Model, Signal, UseModel, component, div, provide, use_context, use_model};

use crate::part::{Part, PartProps};

/// What [`collapsible`] is given.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CollapsibleProps {
    open: Bind<bool>,
    disabled: bool,
}

/// What the parts below a [`collapsible`] read.
#[derive(Clone)]
pub(crate) struct CollapsibleContext {
    pub(crate) open: UseModel<PartProps<CollapsibleProps>, bool>,
    props: Signal<PartProps<CollapsibleProps>>,
}

impl CollapsibleContext {
    /// Whether it is open (tracked).
    pub(crate) fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Open it when closed, close it when open, unless disabled.
    pub(crate) fn toggle(&self) {
        if !self.props.with_untracked(|p| p.props.disabled) {
            self.open.update(|o| *o = !*o);
        }
    }

    /// The context of the collapsible this instance is in.
    pub(crate) fn find() -> Option<Self> {
        use_context::<Self>()
    }
}

/// The collapsible: its trigger and content go inside.
#[must_use]
pub fn collapsible() -> Part<CollapsibleProps> {
    Part::new(CollapsibleProps::default(), div().named("collapsible"), |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(CollapsibleContext {
                open: use_model(props, |p| &p.props.open),
                props,
            });
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

impl Part<CollapsibleProps> {
    /// Open at first, when no model is given (reka's `default-open`).
    #[must_use]
    pub fn default_open(self, open: bool) -> Self {
        self.with_props(|p| p.open = std::mem::take(&mut p.open).with_default(open))
    }

    /// Whether it is open, kept by the caller (`v-model:open`).
    #[must_use]
    pub fn model(self, open: impl Into<Model<bool>>) -> Self {
        self.with_props(|p| p.open = std::mem::take(&mut p.open).with_model(open))
    }

    /// Whether it is open, as the caller shows it (`:open`); a toggle only
    /// calls `on_change`.
    #[must_use]
    pub fn open(self, open: bool) -> Self {
        self.with_props(|p| p.open = std::mem::take(&mut p.open).with_value(open))
    }

    /// Called with each new open state (`@update:open`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(bool) + 'static) -> Self {
        self.with_props(|p| p.open = std::mem::take(&mut p.open).on_change(f))
    }

    /// The trigger does nothing.
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }
}

/// An unstyled pressable trigger that opens and closes its collapsible.
#[must_use]
pub fn collapsible_trigger() -> Part<()> {
    collapsible_trigger_as(div().named("collapsible-trigger").focusable())
}

/// `control` (a button, a row) as the trigger (reka's `as-child`): pressing
/// it opens and closes its collapsible.
#[must_use]
pub fn collapsible_trigger_as(control: Div) -> Part<()> {
    Part::new((), control, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = CollapsibleContext::find();
            move || {
                let control = props.with(|p| p.slot.0.clone());
                match ctx.clone() {
                    Some(ctx) => control
                        .aria_expanded(ctx.is_open())
                        .on_click(move |_| ctx.toggle())
                        .into_element(),
                    None => control.into_element(),
                }
            }
        })
    })
}

/// The content, shown while open (reka unmounts it otherwise); style and
/// fill it as a `Div`.
#[must_use]
pub fn collapsible_content() -> Part<()> {
    Part::new((), div().named("collapsible-content"), |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = CollapsibleContext::find();
            move || {
                if ctx.as_ref().is_none_or(CollapsibleContext::is_open) {
                    props.with(|p| p.slot.0.clone()).into_element()
                } else {
                    Element::Empty
                }
            }
        })
    })
}
