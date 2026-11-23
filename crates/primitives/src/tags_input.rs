//! A tags input (reka's `TagsInput`): a list of tags and a field that adds
//! them (research §9, TagsInput).
//!
//! The root ([`TagsInput`]) keeps the tags (the part's `UseModel`, its own
//! or the caller's), the draft (the field's text), the tag selected by the
//! keyboard and whether the last addition was refused as a duplicate. Focus
//! is always in the field; the selected tag is only a state, told to
//! assistive technology as the current one.
//!
//! # Adding
//!
//! The draft is added by Enter, by typing the delimiter (`,` by default),
//! by Tab or the field losing focus when the config says so
//! ([`TagsConfig`]); a paste adds the pieces of the pasted text
//! ([`TagsInput::paste`]). Focus moving from the field into the list it
//! controls ([`TagsInput::list`], its suggestions) is no leaving: the draft
//! is not added, so a choice made there wins (reka's
//! `TagsInputInput.vue:35-41`). One rule for every path: what is added is
//! trimmed, and an empty one adds nothing.
//!
//! - At the limit ([`TagsConfig::max`], 0 for none) the tag is refused, the
//!   draft kept, and nothing is marked invalid.
//! - A duplicate (by `T`'s `PartialEq`) is refused and marks the input
//!   invalid, the draft kept, unless duplicates are allowed. The next
//!   change of the draft clears the mark.
//! - Otherwise the tag is added at the end and the draft emptied.
//! - Each addition and each refusal is told ([`TagsInput::on_add`],
//!   [`TagsInput::on_invalid`]).
//!
//! # Keys (in the field)
//!
//! The tag keys run only while no input method composes, at least one tag
//! can be used, and the caret is at the start with nothing selected.
//!
//! | Key | What it does |
//! | --- | --- |
//! | Enter, draft not empty | adds it; consumed, so the form does not submit |
//! | Enter, empty draft | the form's |
//! | Tab (when it adds), draft not empty | adds it; focus stays, even when refused |
//! | Command or Control + V, Paste (when it adds on paste) | adds the clipboard's pieces; the draft stays |
//! | the delimiter | not typed; a blank draft is emptied, another is added |
//! | Backspace | none selected: selects the last tag; selected: removes it and selects the next, or the one before when it was the last |
//! | Delete | selected: as Backspace; none: the field's |
//! | ArrowLeft (RTL: ArrowRight) | none: the last tag; else the one before, stopping at the first |
//! | ArrowRight (RTL: ArrowLeft) | on the last tag: back to typing; else the next |
//! | Home / End | the first / the last tag |
//! | ArrowUp / ArrowDown, a tag selected | consumed |
//! | any other key | lets the selected tag go |
//!
//! Disabled tags are passed over by the arrows, Home and End, and by the
//! selection that follows a removal (the next usable tag, else the one
//! before).
//!
//! # Where this differs from reka, and why
//!
//! - One adding rule: reka trims nothing, adds a blank draft on Enter,
//!   empties a blank one on the delimiter, and adds every piece of a paste,
//!   blank or not (`TagsInputInput.vue:100-150`). Here every path trims and
//!   drops what is empty (research §9; ADR-0025, group 5).
//! - A paste adds the pasted text's pieces and leaves the draft, as reka's;
//!   what it could not add is pasted as text (reka drops it).
//! - Home with no tag selected selects the first tag; reka's selects the
//!   last (`TagsInputRoot.vue:206-236`).
//! - A tag's delete button is named for its action ("Remove apple",
//!   [`TagsNames`]); reka's is named by the tag alone, so a screen reader
//!   says "apple" for a button that removes it (ADR-0025, group 5).

use std::rc::Rc;

use gui::events::{KeyEvent, NamedKey};
use gui::focus::FocusId;
use gui::{Direction, Div, Interaction, NodeRef, Signal, TextField};

use crate::availability::Availability;
use crate::collection::{Items, from_fn};
use crate::direction::ReadDirection;
use crate::keymap::{Intent, Wrap, step};
use crate::listbox::Bound;

/// What separates tags typed or pasted in one go: a string, or a pattern
/// (reka's `delimiter`, a string or a `RegExp`).
#[derive(Clone, Debug)]
pub enum Delimiter {
    /// One character (`,`).
    Char(char),
    /// A string.
    Text(String),
    /// A pattern: what it matches separates.
    Pattern(regex::Regex),
}

impl PartialEq for Delimiter {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Char(a), Self::Char(b)) => a == b,
            (Self::Text(a), Self::Text(b)) => a == b,
            (Self::Pattern(a), Self::Pattern(b)) => a.as_str() == b.as_str(),
            _ => false,
        }
    }
}

impl Eq for Delimiter {}

impl Delimiter {
    /// Whether `typed` (what one edit inserted) is a delimiter, whole.
    fn is(&self, typed: &str) -> bool {
        match self {
            Self::Char(c) => typed.chars().eq(std::iter::once(*c)),
            Self::Text(t) => !t.is_empty() && typed == t,
            Self::Pattern(p) => p.find(typed).is_some_and(|m| m.start() == 0 && m.end() == typed.len()),
        }
    }

    /// Whether `text` holds a delimiter.
    fn within(&self, text: &str) -> bool {
        match self {
            Self::Char(c) => text.contains(*c),
            Self::Text(t) => !t.is_empty() && text.contains(t.as_str()),
            Self::Pattern(p) => p.is_match(text),
        }
    }

    /// `text`'s pieces between delimiters.
    fn split<'a>(&self, text: &'a str) -> Vec<&'a str> {
        match self {
            Self::Char(c) => text.split(*c).collect(),
            Self::Text(t) if !t.is_empty() => text.split(t.as_str()).collect(),
            Self::Text(_) => vec![text],
            Self::Pattern(p) => p.split(text).collect(),
        }
    }

    /// What joins the pieces a paste could not add, as they stay typed.
    fn joiner(&self) -> String {
        match self {
            Self::Char(c) => c.to_string(),
            Self::Text(t) => t.clone(),
            Self::Pattern(_) => String::from(" "),
        }
    }
}

/// How a tags input adds (reka's defaults: Enter and `,` add, nothing else
/// does, no duplicates, no limit).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagsConfig {
    pub delimiter: Delimiter,
    /// Enter adds the draft (a field whose Enter a caller takes, picking
    /// from a list, says no: reka adds nothing on a prevented Enter).
    pub add_on_enter: bool,
    pub add_on_tab: bool,
    pub add_on_paste: bool,
    pub add_on_blur: bool,
    pub duplicates: bool,
    /// The most tags; 0 is no limit.
    pub max: usize,
}

impl Default for TagsConfig {
    fn default() -> Self {
        Self {
            delimiter: Delimiter::Char(','),
            add_on_enter: true,
            add_on_tab: false,
            add_on_paste: false,
            add_on_blur: false,
            duplicates: false,
            max: 0,
        }
    }
}

/// What assistive technology is told of a tag's delete button; English by
/// default, an app gives its own.
#[derive(Clone, Copy, Debug)]
pub struct TagsNames {
    pub remove: fn(&str) -> String,
}

impl Default for TagsNames {
    fn default() -> Self {
        Self {
            remove: |tag| format!("Remove {tag}"),
        }
    }
}

/// Why an addition was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Refused {
    Duplicate,
    Limit,
}

type Convert<T> = Rc<dyn Fn(&str) -> T>;
type Shown<T> = Rc<dyn Fn(&T) -> String>;
type Tell<T> = gui::Callback<T>;
type TellRefused<T> = gui::Callback<(T, Refused)>;

struct State<T: 'static> {
    config: TagsConfig,
    convert: Convert<T>,
    shown: Shown<T>,
    on_add: Option<Tell<T>>,
    on_invalid: Option<TellRefused<T>>,
    /// Each tag's availability, by index (none: every tag can be used).
    tag_availability: Option<Rc<dyn Fn(usize) -> Availability>>,
    names: TagsNames,
}

/// A tags input: made once in its root's setup, `Copy`.
pub struct TagsInput<T: 'static> {
    tags: Bound<Vec<T>>,
    field: TextField,
    selected: Signal<Option<usize>>,
    invalid: Signal<bool>,
    availability: Signal<Availability>,
    state: Signal<State<T>>,
    direction: ReadDirection,
    /// The draft as the tags input last wrote it, so that write is no typing.
    wrote: Signal<Option<String>>,
    /// The list the field controls (its suggestions): its node, which the
    /// field names, and whether focus is in it.
    list: NodeRef,
    list_focus: Interaction,
}

impl<T: 'static> Clone for TagsInput<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for TagsInput<T> {}

impl<T: 'static> PartialEq for TagsInput<T> {
    fn eq(&self, other: &Self) -> bool {
        self.tags == other.tags
    }
}

impl<T: 'static> std::fmt::Debug for TagsInput<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TagsInput").finish_non_exhaustive()
    }
}

impl TagsInput<String> {
    /// A tags input of strings, each tag the draft as typed (trimmed).
    #[must_use]
    #[track_caller]
    pub fn strings(tags: Bound<Vec<String>>, field: TextField, config: TagsConfig, direction: ReadDirection) -> Self {
        Self::new(tags, field, config, direction, str::to_owned, String::clone)
    }
}

impl<T: Clone + PartialEq + 'static> TagsInput<T> {
    /// A tags input holding `tags`, its draft typed in `field`; `convert`
    /// makes a tag of a draft (reka's `convertValue`), `shown` gives a tag's
    /// text (`displayValue`). Made in its root's setup.
    #[must_use]
    #[track_caller]
    pub fn new(
        tags: Bound<Vec<T>>,
        field: TextField,
        config: TagsConfig,
        direction: ReadDirection,
        convert: impl Fn(&str) -> T + 'static,
        shown: impl Fn(&T) -> String + 'static,
    ) -> Self {
        let me = Self {
            tags,
            field,
            selected: Signal::labeled("tags selected", None),
            invalid: Signal::labeled("tags invalid", false),
            availability: Signal::new(Availability::Enabled),
            state: Signal::new(State {
                config,
                convert: Rc::new(convert),
                shown: Rc::new(shown),
                on_add: None,
                on_invalid: None,
                tag_availability: None,
                names: TagsNames::default(),
            }),
            direction,
            wrote: Signal::new(None),
            list: NodeRef::new(),
            list_focus: Interaction::new(),
        };
        // What is typed: the delimiter adds, any change clears the invalid
        // mark (a paste is taken as a key, before it is typed).
        gui::watch(
            move || field.text(),
            move |text, old| {
                if !me.state.is_alive() {
                    return;
                }
                if me.wrote.get_untracked().as_deref() == Some(text.as_str()) {
                    me.wrote.set(None);
                    return;
                }
                me.invalid.set(false);
                me.typed(text, old);
            },
        );
        // Leaving the field lets the selected tag go, and adds the draft
        // when it adds on blur, unless focus went into the list the field
        // controls: told once the frame's focus flags are written.
        gui::watch(gui::focus::focused, move |now, old| {
            if !me.state.is_alive() {
                return;
            }
            let was = *old == Some(field.focus.id());
            if was && *now != Some(field.focus.id()) {
                me.selected.set(None);
                if me.config().add_on_blur {
                    gui::after_layout(move || {
                        let back = gui::focus::focused() == Some(field.focus.id());
                        if me.state.is_alive() && !back && !gui::untracked(|| me.list_focus.focus_within()) {
                            me.add_draft();
                        }
                    });
                }
            }
        });
        me
    }

    fn config(self) -> TagsConfig {
        self.state.with_untracked(|s| s.config.clone())
    }

    /// Add as `config` says from now on (a caller's props changed).
    pub fn set_config(self, config: TagsConfig) {
        if self.state.with_untracked(|s| s.config != config) {
            self.state.update(|s| s.config = config);
        }
    }

    fn usable(self) -> bool {
        self.availability.get_untracked().actionable()
    }

    /// Whether the tags input can be used (its `disabled`).
    pub fn set_availability(self, availability: Availability) {
        self.availability.set(availability);
    }

    /// Each tag's availability by index (a tag's own `disabled`).
    pub fn set_tag_availability(self, f: impl Fn(usize) -> Availability + 'static) {
        let f: Rc<dyn Fn(usize) -> Availability> = Rc::new(f);
        self.state.update(|s| s.tag_availability = Some(f));
    }

    /// Tell `f` of each tag added.
    pub fn on_add(self, f: impl Into<gui::Callback<T>>) {
        let f: Tell<T> = f.into();
        self.state.update(|s| s.on_add = Some(f));
    }

    /// Tell `f` of each tag refused, and why.
    pub fn on_invalid(self, f: impl Into<gui::Callback<(T, Refused)>>) {
        let f: TellRefused<T> = f.into();
        self.state.update(|s| s.on_invalid = Some(f));
    }

    /// Name the delete buttons by `names`.
    pub fn set_names(self, names: TagsNames) {
        self.state.update(|s| s.names = names);
    }

    /// The tags (tracked).
    #[must_use]
    pub fn tags(self) -> Vec<T> {
        self.tags.get()
    }

    /// The tag selected by the keyboard (tracked).
    #[must_use]
    pub fn selected(self) -> Option<usize> {
        self.selected.get()
    }

    /// Whether the last addition was refused as a duplicate (tracked).
    #[must_use]
    pub fn is_invalid(self) -> bool {
        self.invalid.get()
    }

    /// Whether it can be used (tracked).
    #[must_use]
    pub fn availability(self) -> Availability {
        self.availability.get()
    }

    /// The field.
    #[must_use]
    pub fn field(self) -> TextField {
        self.field
    }

    fn tag_availability(self, index: usize) -> Availability {
        let root = self.availability.get_untracked();
        if root != Availability::Enabled {
            return root;
        }
        self.state
            .with_untracked(|s| s.tag_availability.clone())
            .map_or(Availability::Enabled, |f| f(index))
    }

    fn draft(self) -> String {
        self.field.edit.with_untracked(|e| e.text().to_owned())
    }

    fn write(self, text: &str) {
        if self.draft() == text {
            return;
        }
        self.wrote.set(Some(text.to_owned()));
        self.field.set_text(text);
    }

    /// Add `draft`, trimmed, by the adding rule; whether it was a tag to add
    /// (not blank), added or refused.
    fn add(self, draft: &str) -> Option<Result<(), Refused>> {
        let draft = draft.trim();
        if draft.is_empty() {
            return None;
        }
        let (convert, config) = self.state.with_untracked(|s| (Rc::clone(&s.convert), s.config.clone()));
        let tag = convert(draft);
        let tags = self.tags.get_untracked();
        let refused = if config.max > 0 && tags.len() >= config.max {
            Some(Refused::Limit)
        } else if !config.duplicates && tags.contains(&tag) {
            Some(Refused::Duplicate)
        } else {
            None
        };
        if let Some(why) = refused {
            if why == Refused::Duplicate {
                self.invalid.set(true);
            }
            if let Some(f) = self.state.with_untracked(|s| s.on_invalid.clone()) {
                f.call((tag, why));
            }
            return Some(Err(why));
        }
        self.tags.update(|t| t.push(tag.clone()));
        if let Some(f) = self.state.with_untracked(|s| s.on_add.clone()) {
            f.call(tag);
        }
        Some(Ok(()))
    }

    /// Add the draft; an added one empties it. Whether there was one.
    pub fn add_draft(self) -> bool {
        if !self.usable() {
            return false;
        }
        let draft = self.draft();
        match self.add(&draft) {
            None => false,
            Some(Ok(())) => {
                self.write("");
                true
            }
            Some(Err(_)) => true,
        }
    }

    /// What was typed, from `old` to `text`.
    fn typed(self, text: &str, old: &str) {
        let config = self.config();
        let delimiter = &config.delimiter;
        if !self.usable() || !delimiter.within(text) {
            return;
        }
        let at = inserted(old, text);
        let typed = text.get(at.clone()).unwrap_or("");
        if delimiter.is(typed) {
            // The delimiter typed: not part of the draft.
            let mut draft = String::with_capacity(text.len());
            draft.push_str(text.get(..at.start).unwrap_or(""));
            draft.push_str(text.get(at.end..).unwrap_or(""));
            if draft.trim().is_empty() {
                self.write("");
            } else {
                self.write(&draft);
                self.add_draft();
            }
        }
    }

    /// Paste `text` (the field's paste, when it adds on paste): the pasted
    /// text alone is split at the delimiter (no delimiter in it: it is one
    /// piece) and each piece is added by the adding rule; the draft is left
    /// as it is (reka's `handlePaste`, `TagsInputInput.vue:132-150`). What
    /// could not be added (a duplicate, the limit) is pasted as text at the
    /// caret, joined by the delimiter, so nothing pasted is lost.
    pub fn paste(self, text: &str) {
        if !self.usable() {
            return;
        }
        let delimiter = self.config().delimiter;
        let mut kept: Vec<&str> = Vec::new();
        for piece in delimiter.split(text) {
            if let Some(Err(_)) = self.add(piece) {
                kept.push(piece.trim());
            }
        }
        if kept.is_empty() {
            return;
        }
        let mut edit = self.field.edit.get_untracked();
        edit.insert(&kept.join(&delimiter.joiner()));
        // The tags input's own write: no typing, the invalid mark kept.
        self.wrote.set(Some(edit.text().to_owned()));
        self.field.edit.set(edit);
    }

    /// Remove the tag at `index` (its delete button), unless it cannot be
    /// used.
    pub fn remove(self, index: usize) {
        if !self.tag_availability(index).actionable() {
            return;
        }
        self.tags.update(|t| {
            if index < t.len() {
                t.remove(index);
            }
        });
    }

    /// Remove every tag (the clear button), disabled ones too, unless the
    /// input cannot be used.
    pub fn clear(self) {
        if !self.availability.get_untracked().actionable() {
            return;
        }
        self.tags.set(Vec::new());
        self.selected.set(None);
    }

    /// The field's keys (the module's table): whether one was taken.
    pub fn key(self, ev: &KeyEvent) -> bool {
        if self.field.edit.with_untracked(|e| e.preedit().is_some()) {
            return false;
        }
        if !self.usable() {
            return false;
        }
        let config = self.config();
        if config.add_on_paste && is_paste(ev) {
            // Taken before the field inserts it: a paste is the clipboard's
            // text, whatever an edit inserts (an input method's commit, an
            // autocorrection) is typing.
            if let Some(text) = gui::clipboard::paste() {
                self.paste(&text);
            }
            return true;
        }
        if ev.is(NamedKey::Enter) {
            return config.add_on_enter && self.add_draft();
        }
        if ev.is(NamedKey::Tab) && config.add_on_tab && !ev.modifiers.shift {
            return self.add_draft();
        }
        let at_start = self.field.edit.with_untracked(|e| e.caret == 0 && !e.has_selection());
        let len = self.tags.get_untracked().len();
        let usable = from_fn(len, |i| self.tag_availability(i), |_| std::borrow::Cow::Borrowed(""));
        let any = (0..len).any(|i| usable.focusable(i));
        if !at_start || !any {
            self.selected.set(None);
            return false;
        }
        let selected = self.selected.get_untracked().filter(|&s| s < len);
        let rtl = self.direction.get() == Direction::Rtl;
        let (back, forward) = if rtl {
            (NamedKey::ArrowRight, NamedKey::ArrowLeft)
        } else {
            (NamedKey::ArrowLeft, NamedKey::ArrowRight)
        };
        let last = step(&usable, None, Intent::Last, Wrap::Stop);
        if ev.is(NamedKey::Backspace) || ev.is(NamedKey::Delete) {
            let Some(at) = selected else {
                if ev.is(NamedKey::Delete) {
                    return false;
                }
                self.selected.set(last);
                return true;
            };
            self.remove(at);
            // The next usable tag (now at `at`), else the one before.
            let left = len.saturating_sub(1);
            let usable = from_fn(left, |i| self.tag_availability(i), |_| std::borrow::Cow::Borrowed(""));
            let next = (at..left)
                .find(|&i| usable.focusable(i))
                .or_else(|| (0..at.min(left)).rev().find(|&i| usable.focusable(i)));
            self.selected.set(next);
            return true;
        }
        if ev.is(back) {
            let to = match selected {
                None => last,
                Some(at) => step(&usable, Some(at), Intent::Prev, Wrap::Stop).or(Some(at)),
            };
            self.selected.set(to);
            return true;
        }
        if ev.is(forward) {
            let Some(at) = selected else { return false };
            let to = step(&usable, Some(at), Intent::Next, Wrap::Stop);
            self.selected.set(to);
            return true;
        }
        if ev.is(NamedKey::Home) || ev.is(NamedKey::End) {
            let intent = if ev.is(NamedKey::Home) {
                Intent::First
            } else {
                Intent::Last
            };
            self.selected.set(step(&usable, None, intent, Wrap::Stop));
            return true;
        }
        if (ev.is(NamedKey::ArrowUp) || ev.is(NamedKey::ArrowDown)) && selected.is_some() {
            return true;
        }
        self.selected.set(None);
        false
    }

    /// `d` as the box around the tags and the field: a press anywhere in it
    /// focuses the field (unless it cannot be used), and its availability
    /// is applied.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        let field = self.field;
        let availability = self.availability.get();
        let d = availability.apply(d);
        if !availability.actionable() {
            return d;
        }
        d.on_click(move |_| field.focus.focus())
    }

    /// `d` as the list the field controls (its suggestions): focus moving
    /// from the field into it adds no draft on blur. The field names it
    /// ([`TagsInput::list_ref`], its `controls`).
    #[must_use]
    pub fn list(self, d: Div) -> Div {
        d.node_ref(self.list).interaction(self.list_focus)
    }

    /// The list's node, which the field controls.
    #[must_use]
    pub fn list_ref(self) -> NodeRef {
        self.list
    }

    /// `d` as the tag at `index`: named by its text (the look's child), the
    /// current one while selected (what the look reads as
    /// [`TagsInput::selected`]), its availability applied.
    #[must_use]
    pub fn item(self, d: Div, index: usize) -> Div {
        let d = self.tag_availability(index).apply(d);
        if self.selected.get() == Some(index) {
            d.aria_current(gui::AriaCurrent::True)
        } else {
            d
        }
    }

    /// `d` as the delete button of the tag at `index`: out of the Tab order,
    /// named for its action, a click removes the tag.
    #[must_use]
    pub fn delete(self, d: Div, index: usize) -> Div {
        let (names, shown) = self.state.with_untracked(|s| (s.names, Rc::clone(&s.shown)));
        let tag = self
            .tags
            .get_untracked()
            .get(index)
            .map(|t| shown(t))
            .unwrap_or_default();
        self.tag_availability(index)
            .apply(d)
            .tab_stop(false)
            .role(gui::Role::Button)
            .aria_label((names.remove)(&tag))
            .on_click(move |_| self.remove(index))
    }

    /// The focus the field takes.
    #[must_use]
    pub fn focus(self) -> FocusId {
        self.field.focus.id()
    }
}

/// A form sends the tags.
impl<T: Clone + PartialEq + 'static> crate::form::FormValue for TagsInput<T> {
    type Value = Vec<T>;

    fn value(&self) -> Vec<T> {
        self.tags.get()
    }

    fn set_value(&self, value: Vec<T>) {
        self.tags.set(value);
    }

    fn focus(&self) -> Option<gui::FocusHandle> {
        Some(self.field.focus)
    }
}

/// Whether `ev` pastes (the platform's shortcut, or a paste key).
fn is_paste(ev: &KeyEvent) -> bool {
    if ev.is(NamedKey::Paste) {
        return true;
    }
    matches!(&ev.key, gui::Key::Character(c) if c.eq_ignore_ascii_case("v"))
        && ev.modifiers.primary()
        && !ev.modifiers.alt
        && !ev.modifiers.shift
}

/// Where `text` has what `old` did not, between their common start and end.
fn inserted(old: &str, text: &str) -> std::ops::Range<usize> {
    let start = old
        .char_indices()
        .zip(text.char_indices())
        .find(|((_, a), (_, b))| a != b)
        .map_or(old.len().min(text.len()), |((i, _), _)| i);
    let tail = old
        .chars()
        .rev()
        .zip(text.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum::<usize>();
    let tail = tail.min(old.len().saturating_sub(start));
    let end = text.len().saturating_sub(tail).max(start);
    start..end
}

#[cfg(test)]
mod tests {
    use super::{Delimiter, inserted};

    fn typed<'a>(old: &str, text: &'a str) -> &'a str {
        text.get(inserted(old, text)).unwrap_or("")
    }

    #[test]
    fn what_a_change_inserted() {
        assert_eq!(typed("ab", "ab,"), ",");
        assert_eq!(typed("ab", "a,b"), ",");
        assert_eq!(typed("", "x,y"), "x,y");
        assert_eq!(typed("aa", "aaa"), "a");
        assert_eq!(typed("abc", "ab"), "");
    }

    #[test]
    fn a_delimiter_is_a_string_or_a_pattern() {
        let p = Delimiter::Pattern(regex::Regex::new("[,;] ?").expect("a pattern"));
        assert!(p.is(";") && p.is(", ") && !p.is("a,"));
        assert_eq!(p.split("a, b;c"), ["a", "b", "c"]);
        let t = Delimiter::Text(String::from("||"));
        assert!(t.is("||") && !t.is("|"));
        assert_eq!(t.split("a||b"), ["a", "b"]);
    }
}
