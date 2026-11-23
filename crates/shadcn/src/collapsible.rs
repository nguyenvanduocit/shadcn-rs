//! Collapsible (`ui/collapsible`): content shown and hidden by a trigger.
//!
//! The behaviour is `primitives::collapsible` (reka's `Collapsible`):
//! [`collapsible`] keeps whether it is open, its own from `default_open`,
//! the caller's through `.model(…)` (reka's `v-model:open`), or a value the
//! caller keeps (`.open(…)` with `.on_change(…)`); its trigger toggles it
//! unless disabled, and its content is gone while closed unless kept
//! mounted. This file draws nothing of its own: the trigger and content are
//! the caller's boxes.

use gui::prelude::*;
use gui::{Bind, Div, Model, component, div, provide, use_context, use_model};
use primitives::Availability;
use primitives::collapsible::Collapsible;

use crate::part::{Part, PartProps};

/// What [`collapsible`] is given.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CollapsibleProps {
    open: Bind<bool>,
    disabled: bool,
    keep_mounted: bool,
}

/// What the parts below a [`collapsible`] read: its primitive.
#[derive(Clone, Copy)]
pub(crate) struct CollapsibleContext(pub(crate) Collapsible<PartProps<CollapsibleProps>>);

impl CollapsibleContext {
    /// Whether it is open (tracked).
    pub(crate) fn is_open(&self) -> bool {
        self.0.is_open()
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
            provide(CollapsibleContext(Collapsible::new(
                use_model(props, |p| &p.props.open),
                move || Availability::from_disabled(props.with(|p| p.props.disabled)),
                move || props.with(|p| p.props.keep_mounted),
            )));
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
    pub fn on_change(self, f: impl Into<gui::Callback<bool>>) -> Self {
        self.with_props(|p| p.open = std::mem::take(&mut p.open).on_change(f))
    }

    /// The trigger takes no focus and no press; the caller can still open
    /// and close it.
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }

    /// The content stays mounted while closed, hidden and inert (reka's
    /// `unmount-on-hide="false"`).
    #[must_use]
    pub fn keep_mounted(self, keep: bool) -> Self {
        self.with_props(|p| p.keep_mounted = keep)
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
pub fn collapsible_trigger_as(control: impl Into<Div>) -> Part<()> {
    Part::new((), control.into(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = primitives::context::use_root::<CollapsibleContext>("collapsible_trigger", "collapsible");
            move || {
                let control = props.with(|p| p.slot.0.clone());
                match ctx {
                    Some(ctx) => ctx.0.trigger(control).into_element(),
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
            let ctx = primitives::context::use_root::<CollapsibleContext>("collapsible_content", "collapsible");
            move || {
                let content = props.with(|p| p.slot.0.clone());
                match ctx {
                    Some(ctx) => ctx.0.content(content),
                    None => content.into_element(),
                }
            }
        })
    })
}
