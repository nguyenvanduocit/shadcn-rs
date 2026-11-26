//! An autocomplete (reka's `Autocomplete`): a combobox whose value is the
//! text in its field (research §9, Autocomplete). It is a [`Combobox`] in
//! its text-value mode ([`Combobox::set_text_value`]), as reka's reuses
//! the combobox's parts: the field, the list, the trigger, the cancel
//! button, the keys, the filter, the popup and its closing are the
//! combobox's ([`Autocomplete::combobox`]). Only what differs is here.
//!
//! - **The value is the field's text**, empty by default. Typing writes it
//!   at once; a caller's write shows in the field.
//! - **Choosing an option** writes the option's value as text: the value,
//!   not the label it shows (`AutocompleteRoot.vue:111-121`). It closes the
//!   list; the text stays (no reset on close or on choosing, since the text
//!   is the value: [`ComboboxConfig::AUTOCOMPLETE`]).
//! - **Opening** filters by the text as it stands (a combobox empties its
//!   term) and highlights the option valued as the text, else the first;
//!   closing empties the term.
//! - Typing highlights the first match, so Enter then puts that option's
//!   value in the field; with no match nothing is highlighted, the field
//!   names no active descendant, and Enter is the form's.
//! - One value only; the field takes focus as the list opens; cancel
//!   empties the value. The combobox's own value is the text too (`None`
//!   while it is empty), so [`Autocomplete::combobox`]'s value and form
//!   value say what the field holds.

use gui::TextField;

use crate::combobox::{Combobox, ComboboxConfig};
use crate::listbox::{Bound, Held};

/// An autocomplete: made once in its root's setup, `Copy`.
#[derive(Clone, Copy, PartialEq)]
pub struct Autocomplete {
    combobox: Combobox<String, Option<String>>,
    value: Bound<String>,
}

impl std::fmt::Debug for Autocomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Autocomplete").finish_non_exhaustive()
    }
}

impl Autocomplete {
    /// An autocomplete whose text is `value`, open as `open` says, typed in
    /// `field`; made in its root's setup.
    #[must_use]
    #[track_caller]
    pub fn new(value: Bound<String>, open: Bound<bool>, field: TextField) -> Self {
        let combobox = Combobox::new(
            Bound::new(TextAsChoice(value)),
            open,
            field,
            ComboboxConfig::AUTOCOMPLETE,
            |v: &Option<String>| v.clone().unwrap_or_default(),
        );
        combobox.set_text_value(value, String::clone);
        Self { combobox, value }
    }

    /// The combobox it is: its parts (`input`, `popup`, `content`,
    /// `option`, `trigger`), keys, source and highlight.
    #[must_use]
    pub fn combobox(self) -> Combobox<String, Option<String>> {
        self.combobox
    }

    /// The text (tracked).
    #[must_use]
    pub fn value(self) -> String {
        self.value.get()
    }
}

/// The text as one value: `None` while empty.
struct TextAsChoice(Bound<String>);

impl Held<Option<String>> for TextAsChoice {
    /// A `Bound` text, itself written through its own `Held`, checked there.
    fn fork_policy(&self) -> crate::listbox::HeldFork {
        crate::listbox::HeldFork::Signals
    }

    fn get(&self) -> Option<String> {
        Some(self.0.get()).filter(|t| !t.is_empty())
    }

    fn get_untracked(&self) -> Option<String> {
        Some(self.0.get_untracked()).filter(|t| !t.is_empty())
    }

    fn set(&self, value: Option<String>) {
        self.0.set(value.unwrap_or_default());
    }
}

/// An autocomplete in a form (reka's hidden input): its text, and its field
/// focused when a check fails.
impl crate::form::FormValue for Autocomplete {
    type Value = String;

    fn value(&self) -> String {
        self.value.get()
    }

    fn set_value(&self, value: String) {
        self.value.set(value);
    }

    fn focus(&self) -> Option<gui::FocusHandle> {
        Some(self.combobox.field().focus)
    }
}
