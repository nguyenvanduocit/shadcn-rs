//! Editable text: the value, the caret and the selection, and the edits a
//! keyboard makes. Pure; [`TextField`] is the reactive state a text input
//! element binds to.

use crate::focus::FocusHandle;
use crate::reactive::Signal;

/// What a password field shows for each character of its value.
const BULLET: char = '\u{2022}';

/// Text with a caret and a selection anchor, both byte offsets on char boundaries.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TextEdit {
    pub text: String,
    /// Where typing goes.
    pub caret: usize,
    /// The other end of the selection; equal to `caret` when nothing is selected.
    pub anchor: usize,
}

impl TextEdit {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self {
            text,
            caret: end,
            anchor: end,
        }
    }

    /// The selected byte range, ordered.
    #[must_use]
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    #[must_use]
    pub fn selected_text(&self) -> &str {
        let (a, b) = self.selection();
        self.text.get(a..b).unwrap_or("")
    }

    fn delete_selection(&mut self) {
        let (a, b) = self.selection();
        self.text.replace_range(a..b, "");
        self.caret = a;
        self.anchor = a;
    }

    /// Replace the selection with `s` and put the caret after it.
    pub fn insert(&mut self, s: &str) {
        self.delete_selection();
        self.text.insert_str(self.caret, s);
        self.caret = self.caret.saturating_add(s.len());
        self.anchor = self.caret;
    }

    fn prev_boundary(&self, at: usize) -> usize {
        self.text
            .get(..at)
            .and_then(|s| s.char_indices().next_back())
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self, at: usize) -> usize {
        self.text
            .get(at..)
            .and_then(|s| s.chars().next())
            .map_or(at, |c| at.saturating_add(c.len_utf8()))
    }

    /// Delete the selection, or the character before the caret.
    pub fn backspace(&mut self) {
        if !self.has_selection() {
            self.anchor = self.prev_boundary(self.caret);
        }
        self.delete_selection();
    }

    /// Delete the selection, or the character after the caret.
    pub fn delete(&mut self) {
        if !self.has_selection() {
            self.anchor = self.next_boundary(self.caret);
        }
        self.delete_selection();
    }

    fn move_to(&mut self, at: usize, extend: bool) {
        self.caret = at;
        if !extend {
            self.anchor = at;
        }
    }

    pub fn left(&mut self, extend: bool) {
        let at = if self.has_selection() && !extend {
            self.selection().0
        } else {
            self.prev_boundary(self.caret)
        };
        self.move_to(at, extend);
    }

    pub fn right(&mut self, extend: bool) {
        let at = if self.has_selection() && !extend {
            self.selection().1
        } else {
            self.next_boundary(self.caret)
        };
        self.move_to(at, extend);
    }

    /// To the start of the caret's line.
    pub fn home(&mut self, extend: bool) {
        let at = self
            .text
            .get(..self.caret)
            .and_then(|s| s.rfind('\n'))
            .map_or(0, |i| i.saturating_add(1));
        self.move_to(at, extend);
    }

    /// To the end of the caret's line.
    pub fn end(&mut self, extend: bool) {
        let at = self
            .text
            .get(self.caret..)
            .and_then(|s| s.find('\n'))
            .map_or(self.text.len(), |i| self.caret.saturating_add(i));
        self.move_to(at, extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// The edit as a password field shows it: a bullet per character (Chrome's
    /// `-webkit-text-security: disc`, U+2022), the caret and the selection on
    /// the same characters. Per character, as the caret moves; Chrome masks
    /// per grapheme, which differs only for a character built of several.
    #[must_use]
    pub fn masked(&self) -> Self {
        let bullets = |end: usize| {
            self.text
                .get(..end)
                .map_or(0, |s| s.chars().count())
                .saturating_mul(BULLET.len_utf8())
        };
        Self {
            text: BULLET.to_string().repeat(self.text.chars().count()),
            caret: bullets(self.caret),
            anchor: bullets(self.anchor),
        }
    }

    /// Where an offset `at` in [`Self::masked`] falls in this text.
    #[must_use]
    pub fn unmasked(&self, at: usize) -> usize {
        let nth = at.checked_div(BULLET.len_utf8()).unwrap_or(0);
        self.text.char_indices().nth(nth).map_or(self.text.len(), |(i, _)| i)
    }

    /// Put the caret at `at` (clamped to a char boundary), as a click does.
    pub fn place(&mut self, at: usize, extend: bool) {
        let mut at = at.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at = at.saturating_sub(1);
        }
        self.move_to(at, extend);
    }
}

/// The state behind one text input: its edit and its focus.
#[derive(Clone)]
pub struct TextField {
    pub edit: Signal<TextEdit>,
    pub focus: FocusHandle,
}

impl TextField {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            edit: Signal::new(TextEdit::new(text)),
            focus: FocusHandle::new(),
        }
    }

    /// The current value (reactive).
    #[must_use]
    pub fn text(&self) -> String {
        self.edit.get().text
    }

    /// Replace the value and put the caret at its end.
    pub fn set_text(&self, text: impl Into<String>) {
        self.edit.set(TextEdit::new(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_replaces_the_selection_and_backspace_respects_char_boundaries() {
        let mut e = TextEdit::new("chào");
        e.backspace();
        assert_eq!(e.text, "chà");
        e.left(false);
        e.left(true);
        assert_eq!(e.selected_text(), "h");
        e.insert("H");
        assert_eq!(e.text, "cHà");
        e.select_all();
        e.insert("x");
        assert_eq!((e.text.as_str(), e.caret), ("x", 1));
    }

    #[test]
    fn a_masked_edit_keeps_its_caret_and_selection_on_the_same_characters() {
        let mut e = TextEdit::new("pässw");
        e.place(1, false);
        e.place(3, true);
        let m = e.masked();
        assert_eq!(m.text, "•••••");
        assert_eq!(m.selected_text(), "•");
        assert_eq!(e.unmasked(m.caret), e.caret);
        assert_eq!(e.unmasked(m.text.len()), e.text.len());
    }

    #[test]
    fn home_and_end_stay_on_the_caret_line() {
        let mut e = TextEdit::new("one\ntwo");
        e.home(false);
        assert_eq!(e.caret, 4);
        e.place(1, false);
        e.end(true);
        assert_eq!(e.selected_text(), "ne");
    }
}
