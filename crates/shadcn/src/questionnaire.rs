//! Questionnaire (`ui/questionnaire`): questions asked one at a time, each
//! answered by choosing (one, or several) or by typing, with previous, skip,
//! next and submit, progress, validation and keyboard shortcuts.
//!
//! Vue registers items and answers as they mount; here the questions are
//! declared up front ([`QuestionnaireItem`], Vue's `items` prop) and their
//! answers live in a [`Questionnaire`] the caller creates once. The rules
//! (status, validity, what is submitted) are pure functions on
//! [`ItemAnswer`]; the handle only reads and writes signals.

use std::rc::Rc;

use gui::Properties;
use gui::prelude::*;
use gui::style::Align;
use gui::{Div, FocusHandle, Icon, KeyEvent, NamedKey, Signal, Text, TextField, TextInput, div, icon, text};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::focus::{destructive_ring, focus_ring};
use crate::input::input;
use crate::theme::theme;

/// Which keys pick a choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestionnaireShortcuts {
    /// A to Z.
    Letters,
    /// 1 to 9.
    Numbers,
}

impl QuestionnaireShortcuts {
    /// The key of the `index`th choice, if there is one.
    fn key(self, index: usize) -> Option<char> {
        let index = u8::try_from(index).ok()?;
        match self {
            Self::Letters => (index < 26).then(|| char::from(b'A'.saturating_add(index))),
            Self::Numbers => (index < 9).then(|| char::from(b'1'.saturating_add(index))),
        }
    }

    /// The choice a typed key picks.
    fn index(self, typed: char) -> Option<usize> {
        let typed = typed.to_ascii_uppercase();
        (0..26).find(|&i| self.key(i) == Some(typed))
    }
}

/// One question: its name (what the answer is submitted under), its choice
/// values in order, and how it is answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionnaireItem {
    pub name: String,
    pub choices: Vec<String>,
    /// Must be answered before moving on; cannot be skipped.
    pub required: bool,
    /// Choices are checkboxes and every checked one is kept.
    pub multiple: bool,
    /// A text box after the choices for an answer of one's own.
    pub input: bool,
    /// Left out of the questionnaire until enabled (Vue's `disabled`).
    pub disabled: bool,
    /// The choices checked when it opens and after a reset (`default-checked`).
    pub checked: Vec<String>,
    /// The words in its text box when it opens and after a reset (`default-value`).
    pub default_value: String,
}

impl QuestionnaireItem {
    #[must_use]
    pub fn new(name: impl Into<String>, choices: &[&str]) -> Self {
        Self {
            name: name.into(),
            choices: choices.iter().map(|&c| c.to_owned()).collect(),
            required: false,
            multiple: false,
            input: false,
            disabled: false,
            checked: Vec::new(),
            default_value: String::new(),
        }
    }

    #[must_use]
    pub fn required(self) -> Self {
        Self { required: true, ..self }
    }

    #[must_use]
    pub fn multiple(self) -> Self {
        Self { multiple: true, ..self }
    }

    #[must_use]
    pub fn with_input(self) -> Self {
        Self { input: true, ..self }
    }

    #[must_use]
    pub fn disabled(self) -> Self {
        Self { disabled: true, ..self }
    }

    /// Check `values` from the start (`default-checked` on those choices).
    #[must_use]
    pub fn checked(self, values: &[&str]) -> Self {
        Self {
            checked: values.iter().map(|&v| v.to_owned()).collect(),
            ..self
        }
    }

    /// A text box holding `words` from the start (`default-value`).
    #[must_use]
    pub fn default_value(self, words: impl Into<String>) -> Self {
        Self {
            input: true,
            default_value: words.into(),
            ..self
        }
    }
}

/// Where a question stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestionnaireStatus {
    Unanswered,
    Answered,
    Skipped,
}

/// A question's answer as submitted: its checked choice values, then the
/// typed answer when it counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionnaireAnswer {
    pub name: String,
    pub values: Vec<String>,
}

/// What the user did on one question. The text box's words live in its
/// `TextField`; they count as the answer once they differ from `baseline`,
/// the words there when a single choice was last picked or the question
/// skipped (typing then replaces that choice, as Vue's one selected answer does).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ItemAnswer {
    chosen: Vec<String>,
    baseline: String,
    skipped: bool,
    /// Moving on was tried: show what is missing.
    attempted: bool,
    /// Marked invalid from outside (Vue's `invalid` prop), after the
    /// caller's own validation.
    flagged: bool,
}

impl ItemAnswer {
    /// The answer an item opens with: its default-checked choices.
    fn initial(item: &QuestionnaireItem) -> Self {
        Self {
            chosen: item.checked.clone(),
            ..Self::default()
        }
    }

    fn typed(&self, item: &QuestionnaireItem, words: &str) -> bool {
        item.input && !words.trim().is_empty() && words != self.baseline
    }

    fn checked(&self, item: &QuestionnaireItem, words: &str, value: &str) -> bool {
        self.chosen.iter().any(|c| c == value) && (item.multiple || !self.typed(item, words))
    }

    fn status(&self, item: &QuestionnaireItem, words: &str) -> QuestionnaireStatus {
        let answered = self.typed(item, words) || item.choices.iter().any(|v| self.checked(item, words, v));
        if answered {
            QuestionnaireStatus::Answered
        } else if self.skipped {
            QuestionnaireStatus::Skipped
        } else {
            QuestionnaireStatus::Unanswered
        }
    }

    /// Vue's `valid`: a disabled item always is, an optional one skipped is,
    /// and otherwise it needs an answer and no outside flag.
    fn valid(&self, item: &QuestionnaireItem, words: &str, disabled: bool) -> bool {
        disabled
            || match self.status(item, words) {
                QuestionnaireStatus::Answered => !self.flagged,
                QuestionnaireStatus::Skipped => !item.required,
                QuestionnaireStatus::Unanswered => false,
            }
    }

    fn invalid(&self, item: &QuestionnaireItem, words: &str, disabled: bool) -> bool {
        let skipped_on_purpose = self.status(item, words) == QuestionnaireStatus::Skipped && !item.required;
        !disabled && !skipped_on_purpose && (self.flagged || (self.attempted && !self.valid(item, words, disabled)))
    }

    fn choose(self, item: &QuestionnaireItem, words: &str, value: &str) -> Self {
        let chosen = if item.multiple {
            if self.chosen.iter().any(|c| c == value) {
                self.chosen.into_iter().filter(|c| c != value).collect()
            } else {
                let mut chosen = self.chosen;
                chosen.push(value.to_owned());
                chosen
            }
        } else {
            vec![value.to_owned()]
        };
        let baseline = if item.multiple { self.baseline } else { words.to_owned() };
        Self {
            chosen,
            baseline,
            skipped: false,
            ..self
        }
    }

    fn skip(self, words: &str) -> Self {
        Self {
            chosen: Vec::new(),
            baseline: words.to_owned(),
            skipped: true,
            ..self
        }
    }

    fn attempt(self) -> Self {
        Self {
            attempted: true,
            ..self
        }
    }

    fn flag(self, flagged: bool) -> Self {
        Self { flagged, ..self }
    }

    fn values(&self, item: &QuestionnaireItem, words: &str) -> Vec<String> {
        let mut values: Vec<String> = item
            .choices
            .iter()
            .filter(|v| self.checked(item, words, v))
            .cloned()
            .collect();
        if self.typed(item, words) {
            values.push(words.trim().to_owned());
        }
        values
    }
}

type Submit = Rc<dyn Fn(&Questionnaire, Vec<QuestionnaireAnswer>)>;
type Change = Rc<dyn Fn(&Questionnaire, &str, &str)>;

/// The questionnaire's state; create it once, outside the render, and pass
/// it to every part.
#[derive(Clone)]
pub struct Questionnaire {
    items: Rc<[QuestionnaireItem]>,
    default_item: Option<String>,
    shortcuts: Signal<Option<QuestionnaireShortcuts>>,
    active: Signal<usize>,
    answers: Signal<Vec<ItemAnswer>>,
    /// Which items are left out (Vue's `disabled` on an item definition).
    disabled: Signal<Vec<bool>>,
    /// One per question; only those with `input` show theirs.
    fields: Rc<[TextField]>,
    /// Each question's choices, to move between them with the arrow keys.
    choice_focus: Rc<[Vec<FocusHandle>]>,
    on_submit: Submit,
    on_change: Option<Change>,
}

impl Questionnaire {
    /// `items` in order, opening on `default_item` (or the first enabled);
    /// `on_submit` gets the questionnaire and every answer once all are valid.
    #[must_use]
    pub fn new(
        items: Vec<QuestionnaireItem>,
        default_item: Option<&str>,
        shortcuts: Option<QuestionnaireShortcuts>,
        on_submit: impl Fn(&Questionnaire, Vec<QuestionnaireAnswer>) + 'static,
    ) -> Self {
        let disabled: Vec<bool> = items.iter().map(|i| i.disabled).collect();
        let start = opening(&items, &disabled, default_item);
        let answers = items.iter().map(ItemAnswer::initial).collect();
        let fields = items.iter().map(|i| TextField::new(i.default_value.clone())).collect();
        let choice_focus = items
            .iter()
            .map(|i| i.choices.iter().map(|_| FocusHandle::new()).collect())
            .collect();
        Self {
            items: items.into(),
            default_item: default_item.map(str::to_owned),
            shortcuts: Signal::labeled("questionnaire shortcuts", shortcuts),
            active: Signal::labeled("questionnaire item", start),
            answers: Signal::labeled("questionnaire answers", answers),
            disabled: Signal::labeled("questionnaire disabled items", disabled),
            fields,
            choice_focus,
            on_submit: Rc::new(on_submit),
            on_change: None,
        }
    }

    /// Call `f` with the questionnaire, the item's name and the choice's
    /// value each time a choice is checked or unchecked (a choice's `@change`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(&Questionnaire, &str, &str) + 'static) -> Self {
        Self {
            on_change: Some(Rc::new(f)),
            ..self
        }
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.items.iter().position(|i| i.name == name)
    }

    fn words(&self, i: usize) -> String {
        self.fields.get(i).map(TextField::text).unwrap_or_default()
    }

    fn answer(&self, i: usize) -> ItemAnswer {
        self.answers.get().get(i).cloned().unwrap_or_default()
    }

    fn set_answer(&self, i: usize, f: impl FnOnce(ItemAnswer) -> ItemAnswer) {
        self.answers.update(|mut all| {
            if let Some(a) = all.get_mut(i) {
                *a = f(std::mem::take(a));
            }
            all
        });
    }

    fn is_disabled(&self, i: usize) -> bool {
        self.disabled.get().get(i).copied().unwrap_or(true)
    }

    /// The items taking part, in order.
    fn enabled(&self) -> Vec<usize> {
        let disabled = self.disabled.get();
        (0..self.items.len())
            .filter(|&i| !disabled.get(i).copied().unwrap_or(true))
            .collect()
    }

    /// The open item's place among the enabled ones.
    fn position(&self) -> Option<usize> {
        let active = self.active.get();
        self.enabled().iter().position(|&i| i == active)
    }

    /// How many questions take part.
    #[must_use]
    pub fn total(&self) -> usize {
        self.enabled().len()
    }

    /// The open question's place, from 1 (0 when none is open).
    #[must_use]
    pub fn current(&self) -> usize {
        self.position().map_or(0, |p| p.saturating_add(1))
    }

    fn first(&self) -> bool {
        self.total() > 0 && self.position() == Some(0)
    }

    fn last(&self) -> bool {
        self.total() > 0 && self.current() == self.total()
    }

    fn active_item(&self) -> Option<&QuestionnaireItem> {
        self.items.get(self.active.get())
    }

    /// The open question's name (`v-model:item`).
    #[must_use]
    pub fn item(&self) -> Option<String> {
        self.active_item().map(|i| i.name.clone())
    }

    /// Open question `name` (`v-model:item`), when it takes part.
    pub fn set_item(&self, name: &str) {
        if let Some(i) = self.index(name).filter(|&i| !self.is_disabled(i)) {
            self.open(i);
        }
    }

    /// Change the keys that pick a choice (`:shortcuts`).
    pub fn set_shortcuts(&self, shortcuts: Option<QuestionnaireShortcuts>) {
        self.shortcuts.set(shortcuts);
    }

    /// Leave question `name` out, or bring it back (`:disabled`).
    pub fn set_disabled(&self, name: &str, disabled: bool) {
        let Some(i) = self.index(name) else { return };
        if self.is_disabled(i) == disabled {
            return;
        }
        self.disabled.update(|mut all| {
            if let Some(d) = all.get_mut(i) {
                *d = disabled;
            }
            all
        });
    }

    /// Mark question `name` invalid from outside, or clear it (`:invalid`).
    pub fn set_invalid(&self, name: &str, invalid: bool) {
        if let Some(i) = self.index(name) {
            self.set_answer(i, |a| a.flag(invalid));
        }
    }

    #[must_use]
    pub fn status(&self, name: &str) -> Option<QuestionnaireStatus> {
        let i = self.index(name)?;
        Some(self.answer(i).status(self.items.get(i)?, &self.words(i)))
    }

    /// The open question's status (the actions' `data-status`).
    #[must_use]
    pub fn active_status(&self) -> Option<QuestionnaireStatus> {
        self.item().and_then(|name| self.status(&name))
    }

    fn valid(&self, i: usize) -> bool {
        let disabled = self.is_disabled(i);
        self.items
            .get(i)
            .is_some_and(|item| self.answer(i).valid(item, &self.words(i), disabled))
    }

    fn invalid(&self, i: usize) -> bool {
        let disabled = self.is_disabled(i);
        self.items
            .get(i)
            .is_some_and(|item| self.answer(i).invalid(item, &self.words(i), disabled))
    }

    /// Show question `i` and move focus into it (Vue focuses its fieldset),
    /// so the questionnaire's keys keep working.
    fn open(&self, i: usize) {
        self.active.set(i);
        if let Some(first) = self.choice_focus.get(i).and_then(|f| f.first()) {
            first.focus();
        }
    }

    /// Open the enabled question at `place` in the order.
    fn open_at(&self, place: usize) {
        if let Some(&i) = self.enabled().get(place) {
            self.open(i);
        }
    }

    /// Check or uncheck a choice of `name`.
    pub fn choose(&self, name: &str, value: &str) {
        let Some(i) = self.index(name) else { return };
        let Some(item) = self.items.get(i) else {
            return;
        };
        let words = self.words(i);
        self.set_answer(i, |a| a.choose(item, &words, value));
        if let Some(f) = &self.on_change {
            f(self, name, value);
        }
    }

    /// Back one question.
    pub fn previous(&self) {
        if let Some(p) = self.position().and_then(|p| p.checked_sub(1)) {
            self.open_at(p);
        }
    }

    /// On to the next question, once this one is answered (or may be skipped).
    pub fn next(&self) {
        let (Some(p), i) = (self.position(), self.active.get()) else {
            return;
        };
        if self.last() {
            return;
        }
        if self.valid(i) {
            self.open_at(p.saturating_add(1));
        } else {
            self.set_answer(i, ItemAnswer::attempt);
        }
    }

    /// Skip an optional question; on the last one, submit.
    pub fn skip(&self) {
        let i = self.active.get();
        if self.items.get(i).is_none_or(|item| item.required) {
            return;
        }
        let words = self.words(i);
        self.set_answer(i, |a| a.skip(&words));
        if self.last() {
            self.submit();
        } else if let Some(p) = self.position() {
            self.open_at(p.saturating_add(1));
        }
    }

    /// Next, or submit on the last question (Enter, ⌘/Ctrl+Enter).
    pub fn confirm(&self) {
        if !self.valid(self.active.get()) {
            self.set_answer(self.active.get(), ItemAnswer::attempt);
        } else if self.last() {
            self.submit();
        } else {
            self.next();
        }
    }

    /// Hand every answer to `on_submit`, or open the first question that
    /// still needs one. Items left out submit nothing, as a disabled
    /// fieldset's inputs are not in the form data.
    pub fn submit(&self) {
        let enabled = self.enabled();
        if let Some(&i) = enabled.iter().find(|&&i| !self.valid(i)) {
            self.set_answer(i, ItemAnswer::attempt);
            self.open(i);
            return;
        }
        let answers = self.answers.get();
        let submitted = enabled
            .iter()
            .filter_map(|&i| {
                let (item, a) = (self.items.get(i)?, answers.get(i)?);
                Some(QuestionnaireAnswer {
                    name: item.name.clone(),
                    values: a.values(item, &self.words(i)),
                })
            })
            .collect();
        (self.on_submit)(self, submitted);
    }

    /// Back to the default answers and the opening question (a form reset).
    pub fn reset(&self) {
        self.answers.set(self.items.iter().map(ItemAnswer::initial).collect());
        for (field, item) in self.fields.iter().zip(self.items.iter()) {
            field.set_text(item.default_value.clone());
        }
        let start = opening(&self.items, &self.disabled.get(), self.default_item.as_deref());
        self.open(start);
    }

    /// The questionnaire's keys, seen once the focus is inside it: arrows
    /// move between questions and choices, ⌘/Ctrl+Enter confirms, and a
    /// shortcut key picks its choice. Keys a text box takes never get here.
    fn key(&self, ev: &KeyEvent) -> bool {
        let m = ev.modifiers;
        if ev.is(NamedKey::Enter) && (m.command || m.control) && !m.alt && !m.shift {
            self.confirm();
            return true;
        }
        if m.command || m.control || m.alt {
            return false;
        }
        let i = self.active.get();
        if ev.is(NamedKey::ArrowLeft) {
            self.previous();
            return true;
        }
        if ev.is(NamedKey::ArrowRight) {
            if self.active_status() != Some(QuestionnaireStatus::Unanswered) {
                self.next();
            }
            return true;
        }
        if ev.is(NamedKey::ArrowDown) || ev.is(NamedKey::ArrowUp) {
            return self.move_choice_focus(i, ev.is(NamedKey::ArrowDown));
        }
        let typed = ev.text.as_deref().and_then(|t| t.chars().next());
        let picked = self.shortcuts.get().zip(typed).and_then(|(s, c)| s.index(c));
        let Some((k, item)) = picked.zip(self.active_item()) else {
            return false;
        };
        let Some(value) = item.choices.get(k) else {
            return false;
        };
        if let Some(h) = self.choice_focus.get(i).and_then(|f| f.get(k)) {
            gui::focus::set_focus(Some(h.id()), true);
        }
        self.choose(&item.name, value);
        true
    }

    fn move_choice_focus(&self, i: usize, down: bool) -> bool {
        let Some(handles) = self.choice_focus.get(i) else {
            return false;
        };
        let Some(at) = handles.iter().position(|h| h.is_focused()) else {
            return false;
        };
        let to = if down { at.checked_add(1) } else { at.checked_sub(1) };
        match to.and_then(|t| handles.get(t)) {
            Some(h) => {
                gui::focus::set_focus(Some(h.id()), true);
                true
            }
            None => false,
        }
    }
}

/// The item a questionnaire opens on: `default_item` when it takes part,
/// else the first that does (Vue's `getInitialItemName`).
fn opening(items: &[QuestionnaireItem], disabled: &[bool], default_item: Option<&str>) -> usize {
    let enabled = |i: &usize| !disabled.get(*i).copied().unwrap_or(true);
    default_item
        .and_then(|name| items.iter().position(|i| i.name == name))
        .filter(enabled)
        .or_else(|| (0..items.len()).find(enabled))
        .unwrap_or(0)
}
/// The progress box's least width: `min-w-[14ch]`, fourteen of Inter's `0`
/// at `text-xs` (108.406px in Chrome, QuestionnaireCardDemo).
const PROGRESS_MIN_WIDTH: f32 = 108.406;

/// The form: a column holding progress, the items and the actions; it
/// takes the questionnaire's keys.
#[must_use]
pub fn questionnaire(q: &Questionnaire) -> Div {
    let q = q.clone();
    div()
        .named("questionnaire")
        .flex_col()
        .w_full()
        .min_w(0.0)
        .gap(6.0)
        .on_key_down(move |ev| q.key(ev))
}

/// "Question 2 of 3".
#[must_use]
pub fn questionnaire_progress(q: &Questionnaire) -> Div {
    questionnaire_progress_with(format!("Question {} of {}", q.current(), q.total()))
}

/// The progress box around `content` of one's own (Vue's progress slot,
/// built from [`Questionnaire::current`] and [`Questionnaire::total`]).
#[must_use]
pub fn questionnaire_progress_with(content: impl IntoElement) -> Div {
    let t = theme();
    div()
        .named("questionnaire-progress")
        .self_start()
        .min_w_px(PROGRESS_MIN_WIDTH)
        .text_xs()
        .font_medium()
        .text_color(t.muted_foreground)
        .tabular_nums()
        .child(content)
}

/// One question; `None` unless it is the open one (Vue hides the rest).
/// Fill it with title, description, choices and error.
#[must_use]
pub fn questionnaire_item(q: &Questionnaire, name: &str) -> Option<Div> {
    let i = q.index(name)?;
    (i == q.active.get() && !q.is_disabled(i)).then(|| div().named("questionnaire-item").flex_col().gap(5.0).min_w(0.0))
}

/// The question.
#[must_use]
pub fn questionnaire_title(title: impl Into<String>) -> Text {
    text(title)
        .named("questionnaire-title")
        .text_base()
        .font_semibold()
        .text_pretty()
}

/// Help under the question. The title is the item's legend, outside the
/// item's gap, so the description sits straight under it.
#[must_use]
pub fn questionnaire_description(description: impl Into<String>) -> Text {
    text(description)
        .named("questionnaire-description")
        .mt(-5.0)
        .text_sm()
        .text_pretty()
        .text_color(theme().muted_foreground)
}

/// The column of [`questionnaire_choice`]s and the [`questionnaire_input`].
#[must_use]
pub fn questionnaire_choices() -> Div {
    div().named("questionnaire-choices").grid().gap(3.0).min_w(0.0)
}

/// A choice of question `name` with `value`: a box (radio or checkbox),
/// its `label` with an optional muted second line, and its shortcut key.
#[must_use]
pub fn questionnaire_choice(
    q: &Questionnaire,
    name: &str,
    value: &str,
    label: impl IntoElement,
    description: Option<&str>,
) -> Div {
    let t = theme();
    let i = q.index(name).unwrap_or_default();
    let item = q.items.get(i).cloned();
    let k = item
        .as_ref()
        .and_then(|it| it.choices.iter().position(|v| v == value))
        .unwrap_or_default();
    let multiple = item.as_ref().is_some_and(|it| it.multiple);
    let words = q.words(i);
    let checked = item.as_ref().is_some_and(|it| q.answer(i).checked(it, &words, value));
    let invalid = q.invalid(i);
    let shortcut = q.shortcuts.get().and_then(|s| s.key(k));
    let focus = q.choice_focus.get(i).and_then(|f| f.get(k)).copied();

    // Vue's `translate-y-[--spacing(0.45)]` (`translate-y-0.5` beside a
    // description): moved down without taking room, as a relative offset is.
    let nudge = if description.is_some() { 0.5 } else { 0.45 };
    let indicator = div()
        .named("questionnaire-choice-indicator")
        .flex()
        .size(4.0)
        .shrink_0()
        .relative()
        .top(nudge)
        .items_center()
        .justify_center()
        .border()
        .rounded(if multiple { 4.0 } else { 9999.0 })
        .border_color(if checked { t.primary } else { t.input })
        .when(checked, |d| d.bg(t.primary).text_color(t.primary_foreground))
        .when(!checked && t.dark, |d| d.bg(t.input.opacity(0.3)))
        .when(checked && multiple, |d| d.child(icon(Icon::Check).size(3.5)))
        .when(checked && !multiple, |d| {
            d.child(div().size(2.0).rounded_full().bg(t.primary_foreground))
        });
    let key = shortcut.map(|c| {
        div()
            .named("questionnaire-choice-shortcut")
            .flex()
            .size(5.0)
            .shrink_0()
            .relative()
            .top(nudge)
            .ml_auto()
            .items_center()
            .justify_center()
            .rounded_md()
            .border()
            .border_color(t.input)
            .bg(t.background)
            .shadow_xs()
            .font_mono()
            .font_medium()
            .text_size(10.0, 10.0)
            .text_color(t.muted_foreground)
            .child(c.to_string())
    });

    let (q2, name2, value2) = (q.clone(), name.to_owned(), value.to_owned());
    let (q3, name3, value3) = (q.clone(), name.to_owned(), value.to_owned());
    div()
        .named("questionnaire-choice")
        .transition_colors()
        .relative()
        .flex()
        .items_start()
        .gap(3.0)
        .min_h(11.0)
        .px(4.0)
        .py(3.5)
        .rounded_md()
        .border()
        .shadow_xs()
        .text_sm()
        .text_left()
        .cursor_pointer()
        .border_color(match (invalid, checked) {
            (true, _) => t.destructive,
            (false, true) => t.primary.opacity(0.4),
            (false, false) => t.input,
        })
        .when(checked, |d| d.bg(t.muted))
        .when(!checked && t.dark, |d| d.bg(t.input.opacity(0.2)))
        .when(!checked, |d| d.hover(move |s| s.bg(t.muted.opacity(0.5))))
        .when_some(focus, Div::track_focus)
        .focus_visible(move |s| focus_ring(s, &t))
        .on_click(move |_| q2.choose(&name2, &value2))
        // Enter on a checked choice moves on, as Vue's answer Enter does.
        .on_key_down(move |ev| {
            if !ev.is(NamedKey::Enter) || ev.modifiers.command || ev.modifiers.control {
                return false;
            }
            let i = q3.index(&name3).unwrap_or_default();
            let words = q3.words(i);
            let checked = q3
                .items
                .get(i)
                .is_some_and(|it| q3.answer(i).checked(it, &words, &value3));
            if checked {
                q3.confirm();
            }
            true
        })
        .child(indicator)
        .child(
            div()
                .named("questionnaire-choice-label")
                .flex_col()
                .flex_1()
                .min_w(0.0)
                .gap(1.0)
                .leading_snug()
                .child(label)
                .child(description.map(choice_description)),
        )
        .child(key)
}

/// A choice's second line, muted.
fn choice_description(description: &str) -> Text {
    text(description)
        .named("questionnaire-choice-description")
        .text_color(theme().muted_foreground)
}

/// The text box for an answer of one's own, after the choices of an item
/// declared [`QuestionnaireItem::with_input`].
#[must_use]
pub fn questionnaire_input(q: &Questionnaire, name: &str) -> Option<TextInput> {
    let t = theme();
    let i = q.index(name)?;
    let field = q.fields.get(i)?;
    let invalid = q.invalid(i);
    Some(
        input(field)
            .named("questionnaire-input")
            .transition_props(Properties::COLOR | Properties::SHADOW | Properties::BACKGROUND)
            .selection_color(t.primary)
            .when(invalid, |d| {
                d.border_color(t.destructive)
                    .ring(3.0, t.destructive.opacity(if t.dark { 0.4 } else { 0.2 }))
            })
            .when(invalid, |d| d.focus_visible(move |s| destructive_ring(s, &t))),
    )
}

/// What is missing, shown once moving on was tried without an answer.
#[must_use]
pub fn questionnaire_error(q: &Questionnaire, name: &str) -> Option<Text> {
    let required = q.items.get(q.index(name)?)?.required;
    let message = if required {
        "Choose an answer to continue."
    } else {
        "Choose an answer or skip this question."
    };
    questionnaire_error_with(q, name, message)
}

/// The error with a message of one's own (Vue's error slot), shown while
/// question `name` is invalid; an empty message keeps an empty line.
#[must_use]
pub fn questionnaire_error_with(q: &Questionnaire, name: &str, message: impl Into<String>) -> Option<Text> {
    let i = q.index(name)?;
    q.invalid(i).then(|| {
        text(message)
            .named("questionnaire-error")
            .text_sm()
            .text_color(theme().destructive)
    })
}

/// The row of previous, skip and next (or submit), each in its own column.
#[must_use]
pub fn questionnaire_actions() -> Div {
    div()
        .named("questionnaire-actions")
        .grid()
        .grid_cols_tracks(vec![gui::Track::Fr(1.0), gui::Track::Auto, gui::Track::Auto])
        .items_center()
        .w_full()
        .min_h(9.0)
        .gap(2.0)
}

/// Back one question; hidden on the first.
#[must_use]
pub fn questionnaire_previous(q: &Questionnaire, label: &str) -> Option<Div> {
    let q2 = q.clone();
    (q.total() > 1 && !q.first()).then(|| {
        let mut b = button(ButtonVariant::Outline, ButtonSize::Default)
            .named("questionnaire-previous")
            .col_start(1)
            .row_start(1)
            .child(label.to_owned())
            .on_click(move |_| q2.previous());
        b.style_mut().justify_self = Some(Align::Start);
        b
    })
}

/// Skip this question; shown only on optional questions.
#[must_use]
pub fn questionnaire_skip(q: &Questionnaire, label: &str) -> Option<Div> {
    let q2 = q.clone();
    let optional = q.active_item().is_some_and(|it| !it.required);
    optional.then(|| {
        button(ButtonVariant::Outline, ButtonSize::Default)
            .named("questionnaire-skip")
            .col_start(2)
            .row_start(1)
            .justify_self_end()
            .child(label.to_owned())
            .on_click(move |_| q2.skip())
    })
}

/// On to the next question; hidden on the last.
#[must_use]
pub fn questionnaire_next(q: &Questionnaire, label: &str) -> Option<Div> {
    questionnaire_next_with(q, label, ButtonVariant::Default, false)
}

/// [`questionnaire_next`] in another `variant`, or `disabled`.
#[must_use]
pub fn questionnaire_next_with(q: &Questionnaire, label: &str, variant: ButtonVariant, disabled: bool) -> Option<Div> {
    let q2 = q.clone();
    (q.total() > 1 && !q.last()).then(|| {
        button(variant, ButtonSize::Default)
            .named("questionnaire-next")
            .col_start(3)
            .row_start(1)
            .justify_self_end()
            .child(label.to_owned())
            .disabled(disabled)
            .on_click(move |_| q2.next())
    })
}

/// Submit every answer; shown only on the last question.
#[must_use]
pub fn questionnaire_submit(q: &Questionnaire, label: &str) -> Option<Div> {
    questionnaire_submit_with(q, label, ButtonVariant::Default, false)
}

/// [`questionnaire_submit`] in another `variant`, or `disabled`.
#[must_use]
pub fn questionnaire_submit_with(
    q: &Questionnaire,
    label: &str,
    variant: ButtonVariant,
    disabled: bool,
) -> Option<Div> {
    let q2 = q.clone();
    (q.total() > 0 && q.last()).then(|| {
        button(variant, ButtonSize::Default)
            .named("questionnaire-submit")
            .col_start(3)
            .row_start(1)
            .justify_self_end()
            .child(label.to_owned())
            .disabled(disabled)
            .on_click(move |_| q2.submit())
    })
}
