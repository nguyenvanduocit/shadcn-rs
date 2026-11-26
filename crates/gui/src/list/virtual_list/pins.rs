//! What decides the rows pinned outside the window (ADR-0026, K4c): the
//! row holding focus, the rows kept around it, the active row.
//!
//! A plan reads these, and the window's source is run again only when its
//! list's version moves. So they are private here and written only through
//! setters that note a change ([`Pins::take_changed`]); every write to a
//! list's state ([`super::ListState::poke`]) bumps its version when one was
//! noted. A plan input added here is tracked by construction; the layout
//! hook's check ([`super::ListState::check_pins`]) compares the rows they
//! pin with the rows built.

use super::KeyedRow;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Pins {
    /// The row holding focus, as last laid out (or followed through a
    /// splice or a reorder), and its key.
    focus_row: Option<KeyedRow>,
    /// The row its owner treats as current (an active option).
    active: Option<u64>,
    /// Rows on each side of the focused one kept built with it.
    around: u64,
    /// One of them changed since the last write told the list's version.
    changed: bool,
}

impl Pins {
    pub(super) fn focus_row(&self) -> Option<KeyedRow> {
        self.focus_row
    }

    pub(super) fn active(&self) -> Option<u64> {
        self.active
    }

    pub(super) fn around(&self) -> u64 {
        self.around
    }

    pub(super) fn set_focus_row(&mut self, row: Option<KeyedRow>) {
        if self.focus_row != row {
            self.focus_row = row;
            self.changed = true;
        }
    }

    pub(super) fn set_active(&mut self, row: Option<u64>) {
        if self.active != row {
            self.active = row;
            self.changed = true;
        }
    }

    pub(super) fn set_around(&mut self, rows: u64) {
        if self.around != rows {
            self.around = rows;
            self.changed = true;
        }
    }

    /// The focused row a plan found and built, taken in with it: what the
    /// source read already, so nothing new to tell.
    pub(super) fn adopt_focus_row(&mut self, row: Option<KeyedRow>) {
        self.focus_row = row;
    }

    /// Whether one changed since last asked.
    pub(super) fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// The active row written past the setter, for the check's own test: a
    /// plan input whose change the version never hears of.
    #[cfg(test)]
    pub(super) fn set_active_untold(&mut self, row: Option<u64>) {
        self.active = row;
    }
}
