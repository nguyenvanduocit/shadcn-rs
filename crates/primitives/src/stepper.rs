//! Numbered steps of a flow (reka's `StepperRoot`, `StepperItem`,
//! `StepperTrigger`, `StepperIndicator`, `StepperSeparator`; research §10
//! "Stepper").
//!
//! - **Value.** The current step, a number from 1 (the default). The user
//!   moves it through a trigger; the caller writes it, or asks
//!   [`Stepper::go_to`], [`Stepper::next`], [`Stepper::prev`]
//!   (`StepperRoot.vue:121-153`).
//! - **State of a step** ([`state_of`], `StepperItem.vue:61-69`): completed
//!   when its item says so, else active when it is the current step, else
//!   completed when before it, else inactive.
//! - **Reach** ([`Stepper::reachable`], `StepperItem.vue:73-80`): a disabled
//!   step never; in a linear stepper (the default) the steps up to the
//!   current one and the one after it; otherwise every step. A step out of
//!   reach is disabled and out of the Tab order.
//! - **Going to a step** refuses one below 1 or past the last, a disabled
//!   one, and in a linear stepper one past the next. reka's check of a
//!   disabled step reads the wrong index and never holds
//!   (`StepperRoot.vue:128`; research §4 item 13); here it does.
//! - **Keys** on a trigger: Enter or Space (without Control or Shift)
//!   moves to its step at once, as the key goes down
//!   (`StepperTrigger.vue:89-90`); the arrows of the stepper's axis focus the
//!   previous or next reachable trigger, no wrap, right to left turning
//!   ArrowLeft and ArrowRight, and select nothing; arrows of the other axis
//!   are consumed and do nothing (`StepperTrigger.vue:84,92-100`); Home and
//!   End are not the stepper's. Each reachable trigger is a Tab stop; there
//!   is no roving.
//! - **Pointer.** A primary press moves to the step as it goes down, not on
//!   release; with Control it neither moves nor takes focus, and its click
//!   does nothing (`StepperTrigger.vue:61-81`). Assistive technology's press
//!   moves.
//! - **ARIA.** The root is a `group` named "progress"; the current step's
//!   item is `aria-current="true"` and its trigger `aria-current="step"`,
//!   completed or not; a polite, atomic status line tells
//!   "Step i of n", i the current trigger's place among those shown, not its
//!   step number, empty when no trigger has the current step
//!   ([`Stepper::announcement`], `StepperRoot.vue:104-117,228-242`). The
//!   strings are [`StepperNames`]'s, English by default.
//! - **Trigger names** ([`StepLabels`], `StepperTrigger.vue:37-56`): a
//!   trigger is named by its step's title and description, those shown
//!   (a relation to a part not in the tree is not told), so its name is
//!   what is seen; one the caller names ([`Stepper::trigger_labelled`]) is
//!   described by the description instead. The line between steps is a
//!   decorative [`crate::separator::Separator`] (`StepperSeparator.vue`),
//!   nothing to assistive technology.

use gui::access::NodeRef;
use gui::element::{AriaCurrent, Live, Node};
use gui::{Axis, Div, FocusHandle, Key, KeyEvent, NamedKey, Role, Signal, UseModel};

use crate::availability::Availability;
use crate::collection::{Collection, Item};
use crate::direction::ReadDirection;
use crate::keymap::{Arrows, Intent, Keymap};

/// Where a step stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StepState {
    Completed,
    Active,
    Inactive,
}

/// The state of `step` while `current` is the current one, `completed`
/// its item's own flag.
#[must_use]
pub fn state_of(step: usize, current: usize, completed: bool) -> StepState {
    if completed {
        StepState::Completed
    } else if step == current {
        StepState::Active
    } else if step < current {
        StepState::Completed
    } else {
        StepState::Inactive
    }
}

/// What a screen reader is told of a stepper: reka's English strings by
/// default.
#[derive(Clone, Copy, Debug)]
pub struct StepperNames {
    /// The root's name (`aria-label="progress"`).
    pub root: &'static str,
    /// An indicator's default content: step `n` (`StepperIndicator.vue:29-31`).
    pub step: fn(usize) -> String,
    /// The status line: trigger `i` (from 1) of `n` is current.
    pub announce: fn(usize, usize) -> String,
}

impl PartialEq for StepperNames {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
            && std::ptr::fn_addr_eq(self.step, other.step)
            && std::ptr::fn_addr_eq(self.announce, other.announce)
    }
}

impl Default for StepperNames {
    fn default() -> Self {
        Self {
            root: "progress",
            step: |n| format!("Step {n}"),
            announce: |i, n| format!("Step {i} of {n}"),
        }
    }
}

/// The title and description of one step, for its trigger's name: each
/// part is given its node with [`Self::title`] and [`Self::description`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StepLabels {
    title: NodeRef,
    description: NodeRef,
}

impl Default for StepLabels {
    fn default() -> Self {
        Self::new()
    }
}

impl StepLabels {
    #[must_use]
    pub fn new() -> Self {
        Self {
            title: NodeRef::new(),
            description: NodeRef::new(),
        }
    }

    /// `n` as the step's title (reka's `StepperTitle`).
    #[must_use]
    pub fn title<K>(self, n: Node<K>) -> Node<K> {
        n.node_ref(self.title)
    }

    /// `n` as the step's description (reka's `StepperDescription`).
    #[must_use]
    pub fn description<K>(self, n: Node<K>) -> Node<K> {
        n.node_ref(self.description)
    }
}

/// A stepper: made once in its root's setup, `Copy`.
pub struct Stepper<P: 'static> {
    step: UseModel<P, usize>,
    linear: Signal<bool>,
    orientation: Signal<Axis>,
    /// The triggers shown, by step and their own availability, in tree
    /// order as last laid out; a trigger out of reach is disabled.
    triggers: Signal<Collection<(usize, Availability)>>,
    /// The steps of the triggers shown, copied from the last layout so the
    /// status line follows it.
    laid: Signal<Vec<usize>>,
    direction: ReadDirection,
    names: Signal<StepperNames>,
    /// Each step's title and description nodes, issued as the step is
    /// first asked for, held by each part that names them (its trigger, its
    /// title, its description) while it is on screen, let go as the last
    /// of them leaves.
    labels: gui::Keyed<usize, StepLabels>,
}

impl<P: 'static> Clone for Stepper<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Stepper<P> {}

impl<P: 'static> std::fmt::Debug for Stepper<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stepper").finish_non_exhaustive()
    }
}

impl<P: 'static> Stepper<P> {
    /// A stepper writing `step` (its `v-model`), its arrows read in
    /// `direction`; called in setup. Linear and across until told otherwise.
    #[must_use]
    #[track_caller]
    pub fn new(step: UseModel<P, usize>, direction: impl Into<ReadDirection>) -> Self {
        Self {
            step,
            linear: Signal::labeled("stepper linear", true),
            orientation: Signal::labeled("stepper orientation", Axis::Horizontal),
            triggers: Signal::new(Collection::new()),
            laid: Signal::labeled("stepper triggers laid out", Vec::new()),
            direction: direction.into(),
            names: Signal::labeled("stepper names", StepperNames::default()),
            labels: gui::Keyed::new(),
        }
    }

    /// The title and description nodes of `step`, the same at each call
    /// while any part holds them: called in a render, held while its output
    /// may be on screen.
    #[must_use]
    #[track_caller]
    pub fn labels(self, step: usize) -> StepLabels {
        self.labels.hold(&step, StepLabels::new)
    }

    /// How many steps the stepper keeps anything of: the triggers shown, as
    /// last laid out, and the steps' title and description nodes (let go
    /// as the last part naming them leaves).
    #[must_use]
    pub fn kept_steps(self) -> usize {
        let shown = self.triggers.with_untracked(Collection::len);
        self.laid.with_untracked(Vec::len).max(shown).max(self.labels.len())
    }

    /// Linear or not, along `orientation`, from now on.
    pub fn set_layout(self, linear: bool, orientation: Axis) {
        self.linear.set(linear);
        self.orientation.set(orientation);
    }

    pub fn set_names(self, names: StepperNames) {
        self.names.set(names);
    }

    /// The triggers shown, as last laid out.
    fn triggers(self) -> Collection<(usize, Availability)> {
        self.triggers.with_untracked(Clone::clone)
    }

    /// The availability step `step`'s trigger was last laid out with, its
    /// own (none: no trigger has that step).
    fn own(self, step: usize) -> Option<Availability> {
        self.triggers()
            .with(|t| t.iter().find(|i| i.value.0 == step).map(|i| i.value.1))
    }

    /// The current step (tracked).
    #[must_use]
    pub fn current(self) -> usize {
        self.step.get()
    }

    /// Whether it is linear (tracked).
    #[must_use]
    pub fn linear(self) -> bool {
        self.linear.get()
    }

    /// Its axis (tracked).
    #[must_use]
    pub fn orientation(self) -> Axis {
        self.orientation.get()
    }

    /// How many steps there are: the triggers shown (untracked: as last
    /// laid out).
    #[must_use]
    pub fn total(self) -> usize {
        self.triggers().len()
    }

    /// The state of `step`, `completed` its item's own flag (tracked).
    #[must_use]
    pub fn state(self, step: usize, completed: bool) -> StepState {
        state_of(step, self.step.get(), completed)
    }

    /// Whether `step`, usable as `own` says (its item's), can be moved to
    /// now (tracked).
    #[must_use]
    pub fn reachable(self, step: usize, own: Availability) -> bool {
        let current = self.step.get();
        own.actionable() && (!self.linear.get() || step <= current.saturating_add(1))
    }

    /// Move to `step` when it may be gone to; whether it was.
    pub fn go_to(self, step: usize) -> bool {
        let own = self.own(step).unwrap_or(Availability::Disabled);
        let ok = step >= 1 && step <= self.total() && gui::untracked(|| self.reachable(step, own));
        if ok {
            self.step.set(step);
        }
        ok
    }

    /// Move to the next step; whether it was.
    pub fn next(self) -> bool {
        self.go_to(self.step.get_untracked().saturating_add(1))
    }

    /// Move to the previous step; whether it was.
    pub fn prev(self) -> bool {
        self.step.get_untracked().checked_sub(1).is_some_and(|s| self.go_to(s))
    }

    /// Whether there is a step after the current one, and it is not disabled.
    #[must_use]
    pub fn has_next(self) -> bool {
        let next = self.step.get().saturating_add(1);
        next <= self.total() && self.own(next).is_some_and(Availability::actionable)
    }

    /// Whether there is a step before the current one, and it is not disabled.
    #[must_use]
    pub fn has_prev(self) -> bool {
        let prev = self.step.get().saturating_sub(1);
        prev >= 1 && self.own(prev).is_some_and(Availability::actionable)
    }

    /// What the status line tells: the current trigger's place among those
    /// shown, empty when none has the current step (tracked: the step, and
    /// the triggers as [`Self::status`] last read them from layout).
    #[must_use]
    pub fn announcement(self) -> String {
        let current = self.step.get();
        self.laid.with(|steps| match steps.iter().position(|s| *s == current) {
            Some(i) => (self.names.get().announce)(i.saturating_add(1), steps.len()),
            None => String::new(),
        })
    }

    /// An indicator's default content for `step`.
    #[must_use]
    pub fn step_name(self, step: usize) -> String {
        (self.names.get().step)(step)
    }

    /// `d` as the root: a `group` named by [`StepperNames::root`].
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        let name = self.names.get().root;
        d.role(Role::Group).aria_label(name)
    }

    /// `d` as the status line (a look hides it from sight): it reads
    /// [`Self::announcement`].
    #[must_use]
    pub fn status(self, d: Div) -> Div {
        // Read the triggers once they are laid out; the write renders the
        // line again in the same frame, only when the triggers changed.
        gui::after_layout(move || {
            let steps = self
                .triggers()
                .with(|t| t.iter().map(|i| i.value.0).collect::<Vec<_>>());
            self.laid.set(steps);
        });
        d.role(Role::Status)
            .aria_live(Live::Polite)
            .aria_label(self.announcement())
    }

    /// `d` as the item of `step`: the current one says so (`aria-current`,
    /// `StepperItem.vue:103`).
    #[must_use]
    pub fn item(self, d: Div, step: usize) -> Div {
        if self.step.get() == step {
            d.aria_current(AriaCurrent::True)
        } else {
            d
        }
    }

    /// What a key on the trigger of `step`, focused through `focus`, does.
    fn key(self, step: usize, focus: FocusHandle, ev: &KeyEvent) -> bool {
        let m = ev.modifiers;
        if (ev.is(NamedKey::Enter) || ev.is(NamedKey::Space) || ev.text.as_deref() == Some(" "))
            && !m.control
            && !m.shift
        {
            self.go_to(step);
            return true;
        }
        let Key::Named(k) = &ev.key else { return false };
        if !matches!(
            k,
            NamedKey::ArrowLeft | NamedKey::ArrowRight | NamedKey::ArrowUp | NamedKey::ArrowDown
        ) {
            return false;
        }
        let arrows = match self.orientation.get_untracked() {
            Axis::Horizontal => Arrows::Horizontal,
            Axis::Vertical => Arrows::Vertical,
        };
        let keymap = Keymap::LINE.arrows(arrows);
        if let Some(intent @ (Intent::Prev | Intent::Next)) = keymap.intent(ev, self.direction.get()) {
            self.triggers().with(|list| {
                let at = list.iter().position(|i| i.focus == focus);
                if let Some(to) = keymap.step(list, at, intent).and_then(|to| list.get(to)) {
                    gui::focus::set_focus(Some(to.focus.id()), true);
                }
            });
        }
        // An arrow of the other axis does nothing, and is consumed.
        true
    }

    /// `d` as the trigger of `step`, focused through `focus`, usable as
    /// `availability` says: a button (reka's `<button>`), named by its
    /// step's title and description ([`Self::labels`]), disabled while out
    /// of reach, its step chosen on a press and by Enter or Space, the
    /// arrows of the stepper's axis. It sets the node's key handler.
    #[must_use]
    pub fn trigger(self, d: Div, step: usize, focus: FocusHandle, availability: Availability) -> Div {
        let labels = self.labels(step);
        self.trigger_part(
            d.labelled_by(labels.title).labelled_by(labels.description),
            step,
            focus,
            availability,
        )
    }

    /// [`Self::trigger`] named `label` by the caller (an `aria-label`): its
    /// step's description describes it, as the title no longer names it.
    #[must_use]
    pub fn trigger_labelled(
        self,
        d: Div,
        step: usize,
        focus: FocusHandle,
        availability: Availability,
        label: impl Into<String>,
    ) -> Div {
        let labels = self.labels(step);
        self.trigger_part(
            d.aria_label(label).described_by(labels.description),
            step,
            focus,
            availability,
        )
    }

    fn trigger_part(self, d: Div, step: usize, focus: FocusHandle, availability: Availability) -> Div {
        let shown = if self.reachable(step, availability) {
            availability
        } else {
            Availability::Disabled
        };
        // The current step's trigger says so, completed or not (`StepperTrigger.vue:129`).
        let d = d.role(Role::Button);
        let d = if self.step.get() == step {
            d.aria_current(AriaCurrent::Step)
        } else {
            d
        };
        self.triggers()
            .item(d, Item::new((step, availability), focus).availability(shown))
            // A Control press moves nothing and takes no focus, so it cannot
            // activate by accident (`StepperTrigger.vue:61-81`).
            .focus_on_press(|m| !m.control)
            .on_mouse_down(move |ev| {
                if !ev.modifiers.control {
                    self.go_to(step);
                }
            })
            // Assistive technology's press moves here. After the pointer's
            // press the step is already this one, so the click writes
            // nothing (the model's equality cutoff), and no mark of the
            // press is kept that a press without a click could leave stale.
            .on_click(move |ev| {
                if !ev.modifiers.control {
                    self.go_to(step);
                }
            })
            .on_key_down(move |ev| self.key(step, focus, ev))
    }
}

#[cfg(test)]
mod tests {
    use super::{StepState, state_of};

    #[test]
    fn a_completed_flag_outranks_the_current_step() {
        assert_eq!(state_of(2, 2, true), StepState::Completed);
        assert_eq!(state_of(2, 2, false), StepState::Active);
        assert_eq!(state_of(1, 2, false), StepState::Completed);
        assert_eq!(state_of(3, 2, false), StepState::Inactive);
    }
}
