//! Editable text: the value, the caret and the selection, and the edits a
//! keyboard makes. Pure; [`TextField`] is the reactive state a text input
//! element binds to.

use crate::focus::FocusHandle;
use crate::reactive::Signal;

/// What a password field shows for each character of its value.
const BULLET: char = '\u{2022}';

/// Text with a caret and a selection anchor, both byte offsets on char boundaries.
///
/// Its text carries a content revision, new with each change to it (and
/// each text made): two edits of one revision hold the same text, so
/// what compares them, or reuses what was shown of one, compares no bytes.
#[derive(Debug, Default)]
pub struct TextEdit {
    text: String,
    /// Its text's content revision ([`revision`]); 0 is the empty text's.
    rev: u64,
    /// Where typing goes.
    pub caret: usize,
    /// The other end of the selection; equal to `caret` when nothing is selected.
    pub anchor: usize,
    /// What an input method is composing at the caret, not yet part of `text`.
    preedit: Option<Preedit>,
    /// The revision of the composition's text: new with each text the
    /// input method composes, kept as only its caret moves.
    composition: u64,
}

impl Clone for TextEdit {
    fn clone(&self) -> Self {
        Self {
            text: copied(&self.text),
            rev: self.rev,
            caret: self.caret,
            anchor: self.anchor,
            preedit: self.preedit.clone(),
            composition: self.composition,
        }
    }
}

impl PartialEq for TextEdit {
    /// By revision: the same revision is the same text, unread.
    fn eq(&self, other: &Self) -> bool {
        self.rev == other.rev
            && self.caret == other.caret
            && self.anchor == other.anchor
            && self.composition == other.composition
            && self.preedit == other.preedit
    }
}

impl Eq for TextEdit {}

/// A new content revision, never one given before.
fn revision() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A copy of a field's text: every copy the edit path makes goes through
/// here, counted ([`text_copies`]).
pub(crate) fn copied<T: for<'a> From<&'a str>>(text: &str) -> T {
    TEXT_COPIES.with(|n| n.set(n.get().saturating_add(1)));
    T::from(text)
}

/// What a text field shows: its value with any composition in it (or a
/// password's bullets), and the caret and the selection's anchor there.
/// Its text is shared by the frame, its leaf and what reads it, and made
/// again only when what it is made from changes ([`TextEdit::composed`]):
/// a caret moved or a field scrolled copies and compares none of it.
#[derive(Clone, Debug, Eq, Default)]
pub struct Shown {
    pub text: std::sync::Arc<str>,
    pub caret: usize,
    pub anchor: usize,
    /// What its text was made from.
    source: Source,
}

/// What a shown text is made from: the value's revision, the composition
/// in it (its revision, and where it is), and whether it is masked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Source {
    rev: u64,
    /// The composition's revision and the selection it stands in for.
    composing: Option<(u64, usize, usize)>,
    masked: bool,
}

/// What a [`Shown`] is, told by revisions, not by its text: the same mark
/// is the same text, caret and selection, and keeps no text alive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShownMark {
    source: Source,
    caret: usize,
    anchor: usize,
}

impl Shown {
    pub(crate) fn mark(&self) -> ShownMark {
        ShownMark {
            source: self.source,
            caret: self.caret,
            anchor: self.anchor,
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
}

impl PartialEq for Shown {
    fn eq(&self, other: &Self) -> bool {
        self.caret == other.caret && self.anchor == other.anchor && same_text(&self.text, &other.text)
    }
}

/// Whether two shown texts are the same: shared, at once; else by their
/// bytes, counted in tests ([`TEXT_COMPARES`]).
pub(crate) fn same_text(a: &std::sync::Arc<str>, b: &std::sync::Arc<str>) -> bool {
    if std::sync::Arc::ptr_eq(a, b) {
        return true;
    }
    #[cfg(test)]
    TEXT_COMPARES.with(|n| n.set(n.get().saturating_add(1)));
    **a == **b
}

/// The text `last` showed, when it was made from `source`: what decides
/// it reads none of the text.
fn reuse(last: Option<&Shown>, source: Source) -> Option<std::sync::Arc<str>> {
    last.filter(|l| l.source == source)
        .map(|l| std::sync::Arc::clone(&l.text))
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
            rev: revision(),
            caret: end,
            anchor: end,
            preedit: None,
            composition: 0,
        }
    }

    /// Its text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Its text's content revision: new with each change to it.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.rev
    }

    /// What an input method is composing at the caret, not yet part of
    /// its text.
    #[must_use]
    pub fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_ref()
    }

    /// Replace its text, keeping the caret and the selection where they
    /// still fall (on a char boundary at or before); a composition ends.
    /// The same text keeps its revision: nothing reading the value changes.
    pub fn set_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text != self.text {
            self.text = text;
            self.rev = revision();
        }
        self.preedit = None;
        let (caret, anchor) = (self.boundary(self.caret), self.boundary(self.anchor));
        (self.caret, self.anchor) = (caret, anchor);
    }

    /// `at`, clamped to its text and moved back to a char boundary.
    fn boundary(&self, at: usize) -> usize {
        let mut at = at.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at = at.saturating_sub(1);
        }
        at
    }

    /// Its text changed: a new revision.
    fn changed(&mut self) {
        self.rev = revision();
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
        if a != b {
            self.text.replace_range(a..b, "");
            self.changed();
        }
        self.caret = a;
        self.anchor = a;
    }

    /// Replace the selection with `s` and put the caret after it.
    pub fn insert(&mut self, s: &str) {
        self.delete_selection();
        if !s.is_empty() {
            self.text.insert_str(self.caret, s);
            self.changed();
        }
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
    /// Its text shared with `last` (what was shown before) when the same.
    #[must_use]
    pub fn masked(&self, last: Option<&Shown>) -> Shown {
        let bullets = |end: usize| {
            self.text
                .get(..end)
                .map_or(0, |s| s.chars().count())
                .saturating_mul(BULLET.len_utf8())
        };
        let source = Source {
            rev: self.rev,
            composing: None,
            masked: true,
        };
        let text = reuse(last, source).unwrap_or_else(|| BULLET.to_string().repeat(self.text.chars().count()).into());
        Shown {
            text,
            caret: bullets(self.caret),
            anchor: bullets(self.anchor),
            source,
        }
    }

    /// Where an offset `at` in [`Self::masked`] falls in this text.
    #[must_use]
    pub fn unmasked(&self, at: usize) -> usize {
        let nth = at.checked_div(BULLET.len_utf8()).unwrap_or(0);
        self.text.char_indices().nth(nth).map_or(self.text.len(), |(i, _)| i)
    }

    /// Show `preedit` in place of the selection; an empty one ends the
    /// composition with nothing typed. The value and the selection are
    /// untouched until it commits: cancelled, they are as they were.
    pub fn compose(&mut self, preedit: Preedit) {
        if preedit.text.is_empty() {
            self.preedit = None;
            return;
        }
        match &mut self.preedit {
            // Only its caret moved: the composition's text stays.
            Some(p) if p.text == preedit.text => p.cursor = preedit.cursor,
            _ => {
                self.preedit = Some(preedit);
                self.composition = revision();
            }
        }
    }

    /// End the composition with nothing typed (focus left it).
    pub fn end_composition(&mut self) {
        self.preedit = None;
    }

    /// End the composition with `s`, typed as a key types it.
    pub fn commit(&mut self, s: &str) {
        self.preedit = None;
        self.insert(s);
    }

    /// The edit as it shows while composing: the preedit in place of the
    /// selection, the caret where the input method puts it, and the byte
    /// range the preedit takes there. Its text `last`'s (what was shown
    /// before) when made from the same revisions: told without reading the
    /// text or building any, so a caret moved, by a key or the input
    /// method, copies and compares nothing.
    #[must_use]
    pub fn composed(&self, last: Option<&Shown>) -> (Shown, Option<(usize, usize)>) {
        let (a, b) = self.selection();
        let source = Source {
            rev: self.rev,
            composing: self.preedit.as_ref().map(|_| (self.composition, a, b)),
            masked: false,
        };
        let text = reuse(last, source);
        let Some(p) = &self.preedit else {
            let shown = Shown {
                text: text.unwrap_or_else(|| copied(&self.text)),
                caret: self.caret,
                anchor: self.anchor,
                source,
            };
            return (shown, None);
        };
        let (start, end) = (a, a.saturating_add(p.text.len()));
        let text = text.unwrap_or_else(|| {
            let mut text: String = copied(&self.text);
            text.replace_range(a..b, &p.text);
            text.into()
        });
        let caret = p
            .cursor
            .map_or(end, |(_, to)| start.saturating_add(to.min(p.text.len())));
        let shown = Shown {
            text,
            caret,
            anchor: caret,
            source,
        };
        (shown, Some((start, end)))
    }

    /// Where an offset `at` in [`Self::composed`] falls in this text: inside
    /// the preedit is where it starts, past it is past the selection it
    /// stands in for.
    #[must_use]
    pub fn uncomposed(&self, at: usize) -> usize {
        let Some(p) = &self.preedit else { return at };
        let (a, b) = self.selection();
        let end = a.saturating_add(p.text.len());
        if at <= a {
            at
        } else if at < end {
            a
        } else {
            at.saturating_sub(p.text.len()).saturating_add(b.saturating_sub(a))
        }
    }

    /// Put the caret at `at` (clamped to a char boundary), as a click does.
    pub fn place(&mut self, at: usize, extend: bool) {
        self.move_to(self.boundary(at), extend);
    }
}

/// The state behind one text input: its edit, its focus, and how it
/// scrolls ([`FieldView`]), as Compose's `TextFieldState` holds its
/// scroll: living as long as the handle (its signals' scope). One field
/// shows it at a time (its focus, its input method and its caret are one
/// node's): two views of one text are two `TextField`s bound to the same
/// model, each with its own caret, selection and scroll.
///
/// What reads its value alone (a model bound to it, a filter, a form)
/// reads it through [`TextField::with_value`] or [`TextField::text`]: they
/// follow the value's revision, so a caret moved, a selection made or a
/// composition shown wakes none of them. [`TextField::edit`] carries the
/// caret and the selection too; reading it is reading all of them.
#[derive(Clone, Copy, PartialEq)]
pub struct TextField {
    pub edit: Signal<TextEdit>,
    pub focus: FocusHandle,
    pub(crate) view: Signal<FieldView>,
    /// The value's revision ([`TextEdit::revision`]): what reads the value
    /// alone follows this, not the edit.
    value: crate::reactive::Computed<u64>,
}

impl TextField {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        let edit = Signal::new(TextEdit::new(text));
        Self {
            edit,
            focus: FocusHandle::new(),
            view: Signal::new(FieldView::default()),
            value: crate::reactive::Computed::new(move || edit.with(TextEdit::revision)),
        }
    }

    /// Whether its scope still holds it: one disposed is read no more.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.edit.is_alive() && self.view.is_alive() && self.value.is_alive()
    }

    /// Its value's revision (reactive: only as the value changes).
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.value.get()
    }

    /// Read its value (reactive: only as the value changes, not its caret,
    /// its selection or a composition), copying none of it.
    pub fn with_value<R>(&self, f: impl FnOnce(&str) -> R) -> R {
        self.value.with(|_| ());
        self.edit.with_untracked(|e| f(&e.text))
    }

    /// The current value, a copy (reactive: only as the value changes).
    #[must_use]
    pub fn text(&self) -> String {
        self.with_value(copied)
    }

    /// Replace the value and put the caret at its end; focused, the
    /// caret is brought into view, as an edit by hand brings it. The value
    /// it has already keeps its revision (what reads the value is not
    /// told); with the caret at its end and nothing composed, it is no
    /// change at all.
    pub fn set_text(&self, text: impl Into<String>) {
        let text = text.into();
        let end = text.len();
        let (same, still) = self.edit.with_untracked(|e| {
            let same = e.text == text;
            (same, same && e.caret == end && e.anchor == end && e.preedit.is_none())
        });
        if still {
            return;
        }
        if same {
            self.edit.update(|e| {
                e.preedit = None;
                e.place(end, false);
            });
        } else {
            self.edit.set(TextEdit::new(text));
        }
        if self.focus.is_focused() {
            self.reveal();
        }
    }

    /// Bring its caret into view as it is next laid out
    /// ([`crate::layout::caret_reveal`]), in its field's box as it is then:
    /// every way of editing it (a key, an input method, a screen reader, a
    /// setter) asks alike. An ask is a new generation: a layout that read
    /// an older one takes only that, never a newer.
    pub(crate) fn reveal(&self) {
        self.view.update(|v| v.reveal = v.reveal.wrapping_add(1));
    }

    /// Its content's scroll offset.
    pub(crate) fn offset(&self) -> (f32, f32) {
        self.view.with_untracked(|v| v.offset)
    }

    /// Scroll its content to `offset`.
    pub(crate) fn scroll_to(&self, offset: (f32, f32)) {
        if self.offset() != offset {
            self.view.update(|v| v.offset = offset);
        }
    }

    /// Laid out at `offset`, having taken reveals up to generation
    /// `applied` (a newer ask stays to be taken).
    pub(crate) fn settled(&self, offset: (f32, f32), applied: u64) {
        if self.view.with_untracked(|v| v.offset != offset || v.applied != applied) {
            self.view.update(|v| {
                v.offset = offset;
                v.applied = applied;
            });
        }
    }
}

/// How a text field's content is scrolled: its offset, and the caret
/// reveals asked (a generation) and taken.
#[derive(Debug, PartialEq)]
pub(crate) struct FieldView {
    pub offset: (f32, f32),
    /// Bumped by each ask to bring the caret into view.
    pub reveal: u64,
    /// The last generation its field took.
    pub applied: u64,
}

impl Default for FieldView {
    fn default() -> Self {
        #[cfg(test)]
        VIEWS.with(|n| n.set(n.get().saturating_add(1)));
        Self {
            offset: (0.0, 0.0),
            reveal: 0,
            applied: 0,
        }
    }
}

#[cfg(test)]
impl Drop for FieldView {
    fn drop(&mut self) {
        // At the thread's end, the count may be gone before the view.
        VIEWS.try_with(|n| n.set(n.get().saturating_sub(1))).unwrap_or_default();
    }
}

#[cfg(test)]
crate::engine::engine_local! {
    /// Field views alive on this thread ([`FieldView::live`]).
    [Fork] static VIEWS: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    /// Times the list placing fields fills grew ([`crate::layout::LayoutTree::swap_fields`]).
    [Fork] pub(crate) static FIELD_GROWS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Times two shown texts were compared by their bytes ([`same_text`]).
    [Fork] pub(crate) static TEXT_COMPARES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

crate::engine::engine_local! {
    /// Times the edit path copied a field's text on this thread
    /// ([`copied`], [`text_copies`]).
    [Fork] pub(crate) static TEXT_COPIES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many times a field's text was copied on this thread (a value read
/// out with [`TextField::text`], an edit cloned, what a field shows made
/// anew): what reads only the value, or only moves the caret, adds none.
#[must_use]
pub fn text_copies() -> u64 {
    TEXT_COPIES.with(std::cell::Cell::get)
}

#[cfg(test)]
impl FieldView {
    /// How many field views live on this thread.
    pub(crate) fn live() -> isize {
        VIEWS.with(std::cell::Cell::get)
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
        let m = e.masked(None);
        assert_eq!(&*m.text, "•••••");
        let (a, b) = m.selection();
        assert_eq!(&m.text[a..b], "•");
        assert_eq!(e.unmasked(m.caret), e.caret);
        assert_eq!(e.unmasked(m.text.len()), e.text.len());
        // Masked again, the caret moved: the same bullets, shared.
        e.place(4, false);
        let again = e.masked(Some(&m));
        assert!(std::sync::Arc::ptr_eq(&again.text, &m.text));
        assert_eq!(e.unmasked(again.caret), e.caret);
    }
}
