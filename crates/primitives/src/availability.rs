//! Whether an item or a control can be used: one notion for every primitive,
//! in two levels (ADR-0025; research §3.2).
//!
//! reka has three notions of "disabled" that disagree: an item's own flag
//! (Collection, RovingFocus and the selection rules skip it), the native
//! `disabled` attribute (the only one `useArrowNavigation` skips), and
//! nothing at all (its Home and End). Here there is one, and every block
//! reads it the same way.

/// Whether a part can be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Availability {
    /// Focusable, acted on, submitted.
    #[default]
    Enabled,
    /// Still focusable and read by assistive technology, but not acted on
    /// (`aria-disabled`): a checkbox at its group's limit, an unavailable
    /// date, a carousel's Prev at the first slide. Navigation and
    /// typeahead stop on it, as it takes focus; clicks (the pointer's,
    /// Enter and Space, a screen reader's) and selection pass it by.
    Unavailable,
    /// Not focusable, skipped by navigation, typeahead and selection, not
    /// submitted.
    Disabled,
}

impl Availability {
    /// [`Self::Disabled`] when `disabled`, else [`Self::Enabled`].
    #[must_use]
    pub const fn from_disabled(disabled: bool) -> Self {
        if disabled { Self::Disabled } else { Self::Enabled }
    }

    /// Whether it takes focus, so navigation may stop on it.
    #[must_use]
    pub const fn focusable(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    /// Whether it may be acted on: clicked, picked, selected.
    #[must_use]
    pub const fn actionable(self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// Whether its value goes with a form's submission.
    #[must_use]
    pub const fn submits(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    /// Whether the look shows it as disabled (dimmed, the not-allowed
    /// cursor): both levels are.
    #[must_use]
    pub const fn dimmed(self) -> bool {
        !matches!(self, Self::Enabled)
    }

    /// `d` as this level makes a node: a disabled one takes no clicks, keys
    /// or focus, and is read as disabled; an unavailable one keeps focus
    /// and keys and is read as disabled (`aria-disabled`), but its clicks
    /// do not run (`gui::Div::unavailable`); an enabled one is left as it
    /// is.
    #[must_use]
    pub fn apply(self, d: gui::Div) -> gui::Div {
        match self {
            Self::Enabled => d,
            Self::Unavailable => d.unavailable(true),
            Self::Disabled => d.disabled(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Availability;

    #[test]
    fn two_levels_of_not_usable() {
        use Availability::{Disabled, Enabled, Unavailable};
        let table = [
            // (level, focusable, actionable, submits, dimmed)
            (Enabled, true, true, true, false),
            (Unavailable, true, false, true, true),
            (Disabled, false, false, false, true),
        ];
        for (a, focusable, actionable, submits, dimmed) in table {
            assert_eq!(a.focusable(), focusable, "{a:?} focusable");
            assert_eq!(a.actionable(), actionable, "{a:?} actionable");
            assert_eq!(a.submits(), submits, "{a:?} submits");
            assert_eq!(a.dimmed(), dimmed, "{a:?} dimmed");
        }
        assert_eq!(Availability::from_disabled(true), Disabled);
        assert_eq!(Availability::from_disabled(false), Enabled);
    }
}
