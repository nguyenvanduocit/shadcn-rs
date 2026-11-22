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
    /// What an input method is composing at the caret, not yet part of `text`.
    pub preedit: Option<Preedit>,
}

/// Text an input method is composing (macOS's marked text, a Telex syllable,
/// kana before conversion): shown at the caret, underlined, and part of the
/// value only once the input method commits it, as a browser's composition.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Preedit {
    pub text: String,
    /// The input method's caret in `text`, a byte range; `None` hides it.
    pub cursor: Option<(usize, usize)>,
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
            preedit: None,
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

    /// Delete the selection, or from the caret to `at` (where Backspace or
    /// Delete reaches, by the text's graphemes or words).
    pub fn delete_to(&mut self, at: usize) {
        if !self.has_selection() {
            self.place(at, true);
        }
        self.delete_selection();
    }

    fn move_to(&mut self, at: usize, extend: bool) {
        self.caret = at;
        if !extend {
            self.anchor = at;
        }
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
            preedit: None,
        }
    }

    /// Where an offset `at` in [`Self::masked`] falls in this text.
    #[must_use]
    pub fn unmasked(&self, at: usize) -> usize {
        let nth = at.checked_div(BULLET.len_utf8()).unwrap_or(0);
        self.text.char_indices().nth(nth).map_or(self.text.len(), |(i, _)| i)
    }

    /// Show `preedit` at the caret in place of the selection; an empty one
    /// ends the composition with nothing typed.
    pub fn compose(&mut self, preedit: Preedit) {
        if preedit.text.is_empty() {
            self.preedit = None;
            return;
        }
        self.delete_selection();
        self.preedit = Some(preedit);
    }

    /// End the composition with `s`, typed as a key types it.
    pub fn commit(&mut self, s: &str) {
        self.preedit = None;
        self.insert(s);
    }

    /// The edit as it shows while composing: the preedit inside the text at
    /// the caret, the caret where the input method puts it, and the byte
    /// range the preedit takes there.
    #[must_use]
    pub fn composed(&self) -> (Self, Option<(usize, usize)>) {
        let Some(p) = &self.preedit else {
            return (self.clone(), None);
        };
        let (start, end) = (self.caret, self.caret.saturating_add(p.text.len()));
        let mut text = self.text.clone();
        text.insert_str(start, &p.text);
        let caret = p
            .cursor
            .map_or(end, |(_, to)| start.saturating_add(to.min(p.text.len())));
        let shown = Self {
            text,
            caret,
            anchor: caret,
            preedit: None,
        };
        (shown, Some((start, end)))
    }

    /// Where an offset `at` in [`Self::composed`] falls in this text: inside
    /// the preedit is the caret, past it moves back by its length.
    #[must_use]
    pub fn uncomposed(&self, at: usize) -> usize {
        let Some(p) = &self.preedit else { return at };
        let end = self.caret.saturating_add(p.text.len());
        if at <= self.caret {
            at
        } else if at < end {
            self.caret
        } else {
            at.saturating_sub(p.text.len())
        }
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
#[derive(Clone, Copy, PartialEq)]
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
    fn typing_replaces_the_selection_and_deleting_keeps_char_boundaries() {
        let mut e = TextEdit::new("chào");
        // Backspace reaches the previous character's start (`TextSystem::step`).
        e.delete_to(e.text.len() - 1);
        assert_eq!(e.text, "chà");
        e.place(1, false);
        e.place(2, true);
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
}
