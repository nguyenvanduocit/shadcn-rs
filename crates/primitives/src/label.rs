//! A control's name (reka's `Label`, which leans on `<label>`'s native
//! behaviour; research §6: "reka writes no line for either").
//!
//! Both of `<label>`'s behaviours are built here:
//!
//! - **Naming.** The control is named by the label's text
//!   (`aria-labelledby`), through typed relations: the label marks its node,
//!   the control points at it.
//! - **Click.** A click on the label acts on its control: a text field
//!   takes focus, a checkbox or switch toggles and takes focus, as a click on
//!   the control would. A click on an interactive node inside the label is
//!   that node's, not the label's (reka's Field `nonNative` mode, which a
//!   select's trigger needs).
//!
//! The look reads whether the control is disabled or invalid (shadcn dims a
//! label beside a disabled control and colours it beside an invalid one).

use gui::value::{IntoValue, Value};
use gui::{Callback, Div, FocusHandle, NodeRef, Role};

use crate::Availability;

/// What a click on a label does to its control.
#[derive(Clone, PartialEq)]
pub enum LabelFor {
    /// Focus it (a text field, a select's trigger).
    Focus(FocusHandle),
    /// Focus it and act on it as a click does (a checkbox, a switch).
    Activate(FocusHandle, Callback<()>),
}

impl std::fmt::Debug for LabelFor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Focus(h) => f.debug_tuple("Focus").field(h).finish(),
            Self::Activate(h, _) => f.debug_tuple("Activate").field(h).finish(),
        }
    }
}

/// A label and its control: made once, in the setup of whatever holds both,
/// `Copy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Label {
    node: NodeRef,
}

impl Default for Label {
    fn default() -> Self {
        Self::new()
    }
}

impl Label {
    #[must_use]
    pub fn new() -> Self {
        Self { node: NodeRef::new() }
    }

    /// The label's node, for a relation of another kind.
    #[must_use]
    pub fn node(self) -> NodeRef {
        self.node
    }

    /// `d` as the label of a control `target` says how to act on: a
    /// label to assistive technology, whatever handles its clicks.
    /// `availability` is the control's own (a value or a closure, read when
    /// the click comes): a disabled control is not acted on or focused, an
    /// unavailable one is focused and not acted on.
    #[must_use]
    pub fn label<M>(self, d: Div, target: LabelFor, availability: impl IntoValue<Availability, M>) -> Div {
        let availability = availability.into_value();
        d.role(Role::Label).node_ref(self.node).on_click(move |_| {
            let now = match &availability {
                Value::Fixed(a) => *a,
                Value::Bound(f) => gui::untracked(|| f()),
            };
            if !now.focusable() {
                return;
            }
            match &target {
                LabelFor::Focus(h) => h.focus(),
                LabelFor::Activate(h, act) => {
                    h.focus();
                    if now.actionable() {
                        act.call(());
                    }
                }
            }
        })
    }

    /// `d` as the control the label names.
    #[must_use]
    pub fn control(self, d: Div) -> Div {
        d.labelled_by(self.node)
    }
}
