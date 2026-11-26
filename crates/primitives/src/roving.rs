//! A group that is one Tab stop (reka's `RovingFocusGroup` and
//! `RovingFocusItem`): Tab enters it at one item and leaves it at once; the
//! arrow keys move focus among its items by its [`Keymap`].
//!
//! - **One Tab stop.** Every item takes focus, but only one is a Tab stop
//!   (`gui::Div::tab_stop`, bound to the node, so the stop moving renders
//!   nothing): the item holding focus while focus is in the group;
//!   otherwise the entry item. So Tab and Shift+Tab both leave the
//!   group in one press, and a group with no focusable item is no stop.
//! - **Entry.** Focus coming into the group by Tab reaches, in this order:
//!   the active item (the selected tab, the checked radio), the item that
//!   held focus last, the first focusable one. reka gets there through a
//!   focusable container that forwards focus, which makes forward Tab and
//!   Shift+Tab enter differently (research §6, "Dễ sót"); here the entry
//!   item itself is the stop, so both enter alike.
//! - **Arrows.** A key the item's keymap reads moves focus to the item the
//!   step reaches ([`crate::keymap::step`]), skipping disabled ones, and is
//!   consumed even when focus stays. Keys reaching the item from a
//!   descendant that holds focus (an input inside it) are not the group's.
//!   An item with keys of its own (a trigger inside a toolbar) sets its
//!   handler after [`Roving::item`] and passes the keys it leaves to
//!   [`Roving::key`]: its own run first, as reka binds the group's listener
//!   after the item's.
//! - The pointer focuses an item it presses (a disabled one takes no
//!   focus), which makes it the group's last.

use gui::engine::{WorldCell, WorldRefCell};

use gui::focus::FocusId;
use gui::{Div, FocusHandle, KeyEvent, Signal};

use crate::collection::{Collection, Item};
use crate::direction::ReadDirection;
use crate::keymap::Keymap;

/// One item of a [`Roving`] group: its value, focus, availability and
/// whether it is active.
pub type RovingItem<T> = Item<T>;

/// What a key did in a roving group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nav<T> {
    /// Not a navigation key here: left to others.
    Ignored,
    /// A navigation key with nowhere to go: consumed, focus stays.
    Stayed,
    /// Focus moved to the item of this value.
    Moved(T),
}

impl<T> Nav<T> {
    /// Whether the key was consumed.
    #[must_use]
    pub fn consumed(&self) -> bool {
        !matches!(self, Self::Ignored)
    }
}

/// What the Tab stop is worked out from: the layout pass the items are
/// from, the node holding focus, and the item that held it last.
type StopInputs = (u64, Option<FocusId>, Option<FocusId>);

/// Its cells are the world's that made the group ([`WorldCell`]): a check
/// writes none of a live group's (ADR-0034, S7).
#[derive(Clone)]
struct State<T: 'static> {
    items: Collection<T>,
    keymap: Keymap,
    direction: ReadDirection,
    /// The Tab stop as last worked out, kept while what it was worked out
    /// from holds: Tab asks every item of the group in one look, and one
    /// walk of the items answers them all.
    stop: WorldCell<Option<(StopInputs, Option<FocusId>)>>,
    /// Focus that reached a node the items as last laid out do not hold,
    /// with the pass they are from, oldest first: an item focused from code
    /// in the tick it mounts, before layout lays it out. Each is decided
    /// once a later pass is laid out ([`Roving::settle`]).
    pending: WorldRefCell<Vec<(u64, FocusId)>>,
}

/// A roving focus group: made once in its root's setup (its watcher lives
/// in that scope), `Copy`.
///
/// Its Tab stop is not kept: it is worked out from the items as layout last
/// laid them out, when Tab looks (`gui::Div::tab_stop` with a closure). So
/// nothing is scheduled from an item's render, which stays pure, and the
/// stop is never stale: an item mounted, unmounted, made active or
/// disabled counts at the next Tab.
pub struct Roving<T: 'static> {
    state: Signal<State<T>>,
    /// The item that held focus last in the group.
    last: Signal<Option<FocusId>>,
}

impl<T: 'static> Clone for Roving<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Roving<T> {}

impl<T: 'static> PartialEq for Roving<T> {
    fn eq(&self, other: &Self) -> bool {
        self.last == other.last
    }
}

impl<T: 'static> std::fmt::Debug for Roving<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Roving").finish_non_exhaustive()
    }
}

impl<T: Clone + 'static> Roving<T> {
    /// A group navigated by `keymap` in `direction` (a fixed one, or
    /// [`crate::use_direction`]'s, read at each key); called in setup.
    #[must_use]
    #[track_caller]
    pub fn new(keymap: Keymap, direction: impl Into<ReadDirection>) -> Self {
        let direction = direction.into();
        let group = Self {
            state: Signal::new(State {
                items: Collection::new(),
                keymap,
                direction,
                stop: WorldCell::new("a roving group's Tab stop", None),
                pending: WorldRefCell::new("a roving group's pending focus", Vec::new()),
            }),
            last: Signal::new(None),
        };
        // Focus reaching one of the group's items makes it the group's last;
        // focus on a node not laid out yet waits for the next pass to say.
        gui::watch(gui::focus::focused, move |now, _| {
            if let Some(f) = *now
                && group.state.is_alive()
            {
                group.settle();
                if group.holds(f) {
                    group.remember(f);
                } else {
                    let pass = group.items().pass();
                    group
                        .state
                        .with_untracked(|s| s.pending.with_mut(|p| p.push((pass, f))));
                }
            }
        });
        group
    }

    /// Navigate by `keymap` from now on (a caller's orientation or `loop`).
    pub fn set_keymap(self, keymap: Keymap) {
        self.state.update(|s| s.keymap = keymap);
    }

    /// Navigate in `direction` from now on.
    pub fn set_direction(self, direction: impl Into<ReadDirection>) {
        let direction = direction.into();
        self.state.update(|s| s.direction = direction);
    }

    /// The group's items, as last laid out.
    #[must_use]
    pub fn items(self) -> Collection<T> {
        self.state.with_untracked(|s| s.items.clone())
    }

    /// The item that is the group's Tab stop now: the focused item while
    /// focus is in the group, else the entry item (untracked: it is worked
    /// out from the items as last laid out, which no signal holds).
    #[must_use]
    pub fn tab_stop(self) -> Option<FocusId> {
        self.settle();
        let focused = gui::untracked(gui::focus::focused);
        let inputs = (self.items().pass(), focused, self.last.get_untracked());
        if let Some((at, stop)) = self.state.with_untracked(|s| s.stop.get())
            && at == inputs
        {
            return stop;
        }
        let stop = focused.filter(|f| self.holds(*f)).or_else(|| self.entry());
        self.state.with_untracked(|s| s.stop.set(Some((inputs, stop))));
        stop
    }

    /// `focus` is the group's last: newer than any focus still waiting.
    fn remember(self, focus: FocusId) {
        self.state.with_untracked(|s| s.pending.with_mut(Vec::clear));
        self.last.set(Some(focus));
    }

    /// Decide the focus that waits for a pass laid out after it: one the
    /// items now hold was an item focused as it mounted, and the newest of
    /// them is the group's last; the rest were never the group's.
    fn settle(self) {
        let pass = self.items().pass();
        let decided: Vec<FocusId> = self.state.with_untracked(|s| {
            s.pending.with_mut(|pending| {
                let (decided, waiting) = pending.drain(..).partition::<Vec<_>, _>(|&(at, _)| at != pass);
                *pending = waiting;
                decided.into_iter().map(|(_, f)| f).collect()
            })
        });
        if let Some(&f) = decided.iter().rev().find(|&&f| self.holds(f)) {
            self.last.set(Some(f));
        }
    }

    /// Whether `focus` is one of the group's focusable items.
    fn holds(self, focus: FocusId) -> bool {
        self.items()
            .with(|list| list.iter().any(|i| i.focus.id() == focus && i.availability.focusable()))
    }

    /// `d` as `item`'s node: it joins the group, takes focus through the
    /// item's handle, is the group's Tab stop when it should be, and takes
    /// the group's keys. Called in the render of the item, which reads
    /// nothing of the group and schedules nothing: the stop is bound to the
    /// node (`gui::Div::tab_stop` with a closure, read when Tab looks), so
    /// moving focus in the group renders none of its items.
    #[must_use]
    pub fn item(self, d: Div, item: RovingItem<T>) -> Div {
        let focus = item.focus;
        self.member(d, item)
            .on_key_down(move |ev| self.key(focus, ev).consumed())
    }

    /// `d` as `item`'s node, as [`Self::item`] but taking no keys: the
    /// group's keys are handled by an ancestor (a toolbar's root, which
    /// gets what its items' own controls leave, [`Self::focused_key`]).
    #[must_use]
    pub fn member(self, d: Div, item: RovingItem<T>) -> Div {
        let id = item.focus.id();
        let items = self.items();
        items
            .item(d, item)
            // A frame kept past the group's disposal asks a gone group: no stop.
            .tab_stop(move || self.state.is_alive() && self.tab_stop() == Some(id))
    }

    /// What `ev` does, reaching an ancestor of the group's items: as
    /// [`Self::key`] from the focused item, ignored while focus is on none
    /// of them (a field inside, a menu opened from an item).
    pub fn focused_key(self, ev: &KeyEvent) -> Nav<T> {
        let Some(f) = gui::untracked(gui::focus::focused) else {
            return Nav::Ignored;
        };
        let from = self
            .items()
            .with(|list| list.iter().find(|i| i.focus.id() == f).map(|i| i.focus));
        from.map_or(Nav::Ignored, |from| self.key(from, ev))
    }

    /// What `ev` does, reaching the item focused through `from`: focus moves
    /// as the keymap says. Only a key pressed while that item itself holds
    /// focus navigates.
    pub fn key(self, from: FocusHandle, ev: &KeyEvent) -> Nav<T> {
        if gui::untracked(gui::focus::focused) != Some(from.id()) {
            return Nav::Ignored;
        }
        let (keymap, direction, items) = self.state.with_untracked(|s| (s.keymap, s.direction, s.items.clone()));
        let Some(intent) = keymap.intent(ev, direction.get()) else {
            return Nav::Ignored;
        };
        let target = items.with(|list| {
            let at = list.iter().position(|i| i.focus == from);
            keymap
                .step(list, at, intent)
                .and_then(|to| list.get(to))
                .map(|i| (i.focus, i.value.clone()))
        });
        match target {
            Some((focus, value)) => {
                gui::focus::set_focus(Some(focus.id()), true);
                self.remember(focus.id());
                Nav::Moved(value)
            }
            None if items.is_empty() => Nav::Ignored,
            None => Nav::Stayed,
        }
    }

    /// Focus the group's entry item, as Tab into it does (a menu opened
    /// from the keyboard focusing its first item); whether one was found.
    pub fn focus_entry(self) -> bool {
        let entry = self.entry();
        if let Some(id) = entry {
            gui::focus::set_focus(Some(id), gui::focus::modality() == gui::Modality::Keyboard);
        }
        entry.is_some()
    }

    /// The entry item: active, else last focused, else first focusable.
    fn entry(self) -> Option<FocusId> {
        self.settle();
        let last = self.last.get_untracked();
        self.items().with(|list| {
            let usable = |i: &&Item<T>| i.availability.focusable();
            list.iter()
                .filter(usable)
                .find(|i| i.active)
                .or_else(|| list.iter().filter(usable).find(|i| Some(i.focus.id()) == last))
                .or_else(|| list.iter().find(usable))
                .map(|i| i.focus.id())
        })
    }
}
