//! A code typed one character per slot (reka's `PinInputRoot`,
//! `PinInputInput`; research §10 "PinInput"), and the rules an OTP input
//! shares with it ([`crate::input_otp`]).
//!
//! - **Value.** One entry per position: a character (a grapheme, so an
//!   emoji or a letter with its accent is one) or none. A slot is bound to
//!   its position; a slot not shown keeps its position's value, and "full"
//!   and the form's value read the slots shown only
//!   (`PinInputRoot.vue:104-122`). Empty entries at the end are dropped,
//!   for both kinds: reka drops them for a text pin only (its check reads
//!   `''`, not a number pin's `undefined`; ADR-0025 group 6 item 15).
//! - **Typing** ([`PinInput::slot`]): a character replaces the slot's,
//!   wherever its caret, and focus goes to the next slot; on the last slot
//!   it replaces the last character (`PinInputInput.vue:89-92`). A number
//!   pin takes ASCII digits only; any other key leaves the slot as it is.
//! - **Input methods** ([`PinInput::composing_at`]): a slot takes an input
//!   method's composition, shown in it while it lasts, its keys left to
//!   the method; what it commits is typed there as below, so committed
//!   characters spread over the slots.
//! - **Several characters at once** (a paste, an autofill, an input
//!   method's commit; [`spread`]): a number pin drops every non-digit first
//!   (`a1b2c3` → `123`); as many characters as slots or more fill from the
//!   first slot, fewer from the focused one; the rest is cut; focus goes to
//!   the slot after the last one filled, or stays on the last. Disabled
//!   slots are passed over: reka writes a paste into them too
//!   (`PinInputInput.vue:216-239`; research §4 item 13). Characters are
//!   split by grapheme, not by UTF-16 unit as reka's (research §10).
//! - **Keys** (slots in tree order, disabled ones skipped):
//!
//!   | Key | Result |
//!   | --- | --- |
//!   | Backspace | A filled slot is cleared and keeps focus; an empty one moves back and clears that one. |
//!   | Delete | Clears the slot; nothing shifts. |
//!   | ArrowLeft / ArrowRight | The slot before / after, no wrap; swapped right to left. |
//!   | Home / End | The first / last slot. |
//!   | ArrowUp / ArrowDown | Not the pin's. |
//!   | Tab | Each slot is a Tab stop. |
//!
//! - **OTP** ([`PinConfig::otp`], `PinInputInput.vue:153-197`): no slot after
//!   the first empty one can be reached; a click there, or focus coming
//!   from outside the pin, goes to the first empty slot, and the arrows and
//!   End stop at it. Tab between slots is not redirected: nothing traps the
//!   keyboard.
//! - **Full** ([`PinInput::set_on_complete`]): every slot shown has a
//!   character; the caller hears the characters at each change while full
//!   (`PinInputRoot.vue:123-132`).
//! - **ARIA.** Each slot is a text input named by its place, "pin input i
//!   of n" in English by default ([`PinNames`]), its character its value
//!   (a bullet when masked); a slot's placeholder shows only
//!   while it is empty and not focused ([`PinInput::placeholder_shown`]).
//! - **Form** ([`PinInput`] is a [`FormValue`], its value [`PinCode`]):
//!   the characters of the slots shown, joined ([`joined`]; `[1, ∅, 3]`
//!   sends `13`); nothing while disabled
//!   (`FormField::availability(move || pin.config().availability)`). A
//!   reset puts the positions' values back. A label's press focuses the
//!   first slot ([`PinInput::focus_first`]).

use gui::{Direction, Div, FocusHandle, Key, KeyEvent, NamedKey, Role, Signal, UseModel, watch};
use unicode_segmentation::UnicodeSegmentation;

use crate::availability::Availability;
use crate::collection::{Collection, Item};
use crate::direction::ReadDirection;
use crate::form::{FormData, FormValue, Submit};

/// Which characters a slot takes (reka's `type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum PinKind {
    /// Any character, spaces too.
    #[default]
    Text,
    /// ASCII digits only.
    Number,
}

impl PinKind {
    /// Whether a slot of this kind takes `grapheme`.
    #[must_use]
    pub fn takes(self, grapheme: &str) -> bool {
        match self {
            Self::Text => !grapheme.is_empty() && !grapheme.chars().all(char::is_control),
            Self::Number => grapheme.len() == 1 && grapheme.chars().all(|c| c.is_ascii_digit()),
        }
    }
}

/// A pin's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PinConfig {
    pub kind: PinKind,
    /// Slots are filled in order (one-time codes; shadcn's default for its
    /// pin input).
    pub otp: bool,
    /// The look masks the characters.
    pub mask: bool,
    pub availability: Availability,
}

/// What a screen reader names each slot by: reka's English string by
/// default (`PinInputInput.vue:298`).
#[derive(Clone, Copy, Debug)]
pub struct PinNames {
    /// Slot `i` (from 1) of `n`.
    pub slot: fn(usize, usize) -> String,
}

impl PartialEq for PinNames {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::fn_addr_eq(self.slot, other.slot)
    }
}

impl Default for PinNames {
    fn default() -> Self {
        Self {
            slot: |i, n| format!("pin input {i} of {n}"),
        }
    }
}

/// The characters of `text`, as a pin of `kind` reads several at once.
#[must_use]
pub fn graphemes(text: &str, kind: PinKind) -> Vec<String> {
    text.graphemes(true)
        .filter(|g| kind.takes(g))
        .map(str::to_owned)
        .collect()
}

/// `values` without the empty entries at its end.
fn trimmed(mut values: Vec<Option<String>>) -> Vec<Option<String>> {
    while values.last().is_some_and(Option::is_none) {
        values.pop();
    }
    values
}

/// `values` with `slot` set to `c`.
fn written(values: &[Option<String>], slot: usize, c: Option<String>) -> Vec<Option<String>> {
    let mut next = values.to_vec();
    if next.len() <= slot {
        next.resize(slot.saturating_add(1), None);
    }
    if let Some(v) = next.get_mut(slot) {
        *v = c;
    }
    trimmed(next)
}

/// Several characters `typed` at once into the slots `order` (positions in
/// tree order, only those usable), from the one at `at` in that order:
/// the values, and where in `order` focus goes.
#[must_use]
pub fn spread(
    values: &[Option<String>],
    order: &[usize],
    at: usize,
    typed: &str,
    kind: PinKind,
) -> (Vec<Option<String>>, usize) {
    let chars = graphemes(typed, kind);
    let from = if chars.len() >= order.len() { 0 } else { at };
    let mut next = values.to_vec();
    let mut last = from;
    for (slot, c) in order.iter().skip(from).zip(chars) {
        next = written(&next, *slot, Some(c));
        last = last.saturating_add(1);
    }
    let focus = last.min(order.len().saturating_sub(1));
    (trimmed(next), focus)
}

/// The characters of the slots `shown`, joined, holes left out: what a form
/// sends (`PinInputRoot.vue:167`).
#[must_use]
pub fn joined(values: &[Option<String>], shown: &[usize]) -> String {
    shown.iter().filter_map(|i| values.get(*i).cloned().flatten()).collect()
}

/// Whether every slot `shown` has a character.
#[must_use]
pub fn full(values: &[Option<String>], shown: &[usize]) -> bool {
    !shown.is_empty() && shown.iter().all(|i| values.get(*i).is_some_and(Option::is_some))
}

/// A pin's value as a form reads it: one entry per position, and the
/// positions of the slots shown, which are what it sends.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PinCode {
    pub values: Vec<Option<String>>,
    pub shown: Vec<usize>,
}

impl PinCode {
    /// The characters of the slots shown, joined: what a form sends and a
    /// rule checks.
    #[must_use]
    pub fn text(&self) -> String {
        joined(&self.values, &self.shown)
    }

    /// Whether every slot shown has a character.
    #[must_use]
    pub fn full(&self) -> bool {
        full(&self.values, &self.shown)
    }
}

impl Submit for PinCode {
    fn form_data(&self) -> FormData {
        FormData::Text(self.text())
    }
}

/// A form reads, checks and resets the positions' values; it focuses no
/// one slot.
impl<P: 'static> FormValue for PinInput<P> {
    type Value = PinCode;

    fn value(&self) -> PinCode {
        PinCode {
            values: self.value.get(),
            // Tracked: the slots shown as last laid out.
            shown: self.laid.get(),
        }
    }

    /// The values go back; which slots are shown is the look's.
    fn set_value(&self, code: PinCode) {
        self.value.set(code.values);
    }
}

type OnComplete = Option<gui::Callback<Vec<String>>>;

/// A pin input: made once in its root's setup, `Copy`.
pub struct PinInput<P: 'static> {
    value: UseModel<P, Vec<Option<String>>>,
    /// The slots mounted, by position, in tree order as last laid out.
    slots: Signal<Collection<usize>>,
    /// The positions of the slots shown, copied from the last layout so
    /// the slots' names ("i of n") follow it.
    laid: Signal<Vec<usize>>,
    config: Signal<PinConfig>,
    direction: ReadDirection,
    on_complete: Signal<OnComplete>,
    names: Signal<PinNames>,
    /// What an input method composes, and at which position.
    composing: Signal<Option<(usize, String)>>,
}

impl<P: 'static> Clone for PinInput<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for PinInput<P> {}

impl<P: 'static> std::fmt::Debug for PinInput<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinInput").finish_non_exhaustive()
    }
}

impl<P: 'static> PinInput<P> {
    /// A pin writing `value` (its `v-model`) by `config`, its arrows read
    /// in `direction`; called in setup.
    #[must_use]
    #[track_caller]
    pub fn new(
        value: UseModel<P, Vec<Option<String>>>,
        config: PinConfig,
        direction: impl Into<ReadDirection>,
    ) -> Self {
        let pin = Self {
            value,
            slots: Signal::new(Collection::new()),
            laid: Signal::labeled("pin input slots laid out", Vec::new()),
            config: Signal::labeled("pin input config", config),
            direction: direction.into(),
            on_complete: Signal::new(None),
            names: Signal::labeled("pin input names", PinNames::default()),
            composing: Signal::labeled("pin input composition", None),
        };
        // Heard at each change while full.
        watch(
            move || value.get(),
            move |values, _| {
                let shown = pin.shown();
                if full(values, &shown)
                    && let Some(f) = pin.on_complete.get_untracked()
                {
                    f.call(shown.iter().filter_map(|i| values.get(*i).cloned().flatten()).collect());
                }
            },
        );
        // A one-time code: focus coming from outside goes to the first empty slot.
        watch(gui::focus::focused, move |now, before| {
            let Some(now) = *now else { return };
            let came_in = before.is_none_or(|b| pin.position_of(b).is_none());
            if pin.config.get_untracked().otp && came_in {
                pin.hold_to_first_empty(now);
            }
        });
        pin
    }

    /// Its settings from now on.
    pub fn set_config(self, config: PinConfig) {
        self.config.set(config);
    }

    /// Name the slots by `names` from now on.
    pub fn set_names(self, names: PinNames) {
        self.names.set(names);
    }

    /// Call `f` with the characters at each change while every slot shown
    /// has one (`@complete`).
    pub fn set_on_complete(self, f: impl Into<gui::Callback<Vec<String>>>) {
        let f: gui::Callback<Vec<String>> = f.into();
        self.on_complete.update(|c| *c = Some(f));
    }

    /// The slots mounted, as last laid out.
    fn slots(self) -> Collection<usize> {
        self.slots.with_untracked(Clone::clone)
    }

    /// Its settings (tracked).
    #[must_use]
    pub fn config(self) -> PinConfig {
        self.config.get()
    }

    /// The character at position `i` (tracked).
    #[must_use]
    pub fn char_at(self, i: usize) -> Option<String> {
        self.value.with(|v| v.get(i).cloned().flatten())
    }

    /// What an input method composes at position `i`, while it does
    /// (tracked): the look shows it in the slot, in place of its character.
    #[must_use]
    pub fn composing_at(self, i: usize) -> Option<String> {
        self.composing
            .with(|c| c.as_ref().filter(|(at, _)| *at == i).map(|(_, t)| t.clone()))
    }

    /// What an input method does on the slot of position `i`: its
    /// composition is shown there; what it commits is typed, one character
    /// or several spread, as keys and a paste are.
    fn ime(self, i: usize, ev: &gui::ImeEvent) {
        match ev {
            gui::ImeEvent::Preedit(p) if p.text.is_empty() => self.composing.set(None),
            gui::ImeEvent::Preedit(p) => {
                if self.config.get_untracked().availability.actionable() {
                    self.composing.set(Some((i, p.text.clone())));
                }
            }
            gui::ImeEvent::Commit(text) => {
                self.composing.set(None);
                if self.config.get_untracked().availability.actionable() {
                    self.type_text(i, text);
                }
            }
        }
    }

    /// The positions of the slots shown, in tree order (as last laid out).
    #[must_use]
    pub fn shown(self) -> Vec<usize> {
        self.slots().with(|s| s.iter().map(|i| i.value).collect())
    }

    /// The usable slots' positions and focus, in tree order.
    fn order(self) -> Vec<(usize, FocusHandle)> {
        self.slots().with(|s| {
            s.iter()
                .filter(|i| i.availability.focusable())
                .map(|i| (i.value, i.focus))
                .collect()
        })
    }

    fn position_of(self, focus: gui::focus::FocusId) -> Option<usize> {
        self.slots().with(|s| s.iter().position(|i| i.focus.id() == focus))
    }

    /// Where in the usable order the first empty slot is (the last when
    /// all are full).
    fn first_empty(self, order: &[(usize, FocusHandle)]) -> usize {
        let values = self.value.get_untracked();
        order
            .iter()
            .position(|(i, _)| values.get(*i).is_none_or(Option::is_none))
            .unwrap_or(order.len().saturating_sub(1))
    }

    /// In a one-time code, focus on a slot past the first empty one goes
    /// back to it.
    fn hold_to_first_empty(self, now: gui::focus::FocusId) {
        let order = self.order();
        let Some(at) = order.iter().position(|(_, f)| f.id() == now) else {
            return;
        };
        let first = self.first_empty(&order);
        if at > first
            && let Some((_, f)) = order.get(first)
        {
            f.focus();
        }
    }

    /// Focus the first slot, as a label's press does.
    pub fn focus_first(self) {
        if let Some((_, f)) = self.order().first() {
            f.focus();
        }
    }

    /// Whether slot `i`'s placeholder shows: empty and not focused (tracked).
    #[must_use]
    pub fn placeholder_shown(self, i: usize, focus: FocusHandle) -> bool {
        self.char_at(i).is_none() && !focus.is_focused()
    }

    /// Where slot position `i` is among the slots shown (from 1), and how
    /// many are shown (tracked: the slots read again once a layout changes
    /// them).
    fn place(self, i: usize) -> (usize, usize) {
        // Read the slots once they are laid out; the write renders the names
        // again in the same frame, only when the slots changed.
        gui::after_layout(move || self.laid.set(self.shown()));
        let shown = self.laid.get();
        let at = shown.iter().position(|s| *s == i).unwrap_or(i);
        (at.saturating_add(1), shown.len().max(i.saturating_add(1)))
    }

    fn focus_in_order(order: &[(usize, FocusHandle)], at: usize) {
        if let Some((_, f)) = order.get(at) {
            f.focus();
        }
    }

    /// What a key on the slot of position `i` does; whether it was the pin's.
    fn key(self, i: usize, ev: &KeyEvent) -> bool {
        let config = self.config.get_untracked();
        if !config.availability.actionable() {
            return false;
        }
        // While an input method composes, its keys are its own.
        if self.composing.with_untracked(Option::is_some) {
            return true;
        }
        let order = self.order();
        let Some(at) = order.iter().position(|(p, _)| *p == i) else {
            return false;
        };
        let last = order.len().saturating_sub(1);
        // In a one-time code, nothing past the first empty slot is reached.
        let reach = if config.otp { self.first_empty(&order) } else { last };
        let rtl = self.direction.get() == Direction::Rtl;
        let m = ev.modifiers;
        match &ev.key {
            Key::Named(NamedKey::Backspace) => {
                if self.char_at(i).is_some() {
                    self.value.update(|v| *v = written(v, i, None));
                } else if let Some(prev) = at.checked_sub(1)
                    && let Some((p, f)) = order.get(prev)
                {
                    let p = *p;
                    self.value.update(|v| *v = written(v, p, None));
                    f.focus();
                }
                true
            }
            Key::Named(NamedKey::Delete) => {
                self.value.update(|v| *v = written(v, i, None));
                true
            }
            Key::Named(k @ (NamedKey::ArrowLeft | NamedKey::ArrowRight)) => {
                let forward = (*k == NamedKey::ArrowRight) != rtl;
                let to = if forward {
                    at.saturating_add(1).min(reach.max(at))
                } else {
                    at.saturating_sub(1)
                };
                Self::focus_in_order(&order, to.min(last));
                true
            }
            Key::Named(NamedKey::Home) => {
                Self::focus_in_order(&order, 0);
                true
            }
            Key::Named(NamedKey::End) => {
                Self::focus_in_order(&order, reach);
                true
            }
            Key::Character(c) if m.primary() && c.eq_ignore_ascii_case("v") => {
                self.type_text(i, &gui::clipboard::paste().unwrap_or_default());
                true
            }
            _ if m.primary() || m.control || m.alt => false,
            _ => match ev.text.as_deref() {
                Some(typed) if !typed.chars().all(char::is_control) => {
                    self.type_text(i, typed);
                    true
                }
                _ => false,
            },
        }
    }

    /// `typed` at the slot of position `i`: one character replaces the
    /// slot's and moves on, several are spread.
    pub fn type_text(self, i: usize, typed: &str) {
        let config = self.config.get_untracked();
        let order = self.order();
        let Some(at) = order.iter().position(|(p, _)| *p == i) else {
            return;
        };
        let chars = graphemes(typed, config.kind);
        match chars.as_slice() {
            [] => {}
            [one] if typed.graphemes(true).count() == 1 => {
                let one = one.clone();
                self.value.update(|v| *v = written(v, i, Some(one)));
                Self::focus_in_order(&order, at.saturating_add(1).min(order.len().saturating_sub(1)));
            }
            _ => {
                let positions: Vec<usize> = order.iter().map(|(p, _)| *p).collect();
                let (next, focus) = spread(&self.value.get_untracked(), &positions, at, typed, config.kind);
                self.value.set(next);
                Self::focus_in_order(&order, focus);
            }
        }
    }

    /// `d` as the slot of position `i`, focused through `focus`, usable as
    /// `availability` says (the pin's and its own, the more restrictive):
    /// a Tab stop named by its place, taking the keys above, held to the
    /// first empty slot in a one-time code. It sets the node's key handler.
    #[must_use]
    pub fn slot(self, d: Div, i: usize, focus: FocusHandle, availability: Availability) -> Div {
        let (place, count) = self.place(i);
        let name = self.names.with(|n| (n.slot)(place, count));
        // Its character is its value to a screen reader, masked as shown.
        let mask = self.config.with(|c| c.mask);
        let held = self
            .char_at(i)
            .map(|c| if mask { "\u{2022}".to_owned() } else { c })
            .unwrap_or_default();
        let d = d.role(Role::TextInput).aria_label(name).aria_value(held);
        self.slots()
            .item(d, Item::new(i, focus).availability(availability))
            .on_key_down(move |ev| self.key(i, ev))
            .on_ime(move |ev| self.ime(i, ev))
            .on_mouse_down(move |_| {
                if self.config.get_untracked().otp {
                    self.hold_to_first_empty(focus.id());
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{PinKind, full, graphemes, joined, spread};

    fn some(s: &str) -> Option<String> {
        Some(s.to_owned())
    }

    #[test]
    fn several_characters_spread_by_grapheme() {
        assert_eq!(graphemes("a1b2c3", PinKind::Number), ["1", "2", "3"]);
        assert_eq!(
            graphemes("e\u{301}👍🏽x", PinKind::Text).len(),
            3,
            "graphemes, not code units"
        );
        let order = [0, 1, 2, 3];
        // As long as the slots or longer: from the first, the rest cut.
        let (v, focus) = spread(&[], &order, 2, "123456", PinKind::Text);
        assert_eq!(v, [some("1"), some("2"), some("3"), some("4")]);
        assert_eq!(focus, 3, "the last slot");
        // Shorter: from the focused slot, focus after the last filled.
        let (v, focus) = spread(&[some("a")], &order, 1, "xy", PinKind::Text);
        assert_eq!(v, [some("a"), some("x"), some("y")]);
        assert_eq!(focus, 3);
        // Disabled slots are not in the order, so nothing is written there.
        let (v, _) = spread(&[], &[0, 2, 3], 0, "123", PinKind::Number);
        assert_eq!(v, [some("1"), None, some("2"), some("3")]);
    }

    #[test]
    fn the_form_joins_the_shown_slots_and_full_reads_them() {
        let v = [some("1"), None, some("3"), some("9")];
        assert_eq!(joined(&v, &[0, 1, 2]), "13", "holes left out");
        assert!(!full(&v, &[0, 1, 2]));
        assert!(full(&v, &[0, 2]), "a slot not shown keeps its value but is not counted");
        assert!(!full(&v, &[]));
    }
}
