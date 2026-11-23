//! A box that is checked, not, or indeterminate (reka's `CheckboxRoot`,
//! `CheckboxIndicator`, `CheckboxGroupRoot`; research §10 "Checkbox").
//!
//! - **Value.** A [`CheckedState`]. The user's toggle ([`CheckedState::toggled`])
//!   turns an indeterminate box checked, a checked one unchecked, anything
//!   else checked (`Checkbox/CheckboxRoot.vue:181-187`): only the caller makes
//!   a box indeterminate.
//! - **Keys.** Space toggles (the engine presses the focused control); Enter
//!   does nothing, as WAI-ARIA has it (`CheckboxRoot.vue:318-320`). A switch
//!   flips on Enter; a checkbox does not.
//! - **Root.** Role `checkbox`, `aria-checked` true, false or mixed,
//!   `aria-required` when required (`CheckboxRoot.vue:311`), one Tab stop. A disabled box takes no focus; an unavailable one (at its group's
//!   limit) keeps focus and arrow navigation, is read as disabled, and its
//!   clicks do not run (`CheckboxRoot.vue:118-123`).
//! - **Indicator.** Shown while checked or indeterminate
//!   ([`CheckedState::shows_indicator`]); it takes no pointer
//!   (`CheckboxIndicator.vue:29-36`).
//! - **Group.** [`CheckboxGroup`] writes one list of values: a box is checked
//!   while its value is in the list, a click appends or removes it
//!   (`CheckboxRoot.vue:139-143,172-178`), a `max` makes the boxes not
//!   checked unavailable once reached (`CheckboxRoot.vue:108-116`). The group
//!   is one roving Tab stop, no wrap, both axes ([`Keymap::ROVING`],
//!   `CheckboxGroupRoot.vue:61-63,81-83`), and has no role of its own.
//! - **Parent.** A box over "all" of a group ([`CheckboxGroup::parent`]):
//!   checked when every value of "all" is in, unchecked when none (or "all"
//!   is empty), indeterminate otherwise; a click adds the usable values of
//!   "all" up to the limit, or, when checked or when adding changes nothing,
//!   removes the usable ones (`Checkbox/utils.ts:18-57`). The limit does not
//!   hold it back (`CheckboxRoot.vue:111-112`). It tells AT the boxes it
//!   controls (`aria-controls`, `CheckboxRoot.vue:256-262`): the nodes of
//!   the values of "all", those mounted. It is one of the group's roving
//!   items under a key of its own ([`Member::Parent`]), never under one of
//!   its boxes' values.
//! - **Form.** A single box sends `name=on` while checked or indeterminate
//!   (`CheckboxRoot.vue:334`) and nothing while unchecked; a group sends its
//!   list as `name[i]` ([`crate::form::FormData::List`]). A required box is
//!   told to AT by [`root`]; the check is the form's rule ([`required`]).
//!   A group's boxes are not required one by one: reka's group `required`
//!   only asks for one value in its list, a form rule on the list.
//!
//! reka compares values structurally and lets a box hold any `trueValue` /
//! `falseValue`; here a box holds a [`CheckedState`] and a group's values are
//! compared by `T: PartialEq` (ADR-0025 §1).

use std::rc::Rc;

use gui::{Div, FocusHandle, KeyEvent, NamedKey, NodeRef, Role, Signal, Toggled, UseModel};

use crate::availability::Availability;
use crate::collection::Item;
use crate::direction::ReadDirection;
use crate::form::{FormData, Submit};
use crate::keymap::Keymap;
use crate::roving::Roving;

/// What a checkbox shows: reka's `boolean | 'indeterminate'`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum CheckedState {
    #[default]
    Unchecked,
    Checked,
    /// Some of what it stands for is checked (a "select all" over a part).
    Indeterminate,
}

impl From<bool> for CheckedState {
    fn from(checked: bool) -> Self {
        if checked { Self::Checked } else { Self::Unchecked }
    }
}

impl CheckedState {
    /// What the user's click or Space turns it to: indeterminate and
    /// unchecked become checked, checked becomes unchecked.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Checked => Self::Unchecked,
            Self::Unchecked | Self::Indeterminate => Self::Checked,
        }
    }

    /// Whether it is checked, and only that: a field's "filled"
    /// (`CheckboxRoot.vue:195-198`), the bool a caller keeps.
    #[must_use]
    pub const fn is_checked(self) -> bool {
        matches!(self, Self::Checked)
    }

    /// Whether the indicator shows: checked or indeterminate.
    #[must_use]
    pub const fn shows_indicator(self) -> bool {
        !matches!(self, Self::Unchecked)
    }

    /// `aria-checked`: true, false or mixed.
    #[must_use]
    pub const fn aria(self) -> Toggled {
        match self {
            Self::Checked => Toggled::True,
            Self::Unchecked => Toggled::False,
            Self::Indeterminate => Toggled::Mixed,
        }
    }
}

/// A single box sends `on` while checked or indeterminate, nothing while
/// unchecked (`CheckboxRoot.vue:334`).
impl Submit for CheckedState {
    fn form_data(&self) -> FormData {
        FormData::Toggle(self.shows_indicator().then(|| "on".to_owned()))
    }
}

/// The message a required box fails while not checked (reka's native
/// `required`, `CheckboxRoot.vue:331-340`); indeterminate counts as checked,
/// as the native input it sends through is.
#[must_use]
pub fn required(state: &CheckedState, message: &str) -> Vec<String> {
    if state.shows_indicator() {
        Vec::new()
    } else {
        vec![message.to_owned()]
    }
}

/// Enter on a checkbox does nothing (WAI-ARIA): consumed, so the engine
/// does not press the box.
fn enter_does_nothing(ev: &KeyEvent) -> bool {
    ev.is(NamedKey::Enter)
}

/// `d` as a checkbox showing `state`, usable as `availability` says and
/// `required` or not: role, `aria-checked`, `aria-required`, focus, and the
/// toggle, which writes [`CheckedState::toggled`] through `set`. It sets
/// the node's key handler (Enter is consumed); a part with keys of its own
/// sets them after it.
#[must_use]
pub fn root(
    d: Div,
    state: CheckedState,
    availability: Availability,
    required: bool,
    set: impl Fn(CheckedState) + 'static,
) -> Div {
    availability
        .apply(
            d.role(Role::CheckBox)
                .aria_checked(state.aria())
                .aria_required(required)
                .focusable(),
        )
        .on_click(move |_| {
            if availability.actionable() {
                set(state.toggled());
            }
        })
        .on_key_down(enter_does_nothing)
}

/// `d` as the indicator inside a box: decoration that takes no pointer.
/// The look shows it while [`CheckedState::shows_indicator`].
#[must_use]
pub fn indicator(d: Div) -> Div {
    d.pass_through()
}

/// What a parent box over `all` shows for the group's `values`: checked
/// when every value of `all` is in, unchecked when none is or `all` is
/// empty, indeterminate otherwise (`Checkbox/utils.ts:18-27`).
#[must_use]
pub fn parent_state<T: PartialEq>(values: &[T], all: &[T]) -> CheckedState {
    let held = all.iter().filter(|v| values.contains(v)).count();
    if all.is_empty() || held == 0 {
        CheckedState::Unchecked
    } else if held == all.len() {
        CheckedState::Checked
    } else {
        CheckedState::Indeterminate
    }
}

/// The group's values after a click on a parent box over `all`
/// (`Checkbox/utils.ts:35-57`): unless checked, the usable values of `all`
/// not yet in are appended in order, stopping at `max`; when checked, or
/// when that adds nothing, every usable value of `all` is removed. Values
/// outside `all` and those of disabled boxes stay as they are.
#[must_use]
pub fn parent_toggled<T: Clone + PartialEq>(
    values: &[T],
    all: &[T],
    usable: impl Fn(&T) -> bool,
    max: Option<usize>,
) -> Vec<T> {
    let mut next = values.to_vec();
    if parent_state(values, all) != CheckedState::Checked {
        for v in all.iter().filter(|v| usable(v) && !values.contains(v)) {
            if max.is_some_and(|m| next.len() >= m) {
                break;
            }
            next.push(v.clone());
        }
        if next.len() != values.len() {
            return next;
        }
    }
    next.retain(|v| !(all.contains(v) && usable(v)));
    next
}

/// One of a group's roving items: its parent box, or the box of a value.
/// The parent has a key of its own, so no two items share one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Member<T> {
    Parent,
    Box(T),
}

/// A group of boxes writing one list (reka's `CheckboxGroupRoot`): made
/// once in the group's setup, `Copy`.
pub struct CheckboxGroup<P: 'static, T: 'static> {
    value: UseModel<P, Vec<T>>,
    /// How many values the list may hold; none is no limit.
    max: Signal<Option<usize>>,
    roving: Roving<Member<T>>,
}

impl<P: 'static, T: 'static> Clone for CheckboxGroup<P, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static, T: 'static> Copy for CheckboxGroup<P, T> {}

impl<P: 'static, T: 'static> std::fmt::Debug for CheckboxGroup<P, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckboxGroup").finish_non_exhaustive()
    }
}

impl<P: 'static, T: Clone + PartialEq + 'static> CheckboxGroup<P, T> {
    /// A group writing `value` (the group's `v-model`), navigated in
    /// `direction`; called in setup.
    #[must_use]
    #[track_caller]
    pub fn new(value: UseModel<P, Vec<T>>, direction: impl Into<ReadDirection>) -> Self {
        Self {
            value,
            max: Signal::labeled("checkbox group max", None),
            roving: Roving::new(Keymap::ROVING, direction),
        }
    }

    /// At most `max` values from now on (reka's `max`); none is no limit.
    pub fn set_max(self, max: Option<usize>) {
        self.max.set(max);
    }

    /// Navigate by `keymap` from now on (an orientation, a `loop`).
    pub fn set_keymap(self, keymap: Keymap) {
        self.roving.set_keymap(keymap);
    }

    /// The values checked (tracked).
    #[must_use]
    pub fn values(self) -> Vec<T> {
        self.value.get()
    }

    /// Whether `value` is checked (tracked).
    #[must_use]
    pub fn checked(self, value: &T) -> bool {
        self.value.with(|v| v.contains(value))
    }

    /// Whether `value`, not checked, is held back by the limit (tracked):
    /// a state of its own for the look, apart from disabled.
    #[must_use]
    pub fn limited(self, value: &T) -> bool {
        let max = self.max.get();
        self.value
            .with(|v| !v.contains(value) && max.is_some_and(|m| v.len() >= m))
    }

    /// Check `value` or let it go, as a click on its box does.
    pub fn flip(self, value: &T) {
        self.value.update(|v| match v.iter().position(|x| x == value) {
            Some(at) => {
                v.remove(at);
            }
            None => v.push(value.clone()),
        });
    }

    /// `d` as the box of `value`, named `node` (what a parent's
    /// `aria-controls` points at), focused through `focus` and usable as
    /// `availability` says (the group's and its own, the more restrictive):
    /// unavailable once the limit holds it back. It joins the group's one
    /// Tab stop and takes the arrow keys; it sets the node's key handler.
    #[must_use]
    pub fn item(self, d: Div, value: T, node: NodeRef, focus: FocusHandle, availability: Availability) -> Div {
        let checked = self.checked(&value);
        let availability = match availability {
            Availability::Enabled if self.limited(&value) => Availability::Unavailable,
            a => a,
        };
        let item = Item::new(Member::Box(value.clone()), focus).availability(availability);
        let d = root(
            d.node_ref(node),
            CheckedState::from(checked),
            availability,
            false,
            move |_| {
                self.flip(&value);
            },
        );
        let roving = self.roving;
        roving
            .member(d, item)
            .on_key_down(move |ev| enter_does_nothing(ev) || roving.key(focus, ev).consumed())
    }

    /// `d` as a parent box over `all`, the values of the boxes it checks
    /// and unchecks, each with its box's node (given to
    /// [`CheckboxGroup::item`]): it controls those boxes (`aria-controls`;
    /// one not mounted is not told). `usable` says which of them can be
    /// acted on (a disabled box's value is left alone). Not held back by
    /// the limit. It joins the group's one Tab stop as [`Member::Parent`].
    #[must_use]
    pub fn parent(
        self,
        d: Div,
        all: Rc<[(T, NodeRef)]>,
        usable: Rc<dyn Fn(&T) -> bool>,
        focus: FocusHandle,
        availability: Availability,
    ) -> Div {
        let values: Rc<[T]> = all.iter().map(|(v, _)| v.clone()).collect();
        let state = self.value.with(|v| parent_state(v, &values));
        let d = all.iter().fold(d, |d, &(_, node)| d.controls(node));
        let d = root(d, state, availability, false, move |_| {
            let max = self.max.get_untracked();
            let next = gui::untracked(|| self.value.with(|v| parent_toggled(v, &values, |x| usable(x), max)));
            self.value.set(next);
        });
        let roving = self.roving;
        roving
            .member(d, Item::new(Member::Parent, focus).availability(availability))
            .on_key_down(move |ev| enter_does_nothing(ev) || roving.key(focus, ev).consumed())
    }
}

#[cfg(test)]
mod tests {
    use super::{CheckedState, parent_state, parent_toggled};

    #[test]
    fn the_users_toggle_never_makes_indeterminate() {
        use CheckedState::{Checked, Indeterminate, Unchecked};
        assert_eq!(
            Indeterminate.toggled(),
            Checked,
            "reka: indeterminate then click is checked"
        );
        assert_eq!(Checked.toggled(), Unchecked);
        assert_eq!(Unchecked.toggled(), Checked);
        assert!(Indeterminate.shows_indicator() && !Indeterminate.is_checked());
    }

    #[test]
    fn a_parent_reads_and_writes_all_as_reka_does() {
        let all = ["a", "b", "c"];
        assert_eq!(parent_state(&[], &all), CheckedState::Unchecked);
        assert_eq!(parent_state(&["a"], &all), CheckedState::Indeterminate);
        assert_eq!(parent_state(&["c", "a", "b"], &all), CheckedState::Checked);
        assert_eq!(
            parent_state::<&str>(&["x"], &[]),
            CheckedState::Unchecked,
            "no all: unchecked"
        );
        // Not checked: the usable values are appended in order, the limit stops it.
        let usable = |v: &&str| *v != "b";
        assert_eq!(parent_toggled(&["x"], &all, usable, None), ["x", "a", "c"]);
        assert_eq!(parent_toggled(&["x"], &all, usable, Some(2)), ["x", "a"]);
        // Only a disabled one left to add: adding changes nothing, so it unchecks.
        assert_eq!(parent_toggled(&["a", "c", "x"], &all, usable, None), ["x"]);
        // Checked: every usable value goes; a disabled box's value stays.
        assert_eq!(parent_toggled(&["a", "b", "c", "x"], &all, usable, None), ["b", "x"]);
    }
}
