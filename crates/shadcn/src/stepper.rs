//! Stepper (`ui/stepper`): numbered steps of a flow, each completed, active
//! or still ahead.
//!
//! [`stepper`] keeps the current step: its own, from `default_value`, or the
//! caller's through `.model(…)` (reka's `v-model`). Each [`stepper_item`]
//! knows where its step stands and gives that to what fills it
//! ([`StepperItem`], reka's `v-slot="{ state }"`), since reka styles a part
//! by the state of the item it sits in (`group-data-[state=…]`).

use std::rc::Rc;

use gui::prelude::*;
use gui::{Bind, Div, Model, Signal, UseModel, component, div, provide, text, use_context, use_model};

use crate::part::{Part, PartProps};
use crate::separator::Orientation;
use crate::theme::theme;

/// Where a step stands against the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Completed,
    Active,
    Inactive,
}

/// What [`stepper`] is given (reka's `StepperRoot`); steps are 1-based.
#[derive(Clone, Debug, PartialEq)]
pub struct StepperProps {
    step: Bind<usize>,
    orientation: Orientation,
    linear: bool,
}

impl Default for StepperProps {
    fn default() -> Self {
        Self {
            step: Bind::default_value(1),
            orientation: Orientation::Horizontal,
            linear: true,
        }
    }
}

/// What the parts below [`stepper`] read.
#[derive(Clone)]
struct StepperContext {
    props: Signal<PartProps<StepperProps>>,
    step: UseModel<PartProps<StepperProps>, usize>,
}

impl StepperContext {
    fn state_of(&self, step: usize) -> StepState {
        match step.cmp(&self.step.get()) {
            std::cmp::Ordering::Less => StepState::Completed,
            std::cmp::Ordering::Equal => StepState::Active,
            std::cmp::Ordering::Greater => StepState::Inactive,
        }
    }

    /// Whether `step` can be moved to now (reka's `isFocusable`).
    fn reachable(&self, step: usize) -> bool {
        !self.props.with(|p| p.props.linear) || step <= self.step.get().saturating_add(1)
    }

    /// Move to `step` when it can be reached.
    fn go(&self, step: usize) {
        if self.reachable(step) {
            self.step.set(step);
        }
    }
}

/// The row (or column) of steps.
#[must_use]
pub fn stepper() -> Part<StepperProps> {
    Part::new(StepperProps::default(), div().named("stepper").flex().gap(2.0), |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(StepperContext {
                props,
                step: use_model(props, |p| &p.props.step),
            });
            move || {
                props
                    .with(|p| {
                        p.slot
                            .0
                            .clone()
                            .when(p.props.orientation == Orientation::Vertical, Styled::flex_col)
                    })
                    .into_element()
            }
        })
    })
}

impl Part<StepperProps> {
    /// The step shown first, when no model is given (reka's `default-value`).
    #[must_use]
    pub fn default_value(self, step: usize) -> Self {
        self.with_props(|p| p.step = std::mem::take(&mut p.step).with_default(step))
    }

    /// The current step, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, step: impl Into<Model<usize>>) -> Self {
        self.with_props(|p| p.step = std::mem::take(&mut p.step).with_model(step))
    }

    /// The current step as the caller shows it (`:model-value`); a move
    /// only calls `on_change`.
    #[must_use]
    pub fn value(self, step: usize) -> Self {
        self.with_props(|p| p.step = std::mem::take(&mut p.step).with_value(step))
    }

    /// Called with each step moved to (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(usize) + 'static) -> Self {
        self.with_props(|p| p.step = std::mem::take(&mut p.step).on_change(f))
    }

    /// Steps down a column instead of across a row.
    #[must_use]
    pub fn orientation(self, orientation: Orientation) -> Self {
        self.with_props(|p| p.orientation = orientation)
    }

    /// Whether only the steps done and the next one can be reached (the default).
    #[must_use]
    pub fn linear(self, linear: bool) -> Self {
        self.with_props(|p| p.linear = linear)
    }
}

/// A step as what fills its item sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepperItem {
    pub step: usize,
    pub state: StepState,
    /// Whether it can be moved to now.
    pub reachable: bool,
}

type Fill = Rc<dyn Fn(StepperItem, Div) -> Div>;

/// What a [`stepper_item`] is given: its step, and what fills it.
#[derive(Clone)]
pub struct StepperItemProps {
    step: usize,
    fill: Option<Fill>,
}

/// What the parts in an item read.
#[derive(Clone, Copy)]
struct ItemContext {
    props: Signal<PartProps<StepperItemProps>>,
}

/// Step `step`: holds its trigger, separator, title and description. What
/// depends on its state goes in through [`fill`](Part::fill).
#[must_use]
pub fn stepper_item(step: usize) -> Part<StepperItemProps> {
    let root = div().named("stepper-item").flex().items_center().gap(2.0);
    Part::new(StepperItemProps { step, fill: None }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let stepper = use_context::<StepperContext>();
            provide(ItemContext { props });
            move || {
                let (step, fill, root) = props.with(|p| (p.props.step, p.props.fill.clone(), p.slot.0.clone()));
                let item = stepper.as_ref().map_or(
                    StepperItem {
                        step,
                        state: StepState::Inactive,
                        reachable: true,
                    },
                    |s| StepperItem {
                        step,
                        state: s.state_of(step),
                        reachable: s.reachable(step),
                    },
                );
                match fill {
                    Some(fill) => fill(item, root).into_element(),
                    None => root.into_element(),
                }
            }
        })
    })
}

impl Part<StepperItemProps> {
    /// Fill the item knowing where its step stands (reka's `v-slot="{ state }"`):
    /// `f` gets the step and the item's box, and returns the box filled.
    #[must_use]
    pub fn fill(self, f: impl Fn(StepperItem, Div) -> Div + 'static) -> Self {
        let f: Fill = Rc::new(f);
        self.with_props(|p| p.fill = Some(f))
    }
}

/// What moves to its item's step: a click, or Enter or Space while focused.
#[must_use]
pub fn stepper_trigger() -> Part<()> {
    stepper_trigger_as(
        div()
            .named("stepper-trigger")
            .p(1.0)
            .flex()
            .flex_col()
            .items_center()
            .text_center()
            .gap(1.0)
            .rounded_md(),
    )
}

/// `child` made the trigger of its item's step (`as-child`), as a button is in the demos.
#[must_use]
pub fn stepper_trigger_as(child: Div) -> Part<()> {
    Part::new((), child, |p| {
        component(p, |cx| {
            let props = cx.props();
            let stepper = use_context::<StepperContext>();
            let item = use_context::<ItemContext>();
            move || {
                let root = props.with(|p| p.slot.0.clone()).focusable();
                let (Some(stepper), Some(item)) = (stepper.clone(), item) else {
                    return root.into_element();
                };
                let step = item.props.with(|p| p.props.step);
                root.when(!stepper.reachable(step), |d| d.disabled(true))
                    .on_click(move |_| stepper.go(step))
                    .into_element()
            }
        })
    })
}

/// The round badge of `item`'s step, filled by its state; put its number or icon in it.
#[must_use]
pub fn stepper_indicator(item: StepperItem) -> Div {
    let t = theme();
    let d = div()
        .named("stepper-indicator")
        .inline_flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .text_color(t.muted_foreground.opacity(0.5))
        .w(8.0)
        .h(8.0);
    let d = match item.state {
        StepState::Active => d.bg(t.primary).text_color(t.primary_foreground),
        StepState::Completed => d.bg(t.accent).text_color(t.accent_foreground),
        StepState::Inactive => d,
    };
    d.when(!item.reachable, |d| d.text_color(t.muted_foreground).opacity(0.5))
}

/// The line from `item`'s step to the next.
#[must_use]
pub fn stepper_separator(item: StepperItem) -> Div {
    let t = theme();
    div()
        .named("stepper-separator")
        .bg(if item.state == StepState::Completed {
            t.accent
        } else {
            t.muted
        })
        .when(!item.reachable, |d| d.opacity(0.5))
}

#[must_use]
pub fn stepper_title(title: impl Into<String>) -> gui::Text {
    // `text-md` is no Tailwind size, so the inherited size stands.
    text(title).named("stepper-title").font_semibold().whitespace_nowrap()
}

#[must_use]
pub fn stepper_description(description: impl Into<String>) -> gui::Text {
    let t = theme();
    text(description)
        .named("stepper-description")
        .text_xs()
        .text_color(t.muted_foreground)
}
