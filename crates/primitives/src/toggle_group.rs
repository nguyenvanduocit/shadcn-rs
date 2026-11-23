//! A row of toggles, one or many pressed (reka's `ToggleGroupRoot`,
//! `ToggleGroupItem`; research §10 "ToggleGroup").
//!
//! - **One or many are two types** (research §3.2): a group of one holds an
//!   `Option<T>`, a group of many a `Vec<T>` ([`Pressing`]). reka infers the
//!   kind from the shape of the value when the caller names none, and then
//!   behaves as one with an empty list as its value
//!   (`shared/useSingleOrMultipleValue.ts:18-40`); here the type says it.
//! - **One.** A press picks the item; pressing the pressed item lets it go,
//!   so the group may be empty (`useSingleOrMultipleValue.ts:53-56`). A
//!   caller that wants one always pressed keeps the value and does not write
//!   the empty one back.
//! - **Many.** A press appends the value or removes it
//!   (`useSingleOrMultipleValue.ts:57-67`).
//! - **Keys.** While roving (the default), the group is one Tab stop,
//!   entered at the first pressed item, else the first usable one; the
//!   arrows move focus only, wrapping, on the group's axis, Home/PageUp and
//!   End/PageDown to the ends, disabled items skipped
//!   ([`Keymap::TOGGLE_GROUP`], `ToggleGroupRoot.vue:47-51,80-104`). Not
//!   roving, each item is a Tab stop and the arrows do nothing
//!   (`ToggleGroupItem.vue:36-40`). Space and Enter press the focused item.
//! - **ARIA.** The root is a `group` with no orientation
//!   (`ToggleGroupRoot.vue:90`); items are toggle buttons with
//!   `aria-pressed`, also in a group of one (no radio semantics).
//! - **Form.** One sends `name=value`, empty as `name=` ; many send
//!   `name[i]`, nothing when empty; a disabled group sends nothing; items do
//!   not send on their own (`Toggle.vue:106`). `Option<String>` and
//!   `Vec<String>`'s [`crate::form::Submit`] encode them so.

use gui::{Div, FocusHandle, Role, Signal, UseModel};

use crate::availability::Availability;
use crate::collection::Item;
use crate::direction::ReadDirection;
use crate::keymap::Keymap;
use crate::roving::Roving;
use crate::toggle;

/// What a toggle group holds: one pressed value or none (`Option<T>`), or
/// any number (`Vec<T>`).
pub trait Pressing<T>: Clone + PartialEq + 'static {
    /// Whether `value` is pressed.
    fn holds(&self, value: &T) -> bool;
    /// The value after a press of `value`.
    #[must_use]
    fn flipped(&self, value: &T) -> Self;
}

/// One pressed, or none: pressing the pressed one lets it go.
impl<T: Clone + PartialEq + 'static> Pressing<T> for Option<T> {
    fn holds(&self, value: &T) -> bool {
        self.as_ref() == Some(value)
    }

    fn flipped(&self, value: &T) -> Self {
        if self.holds(value) { None } else { Some(value.clone()) }
    }
}

/// Any number pressed, in the order pressed.
impl<T: Clone + PartialEq + 'static> Pressing<T> for Vec<T> {
    fn holds(&self, value: &T) -> bool {
        self.contains(value)
    }

    fn flipped(&self, value: &T) -> Self {
        let mut next = self.clone();
        match next.iter().position(|v| v == value) {
            Some(at) => {
                next.remove(at);
            }
            None => next.push(value.clone()),
        }
        next
    }
}

/// A toggle group: made once in its root's setup, `Copy`.
pub struct ToggleGroup<P: 'static, V: 'static, T: 'static> {
    value: UseModel<P, V>,
    roving: Roving<T>,
    /// Whether the group is one Tab stop navigated by the arrows.
    rove: Signal<bool>,
}

impl<P: 'static, V: 'static, T: 'static> Clone for ToggleGroup<P, V, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static, V: 'static, T: 'static> Copy for ToggleGroup<P, V, T> {}

impl<P: 'static, V: 'static, T: 'static> std::fmt::Debug for ToggleGroup<P, V, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToggleGroup").finish_non_exhaustive()
    }
}

impl<P: 'static, V: Pressing<T>, T: Clone + PartialEq + 'static> ToggleGroup<P, V, T> {
    /// A group writing `value` (its `v-model`), navigated in `direction`;
    /// called in setup. It roves, wraps and takes both axes until told
    /// otherwise.
    #[must_use]
    #[track_caller]
    pub fn new(value: UseModel<P, V>, direction: impl Into<ReadDirection>) -> Self {
        Self {
            value,
            roving: Roving::new(Keymap::TOGGLE_GROUP, direction),
            rove: Signal::labeled("toggle group roves", true),
        }
    }

    /// Navigate by `keymap` from now on (an orientation, a `loop`).
    pub fn set_keymap(self, keymap: Keymap) {
        self.roving.set_keymap(keymap);
    }

    /// One Tab stop with arrow keys (true, the default), or a Tab stop per
    /// item (reka's `rovingFocus: false`).
    pub fn set_roving(self, rove: bool) {
        self.rove.set(rove);
    }

    /// Whether `value` is pressed (tracked).
    #[must_use]
    pub fn pressed(self, value: &T) -> bool {
        self.value.with(|v| v.holds(value))
    }

    /// Press `value`, as a click on its item does.
    pub fn flip(self, value: &T) {
        self.value.update(|v| *v = v.flipped(value));
    }

    /// `d` as the group's root: a `group`.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        d.role(Role::Group)
    }

    /// `d` as the item of `value`, focused through `focus`, usable as
    /// `availability` says (the group's and its own, the more
    /// restrictive): a toggle pressed while the group holds its value. While
    /// the group roves it joins its one Tab stop and takes the arrows (it
    /// sets the node's key handler; reads the roving setting, tracked).
    #[must_use]
    pub fn item(self, d: Div, value: T, focus: FocusHandle, availability: Availability) -> Div {
        self.item_then(d, value, focus, availability, || {})
    }

    /// As [`Self::item`], running `after` once a press has written the
    /// group (the item's own `@click`), and only when the press was taken:
    /// never on an item that is not usable.
    #[must_use]
    pub fn item_then(
        self,
        d: Div,
        value: T,
        focus: FocusHandle,
        availability: Availability,
        after: impl Fn() + 'static,
    ) -> Div {
        let pressed = self.pressed(&value);
        let flip = value.clone();
        let d = toggle::root(d, pressed, availability, move |_| {
            self.flip(&flip);
            after();
        });
        let item = Item::new(value, focus).active(pressed).availability(availability);
        if self.rove.get() {
            self.roving.item(d, item)
        } else {
            // Still one of the group's items (in order), but each a Tab stop, no arrows.
            self.roving.member(d, item).tab_stop(true)
        }
    }

    /// The items, in tree order as last laid out (a look joining their
    /// corners reads their order).
    #[must_use]
    pub fn items(self) -> crate::collection::Collection<T> {
        self.roving.items()
    }
}

#[cfg(test)]
mod tests {
    use super::Pressing;

    #[test]
    fn one_may_be_let_go_and_many_toggle_in_order() {
        let one: Option<&str> = None;
        let one = one.flipped(&"a");
        assert_eq!(one, Some("a"));
        assert_eq!(one.flipped(&"b"), Some("b"), "another press picks the other");
        assert_eq!(
            one.flipped(&"a"),
            None,
            "the pressed one is let go: a group of one may be empty"
        );
        let many = vec!["a"].flipped(&"b").flipped(&"c");
        assert_eq!(many, ["a", "b", "c"]);
        assert_eq!(many.flipped(&"b"), ["a", "c"]);
    }
}
