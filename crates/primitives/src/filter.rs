//! Matching a search in a list's texts (reka's `useFilter`): starts with,
//! ends with, contains.
//!
//! - An empty search matches everything.
//! - Both sides are put in Unicode NFC first, so a precomposed "é" matches
//!   "e" with a combining accent.
//! - Characters compare by the collation of a locale at primary strength
//!   (reka's `Intl.Collator(…, { usage: 'search', sensitivity: 'base' })`):
//!   case and accents aside, so "cafe" matches "Café".
//! - "Contains" compares every window of the text as long as the search, in
//!   characters; a search longer than the text matches nothing.
//!
//! The locale is the caller's (ICU4X's collation for it), where reka fixes
//! English (research §6, "Docs lệch"). [`Filter::default`] is the root
//! collation, which is English's, made once per process and shared.
//!
//! A filter is `Send + Sync` and cheap to clone, so a list of millions of
//! rows is filtered on a worker thread. The search is prepared once
//! ([`Filter::search`]: normalized, its length counted); testing an item
//! then allocates nothing unless the item's text is not already NFC.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::sync::{Arc, LazyLock};

use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::{Collator, CollatorBorrowed};
use icu_locale_core::Locale;
use icu_normalizer::ComposingNormalizerBorrowed;

/// Where in a text a search must match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Match {
    StartsWith,
    EndsWith,
    #[default]
    Contains,
}

/// A filter's comparison: a locale's collation at primary strength.
#[derive(Clone)]
pub struct Filter {
    /// `None` when the locale has no collation data, which compiled data
    /// always has: case-folded comparison is the fallback.
    collator: Option<Arc<CollatorBorrowed<'static>>>,
}

impl std::fmt::Debug for Filter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Filter").finish_non_exhaustive()
    }
}

/// The root collation, made the first time a default filter is asked for.
static ROOT: LazyLock<Filter> = LazyLock::new(|| Filter::new(&Locale::UNKNOWN));

impl Default for Filter {
    /// The root collation, shared: no collator is made per call.
    fn default() -> Self {
        ROOT.clone()
    }
}

const NFC: ComposingNormalizerBorrowed<'static> = ComposingNormalizerBorrowed::new_nfc();

impl Filter {
    /// A filter comparing as `locale` collates.
    #[must_use]
    pub fn new(locale: &Locale) -> Self {
        let mut options = CollatorOptions::default();
        options.strength = Some(Strength::Primary);
        Self {
            collator: Collator::try_new(locale.into(), options).ok().map(Arc::new),
        }
    }

    /// `search` prepared to test many texts: normalized and counted once.
    #[must_use]
    pub fn search<'f>(&'f self, search: &str) -> Search<'f> {
        let search = NFC.normalize(search).into_owned();
        Search {
            filter: self,
            width: search.chars().count(),
            search,
        }
    }

    fn equal(&self, a: &str, b: &str) -> bool {
        match &self.collator {
            Some(c) => c.compare(a, b) == Ordering::Equal,
            None => a
                .chars()
                .flat_map(char::to_lowercase)
                .eq(b.chars().flat_map(char::to_lowercase)),
        }
    }
}

/// A search prepared by [`Filter::search`], tested against each item's
/// text.
#[derive(Clone, Debug)]
pub struct Search<'f> {
    filter: &'f Filter,
    search: String,
    width: usize,
}

impl Search<'_> {
    /// Whether the search is empty, which matches everything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.search.is_empty()
    }

    /// Whether `text` matches where `how` says.
    #[must_use]
    pub fn matches(&self, text: &str, how: Match) -> bool {
        if self.is_empty() {
            return true;
        }
        let text: Cow<'_, str> = NFC.normalize(text);
        let text = text.as_ref();
        let width = self.width;
        // The byte offset of the window starting at each character, and of
        // the one `width` characters on (the text's end included).
        let starts = text.char_indices().map(|(i, _)| i);
        let ends = text
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .skip(width);
        let mut windows = starts.zip(ends).filter_map(|(from, to)| text.get(from..to));
        let same = |w: &str| self.filter.equal(w, &self.search);
        match how {
            Match::StartsWith => windows.next().is_some_and(same),
            Match::EndsWith => windows.last().is_some_and(same),
            Match::Contains => windows.any(same),
        }
    }

    #[must_use]
    pub fn starts_with(&self, text: &str) -> bool {
        self.matches(text, Match::StartsWith)
    }

    #[must_use]
    pub fn ends_with(&self, text: &str) -> bool {
        self.matches(text, Match::EndsWith)
    }

    #[must_use]
    pub fn contains(&self, text: &str) -> bool {
        self.matches(text, Match::Contains)
    }
}

#[cfg(test)]
mod tests {
    use super::{Filter, Match};

    #[test]
    fn matches_as_reka_does_with_a_base_collator() {
        let f = Filter::default();
        // (text, search, starts, ends, contains)
        let table = [
            ("Hello World", "", true, true, true),
            ("Hello World", "hello", true, false, true),
            ("Hello World", "WORLD", false, true, true),
            ("Hello World", "lo w", false, false, true),
            ("Café", "cafe", true, true, true),
            ("cafe\u{301}", "café", true, true, true),
            ("Crème brûlée", "brulee", false, true, true),
            ("abc", "abcd", false, false, false),
            ("apple", "x", false, false, false),
        ];
        for (text, search, starts, ends, contains) in table {
            let q = f.search(search);
            assert_eq!(q.matches(text, Match::StartsWith), starts, "{text:?} starts {search:?}");
            assert_eq!(q.matches(text, Match::EndsWith), ends, "{text:?} ends {search:?}");
            assert_eq!(
                q.matches(text, Match::Contains),
                contains,
                "{text:?} contains {search:?}"
            );
        }
    }

    #[test]
    fn the_locale_is_the_callers() {
        let de: icu_locale_core::Locale = "de".parse().unwrap_or(icu_locale_core::Locale::UNKNOWN);
        let f = Filter::new(&de);
        assert!(f.search("STRAßE").contains("Straße"));
        assert!(f.search("arg").starts_with("Ärger"));
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test is the worker: it proves a filter is Send and Sync by using one on a thread"
    )]
    fn a_filter_goes_to_a_worker_thread() {
        fn send_sync<T: Send + Sync>(_: &T) {}
        let f = Filter::default();
        send_sync(&f);
        let rows: Vec<String> = (0..1000).map(|i| format!("Row {i}")).collect();
        let shown = std::thread::spawn(move || {
            let q = f.search("row 99");
            rows.iter().filter(|r| q.contains(r)).count()
        })
        .join()
        .unwrap_or(0);
        assert_eq!(shown, 11, "row 99 and row 990 to 999");
    }
}
