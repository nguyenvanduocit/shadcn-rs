//! Stepper (`ui/stepper`): numbered steps of a flow, each completed, active
//! or still ahead.
//!
//! Every part takes the stepper's state and its step, since reka styles a
//! part by the state of the item it sits in (`group-data-[state=…]`).

use gui::prelude::*;
use gui::{Div, Signal, div, text};

use crate::separator::Orientation;
use crate::theme::theme;

/// Where a step stands against the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Completed,
    Active,
    Inactive,
}

/// The current step (1-based, as reka's) and how steps may be reached.
#[derive(Clone)]
pub struct StepperState {
    pub step: Signal<usize>,
    pub orientation: Orientation,
    /// Only the steps done and the next one can be reached.
    pub linear: bool,
}

impl StepperState {
    #[must_use]
    pub fn new(default_step: usize) -> Self {
        Self {
            step: Signal::labeled("stepper step", default_step),
            orientation: Orientation::Horizontal,
            linear: true,
        }
    }

    #[must_use]
    pub fn state_of(&self, step: usize) -> StepState {
        let current = self.step.get();
        match step.cmp(&current) {
            std::cmp::Ordering::Less => StepState::Completed,
            std::cmp::Ordering::Equal => StepState::Active,
            std::cmp::Ordering::Greater => StepState::Inactive,
        }
    }

    /// Whether `step` can be moved to now (reka's `isFocusable`).
    #[must_use]
    pub fn reachable(&self, step: usize) -> bool {
        !self.linear || step <= self.step.get().saturating_add(1)
    }

    /// Move to `step` when it can be reached.
    pub fn go(&self, step: usize) {
        if self.reachable(step) {
            self.step.set(step);
        }
    }
}

/// The row (or column) of steps.
#[must_use]
pub fn stepper(state: &StepperState) -> Div {
    div()
        .named("stepper")
        .flex()
        .gap(2.0)
        .when(state.orientation == Orientation::Vertical, Styled::flex_col)
}

/// Step `step`: holds its trigger, separator, title and description.
#[must_use]
pub fn stepper_item() -> Div {
    div().named("stepper-item").flex().items_center().gap(2.0)
}

/// What moves to step `step`: a click, or Enter or Space while focused.
#[must_use]
pub fn stepper_trigger(state: &StepperState, step: usize) -> Div {
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
        state,
        step,
    )
}

/// `child` made the trigger of step `step` (`as-child`), as a button is in the demos.
#[must_use]
pub fn stepper_trigger_as(child: Div, state: &StepperState, step: usize) -> Div {
    let go = state.clone();
    child
        .focusable()
        .disabled(!state.reachable(step))
        .on_click(move |_| go.go(step))
}

/// The round badge of step `step`, filled by its state; put its number or icon in it.
#[must_use]
pub fn stepper_indicator(state: &StepperState, step: usize) -> Div {
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
    let d = match state.state_of(step) {
        StepState::Active => d.bg(t.primary).text_color(t.primary_foreground),
        StepState::Completed => d.bg(t.accent).text_color(t.accent_foreground),
        StepState::Inactive => d,
    };
    d.when(!state.reachable(step), |d| {
        d.text_color(t.muted_foreground).opacity(0.5)
    })
}

/// The line from step `step` to the next.
#[must_use]
pub fn stepper_separator(state: &StepperState, step: usize) -> Div {
    let t = theme();
    div()
        .named("stepper-separator")
        .bg(if state.state_of(step) == StepState::Completed {
            t.accent
        } else {
            t.muted
        })
        .when(!state.reachable(step), |d| d.opacity(0.5))
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
