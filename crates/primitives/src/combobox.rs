//! A combobox (reka's `Combobox`, on its `Listbox`): a text field that
//! filters a list of options, which floats against the field's anchor
//! (research §9, Combobox).
//!
//! The root ([`Combobox`]) keeps the value (one or several: [`Choice`]),
//! whether the list is open, the field's text and the filter term, and the
//! highlight. Focus stays in the field; the highlighted option is the
//! field's active descendant ([`ListFocus::Field`]).
//!
//! # State
//!
//! - **Choosing**, one value: the option becomes the value and the list
//!   closes; choosing the chosen one keeps it (Replace, unlike Listbox's
//!   Toggle). Several: the option joins (at the end) or leaves, the list
//!   stays open and focus goes back to the field.
//! - **The field's text** is put back to what the value shows
//!   ([`Combobox::new`]'s `display`): as the list closes when
//!   [`ComboboxConfig::reset_on_close`] (default on), and when the value
//!   changes while the list is closed when
//!   [`ComboboxConfig::reset_on_select`] (default on); a choice of one
//!   value lands as on a closed list (reka closes, then writes), so it is
//!   put back by either.
//!   While it is open, the text stays as typed: keeping the typed text
//!   after a choice takes turning off both.
//! - **The filter term** is what the user typed since the list opened: it
//!   is emptied as the list closes, so a list opened again shows every
//!   option though the field still holds text (`ComboboxRoot.vue:159`
//!   empties it on opening too; emptied at every close, it is already
//!   empty then).
//! - **The highlight** is let go as the list closes, so Enter on a closed
//!   combobox never chooses; opening highlights the chosen option, else
//!   the first, unless typing opened it: then the first match is
//!   highlighted, whatever is chosen.
//!
//! # Keys (in the field)
//!
//! | Key | What it does |
//! | --- | --- |
//! | ArrowDown / ArrowUp, closed | opens, highlighting the chosen option, else the first |
//! | ArrowDown / ArrowUp, open | next / previous usable option; stops at the ends unless it wraps |
//! | Home / End | first / last option; the caret does not move, even while closed |
//! | Enter with a highlight | chooses it; consumed, so the form does not submit |
//! | Control, Alt or Command + Enter; Enter without a highlight | left to the form |
//! | typing, closed | opens, filtered, the first match highlighted |
//! | typing, open | filters; each change highlights the first match |
//! | while an input method composes | the arrows and Enter are the input method's |
//! | Escape | closes (the layer's); the text is put back if `reset_on_close` |
//!
//! While the list is open, PageUp and PageDown go to the first and last
//! option, as in every list of the foundation ([`crate::keymap::Keymap::LIST`]);
//! closed, they are the field's. Tab, Space, Left and Right are always the
//! field's.
//!
//! # Closing
//!
//! Escape, a press outside (not on the root, its anchor or its trigger),
//! choosing one value, the trigger, and focus leaving both the root and the
//! list: [`Combobox::popup`]'s layer, the engine deciding each (ADR-0024:
//! focus outside is told once focus has settled, so moving between two
//! comboboxes does not close the wrong one). It is not modal: a press
//! outside reaches what it is on and the page scrolls.
//!
//! # Where this differs from reka, and why
//!
//! - reka puts the text back 1 ms after closing (`ComboboxRoot.vue:170-176`)
//!   so it lands after the choice; here the close itself does it, after the
//!   value was written.

use std::rc::Rc;

use gui::element::Node;
use gui::events::{KeyEvent, NamedKey};
use gui::{AutoFocus, Dismiss, DismissPolicy, Div, FocusHandle, FocusScope, Layer, Signal, TextField};

use crate::availability::Availability;
use crate::listbox::{Bound, Choice, ListConfig, ListCore, ListFocus, ListKey, Options, Pick, Source};
use crate::relation::{Linked, Points};
use crate::selection::SelectionBehavior;

/// A combobox's behaviour options; reka's defaults (`ComboboxRoot.vue:116-125`,
/// `ComboboxInput.vue:31-34`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComboboxConfig {
    /// Put the field's text back to the value's as the list closes.
    pub reset_on_close: bool,
    /// Put the field's text back when the value changes while closed.
    pub reset_on_select: bool,
    /// Open when the field takes focus.
    pub open_on_focus: bool,
    /// Open when the field is clicked.
    pub open_on_click: bool,
    /// The cancel button clears the value too, not only the text.
    pub cancel_clears_value: bool,
    /// Focus the field as the list opens.
    pub focus_on_open: bool,
}

impl Default for ComboboxConfig {
    fn default() -> Self {
        Self::REKA
    }
}

impl ComboboxConfig {
    pub const REKA: Self = Self {
        reset_on_close: true,
        reset_on_select: true,
        open_on_focus: false,
        open_on_click: false,
        cancel_clears_value: false,
        focus_on_open: true,
    };

    /// An autocomplete's: the text is the value, so nothing puts it back
    /// (`AutocompleteRoot.vue:219-248`); cancel empties it, the field takes
    /// focus as the list opens.
    pub const AUTOCOMPLETE: Self = Self {
        reset_on_close: false,
        reset_on_select: false,
        open_on_focus: false,
        open_on_click: false,
        cancel_clears_value: true,
        focus_on_open: true,
    };
}

/// What the field's text is put back to: the value as shown.
type Display<M> = Rc<dyn Fn(&M) -> String>;

/// A combobox whose value is its text (an autocomplete): the text, and
/// what an option's value writes to it.
struct TextValue<T: 'static> {
    text: Bound<String>,
    of: Rc<dyn Fn(&T) -> String>,
}

impl<T: 'static> Clone for TextValue<T> {
    fn clone(&self) -> Self {
        Self {
            text: self.text,
            of: Rc::clone(&self.of),
        }
    }
}

#[derive(Clone)]
struct State<T: 'static, M: 'static> {
    config: ComboboxConfig,
    display: Display<M>,
    /// The list part's options, held while it is mounted.
    source: gui::Holder<Source<T>>,
    text: Option<TextValue<T>>,
}

/// A combobox: made once in its root's setup, `Copy`.
pub struct Combobox<T: 'static, M: Choice<T>> {
    value: Bound<M>,
    open: Bound<bool>,
    field: TextField,
    term: Signal<String>,
    list: ListCore<T>,
    state: Signal<State<T, M>>,
    trigger: FocusHandle,
    linked: Linked,
    /// The text the combobox last wrote to the field itself, so that write
    /// is no typing.
    wrote: Signal<Option<String>>,
    /// Typing opened the list: the typing highlights the first match, so
    /// the opening does not highlight the chosen option over it.
    typing_opened: Signal<bool>,
    /// Whether it can be used (reka's `disabled`).
    availability: Signal<Availability>,
}

impl<T: 'static, M: Choice<T>> Clone for Combobox<T, M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static, M: Choice<T>> Copy for Combobox<T, M> {}

impl<T: 'static, M: Choice<T>> PartialEq for Combobox<T, M> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.open == other.open
    }
}

impl<T: 'static, M: Choice<T>> std::fmt::Debug for Combobox<T, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Combobox").finish_non_exhaustive()
    }
}

impl<T: Clone + PartialEq + 'static, M: Choice<T>> Combobox<T, M> {
    /// A combobox holding `value`, open as `open` says, typed in `field`,
    /// whose text is put back to `display` of the value; made in its root's
    /// setup (its watchers live in that scope).
    #[must_use]
    #[track_caller]
    pub fn new(
        value: Bound<M>,
        open: Bound<bool>,
        field: TextField,
        config: ComboboxConfig,
        display: impl Fn(&M) -> String + 'static,
    ) -> Self {
        let behavior = if M::MULTIPLE {
            SelectionBehavior::Toggle
        } else {
            SelectionBehavior::Replace
        };
        let display: Display<M> = Rc::new(display);
        let me = Self {
            value,
            open,
            field,
            term: Signal::labeled("combobox term", String::new()),
            list: ListCore::new(
                ListConfig::LISTBOX
                    .focus(ListFocus::Field)
                    .behavior(behavior)
                    .hover(true),
            ),
            state: Signal::new(State {
                config,
                display,
                source: gui::Holder::new(),
                text: None,
            }),
            trigger: FocusHandle::new(),
            linked: Linked::new(Points::Controls),
            wrote: Signal::new(None),
            typing_opened: Signal::new(false),
            availability: Signal::new(Availability::Enabled),
        };
        // Typing filters: a change of the text not written by the
        // combobox is the user's.
        gui::watch(
            move || field.text(),
            move |text, _| {
                if !me.term.is_alive() {
                    return;
                }
                if me.wrote.get_untracked().as_deref() == Some(text.as_str()) {
                    me.wrote.set(None);
                    return;
                }
                me.typed(text);
            },
        );
        // Opening and closing: the term empties, the highlight is let go
        // or set, the text is put back.
        gui::watch(
            move || open.get(),
            move |now, _| {
                if !me.term.is_alive() {
                    return;
                }
                if *now {
                    // Typing that opened it set the term and highlights
                    // its first match (`ComboboxInput.vue:68-81`).
                    if me.typing_opened.get_untracked() {
                        me.typing_opened.set(false);
                        return;
                    }
                    // An autocomplete filters by its text as it stands.
                    if let Some(t) = me.text_value() {
                        me.term.set(t.text.get_untracked());
                    }
                    gui::after_layout(move || me.highlight_chosen());
                } else {
                    // The term is what was typed since the list opened:
                    // emptied as it closes, so the list opens unfiltered
                    // (typing that opens it sets the term after).
                    me.typing_opened.set(false);
                    me.term.set(String::new());
                    me.list.clear();
                    if me.config().reset_on_close {
                        me.reset_text();
                    }
                }
            },
        );
        // The field taking focus opens the list, when the config says so.
        gui::watch(gui::focus::focused, move |now, _| {
            if me.term.is_alive()
                && *now == Some(field.focus.id())
                && me.config().open_on_focus
                && !me.open.get_untracked()
            {
                me.open.set(true);
            }
        });
        // The field starts showing the value (`ComboboxInput.vue:155-158`,
        // an immediate watch).
        me.reset_text();
        // A value written while closed puts the text back.
        gui::watch(
            move || value.get(),
            move |_, _| {
                if me.term.is_alive() && !me.open.get_untracked() && me.config().reset_on_select {
                    me.reset_text();
                }
            },
        );
        me
    }

    /// Whether it can be used (reka's `disabled`): not usable, it does not
    /// open, its field, button and trigger are read as disabled and its keys
    /// do nothing.
    pub fn set_availability(self, availability: Availability) {
        self.availability.set(availability);
        if !availability.actionable() && self.open.get_untracked() {
            self.open.set(false);
        }
    }

    /// Whether it can be used (tracked).
    #[must_use]
    pub fn availability(self) -> Availability {
        self.availability.get()
    }

    fn usable(self) -> bool {
        self.availability.get_untracked().actionable()
    }

    fn config(self) -> ComboboxConfig {
        self.state.with_untracked(|s| s.config)
    }

    fn text_value(self) -> Option<TextValue<T>> {
        self.state.with_untracked(|s| s.text.clone())
    }

    /// Make its value its text (reka's `Autocomplete`, built on this):
    /// typing writes `text`, choosing an option writes `of` its value, a
    /// caller's text shows in the field, opening filters by the text and
    /// highlights the option whose value it is, cancel empties it. Called
    /// in setup, after [`Combobox::new`]; the config should put no text
    /// back ([`ComboboxConfig::AUTOCOMPLETE`]).
    pub fn set_text_value(self, text: Bound<String>, of: impl Fn(&T) -> String + 'static) {
        let of: Rc<dyn Fn(&T) -> String> = Rc::new(of);
        self.state.update(|s| s.text = Some(TextValue { text, of }));
        self.write(text.get_untracked());
        let me = self;
        gui::watch(
            move || text.get(),
            move |v, _| {
                if me.term.is_alive() {
                    me.write(v.clone());
                }
            },
        );
    }

    /// The field's text as the value shows it.
    #[must_use]
    pub fn display(self) -> String {
        let display = self.state.with_untracked(|s| Rc::clone(&s.display));
        display(&self.value.get_untracked())
    }

    /// Write `text` to the field, as the combobox (no typing).
    fn write(self, text: String) {
        if self.field.edit.with_untracked(|e| e.text() == text) {
            return;
        }
        self.wrote.set(Some(text.clone()));
        self.field.set_text(text);
    }

    /// Put the field's text back to what the value shows.
    pub fn reset_text(self) {
        self.write(self.display());
    }

    /// The user typed: the term follows, the list opens, the first match is
    /// highlighted once the list shows the new term.
    fn typed(self, text: &str) {
        if self.field.edit.with_untracked(|e| e.preedit().is_some()) {
            return;
        }
        if let Some(t) = self.text_value() {
            t.text.set(text.to_owned());
        }
        if !self.usable() {
            return;
        }
        if !self.open.get_untracked() {
            self.typing_opened.set(true);
            self.open.set(true);
            if !self.open.get_untracked() {
                // A caller that keeps `open` as a value opens it as it
                // renders again, this turn; one that keeps it closed
                // leaves it as if nothing was typed into the list: no
                // later opening was typing's, and nothing filters it.
                gui::after_layout(move || {
                    if self.term.is_alive() && !self.open.get_untracked() {
                        self.typing_opened.set(false);
                        self.term.set(String::new());
                    }
                });
            }
        }
        self.term.set(text.to_owned());
        gui::after_layout(move || {
            if let Some(o) = self.options() {
                self.list.highlight_first(&*o, false);
            }
        });
    }

    /// The list part gives its options (in its setup); its unmounting takes
    /// them away.
    pub fn set_source(self, source: Source<T>) {
        // Held from its mount to its unmount: a list part that replaced
        // this one (the same `source` or not) holds it after this one left.
        self.state.with_untracked(|s| s.source).hold(source);
    }

    /// The options the list shows now.
    #[must_use]
    pub fn options(self) -> Option<Rc<dyn Options<T>>> {
        let source = self.state.with_untracked(|s| s.source).get_untracked()?;
        Some(gui::untracked(|| source()))
    }

    /// Ask `f` before each choice (reka's `@select`, prevented).
    pub fn on_pick(self, f: impl Into<gui::Callback<T, Pick>>) {
        self.list.on_pick(f.into());
    }

    fn highlight_chosen(self) {
        if !self.open.get_untracked() {
            return;
        }
        let Some(o) = self.options() else { return };
        if let Some(t) = self.text_value() {
            let text = t.text.get_untracked();
            let at = (0..o.len()).find(|&i| o.focusable(i) && o.value(i).is_some_and(|v| (t.of)(&v) == text));
            match at {
                Some(i) => self.list.highlight(&*o, i, false),
                None => {
                    self.list.highlight_first(&*o, false);
                }
            }
            return;
        }
        let value = self.value.get_untracked();
        self.list.highlight_chosen(&*o, &value, false);
    }

    /// The value (tracked).
    #[must_use]
    pub fn value(self) -> M {
        self.value.get()
    }

    /// Whether `value` is chosen (tracked): an option's indicator; an
    /// autocomplete holds the option whose value is its text.
    #[must_use]
    pub fn holds(self, value: &T) -> bool {
        match self.text_value() {
            Some(t) => (t.of)(value) == t.text.get(),
            None => self.value.get().holds(value),
        }
    }

    /// The text, when its value is its text (an autocomplete; tracked).
    #[must_use]
    pub fn text(self) -> Option<String> {
        self.text_value().map(|t| t.text.get())
    }

    /// Whether the list is open (tracked).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.open.get()
    }

    /// The filter term (tracked): what the list filters by.
    #[must_use]
    pub fn term(self) -> String {
        self.term.get()
    }

    /// The field.
    #[must_use]
    pub fn field(self) -> TextField {
        self.field
    }

    /// The highlight, which the list's look reads.
    #[must_use]
    pub fn list(self) -> ListCore<T> {
        self.list
    }

    /// The trigger's focus.
    #[must_use]
    pub fn trigger_focus(self) -> FocusHandle {
        self.trigger
    }

    /// The trigger and list relation (the field controls the list).
    #[must_use]
    pub fn linked(self) -> Linked {
        self.linked
    }

    /// Open the list, focusing the field (when it focuses on open).
    pub fn show(self) {
        if !self.usable() {
            return;
        }
        self.open.set(true);
        if self.config().focus_on_open {
            self.field.focus.focus();
        }
    }

    /// Close the list; `refocus` focuses the trigger (Escape from the list,
    /// a choice with the field inside the list).
    pub fn close(self, refocus: bool) {
        self.open.set(false);
        if refocus {
            gui::focus::set_focus(Some(self.trigger.id()), false);
        }
    }

    /// Choose option `index` of `options`: one value closes the list (and,
    /// with `refocus`, focuses the trigger); several stay open, focus back
    /// in the field.
    pub fn choose<O: Options<T> + ?Sized>(self, options: &O, index: usize, refocus: bool) -> ListKey {
        if let Some(t) = self.text_value() {
            // The option's value, not the label it shows, is the text
            // (`AutocompleteRoot.vue:111-121`).
            let Some(v) = self.list.pickable(options, index) else {
                return ListKey::Refused;
            };
            t.text.set((t.of)(&v));
            self.close(refocus);
            return ListKey::Chose(index);
        }
        if M::MULTIPLE {
            let key = self.list.choose(options, index, Some(self.value));
            if matches!(key, ListKey::Chose(_)) {
                self.field.focus.focus();
            }
            return key;
        }
        // One value. reka closes, then writes the value (`onOpenChange(false)`
        // before `modelValue`), and puts the text back on closing 1 ms later:
        // either reset shows the chosen option's text. Here the close puts
        // the text back at once, so the value is written first, and a value
        // that changed is then put back as written on a closed list.
        let was = self.value.get_untracked();
        let key = self.list.choose(options, index, Some(self.value));
        if matches!(key, ListKey::Chose(_)) {
            self.close(refocus);
            if self.config().reset_on_select && self.value.get_untracked() != was {
                self.reset_text();
            }
        }
        key
    }

    /// Choose the option valued `value` (a click on a row); see
    /// [`Combobox::choose`].
    pub fn choose_value(self, value: &T, refocus: bool) -> ListKey {
        let Some(o) = self.options() else {
            return ListKey::Ignored;
        };
        match o.position(value) {
            Some(i) => self.choose(&*o, i, refocus),
            None => ListKey::Ignored,
        }
    }

    /// Clear the value (a "none" option, reka's `resetValue`): nothing, or
    /// no value at all for several.
    pub fn clear_value(self) {
        self.value.set(M::default());
    }

    /// The field's keys (the module's table); `refocus` as [`Combobox::choose`].
    pub fn key(self, ev: &KeyEvent, refocus: bool) -> bool {
        if !self.usable() || self.field.edit.with_untracked(|e| e.preedit().is_some()) {
            return false;
        }
        let nav = ev.is(NamedKey::ArrowDown) || ev.is(NamedKey::ArrowUp);
        if !self.open.get_untracked() {
            if nav {
                self.show();
                return true;
            }
            // Home and End never move the caret.
            return ev.is(NamedKey::Home) || ev.is(NamedKey::End);
        }
        let Some(o) = self.options() else {
            return false;
        };
        let text = self.text_value().is_some();
        if (text || !M::MULTIPLE) && ev.is(NamedKey::Enter) {
            // Enter is the choice, through `choose`: the text's write, or one
            // value written after the list closes (reka's order).
            let m = ev.modifiers;
            return match self.list.index_in(&*o) {
                Some(i) if !(m.control || m.alt || m.command) => self.choose(&*o, i, refocus).consumed(),
                _ => false,
            };
        }
        if text {
            return self.list.key::<Option<T>, _>(ev, &*o, None).consumed();
        }
        match self.list.key(ev, &*o, Some(self.value)) {
            ListKey::Chose(_) => {
                if M::MULTIPLE {
                    self.field.focus.focus();
                } else {
                    self.close(refocus);
                }
                true
            }
            other => other.consumed(),
        }
    }

    /// `d` as the field around which the text input sits: a combobox to
    /// assistive technology, expanded and controlling the list while open
    /// (`open`, what the look's render read), naming the highlighted
    /// option as its active descendant.
    #[must_use]
    pub fn input<K>(self, d: Node<K>, open: bool) -> Node<K> {
        let d = d
            .node_ref(self.linked.trigger)
            .role(gui::Role::ComboBox)
            .aria_expanded(open)
            .aria_auto_complete(gui::AutoComplete::List);
        let d = if self.availability.get().actionable() {
            d
        } else {
            d.disabled(true)
        };
        let d = if open {
            d.controls(self.linked.content)
                .active_descendant(self.list.has_highlight().then(|| self.list.active()))
        } else {
            d
        };
        let config = self.config();
        if config.open_on_click {
            d.on_click(move |_| self.show())
        } else {
            d
        }
    }

    /// `d` as the floating part that holds the list (reka's
    /// `ComboboxContent` as a dismissable layer; the look declares it, in
    /// its deferred subtree, inside the combobox's root): Escape, a press
    /// outside it and the root, and focus leaving both the root and it
    /// close the list (the engine decides each once focus settles, so
    /// moving between two comboboxes closes the right one). It is not
    /// modal and traps nothing; as it goes while it holds focus (a field
    /// inside it), focus goes to the trigger (`ComboboxContentImpl.vue:
    /// 118-135`).
    #[must_use]
    pub fn popup(self, d: Div) -> Div {
        let layer = Layer::new()
            .dismiss(DismissPolicy::DEFAULT)
            .focus_scope(
                // reka's `FocusScope` here neither loops nor traps: Tab goes on
                // past the list, and focus leaving dismisses it.
                FocusScope::new()
                    .cycle(false)
                    .auto_focus(AutoFocus::Keep)
                    .return_to(self.trigger),
            )
            .on_dismiss(move |_| {
                self.close(false);
                Dismiss::Close
            });
        d.layer(layer)
    }

    /// `d` as option `index` (its place in the source's options), valued
    /// `value`, of a list laid out as data (a virtual list's row, or the
    /// rows of the source): an option chosen or
    /// not ([`Combobox::holds`]), its availability applied, the field's
    /// active descendant while highlighted; the pointer highlights it and a
    /// click chooses it (one value closes the list, and with `refocus`
    /// focuses the trigger).
    #[must_use]
    pub fn option(
        self,
        d: Div,
        index: usize,
        value: &T,
        availability: crate::availability::Availability,
        refocus: bool,
    ) -> Div {
        let chosen = self.holds(value);
        let picked = value.clone();
        self.list.option(d, index, value, availability, chosen, move || {
            self.choose_value(&picked, refocus);
        })
    }

    /// `d` as the list the field controls (its node; the look places it).
    #[must_use]
    pub fn content(self, d: Div) -> Div {
        self.linked
            .content(d, false)
            .role(gui::Role::ListBox)
            .aria_multiselectable(M::MULTIPLE)
    }

    /// `d` as a button that opens the list and stays in the Tab order: the
    /// way in when the field is inside the list (shadcn's combobox, a
    /// button opening a searchable list). Expanded while open, controlling
    /// the list; a click opens or closes it; Enter, Space, ArrowDown and
    /// ArrowUp open it.
    #[must_use]
    pub fn button(self, d: Div, open: bool) -> Div {
        let availability = self.availability.get();
        let d = self
            .linked
            .trigger(availability.apply(d), open)
            .track_focus(self.trigger)
            .aria_expanded(open)
            .aria_has_popup(gui::HasPopup::Listbox);
        if !availability.actionable() {
            return d;
        }
        d.on_click(move |_| {
            if self.open.get_untracked() {
                self.close(false);
            } else {
                self.show();
            }
        })
        .on_key_down(move |ev| {
            let opens = crate::keymap::activates(ev) || ev.is(NamedKey::ArrowDown) || ev.is(NamedKey::ArrowUp);
            if self.open.get_untracked() || !opens {
                return false;
            }
            self.show();
            true
        })
    }

    /// `d` as the trigger: out of the Tab order, expanded and controlling
    /// the list while open, disabled with the combobox
    /// (`ComboboxTrigger.vue:39-55`); a click opens or closes the list;
    /// ArrowDown or ArrowUp in it, open, moves focus to the field. `name` is
    /// its name to assistive technology (reka's "Show popup").
    #[must_use]
    pub fn trigger(self, d: Div, open: bool, name: &str) -> Div {
        let availability = self.availability.get();
        let d = availability
            .apply(d)
            .track_focus(self.trigger)
            .tab_stop(false)
            .aria_label(name)
            .aria_expanded(open)
            .aria_has_popup(gui::HasPopup::Listbox);
        let d = if open { d.controls(self.linked.content) } else { d };
        if !availability.actionable() {
            return d;
        }
        d.on_click(move |_| {
            if self.open.get_untracked() {
                self.close(false);
            } else {
                self.show();
            }
        })
        .on_key_down(move |ev| {
            let nav = ev.is(NamedKey::ArrowDown) || ev.is(NamedKey::ArrowUp);
            if nav && self.open.get_untracked() && !self.config().focus_on_open {
                self.field.focus.focus();
                return true;
            }
            false
        })
    }

    /// The cancel button: empties the text (and the value, when the config
    /// says so), focuses the field.
    pub fn cancel(self) {
        if let Some(t) = self.text_value() {
            t.text.set(String::new());
            self.field.focus.focus();
            return;
        }
        self.clear_search();
        if self.config().cancel_clears_value {
            self.clear_value();
        }
        self.field.focus.focus();
    }

    /// Empty the field and the filter term, the list staying as it is (a
    /// command list's pick empties its search, `CommandItem.vue:71-73`).
    pub fn clear_search(self) {
        self.write(String::new());
        self.term.set(String::new());
    }
}

/// A combobox in a form (reka's hidden input): its value, and its field
/// focused when a check fails.
impl<T: Clone + PartialEq + 'static, M: Choice<T>> crate::form::FormValue for Combobox<T, M> {
    type Value = M;

    fn value(&self) -> M {
        self.value.get()
    }

    fn set_value(&self, value: M) {
        self.value.set(value);
    }

    fn focus(&self) -> Option<FocusHandle> {
        Some(self.field.focus)
    }
}

#[cfg(test)]
mod tests {
    use super::ComboboxConfig;

    #[test]
    fn reka_defaults() {
        let c = ComboboxConfig::default();
        assert!(c.reset_on_close && c.reset_on_select && c.focus_on_open);
        assert!(!c.open_on_focus && !c.open_on_click && !c.cancel_clears_value);
    }
}
