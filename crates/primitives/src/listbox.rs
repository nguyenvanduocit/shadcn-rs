//! A list of options to choose from (reka's `Listbox`), and what every list
//! of group 5 shares: the value chosen ([`Choice`]), the value as a part
//! holds it ([`Bound`]), options by index ([`Options`]), and the highlight
//! ([`ListCore`]). Select, Combobox, Autocomplete, Tree and shadcn's command
//! are built on the core (research §9).
//!
//! # The listbox itself
//!
//! [`Listbox`] is reka's `ListboxRoot` on the core, with its parts:
//! [`Listbox::root`] (the pointer leaving it lets the highlight go),
//! [`Listbox::content`] (the listbox, its keys, the one Tab stop while
//! nothing is highlighted), [`Listbox::item`], [`Listbox::group`],
//! [`Listbox::group_label`], [`Listbox::filter`] (a field that keeps focus,
//! the options its active descendant) and [`Listbox::set_virtual`],
//! [`Listbox::virtual_content`] / [`Listbox::virtual_option`] for a list of
//! millions. Mounting highlights the chosen option (else the first usable
//! one) and moves no focus; Tab into the content highlights the option focus
//! last left (else the chosen, else the first), a pointer press on it does
//! not; focus leaving lets the highlight go and remembers it; a caller's
//! value highlights its option and focuses it, unless a filter or
//! `keep_focus` holds focus where it is. A virtual list keeps every one of
//! these over its data, not its built rows, and brings the highlighted
//! option into its own view.
//!
//! # Two focus models, one highlight
//!
//! - **On the items** ([`ListFocus::Items`]; Select, a Listbox without a
//!   filter, Tree): the highlighted option holds real focus, so the
//!   highlight *is* the focus (reka's `changeHighlight` focuses it).
//! - **In a field** ([`ListFocus::Field`]; Combobox, Autocomplete, a Listbox
//!   with a filter, shadcn's command): focus stays in the text field, and
//!   the field names the highlighted option as its active descendant
//!   (`aria-activedescendant`, [`ListCore::active`]).
//!
//! The look reads one state either way, [`ListCore::is_highlighted`]
//! (research §9, "Ghi chú chung": shadcn's select reads focus and its
//! combobox and command read a highlight; here both are the highlight).
//!
//! # The highlight belongs to the option
//!
//! It is kept as the option's value and its index. When the options change
//! (a filter, a sort, rows inserted above) the index is found again from
//! the value, so the highlight follows its option and is let go when the
//! option is gone (reka's virtualizer, `Listbox/ListboxVirtualizer.vue:
//! 100-148`). Reading it is O(1) while the options stand still.
//!
//! # The keyboard table (no filter, focus on an item)
//!
//! | Key | What it does |
//! | --- | --- |
//! | ArrowDown / ArrowUp (vertical); ArrowRight / ArrowLeft (horizontal, RTL swaps) | next / previous usable option, from none: first / last; stops at the ends unless it wraps |
//! | Home / End, PageUp / PageDown | first / last usable option |
//! | Enter | chooses the highlighted option; Control, Alt or Command with it is left to the form |
//! | Space | chooses it, unless a typeahead search runs (then it is a character of it) |
//! | Control, Alt or Command + A | several, when the list has select-all ([`ListConfig::select_all`]): chooses every usable option, highlights the last |
//! | Shift + a navigation key | several, replace: chooses the range from the anchor to the option reached |
//! | a printable character | typeahead (1000 ms) |
//!
//! With a filter, focus is in the field: the arrows, Home, End and Enter
//! work as above and every other key is the field's (no Space, no
//! typeahead, no select-all: research §9, Listbox, "Filter").
//!
//! # Where this differs from reka, and why
//!
//! - End with no highlight goes to the **last** option; reka's goes to the
//!   first (`ListboxRoot.vue:359-362,384-386`), a bug (research §4 item 13).
//! - Select-all chooses the usable options whether the list is virtual or
//!   not; reka's virtual list also chooses disabled ones
//!   (`ListboxVirtualizer.vue:303-308`). One notion of "cannot be used".
//! - A Shift range leaves out what cannot be chosen, as everywhere
//!   ([`crate::selection::range`]).
//! - Typeahead finds options by index over all of them, so repeated labels
//!   stay apart; reka's virtual list matches strings
//!   (`ListboxVirtualizer.vue:250-264`).

use std::borrow::Cow;
use std::rc::Rc;

use gui::element::Node;
use gui::events::{KeyEvent, NamedKey};
use gui::focus::FocusId;
use gui::list::ListState;
use gui::prelude::*;
use gui::{Direction, Div, FocusHandle, NodeRef, Signal, TextField, UseModel};

use crate::availability::Availability;
use crate::collection::{Collection, Item, Items};
use crate::direction::ReadDirection;
use crate::keymap::{Arrows, Intent, Keymap, Wrap};
use crate::selection::{SelectionBehavior, choose_many, choose_one};
use crate::typeahead::Typeahead;

/// What a list holds: one value (`Option<T>`) or several (`Vec<T>`, in the
/// order chosen). Two types, never one value that is sometimes `T` and
/// sometimes a list, and no `''` meaning "nothing" (research §3.2): reka
/// forbids an item valued `''` for that reason (`SelectItem.vue:206-210`);
/// here no value is `None` and an item may be valued anything.
pub trait Choice<T>: Clone + Default + PartialEq + 'static {
    /// Whether several are held (reka's `multiple`).
    const MULTIPLE: bool;
    /// Whether `value` is chosen.
    fn holds(&self, value: &T) -> bool;
    /// The chosen values, in the order chosen.
    fn values(&self) -> Vec<T>;
    /// The first chosen value.
    fn first(&self) -> Option<&T>;
    /// Nothing is chosen (reka's "empty": `undefined`, `null`, `''`, `[]`).
    fn is_empty(&self) -> bool;
    /// Choose `value` as `behavior` says ([`crate::selection`]).
    fn choose(&mut self, value: &T, behavior: SelectionBehavior);
    /// Hold exactly `values` (several: a range, select-all); one value
    /// holds the last of them.
    fn hold(&mut self, values: Vec<T>);
    /// Let `value` go when it is chosen.
    fn release(&mut self, value: &T);
}

impl<T: Clone + PartialEq + 'static> Choice<T> for Option<T> {
    const MULTIPLE: bool = false;

    fn holds(&self, value: &T) -> bool {
        self.as_ref() == Some(value)
    }

    fn values(&self) -> Vec<T> {
        self.iter().cloned().collect()
    }

    fn first(&self) -> Option<&T> {
        self.as_ref()
    }

    fn is_empty(&self) -> bool {
        self.is_none()
    }

    fn choose(&mut self, value: &T, behavior: SelectionBehavior) {
        *self = choose_one(self.as_ref(), value, behavior);
    }

    fn hold(&mut self, values: Vec<T>) {
        *self = values.into_iter().last();
    }

    fn release(&mut self, value: &T) {
        if self.holds(value) {
            *self = None;
        }
    }
}

impl<T: Clone + PartialEq + 'static> Choice<T> for Vec<T> {
    const MULTIPLE: bool = true;

    fn holds(&self, value: &T) -> bool {
        self.contains(value)
    }

    fn values(&self) -> Vec<T> {
        self.clone()
    }

    fn first(&self) -> Option<&T> {
        <[T]>::first(self)
    }

    fn is_empty(&self) -> bool {
        <[T]>::is_empty(self)
    }

    fn choose(&mut self, value: &T, behavior: SelectionBehavior) {
        *self = choose_many(self, value, behavior);
    }

    fn hold(&mut self, values: Vec<T>) {
        *self = values;
    }

    fn release(&mut self, value: &T) {
        self.retain(|v| v != value);
    }
}

/// A two-way value a primitive reads and writes: a part's `UseModel`
/// (convention 2: the primitive keeps no copy of the caller's value), or a
/// signal of its own.
///
/// A [`Bound`] shares it behind `Rc`, so each implementor states how a
/// fork (a check, ADR-0028) leaves its state ([`Held::fork_policy`]): held
/// in signals, which the reactive arena forks, or written only by what a
/// fork never runs (a handler, the caller's own code). [`Bound::set`]
/// checks it: a write in a fork to a value of the second kind is a bug,
/// loud in a debug build.
pub trait Held<T>: 'static {
    /// How a fork leaves its state (ADR-0028, rule 4): required, so no
    /// implementor shares state behind `Rc` unsaid.
    fn fork_policy(&self) -> HeldFork;
    /// The value, subscribing the reader.
    fn get(&self) -> T;
    fn get_untracked(&self) -> T;
    /// Write it; an equal value is no write (ADR-0025 §1).
    fn set(&self, value: T);
}

/// How a fork leaves a [`Held`] value's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldFork {
    /// All of it is in signals, which the reactive arena forks: a fork may
    /// write it, and its rollback puts it back.
    Signals,
    /// Written only by what a fork never runs (a handler, the caller's own
    /// code between frames), for the reason given: a fork must not write
    /// it, as no rollback would put it back.
    Outside(&'static str),
}

impl<P: 'static, T: Clone + PartialEq + 'static> Held<T> for UseModel<P, T> {
    /// The part's props and its own value: signals.
    fn fork_policy(&self) -> HeldFork {
        HeldFork::Signals
    }

    fn get(&self) -> T {
        UseModel::get(self)
    }

    fn get_untracked(&self) -> T {
        UseModel::get_untracked(self)
    }

    fn set(&self, value: T) {
        UseModel::set(self, value);
    }
}

impl<T: Clone + PartialEq + 'static> Held<T> for Signal<T> {
    fn fork_policy(&self) -> HeldFork {
        HeldFork::Signals
    }

    fn get(&self) -> T {
        Signal::get(self)
    }

    fn get_untracked(&self) -> T {
        Signal::get_untracked(self)
    }

    fn set(&self, value: T) {
        Signal::set(self, value);
    }
}

/// A [`Held`] value, its owner's type forgotten: `Copy`, so a primitive's
/// state holds it whatever props its part has.
pub struct Bound<T: 'static>(Signal<Rc<dyn Held<T>>>);

impl<T: 'static> Clone for Bound<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Bound<T> {}

impl<T: 'static> PartialEq for Bound<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: 'static> std::fmt::Debug for Bound<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Bound").finish()
    }
}

impl<T: Clone + PartialEq + 'static> Bound<T> {
    /// `held` (a `UseModel`, a signal) as a `Copy` handle; made in setup.
    #[must_use]
    #[track_caller]
    pub fn new(held: impl Held<T>) -> Self {
        let held: Rc<dyn Held<T>> = Rc::new(held);
        Self(Signal::new(held))
    }

    fn held(self) -> Option<Rc<dyn Held<T>>> {
        self.0.is_alive().then(|| self.0.with_untracked(Rc::clone))
    }

    /// The value, subscribing the reader.
    #[must_use]
    pub fn get(self) -> T
    where
        T: Default,
    {
        self.held().map(|h| h.get()).unwrap_or_default()
    }

    #[must_use]
    pub fn get_untracked(self) -> T
    where
        T: Default,
    {
        self.held().map(|h| h.get_untracked()).unwrap_or_default()
    }

    /// Write it (an equal value is no write). In a fork, only a value held
    /// in signals is written: one written from outside (`HeldFork::Outside`)
    /// is a bug, loud in a debug build, and left alone in release.
    pub fn set(self, value: T) {
        if let Some(h) = self.held() {
            if gui::engine::forking()
                && let HeldFork::Outside(why) = h.fork_policy()
            {
                debug_assert!(
                    false,
                    "a fork wrote a `Held` value only the outside world writes ({why})"
                );
                return;
            }
            h.set(value);
        }
    }

    /// Change it from what it is now (read untracked).
    pub fn update(self, f: impl FnOnce(&mut T))
    where
        T: Default,
    {
        let mut v = self.get_untracked();
        f(&mut v);
        self.set(v);
    }
}

/// Options by index, in display order: [`Items`] with the value each one
/// chooses and, for options that take real focus, the focus they take.
pub trait Options<T>: Items {
    /// The value option `index` chooses.
    fn value(&self, index: usize) -> Option<T>;
    /// The focus option `index` takes, when focus goes on the items.
    fn focus(&self, _index: usize) -> Option<FocusHandle> {
        None
    }
    /// The index of the option valued `value` (O(n); the highlight asks
    /// only when the options changed under it).
    fn position(&self, value: &T) -> Option<usize>
    where
        T: PartialEq,
    {
        (0..self.len()).find(|&i| self.value(i).as_ref() == Some(value))
    }
}

impl<T: Clone> Options<T> for [Item<T>] {
    fn value(&self, index: usize) -> Option<T> {
        self.get(index).map(|i| i.value.clone())
    }

    fn focus(&self, index: usize) -> Option<FocusHandle> {
        self.get(index).map(|i| i.focus)
    }
}

/// A list of `len` options whose availability, label and value the
/// closures give by index: the data behind a virtual list.
pub fn options_fn<'s, T, A, L, V>(len: usize, availability: A, label: L, value: V) -> OptionsFn<'s, A, L, V>
where
    A: Fn(usize) -> Availability,
    L: Fn(usize) -> Cow<'s, str>,
    V: Fn(usize) -> Option<T>,
{
    OptionsFn {
        len,
        availability,
        label,
        value,
        data: std::marker::PhantomData,
    }
}

/// See [`options_fn`].
pub struct OptionsFn<'s, A, L, V> {
    len: usize,
    availability: A,
    label: L,
    value: V,
    data: std::marker::PhantomData<&'s str>,
}

impl<'s, A, L, V> Items for OptionsFn<'s, A, L, V>
where
    A: Fn(usize) -> Availability,
    L: Fn(usize) -> Cow<'s, str>,
{
    fn len(&self) -> usize {
        self.len
    }

    fn availability(&self, index: usize) -> Availability {
        if index < self.len {
            (self.availability)(index)
        } else {
            Availability::Disabled
        }
    }

    fn label(&self, index: usize) -> Cow<'_, str> {
        if index < self.len {
            (self.label)(index)
        } else {
            Cow::Borrowed("")
        }
    }
}

impl<'s, T, A, L, V> Options<T> for OptionsFn<'s, A, L, V>
where
    A: Fn(usize) -> Availability,
    L: Fn(usize) -> Cow<'s, str>,
    V: Fn(usize) -> Option<T>,
{
    fn value(&self, index: usize) -> Option<T> {
        if index < self.len { (self.value)(index) } else { None }
    }
}

/// Where focus is while the list is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ListFocus {
    /// On the highlighted option (roving: Select, Listbox, Tree).
    #[default]
    Items,
    /// In a text field naming the option as its active descendant
    /// (Combobox, Autocomplete, a Listbox's filter, command).
    Field,
}

/// A caller's answer to a choice about to be made (reka's `select` event,
/// `preventDefault` to refuse it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Pick {
    /// The answer with no caller asked (no veto set).
    #[default]
    Allow,
    Refuse,
}

/// What a key did to a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListKey {
    /// Not the list's key: left to the field, the form, the page.
    Ignored,
    /// The list's key, with nothing to do (an end reached): consumed.
    Stayed,
    /// The highlight moved to this option.
    Highlighted(usize),
    /// This option was chosen (Enter, Space).
    Chose(usize),
    /// A choice was refused, or the option cannot be chosen: consumed.
    Refused,
    /// Several were chosen at once (select-all, a Shift range).
    ChoseMany,
}

impl ListKey {
    /// Whether the key was the list's.
    #[must_use]
    pub fn consumed(&self) -> bool {
        !matches!(self, Self::Ignored)
    }
}

/// How a list navigates and chooses; set at its root.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListConfig {
    pub keymap: Keymap,
    pub behavior: SelectionBehavior,
    pub focus: ListFocus,
    pub direction: ReadDirection,
    /// Whether the pointer moving over an option highlights it (reka's
    /// `highlightOnHover`: off in Listbox, on in Combobox and command).
    pub hover: bool,
    /// Whether Control, Alt or Command + A chooses every usable option of a
    /// list holding several (reka's Listbox does; its Select and Tree do
    /// not, so neither do ours).
    pub select_all: bool,
}

impl ListConfig {
    /// A vertical list that stops at its ends, toggling, focus on its
    /// items, left to right, no hover highlight, select-all: reka's Listbox
    /// defaults.
    pub const LISTBOX: Self = Self {
        keymap: Keymap::LIST,
        behavior: SelectionBehavior::Toggle,
        focus: ListFocus::Items,
        direction: ReadDirection::fixed(Direction::Ltr),
        hover: false,
        select_all: true,
    };

    /// The list runs along `arrows` (reka's `orientation`).
    #[must_use]
    pub const fn arrows(self, arrows: Arrows) -> Self {
        Self {
            keymap: self.keymap.arrows(arrows),
            ..self
        }
    }

    /// The list wraps (reka's `loop`).
    #[must_use]
    pub const fn wrap(self, wrap: Wrap) -> Self {
        Self {
            keymap: self.keymap.wrap(wrap),
            ..self
        }
    }

    #[must_use]
    pub const fn behavior(self, behavior: SelectionBehavior) -> Self {
        Self { behavior, ..self }
    }

    #[must_use]
    pub const fn focus(self, focus: ListFocus) -> Self {
        Self { focus, ..self }
    }

    #[must_use]
    pub const fn direction(self, direction: ReadDirection) -> Self {
        Self { direction, ..self }
    }

    #[must_use]
    pub const fn hover(self, hover: bool) -> Self {
        Self { hover, ..self }
    }

    #[must_use]
    pub const fn select_all(self, select_all: bool) -> Self {
        Self { select_all, ..self }
    }
}

/// A refusal hook: the caller's answer to each choice.
type Veto<T> = gui::Callback<T, Pick>;

#[derive(Clone)]
struct State<T: 'static> {
    config: ListConfig,
    veto: Option<Veto<T>>,
}

/// A list's highlight, typeahead and Shift anchor: made once in its root's
/// setup, `Copy`.
pub struct ListCore<T: 'static> {
    /// The highlighted option: its index as last known, and its value.
    highlight: Signal<Option<(usize, T)>>,
    /// The option highlighted when focus last left the list, highlighted
    /// again when it comes back (reka's `previousElement`).
    previous: Signal<Option<T>>,
    /// Where a Shift range starts (reka's `firstValue`).
    anchor: Signal<Option<T>>,
    typeahead: Typeahead,
    state: Signal<State<T>>,
    /// The highlighted option's node, for a field's active descendant.
    active: NodeRef,
}

impl<T: 'static> Clone for ListCore<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for ListCore<T> {}

impl<T: 'static> PartialEq for ListCore<T> {
    fn eq(&self, other: &Self) -> bool {
        self.highlight == other.highlight
    }
}

impl<T: 'static> std::fmt::Debug for ListCore<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListCore").finish_non_exhaustive()
    }
}

impl<T: Clone + PartialEq + 'static> ListCore<T> {
    /// A list navigating and choosing as `config` says; made in setup.
    #[must_use]
    #[track_caller]
    pub fn new(config: ListConfig) -> Self {
        Self {
            highlight: Signal::new(None),
            previous: Signal::new(None),
            anchor: Signal::new(None),
            typeahead: Typeahead::new(),
            state: Signal::new(State { config, veto: None }),
            active: NodeRef::new(),
        }
    }

    /// How it navigates and chooses now.
    #[must_use]
    pub fn config(self) -> ListConfig {
        self.state.with_untracked(|s| s.config)
    }

    /// Navigate and choose as `config` says from now on (a caller's
    /// orientation, `loop`, direction).
    pub fn set_config(self, config: ListConfig) {
        self.state.update(|s| s.config = config);
    }

    /// Ask `f` before each choice; [`Pick::Refuse`] leaves the value as it
    /// is (reka's `@select` with `preventDefault`).
    pub fn on_pick(self, f: impl Into<gui::Callback<T, Pick>>) {
        let f: Veto<T> = f.into();
        self.state.update(|s| s.veto = Some(f));
    }

    /// The typeahead search (its look reads [`Typeahead::search`]).
    #[must_use]
    pub fn typeahead(self) -> Typeahead {
        self.typeahead
    }

    /// The node the highlighted option carries: what a field names as its
    /// active descendant.
    #[must_use]
    pub fn active(self) -> NodeRef {
        self.active
    }

    /// The highlighted option's value (tracked).
    #[must_use]
    pub fn highlighted(self) -> Option<T> {
        self.highlight.with(|h| h.as_ref().map(|(_, v)| v.clone()))
    }

    /// Whether `value` is highlighted (tracked): what the look reads.
    #[must_use]
    pub fn is_highlighted(self, value: &T) -> bool {
        self.highlight.with(|h| h.as_ref().is_some_and(|(_, v)| v == value))
    }

    /// Whether any option is highlighted (tracked).
    #[must_use]
    pub fn has_highlight(self) -> bool {
        self.highlight.with(Option::is_some)
    }

    /// The highlighted option's index in `options` (untracked): its index as
    /// last known when the option is still there, else found again by its
    /// value; `None` when it is gone.
    #[must_use]
    pub fn index_in<O: Options<T> + ?Sized>(self, options: &O) -> Option<usize> {
        let (at, value) = self.highlight.get_untracked()?;
        if options.value(at).as_ref() == Some(&value) {
            return Some(at);
        }
        options.position(&value)
    }

    /// The highlighted option's index, tracked, so a look reading it renders
    /// again when the highlight moves.
    #[must_use]
    pub fn index_tracked<O: Options<T> + ?Sized>(self, options: &O) -> Option<usize> {
        self.highlight.with(|_| ());
        self.index_in(options)
    }

    /// Highlight option `index` of `options`; with focus on the items it
    /// takes focus (shown as from the keyboard when `by_keyboard`).
    pub fn highlight<O: Options<T> + ?Sized>(self, options: &O, index: usize, by_keyboard: bool) {
        let Some(value) = options.value(index) else {
            return;
        };
        if self
            .highlight
            .with_untracked(|h| h.as_ref() != Some(&(index, value.clone())))
        {
            self.highlight.set(Some((index, value)));
        }
        if self.config().focus == ListFocus::Items
            && let Some(f) = options.focus(index)
            && !f.is_focused()
        {
            gui::focus::set_focus(Some(f.id()), by_keyboard);
        }
    }

    /// Highlight the option valued `value`, if `options` has it.
    pub fn highlight_value<O: Options<T> + ?Sized>(self, options: &O, value: &T, by_keyboard: bool) -> bool {
        match options.position(value) {
            Some(i) => {
                self.highlight(options, i, by_keyboard);
                true
            }
            None => false,
        }
    }

    /// Highlight the first usable option (reka's `highlightFirstItem`).
    pub fn highlight_first<O: Options<T> + ?Sized>(self, options: &O, by_keyboard: bool) -> bool {
        match crate::keymap::step(options, None, Intent::First, Wrap::Stop) {
            Some(i) => {
                self.highlight(options, i, by_keyboard);
                true
            }
            None => {
                self.clear();
                false
            }
        }
    }

    /// Highlight the first chosen option of `value`, else the first usable
    /// one (reka's `highlightSelected`).
    pub fn highlight_chosen<M: Choice<T>, O: Options<T> + ?Sized>(self, options: &O, value: &M, by_keyboard: bool) {
        let chosen =
            (0..options.len()).find(|&i| options.focusable(i) && options.value(i).is_some_and(|v| value.holds(&v)));
        match chosen {
            Some(i) => self.highlight(options, i, by_keyboard),
            None => {
                self.highlight_first(options, by_keyboard);
            }
        }
    }

    /// Let the highlight go (reka's `onLeave`), remembering it for when
    /// focus comes back.
    pub fn leave(self) {
        if let Some((_, v)) = self.highlight.get_untracked() {
            self.previous.set(Some(v));
        }
        self.clear();
    }

    /// Let the highlight go, remembering nothing.
    pub fn clear(self) {
        if self.highlight.with_untracked(Option::is_some) {
            self.highlight.set(None);
        }
    }

    /// Focus came into the list by Tab: highlight the option highlighted
    /// when it left, else the first usable one (reka's `onEnter`).
    pub fn enter<O: Options<T> + ?Sized>(self, options: &O) {
        let previous = self.previous.get_untracked();
        if let Some(v) = previous
            && self.highlight_value(options, &v, true)
        {
            return;
        }
        self.highlight_first(options, true);
    }

    /// The value option `index` would choose, if it can be chosen and the
    /// caller does not refuse it.
    pub fn pickable<O: Options<T> + ?Sized>(self, options: &O, index: usize) -> Option<T> {
        if !options.availability(index).actionable() {
            return None;
        }
        let v = options.value(index)?;
        let veto = self.state.with_untracked(|s| s.veto.clone());
        if veto.is_some_and(|f| f.call(v.clone()) == Some(Pick::Refuse)) {
            return None;
        }
        Some(v)
    }

    /// Choose option `index` into `value` as the list chooses (toggle or
    /// replace), unless it cannot be chosen or the caller refuses; with no
    /// `value`, only tell ([`ListKey::Chose`]): choosing is the caller's.
    pub fn choose<M: Choice<T>, O: Options<T> + ?Sized>(
        self,
        options: &O,
        index: usize,
        value: Option<Bound<M>>,
    ) -> ListKey {
        let Some(v) = self.pickable(options, index) else {
            return ListKey::Refused;
        };
        let behavior = self.config().behavior;
        if let Some(value) = value {
            value.update(|m| m.choose(&v, behavior));
        }
        if behavior == SelectionBehavior::Replace {
            self.anchor.set(Some(v));
        }
        ListKey::Chose(index)
    }

    /// What `ev` does to the list: navigation, choosing, typeahead,
    /// select-all and Shift ranges, by the table in the module's docs.
    /// `value` is what choosing writes (none: choosing is the caller's,
    /// [`ListKey::Chose`] tells it, and select-all and ranges do nothing).
    pub fn key<M: Choice<T>, O: Options<T> + ?Sized>(
        self,
        ev: &KeyEvent,
        options: &O,
        value: Option<Bound<M>>,
    ) -> ListKey {
        let config = self.config();
        let m = ev.modifiers;
        let command = m.control || m.alt || m.command;
        let ranges = M::MULTIPLE && config.behavior == SelectionBehavior::Replace;
        let keymap = config.keymap.with_shift(ranges);
        let current = self.index_in(options);
        if let Some(intent) = keymap.intent(ev, config.direction.get()) {
            let Some(to) = keymap.step(options, current, intent) else {
                return ListKey::Stayed;
            };
            self.highlight(options, to, true);
            if ranges
                && m.shift
                && let Some(value) = value
            {
                self.extend(options, to, value);
                return ListKey::ChoseMany;
            }
            return ListKey::Highlighted(to);
        }
        if ev.is(NamedKey::Enter) {
            // Control, Alt or Command with Enter is the form's (it may submit).
            if command {
                return ListKey::Ignored;
            }
            return match current {
                Some(i) => self.choose(options, i, value),
                None => ListKey::Ignored,
            };
        }
        if config.focus == ListFocus::Field {
            return ListKey::Ignored;
        }
        if command
            && M::MULTIPLE
            && config.select_all
            && is_letter(ev, 'a')
            && let Some(value) = value
        {
            self.choose_all(options, value);
            return ListKey::ChoseMany;
        }
        if let Some(typed) = self.typeahead.typed(ev) {
            let typed = typed.to_owned();
            if let Some(i) = self.typeahead.find(&typed, options, current) {
                self.highlight(options, i, true);
                return ListKey::Highlighted(i);
            }
            return ListKey::Stayed;
        }
        if crate::keymap::activates(ev) {
            return match current {
                Some(i) => self.choose(options, i, value),
                None => ListKey::Stayed,
            };
        }
        ListKey::Ignored
    }

    /// Choose every usable option, highlight the last (select-all).
    pub fn choose_all<M: Choice<T>, O: Options<T> + ?Sized>(self, options: &O, value: Bound<M>) {
        let all: Vec<T> = (0..options.len())
            .filter(|&i| options.availability(i).actionable())
            .filter_map(|i| options.value(i))
            .collect();
        value.update(|m| m.hold(all));
        if let Some(last) = crate::keymap::step(options, None, Intent::Last, Wrap::Stop) {
            self.highlight(options, last, true);
        }
    }

    /// Choose the range from the anchor to option `to` (Shift).
    fn extend<M: Choice<T>, O: Options<T> + ?Sized>(self, options: &O, to: usize, value: Bound<M>) {
        let anchor = self.anchor.get_untracked().and_then(|a| options.position(&a));
        let Some(anchor) = anchor else {
            if let Some(v) = options.value(to) {
                self.anchor.set(Some(v));
            }
            return;
        };
        let values: Vec<T> = crate::selection::range(options, anchor, to)
            .into_iter()
            .flatten()
            .filter_map(|i| options.value(i))
            .collect();
        value.update(|m| m.hold(values));
    }

    /// `d` as option `index`, valued `value`, of a list whose options are
    /// laid out as data (a virtual list): an option to assistive technology,
    /// chosen or not, its availability applied, the active descendant while
    /// highlighted; the pointer highlights it (when the list highlights on
    /// hover, keeping its index, so reading the highlight stays O(1)) and a
    /// click runs `choose`. `chosen` is what the row's render read.
    #[must_use]
    pub fn option(
        self,
        d: Div,
        index: usize,
        value: &T,
        availability: Availability,
        chosen: bool,
        choose: impl Fn() + 'static,
    ) -> Div {
        let highlighted = self.is_highlighted(value);
        let d = availability
            .apply(d)
            .role(gui::Role::ListBoxOption)
            .aria_selected(chosen);
        let d = if highlighted { d.node_ref(self.active) } else { d };
        if !availability.actionable() {
            return d;
        }
        let me = self;
        let hovered = value.clone();
        let hover = self.config().hover;
        d.when(hover, |d| {
            d.on_hover(move |inside| {
                if inside {
                    me.highlight_at(index, hovered.clone());
                }
            })
        })
        .on_click(move |_| choose())
    }

    /// Whether any option is highlighted, untracked (what Tab reads).
    #[must_use]
    pub fn has_highlight_untracked(self) -> bool {
        self.highlight.with_untracked(Option::is_some)
    }

    /// Whether its owner's scope still holds it.
    #[must_use]
    pub fn is_alive(self) -> bool {
        self.highlight.is_alive()
    }

    /// Whether `value` is highlighted, untracked.
    #[must_use]
    pub fn is_highlighted_untracked(self, value: &T) -> bool {
        self.highlight
            .with_untracked(|h| h.as_ref().is_some_and(|(_, v)| v == value))
    }

    /// Highlight the option valued `value` at `index` (the pointer on a
    /// mounted option, which knows its place).
    pub fn highlight_at(self, index: usize, value: T) {
        if self
            .highlight
            .with_untracked(|h| h.as_ref() != Some(&(index, value.clone())))
        {
            self.highlight.set(Some((index, value)));
        }
    }
}

/// What a list's options are right now: the part that lays them out
/// gives its root a way to read them (in its setup), so the root's
/// watchers and keys walk what the list shows, mounted or not.
pub type Source<T> = Rc<dyn Fn() -> Rc<dyn Options<T>>>;

/// A virtual list's rows and the data laid on them.
#[derive(Clone)]
struct Virtual<T: 'static> {
    state: ListState,
    source: Source<T>,
}

/// What a [`Listbox`] keeps beside its value: its mounted options, or the
/// data of a virtual list.
struct ListParts<T: 'static> {
    items: Collection<T>,
    /// A virtual list's, held while the part that gave it is mounted.
    virtual_list: gui::Holder<Virtual<T>>,
}

/// Mounted options, sized, so they are read as `dyn Options` as a virtual
/// list's data is.
struct Mounted<'a, T>(&'a [Item<T>]);

impl<T> Items for Mounted<'_, T> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn availability(&self, index: usize) -> Availability {
        self.0.availability(index)
    }

    fn label(&self, index: usize) -> Cow<'_, str> {
        self.0.label(index)
    }

    fn focusable(&self, index: usize) -> bool {
        self.0.focusable(index)
    }
}

impl<T: Clone> Options<T> for Mounted<'_, T> {
    fn value(&self, index: usize) -> Option<T> {
        self.0.value(index)
    }

    fn focus(&self, index: usize) -> Option<FocusHandle> {
        self.0.focus(index)
    }
}

/// A listbox (reka's `ListboxRoot` and its parts): a list of options the
/// user chooses from, its value its own or the caller's ([`Bound`]). Made
/// once in its root's setup, `Copy`. Its parts add the behaviour to the
/// look's nodes: [`Listbox::root`], [`Listbox::content`],
/// [`Listbox::item`], [`Listbox::group`], [`Listbox::group_label`],
/// [`Listbox::filter`] and, for a list of data laid on a virtual list
/// ([`Listbox::set_virtual`]), [`Listbox::virtual_content`] and
/// [`Listbox::virtual_option`].
///
/// # Focus
///
/// - Without a filter the list is one Tab stop: the highlighted option, or
///   the list itself while none is. Focus coming in by Tab highlights the
///   option highlighted when it left, else the first usable one; a pointer
///   press bringing focus in does not (it highlights what it pressed).
/// - Focus leaving the root lets the highlight go and remembers it.
/// - The pointer leaving the root lets the highlight go.
/// - With a filter ([`Listbox::set_filter`]) focus stays in its field and
///   the options never take it; the field names the highlighted one.
///
/// # A caller's value
///
/// A value the caller writes (not a choice of the user's) highlights the
/// chosen option, else the first: as the list mounts without moving focus
/// or scrolling, and after that bringing it into view and moving focus to
/// it, unless there is a filter or [`Listbox::set_keep_focus`] holds focus
/// elsewhere (reka's `keepFocusOnValueChange`).
pub struct Listbox<T: 'static, M: Choice<T>> {
    value: Bound<M>,
    core: ListCore<T>,
    parts: Signal<Rc<ListParts<T>>>,
    filter: Signal<Option<TextField>>,
    content: FocusHandle,
    node: NodeRef,
    /// Focus is coming in by a pointer press or by the list itself (a
    /// caller's value, on a virtual list): no entry by Tab.
    pressed: Signal<bool>,
    /// The user is writing the value (a choice), not the caller.
    user: Signal<bool>,
    keep_focus: Signal<bool>,
    availability: Signal<Availability>,
    /// The scope it was made in: what an option's leaving queues work for.
    owner: Option<gui::Owner>,
}

impl<T: 'static, M: Choice<T>> Clone for Listbox<T, M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static, M: Choice<T>> Copy for Listbox<T, M> {}

impl<T: 'static, M: Choice<T>> PartialEq for Listbox<T, M> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.core == other.core
    }
}

impl<T: 'static, M: Choice<T>> std::fmt::Debug for Listbox<T, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listbox").finish_non_exhaustive()
    }
}

impl<T: Clone + PartialEq + 'static, M: Choice<T>> Listbox<T, M> {
    /// A listbox holding `value`, navigating and choosing as `config` says
    /// (its focus model is set by [`Listbox::set_filter`]); made in its
    /// root's setup, whose scope its watchers live in.
    #[must_use]
    #[track_caller]
    pub fn new(value: Bound<M>, config: ListConfig) -> Self {
        let me = Self {
            value,
            core: ListCore::new(config.focus(ListFocus::Items)),
            parts: Signal::new(Rc::new(ListParts {
                items: Collection::new(),
                virtual_list: gui::Holder::new(),
            })),
            filter: Signal::new(None),
            content: FocusHandle::new(),
            node: NodeRef::new(),
            pressed: Signal::new(false),
            user: Signal::new(false),
            keep_focus: Signal::new(false),
            availability: Signal::new(Availability::Enabled),
            owner: gui::Owner::current(),
        };
        // Focus on the items: an option focused is highlighted; focus coming
        // to the list itself by Tab is an entry; focus leaving lets go.
        gui::watch(gui::focus::focused, move |now, old| {
            if !me.parts.is_alive() || me.has_filter() {
                return;
            }
            let items = me.items();
            let inside = |f: &Option<FocusId>| f.is_some_and(|f| f == me.content.id() || items.index_of(f).is_some());
            match now.and_then(|f| items.index_of(f)) {
                Some(i) => items.with(|list| me.core.highlight(list, i, gui::focus::focus_visible())),
                None if *now == Some(me.content.id()) => {
                    if me.pressed.get_untracked() {
                        me.pressed.set(false);
                    } else if !inside(old) {
                        me.with_options(|list| me.core.enter(list));
                        me.reveal();
                    }
                }
                None => {
                    if inside(old) {
                        me.core.leave();
                    }
                }
            }
        });
        // The mount highlight: the chosen option (else the first), no focus
        // moved; only a virtual list scrolls, inside itself, to bring it
        // into view (`ListboxVirtualizer.vue:184-195`).
        me.later(move || {
            if me.parts.is_alive() {
                me.highlight_chosen(false);
            }
        });
        // A caller's value highlights its option, focus with it.
        gui::watch(
            move || value.get(),
            move |_, _| {
                if !me.parts.is_alive() {
                    return;
                }
                if me.user.get_untracked() {
                    me.user.set(false);
                    return;
                }
                let inside = me.holds_focus();
                let focus = !me.has_filter() && !(me.keep_focus.get_untracked() && !inside);
                me.later(move || {
                    if me.parts.is_alive() {
                        me.highlight_chosen(focus);
                    }
                });
            },
        );
        me
    }

    fn items(self) -> Collection<T> {
        self.parts.with_untracked(|p| p.items.clone())
    }

    /// A virtual list's rows and data source, when it is one.
    fn virtual_list(self) -> Option<(ListState, Source<T>)> {
        self.parts.with_untracked(|p| {
            p.virtual_list
                .with_untracked(|v| v.map(|v| (v.state, Rc::clone(&v.source))))
        })
    }

    /// Run `f` over the options the list shows now: a virtual list's data,
    /// else the mounted options.
    fn with_options<R>(self, f: impl FnOnce(&dyn Options<T>) -> R) -> R {
        if let Some((_, source)) = self.virtual_list() {
            let options = gui::untracked(|| source());
            return f(&*options);
        }
        self.items().with(|list| f(&Mounted(list)))
    }

    /// A virtual list brings the highlighted option's row into its view.
    fn reveal(self) {
        let Some((state, source)) = self.virtual_list() else {
            return;
        };
        let options = gui::untracked(|| source());
        if let Some(i) = self.core.index_in(&*options) {
            state.scroll_into_view(u64::try_from(i).unwrap_or(u64::MAX));
        }
    }

    fn has_filter(self) -> bool {
        self.filter.with_untracked(Option::is_some)
    }

    /// Whether focus is on the list, one of its options or its filter.
    fn holds_focus(self) -> bool {
        let Some(f) = gui::untracked(gui::focus::focused) else {
            return false;
        };
        f == self.content.id()
            || self.items().index_of(f).is_some()
            || self.filter.with_untracked(|t| t.is_some_and(|t| t.focus.id() == f))
    }

    /// Highlight the chosen option, else the first usable one (reka's
    /// `highlightSelected`); `focus` moves focus to it (to a virtual list,
    /// whose options are its active descendants). A virtual list brings it
    /// into view.
    fn highlight_chosen(self, focus: bool) {
        let value = self.value.get_untracked();
        self.with_options(|list| {
            let chosen = (0..list.len())
                .find(|&i| list.focusable(i) && list.value(i).is_some_and(|v| value.holds(&v)))
                .or_else(|| crate::keymap::step(list, None, Intent::First, Wrap::Stop));
            let Some(i) = chosen else { return };
            if focus {
                self.core.highlight(list, i, false);
            } else if let Some(v) = list.value(i) {
                self.core.highlight_at(i, v);
            }
        });
        if self.virtual_list().is_some() {
            self.reveal();
            if focus && !self.content.is_focused() {
                self.pressed.set(true);
                gui::focus::set_focus(Some(self.content.id()), false);
            }
        }
    }

    /// The list core: highlight, typeahead, Shift anchor, veto.
    #[must_use]
    pub fn core(self) -> ListCore<T> {
        self.core
    }

    /// Navigate and choose as `config` says from now on (orientation,
    /// `loop`, direction, selection behaviour, hover).
    pub fn set_config(self, config: ListConfig) {
        let focus = self.core.config().focus;
        self.core.set_config(config.focus(focus));
    }

    /// Whether it can be used (its `disabled`): disabled, every option is.
    pub fn set_availability(self, availability: Availability) {
        self.availability.set(availability);
    }

    /// Keep focus where it is when the caller writes the value while focus
    /// is outside the list (reka's `keepFocusOnValueChange`).
    pub fn set_keep_focus(self, keep: bool) {
        self.keep_focus.set(keep);
    }

    /// Ask `f` before each choice ([`Pick::Refuse`] leaves the value).
    pub fn on_pick(self, f: impl Into<gui::Callback<T, Pick>>) {
        self.core.on_pick(f.into());
    }

    /// The list has a filter: focus stays in `field` (its setup calls it),
    /// whose typing highlights the first usable option once the list shows
    /// what it filtered.
    pub fn set_filter(self, field: TextField) {
        self.filter.set(Some(field));
        self.core.set_config(self.core.config().focus(ListFocus::Field));
        let me = self;
        gui::watch(
            move || field.revision(),
            move |_, _| {
                if field.edit.with_untracked(|e| e.preedit().is_some()) {
                    return;
                }
                me.later(move || {
                    if me.parts.is_alive() {
                        me.with_options(|list| me.core.highlight_first(list, false));
                    }
                });
            },
        );
    }

    /// The value (tracked).
    #[must_use]
    pub fn value(self) -> M {
        self.value.get()
    }

    /// Whether `value` is chosen (tracked): an option's indicator.
    #[must_use]
    pub fn holds(self, value: &T) -> bool {
        self.value.get().holds(value)
    }

    /// Whether the option valued `value` is highlighted (tracked).
    #[must_use]
    pub fn is_highlighted(self, value: &T) -> bool {
        self.core.is_highlighted(value)
    }

    /// Whether it can be used (tracked).
    #[must_use]
    pub fn availability(self) -> Availability {
        self.availability.get()
    }

    /// Choose option `index` of `options` as the user (a click, Enter, Space).
    fn choose<O: Options<T> + ?Sized>(self, options: &O, index: usize) -> ListKey {
        let before = self.value.get_untracked();
        self.user.set(true);
        let key = self.core.choose(options, index, Some(self.value));
        if self.value.get_untracked() == before {
            self.user.set(false);
        }
        key
    }

    /// The keys of the list (on an option, on the list, or in the filter).
    fn key<O: Options<T> + ?Sized>(self, ev: &KeyEvent, options: &O) -> bool {
        if !self.availability.get_untracked().actionable() {
            return false;
        }
        let before = self.value.get_untracked();
        self.user.set(true);
        let key = self.core.key(ev, options, Some(self.value));
        if self.value.get_untracked() == before {
            self.user.set(false);
        }
        key.consumed()
    }

    /// `d` as the root: the pointer leaving it lets the highlight go.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        let core = self.core;
        d.on_hover(move |inside| {
            if !inside {
                core.clear();
            }
        })
    }

    /// `d` as the list (`ListboxContent`): a listbox to assistive technology,
    /// along its orientation, multi-select when it holds several; without a
    /// filter it is the Tab stop while nothing is highlighted, and takes the
    /// list's keys. `has_highlight` is what the look's render read
    /// ([`ListCore::has_highlight`] through [`Listbox::core`]).
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        let arrows = self.core.config().keymap.arrows;
        let axis = if arrows == Arrows::Horizontal {
            gui::style::Axis::Horizontal
        } else {
            gui::style::Axis::Vertical
        };
        let d = self
            .availability
            .get()
            .apply(d)
            .role(gui::Role::ListBox)
            .aria_orientation(axis)
            .aria_multiselectable(M::MULTIPLE)
            .node_ref(self.node);
        if self.filter.get().is_some() {
            return d;
        }
        let core = self.core;
        let me = self;
        self.pressed_in(d.track_focus(self.content))
            .tab_stop(move || !core.has_highlight_untracked())
            .on_key_down(move |ev| {
                let items = me.items();
                items.with(|list| me.key(ev, list))
            })
    }

    /// A pointer press on `d` (the list) bringing focus in is no entry by
    /// Tab: it highlights what it pressed, not the option last left.
    fn pressed_in(self, d: Div) -> Div {
        let me = self;
        let content = self.content;
        d.on_mouse_down(move |_| {
            if !content.is_focused() {
                me.pressed.set(true);
                // The press is over by the next layout, whether or not
                // focus came (reka's 10 ms window).
                me.later(move || {
                    if me.pressed.is_alive() {
                        me.pressed.set(false);
                    }
                });
            }
        })
    }

    /// `d` as the option valued `value`, labelled `label` (typeahead
    /// matches it), focused through `focus` (made in the option's setup,
    /// which also calls [`Listbox::option_unmounted`] as it goes);
    /// `availability` is its own (the list's being disabled disables it).
    /// An option to assistive technology, selected while chosen, the Tab
    /// stop while highlighted; a click chooses it, and the pointer
    /// highlights it when the list highlights on hover (no focus moved).
    #[must_use]
    pub fn item(self, d: Div, value: T, label: Rc<str>, focus: FocusHandle, availability: Availability) -> Div {
        let availability = match self.availability.get() {
            Availability::Enabled => availability,
            root => root,
        };
        let chosen = self.value.get().holds(&value);
        let highlighted = self.core.is_highlighted(&value);
        let field = self.filter.get().is_some();
        let items = self.items();
        let d = items
            .item(
                d,
                Item::new(value.clone(), focus)
                    .label(label)
                    .availability(availability)
                    .active(chosen),
            )
            .role(gui::Role::ListBoxOption)
            .aria_selected(chosen);
        let d = if field {
            let d = d.tab_stop(false);
            if highlighted { d.node_ref(self.core.active()) } else { d }
        } else {
            let core = self.core;
            let mine = value.clone();
            d.tab_stop(move || core.is_highlighted_untracked(&mine))
        };
        if !availability.actionable() {
            return d;
        }
        let me = self;
        let id = focus.id();
        let hover = self.core.config().hover;
        let hovered = value;
        d.when(hover, |d| {
            d.on_hover(move |inside| {
                if inside {
                    let items = me.items();
                    if let Some(i) = items.index_of(id) {
                        me.core.highlight_at(i, hovered.clone());
                    }
                }
            })
        })
        .on_click(move |_| {
            let items = me.items();
            if let Some(i) = items.index_of(id) {
                items.with(|list| {
                    if !field {
                        me.core.highlight(list, i, false);
                    }
                    me.choose(list, i);
                });
            }
        })
    }

    /// An option valued `value` went: a highlight on it is let go
    /// (`ListboxItem.vue:59-64`, which compares the element, not the
    /// value). The option's setup registers it. Told by the option, not by
    /// its value: one that replaced it with the same value (keyed anew)
    /// keeps the highlight, so it is let go only if no option laid out
    /// after it went holds the value. The work is the list's (queued for
    /// the scope it was made in), as the option is leaving.
    pub fn option_unmounted(self, value: &T) {
        let value = value.clone();
        let work = move || {
            if !self.core.is_alive() || !self.core.is_highlighted_untracked(&value) {
                return;
            }
            let held = self.parts.is_alive() && self.items().with(|list| list.iter().any(|i| i.value == value));
            if !held {
                self.core.clear();
            }
        };
        self.later(work);
    }

    /// Run `f` after the next layout as the list's work: queued for the
    /// scope it was made in, whoever asks (an option leaving, a caller's
    /// code outside any part), and never once that scope is gone.
    fn later(self, f: impl FnOnce() + 'static) {
        match self.owner {
            Some(owner) => owner.after_layout(f),
            None => gui::after_layout(f),
        }
    }

    /// `d` as a group of options, named by its label's node `label`.
    #[must_use]
    pub fn group(d: Div, label: Option<NodeRef>) -> Div {
        let d = d.role(gui::Role::Group);
        match label {
            Some(l) => d.labelled_by(l),
            None => d,
        }
    }

    /// `d` as the label of a group, `label` the node the group names itself by.
    #[must_use]
    pub fn group_label(d: Div, label: NodeRef) -> Div {
        d.node_ref(label)
    }

    /// `d` as the filter field (its setup called [`Listbox::set_filter`]):
    /// a combobox to assistive technology, expanded, controlling the list
    /// and naming the highlighted option (reka's filter is a bare text box,
    /// research §4 item 14); it takes the arrows, Home, End and Enter
    /// before its own editing, and leaves them while an input method
    /// composes.
    #[must_use]
    pub fn filter<K>(self, d: Node<K>) -> Node<K> {
        let me = self;
        let availability = self.availability.get();
        let d = d
            .role(gui::Role::ComboBox)
            .aria_expanded(true)
            .aria_auto_complete(gui::AutoComplete::List)
            .controls(self.node)
            .active_descendant(self.core.has_highlight().then(|| self.core.active()));
        let d = if availability.actionable() { d } else { d.disabled(true) };
        d.on_key_down(move |ev| {
            let composing = me
                .filter
                .with_untracked(|f| f.is_some_and(|f| f.edit.with_untracked(|e| e.preedit().is_some())));
            if composing {
                return false;
            }
            let items = me.items();
            items.with(|list| me.key(ev, list))
        })
    }

    /// The list is virtual (its setup calls it): its options are `source`'s
    /// data, laid on `state`'s rows one per option. The mount highlight, a
    /// caller's value, Tab into the list and the keys all read that data,
    /// so they reach options whose rows were never built, and bring the
    /// highlighted one into view (reka's `ListboxVirtualizer`).
    pub fn set_virtual(self, state: ListState, source: Source<T>) {
        // The highlighted option's row stays built: read as the list renders.
        let (core, data) = (self.core, Rc::clone(&source));
        state.active_from(move || core.index_tracked(&*data()).and_then(|i| u64::try_from(i).ok()));
        // Held from its mount to its unmount: a list that replaced this one
        // (the same `state` and `source` or not) holds it after this one
        // left.
        self.parts
            .with_untracked(|p| p.virtual_list)
            .hold(Virtual { state, source });
    }

    /// `d` as a virtual list's node (its setup called
    /// [`Listbox::set_virtual`]): a listbox that holds focus, takes the
    /// list's keys and names the highlighted option as its active
    /// descendant, whose row stays built while scrolled away; the keys run
    /// on the data, Home, End and the page keys reaching rows never built,
    /// and the row reached is brought into view. Its render reads the data.
    #[must_use]
    pub fn virtual_content(self, d: Div) -> Div {
        let Some((_, source)) = self.virtual_list() else {
            return d;
        };
        let options = source();
        let at = self.core.index_tracked(&*options);
        let me = self;
        let d = self
            .availability
            .get()
            .apply(d)
            .role(gui::Role::ListBox)
            .aria_multiselectable(M::MULTIPLE)
            .node_ref(self.node)
            .track_focus(self.content)
            .active_descendant(at.map(|_| self.core.active()));
        self.pressed_in(d).on_key_down(move |ev| {
            let taken = me.with_options(|o| me.key(ev, o));
            me.reveal();
            taken
        })
    }

    /// `d` as option `index` of a virtual list's `options` (what the row's
    /// render read): an option chosen or not, its availability applied (the
    /// list's being disabled disables it), its place in the whole set, the
    /// active descendant while highlighted; a click highlights and chooses
    /// it.
    #[must_use]
    pub fn virtual_option<O: Options<T> + ?Sized>(self, d: Div, options: &O, index: usize) -> Div {
        let Some(value) = options.value(index) else { return d };
        let availability = match self.availability.get() {
            Availability::Enabled => options.availability(index),
            root => root,
        };
        let chosen = self.value.get().holds(&value);
        let len = options.len();
        let me = self;
        let picked = value.clone();
        self.core
            .option(d, index, &value, availability, chosen, move || {
                me.with_options(|o| {
                    let at = if o.value(index).as_ref() == Some(&picked) {
                        Some(index)
                    } else {
                        o.position(&picked)
                    };
                    if let Some(i) = at {
                        me.core.highlight(o, i, false);
                        me.choose(o, i);
                    }
                });
            })
            .aria_pos_in_set(index.saturating_add(1), len)
    }
}

/// Whether `ev` types the letter `letter`, case aside.
fn is_letter(ev: &KeyEvent, letter: char) -> bool {
    let typed = match &ev.key {
        gui::Key::Character(c) => Some(c.as_str()),
        _ => ev.text.as_deref(),
    };
    typed.is_some_and(|t| t.eq_ignore_ascii_case(letter.encode_utf8(&mut [0; 4])))
}

/// A listbox in a form (reka's hidden input): its value, and its content
/// focused when a check fails.
impl<T: Clone + PartialEq + 'static, M: Choice<T>> crate::form::FormValue for Listbox<T, M> {
    type Value = M;

    fn value(&self) -> M {
        self.value.get()
    }

    fn set_value(&self, value: M) {
        self.value.set(value);
    }

    fn focus(&self) -> Option<FocusHandle> {
        Some(self.content)
    }
}

#[cfg(test)]
mod tests {
    use super::{Choice, SelectionBehavior};

    #[test]
    fn one_or_several_choose_by_the_rule() {
        let mut one: Option<&str> = None;
        one.choose(&"a", SelectionBehavior::Toggle);
        assert_eq!(one, Some("a"));
        one.choose(&"a", SelectionBehavior::Toggle);
        assert_eq!(one, None, "toggle clears the chosen");
        one.choose(&"a", SelectionBehavior::Replace);
        one.choose(&"a", SelectionBehavior::Replace);
        assert_eq!(one, Some("a"), "replace keeps it");
        let mut many: Vec<&str> = Vec::new();
        many.choose(&"b", SelectionBehavior::Toggle);
        many.choose(&"a", SelectionBehavior::Toggle);
        assert_eq!(many, ["b", "a"], "joins at the end");
        many.choose(&"b", SelectionBehavior::Toggle);
        assert_eq!(many, ["a"]);
        many.hold(vec!["x", "y"]);
        assert_eq!(many.values(), ["x", "y"]);
        one.hold(vec!["x", "y"]);
        assert_eq!(one, Some("y"));
    }

    /// Opus 2 (F1 review 4): a `Held` value's fork policy is checked, not
    /// said. A fork writing a value only the outside world writes is loud
    /// in a debug build and writes nothing; one held in signals is written
    /// and put back.
    #[test]
    #[cfg(debug_assertions)]
    fn a_fork_writing_a_value_only_the_outside_writes_is_loud() {
        use super::{Bound, Held, HeldFork};
        use std::cell::Cell;
        use std::rc::Rc;
        struct Outside(Rc<Cell<u32>>);
        impl Held<u32> for Outside {
            fn fork_policy(&self) -> HeldFork {
                HeldFork::Outside("a test's cell")
            }
            fn get(&self) -> u32 {
                self.0.get()
            }
            fn get_untracked(&self) -> u32 {
                self.0.get()
            }
            fn set(&self, value: u32) {
                self.0.set(value);
            }
        }
        let cell = Rc::new(Cell::new(0));
        let outside = Rc::new(Bound::new(Outside(Rc::clone(&cell))));
        let held = Rc::clone(&outside);
        let wrote = gui::testing::in_check(move || std::panic::catch_unwind(|| held.set(1)));
        assert!(wrote.is_err(), "loud");
        assert_eq!(cell.get(), 0, "nothing written");
        outside.set(2);
        assert_eq!(cell.get(), 2, "live, written");
        let signal = gui::Signal::new(0_u32);
        let held = Bound::new(signal);
        gui::testing::in_check(move || held.set(1));
        assert_eq!(signal.get_untracked(), 0, "a signal, written in the fork and put back");
    }
}
