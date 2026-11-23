//! Arrow navigation: one rule for which item a key reaches (reka's
//! `RovingFocus/utils.ts` and `shared/useArrowNavigation.ts`, made one;
//! ADR-0025, research §4 item 6).
//!
//! A [`Keymap`] is data: the arrows a primitive takes and whether it wraps.
//! The table, the same for every primitive:
//!
//! | Key | Intent |
//! | --- | --- |
//! | ArrowUp, ArrowLeft | [`Intent::Prev`] |
//! | ArrowDown, ArrowRight | [`Intent::Next`] |
//! | Home, PageUp | [`Intent::First`] |
//! | End, PageDown | [`Intent::Last`] |
//!
//! - Right to left swaps ArrowLeft and ArrowRight before the table is read;
//!   Up, Down, Home, End and the page keys do not turn.
//! - [`Arrows::Horizontal`] leaves ArrowUp and ArrowDown alone, and
//!   [`Arrows::Vertical`] leaves ArrowLeft and ArrowRight alone.
//! - Control, Alt or Command with any of them is no navigation; Shift is
//!   none unless the keymap takes it (a list selecting a range).
//! - Every step lands on a focusable item ([`crate::Availability`]):
//!   Home and End too, which reka's `useArrowNavigation` does not skip.
//!   From no item, Next reaches the first and Prev the last.
//! - Past an end, [`Wrap::Stop`] stays and [`Wrap::Around`] goes on from
//!   the other end.
//!
//! reka disagrees with itself on two points: RovingFocus takes PageUp and
//! PageDown and does not wrap by default; `useArrowNavigation` takes no page
//! keys and wraps. Here the page keys are First and Last everywhere, and
//! each primitive names its wrap: the keymaps below, as reka's primitives
//! set it.

use gui::Direction;
use gui::events::{KeyEvent, NamedKey};

use crate::collection::Items;

/// The arrows a primitive takes (reka's `orientation`, unset meaning both).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Arrows {
    Horizontal,
    Vertical,
    Both,
}

/// What happens past the first or the last item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Wrap {
    /// Stay on it.
    Stop,
    /// Go on from the other end (reka's `loop`).
    Around,
}

/// Where a key asks focus to go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Intent {
    Prev,
    Next,
    First,
    Last,
}

/// A primitive's arrow keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Keymap {
    pub arrows: Arrows,
    pub wrap: Wrap,
    /// Whether Shift with a navigation key still navigates (a list
    /// extending its selection; reka's `allowShiftKey`).
    pub shift: bool,
}

impl Keymap {
    /// reka's RovingFocusGroup: both axes, no wrap.
    pub const ROVING: Self = Self::new(Arrows::Both, Wrap::Stop);
    /// Tabs (reka's TabsList): wraps; the axis is the list's orientation.
    pub const TABS: Self = Self::new(Arrows::Horizontal, Wrap::Around);
    /// RadioGroup: both axes, wraps.
    pub const RADIO_GROUP: Self = Self::new(Arrows::Both, Wrap::Around);
    /// ToggleGroup: both axes, wraps.
    pub const TOGGLE_GROUP: Self = Self::new(Arrows::Both, Wrap::Around);
    /// Toolbar: its orientation, no wrap (reka's `loop` is off).
    pub const TOOLBAR: Self = Self::new(Arrows::Horizontal, Wrap::Stop);
    /// Menubar: across its menus, no wrap.
    pub const MENUBAR: Self = Self::new(Arrows::Horizontal, Wrap::Stop);
    /// A menu's items (MenuContent), Select's and Listbox's options: down
    /// the list, no wrap.
    pub const LIST: Self = Self::new(Arrows::Vertical, Wrap::Stop);
    /// Accordion's triggers (reka's `useArrowNavigation` default): wraps.
    pub const ACCORDION: Self = Self::new(Arrows::Vertical, Wrap::Around);
    /// NavigationMenu, PinInput, Stepper, TagsInput: their axis, no wrap.
    pub const LINE: Self = Self::new(Arrows::Horizontal, Wrap::Stop);

    /// A keymap of `arrows` that wraps as `wrap` says and ignores Shift.
    #[must_use]
    pub const fn new(arrows: Arrows, wrap: Wrap) -> Self {
        Self {
            arrows,
            wrap,
            shift: false,
        }
    }

    /// This keymap on `arrows` instead (an orientation the caller set).
    #[must_use]
    pub const fn arrows(self, arrows: Arrows) -> Self {
        Self { arrows, ..self }
    }

    /// This keymap wrapping as `wrap` says (a caller's `loop`).
    #[must_use]
    pub const fn wrap(self, wrap: Wrap) -> Self {
        Self { wrap, ..self }
    }

    /// This keymap navigating with Shift held too.
    #[must_use]
    pub const fn with_shift(self, shift: bool) -> Self {
        Self { shift, ..self }
    }

    /// The intent of `ev` in `direction`, if it is a navigation key here.
    #[must_use]
    pub fn intent(&self, ev: &KeyEvent, direction: Direction) -> Option<Intent> {
        let m = ev.modifiers;
        if m.control || m.alt || m.command || (m.shift && !self.shift) {
            return None;
        }
        let rtl = direction == Direction::Rtl;
        let horizontal = !matches!(self.arrows, Arrows::Vertical);
        let vertical = !matches!(self.arrows, Arrows::Horizontal);
        let table = [
            (
                NamedKey::ArrowLeft,
                horizontal,
                if rtl { Intent::Next } else { Intent::Prev },
            ),
            (
                NamedKey::ArrowRight,
                horizontal,
                if rtl { Intent::Prev } else { Intent::Next },
            ),
            (NamedKey::ArrowUp, vertical, Intent::Prev),
            (NamedKey::ArrowDown, vertical, Intent::Next),
            (NamedKey::Home, true, Intent::First),
            (NamedKey::PageUp, true, Intent::First),
            (NamedKey::End, true, Intent::Last),
            (NamedKey::PageDown, true, Intent::Last),
        ];
        table
            .into_iter()
            .find_map(|(key, taken, intent)| (taken && ev.is(key)).then_some(intent))
    }

    /// Where `intent` leads in `items` from item `from` (none: no item has
    /// focus), as this keymap wraps; `None` when focus stays.
    #[must_use]
    pub fn step<I: Items + ?Sized>(&self, items: &I, from: Option<usize>, intent: Intent) -> Option<usize> {
        step(items, from, intent, self.wrap)
    }
}

/// Where `intent` leads in `items` from item `from`, skipping items that
/// take no focus; `None` when there is nowhere to go (focus stays): every
/// item is disabled, or an end was reached without wrapping, or the only
/// focusable item is `from`.
#[must_use]
pub fn step<I: Items + ?Sized>(items: &I, from: Option<usize>, intent: Intent, wrap: Wrap) -> Option<usize> {
    let len = items.len();
    let first = || (0..len).find(|&i| items.focusable(i));
    let last = || (0..len).rev().find(|&i| items.focusable(i));
    let from = from.filter(|&i| i < len);
    let to = match (intent, from) {
        (Intent::First, _) | (Intent::Next, None) => first(),
        (Intent::Last, _) | (Intent::Prev, None) => last(),
        (Intent::Next, Some(at)) => {
            let after = at.checked_add(1).unwrap_or(len);
            (after..len).find(|&i| items.focusable(i)).or_else(|| match wrap {
                Wrap::Around => (0..at).find(|&i| items.focusable(i)),
                Wrap::Stop => None,
            })
        }
        (Intent::Prev, Some(at)) => (0..at).rev().find(|&i| items.focusable(i)).or_else(|| match wrap {
            Wrap::Around => (at.saturating_add(1)..len).rev().find(|&i| items.focusable(i)),
            Wrap::Stop => None,
        }),
    };
    to.filter(|&t| Some(t) != from)
}

/// Enter or Space: what activates the focused item (a menu item, an
/// option). Space typed while a typeahead search runs is part of the search
/// instead ([`crate::typeahead`]).
#[must_use]
pub fn activates(ev: &KeyEvent) -> bool {
    ev.is(NamedKey::Enter) || ev.is(NamedKey::Space) || ev.text.as_deref() == Some(" ")
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use gui::Direction;
    use gui::events::{Key, KeyEvent, Modifiers, NamedKey};

    use super::{Arrows, Intent, Keymap, Wrap, step};
    use crate::availability::Availability;
    use crate::collection::from_fn;

    fn key(k: NamedKey, modifiers: Modifiers) -> KeyEvent {
        KeyEvent {
            key: Key::Named(k),
            text: None,
            modifiers,
        }
    }

    fn plain(k: NamedKey) -> KeyEvent {
        key(k, Modifiers::default())
    }

    #[test]
    fn the_key_table_with_its_axes_and_direction() {
        use Intent::{First, Last, Next, Prev};
        use NamedKey::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp, End, Home, PageDown, PageUp};
        let both = Keymap::ROVING;
        let across = Keymap::ROVING.arrows(Arrows::Horizontal);
        let down = Keymap::ROVING.arrows(Arrows::Vertical);
        // (key, both ltr, both rtl, horizontal, vertical)
        let table = [
            (ArrowLeft, Some(Prev), Some(Next), Some(Prev), None),
            (ArrowRight, Some(Next), Some(Prev), Some(Next), None),
            (ArrowUp, Some(Prev), Some(Prev), None, Some(Prev)),
            (ArrowDown, Some(Next), Some(Next), None, Some(Next)),
            (Home, Some(First), Some(First), Some(First), Some(First)),
            (PageUp, Some(First), Some(First), Some(First), Some(First)),
            (End, Some(Last), Some(Last), Some(Last), Some(Last)),
            (PageDown, Some(Last), Some(Last), Some(Last), Some(Last)),
        ];
        for (k, ltr, rtl, h, v) in table {
            let ev = plain(k);
            assert_eq!(both.intent(&ev, Direction::Ltr), ltr, "{k:?} ltr");
            assert_eq!(both.intent(&ev, Direction::Rtl), rtl, "{k:?} rtl");
            assert_eq!(across.intent(&ev, Direction::Ltr), h, "{k:?} horizontal");
            assert_eq!(down.intent(&ev, Direction::Ltr), v, "{k:?} vertical");
        }
        assert_eq!(both.intent(&plain(NamedKey::Enter), Direction::Ltr), None);
    }

    #[test]
    fn modifiers_stop_navigation_and_shift_only_where_taken() {
        let ev = |m: Modifiers| key(NamedKey::ArrowDown, m);
        for m in [
            Modifiers {
                control: true,
                ..Modifiers::default()
            },
            Modifiers {
                alt: true,
                ..Modifiers::default()
            },
            Modifiers {
                command: true,
                ..Modifiers::default()
            },
        ] {
            assert_eq!(Keymap::LIST.intent(&ev(m), Direction::Ltr), None);
            assert_eq!(Keymap::LIST.with_shift(true).intent(&ev(m), Direction::Ltr), None);
        }
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(Keymap::LIST.intent(&ev(shift), Direction::Ltr), None);
        assert_eq!(
            Keymap::LIST.with_shift(true).intent(&ev(shift), Direction::Ltr),
            Some(Intent::Next)
        );
    }

    #[test]
    fn steps_skip_disabled_stop_or_wrap_and_reach_the_ends() {
        // 0 a, 1 disabled, 2 c, 3 d, 4 disabled
        let items = from_fn(
            5,
            |i| Availability::from_disabled(i == 1 || i == 4),
            |_| Cow::Borrowed(""),
        );
        use Intent::{First, Last, Next, Prev};
        // (from, intent, stop, around)
        let table = [
            (Some(0), Next, Some(2), Some(2)),
            (Some(2), Prev, Some(0), Some(0)),
            (Some(3), Next, None, Some(0)),
            (Some(0), Prev, None, Some(3)),
            (None, Next, Some(0), Some(0)),
            (None, Prev, Some(3), Some(3)),
            (Some(2), First, Some(0), Some(0)),
            (Some(2), Last, Some(3), Some(3)),
            (Some(0), First, None, None),
            (Some(9), Next, Some(0), Some(0)),
        ];
        for (from, intent, stop, around) in table {
            assert_eq!(step(&items, from, intent, Wrap::Stop), stop, "{from:?} {intent:?} stop");
            assert_eq!(
                step(&items, from, intent, Wrap::Around),
                around,
                "{from:?} {intent:?} around"
            );
        }
        let none = from_fn(3, |_| Availability::Disabled, |_| Cow::Borrowed(""));
        for intent in [First, Last, Next, Prev] {
            assert_eq!(step(&none, Some(1), intent, Wrap::Around), None, "all disabled");
        }
        let one = from_fn(3, |i| Availability::from_disabled(i != 1), |_| Cow::Borrowed(""));
        assert_eq!(step(&one, Some(1), Next, Wrap::Around), None, "alone, it stays");
        // Unavailable items still take focus.
        let dim = from_fn(2, |_| Availability::Unavailable, |_| Cow::Borrowed(""));
        assert_eq!(step(&dim, Some(0), Next, Wrap::Stop), Some(1));
    }

    #[test]
    fn steps_run_on_the_data_of_a_virtual_list() {
        let rows = 2_000_000;
        let items = from_fn(
            rows,
            |i| Availability::from_disabled(i == rows - 1),
            |_| Cow::Borrowed(""),
        );
        assert_eq!(Keymap::LIST.step(&items, None, Intent::Last), Some(rows - 2));
        assert_eq!(Keymap::LIST.step(&items, Some(rows - 2), Intent::Next), None);
        assert_eq!(Keymap::ACCORDION.step(&items, Some(rows - 2), Intent::Next), Some(0));
    }
}
