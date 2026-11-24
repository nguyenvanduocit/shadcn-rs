//! Questionnaire (`ui/questionnaire`): questions asked one at a time, each
//! answered by choosing (one, or several) or by typing, with previous, skip,
//! next and submit, progress, validation and keyboard shortcuts.
//!
//! Vue registers items and answers as they mount; here the questions are
//! declared up front ([`QuestionnaireItem`], Vue's `items` prop) and their
//! answers live in the [`questionnaire`] instance, which its parts read (a
//! [`Questionnaire`] is what they and its callbacks see). The open question
//! is its own or the caller's (`v-model:item`). The rules (status,
//! validity, what is submitted) are pure functions on [`ItemAnswer`].

use std::fmt;
use std::rc::Rc;

use gui::prelude::*;
use gui::style::Align;
use gui::{
    Bind, Callback, ComponentElement, Div, Element, FocusHandle, Icon, KeyEvent, Model, NamedKey, Signal, Text,
    TextField, UseModel, component, div, icon, provide, text, use_model,
};
use gui::{Color, Properties};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::focus::{StateLook, destructive_ring};
use crate::input::input;
use std::panic::Location;

use primitives::context::use_root_at;

use crate::part::{Part, PartProps, part_at};
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

/// What [`questionnaire`] is given: Vue's `items`, `default-item` and
/// `v-model:item` (one [`Bind`]), `shortcuts`, `@submit`, `@reset` and each
/// choice's `@change`.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestionnaireProps {
    items: Vec<QuestionnaireItem>,
    /// The open question's name: `default-item`, or `v-model:item`.
    item: Bind<String>,
    shortcuts: Option<QuestionnaireShortcuts>,
    on_submit: Option<Callback<(Questionnaire, Vec<QuestionnaireAnswer>)>>,
    on_change: Option<Callback<(Questionnaire, String, String)>>,
    on_reset: Option<Callback<Questionnaire>>,
}

/// A questionnaire's state and commands, as its parts (and its callbacks)
/// see it. It is the [`questionnaire`] instance's: made in its setup, it
/// lives while the questionnaire is mounted.
#[derive(Clone)]
pub struct Questionnaire {
    props: Signal<PartProps<QuestionnaireProps>>,
    items: Rc<[QuestionnaireItem]>,
    /// The open question's name (`v-model:item`); a name that is not an
    /// enabled question's opens `start`.
    item: UseModel<PartProps<QuestionnaireProps>, String>,
    /// The question it opens on, and goes back to on reset (Vue's
    /// `getInitialItemName`).
    start: usize,
    answers: Signal<Vec<ItemAnswer>>,
    /// Which items are left out (Vue's `disabled` on an item definition).
    disabled: Signal<Vec<bool>>,
    /// One per question; only those with `input` show theirs.
    fields: Rc<[TextField]>,
    /// Each question's choices, to move between them with the arrow keys.
    choice_focus: Rc<[Vec<FocusHandle>]>,
}

impl fmt::Debug for Questionnaire {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Questionnaire").finish_non_exhaustive()
    }
}

impl PartialEq for Questionnaire {
    fn eq(&self, other: &Self) -> bool {
        self.answers == other.answers
    }
}

impl Questionnaire {
    /// Made in the questionnaire's setup from its props.
    fn new(props: Signal<PartProps<QuestionnaireProps>>) -> Self {
        let items = props.with_untracked(|p| p.props.items.clone());
        let disabled: Vec<bool> = items.iter().map(|i| i.disabled).collect();
        let item = use_model(props, |p| &p.props.item);
        let start = opening(&items, &disabled, Some(&item.get_untracked()));
        let answers = items.iter().map(ItemAnswer::initial).collect();
        let fields = items.iter().map(|i| TextField::new(i.default_value.clone())).collect();
        let choice_focus = items
            .iter()
            .map(|i| i.choices.iter().map(|_| FocusHandle::new()).collect())
            .collect();
        Self {
            props,
            items: items.into(),
            item,
            start,
            answers: Signal::labeled("questionnaire answers", answers),
            disabled: Signal::labeled("questionnaire disabled items", disabled),
            fields,
            choice_focus,
        }
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.items.iter().position(|i| i.name == name)
    }

    fn shortcuts(&self) -> Option<QuestionnaireShortcuts> {
        self.props.with(|p| p.props.shortcuts)
    }

    /// The open item's index.
    fn active(&self) -> usize {
        self.item
            .with(|name| self.index(name))
            .filter(|&i| !self.is_disabled(i))
            .unwrap_or(self.start)
    }

    fn words(&self, i: usize) -> String {
        self.fields.get(i).map(TextField::text).unwrap_or_default()
    }

    fn answer(&self, i: usize) -> ItemAnswer {
        self.answers.with(|a| a.get(i).cloned()).unwrap_or_default()
    }

    fn set_answer(&self, i: usize, f: impl FnOnce(ItemAnswer) -> ItemAnswer) {
        self.answers.update(|all| {
            if let Some(a) = all.get_mut(i) {
                *a = f(std::mem::take(a));
            }
        });
    }

    fn is_disabled(&self, i: usize) -> bool {
        self.disabled.with(|d| d.get(i).copied()).unwrap_or(true)
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
        let active = self.active();
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
        self.items.get(self.active())
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

    /// Leave question `name` out, or bring it back (`:disabled`).
    pub fn set_disabled(&self, name: &str, disabled: bool) {
        let Some(i) = self.index(name) else { return };
        if self.is_disabled(i) == disabled {
            return;
        }
        self.disabled.update(|all| {
            if let Some(d) = all.get_mut(i) {
                *d = disabled;
            }
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
        if let Some(item) = self.items.get(i) {
            self.item.set(item.name.clone());
        }
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
        if let Some(f) = self.props.with_untracked(|p| p.props.on_change.clone()) {
            f.call((self.clone(), name.to_owned(), value.to_owned()));
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
        let (Some(p), i) = (self.position(), self.active()) else {
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
        let i = self.active();
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
        let i = self.active();
        if !self.valid(i) {
            self.set_answer(i, ItemAnswer::attempt);
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
        let answers = self.answers.get_untracked();
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
        if let Some(f) = self.props.with_untracked(|p| p.props.on_submit.clone()) {
            f.call((self.clone(), submitted));
        }
    }

    /// Back to the default answers and the opening question (a form reset),
    /// then `on_reset`.
    pub fn reset(&self) {
        self.answers.set(self.items.iter().map(ItemAnswer::initial).collect());
        for (field, item) in self.fields.iter().zip(self.items.iter()) {
            field.set_text(item.default_value.clone());
        }
        self.open(self.start);
        if let Some(f) = self.props.with_untracked(|p| p.props.on_reset.clone()) {
            f.call(self.clone());
        }
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
        let i = self.active();
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
        let picked = self.shortcuts().zip(typed).and_then(|(s, c)| s.index(c));
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

/// The questionnaire around a part (Vue's `injectQuestionnaireContext`).
/// Called from the setup of the part `part`, made at `at` in the caller's
/// code: outside a `questionnaire` it is a reported error and `None`.
fn use_questionnaire(part: &'static str, at: &'static Location<'static>) -> Option<Questionnaire> {
    use_root_at::<Questionnaire>(part, "questionnaire", at)
}

/// The question the part `part`, made at `at`, sits in: outside a
/// `questionnaire_item` it is a reported error and `None`.
fn use_item(part: &'static str, at: &'static Location<'static>) -> Option<ItemContext> {
    use_root_at::<ItemContext>(part, "questionnaire_item", at)
}

/// The question a part sits in (Vue's item context): its name.
#[derive(Clone)]
struct ItemContext(String);

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
/// keeps the answers and the open question, and takes the questionnaire's
/// keys. `items` are its questions in order (Vue's `items`).
#[must_use]
pub fn questionnaire(items: Vec<QuestionnaireItem>) -> Part<QuestionnaireProps> {
    let props = QuestionnaireProps {
        items,
        item: Bind::default_value(String::new()),
        shortcuts: None,
        on_submit: None,
        on_change: None,
        on_reset: None,
    };
    let root = div()
        .named("questionnaire")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .min_width(0.0)
        .gap(24.0);
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let q = Questionnaire::new(props);
            provide(q.clone());
            move || {
                let q = q.clone();
                props
                    .with(|p| p.slot.0.clone())
                    .on_key_down(move |ev| q.key(ev))
                    .into_element()
            }
        })
    })
}

impl Part<QuestionnaireProps> {
    /// The question it opens on, when it takes part (Vue's `default-item`).
    #[must_use]
    pub fn default_item(self, name: &str) -> Self {
        let name = name.to_owned();
        self.with_props(|p| p.item = p.item.clone().with_default(name))
    }

    /// The open question's name, kept by the caller both ways
    /// (`v-model:item`).
    #[must_use]
    pub fn item(self, model: Model<String>) -> Self {
        self.with_props(|p| p.item = p.item.clone().with_model(model))
    }

    /// The open question's name as the caller shows it (`:item`): a move to
    /// another question only calls [`Self::on_item_change`].
    #[must_use]
    pub fn item_value(self, name: &str) -> Self {
        let name = name.to_owned();
        self.with_props(|p| p.item = p.item.clone().with_value(name))
    }

    /// Called with the name of each question opened (`@update:item`).
    #[must_use]
    pub fn on_item_change(self, f: impl Into<gui::Callback<String>>) -> Self {
        self.with_props(|p| p.item = p.item.clone().on_change(f))
    }

    /// The keys that pick a choice (`:shortcuts`), read as they change.
    #[must_use]
    pub fn shortcuts(self, shortcuts: Option<QuestionnaireShortcuts>) -> Self {
        self.with_props(|p| p.shortcuts = shortcuts)
    }

    /// Called with the questionnaire and every answer once all are valid (`@submit`).
    #[must_use]
    pub fn on_submit(self, f: impl Into<Callback<(Questionnaire, Vec<QuestionnaireAnswer>)>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.on_submit = Some(f))
    }

    /// Called with the questionnaire, the item's name and the choice's value
    /// each time a choice is checked or unchecked (a choice's `@change`).
    #[must_use]
    pub fn on_change(self, f: impl Into<Callback<(Questionnaire, String, String)>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.on_change = Some(f))
    }

    /// Called after the answers are reset (`@reset`).
    #[must_use]
    pub fn on_reset(self, f: impl Into<Callback<Questionnaire>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.on_reset = Some(f))
    }
}

/// Where the questionnaire stands, for a progress of one's own (the
/// progress slot's props).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuestionnaireProgressState {
    /// The open question's place, from 1.
    pub current: usize,
    pub total: usize,
}

/// What a part rendered from the questionnaire's state is given: a view of
/// it, made anew by each render of the parent (a slot).
#[derive(Clone)]
pub struct QuestionnaireView<S: 'static>(Rc<dyn Fn(S) -> Element>);

impl<S: 'static> PartialEq for QuestionnaireView<S> {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

/// "Question 2 of 3".
#[must_use]
#[track_caller]
pub fn questionnaire_progress() -> Part<QuestionnaireView<QuestionnaireProgressState>> {
    questionnaire_progress_with(|s| text(format!("Question {} of {}", s.current, s.total)).into_element())
}

/// The progress box around what `content` makes of the questionnaire's
/// place (Vue's progress slot, with its `current` and `total`).
#[must_use]
#[track_caller]
pub fn questionnaire_progress_with(
    content: impl Fn(QuestionnaireProgressState) -> Element + 'static,
) -> Part<QuestionnaireView<QuestionnaireProgressState>> {
    let t = theme();
    let root = div()
        .named("questionnaire-progress")
        .align_self(Align::Start)
        .min_width(PROGRESS_MIN_WIDTH)
        .text_size(12.0)
        .line_height(16.0)
        .font_weight(500)
        .text_color(t.muted_foreground)
        .tabular_nums(true);
    Part::new(QuestionnaireView(Rc::new(content)), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let q = use_questionnaire("questionnaire_progress", part_at(props));
            move || {
                let state = q
                    .as_ref()
                    .map_or(QuestionnaireProgressState { current: 0, total: 0 }, |q| {
                        QuestionnaireProgressState {
                            current: q.current(),
                            total: q.total(),
                        }
                    });
                props
                    .with(|p| p.slot.0.clone().child((p.props.0)(state)))
                    .into_element()
            }
        })
    })
}

/// What a part of one question is given: the question's name.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestionnaireItemProps {
    name: String,
}

/// One question, shown while it is the open one (Vue hides the rest). Fill
/// it with title, description, choices and error: they belong to it.
#[must_use]
#[track_caller]
pub fn questionnaire_item(name: &str) -> Part<QuestionnaireItemProps> {
    let root = div()
        .named("questionnaire-item")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(20.0)
        .min_width(0.0);
    Part::new(QuestionnaireItemProps { name: name.to_owned() }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let name = props.with_untracked(|p| p.props.name.clone());
            provide(ItemContext(name));
            let q = use_questionnaire("questionnaire_item", part_at(props));
            move || {
                props.with(|p| {
                    let shown = q.as_ref().is_some_and(|q| {
                        q.index(&p.props.name)
                            .is_some_and(|i| i == q.active() && !q.is_disabled(i))
                    });
                    if shown {
                        p.slot.0.clone().into_element()
                    } else {
                        Element::Empty
                    }
                })
            }
        })
    })
}

/// The question.
#[must_use]
pub fn questionnaire_title(title: impl Into<String>) -> Text {
    text(title)
        .named("questionnaire-title")
        .text_size(16.0)
        .line_height(24.0)
        .font_weight(600)
        .text_wrap(WrapStyle::Pretty)
}

/// Help under the question. The title is the item's legend, outside the
/// item's gap, so the description sits straight under it.
#[must_use]
pub fn questionnaire_description(description: impl Into<String>) -> Text {
    text(description)
        .named("questionnaire-description")
        .margin_top(-20.0)
        .text_size(14.0)
        .line_height(20.0)
        .text_wrap(WrapStyle::Pretty)
        .text_color(theme().muted_foreground)
}

/// The column of [`questionnaire_choice`]s and the [`questionnaire_input`].
#[must_use]
pub fn questionnaire_choices() -> Div {
    div()
        .named("questionnaire-choices")
        .display(Display::Grid)
        .gap(12.0)
        .min_width(0.0)
}

/// What a choice is given. Its label is the parent's slot, made anew by
/// each render: never equal.
#[derive(Clone)]
pub struct QuestionnaireChoiceProps {
    value: String,
    label: Element,
    description: Option<String>,
    /// Where the caller's code made it (a report of a missing root names it).
    at: &'static Location<'static>,
}

impl PartialEq for QuestionnaireChoiceProps {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

/// A choice of the question it sits in, with `value`: a box (radio or
/// checkbox), its `label` with an optional muted second line, and its
/// shortcut key.
#[must_use]
#[track_caller]
pub fn questionnaire_choice(value: &str, label: impl IntoElement, description: Option<&str>) -> ComponentElement {
    let props = QuestionnaireChoiceProps {
        value: value.to_owned(),
        label: label.into_element(),
        description: description.map(str::to_owned),
        at: Location::caller(),
    };
    component(props, |cx| {
        let props = cx.props();
        let at = props.with_untracked(|p| p.at);
        let q = use_questionnaire("questionnaire_choice", at);
        let item = use_item("questionnaire_choice", at);
        move || {
            let (Some(q), Some(ItemContext(name))) = (&q, &item) else {
                return Element::Empty;
            };
            props.with(|p| choice(q, name, p)).into_element()
        }
    })
}

fn choice(q: &Questionnaire, name: &str, p: &QuestionnaireChoiceProps) -> Div {
    let t = theme();
    let (value, description) = (p.value.as_str(), p.description.as_deref());
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
    let shortcut = q.shortcuts().and_then(|s| s.key(k));
    let focus = q.choice_focus.get(i).and_then(|f| f.get(k)).copied();

    // Vue's `translate-y-[--spacing(0.45)]` (`translate-y-0.5` beside a
    // description): moved down without taking room, as a relative offset is.
    let nudge = if description.is_some() { 2.0 } else { 1.8 };
    let indicator = div()
        .named("questionnaire-choice-indicator")
        .display(Display::Flex)
        .size(16.0)
        .flex_shrink(0.0)
        .position(Position::Relative)
        .top(nudge)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .border(1.0)
        .radius(if multiple { 4.0 } else { 9999.0 })
        .border_color(if checked { t.primary } else { t.input })
        .when(checked, |d| d.background(t.primary).text_color(t.primary_foreground))
        .when(!checked && t.dark, |d| d.background(t.input.opacity(0.3)))
        .when(checked && multiple, |d| d.child(icon(Icon::Check).size(14.0)))
        .when(checked && !multiple, |d| {
            d.child(div().size(8.0).radius(9999.0).background(t.primary_foreground))
        });
    let key = shortcut.map(|c| {
        div()
            .named("questionnaire-choice-shortcut")
            .display(Display::Flex)
            .size(20.0)
            .flex_shrink(0.0)
            .position(Position::Relative)
            .top(nudge)
            .margin_left(Length::Auto)
            .align_items(Align::Center)
            .justify_content(Justify::Center)
            .radius(8.0)
            .border(1.0)
            .border_color(t.input)
            .background(t.background)
            .shadow(Shadow {
                x: 0.0,
                y: 1.0,
                blur: 2.0,
                spread: 0.0,
                color: Color::BLACK.opacity(0.05),
            })
            .font_family(FontFamily::GEIST_MONO)
            .font_weight(500)
            .text_size(10.0)
            .line_height(10.0)
            .text_color(t.muted_foreground)
            .child(c.to_string())
    });

    let (q2, name2, value2) = (q.clone(), name.to_owned(), value.to_owned());
    let (q3, value3) = (q.clone(), value.to_owned());
    let it = Interaction::new();
    div()
        .named("questionnaire-choice")
        .interaction(it)
        .transition(Properties::COLORS)
        .position(Position::Relative)
        .display(Display::Flex)
        .align_items(Align::Start)
        .gap(12.0)
        .min_height(44.0)
        .padding_x(16.0)
        .padding_y(14.0)
        .radius(8.0)
        .border(1.0)
        .shadow(Shadow {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: 0.0,
            color: Color::BLACK.opacity(0.05),
        })
        .text_size(14.0)
        .line_height(20.0)
        .text_align(TextAlign::Left)
        .cursor(Cursor::Pointer)
        .border_color(match (invalid, checked) {
            (true, _) => t.destructive,
            (false, true) => t.primary.opacity(0.4),
            (false, false) => t.input,
        })
        .when(checked, |d| d.background(t.muted))
        .when(!checked && t.dark, |d| d.background(t.input.opacity(0.2)))
        .when(!checked, |d| {
            d.background_while(move || it.hovered(), t.muted.opacity(0.5))
        })
        .when_some(focus, Div::track_focus)
        .focus_ring_on(it, &t)
        .on_click(move |_| q2.choose(&name2, &value2))
        // Enter on a checked choice moves on, as Vue's answer Enter does.
        .on_key_down(move |ev| {
            if !ev.is(NamedKey::Enter) || ev.modifiers.command || ev.modifiers.control {
                return false;
            }
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
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .flex(Flex::grow(1.0))
                .min_width(0.0)
                .gap(4.0)
                .line_height(LineHeight::Relative(1.375))
                .child(p.label.clone())
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

/// What [`questionnaire_input`] is given.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestionnaireInputProps {
    placeholder: Option<String>,
    /// Where the caller's code made it (a report of a missing root names it).
    at: &'static Location<'static>,
}

/// The text box for an answer of one's own, after the choices of an item
/// declared [`QuestionnaireItem::with_input`].
#[must_use]
#[track_caller]
pub fn questionnaire_input() -> QuestionnaireInput {
    QuestionnaireInput(QuestionnaireInputProps {
        placeholder: None,
        at: Location::caller(),
    })
}

/// A [`questionnaire_input`] being built.
pub struct QuestionnaireInput(QuestionnaireInputProps);

impl QuestionnaireInput {
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.0.placeholder = Some(placeholder.into());
        self
    }
}

impl IntoElement for QuestionnaireInput {
    fn into_element(self) -> Element {
        component(self.0, |cx| {
            let props = cx.props();
            let at = props.with_untracked(|p| p.at);
            let q = use_questionnaire("questionnaire_input", at);
            let item = use_item("questionnaire_input", at);
            move || {
                let t = theme();
                let (Some(q), Some(ItemContext(name))) = (&q, &item) else {
                    return Element::Empty;
                };
                let Some(i) = q.index(name) else {
                    return Element::Empty;
                };
                let Some(field) = q.fields.get(i) else {
                    return Element::Empty;
                };
                let invalid = q.invalid(i);
                input()
                    .field(*field)
                    .named("questionnaire-input")
                    .transition(Properties::COLOR | Properties::SHADOW | Properties::BACKGROUND)
                    .when_some(props.with(|p| p.placeholder.clone()), crate::input::Input::placeholder)
                    .when(invalid, |d| {
                        d.border_color(t.destructive)
                            .ring(Ring::new(3.0, t.destructive.opacity(if t.dark { 0.4 } else { 0.2 })))
                    })
                    .map(move |el| {
                        el.selection_color(t.primary).when(invalid, |d| {
                            d.with_interaction(|d, it| {
                                d.ring_while(move || it.focus_visible(), &destructive_ring(gui::Style::default(), &t))
                            })
                        })
                    })
                    .into_element()
            }
        })
        .into_element()
    }
}

/// What is missing, shown once moving on was tried without an answer.
#[must_use]
#[track_caller]
pub fn questionnaire_error() -> ComponentElement {
    error_part("questionnaire_error", None, Location::caller())
}

/// The error with a message of one's own (Vue's error slot), shown while
/// the question is invalid; an empty message keeps an empty line.
#[must_use]
#[track_caller]
pub fn questionnaire_error_with(message: impl Into<String>) -> ComponentElement {
    error_part("questionnaire_error_with", Some(message.into()), Location::caller())
}

/// The error part, made by the public constructor `part`.
fn error_part(part: &'static str, message: Option<String>, at: &'static Location<'static>) -> ComponentElement {
    component((part, message, at), |cx| {
        let props = cx.props();
        let (part, at) = props.with_untracked(|p| (p.0, p.2));
        let q = use_questionnaire(part, at);
        let item = use_item(part, at);
        move || {
            let (Some(q), Some(ItemContext(name))) = (&q, &item) else {
                return Element::Empty;
            };
            let Some(i) = q.index(name) else {
                return Element::Empty;
            };
            if !q.invalid(i) {
                return Element::Empty;
            }
            let required = q.items.get(i).is_some_and(|it| it.required);
            let message = props.with(|p| p.1.clone()).unwrap_or_else(|| {
                if required {
                    "Choose an answer to continue."
                } else {
                    "Choose an answer or skip this question."
                }
                .to_owned()
            });
            text(message)
                .named("questionnaire-error")
                .text_size(14.0)
                .line_height(20.0)
                .text_color(theme().destructive)
                .into_element()
        }
    })
}

/// The row of previous, skip and next (or submit), each in its own column.
#[must_use]
pub fn questionnaire_actions() -> Div {
    div()
        .named("questionnaire-actions")
        .display(Display::Grid)
        .grid_columns(vec![gui::Track::Fr(1.0), gui::Track::Auto, gui::Track::Auto])
        .align_items(Align::Center)
        .width(pct(100.0))
        .min_height(36.0)
        .gap(8.0)
}

/// Which navigation button a part is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Nav {
    Previous,
    Skip,
    Next,
    Submit,
}

impl Nav {
    /// The public part that makes it.
    fn part(self) -> &'static str {
        match self {
            Self::Previous => "questionnaire_previous",
            Self::Skip => "questionnaire_skip",
            Self::Next => "questionnaire_next",
            Self::Submit => "questionnaire_submit",
        }
    }
}

/// When a navigation button is disabled: from the open question's status
/// (the buttons' slot `status`).
type DisabledWhen = gui::Derive<Option<QuestionnaireStatus>, bool>;

/// What a navigation button is given.
#[derive(Clone)]
pub struct QuestionnaireNavProps {
    nav: Nav,
    label: String,
    variant: ButtonVariant,
    disabled_when: Option<DisabledWhen>,
    /// Where the caller's code made it (a report of a missing root names it).
    at: &'static Location<'static>,
}

impl PartialEq for QuestionnaireNavProps {
    fn eq(&self, other: &Self) -> bool {
        // `disabled_when` compares by pointer: one made anew is not equal.
        self.disabled_when == other.disabled_when
            && (self.nav, &self.label, self.variant) == (other.nav, &other.label, other.variant)
    }
}

/// A navigation button being built.
pub struct QuestionnaireNav(QuestionnaireNavProps);

impl QuestionnaireNav {
    #[track_caller]
    fn new(nav: Nav, label: &str) -> Self {
        Self(QuestionnaireNavProps {
            nav,
            label: label.to_owned(),
            variant: ButtonVariant::Default,
            disabled_when: None,
            at: Location::caller(),
        })
    }

    /// Drawn as `variant` (Vue's `variant` on the button).
    #[must_use]
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.0.variant = variant;
        self
    }

    /// Disabled while `f` says so of the open question's status (Vue's
    /// `v-slot="{ status }" :disabled="…"`).
    #[must_use]
    pub fn disabled_when(mut self, f: impl Into<DisabledWhen>) -> Self {
        self.0.disabled_when = Some(f.into());
        self
    }
}

impl IntoElement for QuestionnaireNav {
    fn into_element(self) -> Element {
        component(self.0, |cx| {
            let props = cx.props();
            let q = use_questionnaire(props.with_untracked(|p| p.nav).part(), props.with_untracked(|p| p.at));
            move || {
                let Some(q) = q.clone() else {
                    return Element::Empty;
                };
                props
                    .with(|p| nav_button(q, p))
                    .map_or(Element::Empty, IntoElement::into_element)
            }
        })
        .into_element()
    }
}

fn nav_button(q: Questionnaire, p: &QuestionnaireNavProps) -> Option<Div> {
    let shown = match p.nav {
        Nav::Previous => q.total() > 1 && !q.first(),
        Nav::Skip => q.active_item().is_some_and(|it| !it.required),
        Nav::Next => q.total() > 1 && !q.last(),
        Nav::Submit => q.total() > 0 && q.last(),
    };
    if !shown {
        return None;
    }
    let disabled = p.disabled_when.as_ref().is_some_and(|f| f.call(q.active_status()));
    let (name, column) = match p.nav {
        Nav::Previous => ("questionnaire-previous", 1),
        Nav::Skip => ("questionnaire-skip", 2),
        Nav::Next => ("questionnaire-next", 3),
        Nav::Submit => ("questionnaire-submit", 3),
    };
    let variant = match p.nav {
        Nav::Previous | Nav::Skip => ButtonVariant::Outline,
        Nav::Next | Nav::Submit => p.variant,
    };
    let nav = p.nav;
    let b = button(variant, ButtonSize::Default)
        .named(name)
        .grid_column_start(column)
        .grid_row_start(1)
        .child(p.label.clone())
        .disabled(disabled)
        .on_click(move |_| match nav {
            Nav::Previous => q.previous(),
            Nav::Skip => q.skip(),
            Nav::Next => q.next(),
            Nav::Submit => q.submit(),
        });
    Some(if nav == Nav::Previous {
        let mut b = b;
        b.style_mut().justify_self = Some(Align::Start);
        b.into_div()
    } else {
        b.justify_self(Align::End).into_div()
    })
}

/// Back one question; hidden on the first.
#[must_use]
#[track_caller]
pub fn questionnaire_previous(label: &str) -> QuestionnaireNav {
    QuestionnaireNav::new(Nav::Previous, label)
}

/// Skip this question; shown only on optional questions.
#[must_use]
#[track_caller]
pub fn questionnaire_skip(label: &str) -> QuestionnaireNav {
    QuestionnaireNav::new(Nav::Skip, label)
}

/// On to the next question; hidden on the last.
#[must_use]
#[track_caller]
pub fn questionnaire_next(label: &str) -> QuestionnaireNav {
    QuestionnaireNav::new(Nav::Next, label)
}

/// Submit every answer; shown only on the last question.
#[must_use]
#[track_caller]
pub fn questionnaire_submit(label: &str) -> QuestionnaireNav {
    QuestionnaireNav::new(Nav::Submit, label)
}

/// `button` as the form's reset (`type="reset"`): back to the default
/// answers and the opening question, then the questionnaire's `on_reset`.
#[must_use]
#[track_caller]
pub fn questionnaire_reset(button: impl Into<Div>) -> Part<()> {
    Part::new((), button.into(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let q = use_questionnaire("questionnaire_reset", part_at(props));
            move || {
                let q = q.clone();
                props
                    .with(|p| p.slot.0.clone())
                    .on_click(move |_| {
                        if let Some(q) = &q {
                            q.reset();
                        }
                    })
                    .into_element()
            }
        })
    })
}
