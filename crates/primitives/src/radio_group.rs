//! A set of choices, one checked at a time (reka's `RadioGroupRoot`,
//! `RadioGroupItem`, `RadioGroupIndicator`; research §10 "RadioGroup").
//!
//! - **Value.** `Option<T>`: the checked item's value, or none (the
//!   default, `RadioGroupRoot.vue:68-71`). The user checks an item (click,
//!   Space, an arrow) and cannot uncheck one: a click on the checked item
//!   keeps it (`Radio.vue:69`). The caller writes any value, none included.
//!   A caller keeping the value vetoes a check by not writing it back (reka's
//!   cancelable `select` event, `Radio.vue:64-67`): one way, the model's.
//! - **Keys** ([`Keymap::RADIO_GROUP`]: both axes unless an orientation is
//!   set, wrapping by default):
//!
//!   | Key | Result |
//!   | --- | --- |
//!   | Tab | Enters at the checked item, else the first usable one; does not check. |
//!   | Space | Checks the focused item. |
//!   | Enter | Nothing (`RadioGroupItem.vue:117`). |
//!   | ArrowDown, ArrowRight | Focuses the next item **and checks it**. |
//!   | ArrowUp, ArrowLeft | Focuses the previous item **and checks it**. |
//!   | Home/PageUp, End/PageDown | Focuses the first or last item, **without checking** (`RadioGroupItem.vue:62`). |
//!
//!   Right to left swaps ArrowLeft and ArrowRight. Disabled items are
//!   skipped; an arrow with Control, Alt, Command or Shift does nothing. The
//!   check follows the key at once, so the item that ends up focused after
//!   fast arrows is the one checked, which reka gets by clicking in a
//!   `setTimeout` after the focus lands (`RadioGroupItem.vue:61-94`).
//! - **ARIA.** The root is a `radiogroup` with `aria-orientation` only when
//!   one is set and `aria-required` when required (`RadioGroupRoot.vue:99-105`);
//!   items are `radio`s with `aria-checked`, named by their
//!   label, never by their value (`Radio.vue:58`).
//! - **Indicator.** Shown while its item is checked
//!   ([`RadioGroup::checked`]); decoration.
//! - **Form.** `name=value`, `name=` while nothing is checked; a disabled
//!   group sends nothing (`RadioGroupRoot.vue:109-115`). `Option<String>`'s
//!   [`crate::form::Submit`] encodes it so.

use gui::{Axis, Div, FocusHandle, Role, Signal, UseModel};

use crate::availability::Availability;
use crate::collection::Item;
use crate::direction::ReadDirection;
use crate::keymap::{Arrows, Intent, Keymap, Wrap};
use crate::roving::{Nav, Roving};

/// A radio group: made once in its root's setup, `Copy`.
pub struct RadioGroup<P: 'static, T: 'static> {
    value: UseModel<P, Option<T>>,
    roving: Roving<T>,
    /// The keymap the arrows are read by, the roving group's own: which
    /// keys check depends on the intent they have.
    keymap: Signal<Keymap>,
    direction: ReadDirection,
}

impl<P: 'static, T: 'static> Clone for RadioGroup<P, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static, T: 'static> Copy for RadioGroup<P, T> {}

impl<P: 'static, T: 'static> std::fmt::Debug for RadioGroup<P, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RadioGroup").finish_non_exhaustive()
    }
}

impl<P: 'static, T: Clone + PartialEq + 'static> RadioGroup<P, T> {
    /// A group writing `value` (its `v-model`), navigated in `direction`;
    /// called in setup.
    #[must_use]
    #[track_caller]
    pub fn new(value: UseModel<P, Option<T>>, direction: impl Into<ReadDirection>) -> Self {
        let direction = direction.into();
        Self {
            value,
            roving: Roving::new(Keymap::RADIO_GROUP, direction),
            keymap: Signal::labeled("radio group keymap", Keymap::RADIO_GROUP),
            direction,
        }
    }

    /// Take the arrows of `orientation` only (none: both axes), wrapping as
    /// `wrap` says (reka's `orientation` and `loop`).
    pub fn set_navigation(self, orientation: Option<Axis>, wrap: Wrap) {
        let arrows = match orientation {
            None => Arrows::Both,
            Some(Axis::Horizontal) => Arrows::Horizontal,
            Some(Axis::Vertical) => Arrows::Vertical,
        };
        let keymap = Keymap::RADIO_GROUP.arrows(arrows).wrap(wrap);
        self.keymap.set(keymap);
        self.roving.set_keymap(keymap);
    }

    /// The checked value (tracked).
    #[must_use]
    pub fn value(self) -> Option<T> {
        self.value.get()
    }

    /// Whether `value` is checked (tracked).
    #[must_use]
    pub fn checked(self, value: &T) -> bool {
        self.value.with(|v| v.as_ref() == Some(value))
    }

    /// Check `value`, as a click on its item does.
    pub fn check(self, value: T) {
        self.value.set(Some(value));
    }

    /// `d` as the group's root: a `radiogroup`, its orientation told when
    /// one is set, `required` or not.
    #[must_use]
    pub fn root(self, d: Div, orientation: Option<Axis>, required: bool) -> Div {
        let d = d.role(Role::RadioGroup).aria_required(required);
        match orientation {
            Some(o) => d.aria_orientation(o),
            None => d,
        }
    }

    /// `d` as the item of `value`, focused through `focus`, usable as
    /// `availability` says (the group's and its own, the more
    /// restrictive): a `radio`, checked while the group holds its value,
    /// one of the group's Tab stops, with the keys above. It sets the
    /// node's key handler.
    #[must_use]
    pub fn item(self, d: Div, value: T, focus: FocusHandle, availability: Availability) -> Div {
        let checked = self.checked(&value);
        let item = Item::new(value.clone(), focus)
            .active(checked)
            .availability(availability);
        let d = d.role(Role::RadioButton).aria_checked(checked);
        // Its own keys, not the group's plain roving (handlers add up).
        self.roving
            .member(d, item)
            .on_key_down(move |ev| {
                // WAI-ARIA: Enter does not check a radio, only Space does.
                if ev.is(gui::NamedKey::Enter) {
                    return true;
                }
                let intent = self.keymap.get_untracked().intent(ev, self.direction.get());
                match self.roving.key(focus, ev) {
                    Nav::Moved(to) => {
                        // The four arrows check; Home, End and the page keys only focus.
                        if matches!(intent, Some(Intent::Prev | Intent::Next)) {
                            self.check(to);
                        }
                        true
                    }
                    Nav::Stayed => true,
                    Nav::Ignored => false,
                }
            })
            .on_click(move |_| {
                if availability.actionable() {
                    self.check(value.clone());
                }
            })
    }
}
