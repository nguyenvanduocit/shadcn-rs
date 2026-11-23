//! Typing to reach an item (reka's `useTypeahead` and `getNextMatch`).
//!
//! - The search is what was typed since the last pause: each printable key
//!   adds its character, and **1000 ms** ([`RESET`]) after the last one the
//!   search is empty again. The frame clock times it (`gui::clock`).
//! - An item matches when its label starts with the search, case aside.
//! - The search runs forward from the current item and wraps past the end.
//! - The same character typed again and again ("aaa") searches as one
//!   ("a"), so each press moves on to the next item starting with it.
//! - A one-character search never matches the current item, so focus always
//!   moves on; a longer one keeps the current item while it still matches.
//! - No match, or only the current item, leaves focus where it is.
//! - Items whose labels repeat stay apart: they are told apart by index, on
//!   the whole list a virtual list holds as data, not only the rows mounted
//!   (reka's virtualised listbox matches by string, and always lands on the
//!   first of the repeats).
//! - Disabled items are never reached; unavailable ones are, as they take
//!   focus ([`crate::Availability`]).
//!
//! Only a printable key without Control, Alt or Command searches
//! ([`typed`]). Space searches while a search runs and activates otherwise.
//! That is reka's Select (`SelectContentImpl` adds Space to a running
//! search, `SelectItem` skips it then). reka's Menu differs: its items skip
//! Space while a search runs (`MenuItem`, `isTypingAhead`) but its content
//! returns on Space before the search (`MenuContentImpl`, "prevent Space
//! taken account into handleTypeahead"), so mid-search Space does nothing
//! at all there and "New Tab" cannot be typed. The one rule here is
//! Select's (ADR-0025). Keys typed in a text field inside a
//! list are the field's: the field consumes them before the list's handler
//! sees them.

use std::time::Duration;

use gui::clock::{self, TimerId};
use gui::{KeyEvent, Signal};

use crate::collection::Items;

/// How long after the last key the search empties.
pub const RESET: Duration = Duration::from_millis(1000);

/// A list's search: made once in setup, `Copy`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Typeahead {
    search: Signal<String>,
    timer: Signal<Option<TimerId>>,
}

impl Default for Typeahead {
    #[track_caller]
    fn default() -> Self {
        Self::new()
    }
}

impl Typeahead {
    #[must_use]
    #[track_caller]
    pub fn new() -> Self {
        Self {
            search: Signal::labeled("typeahead", String::new()),
            timer: Signal::new(None),
        }
    }

    /// The search so far (tracked).
    #[must_use]
    pub fn search(self) -> String {
        self.search.get()
    }

    /// Whether a search runs: something was typed less than [`RESET`] ago.
    #[must_use]
    pub fn is_searching(self) -> bool {
        self.search.with_untracked(|s| !s.is_empty())
    }

    /// Add `typed` to the search, which empties [`RESET`] after this.
    pub fn push(self, typed: &str) {
        self.search.update(|s| s.push_str(typed));
        if let Some(t) = self.timer.get_untracked() {
            clock::cancel(t);
        }
        let search = self.search;
        let timer = gui::untracked(|| {
            clock::after(RESET, move || {
                if search.is_alive() {
                    search.set(String::new());
                }
            })
        });
        self.timer.set(Some(timer));
    }

    /// Empty the search now (a menu closing, a select opening).
    pub fn reset(self) {
        if let Some(t) = self.timer.get_untracked() {
            clock::cancel(t);
        }
        self.timer.set(None);
        self.search.set(String::new());
    }

    /// Add `typed` to the search and find the item it reaches from
    /// `current`; `None` leaves focus where it is.
    #[must_use]
    pub fn find<I: Items + ?Sized>(self, typed: &str, items: &I, current: Option<usize>) -> Option<usize> {
        self.push(typed);
        self.search.with_untracked(|search| next_match(items, search, current))
    }

    /// The character `ev` adds to the search, if it is a search key: one
    /// printable character, without Control, Alt or Command; Space only
    /// while a search runs.
    #[must_use]
    pub fn typed(self, ev: &KeyEvent) -> Option<&str> {
        typed(ev, self.is_searching())
    }
}

/// The character `ev` adds to a search, if it is a search key; Space counts
/// only when `searching`.
#[must_use]
pub fn typed(ev: &KeyEvent, searching: bool) -> Option<&str> {
    let m = ev.modifiers;
    if m.control || m.alt || m.command {
        return None;
    }
    let text = ev.text.as_deref()?;
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() || c.is_control() {
        return None;
    }
    if c.is_whitespace() && !searching {
        return None;
    }
    Some(text)
}

/// Whether `label` starts with `search`, case aside (Unicode lowercase on
/// both sides, as reka's `toLowerCase`), without allocating.
fn starts_with(label: &str, search: &str) -> bool {
    let mut label = label.chars().flat_map(char::to_lowercase);
    search
        .chars()
        .flat_map(char::to_lowercase)
        .all(|s| label.next() == Some(s))
}

/// The item `search` reaches in `items` from `current` (reka's
/// `getNextMatch`): `None` when focus stays.
#[must_use]
pub fn next_match<I: Items + ?Sized>(items: &I, search: &str, current: Option<usize>) -> Option<usize> {
    let len = items.len();
    let mut chars = search.chars();
    let first = chars.next()?;
    // "aaa" searches as "a".
    let repeated = chars.all(|c| c == first);
    let mut buf = [0_u8; 4];
    let search: &str = if repeated { first.encode_utf8(&mut buf) } else { search };
    let single = repeated;
    let current = current.filter(|&c| c < len);
    let start = current.unwrap_or(0);
    let found = (0..len)
        .filter_map(|k| start.checked_add(k).and_then(|i| i.checked_rem(len)))
        .filter(|&i| !(single && Some(i) == current))
        .find(|&i| items.focusable(i) && starts_with(&items.label(i), search));
    found.filter(|&f| Some(f) != current)
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use gui::events::{Key, KeyEvent, Modifiers};

    use super::{next_match, typed};
    use crate::availability::Availability;
    use crate::collection::from_fn;

    fn labels(words: &'static [&'static str]) -> impl crate::Items + 'static {
        from_fn(
            words.len(),
            |_| Availability::Enabled,
            move |i| Cow::Borrowed(words.get(i).copied().unwrap_or("")),
        )
    }

    #[test]
    fn matching_follows_reka() {
        let items = labels(&["Apple", "apricot", "Banana", "Blueberry", "apple"]);
        // (search, current, found)
        let table = [
            ("a", None, Some(0)),
            ("a", Some(0), Some(1)),
            ("a", Some(1), Some(4)),
            ("a", Some(4), Some(0)),
            ("aa", Some(0), Some(1)),
            ("aaa", Some(1), Some(4)),
            ("ap", Some(0), None),
            ("apr", Some(0), Some(1)),
            ("APP", Some(1), Some(4)),
            ("b", Some(0), Some(2)),
            ("bl", Some(2), Some(3)),
            ("z", Some(0), None),
            ("", Some(0), None),
        ];
        for (search, current, found) in table {
            assert_eq!(
                next_match(&items, search, current),
                found,
                "{search:?} from {current:?}"
            );
        }
    }

    #[test]
    fn one_character_from_nothing_finds_the_first_and_never_stays() {
        let items = labels(&["Cat", "Dog"]);
        assert_eq!(next_match(&items, "c", None), Some(0), "from no item, the first match");
        assert_eq!(
            next_match(&items, "c", Some(0)),
            None,
            "only the current matches: stays"
        );
    }

    #[test]
    fn repeated_labels_stay_apart_and_disabled_items_are_passed() {
        let items = from_fn(
            1_000_000,
            |i| Availability::from_disabled(i == 2),
            |i| Cow::Borrowed(if i % 250_000 == 0 || i == 2 { "same" } else { "other" }),
        );
        assert_eq!(next_match(&items, "s", Some(0)), Some(250_000), "row 2 is disabled");
        assert_eq!(next_match(&items, "s", Some(250_000)), Some(500_000));
        assert_eq!(next_match(&items, "s", Some(750_000)), Some(0), "wraps by index");
    }

    #[test]
    fn search_keys_are_printable_and_unmodified() {
        let ev = |t: &str, m: Modifiers| KeyEvent {
            key: Key::Character(t.into()),
            text: Some(t.to_owned()),
            modifiers: m,
        };
        let none = Modifiers::default();
        assert_eq!(typed(&ev("a", none), false), Some("a"));
        assert_eq!(typed(&ev("é", none), false), Some("é"));
        assert_eq!(typed(&ev(" ", none), false), None, "Space activates");
        assert_eq!(typed(&ev(" ", none), true), Some(" "), "Space searches mid-search");
        let control = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        assert_eq!(typed(&ev("a", control), false), None);
        assert_eq!(typed(&ev("\u{7f}", none), false), None);
        assert_eq!(typed(&ev("ab", none), false), None);
    }
}
