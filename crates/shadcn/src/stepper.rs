//! Stepper (`ui/stepper`): numbered steps of a flow, each completed, active
//! or still ahead.
//!
//! [`stepper`] keeps the current step: its own, from `default_value`, or the
//! caller's through `.model(…)` (reka's `v-model`). Each [`stepper_item`]
//! knows where its step stands and gives that to what fills it
//! ([`StepperItem`], reka's `v-slot="{ state }"`), since reka styles a part
//! by the state of the item it sits in (`group-data-[state=…]`). Its
//! behaviour (reach, the keys, a press choosing at once, the status line)
//! is `primitives::stepper`'s; this module draws it.

use std::rc::Rc;

use gui::prelude::*;
use gui::{Axis, Bind, Div, FocusHandle, Model, Signal, component, div, provide, text, use_model, watch};
use primitives::context::use_root;
use primitives::separator::Separator;
pub use primitives::stepper::StepState;
use primitives::stepper::{StepLabels, Stepper};
use primitives::visually_hidden::{Hidden, visually_hidden};
use primitives::{Availability, use_direction};

use crate::part::{Part, PartProps};
use crate::separator::Orientation;
use crate::theme::theme;

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
#[derive(Clone, Copy)]
struct StepperContext {
    stepper: Stepper<PartProps<StepperProps>>,
}

impl StepperProps {
    fn layout(&self) -> (bool, Axis) {
        let axis = match self.orientation {
            Orientation::Horizontal => Axis::Horizontal,
            Orientation::Vertical => Axis::Vertical,
        };
        (self.linear, axis)
    }
}

/// The row (or column) of steps.
#[must_use]
pub fn stepper() -> Part<StepperProps> {
    Part::new(
        StepperProps::default(),
        div().named("stepper").display(Display::Flex).gap(8.0),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let stepper = Stepper::new(
                    use_model(props, |p: &PartProps<StepperProps>| &p.props.step),
                    use_direction(None),
                );
                let (linear, axis) = props.with_untracked(|p| p.props.layout());
                stepper.set_layout(linear, axis);
                watch(
                    move || props.with(|p| p.props.layout()),
                    move |(linear, axis), _| stepper.set_layout(*linear, *axis),
                );
                provide(StepperContext { stepper });
                move || {
                    let root = props.with(|p| {
                        p.slot
                            .0
                            .clone()
                            .when(p.props.orientation == Orientation::Vertical, |e| {
                                e.display(Display::Flex).flex_direction(FlexDirection::Column)
                            })
                    });
                    // The status line a screen reader hears as the step moves; not on screen.
                    let status = visually_hidden(stepper.status(div()), Hidden::Visually);
                    stepper.root(root).child(status).into_element()
                }
            })
        },
    )
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
    pub fn on_change(self, f: impl Into<gui::Callback<usize>>) -> Self {
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
    /// Its title's and description's nodes, which name its trigger.
    pub labels: StepLabels,
}

type Fill = Rc<dyn Fn(StepperItem, Div) -> Div>;

/// What a [`stepper_item`] is given: its step, whether it is done or
/// disabled of its own, and what fills it.
#[derive(Clone)]
pub struct StepperItemProps {
    step: usize,
    completed: bool,
    disabled: bool,
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
    let root = div()
        .named("stepper-item")
        .display(Display::Flex)
        .align_items(Align::Center)
        .gap(8.0);
    let props = StepperItemProps {
        step,
        completed: false,
        disabled: false,
        fill: None,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let stepper = use_root::<StepperContext>("stepper_item", "stepper");
            provide(ItemContext { props });
            move || {
                let (item, fill, root) = props.with(|p| (p.props.clone(), p.props.fill.clone(), p.slot.0.clone()));
                let step = item.step;
                let (item, root) = match stepper {
                    None => (
                        StepperItem {
                            step,
                            state: StepState::Inactive,
                            reachable: true,
                            labels: StepLabels::new(),
                        },
                        root,
                    ),
                    Some(s) => (
                        StepperItem {
                            step,
                            state: s.stepper.state(step, item.completed),
                            reachable: s.stepper.reachable(step, Availability::from_disabled(item.disabled)),
                            labels: s.stepper.labels(step),
                        },
                        s.stepper.item(root, step),
                    ),
                };
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

    /// Done whatever the current step (reka's `completed`).
    #[must_use]
    pub fn completed(self, completed: bool) -> Self {
        self.with_props(|p| p.completed = completed)
    }

    /// Never reached, by a press, a key or the caller (reka's `disabled`).
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }
}

/// What moves to its item's step: a click, or Enter or Space while focused.
#[must_use]
pub fn stepper_trigger() -> Part<()> {
    stepper_trigger_as(
        div()
            .named("stepper-trigger")
            .padding(4.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .align_items(Align::Center)
            .text_align(TextAlign::Center)
            .gap(4.0)
            .radius(8.0),
    )
}

/// `child` made the trigger of its item's step (`as-child`), as a button is in the demos.
#[must_use]
pub fn stepper_trigger_as(child: impl Into<Div>) -> Part<()> {
    Part::new((), child.into(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let stepper = use_root::<StepperContext>("stepper_trigger", "stepper");
            let item = use_root::<ItemContext>("stepper_trigger", "stepper_item");
            let focus = FocusHandle::new();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                let (Some(s), Some(item)) = (stepper, item) else {
                    return root.focusable().into_element();
                };
                let (step, disabled) = item.props.with(|p| (p.props.step, p.props.disabled));
                s.stepper
                    .trigger(root, step, focus, Availability::from_disabled(disabled))
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
        .display(Display::Flex)
        .inline(true)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .radius(9999.0)
        .text_color(t.muted_foreground.opacity(0.5))
        .width(32.0)
        .height(32.0);
    let d = match item.state {
        StepState::Active => d.background(t.primary).text_color(t.primary_foreground),
        StepState::Completed => d.background(t.accent).text_color(t.accent_foreground),
        StepState::Inactive => d,
    };
    d.when(!item.reachable, |d| d.text_color(t.muted_foreground).opacity(0.5))
}

/// The line from `item`'s step to the next: decorative, nothing to a
/// screen reader (reka's `StepperSeparator`).
#[must_use]
pub fn stepper_separator(item: StepperItem) -> Div {
    let t = theme();
    Separator::DEFAULT
        .apply(div())
        .named("stepper-separator")
        .background(if item.state == StepState::Completed {
            t.accent
        } else {
            t.muted
        })
        .when(!item.reachable, |d| d.opacity(0.5))
}

/// `item`'s title, which names its trigger.
#[must_use]
pub fn stepper_title(item: StepperItem, title: impl Into<String>) -> gui::Text {
    // `text-md` is no Tailwind size, so the inherited size stands.
    item.labels
        .title(text(title))
        .named("stepper-title")
        .font_weight(600)
        .nowrap(true)
}

/// `item`'s description, which names its trigger after the title.
#[must_use]
pub fn stepper_description(item: StepperItem, description: impl Into<String>) -> gui::Text {
    let t = theme();
    item.labels
        .description(text(description))
        .named("stepper-description")
        .text_size(12.0)
        .line_height(16.0)
        .text_color(t.muted_foreground)
}
