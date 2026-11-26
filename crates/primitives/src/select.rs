//! A select (reka's `Select`): a trigger showing what is chosen, opening a
//! list of options that floats against it (research §9, Select).
//!
//! The root ([`Select`]) keeps the value (one, `Option<T>`, or several,
//! `Vec<T>`: [`Choice`]) and whether the list is open, each its own or the
//! caller's ([`Bound`] over the part's `UseModel`). Its parts add the
//! behaviour to the look's nodes: [`Select::trigger`], [`Select::content`],
//! [`Select::viewport`], [`Select::item`], [`Select::scroll_button`], and
//! the free [`group`], [`group_label`] and [`separator`]. The look reads
//! [`Select::is_open`], [`Select::is_empty`], [`Select::shown`],
//! [`Select::holds`], [`Select::is_highlighted`], [`Select::anchor`] and
//! [`Select::can_scroll`].
//!
//! # Options are known while closed
//!
//! The items stay laid out while the list is closed, hidden and inert
//! ([`Select::content`]: no paint, no hit, no focus, not read), so they are
//! known in display order whether it is open or not: the trigger shows the
//! chosen labels from the first frame and typeahead on a closed trigger
//! chooses among them (reka renders its items into a `DocumentFragment`
//! for this, `SelectContent.vue:37-44,62-72`). They are the same instances
//! open and closed.
//!
//! # Keys
//!
//! Trigger:
//!
//! | Key | What it does |
//! | --- | --- |
//! | Space, Enter, ArrowDown, ArrowUp | opens (unless disabled), the search reset; focus goes to the anchor option |
//! | a printable character, closed, one value | chooses the option the search reaches, without opening; disabled ones are skipped |
//! | Space while a search runs | a character of the search: neither opens nor chooses |
//!
//! Content (focus on an option): the list's table ([`crate::listbox`]) with
//! [`Keymap::LIST`](crate::keymap::Keymap::LIST) (stops at the ends; `wrap` makes it wrap), no select-all
//! and no Shift range (reka's Select has neither); Enter or Space chooses:
//! one value closes and focuses the trigger, several toggle and stay open;
//! Tab is consumed (focus does not leave the list); Escape closes (the
//! layer's) and focuses the trigger.
//!
//! # The anchor option
//!
//! What opening focuses, and what an item-aligned list lines up on the
//! trigger's value (`SelectContentImpl.vue:254-280`): one value, the chosen
//! option, or the first usable one when nothing is chosen or the chosen one
//! comes before it (disabled); several, the first chosen in display order,
//! else the first usable one. When the anchor cannot take focus (disabled),
//! the list itself takes it.
//!
//! # Pointer
//!
//! - A primary press without Control on the trigger opens the list at once
//!   ([`PRESS_SLOP`]). Released within 10 px of the press on both axes, it
//!   chooses nothing: the list just opened under the pointer. Dragged
//!   further and released on an option, it chooses it; released outside the
//!   list, it closes it (`SelectContentImpl.vue:151-199`).
//! - A touch opens the list as the tap ends, and an option is chosen only by
//!   a tap that started on it, so a finger scrolling the list never chooses
//!   (`gui::gesture`: a touch's click is a tap).
//! - The pointer over an option highlights (focuses) it; over a disabled
//!   one, or leaving the highlighted one, the list takes focus back and
//!   nothing is highlighted.
//! - Resting on a scroll button scrolls the list one anchor option's height
//!   every [`SCROLL_EVERY`]; leaving it, or the list reaching its end that
//!   way (the button goes), stops it (`SelectScrollButtonImpl.vue`).
//!
//! # The two placeholder rules
//!
//! The trigger is "empty" by the value ([`Select::is_empty`]: nothing
//! chosen), but the value part shows its placeholder when no option has the
//! value ([`Select::shown`] gives `None`): a value no option has makes a
//! trigger that is not empty showing the placeholder (`SelectTrigger.vue:224`,
//! `SelectValue.vue:31-46`). Several values show their labels joined by
//! `", "`, in the order chosen.
//!
//! # Where this differs from reka, and why
//!
//! - The list is told to assistive technology as multi-select when it holds
//!   several (`aria-multiselectable`); reka's never is (research §4 item 14;
//!   ADR-0025, group 5: a difference to assistive technology, not to pixels,
//!   so not a `verdict/better.tsv` entry).
//! - Items may have any value, `''` included: no value is `None`.
//! - Closing plays no exit animation: the list is kept mounted, so it is
//!   hidden at once as kept-mounted content is ([`crate::presence`]; ADR-0025,
//!   "Group 5: third review fixes", item 3).

use gui::engine::{WorldCell, WorldRefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gui::clock::{self, TimerId};
use gui::events::{DragEvent, DragPhase, KeyEvent, NamedKey};
use gui::focus::FocusId;
use gui::prelude::*;
use gui::{
    AutoFocus, Bounds, Direction, Dismiss, DismissPolicy, DismissReason, Div, FocusHandle, FocusScope, Layer,
    Placement, ScrollHandle, ScrollLayout, SideAlign, Signal,
};

use crate::availability::Availability;
use crate::collection::{Collection, Item};
use crate::direction::ReadDirection;
use crate::keymap::Wrap;
use crate::listbox::{Bound, Choice, ListConfig, ListCore, ListFocus, ListKey, Options, Pick};
use crate::popper::Popper;
use crate::relation::{Linked, Points};
use crate::selection::SelectionBehavior;

/// How far the pointer may move from its press on the trigger and still
/// choose nothing when released (`SelectContentImpl.vue:166-180`, research
/// §2.3), on each axis.
pub const PRESS_SLOP: f32 = 10.0;

/// How often a scroll button scrolls while the pointer rests on it
/// (`SelectScrollButtonImpl.vue:32-47`).
pub const SCROLL_EVERY: Duration = Duration::from_millis(50);

/// An option as the trigger and the look know it: value, label, whether it
/// can be used.
#[derive(Clone, Debug, PartialEq)]
pub struct Known<T> {
    pub value: T,
    pub label: Rc<str>,
    pub availability: Availability,
}

/// The press on the trigger that opened the list, while it is held.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    x: f32,
    y: f32,
    /// Moved beyond [`PRESS_SLOP`] on some axis since.
    moved: bool,
}

/// What a select keeps beside its value and open state. Its cells are the
/// world's that made the select: a check writes none of a live one's
/// (ADR-0034, S7).
struct Parts<T: 'static> {
    items: Collection<T>,
    /// The boxes of the options as last laid out: written by the select's
    /// own layout callback on each option's node (never by a render or a
    /// setup), and let go of with the options that left.
    boxes: WorldRefCell<HashMap<FocusId, Bounds>>,
    list_box: WorldCell<Option<Bounds>>,
    /// The list's scroll: made in the select's setup, so it goes with the
    /// select's scope; started afresh as the list opens and as it is
    /// placed again ([`ScrollHandle::reset`]).
    scroll: ScrollHandle,
    stepping: WorldCell<Option<TimerId>>,
    /// Scroll buttons shown: up, down.
    buttons: WorldCell<(bool, bool)>,
    /// Placed again since it opened (once, as the up button came).
    replaced: WorldCell<bool>,
    /// The next opening is from the keyboard ([`Select::open`]'s word),
    /// so the anchor option shows its focus.
    by_keyboard: WorldCell<bool>,
}

/// A select: made once in its root's setup, `Copy`.
pub struct Select<T: 'static, M: Choice<T>> {
    value: Bound<M>,
    open: Bound<bool>,
    availability: Signal<Availability>,
    required: Signal<bool>,
    list: ListCore<T>,
    known: Signal<Rc<Vec<Known<T>>>>,
    press: Signal<Option<Press>>,
    /// The scroll buttons changed: a render of the content reads it.
    buttons: Signal<(bool, bool)>,
    /// Counts the times an item-aligned list was placed again: the
    /// viewport's render reads it, and a new count opens a new scroll.
    placed: Signal<u32>,
    parts: Signal<Rc<Parts<T>>>,
    trigger: FocusHandle,
    content: FocusHandle,
    linked: Linked,
    direction: ReadDirection,
    /// What a form sends while nothing is chosen (reka's `BubbleSelect`
    /// fallback, `''` by default).
    fallback: Signal<String>,
    /// The scope it was made in: what an option's leaving queues work for.
    owner: Option<gui::Owner>,
}

impl<T: 'static, M: Choice<T>> Clone for Select<T, M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static, M: Choice<T>> Copy for Select<T, M> {}

impl<T: 'static, M: Choice<T>> PartialEq for Select<T, M> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.open == other.open
    }
}

impl<T: 'static, M: Choice<T>> std::fmt::Debug for Select<T, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Select").finish_non_exhaustive()
    }
}

impl<T: Clone + PartialEq + 'static, M: Choice<T>> Select<T, M> {
    /// A select holding `value` and opening as `open` says, navigating in
    /// `direction`; made in its root's setup (its watchers live in that
    /// scope).
    #[must_use]
    #[track_caller]
    pub fn new(value: Bound<M>, open: Bound<bool>, direction: impl Into<ReadDirection>) -> Self {
        // One value replaces: choosing the chosen option keeps it
        // (`SelectRoot.vue` assigns `modelValue = value`); several toggle.
        let behavior = if M::MULTIPLE {
            SelectionBehavior::Toggle
        } else {
            SelectionBehavior::Replace
        };
        let direction = direction.into();
        let config = ListConfig::LISTBOX
            .direction(direction)
            .focus(ListFocus::Items)
            .behavior(behavior)
            .select_all(false);
        let select = Self {
            value,
            open,
            availability: Signal::new(Availability::Enabled),
            required: Signal::new(false),
            list: ListCore::new(config),
            known: Signal::new(Rc::new(Vec::new())),
            press: Signal::new(None),
            buttons: Signal::new((false, false)),
            parts: Signal::new(Rc::new(Parts {
                items: Collection::new(),
                boxes: WorldRefCell::new("a select's option boxes", HashMap::new()),
                list_box: WorldCell::new("a select's list box", None),
                scroll: ScrollHandle::new(),
                stepping: WorldCell::new("a select's scroll step timer", None),
                buttons: WorldCell::new("a select's scroll buttons", (false, false)),
                replaced: WorldCell::new("whether a select was placed again", false),
                by_keyboard: WorldCell::new("whether a select opens from the keyboard", false),
            })),
            placed: Signal::new(0),
            trigger: FocusHandle::new(),
            content: FocusHandle::new(),
            linked: Linked::new(Points::Controls),
            direction,
            fallback: Signal::new(String::new()),
            owner: gui::Owner::current(),
        };
        // The highlight is the focus: an option focused is highlighted, and
        // focus anywhere else lets the highlight go.
        gui::watch(gui::focus::focused, move |now, _| {
            if !select.parts.is_alive() {
                return;
            }
            let items = select.items();
            let at = now.and_then(|f| items.index_of(f));
            items.with(|list| match at {
                Some(i) => select.list.highlight(list, i, gui::focus::focus_visible()),
                None => select.list.clear(),
            });
        });
        // Opening, whoever opens it (`open`, the caller's model, open from
        // the start), starts at the top and focuses the anchor option;
        // closing stops a scroll button and forgets the press.
        gui::watch(
            move || open.get(),
            move |open, _| {
                if !select.parts.is_alive() {
                    return;
                }
                if *open {
                    select.opened();
                } else {
                    select.stop_scrolling();
                    select.press.set(None);
                    select.list.typeahead().reset();
                }
            },
        );
        if open.get_untracked() {
            select.opened();
        }
        select
    }

    /// What opening does, whoever opened it: the typeahead starts afresh,
    /// the list opens at its top (or where item-aligned placement puts it),
    /// and once placed its anchor option takes focus
    /// (`SelectContentImpl.vue:139-146`).
    fn opened(self) {
        self.list.typeahead().reset();
        let parts = self.parts();
        parts.scroll.reset();
        self.buttons.set((false, false));
        parts.buttons.set((false, false));
        parts.replaced.set(false);
        let by_keyboard = parts.by_keyboard.replace(false);
        self.later(move || self.focus_anchor(by_keyboard));
    }

    /// Whether it can be used (its `disabled`; a field's).
    pub fn set_availability(self, availability: Availability) {
        self.availability.set(availability);
    }

    /// Whether a form needs it filled (its `required`; a field's).
    pub fn set_required(self, required: bool) {
        self.required.set(required);
    }

    /// Ask `f` before each choice, by the pointer, the keys or typeahead on
    /// a closed trigger; [`Pick::Refuse`] leaves the value as it is and the
    /// list as it is (`SelectItem.vue:84-89,97-108`).
    pub fn on_pick(self, f: impl Into<gui::Callback<T, Pick>>) {
        self.list.on_pick(f.into());
    }

    /// What a form sends while nothing is chosen (`''` by default).
    pub fn set_fallback(self, fallback: impl Into<String>) {
        self.fallback.set(fallback.into());
    }

    /// The direction it reads in: what an item-aligned list lines up by.
    #[must_use]
    pub fn direction(self) -> Direction {
        self.direction.get()
    }

    /// How tall an item-aligned list is at least: five anchor options
    /// (`SelectItemAlignedPosition.vue:120-123`); the engine takes the
    /// list's own height when it is less.
    #[must_use]
    pub fn aligned_min_height(self) -> f32 {
        self.step_height() * 5.0
    }

    /// Navigate wrapping at the ends or not (reka's `loop`).
    pub fn set_wrap(self, wrap: Wrap) {
        self.list.set_config(self.list.config().wrap(wrap));
    }

    fn items(self) -> Collection<T> {
        self.parts.with_untracked(|p| p.items.clone())
    }

    fn parts(self) -> Rc<Parts<T>> {
        self.parts.with_untracked(Rc::clone)
    }

    /// What the select keeps beside its value, watched (tests: it goes
    /// with the select's scope).
    #[doc(hidden)]
    #[must_use]
    pub fn parts_held(self) -> std::rc::Weak<dyn std::any::Any> {
        let parts: Rc<dyn std::any::Any> = self.parts();
        Rc::downgrade(&parts)
    }

    /// The value (tracked).
    #[must_use]
    pub fn value(self) -> M {
        self.value.get()
    }

    /// Whether the list is open (tracked).
    #[must_use]
    pub fn is_open(self) -> bool {
        self.open.get()
    }

    /// Whether it can be used (tracked).
    #[must_use]
    pub fn availability(self) -> Availability {
        self.availability.get()
    }

    /// Whether nothing is chosen (tracked): the trigger's placeholder state.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.value.get().is_empty()
    }

    /// Whether `value` is chosen (tracked): an option's indicator.
    #[must_use]
    pub fn holds(self, value: &T) -> bool {
        self.value.get().holds(value)
    }

    /// Whether the option valued `value` is highlighted, which is to say
    /// focused (tracked).
    #[must_use]
    pub fn is_highlighted(self, value: &T) -> bool {
        self.list.is_highlighted(value)
    }

    /// The typeahead search (tracked).
    #[must_use]
    pub fn search(self) -> String {
        self.list.typeahead().search()
    }

    /// What the value part shows (tracked): the labels of the chosen
    /// options in the order chosen, joined by `", "`; `None` (the
    /// placeholder) when no option has the value.
    #[must_use]
    pub fn shown(self) -> Option<String> {
        let value = self.value.get();
        let known = self.known.get();
        let labels: Vec<&str> = value
            .values()
            .iter()
            .filter_map(|v| known.iter().find(|k| k.value == *v).map(|k| &*k.label))
            .collect();
        (!labels.is_empty()).then(|| labels.join(", "))
    }

    /// The anchor option's value (tracked): what opening focuses and what an
    /// item-aligned list lines up on the trigger.
    #[must_use]
    pub fn anchor(self) -> Option<T> {
        let value = self.value.get();
        let known = self.known.get();
        anchor_of(&known, &value)
            .and_then(|i| known.get(i))
            .map(|k| k.value.clone())
    }

    /// The options as last laid out (tracked): value, label, availability.
    #[must_use]
    pub fn options(self) -> Rc<Vec<Known<T>>> {
        self.known.get()
    }

    /// Whether the list can scroll up and down now (tracked): which scroll
    /// buttons show.
    #[must_use]
    pub fn can_scroll(self) -> (bool, bool) {
        self.buttons.get()
    }

    /// The trigger's focus.
    #[must_use]
    pub fn trigger_focus(self) -> FocusHandle {
        self.trigger
    }

    /// Open the list, shown as from the keyboard when `by_keyboard`: what
    /// any opening does follows: the search reset, the list at its top, the
    /// anchor option focused once the list is laid out open. An open list
    /// stays as it is: where it scrolled, what is focused.
    pub fn open(self, by_keyboard: bool) {
        if !self.availability.get_untracked().actionable() || self.open.get_untracked() {
            return;
        }
        let parts = self.parts();
        parts.by_keyboard.set(by_keyboard);
        self.open.set(true);
        if !self.open.get_untracked() {
            // A caller that keeps `open` as a value opens it as it renders
            // again, this turn, when opening reads the opener's modality;
            // one that keeps it closed leaves no word for a later opening.
            let select = self;
            self.later(move || {
                if select.parts.is_alive() {
                    select.parts().by_keyboard.set(false);
                }
            });
        }
    }

    /// Close the list; `refocus` gives focus to the trigger (Escape, a
    /// choice), shown as from the keyboard when `by_keyboard`.
    pub fn close(self, refocus: bool, by_keyboard: bool) {
        self.open.set(false);
        if refocus {
            gui::focus::set_focus(Some(self.trigger.id()), by_keyboard);
        }
    }

    /// Whether focus is on the list or one of its options, or nowhere.
    fn focus_inside(self) -> bool {
        match gui::untracked(gui::focus::focused) {
            None => true,
            Some(f) => f == self.content.id() || self.items().index_of(f).is_some(),
        }
    }

    fn focus_anchor(self, by_keyboard: bool) {
        if !self.open.get_untracked() {
            return;
        }
        let items = self.items();
        let value = self.value.get_untracked();
        let target = items.with(|list| {
            let known: Vec<Known<T>> = list.iter().map(known_of).collect();
            anchor_of(&known, &value)
                .filter(|&i| list.get(i).is_some_and(|it| it.availability.focusable()))
                .and_then(|i| list.get(i).map(|it| it.focus))
        });
        let to = target.unwrap_or(self.content);
        gui::focus::set_focus(Some(to.id()), by_keyboard);
    }

    /// Read the options as laid out into what the trigger and the look
    /// read, after the next layout: an option's setup, its unmounting and a
    /// change of its value, label or availability call it. The work is the
    /// select's (queued for the scope it was made in), so an option that is
    /// leaving may ask for it.
    pub fn refresh(self) {
        let work = move || {
            if !self.known.is_alive() {
                return;
            }
            let now: Vec<Known<T>> = self.items().with(|list| list.iter().map(known_of).collect());
            if self.known.with_untracked(|k| **k != now) {
                self.known.set(Rc::new(now));
            }
        };
        self.later(work);
    }

    /// Run `f` after the next layout as the select's work: queued for the
    /// scope it was made in, whoever asks (an option leaving, a caller's
    /// code outside any part), and never once that scope is gone.
    fn later(self, f: impl FnOnce() + 'static) {
        match self.owner {
            Some(owner) => owner.after_layout(f),
            None => gui::after_layout(f),
        }
    }

    /// Choose option `index` as an option is picked, closing the list (and
    /// focusing the trigger) when it holds one value.
    fn pick(self, index: usize, by_keyboard: bool) {
        let items = self.items();
        let picked = items.with(|list| self.list.choose(list, index, Some(self.value)));
        if matches!(picked, ListKey::Chose(_)) && !M::MULTIPLE {
            self.close(true, by_keyboard);
        }
    }

    /// `d` as the trigger: a combobox to assistive technology, as Chrome
    /// gives a `<select>`: named by its field's label (the caller's
    /// `labelled_by` or `aria_label`, never its text), expanded and
    /// controlling the list while open, its availability applied, its
    /// value the chosen labels ([`Select::shown`]), which UI Automation's
    /// and the Mac's value carry and announce as they change. AT-SPI reads
    /// numeric values alone, so the look's value part stays read too (a
    /// label whose text changes): the one way Linux reads the choice.
    /// Closed, the options are out of what is read. It takes the trigger's
    /// keys and opens on a press (see the module's docs). `open` is what
    /// the look's render read ([`Select::is_open`]).
    #[must_use]
    pub fn trigger(self, d: Div, open: bool) -> Div {
        let availability = self.availability.get();
        let d = self
            .linked
            .trigger(availability.apply(d), open)
            .role(gui::Role::ComboBox)
            .aria_expanded(open)
            .aria_required(self.required.get())
            .track_focus(self.trigger);
        let d = match self.shown() {
            Some(value) => d.aria_value(value),
            None => d,
        };
        if !availability.actionable() {
            return d;
        }
        d.on_key_down(move |ev| self.trigger_key(ev))
            .on_mouse_down(move |ev| {
                if ev.modifiers.control || self.open.get_untracked() {
                    return;
                }
                self.press.set(Some(Press {
                    x: ev.x,
                    y: ev.y,
                    moved: false,
                }));
                self.open(false);
            })
            .on_drag(move |ev| self.drag(ev))
            .on_click(move |_| {
                // The press that opened the list ends here: nothing to do.
                // A click without a press (a tap, a label, a screen reader)
                // opens it.
                if self.press.get_untracked().is_some() {
                    self.press.set(None);
                    return;
                }
                if !self.open.get_untracked() {
                    self.open(false);
                }
            })
    }

    /// The trigger's keys.
    fn trigger_key(self, ev: &KeyEvent) -> bool {
        let typeahead = self.list.typeahead();
        let searching = typeahead.is_searching();
        if let Some(typed) = typeahead.typed(ev) {
            let typed = typed.to_owned();
            if self.open.get_untracked() {
                let items = self.items();
                items.with(|list| {
                    let current = self.list.index_in(list);
                    if let Some(i) = typeahead.find(&typed, list, current) {
                        self.list.highlight(list, i, true);
                    }
                });
            } else if !M::MULTIPLE {
                self.choose_typed(&typed);
            } else {
                typeahead.push(&typed);
            }
            if searching || !ev.is(NamedKey::Space) {
                return true;
            }
        }
        let opens = crate::keymap::activates(ev) || ev.is(NamedKey::ArrowDown) || ev.is(NamedKey::ArrowUp);
        if opens && !self.open.get_untracked() {
            self.open(true);
            return true;
        }
        false
    }

    /// Closed: choose the option the search reaches from the chosen one.
    fn choose_typed(self, typed: &str) {
        let items = self.items();
        let value = self.value.get_untracked();
        let found = items.with(|list| {
            let current = list.iter().position(|i| value.holds(&i.value));
            self.list.typeahead().find(typed, list, current)
        });
        if let Some(i) = found {
            items.with(|list| self.list.choose(list, i, Some(self.value)));
        }
    }

    /// The trigger's press dragged: released far enough on an option, it
    /// chooses it; outside the list, it closes it.
    fn drag(self, ev: &DragEvent) {
        let Some(mut press) = self.press.get_untracked() else {
            return;
        };
        let far = (ev.x - press.x).abs() > PRESS_SLOP || (ev.y - press.y).abs() > PRESS_SLOP;
        if far && !press.moved {
            press.moved = true;
            self.press.set(Some(press));
        }
        if !ev.phase.is_last() {
            return;
        }
        self.press.set(None);
        // Cancelled (the trigger was disabled mid-press): nothing is chosen.
        if ev.phase == DragPhase::Cancel {
            return;
        }
        if !press.moved || !self.open.get_untracked() {
            return;
        }
        let parts = self.parts();
        let hit = self.items().with(|list| {
            let boxes = parts.boxes.borrow();
            list.iter()
                .position(|i| boxes.get(&i.focus.id()).is_some_and(|b| b.contains(ev.x, ev.y)))
        });
        match hit {
            Some(i) => self.pick(i, false),
            None => {
                if !parts.list_box.get().is_some_and(|b| b.contains(ev.x, ev.y)) {
                    self.close(true, false);
                }
            }
        }
    }

    /// `d` as the list (`SelectContent`). Open (`open`, what the look's
    /// render read), it is a modal layer: a press outside closes it and
    /// reaches nothing below, the page does not scroll, assistive technology
    /// reads only it, Tab stays in it, focus leaving does not close it,
    /// Escape closes it and focuses the trigger; it is a listbox controlled
    /// by the trigger, multi-select when it holds several, and takes the
    /// list's keys. Closed, it stays laid out, dormant ([`gui::Div::dormant`])
    /// and out of the flow, so its options are known and typeahead and the
    /// arrows choose among them; an option hidden otherwise is not one. The look wraps the open one in its
    /// floating placement.
    #[must_use]
    pub fn content(self, d: Div, open: bool) -> Div {
        if !open {
            return d
                .position(Position::Absolute)
                .width(0.0)
                .height(0.0)
                .min_width(0.0)
                .min_height(0.0)
                .overflow(Overflow::Hidden)
                .dormant(true);
        }
        let layer = Layer::new()
            .modal(true)
            .dismiss(DismissPolicy::DEFAULT.focus_outside(false))
            .focus_scope(FocusScope::new().trap(true).auto_focus(AutoFocus::Keep))
            .on_dismiss(move |reason| {
                // Focus goes back to the trigger as the list goes (a press
                // outside included: it reaches nothing below, so nothing
                // else took focus), unless something else holds it by then
                // (`SelectContentImpl.vue:305-312`).
                let refocus = self.focus_inside();
                self.close(refocus, matches!(reason, DismissReason::Escape));
                Dismiss::Close
            });
        let parts = self.parts();
        self.linked
            .content(d, false)
            .role(gui::Role::ListBox)
            .aria_multiselectable(M::MULTIPLE)
            .track_focus(self.content)
            .tab_stop(false)
            .layer(layer)
            .on_measured(move |b| parts.list_box.set(Some(*b)))
            .on_key_down(move |ev| self.content_key(ev))
    }

    /// The list's keys (focus on an option or on the list).
    fn content_key(self, ev: &KeyEvent) -> bool {
        if ev.is(NamedKey::Tab) {
            return true;
        }
        let items = self.items();
        let key = items.with(|list| self.list.key(ev, list, Some(self.value)));
        match key {
            ListKey::Chose(_) => {
                if !M::MULTIPLE {
                    self.close(true, true);
                }
                true
            }
            other => other.consumed(),
        }
    }

    /// `d` as the scrolling viewport the options sit in (`SelectViewport`):
    /// it scrolls by the select's handle, and tells which scroll buttons
    /// show. Its scrollbar is the look's to hide. `aligned`: the list lines
    /// its anchor option up on the trigger (item-aligned) rather than
    /// floating below it, as the look's render says; such a list is placed
    /// again once, as its up button first shows and pushes the options
    /// down, and focus goes to the anchor option again
    /// (`SelectItemAlignedPosition.vue:211-217`). The render hands it to the
    /// viewport's layout hook, which is the handle's (forked with it): a
    /// render writes nothing live.
    #[must_use]
    pub fn viewport(self, d: Div, aligned: bool) -> Div {
        self.placed.get();
        let parts = self.parts();
        let handle = parts.scroll;
        let shows = |l: &ScrollLayout| (l.offset.1 > 0.5, l.offset.1 < l.max.1 - 0.5);
        // Weak: the hook is held by the handle's state, which the parts name.
        let shown = Rc::downgrade(&parts);
        let buttons = self.buttons;
        let placed = self.placed;
        let select = self;
        handle.on_layout(move |l| {
            let Some(shown) = shown.upgrade() else {
                return false;
            };
            let now = shows(l);
            if now == shown.buttons.get() {
                return false;
            }
            let came = now.0 && !shown.buttons.get().0;
            shown.buttons.set(now);
            gui::untracked(|| buttons.set(now));
            if came && aligned && !shown.replaced.get() {
                shown.replaced.set(true);
                // The scroll afresh: the engine scrolls it as it opens,
                // the anchor over the trigger with the button above it.
                shown.scroll.reset();
                gui::untracked(|| placed.update(|n| *n = n.wrapping_add(1)));
                select.later(move || select.focus_anchor(gui::focus::focus_visible()));
            }
            true
        });
        d.track_scroll(&handle).role(gui::Role::GenericContainer)
    }

    /// `d` as a scroll button (`up` or down): hidden from assistive
    /// technology; while the pointer rests on it the list scrolls one
    /// anchor option's height every [`SCROLL_EVERY`], and nothing is
    /// highlighted.
    #[must_use]
    pub fn scroll_button(self, d: Div, up: bool) -> Div {
        d.aria_hidden().on_hover(move |inside| {
            if inside {
                gui::focus::set_focus(Some(self.content.id()), false);
                self.step(up);
            } else {
                self.stop_scrolling();
            }
        })
    }

    /// The anchor option's height, the step a scroll button scrolls by.
    fn step_height(self) -> f32 {
        let parts = self.parts();
        let value = self.value.get_untracked();
        let h = self.items().with(|list| {
            let known: Vec<Known<T>> = list.iter().map(known_of).collect();
            let boxes = parts.boxes.borrow();
            anchor_of(&known, &value)
                .and_then(|i| list.get(i))
                .or_else(|| list.first())
                .and_then(|i| boxes.get(&i.focus.id()).map(|b| b.height))
        });
        h.unwrap_or(0.0)
    }

    fn step(self, up: bool) {
        self.stop_scrolling();
        if !self.open.get_untracked() {
            return;
        }
        let dy = if up { -self.step_height() } else { self.step_height() };
        let handle = self.parts().scroll;
        let (x, y) = handle.offset();
        handle.scroll_to((x, (y + dy).max(0.0)));
        // The select owns the step, not whatever runs this (a hover
        // handler's render scope, which may run again): closing, leaving
        // the button, the button going (the list cannot scroll its way any
        // further) or unmounting stops it (`SelectScrollButtonImpl.vue`
        // clears its timer as the button unmounts).
        let timer = gui::detached(|| {
            clock::after(SCROLL_EVERY, move || {
                if !self.parts.is_alive() {
                    return;
                }
                let (shows_up, shows_down) = self.parts().buttons.get();
                if if up { shows_up } else { shows_down } {
                    self.step(up);
                } else {
                    self.parts().stepping.set(None);
                }
            })
        });
        self.parts().stepping.set(Some(timer));
    }

    fn stop_scrolling(self) {
        if !self.parts.is_alive() {
            return;
        }
        if let Some(t) = self.parts().stepping.take() {
            clock::cancel(t);
        }
    }

    /// `d` as the option valued `value`, labelled `label` (its text: what
    /// typeahead matches and the trigger shows), focused through `focus`
    /// (made in the option's setup); `availability` is its own (the
    /// select's being disabled disables it). An option to assistive
    /// technology, selected while chosen; the pointer focuses it and a
    /// click chooses it. The option's setup calls [`Select::refresh`].
    #[must_use]
    pub fn item(self, d: Div, value: T, label: Rc<str>, focus: FocusHandle, availability: Availability) -> Div {
        let availability = match self.availability.get() {
            Availability::Enabled => availability,
            root => root,
        };
        let chosen = self.value.get().holds(&value);
        let parts = self.parts();
        let id = focus.id();
        let measured = Rc::clone(&parts);
        let d = parts
            .items
            .item(
                d,
                Item::new(value, focus)
                    .label(label)
                    .availability(availability)
                    .active(chosen),
            )
            .role(gui::Role::ListBoxOption)
            .aria_selected(chosen)
            .tab_stop(false)
            .on_measured(move |b| {
                let mut boxes = measured.boxes.borrow_mut();
                boxes.insert(id, *b);
                // Kept for the options laid out: those that left are let
                // go once they outnumber them, so the boxes grow with the
                // options shown, never with every option ever shown.
                let laid = measured.items.len();
                if boxes.len() > laid.saturating_mul(2).saturating_add(8) {
                    boxes.retain(|f, _| measured.items.index_of(*f).is_some());
                }
            });
        if !availability.focusable() {
            return d.on_hover(move |inside| {
                if inside {
                    gui::focus::set_focus(Some(self.content.id()), false);
                }
            });
        }
        d.on_hover(move |inside| {
            if inside {
                if !focus.is_focused() {
                    gui::focus::set_focus(Some(id), false);
                }
            } else if focus.is_focused() {
                gui::focus::set_focus(Some(self.content.id()), false);
            }
        })
        .on_click(move |_| {
            let items = self.items();
            if let Some(i) = items.index_of(id) {
                self.pick(i, false);
            }
        })
    }
}

/// `d` as a group of options (`role=group`), named by its label: `label`
/// is the node [`group_label`] gives the label inside it; a group without
/// one is unnamed (`SelectGroup.vue:24-31`).
#[must_use]
pub fn group(d: Div, label: gui::NodeRef) -> Div {
    d.role(gui::Role::Group).labelled_by(label)
}

/// `d` as the label of its group: plain text, never focused nor chosen;
/// `label` is the node its [`group`] is named by (`SelectLabel.vue:20-26`).
#[must_use]
pub fn group_label(d: Div, label: gui::NodeRef) -> Div {
    d.node_ref(label)
}

/// `d` as a separator between options: decorative, hidden from assistive
/// technology (`SelectSeparator.vue:14-19`), and the pointer passes through
/// it.
#[must_use]
pub fn separator(d: Div) -> Div {
    d.aria_hidden().pass_through()
}

/// Where a select's list opens in popper mode: below, at the start, as wide
/// as the trigger at least and no taller than the room below
/// (`SelectPopperPosition.vue:12-15`). `.arrow()` gives it reka's arrow
/// ([`Select::arrow`]).
pub const POPPER: Popper = Popper::DEFAULT
    .align(SideAlign::Start)
    .match_width(true)
    .fit_height(true);

impl<T: Clone + PartialEq + 'static, M: Choice<T>> Select<T, M> {
    /// `d` as the arrow of a list opened by `popper` (with its arrow) and
    /// placed as `placed` says (popper mode only): reka's 10 × 5 arrow,
    /// hidden from assistive technology, on the list's edge facing the
    /// trigger, pointing at its centre, kept in place but unseen when it
    /// cannot (`SelectArrow.vue:20-25`). The look draws its shape.
    #[must_use]
    pub fn arrow(d: Div, popper: &Popper, placed: Option<&Placement>) -> Div {
        let (w, h) = popper.arrow.unwrap_or(crate::popper::ARROW);
        let d = d.aria_hidden().position(Position::Absolute).width(w).height(h);
        let Some(at) = placed.and_then(|p| popper.arrow_at(p)) else {
            return d.invisible(true);
        };
        let d = match at.edge {
            gui::Side::Top => d.left(at.offset).top(-h),
            gui::Side::Bottom => d.left(at.offset).bottom(-h),
            gui::Side::Left => d.top(at.offset).left(-h),
            gui::Side::Right => d.top(at.offset).right(-h),
        };
        d.invisible(at.hidden)
    }

    /// What a form sends: the chosen values as [`crate::form::Submit`]
    /// sends them, or the fallback while nothing is chosen.
    #[must_use]
    pub fn form_data(self) -> crate::form::FormData
    where
        M: crate::form::Submit,
    {
        let value = self.value.get_untracked();
        if value.is_empty() {
            return crate::form::FormData::Text(self.fallback.get_untracked());
        }
        value.form_data()
    }
}

/// A form sends a select's value.
impl<T: Clone + PartialEq + 'static, M: Choice<T>> crate::form::FormValue for Select<T, M> {
    type Value = M;

    fn value(&self) -> M {
        self.value.get()
    }

    fn set_value(&self, value: M) {
        self.value.set(value);
    }

    fn focus(&self) -> Option<FocusHandle> {
        Some(self.trigger)
    }
}

fn known_of<T: Clone>(i: &Item<T>) -> Known<T> {
    Known {
        value: i.value.clone(),
        label: Rc::clone(&i.label),
        availability: i.availability,
    }
}

/// The anchor among `known` for `value` (see the module's docs).
fn anchor_of<T: PartialEq, M: Choice<T>>(known: &[Known<T>], value: &M) -> Option<usize> {
    let first_usable = known.iter().position(|k| k.availability.focusable());
    if M::MULTIPLE {
        return known.iter().position(|k| value.holds(&k.value)).or(first_usable);
    }
    let chosen = known.iter().position(|k| value.holds(&k.value));
    match (chosen, first_usable) {
        (Some(c), Some(f)) => Some(c.max(f)),
        (c, f) => c.or(f),
    }
}

impl<T> Options<T> for [Known<T>]
where
    T: Clone,
{
    fn value(&self, index: usize) -> Option<T> {
        self.get(index).map(|k| k.value.clone())
    }
}

impl<T> crate::collection::Items for [Known<T>] {
    fn len(&self) -> usize {
        <[Known<T>]>::len(self)
    }

    fn availability(&self, index: usize) -> Availability {
        self.get(index).map_or(Availability::Disabled, |k| k.availability)
    }

    fn label(&self, index: usize) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(self.get(index).map_or("", |k| &k.label))
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::{Known, anchor_of};
    use crate::availability::Availability;

    fn known(disabled: &[bool]) -> Vec<Known<u8>> {
        disabled
            .iter()
            .zip(0_u8..)
            .map(|(d, v)| Known {
                value: v,
                label: Rc::from(""),
                availability: Availability::from_disabled(*d),
            })
            .collect()
    }

    #[test]
    fn the_anchor_follows_reka() {
        let k = known(&[true, false, false, false]);
        assert_eq!(anchor_of(&k, &None::<u8>), Some(1), "nothing chosen: the first usable");
        assert_eq!(anchor_of(&k, &Some(2_u8)), Some(2), "the chosen one");
        assert_eq!(
            anchor_of(&k, &Some(0_u8)),
            Some(1),
            "chosen and disabled before the first usable: the first usable wins"
        );
        let k = known(&[false, false, true]);
        assert_eq!(
            anchor_of(&k, &Some(2_u8)),
            Some(2),
            "chosen, disabled, after: it is the anchor"
        );
        assert_eq!(
            anchor_of(&k, &vec![1_u8, 0]),
            Some(0),
            "several: the first chosen in display order"
        );
        assert_eq!(anchor_of(&k, &Vec::<u8>::new()), Some(0));
    }
}
